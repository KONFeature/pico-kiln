//! Tiny pure helpers for on-device diagnostics — kept here (host-tested) so the
//! firmware's `unsafe` stack scan only does raw pointer→slice wrapping and
//! delegates the index arithmetic to code that runs under `cargo test`.

/// Index of the first word that differs from `pattern`, scanning low→high.
///
/// The firmware paints its free stack region with a known `pattern`; the running
/// stack (which grows DOWN) overwrites it from the high end. Scanning the region
/// from its low address up, the first word that no longer matches `pattern` marks
/// the deepest the stack ever reached → the high-water point. `None` means the
/// whole region is still pristine (stack never descended into it).
pub fn first_dirty_word(words: &[u32], pattern: u32) -> Option<usize> {
    words.iter().position(|&w| w != pattern)
}

#[cfg(test)]
mod tests {
    use super::*;
    const P: u32 = 0xAAAA_AAAA;

    #[test]
    fn all_clean_is_none() {
        assert_eq!(first_dirty_word(&[P; 8], P), None);
    }

    #[test]
    fn finds_first_dirty() {
        let mut w = [P; 8];
        w[3] = 0x1234_5678;
        w[5] = 0; // a later dirty word must not shadow the first
        assert_eq!(first_dirty_word(&w, P), Some(3));
    }

    #[test]
    fn dirty_at_zero() {
        assert_eq!(first_dirty_word(&[0, P, P], P), Some(0));
    }

    #[test]
    fn empty_is_none() {
        assert_eq!(first_dirty_word(&[], P), None);
    }
}
