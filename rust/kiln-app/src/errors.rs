//! Human error strings for the web `error` field — the presentation that the
//! typed [`KilnError`] deliberately omits from `kiln-core`. Reconstructed here at
//! the web boundary, mirroring the reference's `set_error` messages.

use core::fmt::{self, Write};
use kiln_core::state::KilnError;

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
}
