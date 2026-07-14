//! Kiln firing state machine — port of `kiln/state.py` (`KilnController`).
//!
//! Two faithful-but-deliberate departures from the MicroPython version:
//!
//! * **Time is injected** (`now_ms: i64` monotonic milliseconds) instead of
//!   `time.time()`, so the elapsed-time accumulation — including the NTP-jump
//!   guard — is deterministic and host-testable. The anchors are integer ms (no
//!   soft-float on the M33); the guard still uses a *signed* delta so an
//!   out-of-order step clamps exactly as the f64 reference did.
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

/// Arrival band (°C): within this margin of a ramp's final target the kiln is
/// *arriving*, not stalling — the PID throttles on approach, the measured rate
/// over the window legitimately collapses, and the reference's stall check
/// would fault 1 °C short of the step boundary (seen on hardware at 879/880).
pub const STALL_ARRIVAL_BAND: f32 = 5.0;
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
    /// Fraction of a ramp's `desired_rate` used as the fallback minimum rate when
    /// the step omits an explicit `min_rate`. A step running slower than this for
    /// [`stall_consecutive_fails`](Self::stall_consecutive_fails) checks faults as
    /// a stall. `0.0` disables the fallback check.
    pub stall_rate_ratio: f32,
    /// SSR output (%) at or above which the loop counts as saturated for the
    /// stall gate: a kiln can only be *stalled* when the SSR is already giving
    /// everything it has. Below this, a slow rate means the PID is deliberately
    /// limiting power (integral still winding up, rate control), not a stall.
    pub stall_saturation_output: f32,
    /// How long (seconds) the SSR must have been continuously saturated before a
    /// slow rate may count as a stall failure. `0.0` disables the saturation
    /// gate (the reference Python behaviour — used by the golden replays).
    pub stall_saturation_window: f32,
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
            stall_rate_ratio: 0.8,
            stall_saturation_output: 95.0,
            stall_saturation_window: 300.0,
        }
    }
}

/// The firing state machine. Construct with [`KilnController::new`], drive with
/// [`KilnController::update`].
#[derive(Debug, Clone)]
pub struct KilnController {
    pub state: KilnState,
    profile: Option<Profile>,
    // Absolute monotonic-millisecond anchors (integer): only their *difference*
    // (a small `dt`) is ever used, narrowed to f32 in `get_elapsed_time`. Integer
    // ms keeps the subtraction exact and off the soft-float path; it also dodges
    // the catastrophic cancellation a large f32 timestamp would suffer.
    start_time: Option<i64>,
    elapsed_offset: f32,
    last_update_time: Option<i64>,

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
    /// Elapsed second at which the SSR output last *entered* saturation
    /// (≥ [`ControllerConfig::stall_saturation_output`]); `None` while below.
    saturated_since: Option<f32>,

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
            saturated_since: None,
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

