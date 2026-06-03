//! On-device static-page templating: the `{profiles_list}` substitution that
//! `server/html_cache.py` (`render_profiles_list`) + `main.py`'s boot-time
//! prerender performed. The reference rendered the profile list once at boot and
//! `str.replace`'d it into the cached `index.html`; here the firmware serves the
//! compiled-in `index.html` and a handler splices the freshly-rendered list in at
//! request time (the profile set is tiny and rarely changes).
//!
//! Pure + host-tested: no allocation, writes into any [`core::fmt::Write`]. The
//! embassy handler in `server.rs` only collects the profile names and streams
//! prefix + [`render_profiles_list`] + suffix.

use core::fmt::{self, Write};

/// The placeholder `index.html` carries where the server-rendered profile list
/// goes (`static/index.html`, the `<h2>Profiles</h2>` section).
pub const PROFILES_PLACEHOLDER: &str = "{profiles_list}";

/// Map a profile *filename* to the identifier the UI uses: the stem without the
/// `.json` extension. Mirrors `ProfileCache.list_profiles`, whose stems feed
/// `startProfile('{name}')` → `POST /api/run`, where both the reference
/// (`web_server.py:198`) and the Rust handler (`server.rs` `load_profile`)
/// re-append `.json`. Non-`.json` (or bare `.json`) entries return `None`.
pub fn profile_display_name(filename: &str) -> Option<&str> {
    filename
        .strip_suffix(".json")
        .filter(|stem| !stem.is_empty())
}

/// Render the `<ul>` profile list exactly as `render_profiles_list`
/// (`html_cache.py:78-86`): empty → a single "No profiles found" item, else one
/// `<li>{name} <button onclick="startProfile('{name}')">Start</button></li>` per
/// name. `names` are display stems (see [`profile_display_name`]).
pub fn render_profiles_list<W: Write, S: AsRef<str>>(w: &mut W, names: &[S]) -> fmt::Result {
    if names.is_empty() {
        return w.write_str("<ul><li>No profiles found</li></ul>");
    }
    w.write_str("<ul>")?;
    for name in names {
        let name = name.as_ref();
        write!(
            w,
            "<li>{name} <button onclick=\"startProfile('{name}')\">Start</button></li>"
        )?;
    }
    w.write_str("</ul>")
}

/// Split a template at [`PROFILES_PLACEHOLDER`] into `(prefix, suffix)` so a
/// handler can stream prefix + rendered list + suffix without buffering the whole
/// page. `None` if the placeholder is absent (caller serves the bytes verbatim).
pub fn split_profiles_placeholder(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let needle = PROFILES_PLACEHOLDER.as_bytes();
    bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| (&bytes[..i], &bytes[i + needle.len()..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(names: &[&str]) -> String {
        let mut s = String::new();
        render_profiles_list(&mut s, names).unwrap();
        s
    }

    #[test]
    fn empty_list_matches_reference() {
        assert_eq!(render(&[]), "<ul><li>No profiles found</li></ul>");
    }

    #[test]
    fn list_matches_reference_format() {
        // Verbatim `html_cache.render_profiles_list` output for two profiles.
        assert_eq!(
            render(&["biscuit_faience", "test_ramp"]),
            "<ul>\
<li>biscuit_faience <button onclick=\"startProfile('biscuit_faience')\">Start</button></li>\
<li>test_ramp <button onclick=\"startProfile('test_ramp')\">Start</button></li>\
</ul>"
        );
    }

    #[test]
    fn display_name_strips_json_only() {
        assert_eq!(profile_display_name("biscuit_faience.json"), Some("biscuit_faience"));
        assert_eq!(profile_display_name("test_ramp.json"), Some("test_ramp"));
        assert_eq!(profile_display_name("notes.txt"), None);
        assert_eq!(profile_display_name("README"), None);
        assert_eq!(profile_display_name(".json"), None);
    }

    #[test]
    fn split_finds_placeholder_in_real_index_html() {
        let bytes = include_bytes!("../../../static/index.html");
        let (pre, post) = split_profiles_placeholder(bytes).expect("placeholder present");
        // Neither half still contains the placeholder, and they bracket it.
        assert!(split_profiles_placeholder(pre).is_none());
        assert!(split_profiles_placeholder(post).is_none());
        assert!(pre.ends_with(b"<h2>Profiles</h2>\n"));
        // pre + placeholder + post reconstructs the original file.
        let mut rebuilt = Vec::new();
        rebuilt.extend_from_slice(pre);
        rebuilt.extend_from_slice(PROFILES_PLACEHOLDER.as_bytes());
        rebuilt.extend_from_slice(post);
        assert_eq!(rebuilt, bytes);
    }

    #[test]
    fn split_absent_returns_none() {
        assert!(split_profiles_placeholder(b"<html>no token here</html>").is_none());
    }
}
