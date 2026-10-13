//! Firing-profile model + duration/progress math — port of `kiln/profile.py`.
//!
//! Only the *control-relevant* parts live here: the step schedule and the
//! duration / progress / completion math. JSON (de)serialisation, file I/O, and
//! the human-facing name/description are presentation concerns handled by a
//! higher layer, so this stays `no_std` and allocation-free — steps are stored
//! inline in a fixed-capacity array (no heap).

/// `|x|` without `std`/libm. Matches Python `abs` for finite values.
#[inline]
fn abs(x: f32) -> f32 {
    if x < 0.0 {
        -x
    } else {
        x
    }
}

/// Default ramp rate when a ramp omits it — mirrors `step.get('desired_rate', 100)`.
pub const DEFAULT_RAMP_RATE: f32 = 100.0;
/// Assumed natural-cooling rate, used only for duration *estimation* — mirrors
/// the reference's `dtemp / 100.0` in cooling steps.
pub const COOLING_ESTIMATE_RATE: f32 = 100.0;
/// Start temperature assumed when the first step has no target — mirrors
/// `self.steps[0].get('target_temp', 20)`.
pub const ASSUMED_START_TEMP: f32 = 20.0;
/// Max inline steps (no heap). Real kiln schedules are 4-5 steps; 16 keeps
/// generous headroom (e.g. for future step types) while halving the inline
/// `[Step; MAX_STEPS]` array. `Profile` is duplicated in every in-flight command,
/// in the controller, and in the scheduler, so this size matters for RAM.
pub const MAX_STEPS: usize = 16;

/// Step kind. The reference uses the strings `"ramp" | "hold" | "cooling"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Ramp,
    Hold,
    Cooling,
}

/// A single profile step. Optional fields mirror the reference's `dict.get`
/// access pattern (some keys are only meaningful for certain step types).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    pub kind: StepKind,
    pub target_temp: Option<f32>,
    pub desired_rate: Option<f32>,
    pub min_rate: Option<f32>,
    pub duration: Option<f32>,
}

impl Default for Step {
    fn default() -> Self {
        Step {
            kind: StepKind::Cooling,
            target_temp: None,
            desired_rate: None,
            min_rate: None,
            duration: None,
        }
    }
}

impl Step {
    /// Controlled ramp to `target_temp` at `desired_rate` °C/h (defaults to
    /// [`DEFAULT_RAMP_RATE`] when `None`), with an optional `min_rate`. Used by
    /// `kiln-sim` and the replay/unit tests to build profile fixtures.
    pub fn ramp(target_temp: f32, desired_rate: Option<f32>, min_rate: Option<f32>) -> Self {
        Step {
            kind: StepKind::Ramp,
            target_temp: Some(target_temp),
            desired_rate,
            min_rate,
            duration: None,
        }
    }

    /// Hold `target_temp` for `duration` seconds.
    pub fn hold(target_temp: f32, duration: f32) -> Self {
        Step {
            kind: StepKind::Hold,
            target_temp: Some(target_temp),
            desired_rate: None,
            min_rate: None,
            duration: Some(duration),
        }
    }

    /// Natural cooling, optionally until `target_temp` is reached. Used by
    /// `kiln-sim` and the replay/unit tests to build profile fixtures.
    pub fn cooling(target_temp: Option<f32>) -> Self {
        Step {
            kind: StepKind::Cooling,
            target_temp,
            desired_rate: None,
            min_rate: None,
            duration: None,
        }
    }

    /// `step.get('desired_rate', 100)`.
    pub fn desired_rate_or_default(&self) -> f32 {
        match self.desired_rate {
            Some(r) => r,
            None => DEFAULT_RAMP_RATE,
        }
    }
}

/// Fastest ramp a runnable profile may request (°C/h). Far above what any
/// electric kiln can do, so the common "as fast as possible" convention (9999)
/// still fits; the bound exists to reject absurd and non-finite rates.
pub const MAX_RAMP_RATE: f32 = 10_000.0;
/// Longest hold a runnable profile may request (72 h, in seconds).
pub const MAX_HOLD_DURATION_S: f32 = 72.0 * 3600.0;
/// Lowest target temperature a runnable profile may request (°C).
pub const MIN_TARGET_TEMP: f32 = 0.0;

