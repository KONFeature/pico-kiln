//! Equivalence test: replay a set_output / update / force_off script captured
//! from the REAL `kiln/hardware.py` `SSRController` and assert the Rust
//! `SsrSchedule` returns the same ON/OFF decision and the same locked duty at
//! every step — including the minimum on-time floor, the mid-cycle duty lock,
//! and the single-cycle-advance "fall behind" quirk.
//!
//! Fixture from `tools/gen_ssr_schedule_golden.py`.

use kiln_core::ssr_schedule::SsrSchedule;
use std::path::PathBuf;

const TOL: f64 = 1e-9;

enum Step {
    SetOutput { percent: f64, duty: f64 },     // S: request -> resulting duty_cycle
    Update { now_ms: u64, on: bool, locked: f64 }, // U: should_be_on + duty_cycle_locked
    ForceOff { locked: f64 },                   // F: locked duty after force_off (0)
}

fn fixture_path() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", "ssr_schedule_golden.csv"]
        .iter()
        .collect()
}

fn parse_header(line: &str) -> f64 {
    let mut cycle_time = 0.0;
    for part in line.trim_start_matches('#').trim().split('|') {
        if let Some(v) = part.strip_prefix("cycle_time=") {
            cycle_time = v.trim().parse().unwrap();
        }
    }
    assert!(cycle_time > 0.0, "bad cycle_time in header: {line:?}");
    cycle_time
}

fn load() -> (f64, Vec<Step>) {
    let path = fixture_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("cannot read fixture {path:?}: {e}\nrun: python3 rust/kiln-core/tools/gen_ssr_schedule_golden.py")
    });

    let mut lines = text.lines();
    let cycle_time = parse_header(lines.next().expect("empty fixture"));

    let mut steps = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() || line.starts_with("kind") {
            continue;
        }
        let c: Vec<&str> = line.split(',').collect();
        assert_eq!(c.len(), 4, "malformed row: {line:?}");
        let kind = c[0].trim();
        let arg = c[1].trim();
        let on = c[2].trim();
        let duty: f64 = c[3].trim().parse().unwrap();
        let step = match kind {
            "S" => Step::SetOutput { percent: arg.parse().unwrap(), duty },
            "U" => Step::Update {
                now_ms: arg.parse().unwrap(),
                on: on == "1",
                locked: duty,
            },
            "F" => Step::ForceOff { locked: duty },
            other => panic!("unknown kind {other:?} in row {line:?}"),
        };
        steps.push(step);
    }
    (cycle_time, steps)
}

#[test]
fn replay_matches_reference_ssr_schedule() {
    let (cycle_time, steps) = load();
    assert!(steps.len() >= 20, "fixture too small ({} steps)", steps.len());

    // Generator seeds the SSRController's clock at 0 before construction.
    let mut s = SsrSchedule::new(cycle_time, 0);

    let mut on_count = 0u32;
    let mut off_count = 0u32;
    let mut saw_force_off = false;

    for (i, step) in steps.iter().enumerate() {
        match *step {
            Step::SetOutput { percent, duty } => {
                s.set_output(percent);
                let got = s.duty_cycle();
                assert!(
                    (got - duty).abs() <= TOL,
                    "row {i}: set_output({percent}) duty rust={got} ref={duty}"
                );
            }
            Step::Update { now_ms, on, locked } => {
                let got_on = s.update(now_ms);
                assert_eq!(got_on, on, "row {i}: update({now_ms}) on rust={got_on} ref={on}");
                let got_locked = s.duty_cycle_locked();
                assert!(
                    (got_locked - locked).abs() <= TOL,
                    "row {i}: update({now_ms}) locked rust={got_locked} ref={locked}"
                );
                if got_on {
                    on_count += 1;
                } else {
                    off_count += 1;
                }
            }
            Step::ForceOff { locked } => {
                s.force_off();
                let got_locked = s.duty_cycle_locked();
                assert!(
                    (got_locked - locked).abs() <= TOL,
                    "row {i}: force_off locked rust={got_locked} ref={locked}"
                );
                assert_eq!(s.duty_cycle(), 0.0, "row {i}: force_off must zero the request");
                saw_force_off = true;
            }
        }
    }

    assert!(on_count >= 5, "fixture barely exercised the ON state ({on_count})");
    assert!(off_count >= 5, "fixture barely exercised the OFF state ({off_count})");
    assert!(saw_force_off, "fixture never exercised force_off");
}
