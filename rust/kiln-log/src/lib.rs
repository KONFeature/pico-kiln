//! Pure logging primitives for the pico-kiln firmware — no `core`-external deps,
//! so this crate adds no build script to the pure host-test build (see
//! ../TESTING.md). The embassy-coupled glue (the `log::Log` impl, channels,
//! tasks, picoserve handlers) lives in `kiln-app::logging`.
#![cfg_attr(not(test), no_std)]

pub mod format;
pub mod ring;
pub mod rotation;

pub use format::{format_line, tag_of};
pub use ring::Ring;
pub use rotation::{evict_count, should_rotate};

/// Rotate the active diag file once it reaches this size.
pub const MAX_FILE_BYTES: u32 = 64 * 1024;
/// Free space the BOOT prune reclaims to (deleting diag-first, then oldest
/// non-active runs). Boot is idle — the SSR is off — so reclaim generously for a
/// long runway before the next, mid-run, prune.
pub const BOOT_FREE_TARGET: u32 = 256 * 1024;
/// Free space a mid-RUN prune reclaims TO. Lower than boot on purpose: each run
/// prune happens through the SSR-pausing flash handshake, so freeing fewer files
/// per event = a shorter paused remove batch. The trade is more frequent, smaller
/// prunes — still rare relative to the flush cadence.
pub const RUN_FREE_TARGET: u32 = 128 * 1024;
/// Free space a mid-RUN prune triggers AT (hysteresis low-water; the target above
/// is the high-water). Distinct from the target so a prune reclaims a big margin
/// (≈112 KiB) in one go, then ~14 flushes pass before space dips back to the
/// trigger — instead of trigger == target, which would prune on essentially every
/// flush once past the line. Sized to clear one worst-case flush (CSV 8 KiB + diag
/// 8 KiB ≈ 16 KiB) so a write that *skips* the prune still fits; if it ever does
/// not, the CSV rows are retained (not lost) and the next flush prunes.
pub const RUN_PRUNE_TRIGGER: u32 = 16 * 1024;
