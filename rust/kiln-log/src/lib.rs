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
pub use rotation::{boot_prune_count, can_append, should_rotate, DiagEntry};

/// Rotate the active diag file once it reaches this size.
pub const MAX_FILE_BYTES: u32 = 64 * 1024;
/// Absolute cap across all diag files; runtime appends hard-stop here.
pub const MAX_TOTAL_BYTES: u32 = 256 * 1024;
/// Boot prune deletes oldest-first until total drops below this (¾ of the cap).
pub const PRUNE_TARGET_BYTES: u32 = 192 * 1024;
