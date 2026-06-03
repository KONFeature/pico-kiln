//! Equivalence test: replay state-machine scenarios captured from the REAL
//! `kiln/state.py` (`KilnController`) and assert the Rust port produces the same
//! per-update outputs — state, target temperature, step index, recovery flag,
//! and measured rate.
//!
//! Each fixture is self-describing (config + profile + op header), so one parser
//! drives all three scenarios. Fixtures from `tools/gen_state_golden.py`:
//!   * state_run_golden.csv      — ramp/hold/cooling -> COMPLETE, with NTP jumps
//!   * state_stall_golden.csv    — stall detection -> ERROR
//!   * state_recovery_golden.csv — resume_profile + recovery hold

use kiln_core::profile::{Profile, Step, StepKind};
use kiln_core::state::{ControllerConfig, KilnController};
use std::path::PathBuf;

const TOL: f64 = 1e-6;

fn fixture(name: &str) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name].iter().collect()
}

fn opt(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.parse().unwrap_or_else(|e| panic!("bad float {s:?}: {e}")))
    }
}

fn parse_step(enc: &str) -> Step {
    let p: Vec<&str> = enc.split(',').collect();
    assert_eq!(p.len(), 5, "bad step encoding {enc:?}");
    let kind = match p[0].trim() {
        "r" => StepKind::Ramp,
        "h" => StepKind::Hold,
        "c" => StepKind::Cooling,
        other => panic!("unknown step kind {other:?}"),
    };
    Step {
        kind,
        target_temp: opt(p[1]),
        desired_rate: opt(p[2]),
        min_rate: opt(p[3]),
        duration: opt(p[4]),
    }
}

fn parse_steps(enc: &str) -> Vec<Step> {
    enc.split(';').map(parse_step).collect()
}

fn parse_config(rest: &str) -> ControllerConfig {
    let c: Vec<&str> = rest.split(',').collect();
    assert_eq!(c.len(), 6, "bad config {rest:?}");
    ControllerConfig {
        max_temp: c[0].trim().parse().unwrap(),
        rate_measurement_window: c[1].trim().parse().unwrap(),
        rate_recording_interval: c[2].trim().parse().unwrap(),
        stall_check_interval: c[3].trim().parse().unwrap(),
        stall_consecutive_fails: c[4].trim().parse().unwrap(),
        stall_min_step_time: c[5].trim().parse().unwrap(),
    }
}

/// `run|pre_run_temp|run_now` or
/// `resume|elapsed|last_logged|current|step_index|now`.
enum Op {
    Run { pre_run_temp: f64, run_now: f64 },
    Resume {
        elapsed: f64,
        last_logged: Option<f64>,
        current: Option<f64>,
        step_index: Option<usize>,
        now: f64,
    },
}

fn parse_op(rest: &str) -> Op {
    let p: Vec<&str> = rest.split('|').collect();
    match p[0].trim() {
        "run" => Op::Run {
            pre_run_temp: p[1].trim().parse().unwrap(),
            run_now: p[2].trim().parse().unwrap(),
        },
        "resume" => Op::Resume {
            elapsed: p[1].trim().parse().unwrap(),
            last_logged: opt(p[2]),
            current: opt(p[3]),
            step_index: {
                let s = p[4].trim();
                if s.is_empty() {
                    None
                } else {
                    Some(s.parse().unwrap())
                }
            },
            now: p[5].trim().parse().unwrap(),
        },
        other => panic!("unknown op {other:?}"),
    }
}

fn close(a: f64, b: f64, idx: usize, field: &str, name: &str) {
    let d = (a - b).abs();
    assert!(d <= TOL, "{name} row {idx} {field}: rust={a} ref={b} (|Δ|={d:e})");
}

fn run_fixture(name: &str) -> usize {
    let path = fixture(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {path:?}: {e}\nrun: python3 rust/kiln-core/tools/gen_state_golden.py"));

    let mut cfg = None;
    let mut steps = None;
    let mut op = None;
    let mut data = Vec::new();
    let mut in_data = false;

    for line in text.lines() {
        if let Some(r) = line.strip_prefix("# config|") {
            cfg = Some(parse_config(r));
        } else if let Some(r) = line.strip_prefix("# profile|") {
            steps = Some(parse_steps(r));
        } else if let Some(r) = line.strip_prefix("# op|") {
            op = Some(parse_op(r));
        } else if line.starts_with("idx,") {
            in_data = true;
        } else if in_data && !line.trim().is_empty() {
            data.push(line);
        }
    }

    let cfg = cfg.expect("missing config");
    let steps = steps.expect("missing profile");
    let op = op.expect("missing op");
    let profile = Profile::new(&steps).expect("profile build");

    let mut c = KilnController::new(cfg);
    match op {
        Op::Run { pre_run_temp, run_now } => {
            c.current_temp = pre_run_temp;
            c.run_profile(profile, run_now).expect("run_profile");
        }
        Op::Resume { elapsed, last_logged, current, step_index, now } => {
            c.resume_profile(profile, elapsed, last_logged, current, step_index, now)
                .expect("resume_profile");
        }
    }

    for line in &data {
        let f: Vec<&str> = line.split(',').collect();
        assert_eq!(f.len(), 8, "{name}: bad row {line:?}");
        let idx: usize = f[0].trim().parse().unwrap();
        let now: f64 = f[1].trim().parse().unwrap();
        let temp: f64 = f[2].trim().parse().unwrap();
        let exp_state: u8 = f[3].trim().parse().unwrap();
        let exp_target: f64 = f[4].trim().parse().unwrap();
        let exp_step: usize = f[5].trim().parse().unwrap();
        let exp_recovering = f[6].trim() == "1";
        let exp_rate: f64 = f[7].trim().parse().unwrap();

        let out = c.update(temp, now);

        assert_eq!(c.state.as_u8(), exp_state, "{name} row {idx} state: rust={} ref={exp_state}", c.state.as_u8());
        close(out, exp_target, idx, "target", name);
        assert_eq!(c.current_step_index(), exp_step, "{name} row {idx} step_index");
        assert_eq!(c.is_recovering(), exp_recovering, "{name} row {idx} recovering");
        close(c.measured_rate(), exp_rate, idx, "rate", name);
    }

    data.len()
}

#[test]
fn replay_run_progression_with_ntp_jumps() {
    let n = run_fixture("state_run_golden.csv");
    assert!(n >= 150, "run fixture too small ({n})");
}

#[test]
fn replay_stall_detection() {
    let n = run_fixture("state_stall_golden.csv");
    assert!(n >= 10, "stall fixture too small ({n})");
}

#[test]
fn replay_crash_recovery() {
    let n = run_fixture("state_recovery_golden.csv");
    assert!(n >= 15, "recovery fixture too small ({n})");
}
