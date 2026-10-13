//! `kiln-control` — the Core 1 real-time safety loop for the pico-kiln
//! controller, a platform-generic port of `kiln/control_thread.py`.
//!
//! The crate is split so the fire-relevant logic is provable off-device:
//!
//! - [`Controller`] is a **synchronous** state machine that performs one control
//!   iteration per call ([`Controller::iterate`]) plus the 10 Hz SSR sub-tick
//!   ([`Controller::ssr_subtick`]). It is generic over the [`kiln_hal::platform`]
//!   traits (sensor, SSR, watchdog), takes time injected, and pulls in no
//!   `embassy` dependency — so the exact shipping logic runs under `cargo test`
//!   and in `kiln-sim` against a thermal model.
//!
//! The async task that drives the `Controller` (embassy-time cadence + channel
//! I/O) lives in `kiln-firmware`, which reimplements the loop to weave in the
//! cross-core flash-pause handshake.
#![cfg_attr(not(test), no_std)]

pub mod controller;
pub mod params;

#[cfg(any(test, feature = "mock"))]
pub mod mock;

pub use controller::{Controller, IterationOutcome, ScheduledItem};
pub use params::ControlParams;

#[cfg(test)]
mod tests {
    use super::mock::{CountingWatchdog, MockSensor, MockSsr};
    use super::{ControlParams, Controller};
    use kiln_core::profile::{Profile, Step};
    use kiln_core::protocol::{Command, ProfileName};
    use kiln_core::state::{KilnError, KilnState};
    use kiln_core::tuner::TuningMode;

    fn name(s: &str) -> ProfileName {
        ProfileName::new(s).unwrap()
    }

    fn new_controller() -> Controller<MockSensor, MockSsr, CountingWatchdog> {
        Controller::new(
            MockSensor::new(25.0),
            MockSsr::new(),
            CountingWatchdog::new(),
            ControlParams::default(),
            0,
        )
    }

    #[test]
    fn constructor_arms_watchdog() {
        let c = new_controller();
        assert_eq!(c.watchdog().started_ms, Some(8_000));
        assert_eq!(c.state(), KilnState::Idle);
    }

    #[test]
    fn run_profile_then_pid_commands_heat_below_target() {
        let mut c = new_controller();
        let profile = Profile::new(&[Step::ramp(200.0, Some(100.0), None)]).unwrap();

        // Warm-up idle tick so the loop has read the sensor (current_temp = 25),
        // as the real loop is already reading temperature when a run arrives;
        // run_profile then seeds step_start_temp from 25, not 0.
        c.iterate(None, 500, 0);

        let out = c.iterate(
            Some(Command::RunProfile {
                profile: name("glaze.json"),
                parsed: profile,
            }),
            1_000,
            1,
        );
        assert!(!out.faulted);
        assert_eq!(c.state(), KilnState::Running);

        // Hold the measured temperature at 25 °C while the ramp setpoint climbs;
        // the PID must then command heat and the watchdog must be fed each tick.
        for i in 2..=120u64 {
            let o = c.iterate(None, i * 1_000, i as i64);
            assert!(!o.faulted);
        }

        let snap = c.snapshot(121_000, 121);
        assert_eq!(snap.state, KilnState::Running);
        assert!(
            snap.target_temp > 26.0,
            "ramp target should climb above the 25C start, got {}",
            snap.target_temp
        );
        assert!(
            snap.ssr_output > 0.0,
            "PID should command heat below target, got {}",
            snap.ssr_output
        );
        assert_eq!(snap.profile_name.unwrap().as_str(), "glaze.json");
        // warm-up tick + run tick + 119 loop ticks, each fed once.
        assert_eq!(c.watchdog().feeds, 121);
    }

