//! Real-log replay for the Rust-only heating-response watchdog.
//!
//! There is no Python reference for `heat_watch`, so instead of a golden this
//! replays condensed REAL device logs (`tools/gen_heat_watch_fixture.py`): the
//! hottest firing on record (983 °C), a firing interrupted by three power
//! outages (recovery holds at 100 %), and a HIGH_TEMP autotune (open-loop 100 %
//! steps). It proves the default thresholds never trip on a healthy kiln, and
//! that the same traces trip within one window once a frozen reading is
//! injected.

use kiln_core::heat_watch::{HeatWatch, HeatWatchConfig};
use std::path::PathBuf;

#[derive(Clone, Copy)]
struct Row {
    seg: u32,
    t_s: i64,
    temp: f32,
    target: f32,
    output: f32,
    tuning: bool,
}

impl Row {
    fn setpoint(&self) -> Option<f32> {
        if self.tuning {
            None
        } else {
            Some(self.target)
        }
    }
}

fn rows() -> Vec<Row> {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "fixtures",
        "heat_watch_real_logs.csv",
    ]
    .iter()
    .collect();
    let text = std::fs::read_to_string(&path).expect("fixture");
    text.lines()
        .skip(1)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            Row {
                seg: f[0].parse().unwrap(),
                t_s: f[1].parse().unwrap(),
                temp: f[2].parse().unwrap(),
                target: f[3].parse().unwrap(),
                output: f[4].parse().unwrap(),
                tuning: f[5] == "T",
            }
        })
        .collect()
}

fn segments(rows: &[Row]) -> Vec<Vec<Row>> {
    let mut out: Vec<Vec<Row>> = Vec::new();
    for r in rows {
        match out.last_mut() {
            Some(seg) if seg[0].seg == r.seg => seg.push(*r),
            _ => out.push(vec![*r]),
        }
    }
    out
}

#[test]
fn fixture_covers_what_it_claims() {
    let rows = rows();
    let segs = segments(&rows);
    assert!(segs.len() >= 3);
    let max_temp = rows.iter().map(|r| r.temp).fold(f32::MIN, f32::max);
    assert!(max_temp > 950.0, "hot firing present ({max_temp})");
    let full_power = rows.iter().filter(|r| r.output >= 95.0).count();
    assert!(full_power > 300, "plenty of full-power samples ({full_power})");
    assert!(rows.iter().any(|r| r.tuning && r.output >= 95.0), "open-loop 100 % tuning");
    assert!(segs.len() >= 5, "recovery segments present");
}

#[test]
fn real_firings_and_tuning_never_trip() {
    for seg in segments(&rows()) {
        let mut w = HeatWatch::new(HeatWatchConfig::default());
        for r in &seg {
            let res = w.update(r.t_s * 1_000, r.temp, r.output, r.setpoint());
            assert!(
                res.is_ok(),
                "false trip in segment {} at t={}s ({:.1} C, target {:.1}, {}%): {:?}",
                r.seg,
                r.t_s,
                r.temp,
                r.target,
                r.output,
                res
            );
        }
    }
}

/// Freeze the reading from the first sample of `seg` where `pick` holds; the
/// PID (or the open-loop tuner) then demands full power. Returns
/// `(freeze time, frozen value, trip time)`, or `None` if it never trips.
fn trip_after_freeze(seg: &[Row], pick: impl Fn(&Row) -> bool) -> Option<(i64, f32, i64)> {
    let mut w = HeatWatch::new(HeatWatchConfig::default());
    let mut frozen: Option<(i64, f32)> = None;
    for r in seg {
        if frozen.is_none() && pick(r) {
            frozen = Some((r.t_s, r.temp));
        }
        let (temp, output) = match frozen {
            Some((_, t)) => (t, 100.0),
            None => (r.temp, r.output),
        };
        if w.update(r.t_s * 1_000, temp, output, r.setpoint()).is_err() {
            let (t0, t_frozen) = frozen.expect("must not trip before the fault");
            return Some((t0, t_frozen, r.t_s));
        }
    }
    None
}

const BAND: f32 = 10.0; // HeatWatchConfig::default().error_band
const SAMPLE_S: i64 = 60; // fixture decimation for firings

#[test]
fn frozen_reading_mid_firing_trips_one_window_after_leaving_the_band() {
    let segs = segments(&rows());
    let firing = segs
        .iter()
        .max_by(|a, b| {
            let ma = a.iter().map(|r| r.temp).fold(f32::MIN, f32::max);
            let mb = b.iter().map(|r| r.temp).fold(f32::MIN, f32::max);
            ma.partial_cmp(&mb).unwrap()
        })
        .unwrap();
    // The 983 C firing: freeze at 600, 800 and 950 C on the way up. While the
    // profile holds near the frozen value the watch (correctly) stays disarmed;
    // once the logged target moves more than the band above it, a full-power
    // window must trip.
    let default = HeatWatchConfig::default();
    assert_eq!(default.error_band, BAND);
    for at in [600.0_f32, 800.0, 950.0] {
        let (t0, frozen, tripped) = trip_after_freeze(firing, |r| !r.tuning && r.temp >= at)
            .unwrap_or_else(|| panic!("reading frozen at {at} C was never caught"));
        let armed = firing
            .iter()
            .find(|r| r.t_s >= t0 && r.target - frozen > BAND)
            .map(|r| r.t_s)
            .expect("target leaves the band");
        assert!(
            tripped - armed <= default.window_s as i64 + SAMPLE_S,
            "frozen at {frozen} C (t={t0}s): left band at {armed}s, tripped at {tripped}s"
        );
    }
}

#[test]
fn frozen_reading_during_open_loop_tuning_trips() {
    let segs = segments(&rows());
    let tuning = segs.iter().find(|s| s[0].tuning).expect("tuning segment");
    let (t0, _, tripped) = trip_after_freeze(tuning, |r| r.output >= 95.0 && r.temp > 300.0)
        .expect("frozen reading during a 100 % tuning step was never caught");
    // Open-loop tuning is always armed, so the window in progress at the freeze
    // may be diluted by an earlier 0-80 % step: it then restarts, and the next
    // (all-100 %) window trips. Bound: two windows.
    assert!(tripped - t0 <= 2 * 900 + 20, "tripped {}s after the freeze", tripped - t0);
}
