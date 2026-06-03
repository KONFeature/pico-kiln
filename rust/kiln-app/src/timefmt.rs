//! Wall-clock formatting for CSV rows and log filenames.
//!
//! Hand-rolled UTC civil-date conversion (Howard Hinnant's `civil_from_days`)
//! from Unix epoch seconds — no `chrono`, no `std`, no allocation. The reference
//! uses `time.localtime()`; the Rust port carries real Unix-epoch seconds (from
//! NTP via `sntpc`) and formats UTC, which the CSV/recovery path round-trips
//! self-consistently.

use core::fmt::{self, Write};

/// `(year, month, day)` from days since the Unix epoch (1970-01-01), valid for
/// any day in the supported range. Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Split Unix seconds into `(year, month, day, hour, minute, second)` (UTC).
pub fn civil(unix_seconds: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = unix_seconds.div_euclid(86_400);
    let rem = unix_seconds.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    (
        y,
        mo,
        d,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

/// Write the CSV timestamp `YYYY-MM-DD HH:MM:SS`.
pub fn write_iso<W: Write>(w: &mut W, unix_seconds: i64) -> fmt::Result {
    let (y, mo, d, h, mi, s) = civil(unix_seconds);
    write!(w, "{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, mo, d, h, mi, s)
}

/// Write the filename timestamp `YYYY-MM-DD_HH-MM-SS`.
pub fn write_filename_stamp<W: Write>(w: &mut W, unix_seconds: i64) -> fmt::Result {
    let (y, mo, d, h, mi, s) = civil(unix_seconds);
    write!(w, "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}", y, mo, d, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_timestamp_splits_correctly() {
        assert_eq!(civil(1_700_000_000), (2023, 11, 14, 22, 13, 20));
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(946_684_800), (2000, 1, 1, 0, 0, 0));
    }

    #[test]
    fn iso_and_filename_formats() {
        let mut iso = String::new();
        write_iso(&mut iso, 1_700_000_000).unwrap();
        assert_eq!(iso, "2023-11-14 22:13:20");

        let mut fname = String::new();
        write_filename_stamp(&mut fname, 1_700_000_000).unwrap();
        assert_eq!(fname, "2023-11-14_22-13-20");
    }
}
