//! Firing-profile model + duration/progress math — port of `kiln/profile.py`.
//!
//! Only the *control-relevant* parts live here: the step schedule and the
//! duration / progress / completion math. JSON (de)serialisation, file I/O, and
//! the human-facing name/description are presentation concerns handled by a
//! higher layer, so this stays `no_std` and allocation-free — steps are stored
//! inline in a fixed-capacity array (no heap).

/// `|x|` without `std`/libm. Matches Python `abs` for finite values.
#[inline]
fn abs(x: f64) -> f64 {
    if x < 0.0 {
        -x
    } else {
        x
    }
}

/// Default ramp rate when a ramp omits it — mirrors `step.get('desired_rate', 100)`.
pub const DEFAULT_RAMP_RATE: f64 = 100.0;
/// Assumed natural-cooling rate, used only for duration *estimation* — mirrors
/// the reference's `dtemp / 100.0` in cooling steps.
pub const COOLING_ESTIMATE_RATE: f64 = 100.0;
/// Start temperature assumed when the first step has no target — mirrors
/// `self.steps[0].get('target_temp', 20)`.
pub const ASSUMED_START_TEMP: f64 = 20.0;
/// Max inline steps (no heap). Real kiln schedules are far smaller.
pub const MAX_STEPS: usize = 32;

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
    pub target_temp: Option<f64>,
    pub desired_rate: Option<f64>,
    pub min_rate: Option<f64>,
    pub duration: Option<f64>,
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
    /// [`DEFAULT_RAMP_RATE`] when `None`), with an optional `min_rate`.
    pub fn ramp(target_temp: f64, desired_rate: Option<f64>, min_rate: Option<f64>) -> Self {
        Step {
            kind: StepKind::Ramp,
            target_temp: Some(target_temp),
            desired_rate,
            min_rate,
            duration: None,
        }
    }

    /// Hold `target_temp` for `duration` seconds.
    pub fn hold(target_temp: f64, duration: f64) -> Self {
        Step {
            kind: StepKind::Hold,
            target_temp: Some(target_temp),
            desired_rate: None,
            min_rate: None,
            duration: Some(duration),
        }
    }

    /// Natural cooling, optionally until `target_temp` is reached.
    pub fn cooling(target_temp: Option<f64>) -> Self {
        Step {
            kind: StepKind::Cooling,
            target_temp,
            desired_rate: None,
            min_rate: None,
            duration: None,
        }
    }

    /// `step.get('desired_rate', 100)`.
    pub fn desired_rate_or_default(&self) -> f64 {
        match self.desired_rate {
            Some(r) => r,
            None => DEFAULT_RAMP_RATE,
        }
    }
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
    duration: f64,
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

    /// The active steps.
    pub fn steps(&self) -> &[Step] {
        &self.steps[..self.n_steps]
    }

    /// Number of steps.
    pub fn step_count(&self) -> usize {
        self.n_steps
    }

    /// Estimated total duration in seconds.
    pub fn duration(&self) -> f64 {
        self.duration
    }

    /// Port of `_calculate_duration`. Estimates duration from desired rates;
    /// actual run time varies with the kiln's thermal capacity.
    fn calculate_duration(&self) -> f64 {
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
    /// sequencing is the primary mechanism in the controller).
    pub fn is_complete(&self, elapsed_seconds: f64) -> bool {
        elapsed_seconds >= self.duration
    }

    /// Progress percentage in `[.., 100]`. Mirrors
    /// `min(100.0, (elapsed / duration) * 100)`, returning `100.0` when the
    /// duration is zero.
    pub fn progress(&self, elapsed_seconds: f64) -> f64 {
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
            (p.duration() - (0.0 + 600.0 + 18000.0)).abs() < 1e-9,
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
        assert!((p.duration() - 3600.0).abs() < 1e-9, "dur={}", p.duration());
    }

    #[test]
    fn progress_and_complete() {
        let p = Profile::new(&[Step::hold(600.0, 1000.0)]).unwrap();
        assert_eq!(p.progress(0.0), 0.0);
        assert!((p.progress(500.0) - 50.0).abs() < 1e-9);
        assert_eq!(p.progress(1000.0), 100.0);
        assert_eq!(p.progress(5000.0), 100.0); // clamped
        assert!(!p.is_complete(999.0));
        assert!(p.is_complete(1000.0));
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
