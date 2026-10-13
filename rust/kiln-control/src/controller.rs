//! The synchronous control logic — a faithful port of `ControlThread`'s
//! `control_loop_iteration`, `tuning_loop_iteration`, and `handle_command`
//! (`kiln/control_thread.py`), with **all I/O behind injected traits** so the
//! exact same code runs on the RP2350 and under `cargo test` on the host.
//!
//! The fire-relevant ordering is preserved tick-for-tick (read → state → PID →
//! SSR → watchdog), and the watchdog is fed **only** on a clean iteration — an
//! emergency (sensor shutdown or tuning over-temp) forces the SSR off and skips
//! the feed, exactly as the reference's `except` / early-`return` paths do.
//!
//! Time is injected two ways, both integer, matching the safety analysis:
//! - `now_ms` (monotonic milliseconds, `u64`): SSR time-proportional schedule +
//!   the status cadence; also passed (as `i64` ms) to the PID `dt`, the
//!   state-machine elapsed accumulation, and the tuner — monotonic so an NTP
//!   step can never corrupt the control math, and integer so the hot path never
//!   touches the soft-float f64 path on the M33.
//! - `wall_s` (Unix wall-clock seconds, `i64`): the scheduler's absolute
//!   start-time comparison and the status `timestamp` only.

use kiln_core::gain_schedule::GainSchedule;
use kiln_core::heat_watch::{HeatWatch, NoHeatResponse};
use kiln_core::pid::Pid;
use kiln_core::profile::Profile;
use kiln_core::protocol::{Command, ProfileName, ScheduledSnapshot, Status, TuningSnapshot};
use kiln_core::scheduler::ScheduledProfileQueue;
use kiln_core::ssr_schedule::SsrSchedule;
use kiln_core::state::{KilnController, KilnError, KilnState};
use kiln_core::temp_filter::{TempError, TempFilter};
use kiln_core::tuner::{TuningStage, ZieglerNicholsTuner};
use kiln_hal::platform::{SsrOutput, TempSensor, Watchdog};

use crate::params::ControlParams;

/// Median-filter capacity (covers `TEMP_MEDIAN_WINDOW` up to 8; default 3).
const MEDIAN_CAP: usize = kiln_core::temp_filter::MAX_MEDIAN_WINDOW;

/// A scheduled profile waiting to start: its filename plus the Core-0-parsed
/// [`Profile`] (Core 1 never reads the filesystem — see [`Command::RunProfile`]).
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledItem {
    pub name: ProfileName,
    pub profile: Profile,
}

/// The result of one control iteration.
pub struct IterationOutcome {
    /// A status to publish this iteration, or `None`. The reference calls
    /// `send_status_update` at most once per iteration; this mirrors that.
    pub publish: Option<Status>,
    /// `true` if the iteration hit the emergency path (sensor emergency shutdown,
    /// or tuning over-temp): the SSR was forced off and the watchdog was **not**
    /// fed. The async runner then throttles (the reference's `sleep(1)`).
    pub faulted: bool,
}

/// The Core 1 controller: owns the drivers and every `kiln-core` decision
/// component, and advances them one tick at a time.
pub struct Controller<S, O, W> {
    sensor: S,
    ssr_out: O,
    wdt: W,
    params: ControlParams,

    state: KilnController,
    pid: Pid,
    gains: GainSchedule,
    filter: TempFilter<MEDIAN_CAP>,
    ssr_sched: SsrSchedule,
    scheduler: ScheduledProfileQueue<ScheduledItem>,
    tuner: Option<ZieglerNicholsTuner>,
    heat_watch: HeatWatch,

    current_profile: Option<ProfileName>,
    last_status_ms: u64,
    published: bool,
    last_temp_error: KilnError,
}

