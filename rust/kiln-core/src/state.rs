//! Kiln firing state machine — port of `kiln/state.py` (`KilnController`).
//!
//! Two faithful-but-deliberate departures from the MicroPython version:
//!
//! * **Time is injected** (`now: f64` seconds) instead of `time.time()`, so the
//!   elapsed-time accumulation — including the NTP-jump guard — is deterministic
//!   and host-testable.
//! * **Errors are a typed [`KilnError`]** rather than strings; the human message
//!   is presentation and lives in the firmware/web layer.
//!
//! Everything else (step sequencing, ramp/hold/cooling targets, stall detection,
//! crash recovery) mirrors the reference branch-for-branch.

use crate::profile::{Profile, Step, StepKind};
use crate::rate_monitor::TempHistory;

/// `|x|` without `std`/libm.
#[inline]
fn abs(x: f32) -> f32 {
    if x < 0.0 {
        -x
    } else {
        x
    }
}

/// Temperature-loss tolerance (°C) for recovery detection — `TEMP_LOSS_THRESHOLD`.
pub const TEMP_LOSS_THRESHOLD: f32 = 5.0;
/// Start temperature assumed by `_find_step_for_elapsed` (note: differs from the
/// duration estimator, which seeds from `steps[0].target_temp`).
const FIND_START_TEMP: f32 = 20.0;
const RATE_HISTORY_CAP: usize = 60;

/// Controller state. Integer values match the reference `KilnState` constants
/// (IDLE=0 … ERROR=4) so traces can be compared directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KilnState {
    Idle,
    Running,
    Tuning,
    Complete,
    Error,
}

/// Typed fault reason (replaces the reference's error strings).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KilnError {
    MaxTempExceeded {
        temp: f32,
        max: f32,
    },
    NoActiveProfile,
    Stall {
        actual_rate: f32,
        min_rate: f32,
    },
    /// Emergency shutdown: too many consecutive sensor faults
    /// (`temp_filter::TempError::EmergencyShutdown`). Mirrors the reference's
    /// control-loop `except` path raising out of `TemperatureSensor.read()`.
    SensorFault,
    /// A sensor fault arrived before any valid reading
    /// (`temp_filter::TempError::NotInitialized`).
    SensorNotInitialized,
}

/// Safety/rate-control parameters (from `config.py`, with the same defaults as
/// `config.example.py`).
#[derive(Debug, Clone, Copy)]
pub struct ControllerConfig {
    pub max_temp: f32,
    pub rate_measurement_window: f32,
    pub rate_recording_interval: f32,
    pub stall_check_interval: f32,
    pub stall_consecutive_fails: u32,
    pub stall_min_step_time: f32,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            max_temp: 1300.0,
            rate_measurement_window: 600.0,
            rate_recording_interval: 10.0,
            stall_check_interval: 60.0,
            stall_consecutive_fails: 3,
            stall_min_step_time: 600.0,
        }
    }
}

/// The firing state machine. Construct with [`KilnController::new`], drive with
/// [`KilnController::update`].
#[derive(Debug, Clone)]
pub struct KilnController {
    pub state: KilnState,
    profile: Option<Profile>,
    // Absolute monotonic-seconds anchors stay f64: only their *difference* (a
    // small `dt`) is ever used, and that difference is narrowed to f32 in
    // `get_elapsed_time`. Keeping the anchors f64 avoids the catastrophic
    // cancellation a large f32 timestamp would suffer.
    start_time: Option<f64>,
    elapsed_offset: f32,
    last_update_time: Option<f64>,

    pub current_temp: f32,
    pub target_temp: f32,
    pub ssr_output: f32,

    cfg: ControllerConfig,

    current_step_index: usize,
    step_start_time: f32,
    step_start_temp: f32,

    temp_history: TempHistory<RATE_HISTORY_CAP>,
    last_temp_recording: f32,
    last_stall_check: f32,
    stall_fail_count: u32,

    error: Option<KilnError>,

    recovery_target_temp: Option<f32>,
}

