//! Plain-text log line formatting: `HH:MM:SS LEVEL tag: message\n`, written into
//! any `core::fmt::Write` sink (the embassy side passes a `heapless::String`).
//! Time is rendered as a wall-clock-of-day derived from `secs` (Unix seconds, or
//! an uptime fallback supplied by the caller); only the time-of-day is shown.

use core::fmt::{self, Write};

/// Map a `log` record target (a module path like `kiln_app::server`) to a short
/// tag. Unknown targets fall back to the last `::`-separated segment.
pub fn tag_of(target: &str) -> &str {
    match target {
        t if t.starts_with("kiln_control") => "ctrl",
        t if t.starts_with("kiln_core") => "ctrl",
        t if t.starts_with("cyw43") => "wifi",
        t if t.starts_with("embassy_net") || t.starts_with("smoltcp") => "net",
        t if t.starts_with("embassy_usb") => "usb",
        t if t.starts_with("picoserve") => "web",
        t if t.starts_with("kiln_app::server") => "web",
        t if t.starts_with("kiln_app::logging") => "log",
        t if t.starts_with("kiln_app") => "app",
        t => t.rsplit("::").next().unwrap_or(t),
    }
}

/// Write one formatted line (including the trailing newline) into `w`.
///
/// `secs` is a time value in seconds (Unix wall-clock when known, else an uptime
/// fallback); only `secs.rem_euclid(86_400)` is shown as `HH:MM:SS` (so negative
/// values still render a valid time). If `w` runs out of capacity mid-write it
/// returns `Err` and the caller is responsible for guaranteeing the trailing
/// newline (the embassy side does this).
///
/// Callers must NOT embed `\n` in `args`: the downstream `Ring`/SSE layer treats
/// every newline as a record boundary, so an embedded newline splits one logical
/// line into two fragments.
pub fn format_line<W: Write>(
    w: &mut W,
    secs: i64,
    level: &str,
    tag: &str,
    args: fmt::Arguments,
) -> fmt::Result {
    let day = secs.rem_euclid(86_400);
    let h = day / 3600;
    let m = (day % 3600) / 60;
    let s = day % 60;
    write!(w, "{:02}:{:02}:{:02} {} {}: ", h, m, s, level, tag)?;
    w.write_fmt(args)?;
    w.write_char('\n')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_timestamp_level_tag_message() {
        let mut s = String::new();
        // 01:02:03 == 3723 seconds into the day.
        format_line(&mut s, 3723, "INFO", "ctrl", format_args!("temp={}", 812)).unwrap();
        assert_eq!(s, "01:02:03 INFO ctrl: temp=812\n");
    }

    #[test]
    fn wraps_seconds_into_day() {
        let mut s = String::new();
        // One full day + 1 second -> 00:00:01.
        format_line(&mut s, 86_401, "WARN", "net", format_args!("x")).unwrap();
        assert!(s.starts_with("00:00:01 WARN net: x"));
    }

    #[test]
    fn tag_mapping() {
        assert_eq!(tag_of("kiln_control::controller"), "ctrl");
        assert_eq!(tag_of("kiln_core::pid"), "ctrl");
        assert_eq!(tag_of("cyw43::runner"), "wifi");
        assert_eq!(tag_of("embassy_net::tcp"), "net");
        assert_eq!(tag_of("smoltcp::iface"), "net");
        assert_eq!(tag_of("embassy_usb::cdc_ncm"), "usb");
        assert_eq!(tag_of("picoserve::routing"), "web");
        assert_eq!(tag_of("some_unknown::deep::leaf"), "leaf");
    }

    #[test]
    fn tag_mapping_specific_before_general_kiln_app() {
        // Order-sensitive: the specific kiln_app arms must win over the general one.
        assert_eq!(tag_of("kiln_app::server"), "web");
        assert_eq!(tag_of("kiln_app::logging"), "log");
        assert_eq!(tag_of("kiln_app::config"), "app");
    }
}
