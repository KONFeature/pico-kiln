//! CSV logging — the byte-exact port of `server/data_logger.py`'s row/header/
//! filename formatting, the part `analyze_tuning.py` and crash recovery parse.
//!
//! Only the *formatting* lives here (pure, host-tested); opening the file, the
//! 30 s / 2 s-tuning interval gate, the flush-per-row, and the file-handle
//! recovery are the embassy littlefs task's job (`server.rs`). The schema is the
//! 10-column header verbatim, the `%.1f`/`%.2f` field precision, the two recovery
//! conventions (an `is_recovering` data row vs. a one-shot RECOVERY *event* row),
//! and the `{safe_profile}_{YYYY-MM-DD_HH-MM-SS}.csv` filename.

use core::fmt::{self, Write};
use kiln_core::protocol::Status;

use crate::json::{state_str, step_kind_str};
use crate::timefmt::{write_filename_stamp, write_iso};

/// The CSV header row (with trailing newline) — the 10 columns verbatim.
pub const HEADER: &str = "timestamp,elapsed_seconds,current_temp_c,target_temp_c,\
ssr_output_percent,state,step_name,step_index,total_steps,measured_rate_c_per_hour\n";

/// Write a data row for `s` (with trailing newline), reproducing
/// `data_logger.log_status`'s field selection exactly, including the
/// `is_recovering` markers (`step_name=RECOVERY`, `step_index=-1`, forced
/// `measured_rate=0.0`).
pub fn write_row<W: Write>(w: &mut W, s: &Status) -> fmt::Result {
    write_iso(w, s.timestamp as i64)?;
    write!(
        w,
        ",{:.1},{:.2},{:.2},{:.2},{},",
        s.elapsed,
        s.current_temp,
        s.target_temp,
        s.ssr_output,
        state_str(s.state)
    )?;

    if s.is_recovering {
        // `total_steps or ''`: None *and* 0 render empty (0 is falsy in Python).
        w.write_str("RECOVERY,-1,")?;
        write_total_steps(w, s.total_steps)?;
        return writeln!(w, ",{:.1}", 0.0);
    }

    // step_name: the step type, or empty (no active profile, or past the last
    // step) — `status.get('step_name') or ''`.
    if let Some(k) = s.step_kind {
        w.write_str(step_kind_str(k))?;
    }
    w.write_char(',')?;
    // step_index: the integer if present, else empty — including past-last-step,
    // where the index still prints though step_name is empty.
    if let Some(i) = s.step_index {
        write!(w, "{}", i)?;
    }
    w.write_char(',')?;
    write_total_steps(w, s.total_steps)?;
    writeln!(w, ",{:.1}", s.measured_rate)
}

/// Write a one-shot recovery *event* row (`data_logger.log_recovery_event`):
/// `RECOVERY` in the state column, three empty step fields, `elapsed` from the
/// recovery context.
pub fn write_recovery_event_row<W: Write>(
    w: &mut W,
    timestamp: f64,
    elapsed: f64,
    current_temp: f64,
    target_temp: f64,
    ssr_output: f64,
    measured_rate: f64,
) -> fmt::Result {
    write_iso(w, timestamp as i64)?;
    writeln!(
        w,
        ",{:.1},{:.2},{:.2},{:.2},RECOVERY,,,,{:.1}",
        elapsed, current_temp, target_temp, ssr_output, measured_rate
    )
}

fn write_total_steps<W: Write>(w: &mut W, total_steps: Option<usize>) -> fmt::Result {
    match total_steps {
        Some(n) if n != 0 => write!(w, "{}", n),
        _ => Ok(()),
    }
}

/// Write the log filename `{safe_profile}_{YYYY-MM-DD_HH-MM-SS}.csv` (no
/// directory prefix). `safe_profile` replaces spaces and `/` with `_`, matching
/// `start_logging`.
///
/// The carried `profile_name` is the profile *filename* (`cone6.json`), so its
/// `.json` extension is stripped first — otherwise the log stem would be
/// `cone6.json` and crash recovery (which does `profile_stem` → lowercase →
/// `+ ".json"`) would look up `profiles/cone6.json.json` and never resume
/// (`recovery_io::profile_stem`). The reference sidesteps this by logging the
/// profile's *display name* (`profile.name`), which carries no extension.
pub fn write_log_filename<W: Write>(
    w: &mut W,
    profile_name: &str,
    unix_seconds: i64,
) -> fmt::Result {
    let stem = profile_name.strip_suffix(".json").unwrap_or(profile_name);
    write_safe_profile(w, stem)?;
    w.write_char('_')?;
    write_filename_stamp(w, unix_seconds)?;
    w.write_str(".csv")
}