impl KilnController {
    /// Create an idle controller with the given config.
    pub fn new(cfg: ControllerConfig) -> Self {
        Self {
            state: KilnState::Idle,
            profile: None,
            start_time: None,
            elapsed_offset: 0.0,
            last_update_time: None,
            current_temp: 0.0,
            target_temp: 0.0,
            ssr_output: 0.0,
            cfg,
            current_step_index: 0,
            step_start_time: 0.0,
            step_start_temp: 0.0,
            temp_history: TempHistory::new(),
            last_temp_recording: 0.0,
            last_stall_check: 0.0,
            stall_fail_count: 0,
            error: None,
            recovery_target_temp: None,
        }
    }

    // ---- accessors -------------------------------------------------------

    pub fn current_step_index(&self) -> usize {
        self.current_step_index
    }
    /// Accumulated elapsed run time (seconds). Already advanced by
    /// [`update`](Self::update) each tick, so reading it for a status snapshot is
    /// idempotent (unlike re-calling the reference's `get_elapsed_time`).
    pub fn elapsed(&self) -> f32 {
        self.elapsed_offset
    }
    /// Elapsed time at which the current step began (seconds).
    pub fn step_start_time(&self) -> f32 {
        self.step_start_time
    }
    /// The active firing profile, if any (for status: step count/kind/rate).
    pub fn active_profile(&self) -> Option<&Profile> {
        self.profile.as_ref()
    }
    pub fn is_recovering(&self) -> bool {
        self.recovery_target_temp.is_some()
    }
    pub fn error(&self) -> Option<KilnError> {
        self.error
    }
    pub fn recovery_target_temp(&self) -> Option<f32> {
        self.recovery_target_temp
    }
    /// Measured rate over the configured window (°C/h).
    pub fn measured_rate(&self) -> f32 {
        self.temp_history.get_rate(self.cfg.rate_measurement_window)
    }

    // ---- lifecycle -------------------------------------------------------

    /// Start running `profile` at time `now`. Mirrors `run_profile`.
    pub fn run_profile(&mut self, profile: Profile, now: f64) -> bool {
        if matches!(self.state, KilnState::Running | KilnState::Tuning) {
            return false;
        }
        self.profile = Some(profile);
        self.state = KilnState::Running;
        self.start_time = Some(now);
        self.elapsed_offset = 0.0;
        self.last_update_time = None;
        self.error = None;
        self.current_step_index = 0;
        self.step_start_time = 0.0;
        self.step_start_temp = self.current_temp;
        self.temp_history.clear();
        self.last_temp_recording = 0.0;
        self.last_stall_check = 0.0;
        self.stall_fail_count = 0;
        true
    }

    /// Stop and return to idle. Mirrors `stop` -> `_reset_to_idle`.
    pub fn stop(&mut self) {
        self.reset_to_idle();
    }

    fn reset_to_idle(&mut self) {
        self.state = KilnState::Idle;
        self.profile = None;
        self.target_temp = 0.0;
        self.start_time = None;
        self.elapsed_offset = 0.0;
        self.last_update_time = None;
        self.error = None;
        self.current_step_index = 0;
        self.step_start_time = 0.0;
        self.step_start_temp = 0.0;
        self.recovery_target_temp = None;
        self.temp_history.clear();
        self.last_temp_recording = 0.0;
        self.last_stall_check = 0.0;
        self.stall_fail_count = 0;
    }

    /// Set the ERROR state with a typed reason. Mirrors `set_error`.
    pub fn set_error(&mut self, err: KilnError) {
        self.state = KilnState::Error;
        self.error = Some(err);
        self.target_temp = 0.0;
    }

    /// Clear an error back to idle. Returns `false` if not in ERROR.
    pub fn clear_error(&mut self) -> bool {
        if self.state != KilnState::Error {
            return false;
        }
        self.reset_to_idle();
        true
    }

    // ---- elapsed time (delta accumulation with NTP-jump guard) -----------

    fn get_elapsed_time(&mut self, now: f64) -> f32 {
        if self.start_time.is_none() {
            return 0.0;
        }
        match self.last_update_time {
            None => {
                self.last_update_time = Some(now);
                self.elapsed_offset
            }
            Some(last) => {
                // The subtraction is done in f64 (both anchors are large absolute
                // timestamps); only the small delta is narrowed to f32.
                let mut delta = now - last;
                if !(0.0..=60.0).contains(&delta) {
                    delta = 1.0; // NTP jump / stall: assume 1 s passed
                }
                self.last_update_time = Some(now);
                if self.recovery_target_temp.is_none() {
                    self.elapsed_offset += delta as f32;
                }
                self.elapsed_offset
            }
        }
    }

