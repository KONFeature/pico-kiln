//! Platform abstraction traits the two halves are generic over.
//!
//! `kiln-control` and `kiln-app` never name a concrete chip; they are generic
//! over a small set of behaviours that `kiln-firmware` supplies for the RP2350
//! (and that the host sim / tests supply with mocks). Per `ARCHITECTURE.md §3`
//! this is the natural home for them: `kiln-hal` is already `no_std`, already on
//! the dependency path of both halves, and depends only on `embedded-hal` — so
//! the traits cost no extra crate and pull in no `embassy` dependency.
//!
//! - [`Watchdog`] — the hardware watchdog feed (`control_thread.py` `WDT`).
//! - [`TempSensor`] — the thermocouple read + fault check (the SPI half of
//!   `TemperatureSensor.read()`); [`crate::Max31856`] implements it.
//! - [`SsrOutput`] — the relay actuation the duty schedule drives (the
//!   `pin.value()` half of `SSRController`); `kiln-firmware`'s `ConfiguredSsr`
//!   implements it.

/// A hardware watchdog: armed once with a timeout, then fed to prevent a reset.
///
/// Mirrors `machine.WDT`: `start` arms it (like `WDT(timeout=ms)`), `feed`
/// re-arms the countdown (like `wdt.feed()`). The timeout is plain milliseconds
/// so this trait stays free of any `embassy` types; `kiln-firmware` converts to
/// the chip's units. `kiln-control` feeds it only at the end of a successful
/// tick, so a hung loop resets the chip.
pub trait Watchdog {
    /// Arm the watchdog with `timeout_ms`; a reset fires if not fed in time.
    fn start(&mut self, timeout_ms: u32);
    /// Reset the countdown. Called only on a successful control iteration.
    fn feed(&mut self);
}

/// A thermocouple amplifier: read the latest temperature and check for faults.
///
/// This is the I/O the control loop wraps in `kiln_core::temp_filter`: the
/// reference reads the fault register first and treats any set bit as a fault,
/// otherwise unpacks the temperature. Both calls may fail at the bus level
/// (the same `Self::Error`), which the caller also treats as a fault.
pub trait TempSensor {
    /// The underlying bus error type.
    type Error;
    /// `true` if any fault bit is set (the reference's `any(fault)` shutdown
    /// trigger). Checked before reading the temperature.
    fn has_fault(&mut self) -> Result<bool, Self::Error>;
    /// The latest linearised thermocouple temperature in °C (raw, pre-offset).
    fn read_temperature(&mut self) -> Result<f32, Self::Error>;
}

/// A solid-state-relay output the duty schedule actuates.
///
/// `kiln_core::ssr_schedule` decides ON/OFF each 10 Hz sub-tick; this trait
/// applies it. `now_ms` is passed through so a multi-relay implementation can
/// stagger turn-on across sub-ticks (inrush protection) without blocking;
/// single-relay implementations ignore it. [`force_off`](Self::force_off)
/// de-energises every relay immediately (the emergency path).
pub trait SsrOutput {
    /// The underlying pin error type.
    type Error;
    /// Drive the relay(s) to `on`. `now_ms` is a monotonic millisecond clock for
    /// stagger timing; ignored by single-relay outputs.
    fn apply(&mut self, on: bool, now_ms: u64) -> Result<(), Self::Error>;
    /// Immediately de-energise every relay (emergency stop).
    fn force_off(&mut self) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct CountingWatchdog {
        started: Option<u32>,
        feeds: u32,
    }
    impl Watchdog for CountingWatchdog {
        fn start(&mut self, timeout_ms: u32) {
            self.started = Some(timeout_ms);
        }
        fn feed(&mut self) {
            self.feeds += 1;
        }
    }

    #[test]
    fn watchdog_trait_records_start_and_feeds() {
        let mut w = CountingWatchdog::default();
        w.start(8000);
        assert_eq!(w.started, Some(8000));
        assert_eq!(w.feeds, 0);
        w.feed();
        w.feed();
        assert_eq!(w.feeds, 2);
    }
}