    #[test]
    fn sustained_sensor_faults_emergency_without_feeding_watchdog() {
        let mut c = new_controller();

        // One good reading initialises the filter (watchdog fed).
        let o = c.iterate(None, 1_000, 1);
        assert!(!o.faulted);
        assert_eq!(c.watchdog().feeds, 1);

        // Now the sensor faults forever. Early faults return last-good (still
        // fed); once the cold-start budget (40) is exhausted the loop must hit
        // the emergency path: SSR forced off, error state, NO watchdog feed.
        c.sensor_mut().set_fault(true);
        let mut faulted = false;
        for i in 2..=80u64 {
            let feeds_before = c.watchdog().feeds;
            let off_before = c.ssr().force_off_count;
            let o = c.iterate(None, i * 1_000, i as i64);
            if o.faulted {
                assert_eq!(
                    c.watchdog().feeds,
                    feeds_before,
                    "watchdog must NOT be fed on the emergency iteration"
                );
                assert!(
                    c.ssr().force_off_count > off_before,
                    "SSR must be forced off on emergency"
                );
                assert_eq!(c.state(), KilnState::Error);
                faulted = true;
                break;
            } else {
                assert_eq!(c.watchdog().feeds, feeds_before + 1);
            }
        }
        assert!(
            faulted,
            "sustained sensor faults must trigger the emergency path"
        );
    }

    #[test]
    fn emergency_publishes_error_status_once_for_logging() {
        let mut c = new_controller();
        // One good reading initialises the filter.
        assert!(!c.iterate(None, 1_000, 1).faulted);

        // Sensor faults forever; the emergency iteration must PUBLISH an Error
        // status so Core 0 can log the terminal ERROR row and clear the recovery
        // pointer. Faulted ticks return before the normal status check, so without
        // the fix they publish nothing and the last logged row stays RUNNING.
        c.sensor_mut().set_fault(true);
        let mut emergency = None;
        for i in 2..=80u64 {
            let o = c.iterate(None, i * 1_000, i as i64);
            if o.faulted {
                emergency = Some(o);
                break;
            }
        }
        let o = emergency.expect("emergency path must be reached");
        let snap = o
            .publish
            .expect("emergency iteration must publish an Error status for logging");
        assert_eq!(snap.state, KilnState::Error);
        assert!(matches!(snap.error, Some(KilnError::SensorFault)));

        // A subsequent faulted tick (already in Error) must NOT republish — avoid
        // waking the logger every second once the terminal row is written.
        let o2 = c.iterate(None, 81_000, 81);
        assert!(o2.faulted);
        assert!(
            o2.publish.is_none(),
            "must not republish while already in Error"
        );
    }

    #[test]
    fn over_temp_in_run_publishes_error_once_off_cycle() {
        // The normal (non-fault) path only checks status_due to publish. A clean
        // Running→Error transition (over-temp) landing inside the 5 s status window
        // must still publish once, or Core 0 never logs the terminal ERROR row /
        // fixes the recovery pointer. `faulted` must stay false (soft-recoverable).
        let mut c = new_controller();
        let profile = Profile::new(&[Step::hold(100.0, 100_000.0)]).unwrap();
        c.iterate(None, 500, 0); // warm-up: first status publish (t=500)
        c.iterate(
            Some(Command::RunProfile {
                profile: name("glaze.json"),
                parsed: profile,
            }),
            1_000,
            1,
        );
        assert_eq!(c.state(), KilnState::Running);

        // 1400 °C is in-range (≤1500) so it is an over-temp, not a sensor fault.
        // With a 3-sample median, the second reading carries the median over 1300.
        c.sensor_mut().set_temp(1400.0);

        // Still within the 5 s window (last publish t=500): a normal tick must not
        // publish — this establishes status_due is false here.
        let pre = c.iterate(None, 1_500, 1);
        assert!(
            pre.publish.is_none(),
            "status_due should be false mid-window"
        );

        // Median now crosses max_temp → Running→Error. Must force a publish despite
        // status_due still being false.
        let o = c.iterate(None, 2_000, 2);
        assert!(
            !o.faulted,
            "over-temp is soft-recoverable: faulted must stay false"
        );
        assert_eq!(c.state(), KilnState::Error);
        let snap = o
            .publish
            .expect("clean→Error transition must publish for logging");
        assert_eq!(snap.state, KilnState::Error);
        assert!(matches!(
            snap.error,
            Some(KilnError::MaxTempExceeded { .. })
        ));

        // Already in Error, still mid-window: must not republish every tick.
        let o2 = c.iterate(None, 2_500, 2);
        assert!(
            o2.publish.is_none(),
            "must not republish while already in Error mid-window"
        );
    }