    // ---- main update -----------------------------------------------------

    /// Advance the state machine with the latest `current_temp` at time `now`,
    /// returning the target temperature for the PID. Mirrors `update`.
    pub fn update(&mut self, current_temp: f32, now: f64) -> f32 {
        self.current_temp = current_temp;

        if current_temp > self.cfg.max_temp {
            self.set_error(KilnError::MaxTempExceeded {
                temp: current_temp,
                max: self.cfg.max_temp,
            });
            return 0.0;
        }

        match self.state {
            KilnState::Running => self.update_running(now),
            _ => 0.0,
        }
    }

    fn update_running(&mut self, now: f64) -> f32 {
        if self.profile.is_none() {
            self.set_error(KilnError::NoActiveProfile);
            return 0.0;
        }

        let elapsed = self.get_elapsed_time(now);

        if elapsed - self.last_temp_recording >= self.cfg.rate_recording_interval {
            self.record_temp_for_rate(elapsed);
        }

        let n_steps = self.profile.as_ref().unwrap().step_count();
        if self.is_step_complete(elapsed) {
            if self.current_step_index >= n_steps - 1 {
                self.state = KilnState::Complete;
                self.target_temp = 0.0;
                return 0.0;
            }
            self.advance_to_next_step(elapsed);
        }

        // Copy the active step out (Step: Copy) so we can mutate self freely.
        let current_step = self.profile.as_ref().unwrap().steps()[self.current_step_index];

        // Recovery hold: stay at the recovery target until the temp catches up.
        if let Some(rec_target) = self.recovery_target_temp {
            if self.current_temp >= rec_target - 1.0 {
                self.recovery_target_temp = None;
                self.temp_history.clear();
                self.last_temp_recording = self.elapsed_offset;
                self.last_stall_check = self.elapsed_offset;
                // fall through to normal execution
            } else {
                self.target_temp = rec_target;
                return rec_target;
            }
        }

        // Stall detection (ramp steps only).
        let mut min_rate = current_step.min_rate;
        if current_step.kind == StepKind::Ramp && min_rate.is_none() {
            min_rate = Some(current_step.desired_rate_or_default() * 0.8);
        }
        if current_step.kind == StepKind::Ramp {
            if let Some(mr) = min_rate {
                if mr > 0.0 && elapsed - self.last_stall_check >= self.cfg.stall_check_interval {
                    self.last_stall_check = elapsed;
                    let time_in_step = elapsed - self.step_start_time;
                    if time_in_step >= self.cfg.stall_min_step_time {
                        let actual_rate =
                            self.temp_history.get_rate(self.cfg.rate_measurement_window);
                        if abs(actual_rate) < mr {
                            self.stall_fail_count += 1;
                            if self.stall_fail_count >= self.cfg.stall_consecutive_fails {
                                self.set_error(KilnError::Stall {
                                    actual_rate: abs(actual_rate),
                                    min_rate: mr,
                                });
                                return 0.0;
                            }
                        } else {
                            self.stall_fail_count = 0;
                        }
                    }
                }
            }
        }

        let target = self.get_step_target_temp(elapsed, &current_step);
        self.target_temp = target;
        target
    }

    fn is_step_complete(&self, elapsed: f32) -> bool {
        let prof = match &self.profile {
            Some(p) => p,
            None => return false,
        };
        if self.current_step_index >= prof.step_count() {
            return false;
        }
        let step = prof.steps()[self.current_step_index];
        let time_in_step = elapsed - self.step_start_time;

        match step.kind {
            StepKind::Hold => time_in_step >= step.duration.unwrap_or(0.0),
            StepKind::Ramp => {
                let target = step.target_temp.unwrap_or(0.0);
                if target > self.step_start_temp {
                    self.current_temp >= target
                } else {
                    self.current_temp <= target
                }
            }
            StepKind::Cooling => match step.target_temp {
                Some(t) => self.current_temp <= t,
                None => false,
            },
        }
    }