/// Why a step fails [`Profile::validate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepProblem {
    /// `target_temp` is not finite or outside `[MIN_TARGET_TEMP, max_temp]`.
    TargetOutOfRange,
    /// A hold without `target_temp`: the state machine would hold at 0 °C,
    /// i.e. silently switch the kiln off for the whole hold.
    HoldWithoutTarget,
    /// A ramp's `desired_rate` is not in `(0, MAX_RAMP_RATE]`. Zero never moves
    /// the setpoint (the run never ends); negative moves it the wrong way and
    /// disables the stall check.
    RateOutOfRange,
    /// A ramp's `min_rate` is not in `[0, desired_rate]`.
    MinRateOutOfRange,
    /// A hold's `duration` is not finite or outside `[0, MAX_HOLD_DURATION_S]`.
    DurationOutOfRange,
}

/// The first failing step of [`Profile::validate`] (`step` is 0-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepIssue {
    pub step: usize,
    pub kind: StepKind,
    pub problem: StepProblem,
}

/// Why building a [`Profile`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileError {
    /// No steps supplied (`Profile must have at least one step`).
    NoSteps,
    /// More than [`MAX_STEPS`] steps supplied.
    TooManySteps,
}

/// A firing profile: an ordered list of steps and its estimated duration.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    steps: [Step; MAX_STEPS],
    n_steps: usize,
    duration: f32,
}

impl Profile {
    /// Build a profile from `steps`, computing the estimated total duration
    /// exactly as `Profile._calculate_duration`.
    pub fn new(steps: &[Step]) -> Result<Self, ProfileError> {
        if steps.is_empty() {
            return Err(ProfileError::NoSteps);
        }
        if steps.len() > MAX_STEPS {
            return Err(ProfileError::TooManySteps);
        }
        let mut arr = [Step::default(); MAX_STEPS];
        arr[..steps.len()].copy_from_slice(steps);
        let mut p = Profile {
            steps: arr,
            n_steps: steps.len(),
            duration: 0.0,
        };
        p.duration = p.calculate_duration();
        Ok(p)
    }

    /// Safety bounds for a profile that is about to be run (Rust-only; the
    /// reference accepts anything). [`Profile::new`] stays a faithful port of
    /// the Python constructor, so this is a separate gate, applied wherever a
    /// profile enters the system: upload, run/schedule, crash recovery, and
    /// again on Core 1 before a run command is accepted. Every comparison is
    /// written so NaN fails.
    pub fn validate(&self, max_temp: f32) -> Result<(), StepIssue> {
        for (step, s) in self.steps().iter().enumerate() {
            let bad = |problem| {
                Err(StepIssue {
                    step,
                    kind: s.kind,
                    problem,
                })
            };
            if let Some(t) = s.target_temp {
                if !(MIN_TARGET_TEMP..=max_temp).contains(&t) {
                    return bad(StepProblem::TargetOutOfRange);
                }
            }
            match s.kind {
                StepKind::Ramp => {
                    let rate = s.desired_rate_or_default();
                    if !(rate > 0.0 && rate <= MAX_RAMP_RATE) {
                        return bad(StepProblem::RateOutOfRange);
                    }
                    if let Some(m) = s.min_rate {
                        if !(0.0..=rate).contains(&m) {
                            return bad(StepProblem::MinRateOutOfRange);
                        }
                    }
                }
                StepKind::Hold => {
                    if s.target_temp.is_none() {
                        return bad(StepProblem::HoldWithoutTarget);
                    }
                    let d = s.duration.unwrap_or(0.0);
                    if !(0.0..=MAX_HOLD_DURATION_S).contains(&d) {
                        return bad(StepProblem::DurationOutOfRange);
                    }
                }
                StepKind::Cooling => {}
            }
        }
        Ok(())
    }

    /// The active steps.
    pub fn steps(&self) -> &[Step] {
        &self.steps[..self.n_steps]
    }

