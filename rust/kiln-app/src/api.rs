//! Request validation for the REST API — the exactness-critical guards from
//! `server/web_server.py`, lifted out of the async handlers so they are pure and
//! host-tested. The picoserve routes in `server.rs` call these and render the
//! responses; the predicates, the inclusive/strict boundaries, the verbatim
//! error strings, and the connection/upload limits all live here.
//!
//! Also a minimal, panic-safe extractor for the tiny flat-object JSON command
//! bodies (`{"profile": "...", "start_time": ...}`), replacing `json.loads` +
//! `data.get(...)`. It is deliberately not a full parser: it scans for a quoted
//! key followed by `:` and reads the immediate string/number value, returning
//! `None` on anything unexpected so malformed input fails validation cleanly.

use kiln_core::tuner::TuningMode;

/// Max simultaneous TCP connections (`MAX_CONCURRENT_CONNECTIONS`).
pub const MAX_CONCURRENT_CONNECTIONS: usize = 3;
/// Max upload size in bytes (`MAX_UPLOAD_SIZE`, 500 KB).
pub const MAX_UPLOAD_SIZE: u32 = 512_000;
/// Max buffered non-upload request body (`MAX_JSON_BODY`).
pub const MAX_JSON_BODY: usize = 4096;
/// File streaming chunk size (`FILE_CHUNK_SIZE`).
pub const FILE_CHUNK_SIZE: usize = 1024;

/// `", ".join(VALID_TUNING_MODES)` — the order is SAFE, STANDARD, THOROUGH,
/// HIGH_TEMP, embedded in the invalid-mode message.
pub const INVALID_MODE_MESSAGE: &str =
    "Invalid mode. Must be one of: SAFE, STANDARD, THOROUGH, HIGH_TEMP";
/// The out-of-range max-temp message (note the degree signs, as in the source).
pub const MAX_TEMP_RANGE_MESSAGE: &str =
    "Maximum temperature must be between 50\u{b0}C and 500\u{b0}C";

/// A file-operation target directory — `VALID_DIRECTORIES`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Directory {
    Profiles,
    Logs,
}

impl Directory {
    /// Parse the path segment, `None` for anything but `profiles`/`logs`
    /// (`validate_directory`).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "profiles" => Some(Directory::Profiles),
            "logs" => Some(Directory::Logs),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Directory::Profiles => "profiles",
            Directory::Logs => "logs",
        }
    }
}

/// Whether a filename is safe from directory traversal (`safe_filename`):
/// no `/`, `\`, or `..`; non-empty; not a dotfile.
pub fn safe_filename(filename: &str) -> bool {
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return false;
    }
    !filename.is_empty() && !filename.starts_with('.')
}

/// Whether file operations are permitted — only while IDLE (`check_idle_state`).
pub fn file_ops_allowed(state: kiln_core::state::KilnState) -> bool {
    state == kiln_core::state::KilnState::Idle
}

/// Whether bulk delete is permitted for `dir` — logs only
/// (`handle_api_files_delete_all`).
pub fn bulk_delete_allowed(dir: Directory) -> bool {
    dir == Directory::Logs
}

/// Parse + validate a tuning mode string against `VALID_TUNING_MODES`.
pub fn parse_tuning_mode(s: &str) -> Option<TuningMode> {
    match s {
        "SAFE" => Some(TuningMode::Safe),
        "STANDARD" => Some(TuningMode::Standard),
        "THOROUGH" => Some(TuningMode::Thorough),
        "HIGH_TEMP" => Some(TuningMode::HighTemp),
        _ => None,
    }
}

/// Validate an optional max-temp: `None` keeps the mode default; otherwise it
/// must be within `[50, 500]` °C inclusive (`handle_api_tuning_start`).
pub fn max_temp_valid(max_temp: Option<f64>) -> bool {
    match max_temp {
        None => true,
        Some(t) => (50.0..=500.0).contains(&t),
    }
}

