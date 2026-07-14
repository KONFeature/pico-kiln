//! Crash-recovery decision — the sans-IO half of `server/recovery.py`.
//!
//! After a reboot the controller asks: was the most recent run interrupted
//! mid-firing, and is it safe to resume? The *I/O* — scanning the logs dir for
//! the most recent CSV, reading its last line, splitting the columns, and
//! pulling the profile name out of the filename — stays in `kiln-app`
//! (`server/recovery.py:_find_most_recent_log` / `_parse_last_log_entry`). This
//! module is only the **decision** (`check_recovery`'s body once it has the
//! already-parsed last-log entry):
//!
//! 1. the last logged state must have been [`KilnState::Running`] (not
//!    Complete/Error/Idle/Tuning, nor a non-firing marker), and
//! 2. the current temperature must still be within `max_temp_delta` °C of the
//!    last logged temperature — the primary "the crash was recent enough"
//!    safety check (a matching temperature means little time has passed).
//!
//! Per `kiln-core`'s rules the reference's human-readable `recovery_reason`
//! string is dropped (it was console-diagnostic only); only `can_recover` and
//! the echoed resume parameters cross into the controller. The arithmetic
//! mirrors the reference (validated by `tests/replay_recovery.rs`).

use crate::state::KilnState;

/// `|x|` without `std`/libm.
#[inline]
fn abs(x: f32) -> f32 {
    if x < 0.0 {
        -x
    } else {
        x
    }
}

/// The already-parsed tail of a run's CSV log — the inputs the recovery decision
/// needs. The caller (`kiln-app`) does all the filesystem/CSV work and the
/// `state`-string → [`KilnState`] parse before handing this over. `RUNNING`
/// (and, deliberately, the one-shot `RECOVERY` resume marker — see
/// `kiln-app::recovery_io::parse_state`) reach here as [`KilnState::Running`];
/// terminal states and anything malformed map to some non-`Running` variant
/// and are rejected identically.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LastLogEntry {
    /// Last logged controller state (CSV `state` column, parsed).
    pub state: KilnState,
    /// Last logged current temperature (°C) — CSV `current_temp`. The deviation
    /// check compares against this.
    pub last_temp: f32,
    /// Last logged target temperature (°C) — CSV `target_temp`. Parsed for
    /// schema validation (a malformed column rejects the row, matching the
    /// reference's `float(values[3])`); not part of the decision.
    pub last_target_temp: f32,
    /// How far into the run the last entry was (seconds) — CSV `elapsed`.
    pub elapsed_seconds: f32,
    /// Last step index (0-based), or `None` if the column was blank.
    pub step_index: Option<usize>,
}

/// The recovery decision: whether to resume and the resume parameters (echoed
/// from the entry) the controller needs to `resume_profile`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecoveryDecision {
    /// Whether resuming is safe and warranted.
    pub can_recover: bool,
    /// Echoed `elapsed_seconds` to resume from.
    pub elapsed_seconds: f32,
    /// Echoed last logged temperature.
    pub last_temp: f32,
    /// Echoed last step index.
    pub step_index: Option<usize>,
}

/// Decide whether the interrupted run described by `entry` can be safely resumed,
/// given the `current_temp` reading and the configured `max_temp_delta` (°C).
///
/// Mirrors `server/recovery.py:check_recovery` from the point it has the parsed
/// last-log entry (line 197 onward), branch-for-branch:
///
/// 1. the resume parameters (`elapsed_seconds`, `last_temp`, `step_index`) are
///    echoed through first — they are populated before any condition is checked,
///    so they are present even when recovery is refused;
/// 2. `state != Running` → reject;
/// 3. `|current_temp − last_temp| > max_temp_delta` → reject; the comparison is
///    strict `>`, so an exact-delta match still recovers;
/// 4. otherwise recover.
pub fn check_recovery(
    entry: &LastLogEntry,
    current_temp: f32,
    max_temp_delta: f32,
) -> RecoveryDecision {
    // Echo the resume parameters through regardless of outcome, matching how the
    // reference fills RecoveryInfo (recovery.py:198-201) before the checks.
    let mut decision = RecoveryDecision {
        can_recover: false,
        elapsed_seconds: entry.elapsed_seconds,
        last_temp: entry.last_temp,
        step_index: entry.step_index,
    };

    // 1. Was the last logged state RUNNING? (recovery.py:218)
    if entry.state != KilnState::Running {
        return decision;
    }

    // 2. Is the current temperature still close to the last logged temperature?
    //    The primary safety check: a matching temperature means the crash was
    //    recent enough to resume. Strict `>` mirrors the reference (recovery.py:224-231).
    let temp_deviation = abs(current_temp - entry.last_temp);
    if temp_deviation > max_temp_delta {
        return decision;
    }

    // All checks passed — recovery is safe. (recovery.py:234-235)
    decision.can_recover = true;
    decision
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(last_temp: f32) -> LastLogEntry {
        LastLogEntry {
            state: KilnState::Running,
            last_temp,
            last_target_temp: last_temp + 5.0,
            elapsed_seconds: 1800.0,
            step_index: Some(3),
        }
    }

    #[test]
    fn recovers_when_running_and_within_delta() {
        let e = running(300.0);
        let d = check_recovery(&e, 298.0, 15.0);
        assert!(d.can_recover);
        // Resume parameters echoed straight through.
        assert_eq!(d.elapsed_seconds, 1800.0);
        assert_eq!(d.last_temp, 300.0);
        assert_eq!(d.step_index, Some(3));
    }

    #[test]
    fn deviation_check_is_inclusive_at_the_boundary() {
        // |285 - 300| == 15 == max_delta; strict `>` means this still recovers.
        let d = check_recovery(&running(300.0), 285.0, 15.0);
        assert!(d.can_recover, "exact-delta match must recover (strict >)");
    }

    #[test]
    fn rejects_when_deviation_exceeds_delta_either_sign() {
        // current well below last
        let lo = check_recovery(&running(300.0), 250.0, 15.0);
        assert!(!lo.can_recover);
        // current well above last (abs handles the negative argument)
        let hi = check_recovery(&running(300.0), 400.0, 15.0);
        assert!(!hi.can_recover);
        // Parameters are still echoed on a refusal.
        assert_eq!(hi.elapsed_seconds, 1800.0);
        assert_eq!(hi.step_index, Some(3));
    }

    #[test]
    fn rejects_every_non_running_state() {
        for state in [
            KilnState::Idle,
            KilnState::Tuning,
            KilnState::Complete,
            KilnState::Error,
        ] {
            let e = LastLogEntry {
                state,
                ..running(300.0)
            };
            // Temperature matches exactly, so only the state gates this.
            let d = check_recovery(&e, 300.0, 15.0);
            assert!(!d.can_recover, "{state:?} must not recover");
        }
    }

    #[test]
    fn passes_through_blank_step_index() {
        let e = LastLogEntry {
            step_index: None,
            ..running(300.0)
        };
        let d = check_recovery(&e, 300.0, 15.0);
        assert!(d.can_recover);
        assert_eq!(d.step_index, None);
    }

    #[test]
    fn zero_delta_requires_exact_temperature() {
        // With max_delta 0, any non-zero deviation is rejected; exact recovers.
        assert!(check_recovery(&running(300.0), 300.0, 0.0).can_recover);
        assert!(!check_recovery(&running(300.0), 300.1, 0.0).can_recover);
    }
}
