//! The tuning step-name table — the presentation labels `kiln-core::tuner`
//! deliberately dropped. Reconstructed at the web boundary from `(mode,
//! step_index)`, mirroring `kiln/tuner.py::_build_step_sequence` exactly so
//! `/api/tuning/status` reports the same `step_name` strings.

use kiln_core::tuner::TuningMode;

const SAFE: [&str; 3] = ["heat_60pct_to_100C", "hold_30pct_5min", "cool_to_50C"];

const STANDARD: [&str; 6] = [
    "heat_25pct_plateau",
    "cool_10min",
    "heat_50pct_plateau",
    "cool_10min",
    "heat_75pct_plateau",
    "cool_to_ambient",
];

const THOROUGH: [&str; 13] = [
    "heat_20pct_plateau",
    "hold_20pct_5min",
    "cool_30C",
    "heat_40pct_plateau",
    "hold_40pct_5min",
    "cool_30C",
    "heat_60pct_plateau",
    "hold_60pct_5min",
    "cool_30C",
    "heat_80pct_plateau",
    "hold_80pct_5min",
    "cool_30C",
    "final_cooldown",
];

const HIGH_TEMP: [&str; 8] = [
    "fast_heat_to_200C",
    "cool_10min",
    "heat_60pct_plateau",
    "cool_10min",
    "heat_80pct_plateau",
    "cool_10min",
    "heat_100pct_to_max",
    "final_cooldown",
];

fn table(mode: TuningMode) -> &'static [&'static str] {
    match mode {
        TuningMode::Safe => &SAFE,
        TuningMode::Standard => &STANDARD,
        TuningMode::Thorough => &THOROUGH,
        TuningMode::HighTemp => &HIGH_TEMP,
    }
}

/// The step label for `(mode, index)`, or `""` past the last step (matching the
/// reference, which leaves the label empty once the sequence is exhausted).
pub fn step_name(mode: TuningMode, index: usize) -> &'static str {
    table(mode).get(index).copied().unwrap_or("")
}

/// The tuning-mode string (`SAFE` / `STANDARD` / `THOROUGH` / `HIGH_TEMP`).
pub fn mode_str(mode: TuningMode) -> &'static str {
    match mode {
        TuningMode::Safe => "SAFE",
        TuningMode::Standard => "STANDARD",
        TuningMode::Thorough => "THOROUGH",
        TuningMode::HighTemp => "HIGH_TEMP",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_core::tuner::ZieglerNicholsTuner;

    #[test]
    fn table_lengths_match_tuner_step_counts() {
        for mode in [
            TuningMode::Safe,
            TuningMode::Standard,
            TuningMode::Thorough,
            TuningMode::HighTemp,
        ] {
            let n = ZieglerNicholsTuner::new(mode, None).total_steps();
            assert_eq!(
                table(mode).len(),
                n,
                "step-name table length must match the tuner's step count for {mode:?}"
            );
        }
    }

    #[test]
    fn known_labels_and_out_of_range() {
        assert_eq!(step_name(TuningMode::Safe, 0), "heat_60pct_to_100C");
        assert_eq!(step_name(TuningMode::Standard, 5), "cool_to_ambient");
        assert_eq!(step_name(TuningMode::Thorough, 12), "final_cooldown");
        assert_eq!(step_name(TuningMode::Safe, 99), "");
        assert_eq!(mode_str(TuningMode::HighTemp), "HIGH_TEMP");
    }
}
