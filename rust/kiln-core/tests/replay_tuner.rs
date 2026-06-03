//! Equivalence test: replay tuner traces captured from the REAL `kiln/tuner.py`
//! (`ZieglerNicholsTuner`) and assert the Rust port produces the same SSR
//! output, continue flag, stage, and step index at every update.
//!
//! Fixtures from `tools/gen_tuner_golden.py`:
//!   * tuner_safe_golden.csv     — target reach -> timeout -> cooling -> COMPLETE
//!   * tuner_standard_golden.csv — plateau x3 + timeout x3 -> COMPLETE
//!   * tuner_error_golden.csv    — over-max-temp -> ERROR

use kiln_core::tuner::{TuningMode, ZieglerNicholsTuner};
use std::path::PathBuf;

const TOL: f64 = 1e-6;

fn fixture(name: &str) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect()
}

fn parse_mode(s: &str) -> TuningMode {
    match s.trim() {
        "SAFE" => TuningMode::Safe,
        "STANDARD" => TuningMode::Standard,
        "THOROUGH" => TuningMode::Thorough,
        "HIGH_TEMP" => TuningMode::HighTemp,
        other => panic!("unknown mode {other:?}"),
    }
}

fn run_fixture(name: &str) -> usize {
    let path = fixture(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("cannot read {path:?}: {e}\nrun: python3 rust/kiln-core/tools/gen_tuner_golden.py")
    });

    let mut mode = None;
    let mut max_temp = None;
    let mut start_now = None;
    let mut data = Vec::new();
    let mut in_data = false;

    for line in text.lines() {
        if let Some(r) = line.strip_prefix("# mode|") {
            mode = Some(parse_mode(r));
        } else if let Some(r) = line.strip_prefix("# max_temp|") {
            max_temp = Some(r.trim().parse::<f64>().unwrap());
        } else if let Some(r) = line.strip_prefix("# start_now|") {
            start_now = Some(r.trim().parse::<f64>().unwrap());
        } else if line.starts_with("idx,") {
            in_data = true;
        } else if in_data && !line.trim().is_empty() {
            data.push(line);
        }
    }

    let mode = mode.expect("missing mode");
    let max_temp = max_temp.expect("missing max_temp");
    let start_now = start_now.expect("missing start_now");

    let mut t = ZieglerNicholsTuner::new(mode, Some(max_temp));
    t.start(start_now);

    for line in &data {
        let f: Vec<&str> = line.split(',').collect();
        assert_eq!(f.len(), 7, "{name}: bad row {line:?}");
        let idx: usize = f[0].trim().parse().unwrap();
        let now: f64 = f[1].trim().parse().unwrap();
        let temp: f64 = f[2].trim().parse().unwrap();
        let exp_ssr: f64 = f[3].trim().parse().unwrap();
        let exp_cont = f[4].trim() == "1";
        let exp_stage: u8 = f[5].trim().parse().unwrap();
        let exp_step: usize = f[6].trim().parse().unwrap();

        let (ssr, cont) = t.update(temp, now);

        let d = (ssr - exp_ssr).abs();
        assert!(
            d <= TOL,
            "{name} row {idx} ssr: rust={ssr} ref={exp_ssr} (|Δ|={d:e})"
        );
        assert_eq!(
            cont, exp_cont,
            "{name} row {idx} continue: rust={cont} ref={exp_cont}"
        );
        assert_eq!(
            t.stage().as_u8(),
            exp_stage,
            "{name} row {idx} stage: rust={} ref={exp_stage}",
            t.stage().as_u8()
        );
        assert_eq!(
            t.current_step_index(),
            exp_step,
            "{name} row {idx} step_index"
        );
    }

    data.len()
}

#[test]
fn replay_safe_mode() {
    let n = run_fixture("tuner_safe_golden.csv");
    assert!(n >= 20, "safe fixture too small ({n})");
}

#[test]
fn replay_standard_mode_plateaus() {
    let n = run_fixture("tuner_standard_golden.csv");
    assert!(n >= 15, "standard fixture too small ({n})");
}

#[test]
fn replay_over_temp_error() {
    let n = run_fixture("tuner_error_golden.csv");
    assert!(n >= 2, "error fixture too small ({n})");
}
