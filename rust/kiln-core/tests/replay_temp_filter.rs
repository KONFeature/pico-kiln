//! Equivalence test: replay a reading/fault script captured from the REAL
//! reworked `kiln/hardware.py` `TemperatureSensor` and assert the Rust
//! `TempFilter` returns matching median-filtered temperatures, last-good
//! recoveries, and the same fatal errors (NotInitialized, EmergencyShutdown).
//!
//! Fixture from `tools/gen_temp_filter_golden.py`.

use kiln_core::temp_filter::{TempError, TempFilter};
use std::path::PathBuf;

// Relaxed from 1e-9: the filter now computes in f32, so values differ from the
// f64 reference by ~f32 representation error (≤ ~2e-4 °C at kiln temperatures).
const TOL: f64 = 1e-3;
const CAP: usize = 8; // max median window; the fixture uses 3

enum Expect {
    Value(f64),
    NotInitialized,
    EmergencyShutdown,
}

struct Row {
    kind: char, // 'R' reading or 'F' fault
    input: f64, // raw reading for 'R'; unused for 'F'
    expect: Expect,
}

fn fixture_path() -> PathBuf {
    [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "fixtures",
        "temp_filter_golden.csv",
    ]
    .iter()
    .collect()
}

fn parse_header(line: &str) -> (f64, usize) {
    let mut offset = 0.0;
    let mut window = 0;
    for part in line.trim_start_matches('#').trim().split('|') {
        if let Some(v) = part.strip_prefix("offset=") {
            offset = v.trim().parse().unwrap();
        } else if let Some(v) = part.strip_prefix("window=") {
            window = v.trim().parse().unwrap();
        }
    }
    assert!(window >= 1, "bad window in header: {line:?}");
    (offset, window)
}

fn load() -> (f64, usize, Vec<Row>) {
    let path = fixture_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("cannot read fixture {path:?}: {e}\nrun: python3 rust/kiln-core/tools/gen_temp_filter_golden.py")
    });

    let mut lines = text.lines();
    let (offset, window) = parse_header(lines.next().expect("empty fixture"));

    let mut rows = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() || line.starts_with("kind") {
            continue;
        }
        let c: Vec<&str> = line.split(',').collect();
        assert_eq!(c.len(), 3, "malformed row: {line:?}");
        let kind = c[0].trim().chars().next().unwrap();
        let input = if c[1].trim().is_empty() {
            0.0
        } else {
            c[1].trim().parse().unwrap()
        };
        let expect = match c[2].trim() {
            "ERR:NotInitialized" => Expect::NotInitialized,
            "ERR:EmergencyShutdown" => Expect::EmergencyShutdown,
            v => Expect::Value(v.parse().unwrap()),
        };
        rows.push(Row {
            kind,
            input,
            expect,
        });
    }
    (offset, window, rows)
}

#[test]
fn replay_matches_reference_temp_filter() {
    let (offset, window, rows) = load();
    assert!(rows.len() >= 30, "fixture too small ({} rows)", rows.len());

    let mut f = TempFilter::<CAP>::new(offset as f32, window);
    let mut saw_not_init = false;
    let mut saw_shutdown = false;
    let mut last_good_returns = 0u32;

    for (i, r) in rows.iter().enumerate() {
        let got = match r.kind {
            'R' => f.push_reading(r.input as f32),
            'F' => f.push_fault(),
            other => panic!("row {i}: unknown kind {other:?}"),
        };
        if r.kind == 'F' && got.is_ok() {
            last_good_returns += 1;
        }
        match &r.expect {
            Expect::Value(want) => {
                let got =
                    got.unwrap_or_else(|e| panic!("row {i}: expected {want}, got Err({e:?})"));
                let diff = (got as f64 - want).abs();
                assert!(
                    diff <= TOL,
                    "row {i}: rust={got} ref={want} (|Δ|={diff:e} > {TOL:e})"
                );
            }
            Expect::NotInitialized => {
                assert_eq!(got, Err(TempError::NotInitialized), "row {i}");
                saw_not_init = true;
            }
            Expect::EmergencyShutdown => {
                assert_eq!(got, Err(TempError::EmergencyShutdown), "row {i}");
                saw_shutdown = true;
            }
        }
    }

    assert!(saw_not_init, "fixture never exercised NotInitialized");
    assert!(saw_shutdown, "fixture never exercised EmergencyShutdown");
    assert!(
        last_good_returns >= 5,
        "fixture barely exercised last-good recovery"
    );
}
