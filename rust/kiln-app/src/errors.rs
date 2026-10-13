//! Human error strings for the web `error` field — the presentation that the
//! typed [`KilnError`] deliberately omits from `kiln-core`. Reconstructed here at
//! the web boundary, mirroring the reference's `set_error` messages.

use core::fmt::{self, Write};
use kiln_core::state::{FiringLock, KilnError};

/// Write the human-readable message for `e`.
pub fn write_error<W: Write>(w: &mut W, e: &KilnError) -> fmt::Result {
    match e {
        KilnError::MaxTempExceeded { temp, max } => {
            write!(w, "Temperature {:.1}C exceeds maximum {:.0}C", temp, max)
        }
        KilnError::Stall {
            actual_rate,
            min_rate,
        } => write!(
            w,
            "Heating stalled: {:.1}C/h below minimum {:.1}C/h",
            actual_rate, min_rate
        ),
        KilnError::NoActiveProfile => write!(w, "No active profile"),
        KilnError::SensorFault => {
            write!(w, "Sensor emergency shutdown: too many consecutive faults")
        }
        KilnError::SensorNotInitialized => write!(w, "Sensor not initialized"),
        // Rust-only errors. Plain ASCII, no quotes: `json.rs` embeds this text
        // verbatim inside a JSON string.
        KilnError::NoHeatResponse {
            rise,
            window_s,
            avg_output,
        } => write!(
            w,
            "No heating response: temperature rose {:.1}C in {:.0} min at {:.0}% power - check the thermocouple and elements, or the kiln cannot reach this temperature",
            rise,
            window_s / 60.0,
            avg_output
        ),
        KilnError::FiringLocked(lock) => w.write_str(firing_lock_message(*lock)),
    }
}

/// The operator-facing reason firing is locked (status `error` text and the 503
/// body of the start-firing endpoints). Plain ASCII, no quotes.
pub fn firing_lock_message(lock: FiringLock) -> &'static str {
    match lock {
        FiringLock::ConfigMissing => {
            "Firing locked: no config.json on the controller - save the configuration, then reboot"
        }
        FiringLock::ConfigUnreadable => {
            "Firing locked: config.json could not be read or parsed - save the configuration again, then reboot"
        }
        FiringLock::ConfigInvalid => {
            "Firing locked: config.json has an invalid value (see /api/logs) - fix the configuration, then reboot"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(e: KilnError) -> String {
        let mut s = String::new();
        write_error(&mut s, &e).unwrap();
        s
    }

    #[test]
    fn messages_match_reference_phrasing() {
        assert_eq!(
            msg(KilnError::MaxTempExceeded {
                temp: 1301.0,
                max: 1300.0
            }),
            "Temperature 1301.0C exceeds maximum 1300C"
        );
        assert_eq!(msg(KilnError::NoActiveProfile), "No active profile");
        assert_eq!(
            msg(KilnError::SensorFault),
            "Sensor emergency shutdown: too many consecutive faults"
        );
    }

    #[test]
    fn rust_only_errors_render_json_safe_text() {
        let m = msg(KilnError::NoHeatResponse {
            rise: 0.0,
            window_s: 900.0,
            avg_output: 100.0,
        });
        assert_eq!(
            m,
            "No heating response: temperature rose 0.0C in 15 min at 100% power - check the thermocouple and elements, or the kiln cannot reach this temperature"
        );
        for lock in [
            FiringLock::ConfigMissing,
            FiringLock::ConfigUnreadable,
            FiringLock::ConfigInvalid,
        ] {
            let m = msg(KilnError::FiringLocked(lock));
            // The web app detects the lock by this prefix (FIRING_LOCKED_PREFIX in
            // web/src/lib/config/schema.ts): keep them in sync.
            assert!(m.starts_with("Firing locked: "), "{m}");
            assert!(!m.contains('"') && !m.contains('\\') && m.is_ascii(), "{m}");
        }
    }
}