fn write_safe_profile<W: Write>(w: &mut W, name: &str) -> fmt::Result {
    for c in name.chars() {
        match c {
            ' ' | '/' => w.write_char('_')?,
            c => w.write_char(c)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_core::profile::StepKind;
    use kiln_core::state::KilnState;

    fn render(s: &Status) -> String {
        let mut out = String::new();
        write_row(&mut out, s).unwrap();
        out
    }

    #[test]
    fn header_is_ten_columns_verbatim() {
        assert_eq!(HEADER.matches(',').count(), 9);
        assert!(HEADER.ends_with("measured_rate_c_per_hour\n"));
    }

    #[test]
    fn running_ramp_row() {
        let s = Status {
            timestamp: 1_700_000_000.0,
            state: KilnState::Running,
            current_temp: 123.456,
            target_temp: 200.0,
            ssr_output: 75.5,
            elapsed: 65.25,
            step_index: Some(1),
            step_kind: Some(StepKind::Ramp),
            total_steps: Some(3),
            measured_rate: 95.04,
            ..Status::idle()
        };
        assert_eq!(
            render(&s),
            "2023-11-14 22:13:20,65.2,123.46,200.00,75.50,RUNNING,ramp,1,3,95.0\n"
        );
    }

    #[test]
    fn idle_row_has_empty_step_columns() {
        let s = Status {
            timestamp: 1_700_000_000.0,
            ..Status::idle()
        };
        // state IDLE; step_name, step_index, total_steps all empty.
        assert_eq!(
            render(&s),
            "2023-11-14 22:13:20,0.0,0.00,0.00,0.00,IDLE,,,,0.0\n"
        );
    }

    #[test]
    fn past_last_step_prints_index_but_empty_name() {
        let s = Status {
            timestamp: 1_700_000_000.0,
            state: KilnState::Running,
            step_index: Some(3),
            step_kind: None,
            total_steps: Some(3),
            measured_rate: 0.0,
            ..Status::idle()
        };
        assert_eq!(
            render(&s),
            "2023-11-14 22:13:20,0.0,0.00,0.00,0.00,RUNNING,,3,3,0.0\n"
        );
    }

    #[test]
    fn recovering_row_uses_markers() {
        let s = Status {
            timestamp: 1_700_000_000.0,
            state: KilnState::Running,
            current_temp: 500.0,
            target_temp: 600.0,
            ssr_output: 100.0,
            elapsed: 3600.0,
            step_index: Some(2),
            step_kind: Some(StepKind::Ramp),
            total_steps: Some(4),
            is_recovering: true,
            measured_rate: 120.0,
            ..Status::idle()
        };
        // step_name=RECOVERY, step_index=-1, total_steps kept, measured_rate forced 0.0.
        assert_eq!(
            render(&s),
            "2023-11-14 22:13:20,3600.0,500.00,600.00,100.00,RUNNING,RECOVERY,-1,4,0.0\n"
        );
    }

    #[test]
    fn recovery_event_row_has_empty_step_fields() {
        let mut out = String::new();
        write_recovery_event_row(&mut out, 1_700_000_000.0, 1234.5, 250.0, 300.0, 80.0, 90.0)
            .unwrap();
        assert_eq!(
            out,
            "2023-11-14 22:13:20,1234.5,250.00,300.00,80.00,RECOVERY,,,,90.0\n"
        );
    }

    #[test]
    fn filename_sanitizes_and_stamps() {
        let mut out = String::new();
        write_log_filename(&mut out, "glaze cone6/v2", 1_700_000_000).unwrap();
        assert_eq!(out, "glaze_cone6_v2_2023-11-14_22-13-20.csv");
    }

    #[test]
    fn filename_strips_json_extension() {
        // The carried name is the profile *filename*; its `.json` is dropped so
        // the log stem is the bare profile name.
        let mut out = String::new();
        write_log_filename(&mut out, "cone6.json", 1_700_000_000).unwrap();
        assert_eq!(out, "cone6_2023-11-14_22-13-20.csv");
    }

    #[test]
    fn log_stem_round_trips_through_recovery() {
        // The log filename a run writes must let recovery rebuild the profile
        // path: write `{profile}.json` → derive the stem → `+ ".json"` and get
        // back the original filename (the H3/R3 doubled-`.json` regression).
        use crate::recovery_io::profile_stem;
        let mut log = String::new();
        write_log_filename(&mut log, "cone6.json", 1_700_000_000).unwrap();
        let stem = profile_stem(&log).unwrap();
        let mut profile_file = String::new();
        crate::recovery_io::write_lowercase(&mut profile_file, stem).unwrap();
        profile_file.push_str(".json");
        assert_eq!(profile_file, "cone6.json");
    }
}
