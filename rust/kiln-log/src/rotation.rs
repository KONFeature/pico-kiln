//! Pure flash rotation/retention decisions. The embassy flash-writer executes
//! what these return; all size policy lives here so it is host-testable.

use crate::MAX_FILE_BYTES;

/// True when the active file has reached the per-file rotation size.
pub fn should_rotate(active_size: u32) -> bool {
    active_size >= MAX_FILE_BYTES
}

/// How many entries to evict from the FRONT of an eviction-ordered list to free at
/// least `bytes_to_free`.
///
/// The caller builds the list sacrificial-first: all diag files (oldest→newest)
/// before any CSV run file (oldest→newest, the active run excluded). This returns
/// the smallest prefix whose summed sizes cover the deficit, capped at the list
/// length — if evicting everything still falls short, evict everything (best
/// effort; the active run is never in the list, so it is always preserved).
pub fn evict_count(ordered_sizes: &[u32], bytes_to_free: u32) -> usize {
    if bytes_to_free == 0 {
        return 0;
    }
    let mut freed: u32 = 0;
    for (k, &sz) in ordered_sizes.iter().enumerate() {
        freed = freed.saturating_add(sz);
        if freed >= bytes_to_free {
            return k + 1;
        }
    }
    ordered_sizes.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_boundary() {
        assert!(!should_rotate(MAX_FILE_BYTES - 1));
        assert!(should_rotate(MAX_FILE_BYTES));
    }

    #[test]
    fn evict_none_when_no_deficit() {
        assert_eq!(evict_count(&[50, 50, 50], 0), 0);
    }

    #[test]
    fn evict_smallest_prefix_covering_deficit() {
        assert_eq!(evict_count(&[50, 50, 50], 100), 2); // 50+50 = 100 >= 100
        assert_eq!(evict_count(&[50, 50, 50], 99), 2); // first that crosses
        assert_eq!(evict_count(&[50, 50, 50], 50), 1);
        assert_eq!(evict_count(&[50, 50, 50], 1), 1);
    }

    #[test]
    fn evict_all_when_insufficient() {
        assert_eq!(evict_count(&[50, 50, 50], 1000), 3);
    }

    #[test]
    fn evict_empty_list() {
        assert_eq!(evict_count(&[], 100), 0);
    }

    #[test]
    fn evict_saturates_without_overflow() {
        // A huge first file covers the deficit; the running sum must not wrap.
        assert_eq!(evict_count(&[u32::MAX, u32::MAX], 100), 1);
    }
}