    #[test]
    fn shutdown_forces_ssr_off_and_returns_to_idle() {
        let mut c = new_controller();
        let profile = Profile::new(&[Step::hold(100.0, 10_000.0)]).unwrap();
        c.iterate(
            Some(Command::RunProfile {
                profile: name("bisque.json"),
                parsed: profile,
            }),
            1_000,
            1,
        );
        assert_eq!(c.state(), KilnState::Running);

        let off_before = c.ssr().force_off_count;
        c.iterate(Some(Command::Shutdown), 2_000, 2);
        assert_eq!(c.state(), KilnState::Idle);
        assert!(
            c.ssr().force_off_count > off_before,
            "Shutdown must force the SSR off"
        );
        assert!(c.snapshot(2_000, 2).profile_name.is_none());
    }

    #[test]
    fn start_tuning_enters_tuning_and_publishes_snapshot() {
        let mut c = new_controller();
        c.iterate(
            Some(Command::StartTuning {
                mode: TuningMode::Safe,
                max_temp: None,
            }),
            1_000,
            1,
        );
        assert_eq!(c.state(), KilnState::Tuning);

        // Next tick runs the tuning path: SAFE step 0 holds 60% SSR.
        c.iterate(None, 2_000, 2);
        let snap = c.snapshot(2_000, 2);
        assert_eq!(snap.state, KilnState::Tuning);
        let t = snap.tuning.expect("tuning snapshot present while tuning");
        assert_eq!(t.mode, TuningMode::Safe);
        assert_eq!(t.total_steps, 3);
        assert_eq!(t.ssr_percent, 60.0);

        // Stop tuning returns to idle and clears the snapshot.
        c.iterate(Some(Command::StopTuning), 3_000, 3);
        assert_eq!(c.state(), KilnState::Idle);
        assert!(c.snapshot(3_000, 3).tuning.is_none());
    }

    #[test]
    fn flash_pause_does_not_zero_duty_until_next_cycle() {
        // Regression: the firmware's flash-write handshake de-energises the SSR
        // before parking Core 1. It used `force_ssr_off`, which zeroes the LOCKED
        // duty — re-latched only at the next SSR cycle boundary — so a flash
        // flush at 100% duty knocked the relay off for the REMAINDER of the 20 s
        // cycle (observed: ~15 s dropout every 120 s flush). `pause_ssr_off` must
        // leave the schedule untouched so the first sub-tick after the park
        // re-energises immediately.
        let mut c = new_controller();
        let profile = Profile::new(&[Step::ramp(1200.0, Some(10_000.0), None)]).unwrap();
        c.iterate(None, 500, 0); // warm-up read (25°C)
        c.iterate(
            Some(Command::RunProfile {
                profile: name("glaze.json"),
                parsed: profile,
            }),
            1_000,
            1,
        );
        // Far below target → PID commands (near) full duty. Advance past a cycle
        // boundary so the duty is locked in and the relay is ON.
        c.iterate(None, 2_000, 2);
        assert!(c.ssr_subtick(21_000).unwrap(), "relay ON at high duty");

        // Flash pause mid-cycle: pins off, schedule untouched.
        c.pause_ssr_off().unwrap();

        // First sub-tick after the park (100 ms later, same cycle) must be ON
        // again — with the old force_ssr_off this stayed OFF until the next
        // cycle boundary (up to SSR_CYCLE_TIME later).
        assert!(
            c.ssr_subtick(21_100).unwrap(),
            "relay must re-energise on the first sub-tick after a flash pause"
        );
    }

    // ---- K1: sensor reading low-but-plausible --------------------------

    /// Warm-up read at `t0_s`, then start `profile`. Returns the next tick time.
    fn start(
        c: &mut Controller<MockSensor, MockSsr, CountingWatchdog>,
        profile: Profile,
    ) -> u64 {
        c.iterate(None, 500, 0);
        c.iterate(
            Some(Command::RunProfile {
                profile: name("k1.json"),
                parsed: profile,
            }),
            1_000,
            1,
        );
        assert_eq!(c.state(), KilnState::Running);
        2
    }