    /// Number of steps.
    pub fn step_count(&self) -> usize {
        self.n_steps
    }

    /// Estimated total duration in seconds. Used by the golden replay test
    /// (`tests/replay_profile.rs`).
    pub fn duration(&self) -> f32 {
        self.duration
    }

    /// Port of `_calculate_duration`. Estimates duration from desired rates;
    /// actual run time varies with the kiln's thermal capacity.
    fn calculate_duration(&self) -> f32 {
        let steps = &self.steps[..self.n_steps];
        let mut total = 0.0;
        // `self.steps[0].get('target_temp', 20)`
        let mut current_temp = match steps[0].target_temp {
            Some(t) => t,
            None => ASSUMED_START_TEMP,
        };

        for step in steps {
            match step.kind {
                StepKind::Hold => {
                    total += step.duration.unwrap_or(0.0);
                }
                StepKind::Ramp => {
                    let target = step.target_temp.unwrap_or(0.0);
                    let dtemp = abs(target - current_temp);
                    let rate = step.desired_rate_or_default();
                    if rate > 0.0 {
                        total += (dtemp / rate) * 3600.0;
                    }
                    current_temp = target;
                }
                StepKind::Cooling => {
                    if let Some(target) = step.target_temp {
                        let dtemp = abs(current_temp - target);
                        total += (dtemp / COOLING_ESTIMATE_RATE) * 3600.0;
                        current_temp = target;
                    }
                }
            }
        }
        total
    }

    /// `elapsed_seconds >= duration` (fallback completion check; step
    /// sequencing is the primary mechanism in the controller). Used by the
    /// golden replay test (`tests/replay_profile.rs`).
    pub fn is_complete(&self, elapsed_seconds: f32) -> bool {
        elapsed_seconds >= self.duration
    }