    /// Start running `profile` at time `now_ms` (monotonic ms). Mirrors `run_profile`.
    pub fn run_profile(&mut self, profile: Profile, now_ms: i64) -> bool {
        if matches!(self.state, KilnState::Running | KilnState::Tuning) {
            return false;
        }
        self.profile = Some(profile);
        self.state = KilnState::Running;
        self.start_time = Some(now_ms);
        self.elapsed_offset = 0.0;
        self.last_update_time = None;
        self.error = None;
        // Skip steps the kiln has already climbed past: a fresh launch at 683 °C
        // should join the step whose band still lies ahead, not re-run (and try
        // to cool down through) earlier ramps/holds. On a cold start this lands
        // on step 0, preserving the previous behaviour.
        self.current_step_index = self.find_step_for_temp(self.current_temp);
        self.step_start_time = 0.0;
        self.step_start_temp = self.current_temp;
        self.temp_history.clear();
        self.last_temp_recording = 0.0;
        self.last_stall_check = 0.0;
        self.stall_fail_count = 0;
        self.saturated_since = None;
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
        self.saturated_since = None;
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

    fn get_elapsed_time(&mut self, now_ms: i64) -> f32 {
        if self.start_time.is_none() {
            return 0.0;
        }
        match self.last_update_time {
            None => {
                self.last_update_time = Some(now_ms);
                self.elapsed_offset
            }
            Some(last) => {
                // Signed integer subtraction (exact); the small delta is narrowed
                // to f32 seconds. The guard rejects a negative (out-of-order /
                // backward) or >60 s (NTP forward jump / stall) step, exactly as
                // the f64 reference did.
                let mut delta = (now_ms - last) as f32 / 1000.0;
                if !(0.0..=60.0).contains(&delta) {
                    delta = 1.0; // NTP jump / stall: assume 1 s passed
                }
                self.last_update_time = Some(now_ms);
                if self.recovery_target_temp.is_none() {
                    self.elapsed_offset += delta;
                }
                self.elapsed_offset
            }
        }
    }

    // ---- main update -----------------------------------------------------

    /// Advance the state machine with the latest `current_temp` at time `now_ms`
    /// (monotonic ms), returning the target temperature for the PID. Mirrors `update`.
    pub fn update(&mut self, current_temp: f32, now_ms: i64) -> f32 {
        self.current_temp = current_temp;

        if current_temp > self.cfg.max_temp {
            self.set_error(KilnError::MaxTempExceeded {
                temp: current_temp,
                max: self.cfg.max_temp,
            });
            return 0.0;
        }

        match self.state {
            KilnState::Running => self.update_running(now_ms),
            _ => 0.0,
        }
    }

    fn update_running(&mut self, now_ms: i64) -> f32 {
        if self.profile.is_none() {
            self.set_error(KilnError::NoActiveProfile);
            return 0.0;
        }

        let elapsed = self.get_elapsed_time(now_ms);

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

        // Track SSR saturation for the stall gate. `ssr_output` is written by
        // the control loop after the previous tick's PID update, so this sees a
        // one-tick-old value — irrelevant against a minutes-long window.
        if self.ssr_output >= self.cfg.stall_saturation_output {
            if self.saturated_since.is_none() {
                self.saturated_since = Some(elapsed);
            }
        } else {
            self.saturated_since = None;
        }

        // Stall detection (ramp steps only).
        let mut min_rate = current_step.min_rate;
        if current_step.kind == StepKind::Ramp && min_rate.is_none() {
            min_rate = Some(current_step.desired_rate_or_default() * self.cfg.stall_rate_ratio);
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
                            // Two false-positive guards before counting a failure
                            // (both observed on hardware: error at 879 °C on an
                            // 880 °C step boundary with the PID throttling):
                            //
                            // 1. Arrival band — within STALL_ARRIVAL_BAND of the
                            //    ramp's final target the rate legitimately
                            //    collapses as the PID lands the setpoint; step
                            //    completion is about to fire, not a stall.
                            // 2. Saturation — a kiln can only be physically
                            //    stalled if the SSR has been giving ~everything
                            //    (≥ stall_saturation_output) continuously for
                            //    stall_saturation_window seconds. A slow rate at
                            //    partial power is the PID limiting (integral
                            //    wind-up still in progress), not a stall.
                            //    A window of 0 disables this gate (the reference
                            //    Python behaviour, used by the golden replays).
                            let target = current_step.target_temp.unwrap_or(0.0);
                            let arriving = abs(target - self.current_temp) <= STALL_ARRIVAL_BAND;
                            let saturated = self.cfg.stall_saturation_window <= 0.0
                                || self.saturated_since.is_some_and(|since| {
                                    elapsed - since >= self.cfg.stall_saturation_window
                                });
                            if arriving || !saturated {
                                // Not a stall candidate — don't accrue, don't
                                // reset (a genuine stall interrupted by a brief
                                // dip below saturation keeps its count).
                            } else {
                                self.stall_fail_count += 1;
                                if self.stall_fail_count >= self.cfg.stall_consecutive_fails {
                                    self.set_error(KilnError::Stall {
                                        actual_rate: abs(actual_rate),
                                        min_rate: mr,
                                    });
                                    return 0.0;
                                }
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
        now_ms: i64,
    ) -> bool {
        if matches!(self.state, KilnState::Running | KilnState::Tuning) {
            return false;
        }

        self.profile = Some(profile);
        self.state = KilnState::Running;
        self.start_time = Some(now_ms);
        self.elapsed_offset = elapsed_seconds;
        self.last_update_time = None;
        self.error = None;

        let (calc_index, time_in_step, calc_start_temp) =
            self.find_step_for_elapsed(elapsed_seconds);
        // The logged index can be stale (profile edited between crash and boot);
        // clamp it so the step indexing below cannot panic the control loop.
        let last_step = self.profile.as_ref().unwrap().step_count() - 1;
        self.current_step_index = step_index.unwrap_or(calc_index).min(last_step);
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
        self.saturated_since = None;

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

    /// Find the first step whose target the kiln has *not* yet reached, so a
    /// fresh launch can skip steps already satisfied by `current_temp`. Walks the
    /// nominal schedule (seeded from [`FIND_START_TEMP`], like
    /// `find_step_for_elapsed`), tracking the heat/cool direction so each step's
    /// target is compared against `current_temp` in the direction the profile is
    /// travelling. Returns the step index to start in (last step if every target
    /// is already reached).
    fn find_step_for_temp(&self, current_temp: f32) -> usize {
        let prof = match &self.profile {
            Some(p) if p.step_count() > 0 => p,
            _ => return 0,
        };
        let steps = prof.steps();
        let mut profile_temp = FIND_START_TEMP;
        let mut heating = true;

        for (i, step) in steps.iter().enumerate() {
            let reached = match step.kind {
                StepKind::Ramp => {
                    let target = step.target_temp.unwrap_or(profile_temp);
                    if target >= profile_temp {
                        current_temp >= target
                    } else {
                        current_temp <= target
                    }
                }
                StepKind::Hold => {
                    let target = step.target_temp.unwrap_or(profile_temp);
                    if heating {
                        current_temp >= target
                    } else {
                        current_temp <= target
                    }
                }
                StepKind::Cooling => match step.target_temp {
                    Some(target) => current_temp <= target,
                    None => false,
                },
            };
            if !reached {
                return i;
            }
            // Advance the nominal profile temperature / direction for the next step.
            match step.kind {
                StepKind::Ramp => {
                    let target = step.target_temp.unwrap_or(profile_temp);
                    heating = target >= profile_temp;
                    profile_temp = target;
                }
                StepKind::Cooling => {
                    if let Some(target) = step.target_temp {
                        heating = false;
                        profile_temp = target;
                    }
                }
                StepKind::Hold => {}
            }
        }
        steps.len() - 1
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
        let out = c.update(1301.0, 0);
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
        assert!(c.run_profile(p.clone(), 0));
        assert!(!c.run_profile(p, 1000));
    }

    #[test]
    fn hold_completes_after_duration_then_profile_completes() {
        let mut c = KilnController::new(cfg());
        // current_temp starts at 0 -> step_start_temp = 0 at run.
        c.current_temp = 100.0;
        let p = Profile::new(&[Step::hold(100.0, 30.0)]).unwrap();
        assert!(c.run_profile(p, 0));

        // t=0 first call: elapsed 0, hold target 100.
        let t0 = c.update(100.0, 0);
        assert_eq!(c.state, KilnState::Running);
        assert_eq!(t0, 100.0);

        // Advance past the 30 s hold duration.
        let _ = c.update(100.0, 20_000);
        let _ = c.update(100.0, 40_000); // elapsed ~40 > 30 -> complete (last step)
        assert_eq!(c.state, KilnState::Complete);
        assert_eq!(c.target_temp, 0.0);
    }

    #[test]
    fn ntp_forward_jump_is_clamped_to_one_second() {
        let mut c = KilnController::new(cfg());
        c.current_temp = 50.0;
        let p = Profile::new(&[Step::hold(50.0, 1000.0)]).unwrap();
        assert!(c.run_profile(p, 100_000));
        c.update(50.0, 100_000); // first call: elapsed 0, sets last_update_time
        c.update(50.0, 100_500); // delta 0.5 -> elapsed 0.5
        c.update(50.0, 5_000_000); // delta huge -> clamped to 1.0 -> elapsed 1.5
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

    #[test]
    fn fresh_launch_seeks_step_from_current_temp() {
        // User scenario: kiln already hot (683 °C) when relaunching a profile.
        // Steps 1 (0→100) and 2 (100→600) are already climbed past; the run must
        // join step 3 (600→860) and ramp *up*, not restart at step 1.
        let mut c = KilnController::new(cfg());
        c.current_temp = 683.0;
        let p = Profile::new(&[
            Step::ramp(100.0, Some(50.0), None),
            Step::ramp(600.0, Some(100.0), None),
            Step::ramp(860.0, Some(150.0), None),
        ])
        .unwrap();
        assert!(c.run_profile(p, 0));
        assert_eq!(c.current_step_index(), 2);
        // First tick: ascending target starts at current temp and climbs.
        let target = c.update(683.0, 0);
        assert!(target >= 683.0, "target should ramp up from 683, got {target}");
        assert_eq!(c.state, KilnState::Running);
    }

    #[test]
    fn fresh_launch_cold_starts_at_step_zero() {
        // A cold kiln must still begin at step 0 (previous behaviour preserved).
        let mut c = KilnController::new(cfg());
        c.current_temp = 20.0;
        let p = Profile::new(&[
            Step::ramp(100.0, Some(50.0), None),
            Step::ramp(600.0, Some(100.0), None),
        ])
        .unwrap();
        assert!(c.run_profile(p, 0));
        assert_eq!(c.current_step_index(), 0);
    }

    /// Drive `n` stall checks: constant temp, one update per check interval.
    /// Returns after `n` checks have run (each `update` past `stall_min_step_time`
    /// with `elapsed - last_stall_check >= stall_check_interval` counts one).
    fn stall_cfg() -> ControllerConfig {
        ControllerConfig {
            stall_check_interval: 2.0,
            stall_consecutive_fails: 2,
            stall_min_step_time: 4.0,
            rate_recording_interval: 1.0,
            stall_saturation_output: 95.0,
            stall_saturation_window: 3.0,
            ..ControllerConfig::default()
        }
    }

    /// Run a flat-temperature ramp for `seconds`, ticking once per second with
    /// the given constant SSR output. Returns the controller state at the end.
    fn run_flat(c: &mut KilnController, temp: f32, ssr: f32, seconds: i64) -> KilnState {
        for s in 1..=seconds {
            c.ssr_output = ssr;
            let _ = c.update(temp, s * 1000);
            if c.state == KilnState::Error {
                break;
            }
        }
        c.state
    }

    #[test]
    fn stall_faults_when_saturated_and_flat() {
        // Baseline: SSR pinned at 100 % long past the saturation window, zero
        // rate → genuine stall → ERROR.
        let mut c = KilnController::new(stall_cfg());
        c.current_temp = 100.0;
        let p = Profile::new(&[Step::ramp(500.0, Some(100.0), Some(50.0))]).unwrap();
        assert!(c.run_profile(p, 0));
        let end = run_flat(&mut c, 100.0, 100.0, 60);
        assert_eq!(end, KilnState::Error);
        assert!(matches!(c.error(), Some(KilnError::Stall { .. })));
    }

    #[test]
    fn stall_suppressed_below_saturation() {
        // Same flat rate, but the SSR sits at 60 % — the PID is limiting power
        // (rate control / integral wind-up), not a stalled kiln. No error.
        let mut c = KilnController::new(stall_cfg());
        c.current_temp = 100.0;
        let p = Profile::new(&[Step::ramp(500.0, Some(100.0), Some(50.0))]).unwrap();
        assert!(c.run_profile(p, 0));
        let end = run_flat(&mut c, 100.0, 60.0, 60);
        assert_eq!(end, KilnState::Running);
    }

    #[test]
    fn stall_suppressed_in_arrival_band() {
        // Kiln 1 °C short of the ramp target (the 879/880 incident): the rate
        // collapses as the PID lands the setpoint, SSR still saturated — the
        // arrival band must suppress the stall, letting step completion win.
        let mut c = KilnController::new(stall_cfg());
        c.current_temp = 100.0;
        let p = Profile::new(&[
            Step::ramp(880.0, Some(100.0), Some(50.0)),
            Step::hold(880.0, 10_000.0),
        ])
        .unwrap();
        assert!(c.run_profile(p, 0));
        // Jump close to target (simulates hours of climbing), then sit flat at
        // 879 — inside the 5 °C arrival band — with the SSR saturated.
        let end = run_flat(&mut c, 879.0, 100.0, 60);
        assert_eq!(end, KilnState::Running, "arrival band must suppress stall");
        assert_eq!(c.current_step_index(), 0);
        // Crossing the boundary completes the step instead of erroring.
        c.ssr_output = 100.0;
        let _ = c.update(880.0, 61_000);
        assert_eq!(c.current_step_index(), 1);
    }

    #[test]
    fn stall_saturation_window_zero_disables_gate() {
        // Reference behaviour (golden replays): window 0 → saturation not
        // required, flat rate faults even at 0 % output.
        let mut c = KilnController::new(ControllerConfig {
            stall_saturation_window: 0.0,
            ..stall_cfg()
        });
        c.current_temp = 100.0;
        let p = Profile::new(&[Step::ramp(500.0, Some(100.0), Some(50.0))]).unwrap();
        assert!(c.run_profile(p, 0));
        let end = run_flat(&mut c, 100.0, 0.0, 60);
        assert_eq!(end, KilnState::Error);
    }

    #[test]
    fn resume_clamps_out_of_range_step_index() {
        // The logged step_index can exceed the step count if the profile JSON was
        // edited between the crash and this boot; resume must clamp, not panic.
        let mut c = KilnController::new(cfg());
        let p = Profile::new(&[Step::hold(100.0, 1000.0), Step::cooling(Some(50.0))]).unwrap();
        assert!(c.resume_profile(p, 600.0, Some(100.0), Some(98.0), Some(7), 0));
        assert_eq!(c.current_step_index(), 1);
        assert_eq!(c.state, KilnState::Running);
        // The next update must run without panicking on the clamped index.
        let _ = c.update(98.0, 1_000);
    }
}
