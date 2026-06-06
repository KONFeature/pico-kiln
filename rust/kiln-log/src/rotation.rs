//! Pure flash rotation/retention decisions. The embassy flash-writer executes
//! what these return; all size/age policy lives here so it is host-testable.

use crate::{MAX_AGE_SECS, MAX_FILE_BYTES, MAX_TOTAL_BYTES, PRUNE_TARGET_BYTES};

/// One existing diag file, as seen by a directory scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagEntry {
    /// Size in bytes.
    pub size: u32,
    /// Last-modified Unix seconds (0 if unknown).
    pub mtime: i64,
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
/// `PRUNE_TARGET_BYTES` AND no remaining file with a known mtime is older than
/// `MAX_AGE_SECS`.
///
/// PRECONDITION: `entries` MUST be sorted oldest-first (ascending suffix). The
/// front-contiguous prune relies on it; an unsorted slice yields a wrong count.
///
/// `now` is the current Unix time; pass `0` when the clock is not yet synced to
/// skip the age check (size bounding still applies). A file with `mtime == 0`
/// (unknown — littlefs may not track mtimes) is NEVER age-pruned, so an unsynced
/// or mtime-less filesystem can't lose its diag logs to the age rule; size
/// bounding still governs it.
pub fn boot_prune_count(entries: &[DiagEntry], now: i64) -> usize {
    // `total` is the sum of all sizes, so each `-=` below removes a term that was
    // included; it can never underflow. The 256 KiB hard cap (`can_append`) keeps
    // the sum far under `u32::MAX`.
    let mut total: u32 = entries.iter().map(|e| e.size).sum();
    let mut k = 0usize;

    // Size: drop oldest until under target.
    while total >= PRUNE_TARGET_BYTES && k < entries.len() {
        total -= entries[k].size;
        k += 1;
    }

    // Age: drop further leading (oldest) files that are expired. Only with a real
    // clock (now > 0) and a known mtime (> 0); files are oldest-first so expiry is
    // contiguous from the front, and we stop at the first non-expired/unknown one.
    if now > 0 {
        while k < entries.len() && entries[k].mtime > 0 && (now - entries[k].mtime) > MAX_AGE_SECS {
            k += 1;
        }
    }

    k
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(size: u32, mtime: i64) -> DiagEntry {
        DiagEntry { size, mtime }
    }

    #[test]
    fn rotate_and_append_boundaries() {
        assert!(!should_rotate(MAX_FILE_BYTES - 1));
        assert!(should_rotate(MAX_FILE_BYTES));
        assert!(can_append(MAX_TOTAL_BYTES - 1));
        assert!(!can_append(MAX_TOTAL_BYTES));
    }

    #[test]
    fn no_prune_when_under_target_and_fresh() {
        let files = [e(50 * 1024, 1000), e(50 * 1024, 2000)]; // 100 KiB < 192 KiB
        assert_eq!(boot_prune_count(&files, 3000), 0);
    }

    #[test]
    fn size_prune_drops_oldest_until_under_target() {
        // 4 x 64 KiB = 256 KiB >= 192 KiB target. Drop oldest until < 192 KiB:
        // 256 -> 192 (drop 1, still == target, keep going) -> 128 (drop 2). k=2.
        let files = [
            e(64 * 1024, 100),
            e(64 * 1024, 200),
            e(64 * 1024, 300),
            e(64 * 1024, 400),
        ];
        assert_eq!(boot_prune_count(&files, 500), 2);
    }

    #[test]
    fn age_prune_drops_expired_even_when_small() {
        // Small total (under target) but the oldest two are > 7 days old.
        let now = 100 * 24 * 3600;
        let files = [
            e(1024, now - 9 * 24 * 3600), // expired
            e(1024, now - 8 * 24 * 3600), // expired
            e(1024, now - 1 * 24 * 3600), // fresh
        ];
        assert_eq!(boot_prune_count(&files, now), 2);
    }

    #[test]
    fn age_check_skipped_without_clock() {
        let files = [e(1024, 0), e(1024, 0)];
        assert_eq!(boot_prune_count(&files, 0), 0);
    }

    #[test]
    fn unknown_mtime_is_never_age_pruned() {
        // mtime == 0 means "unknown" (littlefs may not track it). With a real
        // clock it must NOT be treated as infinitely old, else a synced boot would
        // wipe every diag file. Small total, so size pruning does not apply.
        let now = 100 * 24 * 3600;
        let files = [e(1024, 0), e(1024, 0), e(1024, 0)];
        assert_eq!(boot_prune_count(&files, now), 0);
    }

    #[test]
    fn combined_size_then_age_prune() {
        // Over target AND the surviving-oldest is also expired: the size loop runs
        // first, then the age loop continues from where it stopped.
        let now = 100 * 24 * 3600;
        let files = [
            e(64 * 1024, now - 9 * 24 * 3600), // dropped by size, also expired
            e(64 * 1024, now - 9 * 24 * 3600), // dropped by size, also expired
            e(64 * 1024, now - 8 * 24 * 3600), // survives size (128 KiB), but expired
            e(64 * 1024, now - 1 * 24 * 3600), // fresh
        ];
        // Size: 256 -> 192 -> 128 KiB => k=2. Age: entry[2] expired => k=3; entry[3]
        // fresh => stop.
        assert_eq!(boot_prune_count(&files, now), 3);
    }

    #[test]
    fn all_files_pruned_returns_full_len() {
        // Every file expired and total still over target: prune them all, no panic.
        let now = 100 * 24 * 3600;
        let files = [
            e(96 * 1024, now - 9 * 24 * 3600),
            e(96 * 1024, now - 9 * 24 * 3600),
            e(96 * 1024, now - 9 * 24 * 3600),
        ];
        assert_eq!(boot_prune_count(&files, now), 3);
    }
}