impl<S, O, W> Controller<S, O, W>
where
    S: TempSensor,
    O: SsrOutput,
    W: Watchdog,
{
    /// Build the controller from `params`, arming the watchdog and seeding the
    /// SSR cycle from `now_ms`. Mirrors `ControlThread.setup_hardware` (minus the
    /// concrete driver construction, which the firmware does and injects here).
    pub fn new(sensor: S, ssr_out: O, mut wdt: W, params: ControlParams, now_ms: u64) -> Self {
        wdt.start(params.watchdog_timeout_ms);
        let pid = Pid::new(
            params.pid_base.kp,
            params.pid_base.ki,
            params.pid_base.kd,
            0.0,
            100.0,
        );
        let gains = GainSchedule::new(params.pid_base, params.thermal_h, params.thermal_t_ambient);
        let filter = TempFilter::new(params.thermocouple_offset, params.median_window);
        let ssr_sched = SsrSchedule::new(params.ssr_cycle_time_s, now_ms);
        let mut state = KilnController::new(params.controller);
        // An untrusted boot configuration latches the lock error from the first
        // status on, so the UI shows why firing is refused.
        if let Some(lock) = params.firing_lock {
            state.set_error(KilnError::FiringLocked(lock));
        }
        Controller {
            sensor,
            ssr_out,
            wdt,
            state,
            pid,
            gains,
            filter,
            ssr_sched,
            scheduler: ScheduledProfileQueue::new(),
            tuner: None,
            heat_watch: HeatWatch::new(params.heat_watch),
            current_profile: None,
            last_status_ms: 0,
            published: false,
            last_temp_error: KilnError::SensorNotInitialized,
            params,
        }
    }

    // ---- inspection (sim / tests) ---------------------------------------

    #[cfg(test)]
    pub fn state(&self) -> KilnState {
        self.state.state
    }
    pub fn watchdog(&self) -> &W {
        &self.wdt
    }
    #[cfg(test)]
    pub fn ssr(&self) -> &O {
        &self.ssr_out
    }
    pub fn sensor_mut(&mut self) -> &mut S {
        &mut self.sensor
    }
    /// SSR sub-tick schedule for the async runner: `(count, period_ms)`, where
    /// `count = TEMP_READ_INTERVAL / SSR_UPDATE_INTERVAL` (the reference's
    /// `update_count`). At the defaults this is `(10, 100)` — 10 sub-ticks at
    /// 10 Hz spanning the 1 s outer tick.
    pub fn timing(&self) -> (u32, u64) {
        let sub = self.params.ssr_update_interval_ms.max(1);
        let count = (self.params.temp_read_interval_ms / sub) as u32;
        (count.max(1), sub)
    }

    // ---- the one-tick control logic -------------------------------------

    /// One outer (≈1 Hz) control iteration. `now_ms` is monotonic; `wall_s` is
    /// Unix wall-clock seconds. Returns what (if anything) to publish and whether
    /// the iteration faulted. Feeds the watchdog itself on a clean iteration.
    pub fn iterate(&mut self, cmd: Option<Command>, now_ms: u64, wall_s: i64) -> IterationOutcome {
        if self.state.state == KilnState::Tuning {
            return self.tuning_iterate(cmd, now_ms, wall_s);
        }
        let mono_ms = now_ms as i64;

        if let Some(c) = cmd {
            self.handle_command(c, now_ms, wall_s);
        }
        // Firing lock: `Stop` may have dropped the latched lock error back to
        // Idle; re-latch it (no heating command can be accepted while locked).
        if let Some(lock) = self.params.firing_lock {
            if self.state.state != KilnState::Error {
                self.state.set_error(KilnError::FiringLocked(lock));
            }
        }

        let mut pending: Option<Status> = None;

        if self.state.state == KilnState::Complete {
            pending = Some(self.publish_now(now_ms, wall_s));
            self.state.stop();
            self.current_profile = None;
        }

        if self.state.state == KilnState::Idle && self.scheduler.can_consume(wall_s) {
            if let Some(item) = self.scheduler.consume(wall_s) {
                if self.state.run_profile(item.profile.clone(), mono_ms) {
                    self.current_profile = Some(item.name);
                    self.pid.reset();
                }
            }
        }

        let temp = match self.read_temp() {
            Ok(t) => t,
            Err(()) => {
                self.ssr_sched.force_off();
                let _ = self.ssr_out.force_off();
                let was_error = self.state.state == KilnState::Error;
                // While firing is locked keep the lock as THE error: it is what
                // the operator must fix, and only Stop would otherwise restore it.
                if self.params.firing_lock.is_none() {
                    self.state.set_error(self.last_temp_error);
                }
                // Publish the Error transition (once). A faulted tick returns here,
                // before the normal status_due check, so without this the Error is
                // never published: Core 0 never logs the terminal ERROR row and
                // never clears the recovery pointer, leaving the last CSV row stuck
                // at RUNNING. Republishing every faulted tick would only spam the
                // logger, so suppress once already in Error.
                let publish = if was_error {
                    pending
                } else {
                    Some(self.publish_now(now_ms, wall_s))
                };
                return IterationOutcome {
                    publish,
                    faulted: true,
                };
            }
        };

        // Snapshot the error state before `state.update` so a clean→Error
        // transition this tick (e.g. over-temp / MaxTempExceeded) can be force-
        // published below, even outside the status_due window.
        let was_error = self.state.state == KilnState::Error;
        let target = self.state.update(temp, mono_ms);

        let mut ssr_output = if self.state.state == KilnState::Running {
            if let Some(g) = self.gains.update(temp) {
                self.pid.set_gains(Some(g.kp), Some(g.ki), Some(g.kd));
            }
            self.pid.update(target, temp, mono_ms)
        } else {
            self.pid.reset();
            0.0
        };

        // Heating-response watchdog (Rust-only): full power, short of the
        // setpoint, and the reading is not rising → the thermocouple reads low
        // or the elements are dead. Covers holds and the recovery hold, which
        // the ramp-only stall check never looks at.
        if self.state.state == KilnState::Running {
            if let Err(e) = self
                .heat_watch
                .update(mono_ms, temp, ssr_output, Some(target))
            {
                self.state.set_error(no_heat_error(e));
                self.heat_watch.reset();
                self.pid.reset();
                ssr_output = 0.0;
            }
        } else {
            self.heat_watch.reset();
        }

        self.state.ssr_output = ssr_output;
        self.ssr_sched.set_output(ssr_output);
        if self.state.state == KilnState::Error {
            self.ssr_sched.force_off();
            let _ = self.ssr_out.force_off();
        }

        // A clean→Error transition (over-temp from `state.update`) must publish
        // once so Core 0 logs the terminal ERROR row and fixes the recovery
        // pointer — otherwise it only surfaces if it happens to land in the
        // status_due window. `faulted` stays false: over-temp is soft-recoverable
        // (sensor OK, relay already forced off above, operator clears on cooldown).
        let entered_error = !was_error && self.state.state == KilnState::Error;
        if pending.is_none() && (entered_error || self.status_due(now_ms)) {
            pending = Some(self.publish_now(now_ms, wall_s));
        }

        self.wdt.feed();
        IterationOutcome {
            publish: pending,
            faulted: false,
        }
    }

    fn tuning_iterate(
        &mut self,
        cmd: Option<Command>,
        now_ms: u64,
        wall_s: i64,
    ) -> IterationOutcome {
        let mono_ms = now_ms as i64;

        if let Some(c) = cmd {
            self.handle_command(c, now_ms, wall_s);
        }
        if self.state.state != KilnState::Tuning || self.tuner.is_none() {
            self.wdt.feed();
            return IterationOutcome {
                publish: None,
                faulted: false,
            };
        }

        let temp = match self.read_temp() {
            Ok(t) => t,
            Err(()) => {
                self.ssr_sched.force_off();
                let _ = self.ssr_out.force_off();
                self.state.set_error(self.last_temp_error);
                self.tuner = None;
                // Publish the Error so Core 0 logs it (this is a Tuning→Error
                // transition, so it is always the first Error tick — see iterate).
                return IterationOutcome {
                    publish: Some(self.publish_now(now_ms, wall_s)),
                    faulted: true,
                };
            }
        };
        self.state.current_temp = temp;

        if temp > self.params.controller.max_temp {
            self.state.set_error(KilnError::MaxTempExceeded {
                temp,
                max: self.params.controller.max_temp,
            });
            self.tuner = None;
            self.ssr_sched.force_off();
            let _ = self.ssr_out.force_off();
            // Publish the over-temp Error so Core 0 logs the terminal row.
            return IterationOutcome {
                publish: Some(self.publish_now(now_ms, wall_s)),
                faulted: true,
            };
        }

        let (ssr_output, continue_tuning) = self.tuner.as_mut().unwrap().update(temp, mono_ms);

        // Heating-response watchdog: the tuner runs open loop at fixed duty, so
        // nothing else notices a reading that stays low at 100 % power.
        if continue_tuning {
            if let Err(e) = self.heat_watch.update(mono_ms, temp, ssr_output, None) {
                self.state.set_error(no_heat_error(e));
                self.state.ssr_output = 0.0;
                self.tuner = None;
                self.heat_watch.reset();
                self.ssr_sched.force_off();
                let _ = self.ssr_out.force_off();
                let publish = Some(self.publish_now(now_ms, wall_s));
                self.wdt.feed();
                return IterationOutcome {
                    publish,
                    faulted: false,
                };
            }
        }

        self.state.ssr_output = ssr_output;
        self.state.target_temp = self
            .tuner
            .as_ref()
            .unwrap()
            .step_target_temp()
            .unwrap_or(0.0);
        self.ssr_sched.set_output(ssr_output);

        if !continue_tuning {
            let max = self.tuner.as_ref().unwrap().max_temp;
            match self.tuner.as_ref().unwrap().stage() {
                TuningStage::Complete => self.state.state = KilnState::Idle,
                TuningStage::Error => self
                    .state
                    .set_error(KilnError::MaxTempExceeded { temp, max }),
                TuningStage::Running => {}
            }
            self.tuner = None;
            self.ssr_sched.force_off();
            let _ = self.ssr_out.force_off();
        }

        let publish = if self.status_due(now_ms) {
            Some(self.publish_now(now_ms, wall_s))
        } else {
            None
        };

        self.wdt.feed();
        IterationOutcome {
            publish,
            faulted: false,
        }
    }

    /// One 10 Hz SSR sub-tick: advance the time-proportional schedule and apply
    /// the ON/OFF to the relay(s). Returns the relay state. Mirrors the inner
    /// `for _ in range(update_count): ssr.update(); sleep(0.1)` loop.
    pub fn ssr_subtick(&mut self, now_ms: u64) -> Result<bool, O::Error> {
        let on = self.ssr_sched.update(now_ms);
        self.ssr_out.apply(on, now_ms)?;
        Ok(on)
    }

    /// Immediately de-energise the relay(s) and zero the duty — the EMERGENCY
    /// path (sensor fault, over-temp, stop). Zeroing the schedule keeps the
    /// relay off until a fresh `set_output`.
    pub fn force_ssr_off(&mut self) -> Result<(), O::Error> {
        self.ssr_sched.force_off();
        self.ssr_out.force_off()
    }

    /// De-energise the relay(s) for a TRANSIENT pause (the firmware's flash-write
    /// handshake) WITHOUT touching the duty schedule. The next
    /// [`ssr_subtick`](Self::ssr_subtick) after the pause re-applies the scheduled
    /// state, so heating resumes immediately.
    ///
    /// Using [`force_ssr_off`](Self::force_ssr_off) here instead would zero the
    /// *locked* duty, which only re-latches at the next SSR cycle boundary — up to
    /// a full `SSR_CYCLE_TIME` (20 s) of unintended relay-off per flash flush,
    /// phase-locked to the flush cadence (observed on hardware as a ~15 s SSR
    /// dropout every 120 s at 100% output).
    pub fn pause_ssr_off(&mut self) -> Result<(), O::Error> {
        self.ssr_out.force_off()
    }

    /// Build a fresh status snapshot for the current state (`wall_s` clock).
    pub fn snapshot(&self, now_ms: u64, wall_s: i64) -> Status {
        self.build_status(now_ms, wall_s)
    }

    // ---- command handling -----------------------------------------------

    fn handle_command(&mut self, cmd: Command, _now_ms: u64, wall_s: i64) {
        let mono_ms = _now_ms as i64;
        // Locked: refuse everything that could heat, and the error clear that
        // would otherwise hide the lock. Stop/Shutdown/Ping/CancelScheduled stay.
        if self.params.firing_lock.is_some()
            && matches!(
                cmd,
                Command::RunProfile { .. }
                    | Command::ResumeProfile { .. }
                    | Command::StartTuning { .. }
                    | Command::ScheduleProfile { .. }
                    | Command::ClearError
            )
        {
            return;
        }
        // An error must be acknowledged (ClearError, or Stop) before anything can
        // heat again: otherwise one Run after a NoHeatResponse / over-temp /
        // stall would restart the firing with the fault unexamined. (Rust-only;
        // the reference's run_profile accepted ERROR.) Core 0 answers 409 first.
        if self.state.state == KilnState::Error
            && matches!(
                cmd,
                Command::RunProfile { .. }
                    | Command::ResumeProfile { .. }
                    | Command::StartTuning { .. }
                    | Command::ScheduleProfile { .. }
            )
        {
            return;
        }
        // Defense in depth: Core 0 validates every profile it sends (upload,
        // run, schedule, recovery), but Core 1 never trusts it to have done so.
        let max_temp = self.params.controller.max_temp;
        if let Command::RunProfile { parsed, .. }
        | Command::ResumeProfile { parsed, .. }
        | Command::ScheduleProfile { parsed, .. } = &cmd
        {
            if parsed.validate(max_temp).is_err() {
                return;
            }
        }
        match cmd {
            Command::RunProfile { profile, parsed } => {
                if matches!(self.state.state, KilnState::Running | KilnState::Tuning) {
                    return;
                }
                if self.state.run_profile(parsed, mono_ms) {
                    self.current_profile = Some(profile);
                    self.pid.reset();
                    self.heat_watch.reset();
                }
            }
            Command::ResumeProfile {
                profile,
                parsed,
                elapsed_seconds,
                last_logged_temp,
                current_temp,
                step_index,
            } => {
                if self.state.resume_profile(
                    parsed,
                    elapsed_seconds,
                    last_logged_temp,
                    current_temp,
                    step_index,
                    mono_ms,
                ) {
                    self.current_profile = Some(profile);
                    self.pid.reset();
                    self.heat_watch.reset();
                }
            }
            Command::Stop | Command::Shutdown => {
                self.state.stop();
                self.ssr_sched.force_off();
                let _ = self.ssr_out.force_off();
                self.current_profile = None;
            }
            Command::StartTuning { mode, max_temp } => {
                if self.state.state != KilnState::Idle {
                    return;
                }
                let mut t = ZieglerNicholsTuner::new(mode, max_temp);
                t.start(mono_ms);
                self.tuner = Some(t);
                self.heat_watch.reset();
                self.state.state = KilnState::Tuning;
            }
            Command::StopTuning => {
                if self.state.state == KilnState::Tuning {
                    self.state.state = KilnState::Idle;
                    self.tuner = None;
                    self.ssr_sched.force_off();
                    let _ = self.ssr_out.force_off();
                }
            }
            Command::ScheduleProfile {
                profile,
                parsed,
                start_time,
            } => {
                let item = ScheduledItem {
                    name: profile,
                    profile: parsed,
                };
                self.scheduler.schedule(item, start_time as i64, wall_s);
            }
            Command::CancelScheduled => {
                self.scheduler.cancel();
            }
            Command::ClearError => {
                if self.state.state == KilnState::Error && self.state.clear_error() {
                    self.ssr_sched.force_off();
                    let _ = self.ssr_out.force_off();
                    self.pid.reset();
                    self.current_profile = None;
                }
            }
            Command::Ping => {}
        }
    }

    // ---- helpers ---------------------------------------------------------

    /// Whether the heating-response watchdog currently has a window open.
    pub fn heat_watch_armed(&self) -> bool {
        self.heat_watch.armed()
    }

    /// Read the thermocouple with the reference's fault tolerance: check the
    /// fault register first (a bus error counts as a fault), else read the
    /// temperature, feeding either through the median/fault filter. `Err(())`
    /// is the emergency shutdown (the filter exhausted its fault budget), with
    /// the typed reason stashed in `last_temp_error`.
    fn read_temp(&mut self) -> Result<f32, ()> {
        let faulted = self.sensor.has_fault().unwrap_or(true);
        let result = if faulted {
            self.filter.push_fault()
        } else {
            match self.sensor.read_temperature() {
                Ok(raw) => self.filter.push_reading(raw),
                Err(_) => self.filter.push_fault(),
            }
        };
        match result {
            Ok(t) => Ok(t),
            Err(TempError::EmergencyShutdown) => {
                self.last_temp_error = KilnError::SensorFault;
                Err(())
            }
            Err(TempError::NotInitialized) => {
                self.last_temp_error = KilnError::SensorNotInitialized;
                Err(())
            }
        }
    }

    fn status_due(&self, now_ms: u64) -> bool {
        !self.published
            || now_ms.saturating_sub(self.last_status_ms) >= self.params.status_update_interval_ms
    }

    fn publish_now(&mut self, now_ms: u64, wall_s: i64) -> Status {
        self.published = true;
        self.last_status_ms = now_ms;
        self.build_status(now_ms, wall_s)
    }

    fn build_status(&self, now_ms: u64, wall_s: i64) -> Status {
        let mono_ms = now_ms as i64;
        let elapsed = self.state.elapsed();

        let mut step_index = None;
        let mut step_kind = None;
        let mut total_steps = None;
        let mut desired_rate = 0.0;
        let mut step_elapsed = 0.0;
        if let Some(profile) = self.state.active_profile() {
            total_steps = Some(profile.step_count());
            let idx = self.state.current_step_index();
            step_index = Some(idx);
            if idx < profile.step_count() {
                let step = profile.steps()[idx];
                step_kind = Some(step.kind);
                desired_rate = step.desired_rate.unwrap_or(0.0);
                step_elapsed = elapsed - self.state.step_start_time();
            }
        }

        let scheduled = self.scheduler.status(wall_s).map(|s| ScheduledSnapshot {
            profile: s.payload.name,
            start_time: s.start_time.max(0) as u64,
            seconds_until_start: s.seconds_until_start,
        });

        let tuning = self.tuner.as_ref().map(|t| TuningSnapshot {
            stage: t.stage(),
            mode: t.mode,
            max_temp: t.max_temp,
            step_index: t.current_step_index(),
            total_steps: t.total_steps(),
            step_elapsed: t.step_elapsed(mono_ms),
            ssr_percent: t.step_ssr_percent(),
            target_temp: t.step_target_temp(),
            timeout: t.step_timeout(),
            plateau_detected: t.step_plateau_detected(),
            peak_temp: t.step_peak_temp(),
        });

        // Status is f32 throughout except `timestamp`, which is wall-clock epoch
        // seconds as an integer (`i64`) — f32 ulp at ~1.7e9 is 128 s, so it can
        // never be f32; integer seconds keep it exact and off the soft-float path.
        Status {
            timestamp: wall_s,
            state: self.state.state,
            current_temp: self.state.current_temp,
            target_temp: self.state.target_temp,
            ssr_output: self.state.ssr_output,
            elapsed,
            error: self.state.error(),
            step_index,
            step_kind,
            total_steps,
            desired_rate,
            step_elapsed,
            is_recovering: self.state.is_recovering(),
            recovery_target_temp: self.state.recovery_target_temp(),
            stall_advances: self.state.stall_advances(),
            measured_rate: self.state.measured_rate(),
            profile_name: self.current_profile,
            scheduled,
            tuning,
        }
    }
}

/// The typed error for a failed heating-response window.
fn no_heat_error(e: NoHeatResponse) -> KilnError {
    KilnError::NoHeatResponse {
        rise: e.rise,
        window_s: e.window_s,
        avg_output: e.avg_output,
    }
}