    fn advance_to_next_step(&mut self, elapsed: f32) {
        self.current_step_index += 1;
        self.step_start_time = elapsed;
        self.step_start_temp = self.current_temp;
        self.temp_history.clear();
        self.last_stall_check = elapsed;
        self.stall_fail_count = 0;
    }

    fn record_temp_for_rate(&mut self, elapsed: f32) {
        self.temp_history.add(elapsed, self.current_temp);
        self.last_temp_recording = elapsed;
    }

    fn get_step_target_temp(&self, elapsed: f32, step: &Step) -> f32 {
        match step.kind {
            StepKind::Hold => step.target_temp.unwrap_or(0.0),
            StepKind::Ramp => {
                let time_in_step = elapsed - self.step_start_time;
                let hours_in_step = time_in_step / 3600.0;
                let target = step.target_temp.unwrap_or(0.0);
                let temp_change = step.desired_rate_or_default() * hours_in_step;
                if target > self.step_start_temp {
                    let calc = self.step_start_temp + temp_change;
                    if calc < target {
                        calc
                    } else {
                        target
                    } // min(calc, target)
                } else {
                    let calc = self.step_start_temp - temp_change;
                    if calc > target {
                        calc
                    } else {
                        target
                    } // max(calc, target)
                }
            }
            StepKind::Cooling => 0.0,
        }
    }

    // ---- crash recovery --------------------------------------------------

    /// Resume an interrupted profile. Mirrors `resume_profile`; `step_index`
    /// overrides the calculated step when provided (from the CSV log).
    #[allow(clippy::too_many_arguments)]
    pub fn resume_profile(
        &mut self,
        profile: Profile,
        elapsed_seconds: f32,
        last_logged_temp: Option<f32>,
        current_temp: Option<f32>,
        step_index: Option<usize>,
        now: f64,
    ) -> bool {
        if matches!(self.state, KilnState::Running | KilnState::Tuning) {
            return false;
        }

        self.profile = Some(profile);
        self.state = KilnState::Running;
        self.start_time = Some(now);
        self.elapsed_offset = elapsed_seconds;
        self.last_update_time = None;
        self.error = None;

        let (calc_index, time_in_step, calc_start_temp) =
            self.find_step_for_elapsed(elapsed_seconds);
        self.current_step_index = step_index.unwrap_or(calc_index);
        self.step_start_time = elapsed_seconds - time_in_step;

        let current_step = self.profile.as_ref().unwrap().steps()[self.current_step_index];
        let ramp_llt = if current_step.kind == StepKind::Ramp && time_in_step > 0.0 {
            last_logged_temp
        } else {
            None
        };
        if let Some(llt) = ramp_llt {
            let rate = current_step.desired_rate_or_default();
            let hours_in_step = time_in_step / 3600.0;
            let temp_change = rate * hours_in_step;
            let target = current_step.target_temp.unwrap_or(0.0);
            self.step_start_temp = if target > llt {
                llt - temp_change
            } else {
                llt + temp_change
            };
        } else {
            self.step_start_temp = calc_start_temp;
        }

        self.temp_history.clear();
        self.last_temp_recording = elapsed_seconds;
        self.last_stall_check = elapsed_seconds;
        self.stall_fail_count = 0;

        if let (Some(llt), Some(cur)) = (last_logged_temp, current_temp) {
            let is_cooling = current_step.kind == StepKind::Cooling
                || (current_step.kind == StepKind::Ramp
                    && current_step.target_temp.unwrap_or(0.0) < self.step_start_temp);
            let temp_loss = llt - cur;
            if temp_loss > TEMP_LOSS_THRESHOLD && !is_cooling {
                self.recovery_target_temp = Some(llt);
                return true;
            }
            // temp_loss during cooling is expected — fall through, no recovery.
        }
        true
    }

