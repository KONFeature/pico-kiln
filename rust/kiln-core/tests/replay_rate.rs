//! Equivalence test: replay add/clear operations captured from the REAL
//! `kiln/rate_monitor.py` and assert the Rust `TempHistory` returns matching
//! rates over every window. Covers ring-buffer overflow and a mid-run clear.
//!
//! Fixture from `tools/gen_rate_golden.py`.

use kiln_core::rate_monitor::TempHistory;
use std::path::PathBuf;

const TOL: f64 = 1e-6;
const CAP: usize = 60; // must match the generator's capacity

struct Row {
    idx: usize,
    op: String,
    time: f64,
    temp: f64,
    rate60: f64,
    rate120: f64,
    rate600: f64,
}

fn fixture_path() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", "rate_golden.csv"]
        .iter()
        .collect()
}

fn load_rows() -> Vec<Row> {
    let path = fixture_path();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read fixture {path:?}: {e}\nrun: python3 rust/kiln-core/tools/gen_rate_golden.py"));

    let mut rows = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if i == 0 || line.trim().is_empty() {
            continue;
        }
        let c: Vec<&str> = line.split(',').collect();
        assert_eq!(c.len(), 7, "malformed row {i}: {line:?}");
        // time/temp are empty on `clear` rows; default to 0.0 (unused there).
        let f = |s: &str| -> f64 { if s.trim().is_empty() { 0.0 } else { s.trim().parse().unwrap() } };
        rows.push(Row {
            idx: c[0].trim().parse().unwrap(),
            op: c[1].trim().to_string(),
            time: f(c[2]),
            temp: f(c[3]),
            rate60: f(c[4]),
            rate120: f(c[5]),
            rate600: f(c[6]),
        });
    }
    rows
}

fn assert_close(actual: f64, expected: f64, idx: usize, field: &str) {
    let diff = (actual - expected).abs();
    assert!(
        diff <= TOL,
        "row {idx} {field}: rust={actual} ref={expected} (|Δ|={diff:e} > {TOL:e})"
    );
}

#[test]
fn replay_matches_reference_rate_monitor() {
    let rows = load_rows();
    assert!(rows.len() >= 90, "fixture too small ({} rows)", rows.len());

    let mut h = TempHistory::<CAP>::new();
    let mut saw_clear = false;
    let mut saw_full = false;

    for r in &rows {
        match r.op.as_str() {
            "add" => h.add(r.time, r.temp),
            "clear" => {
                h.clear();
                saw_clear = true;
            }
            other => panic!("row {}: unknown op {other:?}", r.idx),
        }
        if h.is_full() {
            saw_full = true;
        }

        assert_close(h.get_rate(60.0), r.rate60, r.idx, "rate60");
        assert_close(h.get_rate(120.0), r.rate120, r.idx, "rate120");
        assert_close(h.get_rate(600.0), r.rate600, r.idx, "rate600");
    }

    assert!(saw_clear, "fixture never exercised clear()");
    assert!(saw_full, "fixture never filled the ring buffer");
}
