//! The filesystem/CSV half of `server/recovery.py` that `kiln-core::recovery`
//! deliberately left out: reading the last log line and splitting its columns
//! (`_parse_last_log_entry`), pulling the profile name out of the filename
//! (`check_recovery` lines 203-213), and the candidate filter
//! (`_find_most_recent_log`). The actual directory scan + `mtime` sort stays in
//! the embassy FS task; everything here is pure and host-tested, and feeds a
//! [`LastLogEntry`] straight into [`kiln_core::recovery::check_recovery`].

use core::fmt::{self, Write};
use kiln_core::recovery::LastLogEntry;
use kiln_core::state::KilnState;

/// Map a CSV `state` column to [`KilnState`]. Only `RUNNING` matters to the
/// decision; every other string (terminal states, the `RECOVERY` marker, or
/// anything malformed) maps to a non-`Running` variant and is rejected
/// identically — exactly the reference's `state != 'RUNNING'` test.
fn parse_state(s: &str) -> KilnState {
    match s {
        "RUNNING" => KilnState::Running,
        "TUNING" => KilnState::Tuning,
        "COMPLETE" => KilnState::Complete,
        "ERROR" => KilnState::Error,
        _ => KilnState::Idle,
    }
}

/// Parse a single CSV data line into a [`LastLogEntry`], or `None` if it is not
/// exactly 10 columns or a numeric field is unparseable (the reference returns
/// `None` on any exception). Mirrors `_parse_last_log_entry`'s field selection:
/// `[1]` elapsed, `[2]` current_temp, `[3]` target_temp, `[5]` state, `[7]`
/// step_index.
pub fn parse_last_log_entry(line: &str) -> Option<LastLogEntry> {
    let mut cols: [&str; 10] = [""; 10];
    let mut n = 0;
    for c in line.split(',') {
        if n < 10 {
            cols[n] = c;
        }
        n += 1;
    }
    if n != 10 {
        return None;
    }

    // `int(values[7]) if values[7] else None`. The `is_recovering` marker writes
    // -1 here; resume treats the index as advisory (`unwrap_or(calc_index)`), so
    // mapping any non-positive/blank to None just recomputes the step from
    // elapsed — safe, and sidesteps the reference's negative-index leak.
    let step_index = match cols[7].trim() {
        "" => None,
        s => match s.parse::<i64>() {
            Ok(i) if i >= 0 => Some(i as usize),
            _ => None,
        },
    };

    Some(LastLogEntry {
        state: parse_state(cols[5]),
        last_temp: cols[2].trim().parse::<f64>().ok()? as f32,
        last_target_temp: cols[3].trim().parse::<f64>().ok()? as f32,
        elapsed_seconds: cols[1].trim().parse::<f64>().ok()? as f32,
        step_index,
    })
}

/// The last non-empty line of `content`, but only if the file has at least two
/// lines (header + ≥1 data row), mirroring `_parse_last_log_entry`'s
/// `line_count < 2` guard.
pub fn select_last_data_line(content: &str) -> Option<&str> {
    let mut last = None;
    let mut count = 0usize;
    for line in content.lines() {
        count += 1;
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            last = Some(trimmed);
        }
    }
    if count < 2 {
        return None;
    }
    last
}

/// Parse the tail of a whole CSV file — `select_last_data_line` then
/// `parse_last_log_entry`.
pub fn last_log_entry_from_csv(content: &str) -> Option<LastLogEntry> {
    parse_last_log_entry(select_last_data_line(content)?)
}

/// The profile-name portion of a `{profile}_{YYYY-MM-DD}_{HH-MM-SS}.csv`
/// filename — everything before the last two `_`-separated components, with any
/// directory prefix stripped. `None` if there are fewer than two underscores
/// (the reference's `len(parts) >= 3` guard). Not lowercased; pair with
/// [`write_lowercase`] to reproduce `parts[0].lower()`.
pub fn profile_stem(filename: &str) -> Option<&str> {
    let base = filename.rsplit('/').next().unwrap_or(filename);
    let mut it = base.rsplitn(3, '_');
    let _time = it.next()?;
    let _date = it.next()?;
    it.next()
}

/// Write `s` lowercased (ASCII), reproducing `str.lower()` for the profile name
/// without allocating.
pub fn write_lowercase<W: Write>(w: &mut W, s: &str) -> fmt::Result {
    for c in s.chars() {
        w.write_char(c.to_ascii_lowercase())?;
    }
    Ok(())
}

