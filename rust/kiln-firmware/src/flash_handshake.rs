//! The cross-core flash-write handshake (Oracle ruling Q1) — the one piece of
//! genuinely RP2350-specific safety logic the port adds.
//!
//! THE HAZARD. On the RP2350 a flash erase/program disables XIP, so *any* code
//! executing from flash stalls for the duration. The Core 1 control loop runs
//! from flash; if Core 0 wrote a CSV row (a flash program) while Core 1 was
//! mid-tick with the SSR energised, the relay could stay on for an unbounded
//! window — a fire hazard — and the hardware watchdog could trip mid-write.
//!
//! THE PROTOCOL. A single [`AtomicU8`] coordinates the two cores:
//!
//! 1. Core 0, before a flash write, publishes [`REQUEST`] and waits for
//!    [`PARKED`].
//! 2. Core 1, between sub-ticks (flash still live), sees [`REQUEST`], forces the
//!    SSR **off** through the normal driver, then publishes [`PARKED`] and enters
//!    [`park_until_idle`] — a **RAM-resident** spin that feeds the watchdog via a
//!    raw register write (no flash access) until Core 0 is done.
//! 3. Core 0 performs the flash write and publishes [`IDLE`]; Core 1 leaves the
//!    spin and resumes. The SSR was de-energised *before* the write, so it cannot
//!    be stuck on across the XIP stall.
//!
//! Only [`park_until_idle`] and the raw watchdog feed must live in RAM (the
//! ruling's "not full RAM-resident"): the rest of the loop runs normally from
//! flash because Core 0 only writes once Core 1 has parked.
//!
//! DEVICE-VERIFICATION SURFACE. The raw watchdog register poke and the
//! `.data` placement are RP2350 specifics validated on hardware; the *protocol*
//! (the ordering above) is the safety-relevant invariant.

use core::sync::atomic::{AtomicU8, Ordering};

/// No flash operation in progress; Core 1 runs normally.
pub const IDLE: u8 = 0;
/// Core 0 has requested a pause and is waiting for Core 1 to park.
pub const REQUEST: u8 = 1;
/// Core 1 has forced the SSR off and is spinning in RAM; Core 0 may write flash.
pub const PARKED: u8 = 2;

/// The shared handshake flag. One writer per state transition, so plain
/// `Acquire`/`Release` ordering is sufficient.
pub static FLASH_LOCK: AtomicU8 = AtomicU8::new(IDLE);

/// Core 0: request a flash pause and block until Core 1 has parked (SSR off).
/// Call immediately before a flash erase/program.
pub fn request_pause() {
    FLASH_LOCK.store(REQUEST, Ordering::Release);
    while FLASH_LOCK.load(Ordering::Acquire) != PARKED {
        core::hint::spin_loop();
    }
}

/// Core 0: release Core 1 after the flash write completes.
pub fn release() {
    FLASH_LOCK.store(IDLE, Ordering::Release);
}

/// Core 1: if a pause is pending, return `true` so the caller forces the SSR off
/// (via the normal driver, while flash is still live) before parking.
pub fn pause_requested() -> bool {
    FLASH_LOCK.load(Ordering::Acquire) == REQUEST
}

/// Core 1: publish [`PARKED`] and spin until Core 0 signals [`IDLE`], feeding the
/// watchdog through `raw_feed` so the chip is not reset during the write.
///
/// MUST be RAM-resident: it executes while flash (XIP) is disabled. `raw_feed`
/// must likewise touch only registers/RAM. The caller must have de-energised the
/// SSR before calling this.
#[link_section = ".data.ram_func"]
#[inline(never)]
pub fn park_until_idle(mut raw_feed: impl FnMut()) {
    FLASH_LOCK.store(PARKED, Ordering::Release);
    while FLASH_LOCK.load(Ordering::Acquire) != IDLE {
        raw_feed();
        core::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    // The protocol's ordering is what matters; it is exercised by the host-side
    // state machine in kiln-control's tests via the same force-off path. The
    // RAM-resident spin and raw register feed are device-verified.
    use super::*;

    #[test]
    fn states_are_distinct() {
        assert_ne!(IDLE, REQUEST);
        assert_ne!(REQUEST, PARKED);
        assert_ne!(IDLE, PARKED);
    }
}
