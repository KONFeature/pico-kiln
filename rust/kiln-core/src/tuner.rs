//! PID auto-tuner — port of `kiln/tuner.py`
//! (`ZieglerNicholsTuner` + `TuningStep`).
//!
//! Drives a sequence of fixed-SSR steps that complete on a temperature target,
//! a stabilisation plateau, or a timeout, collecting data for offline PID
//! analysis. As elsewhere, **time is injected** (`now_ms: i64`, monotonic ms) and the
//! presentation-only step *labels* are dropped to keep this `no_std` and
//! allocation-free — the control-relevant outputs (SSR %, completion, stage,
//! step index) are what the equivalence test pins down.

/// Tuning mode (different time/temperature profiles).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuningMode {
    Safe,
    Standard,
    Thorough,
    HighTemp,
}

/// Tuning lifecycle stage. Integer values are used by the golden traces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuningStage {
    Running,
    Complete,
    Error,
}


const PLATEAU_WINDOW: usize = 5;
const PLATEAU_CHECK_INTERVAL_MS: i64 = 60_000; // compared against a monotonic-ms delta
const PLATEAU_RANGE: f32 = 0.5;

/// Max steps any mode produces (THOROUGH builds 13).
pub const MAX_TUNING_STEPS: usize = 16;

#[inline]
fn fmin(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

/// A single tuning step: a fixed SSR output held until a target/plateau/timeout
/// completion condition is met.
#[derive(Debug, Clone, Copy)]
pub struct TuningStep {
    pub ssr_percent: f32,
    pub target_temp: Option<f32>,
    pub hold_time: f32,
    pub timeout: f32,
    pub plateau_detect: bool,

    // Absolute monotonic-millisecond anchors (integer); durations derived from
    // them are narrowed to f32 seconds at use (see `update`).
    start_time: Option<i64>,
    target_reached_time: Option<i64>,
    peak_temp: f32,
    temp_history: [f32; PLATEAU_WINDOW],
    temp_len: usize,
    last_plateau_check: i64,
    plateau_detected: bool,
}

impl TuningStep {
    /// Define a step (its completion conditions). Runtime state is set by
    /// [`start`](Self::start).
    pub fn new(
        ssr_percent: f32,
        target_temp: Option<f32>,
        hold_time: f32,
        timeout: f32,
        plateau_detect: bool,
    ) -> Self {
        Self {
            ssr_percent,
            target_temp,
            hold_time,
            timeout,
            plateau_detect,
            start_time: None,
            target_reached_time: None,
            peak_temp: 0.0,
            temp_history: [0.0; PLATEAU_WINDOW],
            temp_len: 0,
            last_plateau_check: 0,
            plateau_detected: false,
        }
    }

    const fn placeholder() -> Self {
        Self {
            ssr_percent: 0.0,
            target_temp: None,
            hold_time: 0.0,
            timeout: 0.0,
            plateau_detect: false,
            start_time: None,
            target_reached_time: None,
            peak_temp: 0.0,
            temp_history: [0.0; PLATEAU_WINDOW],
            temp_len: 0,
            last_plateau_check: 0,
            plateau_detected: false,
        }
    }

    fn start(&mut self, current_temp: f32, now_ms: i64) {
        self.start_time = Some(now_ms);
        self.target_reached_time = None;
        self.peak_temp = current_temp;
        self.temp_len = 0;
        self.last_plateau_check = now_ms;
        self.plateau_detected = false;
    }

    fn push_temp(&mut self, t: f32) {
        if self.temp_len < PLATEAU_WINDOW {
            self.temp_history[self.temp_len] = t;
            self.temp_len += 1;
        } else {
            for i in 1..PLATEAU_WINDOW {
                self.temp_history[i - 1] = self.temp_history[i];
            }
            self.temp_history[PLATEAU_WINDOW - 1] = t;
        }
    }

    fn history_range(&self) -> f32 {
        let mut mn = self.temp_history[0];
        let mut mx = self.temp_history[0];
        for &v in &self.temp_history[1..self.temp_len] {
            if v < mn {
                mn = v;
            }
            if v > mx {
                mx = v;
            }
        }
        mx - mn
    }

    /// Returns `(ssr_output, step_complete)`. Mirrors `TuningStep.update`.
    fn update(&mut self, current_temp: f32, now_ms: i64) -> (f32, bool) {
        let elapsed = (now_ms - self.start_time.unwrap_or(now_ms)) as f32 / 1000.0;

        if elapsed >= self.timeout {
            return (self.ssr_percent, true);
        }

        if current_temp > self.peak_temp {
            self.peak_temp = current_temp;
        }

        if self.plateau_detect && now_ms - self.last_plateau_check >= PLATEAU_CHECK_INTERVAL_MS {
            self.push_temp(current_temp);
            self.last_plateau_check = now_ms;
            if self.temp_len == PLATEAU_WINDOW && self.history_range() < PLATEAU_RANGE {
                self.plateau_detected = true;
                return (self.ssr_percent, true);
            }
        }

        if let Some(target) = self.target_temp {
            if self.ssr_percent > 0.0 {
                // Heating: absolute target, then hold.
                if current_temp >= target {
                    if self.target_reached_time.is_none() {
                        self.target_reached_time = Some(now_ms);
                    }
                    let hold_elapsed =
                        (now_ms - self.target_reached_time.unwrap()) as f32 / 1000.0;
                    if hold_elapsed >= self.hold_time {
                        return (self.ssr_percent, true);
                    }
                }
            } else {
                // Cooling: target is degrees below the peak.
                let cooling_target = self.peak_temp - target;
                if current_temp <= cooling_target {
                    return (self.ssr_percent, true);
                }
            }
        }

        (self.ssr_percent, false)
    }
}

/// Fixed-capacity step buffer (no heap) used while building a mode's sequence.
struct StepBuf {
    steps: [TuningStep; MAX_TUNING_STEPS],
    n: usize,
}

impl StepBuf {
    fn new() -> Self {
        Self {
            steps: [TuningStep::placeholder(); MAX_TUNING_STEPS],
            n: 0,
        }
    }
    fn push(&mut self, s: TuningStep) {
        self.steps[self.n] = s;
        self.n += 1;
    }
}

fn build_step_sequence(mode: TuningMode, max_temp: f32) -> StepBuf {
    let mut b = StepBuf::new();
    match mode {
        TuningMode::Safe => {
            b.push(TuningStep::new(
                60.0,
                Some(fmin(100.0, max_temp)),
                0.0,
                2400.0,
                false,
            ));
            b.push(TuningStep::new(30.0, None, 0.0, 300.0, false));
            b.push(TuningStep::new(0.0, Some(50.0), 0.0, 1800.0, false));
        }
        TuningMode::Standard => {
            b.push(TuningStep::new(25.0, None, 0.0, 1800.0, true));
            b.push(TuningStep::new(0.0, None, 0.0, 1200.0, false));
            b.push(TuningStep::new(50.0, None, 0.0, 1800.0, true));
            b.push(TuningStep::new(0.0, None, 0.0, 1200.0, false));
            b.push(TuningStep::new(75.0, None, 0.0, 1800.0, true));
            b.push(TuningStep::new(0.0, None, 0.0, 3600.0, false));
        }
        TuningMode::Thorough => {
            for &power in &[20.0_f32, 40.0, 60.0, 80.0] {
                b.push(TuningStep::new(power, None, 0.0, 2700.0, true));
                b.push(TuningStep::new(power, None, 0.0, 300.0, false));
                b.push(TuningStep::new(0.0, Some(50.0), 0.0, 1200.0, false));
            }
            b.push(TuningStep::new(0.0, None, 0.0, 3600.0, false));
        }
        TuningMode::HighTemp => {
            b.push(TuningStep::new(100.0, Some(200.0), 0.0, 3600.0, false));
            b.push(TuningStep::new(0.0, None, 0.0, 600.0, false));
            b.push(TuningStep::new(60.0, None, 0.0, 1800.0, true));
            b.push(TuningStep::new(0.0, None, 0.0, 600.0, false));
            b.push(TuningStep::new(80.0, None, 0.0, 1800.0, true));
            b.push(TuningStep::new(0.0, None, 0.0, 600.0, false));
            b.push(TuningStep::new(
                100.0,
                Some(fmin(600.0, max_temp)),
                300.0,
                1800.0,
                false,
            ));
            b.push(TuningStep::new(0.0, None, 0.0, 3600.0, false));
        }
    }
    b
}

fn default_max_temp(mode: TuningMode) -> f32 {
    match mode {
        TuningMode::Safe => 200.0,
        _ => 900.0,
    }
}

/// Multi-mode PID tuner. Build with [`new`](Self::new), call [`start`](Self::start)
/// once, then [`update`](Self::update) each control loop with the latest temp.
#[derive(Debug, Clone)]
pub struct ZieglerNicholsTuner {
    pub mode: TuningMode,
    pub max_temp: f32,
    steps: [TuningStep; MAX_TUNING_STEPS],
    n_steps: usize,
    stage: TuningStage,
    // Absolute monotonic-millisecond anchor (durations narrowed to f32 at use).
    start_time: Option<i64>,
    current_step_index: usize,
}

impl ZieglerNicholsTuner {
    /// Create a tuner; `max_temp` falls back to the mode default when `None`.
    pub fn new(mode: TuningMode, max_temp: Option<f32>) -> Self {
        let max_temp = max_temp.unwrap_or_else(|| default_max_temp(mode));
        let buf = build_step_sequence(mode, max_temp);
        Self {
            mode,
            max_temp,
            steps: buf.steps,
            n_steps: buf.n,
            stage: TuningStage::Running,
            start_time: None,
            current_step_index: 0,
        }
    }

    /// Begin tuning at time `now_ms` (monotonic ms).
    pub fn start(&mut self, now_ms: i64) {
        self.start_time = Some(now_ms);
        self.stage = TuningStage::Running;
        self.current_step_index = 0;
    }

    /// Advance tuning with the latest `current_temp` at time `now_ms` (monotonic
    /// ms). Returns `(ssr_output, continue_tuning)`. Mirrors `ZieglerNicholsTuner.update`.
    pub fn update(&mut self, current_temp: f32, now_ms: i64) -> (f32, bool) {
        if current_temp > self.max_temp {
            self.stage = TuningStage::Error;
            return (0.0, false);
        }

        if self.steps[self.current_step_index].start_time.is_none() {
            self.steps[self.current_step_index].start(current_temp, now_ms);
        }

        let (ssr_output, step_complete) =
            self.steps[self.current_step_index].update(current_temp, now_ms);

        if step_complete {
            self.current_step_index += 1;
            if self.current_step_index >= self.n_steps {
                self.stage = TuningStage::Complete;
                return (0.0, false);
            }
            self.steps[self.current_step_index].start(current_temp, now_ms);
            return (self.steps[self.current_step_index].ssr_percent, true);
        }

        (ssr_output, true)
    }

    pub fn stage(&self) -> TuningStage {
        self.stage
    }
    pub fn current_step_index(&self) -> usize {
        self.current_step_index
    }
    pub fn total_steps(&self) -> usize {
        self.n_steps
    }

    /// The current step, clamped to the last valid index. After the final step
    /// completes the reference's `current_step` still points at the last step
    /// (only `current_step_index` advances past the end), so the snapshot
    /// accessors below read real step data rather than an unused slot.
    fn current_step(&self) -> &TuningStep {
        let idx = if self.current_step_index >= self.n_steps {
            self.n_steps.saturating_sub(1)
        } else {
            self.current_step_index
        };
        &self.steps[idx]
    }

    /// Seconds elapsed in the current step (`(now_ms − start) / 1000`, or `0`
    /// before it starts) — the reference's merged `elapsed` in `get_status`.
    pub fn step_elapsed(&self, now_ms: i64) -> f32 {
        match self.current_step().start_time {
            Some(t) => (now_ms - t) as f32 / 1000.0,
            None => 0.0,
        }
    }
    /// The current step's fixed SSR output (%).
    pub fn step_ssr_percent(&self) -> f32 {
        self.current_step().ssr_percent
    }
    /// The current step's temperature target, if any.
    pub fn step_target_temp(&self) -> Option<f32> {
        self.current_step().target_temp
    }
    /// The current step's timeout (seconds).
    pub fn step_timeout(&self) -> f32 {
        self.current_step().timeout
    }
    /// Whether the current step has detected a plateau.
    pub fn step_plateau_detected(&self) -> bool {
        self.current_step().plateau_detected
    }
    /// Peak temperature seen during the current step (°C).
    pub fn step_peak_temp(&self) -> f32 {
        self.current_step().peak_temp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_step_counts_match_reference() {
        assert_eq!(
            ZieglerNicholsTuner::new(TuningMode::Safe, None).total_steps(),
            3
        );
        assert_eq!(
            ZieglerNicholsTuner::new(TuningMode::Standard, None).total_steps(),
            6
        );
        assert_eq!(
            ZieglerNicholsTuner::new(TuningMode::Thorough, None).total_steps(),
            13
        );
        assert_eq!(
            ZieglerNicholsTuner::new(TuningMode::HighTemp, None).total_steps(),
            8
        );
    }

    #[test]
    fn safe_default_max_temp_is_200() {
        assert_eq!(
            ZieglerNicholsTuner::new(TuningMode::Safe, None).max_temp,
            200.0
        );
        assert_eq!(
            ZieglerNicholsTuner::new(TuningMode::Standard, None).max_temp,
            900.0
        );
    }

    #[test]
    fn over_max_temp_errors_immediately() {
        let mut t = ZieglerNicholsTuner::new(TuningMode::Safe, Some(100.0));
        t.start(0);
        let (ssr, cont) = t.update(150.0, 1000);
        assert_eq!(ssr, 0.0);
        assert!(!cont);
        assert_eq!(t.stage(), TuningStage::Error);
    }

    #[test]
    fn first_step_outputs_its_ssr() {
        let mut t = ZieglerNicholsTuner::new(TuningMode::Safe, None);
        t.start(0);
        let (ssr, cont) = t.update(20.0, 1000);
        assert_eq!(ssr, 60.0); // SAFE step 0 is 60%
        assert!(cont);
        assert_eq!(t.current_step_index(), 0);
    }

    #[test]
    fn accessors_expose_current_step_snapshot_fields() {
        let mut t = ZieglerNicholsTuner::new(TuningMode::Safe, None);
        t.start(0);
        assert_eq!(t.step_elapsed(0), 0.0);

        t.update(20.0, 1000);
        assert_eq!(t.step_ssr_percent(), 60.0);
        assert_eq!(t.step_target_temp(), Some(100.0));
        assert_eq!(t.step_timeout(), 2400.0);
        assert!(!t.step_plateau_detected());
        assert_eq!(t.step_peak_temp(), 20.0);
        assert_eq!(t.step_elapsed(6000), 5.0);

        t.update(35.0, 7000);
        assert_eq!(t.step_peak_temp(), 35.0);
    }

    #[test]
    fn timeout_only_step_completes_and_advances() {
        // SAFE step 0 heats to 100; once reached (hold 0) it completes and the
        // next step (30%, timeout 300) begins.
        let mut t = ZieglerNicholsTuner::new(TuningMode::Safe, None);
        t.start(0);
        t.update(20.0, 1000);
        let (ssr, _) = t.update(100.0, 2000); // reaches target -> advance to step 1
        assert_eq!(ssr, 30.0);
        assert_eq!(t.current_step_index(), 1);
    }
}
