//! Equivalence test: rebuild each profile from steps captured in the fixture
//! and assert `duration`, `progress`, and `is_complete` match the REAL
//! `kiln/profile.py` outputs.
//!
//! The fixture encodes the steps per row (single source of truth), so there are
//! no hand-mirrored step lists to drift. Fixture from
//! `tools/gen_profile_golden.py`.

use kiln_core::profile::{Profile, Step, StepKind};
use std::path::PathBuf;

const TOL: f64 = 1e-6;

fn fixture_path() -> PathBuf {
    [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "fixtures",
        "profile_golden.csv",
    ]
    .iter()
    .collect()
}

fn opt(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.parse().unwrap_or_else(|e| panic!("bad float {s:?}: {e}")))
    }
}

/// Decode `kind,target,rate,min,dur` (empty = absent) into a `Step`. Fields are
/// set directly so optionality matches the reference exactly (not via the
/// convenience constructors).
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

#[test]
fn replay_matches_reference_profile() {
    let path = fixture_path();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read fixture {path:?}: {e}\nrun: python3 rust/kiln-core/tools/gen_profile_golden.py"));

    let mut rows = 0;
    let mut profiles_seen = std::collections::BTreeSet::new();

    for (i, line) in text.lines().enumerate() {
        if i == 0 || line.trim().is_empty() {
            continue;
        }
        let c: Vec<&str> = line.split('|').collect();
        assert_eq!(c.len(), 6, "malformed row {i}: {line:?}");

        let idx: usize = c[0].trim().parse().unwrap();
        let steps = parse_steps(c[1]);
        let elapsed: f64 = c[2].trim().parse().unwrap();
        let exp_duration: f64 = c[3].trim().parse().unwrap();
        let exp_progress: f64 = c[4].trim().parse().unwrap();
        let exp_complete: bool = c[5].trim() == "1";

        let p =
            Profile::new(&steps).unwrap_or_else(|e| panic!("row {i}: profile build failed: {e:?}"));
        profiles_seen.insert(idx);
        rows += 1;

        let dd = (p.duration() - exp_duration).abs();
        assert!(
            dd <= TOL,
            "profile {idx} duration: rust={} ref={} (|Δ|={dd:e})",
            p.duration(),
            exp_duration
        );

        let pg = p.progress(elapsed);
        let pd = (pg - exp_progress).abs();
        assert!(
            pd <= TOL,
            "profile {idx} progress@{elapsed}: rust={pg} ref={exp_progress} (|Δ|={pd:e})"
        );

        assert_eq!(
            p.is_complete(elapsed),
            exp_complete,
            "profile {idx} is_complete@{elapsed}: rust={} ref={}",
            p.is_complete(elapsed),
            exp_complete
        );
    }

    assert!(rows >= 20, "fixture too small ({rows} rows)");
    assert!(
        profiles_seen.len() >= 5,
        "expected >=5 distinct profiles, saw {}",
        profiles_seen.len()
    );
}