    /// Estimate which step `elapsed_seconds` falls in. Mirrors
    /// `_find_step_for_elapsed` (note: seeds `profile_temp` from 20, unlike the
    /// duration estimator). Returns `(index, time_in_step, step_start_temp)`.
    fn find_step_for_elapsed(&self, elapsed_seconds: f32) -> (usize, f32, f32) {
        let prof = match &self.profile {
            Some(p) if p.step_count() > 0 => p,
            _ => return (0, 0.0, self.current_temp),
        };
        let steps = prof.steps();
        let mut cumulative_time = 0.0;
        let mut profile_temp = FIND_START_TEMP;

        for (i, step) in steps.iter().enumerate() {
            let step_duration = match step.kind {
                StepKind::Hold => step.duration.unwrap_or(0.0),
                StepKind::Ramp => {
                    let target = step.target_temp.unwrap_or(0.0);
                    let dtemp = abs(target - profile_temp);
                    let rate = step.desired_rate_or_default();
                    if rate > 0.0 {
                        (dtemp / rate) * 3600.0
                    } else {
                        0.0
                    }
                }
                StepKind::Cooling => match step.target_temp {
                    Some(target) => (abs(profile_temp - target) / 100.0) * 3600.0,
                    None => 0.0,
                },
            };

            if cumulative_time + step_duration >= elapsed_seconds {
                let time_in_step = elapsed_seconds - cumulative_time;
                return (i, time_in_step, profile_temp);
            }

            cumulative_time += step_duration;
            match step.kind {
                StepKind::Ramp => profile_temp = step.target_temp.unwrap_or(0.0),
                StepKind::Cooling => {
                    if let Some(target) = step.target_temp {
                        profile_temp = target;
                    }
                }
                StepKind::Hold => {}
            }
        }
        (steps.len() - 1, 0.0, profile_temp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Step;

    fn cfg() -> ControllerConfig {
        ControllerConfig::default()
    }

    #[test]
    fn max_temp_exceeded_sets_error_and_zero_output() {
        let mut c = KilnController::new(cfg());
        let out = c.update(1301.0, 0.0);
        assert_eq!(out, 0.0);
        assert_eq!(c.state, KilnState::Error);
        assert_eq!(
            c.error(),
            Some(KilnError::MaxTempExceeded {
                temp: 1301.0,
                max: 1300.0
            })
        );
    }

    #[test]
    fn cannot_start_while_running() {
        let mut c = KilnController::new(cfg());
        let p = Profile::new(&[Step::hold(100.0, 10.0)]).unwrap();
        assert!(c.run_profile(p.clone(), 0.0));
        assert!(!c.run_profile(p, 1.0));
    }

    #[test]
    fn hold_completes_after_duration_then_profile_completes() {
        let mut c = KilnController::new(cfg());
        // current_temp starts at 0 -> step_start_temp = 0 at run.
        c.current_temp = 100.0;
        let p = Profile::new(&[Step::hold(100.0, 30.0)]).unwrap();
        assert!(c.run_profile(p, 0.0));

        // t=0 first call: elapsed 0, hold target 100.
        let t0 = c.update(100.0, 0.0);
        assert_eq!(c.state, KilnState::Running);
        assert_eq!(t0, 100.0);

        // Advance past the 30 s hold duration.
        let _ = c.update(100.0, 20.0);
        let _ = c.update(100.0, 40.0); // elapsed ~40 > 30 -> complete (last step)
        assert_eq!(c.state, KilnState::Complete);
        assert_eq!(c.target_temp, 0.0);
    }

    #[test]
    fn ntp_forward_jump_is_clamped_to_one_second() {
        let mut c = KilnController::new(cfg());
        c.current_temp = 50.0;
        let p = Profile::new(&[Step::hold(50.0, 1000.0)]).unwrap();
        assert!(c.run_profile(p, 100.0));
        c.update(50.0, 100.0); // first call: elapsed 0, sets last_update_time
        c.update(50.0, 100.5); // delta 0.5 -> elapsed 0.5
        c.update(50.0, 5000.0); // delta huge -> clamped to 1.0 -> elapsed 1.5
                                // Not complete yet (hold is 1000 s), still running.
        assert_eq!(c.state, KilnState::Running);
    }

    #[test]
    fn clear_error_only_from_error_state() {
        let mut c = KilnController::new(cfg());
        assert!(!c.clear_error());
        c.set_error(KilnError::NoActiveProfile);
        assert!(c.clear_error());
        assert_eq!(c.state, KilnState::Idle);
    }
}