    /// Progress percentage in `[.., 100]`. Mirrors
    /// `min(100.0, (elapsed / duration) * 100)`, returning `100.0` when the
    /// duration is zero. Used by the golden replay test
    /// (`tests/replay_profile.rs`).
    pub fn progress(&self, elapsed_seconds: f32) -> f32 {
        if self.duration == 0.0 {
            return 100.0;
        }
        let p = (elapsed_seconds / self.duration) * 100.0;
        if p < 100.0 {
            p
        } else {
            100.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_oversized() {
        assert_eq!(Profile::new(&[]).unwrap_err(), ProfileError::NoSteps);
        let many = [Step::hold(100.0, 1.0); MAX_STEPS + 1];
        assert_eq!(Profile::new(&many).unwrap_err(), ProfileError::TooManySteps);
    }

    #[test]
    fn leading_ramp_contributes_zero_due_to_start_temp_quirk() {
        // FAITHFUL-PORT QUIRK: `_calculate_duration` seeds current_temp from
        // steps[0].target_temp, so a *leading* ramp has dtemp=0 and adds no
        // time. ramp(600)->0s ; hold->600s ; cooling 600->100 @100C/h->18000s.
        // (Verified against the real profile.py: cone6 duration == 18600s.)
        let p = Profile::new(&[
            Step::ramp(600.0, Some(100.0), Some(80.0)),
            Step::hold(600.0, 600.0),
            Step::cooling(Some(100.0)),
        ])
        .unwrap();
        assert!(
            (p.duration() - (0.0 + 600.0 + 18000.0)).abs() < 1e-1,
            "dur={}",
            p.duration()
        );
        assert_eq!(p.step_count(), 3);
    }

    #[test]
    fn ramp_after_leading_hold_uses_default_rate() {
        // A leading hold sets (and doesn't advance) the start temp, so the ramp
        // truly spans 20->120. Default rate 100 C/h => 100/100*3600 = 3600 s.
        let p = Profile::new(&[Step::hold(20.0, 0.0), Step::ramp(120.0, None, None)]).unwrap();
        assert!((p.duration() - 3600.0).abs() < 1e-1, "dur={}", p.duration());
    }

    #[test]
    fn progress_and_complete() {
        let p = Profile::new(&[Step::hold(600.0, 1000.0)]).unwrap();
        assert_eq!(p.progress(0.0), 0.0);
        assert!((p.progress(500.0) - 50.0).abs() < 1e-1);
        assert_eq!(p.progress(1000.0), 100.0);
        assert_eq!(p.progress(5000.0), 100.0); // clamped
        assert!(!p.is_complete(999.0));
        assert!(p.is_complete(1000.0));
    }

    #[test]
    fn validate_accepts_real_profiles() {
        let p = Profile::new(&[
            Step::ramp(600.0, Some(150.0), Some(80.0)),
            Step::ramp(1050.0, Some(100.0), Some(50.0)),
            Step::hold(1050.0, 600.0),
            Step::ramp(100.0, None, None),
            Step::cooling(None),
        ])
        .unwrap();
        assert_eq!(p.validate(1300.0), Ok(()));
        // min_rate 0 is an explicit opt-out of the stall check; allowed.
        let p = Profile::new(&[Step::ramp(600.0, Some(100.0), Some(0.0))]).unwrap();
        assert_eq!(p.validate(1300.0), Ok(()));
    }

    #[test]
    fn validate_rejects_the_audit_h3_profiles() {
        let issue = |steps: &[Step]| Profile::new(steps).unwrap().validate(1300.0).unwrap_err();
        let hold = |t: Option<f32>, d: f32| Step {
            kind: StepKind::Hold,
            target_temp: t,
            desired_rate: None,
            min_rate: None,
            duration: Some(d),
        };
        use StepProblem::*;
        assert_eq!(issue(&[Step::ramp(600.0, Some(0.0), None)]).problem, RateOutOfRange);
        assert_eq!(issue(&[Step::ramp(600.0, Some(-50.0), None)]).problem, RateOutOfRange);
        assert_eq!(issue(&[Step::ramp(600.0, Some(f32::NAN), None)]).problem, RateOutOfRange);
        assert_eq!(issue(&[Step::ramp(600.0, Some(20_000.0), None)]).problem, RateOutOfRange);
        // "As fast as possible" (9999) is a legitimate convention.
        assert!(Profile::new(&[Step::ramp(600.0, Some(9999.0), None)]).unwrap().validate(1300.0).is_ok());
        assert_eq!(issue(&[Step::ramp(5000.0, Some(100.0), None)]).problem, TargetOutOfRange);
        assert_eq!(issue(&[Step::ramp(f32::INFINITY, None, None)]).problem, TargetOutOfRange);
        assert_eq!(issue(&[Step::ramp(-10.0, None, None)]).problem, TargetOutOfRange);
        assert_eq!(issue(&[Step::ramp(600.0, Some(100.0), Some(-1.0))]).problem, MinRateOutOfRange);
        assert_eq!(issue(&[Step::ramp(600.0, Some(100.0), Some(150.0))]).problem, MinRateOutOfRange);
        assert_eq!(issue(&[hold(None, 600.0)]).problem, HoldWithoutTarget);
        assert_eq!(issue(&[hold(Some(600.0), f32::INFINITY)]).problem, DurationOutOfRange);
        assert_eq!(issue(&[hold(Some(600.0), -1.0)]).problem, DurationOutOfRange);
        assert_eq!(issue(&[Step::cooling(Some(1e9))]).problem, TargetOutOfRange);
        // The index points at the failing step.
        let i = issue(&[Step::hold(100.0, 60.0), Step::ramp(600.0, Some(0.0), None)]);
        assert_eq!((i.step, i.kind), (1, StepKind::Ramp));
        // The max_temp bound is the caller's MAX_TEMP.
        let p = Profile::new(&[Step::ramp(1250.0, None, None)]).unwrap();
        assert!(p.validate(1300.0).is_ok());
        assert!(p.validate(1200.0).is_err());
    }

    #[test]
    fn zero_duration_profile_is_fully_complete() {
        // A single cooling step with no target estimates 0 duration.
        let p = Profile::new(&[Step::cooling(None)]).unwrap();
        assert_eq!(p.duration(), 0.0);
        assert_eq!(p.progress(0.0), 100.0);
        assert!(p.is_complete(0.0));
    }
}