/// Whether `profile` is present and non-empty — Python's `not profile_name`
/// truthiness test (run + schedule).
pub fn profile_present(profile: Option<&str>) -> bool {
    matches!(profile, Some(p) if !p.is_empty())
}

/// Whether both schedule fields are present and truthy: `not profile_name or
/// not start_time`, where `start_time` of `0` is also "missing".
pub fn schedule_fields_present(profile: Option<&str>, start_time: Option<f64>) -> bool {
    profile_present(profile) && matches!(start_time, Some(t) if t != 0.0)
}

/// Whether a scheduled start is in the future — strict `>` so a start at exactly
/// `now` is rejected, matching `start_time <= time.time()` (`handle_api_schedule`).
pub fn start_time_in_future(start_time: f64, now: f64) -> bool {
    start_time > now
}

/// The outcome of validating an upload's `Content-Length`
/// (`handle_api_files_upload`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadSize {
    /// Valid: within `(0, MAX_UPLOAD_SIZE]`.
    Ok,
    /// `<= 0`: 400 "Missing or invalid Content-Length".
    Missing,
    /// `> MAX_UPLOAD_SIZE`: 413 (dynamic message at the boundary).
    TooLarge,
}

/// Classify an upload `Content-Length`.
pub fn validate_upload_size(content_length: i64) -> UploadSize {
    if content_length <= 0 {
        UploadSize::Missing
    } else if content_length as u64 > MAX_UPLOAD_SIZE as u64 {
        UploadSize::TooLarge
    } else {
        UploadSize::Ok
    }
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

/// The byte index just past `"key"`, found anywhere in `body`.
fn find_key_end(body: &str, key: &str) -> Option<usize> {
    let b = body.as_bytes();
    let k = key.as_bytes();
    let needle = k.len() + 2;
    let mut i = 0;
    while i + needle <= b.len() {
        if b[i] == b'"' && &b[i + 1..i + 1 + k.len()] == k && b[i + 1 + k.len()] == b'"' {
            return Some(i + needle);
        }
        i += 1;
    }
    None
}

/// The byte index of the value following `"key":`, skipping whitespace.
fn value_start(body: &str, key: &str) -> Option<usize> {
    let b = body.as_bytes();
    let mut i = find_key_end(body, key)?;
    while i < b.len() && is_ws(b[i]) {
        i += 1;
    }
    if i >= b.len() || b[i] != b':' {
        return None;
    }
    i += 1;
    while i < b.len() && is_ws(b[i]) {
        i += 1;
    }
    if i >= b.len() {
        None
    } else {
        Some(i)
    }
}

/// Extract a string field's value (without unescaping), or `None` if the key is
/// absent or its value is not a string.
pub fn json_get_str<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let b = body.as_bytes();
    let start = value_start(body, key)?;
    if b[start] != b'"' {
        return None;
    }
    let s = start + 1;
    let mut i = s;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return body.get(s..i),
            _ => i += 1,
        }
    }
    None
}

/// Extract a numeric field's raw token, or `None` if absent/non-numeric. Parse
/// with [`json_get_f64`].
pub fn json_get_number<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let b = body.as_bytes();
    let start = value_start(body, key)?;
    let mut i = start;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E') {
            i += 1;
        } else {
            break;
        }
    }
    if i == start {
        None
    } else {
        body.get(start..i)
    }
}