    #[test]
    fn hold_with_sensor_stuck_low_errors_within_one_window() {
        // The audit's K1 reproduction: a 1000 C hold while the amplifier reads a
        // constant, fault-free 0.0 C. Before the heat watch this ran the relay at
        // 100 % for the full 4 h hold with no error and the watchdog fed.
        let mut c = new_controller();
        c.sensor_mut().set_temp(0.0);
        let t = start(&mut c, Profile::new(&[Step::hold(1000.0, 4.0 * 3600.0)]).unwrap());
        let mut tripped_at = None;
        for s in t..4 * 3600 {
            let o = c.iterate(None, s * 1_000, s as i64);
            if c.state() == KilnState::Error {
                tripped_at = Some(s);
                assert!(o.publish.is_some(), "the Error transition must be published");
                assert!(!o.faulted, "soft error: watchdog keeps being fed");
                break;
            }
        }
        let s = tripped_at.expect("must not heat blind for the whole hold");
        assert!(s <= 2 + 900 + 1, "tripped at {s}s, expected within one 15 min window");
        let snap = c.snapshot(s * 1_000, s as i64);
        assert!(matches!(snap.error, Some(KilnError::NoHeatResponse { .. })));
        assert_eq!(snap.ssr_output, 0.0);
        assert!(c.ssr().force_off_count > 0, "relay forced off");
    }

    #[test]
    fn tuning_with_sensor_stuck_low_errors() {
        // HIGH_TEMP tuning with the reading stuck at 25 C previously ran 5820 s of
        // relay-on and "completed".
        let mut c = new_controller();
        c.iterate(
            Some(Command::StartTuning {
                mode: TuningMode::HighTemp,
                max_temp: None,
            }),
            1_000,
            1,
        );
        assert_eq!(c.state(), KilnState::Tuning);
        let mut tripped_at = None;
        for s in 2..6_000u64 {
            c.iterate(None, s * 1_000, s as i64);
            if c.state() == KilnState::Error {
                tripped_at = Some(s);
                break;
            }
        }
        let s = tripped_at.expect("open-loop tuning must not heat blind");
        assert!(s <= 1_000, "tripped at {s}s");
        let snap = c.snapshot(s * 1_000, s as i64);
        assert!(matches!(snap.error, Some(KilnError::NoHeatResponse { .. })));
        assert!(snap.tuning.is_none(), "tuner dropped");
    }

    #[test]
    fn real_heating_does_not_trip_the_heat_watch() {
        // Same 1000 C hold, but the kiln responds (+60 C/h while short of the
        // setpoint): no error for two hours.
        let mut c = new_controller();
        c.sensor_mut().set_temp(800.0);
        let t = start(&mut c, Profile::new(&[Step::hold(1000.0, 4.0 * 3600.0)]).unwrap());
        for s in t..2 * 3600 {
            c.sensor_mut().set_temp(800.0 + s as f32 * 60.0 / 3600.0);
            c.iterate(None, s * 1_000, s as i64);
            assert_eq!(c.state(), KilnState::Running, "false trip at {s}s");
        }
    }

    #[test]
    fn recovery_hold_with_sensor_stuck_low_errors() {
        // After a power cut the controller resumes into a recovery hold at the
        // last logged temperature (700 C); the reading is stuck at 600 C.
        let mut c = new_controller();
        c.sensor_mut().set_temp(600.0);
        c.iterate(None, 500, 0);
        c.iterate(
            Some(Command::ResumeProfile {
                profile: name("k1.json"),
                parsed: Profile::new(&[Step::ramp(1000.0, Some(100.0), None)]).unwrap(),
                elapsed_seconds: 3_600.0,
                last_logged_temp: Some(700.0),
                current_temp: Some(600.0),
                step_index: Some(0),
            }),
            1_000,
            1,
        );
        assert_eq!(c.state(), KilnState::Running);
        assert!(c.snapshot(1_000, 1).is_recovering, "recovery hold active");
        let mut tripped_at = None;
        for s in 2..3_600u64 {
            c.iterate(None, s * 1_000, s as i64);
            if c.state() == KilnState::Error {
                tripped_at = Some(s);
                break;
            }
        }
        let s = tripped_at.expect("the recovery hold must not heat blind");
        assert!(s <= 2 + 900 + 1, "tripped at {s}s");
        assert!(matches!(
            c.snapshot(s * 1_000, s as i64).error,
            Some(KilnError::NoHeatResponse { .. })
        ));
    }

