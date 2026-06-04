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
    use kiln_core::state::KilnState;
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