/// Extract a numeric field as `f64`.
pub fn json_get_f64(body: &str, key: &str) -> Option<f64> {
    json_get_number(body, key)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_core::state::KilnState;

    #[test]
    fn safe_filename_blocks_traversal() {
        assert!(safe_filename("cone6.json"));
        assert!(safe_filename("my-profile_v2.json"));
        assert!(!safe_filename("../etc/passwd"));
        assert!(!safe_filename("a/b.json"));
        assert!(!safe_filename("a\\b.json"));
        assert!(!safe_filename(""));
        assert!(!safe_filename(".hidden"));
    }

    #[test]
    fn directory_parsing() {
        assert_eq!(Directory::parse("profiles"), Some(Directory::Profiles));
        assert_eq!(Directory::parse("logs"), Some(Directory::Logs));
        assert_eq!(Directory::parse("etc"), None);
        assert!(bulk_delete_allowed(Directory::Logs));
        assert!(!bulk_delete_allowed(Directory::Profiles));
    }

    #[test]
    fn file_ops_only_when_idle() {
        assert!(file_ops_allowed(KilnState::Idle));
        for s in [
            KilnState::Running,
            KilnState::Tuning,
            KilnState::Complete,
            KilnState::Error,
        ] {
            assert!(!file_ops_allowed(s));
        }
    }

    #[test]
    fn tuning_mode_validation() {
        assert_eq!(parse_tuning_mode("SAFE"), Some(TuningMode::Safe));
        assert_eq!(parse_tuning_mode("HIGH_TEMP"), Some(TuningMode::HighTemp));
        assert_eq!(parse_tuning_mode("safe"), None);
        assert_eq!(parse_tuning_mode("BOGUS"), None);
    }

    #[test]
    fn max_temp_inclusive_bounds() {
        assert!(max_temp_valid(None));
        assert!(max_temp_valid(Some(50.0)));
        assert!(max_temp_valid(Some(500.0)));
        assert!(max_temp_valid(Some(275.0)));
        assert!(!max_temp_valid(Some(49.9)));
        assert!(!max_temp_valid(Some(500.1)));
    }

    #[test]
    fn presence_and_future_checks() {
        assert!(profile_present(Some("cone6")));
        assert!(!profile_present(Some("")));
        assert!(!profile_present(None));

        assert!(schedule_fields_present(
            Some("cone6"),
            Some(1_700_000_000.0)
        ));
        assert!(!schedule_fields_present(Some("cone6"), Some(0.0)));
        assert!(!schedule_fields_present(None, Some(1.0)));

        assert!(start_time_in_future(1000.0, 999.0));
        assert!(!start_time_in_future(1000.0, 1000.0));
        assert!(!start_time_in_future(1000.0, 1001.0));
    }

    #[test]
    fn upload_size_classification() {
        assert_eq!(validate_upload_size(0), UploadSize::Missing);
        assert_eq!(validate_upload_size(-5), UploadSize::Missing);
        assert_eq!(validate_upload_size(1024), UploadSize::Ok);
        assert_eq!(validate_upload_size(MAX_UPLOAD_SIZE as i64), UploadSize::Ok);
        assert_eq!(
            validate_upload_size(MAX_UPLOAD_SIZE as i64 + 1),
            UploadSize::TooLarge
        );
    }

    #[test]
    fn json_extracts_string_and_number() {
        let body = r#"{"profile": "cone6_glaze", "start_time": 1700000000}"#;
        assert_eq!(json_get_str(body, "profile"), Some("cone6_glaze"));
        assert_eq!(json_get_str(body, "missing"), None);

        let tuning = r#"{ "mode":"SAFE" , "max_temp" : 200.5 }"#;
        assert_eq!(json_get_str(tuning, "mode"), Some("SAFE"));
        assert_eq!(json_get_f64(tuning, "max_temp"), Some(200.5));
    }

    #[test]
    fn json_extractor_is_panic_safe_on_garbage() {
        for body in [
            "",
            "{",
            "\"profile\"",
            "{\"profile\":",
            "{\"profile\": \"unterminated",
            "{\"max_temp\": }",
            "not json at all",
            "{\"profile\":\"\\",
        ] {
            // Must not panic; just returns None.
            let _ = json_get_str(body, "profile");
            let _ = json_get_f64(body, "max_temp");
        }
        assert_eq!(
            json_get_str("{\"profile\": \"unterminated", "profile"),
            None
        );
        assert_eq!(json_get_str("{\"a\":\"b\"}", "profile"), None);
    }

    #[test]
    fn json_number_without_value_is_none() {
        assert_eq!(json_get_f64(r#"{"max_temp": }"#, "max_temp"), None);
        assert_eq!(json_get_str(r#"{"max_temp": 5}"#, "max_temp"), None);
    }
}