    #[test]
    fn holding_within_the_band_never_trips() {
        // A kiln 5 C short of its hold target at saturated output (a tired kiln
        // near its ceiling) is inside the 10 C arrival band: no error.
        let mut c = new_controller();
        c.sensor_mut().set_temp(995.0);
        let t = start(&mut c, Profile::new(&[Step::hold(1000.0, 4.0 * 3600.0)]).unwrap());
        for s in t..3 * 3600 {
            c.iterate(None, s * 1_000, s as i64);
            assert_eq!(c.state(), KilnState::Running, "false trip at {s}s");
            assert!(!c.heat_watch_armed());
        }
    }

    #[test]
    fn completing_the_profile_disarms_the_heat_watch() {
        // Armed during a short hold far below target; the hold ends, the
        // profile completes, the controller idles and the watch is disarmed.
        let mut c = new_controller();
        c.sensor_mut().set_temp(100.0);
        let t = start(&mut c, Profile::new(&[Step::hold(500.0, 120.0)]).unwrap());
        c.iterate(None, t * 1_000, t as i64);
        assert!(c.heat_watch_armed());
        let mut s = t + 1;
        while c.state() == KilnState::Running && s < 600 {
            c.iterate(None, s * 1_000, s as i64);
            s += 1;
        }
        // Complete on the tick the hold ends (already disarmed), Idle on the next.
        assert_eq!(c.state(), KilnState::Complete, "profile completed");
        assert!(!c.heat_watch_armed());
        c.iterate(None, s * 1_000, s as i64);
        assert_eq!(c.state(), KilnState::Idle);
        assert!(!c.heat_watch_armed());
    }

    /// Run a 1000 C hold with the reading stuck at 0 C until the heat watch
    /// trips; returns the controller and the next free tick.
    fn tripped_controller() -> (Controller<MockSensor, MockSsr, CountingWatchdog>, u64) {
        let mut c = new_controller();
        c.sensor_mut().set_temp(0.0);
        let mut s = start(&mut c, Profile::new(&[Step::hold(1000.0, 4.0 * 3600.0)]).unwrap());
        while c.state() != KilnState::Error {
            c.iterate(None, s * 1_000, s as i64);
            s += 1;
            assert!(s < 2_000);
        }
        (c, s)
    }

    #[test]
    fn an_error_must_be_cleared_before_anything_heats_again() {
        let (mut c, mut s) = tripped_controller();
        let profile = Profile::new(&[Step::hold(1000.0, 3600.0)]).unwrap();
        let cmds = [
            Command::RunProfile {
                profile: name("again.json"),
                parsed: profile.clone(),
            },
            Command::ResumeProfile {
                profile: name("again.json"),
                parsed: profile.clone(),
                elapsed_seconds: 10.0,
                last_logged_temp: None,
                current_temp: None,
                step_index: Some(0),
            },
            Command::ScheduleProfile {
                profile: name("again.json"),
                parsed: profile.clone(),
                start_time: s + 5,
            },
            Command::StartTuning {
                mode: TuningMode::Safe,
                max_temp: None,
            },
        ];
        for cmd in cmds {
            c.iterate(Some(cmd), s * 1_000, s as i64);
            assert_eq!(c.state(), KilnState::Error);
            assert!(matches!(
                c.snapshot(s * 1_000, s as i64).error,
                Some(KilnError::NoHeatResponse { .. })
            ));
            assert!(c.snapshot(s * 1_000, s as i64).scheduled.is_none());
            s += 1;
        }
        // Acknowledged: Run works again.
        c.iterate(Some(Command::ClearError), s * 1_000, s as i64);
        assert_eq!(c.state(), KilnState::Idle);
        s += 1;
        c.iterate(
            Some(Command::RunProfile {
                profile: name("again.json"),
                parsed: profile,
            }),
            s * 1_000,
            s as i64,
        );
        assert_eq!(c.state(), KilnState::Running);
    }

    // ---- K2: firing lock on an untrusted config ------------------------

    fn locked_controller() -> Controller<MockSensor, MockSsr, CountingWatchdog> {
        Controller::new(
            MockSensor::new(25.0),
            MockSsr::new(),
            CountingWatchdog::new(),
            ControlParams {
                firing_lock: Some(kiln_core::state::FiringLock::ConfigMissing),
                ..ControlParams::default()
            },
            0,
        )
    }