/// Whether a directory entry is a recovery candidate: a `.csv` file that is not
/// a `tuning_` log, the filter in `_find_most_recent_log`.
pub fn is_recovery_candidate(filename: &str) -> bool {
    filename.ends_with(".csv") && !filename.starts_with("tuning_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_running_row() {
        let line = "2023-11-14 22:13:20,1800.0,300.50,305.00,80.00,RUNNING,ramp,2,4,95.0";
        let e = parse_last_log_entry(line).unwrap();
        assert_eq!(e.state, KilnState::Running);
        assert_eq!(e.elapsed_seconds, 1800.0);
        assert_eq!(e.last_temp, 300.5);
        assert_eq!(e.last_target_temp, 305.0);
        assert_eq!(e.step_index, Some(2));
    }

    #[test]
    fn recovery_event_row_is_non_running_with_blank_index() {
        // state column is RECOVERY, step fields empty.
        let line = "2023-11-14 22:13:20,1234.5,250.00,300.00,80.00,RECOVERY,,,,90.0";
        let e = parse_last_log_entry(line).unwrap();
        assert_ne!(e.state, KilnState::Running);
        assert_eq!(e.step_index, None);
    }

    #[test]
    fn recovering_data_row_maps_minus_one_index_to_none() {
        // is_recovering row: state RUNNING, step_name RECOVERY, step_index -1.
        let line = "2023-11-14 22:13:20,3600.0,500.00,600.00,100.00,RUNNING,RECOVERY,-1,4,0.0";
        let e = parse_last_log_entry(line).unwrap();
        assert_eq!(e.state, KilnState::Running);
        assert_eq!(e.step_index, None);
    }

    #[test]
    fn rejects_wrong_column_count() {
        assert!(parse_last_log_entry("a,b,c").is_none());
        // 11 columns
        assert!(parse_last_log_entry("0,1,2,3,4,RUNNING,6,7,8,9,10").is_none());
    }

    #[test]
    fn rejects_unparseable_numbers() {
        let line = "ts,notanumber,2,3,4,RUNNING,ramp,0,1,5.0";
        assert!(parse_last_log_entry(line).is_none());
    }

    #[test]
    fn blank_step_index_is_none() {
        let line = "ts,10.0,2,3,4,RUNNING,,,,5.0";
        let e = parse_last_log_entry(line).unwrap();
        assert_eq!(e.step_index, None);
    }

    #[test]
    fn select_last_line_requires_two_lines_and_skips_blanks() {
        assert!(select_last_data_line("only one line").is_none());
        let csv = "header\nrow1\nrow2\n\n";
        assert_eq!(select_last_data_line(csv), Some("row2"));
    }

    #[test]
    fn parses_tail_of_full_csv() {
        let csv = "timestamp,elapsed_seconds,current_temp_c,target_temp_c,ssr_output_percent,\
state,step_name,step_index,total_steps,measured_rate_c_per_hour\n\
2023-11-14 22:13:20,0.0,25.00,200.00,0.00,RUNNING,ramp,0,3,0.0\n\
2023-11-14 22:13:50,30.0,40.00,200.00,100.00,RUNNING,ramp,0,3,1800.0\n";
        let e = last_log_entry_from_csv(csv).unwrap();
        assert_eq!(e.elapsed_seconds, 30.0);
        assert_eq!(e.last_temp, 40.0);
        assert_eq!(e.step_index, Some(0));
    }

    #[test]
    fn profile_stem_strips_date_time_and_dir() {
        assert_eq!(
            profile_stem("logs/Biscuit_Faience_2025-11-02_13-28-09.csv"),
            Some("Biscuit_Faience")
        );
        assert_eq!(profile_stem("cone6_2023-11-14_22-13-20.csv"), Some("cone6"));
        // Fewer than two underscores → None.
        assert_eq!(profile_stem("noprofile.csv"), None);
        assert_eq!(profile_stem("a_b.csv"), None);
    }

    #[test]
    fn lowercase_matches_python_lower() {
        let mut out = String::new();
        write_lowercase(&mut out, "Biscuit_Faience").unwrap();
        assert_eq!(out, "biscuit_faience");
    }

    #[test]
    fn candidate_filter_excludes_tuning_and_non_csv() {
        assert!(is_recovery_candidate("cone6_2023-11-14_22-13-20.csv"));
        assert!(!is_recovery_candidate("tuning_2023-11-14_22-13-20.csv"));
        assert!(!is_recovery_candidate("notes.txt"));
    }
}
