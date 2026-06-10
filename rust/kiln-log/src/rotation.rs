//! Pure flash rotation/retention decisions. The embassy flash-writer executes
//! what these return; all size/age policy lives here so it is host-testable.

use crate::{MAX_FILE_BYTES, MAX_TOTAL_BYTES, PRUNE_TARGET_BYTES};

/// One existing diag file, as seen by a directory scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagEntry {
    /// Size in bytes.
    pub size: u32,
}

/// True when the active file has reached the per-file rotation size.
pub fn should_rotate(active_size: u32) -> bool {
    active_size >= MAX_FILE_BYTES
}

/// True while there is still room to append under the hard total cap.
pub fn can_append(total: u32) -> bool {
    total < MAX_TOTAL_BYTES
}

/// How many of the OLDEST files to delete at boot so that afterwards: total <
/// `PRUNE_TARGET_BYTES`.
///
/// PRECONDITION: `entries` MUST be sorted oldest-first (ascending suffix). The
/// front-contiguous prune relies on it; an unsorted slice yields a wrong count.
///
/// Size-only: diag files carry no real mtime on-device (littlefs keeps none and
/// `diag-NNNNNN.log` names embed no timestamp), so an age rule could never fire
/// there and is not applied.
pub fn boot_prune_count(entries: &[DiagEntry]) -> usize {
    // `total` is the sum of all sizes, so each `-=` below removes a term that was
    // included; it can never underflow. The 256 KiB hard cap (`can_append`) keeps
    // the sum far under `u32::MAX`.
    let mut total: u32 = entries.iter().map(|e| e.size).sum();
    let mut k = 0usize;

    while total >= PRUNE_TARGET_BYTES && k < entries.len() {
        total -= entries[k].size;
        k += 1;
    }

    k
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(size: u32) -> DiagEntry {
        DiagEntry { size }
    }

    #[test]
    fn rotate_and_append_boundaries() {
        assert!(!should_rotate(MAX_FILE_BYTES - 1));
        assert!(should_rotate(MAX_FILE_BYTES));
        assert!(can_append(MAX_TOTAL_BYTES - 1));
        assert!(!can_append(MAX_TOTAL_BYTES));
    }

    #[test]
    fn no_prune_when_under_target() {
        let files = [e(50 * 1024), e(50 * 1024)]; // 100 KiB < 192 KiB
        assert_eq!(boot_prune_count(&files), 0);
    }

    #[test]
    fn size_prune_drops_oldest_until_under_target() {
        // 4 x 64 KiB = 256 KiB >= 192 KiB target. Drop oldest until < 192 KiB:
        // 256 -> 192 (drop 1, still == target, keep going) -> 128 (drop 2). k=2.
        let files = [e(64 * 1024), e(64 * 1024), e(64 * 1024), e(64 * 1024)];
        assert_eq!(boot_prune_count(&files), 2);
    }

    #[test]
    fn all_files_pruned_returns_full_len() {
        // A single file at/over target: every entry drops, no panic past the end.
        let files = [e(96 * 1024), e(96 * 1024), e(192 * 1024)];
        assert_eq!(boot_prune_count(&files), 3);
    }
}