    #[test]
    fn firing_lock_refuses_every_heating_command() {
        let mut c = locked_controller();
        let lock = Some(KilnError::FiringLocked(
            kiln_core::state::FiringLock::ConfigMissing,
        ));
        assert_eq!(c.snapshot(0, 0).error, lock, "lock visible from boot");
        let profile = Profile::new(&[Step::ramp(200.0, Some(100.0), None)]).unwrap();
        let cmds = [
            Command::RunProfile {
                profile: name("a.json"),
                parsed: profile.clone(),
            },
            Command::ResumeProfile {
                profile: name("a.json"),
                parsed: profile.clone(),
                elapsed_seconds: 10.0,
                last_logged_temp: Some(25.0),
                current_temp: Some(25.0),
                step_index: Some(0),
            },
            Command::StartTuning {
                mode: TuningMode::Safe,
                max_temp: None,
            },
            Command::ScheduleProfile {
                profile: name("a.json"),
                parsed: profile,
                start_time: 5,
            },
            Command::ClearError,
            Command::Stop,
        ];
        let mut s = 1u64;
        for cmd in cmds {
            let o = c.iterate(Some(cmd), s * 1_000, s as i64);
            assert!(!o.faulted);
            assert_eq!(c.state(), KilnState::Error);
            assert_eq!(c.snapshot(s * 1_000, s as i64).error, lock);
            assert_eq!(c.snapshot(s * 1_000, s as i64).ssr_output, 0.0);
            s += 1;
        }
        // The (refused) schedule never fires either.
        for _ in 0..10 {
            c.iterate(None, s * 1_000, s as i64);
            assert_eq!(c.state(), KilnState::Error);
            s += 1;
        }
        // Still reading temperature and feeding the watchdog while locked.
        assert!(c.watchdog().feeds >= 15);
    }

    #[test]
    fn a_sensor_fault_does_not_mask_the_firing_lock() {
        let mut c = locked_controller();
        let lock = Some(KilnError::FiringLocked(
            kiln_core::state::FiringLock::ConfigMissing,
        ));
        c.sensor_mut().set_fault(true);
        for s in 1..5u64 {
            c.iterate(None, s * 1_000, s as i64);
            assert_eq!(c.state(), KilnState::Error);
            assert_eq!(c.snapshot(s * 1_000, s as i64).error, lock);
        }
    }

    #[test]
    fn core1_refuses_an_out_of_bounds_profile() {
        // A rate-0 ramp (never ends) and an above-MAX_TEMP target are refused
        // even if they reach Core 1 without Core 0's validation.
        for profile in [
            Profile::new(&[Step::ramp(600.0, Some(0.0), None)]).unwrap(),
            Profile::new(&[Step::ramp(5000.0, Some(100.0), None)]).unwrap(),
        ] {
            let mut c = new_controller();
            c.iterate(None, 500, 0);
            c.iterate(
                Some(Command::RunProfile {
                    profile: name("bad.json"),
                    parsed: profile,
                }),
                1_000,
                1,
            );
            assert_eq!(c.state(), KilnState::Idle);
        }
    }

    #[test]
    fn scheduled_profile_fires_when_due() {
        let mut c = new_controller();
        let profile = Profile::new(&[Step::hold(100.0, 10_000.0)]).unwrap();
        c.iterate(
            Some(Command::ScheduleProfile {
                profile: name("sched.json"),
                parsed: profile,
                start_time: 5_000,
            }),
            1_000,
            1_000,
        );
        assert_eq!(c.state(), KilnState::Idle);

        let snap = c.snapshot(2_000, 4_000);
        let s = snap.scheduled.expect("scheduled snapshot present");
        assert_eq!(s.profile.as_str(), "sched.json");
        assert_eq!(s.seconds_until_start, 1_000);

        // Before the start time the kiln stays idle.
        c.iterate(None, 2_000, 4_000);
        assert_eq!(c.state(), KilnState::Idle);

        // At/after the start time the scheduler hands the profile to the loop.
        c.iterate(None, 3_000, 6_000);
        assert_eq!(c.state(), KilnState::Running);
        assert_eq!(
            c.snapshot(3_000, 6_000).profile_name.unwrap().as_str(),
            "sched.json"
        );
    }
}
