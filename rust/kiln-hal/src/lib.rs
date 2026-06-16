//! `kiln-hal` — device drivers for the pico-kiln controller.
//!
//! Thin, `#![no_std]` drivers generic over `embedded-hal` 1.0 traits
//! (`SpiDevice`, `OutputPin`), so the same code runs on the RP2350 and against
//! mock buses under `cargo test`. These drivers do **I/O only** — every control
//! decision (filtering, duty scheduling, state) lives in `kiln-core`.
//!
//! - [`max31856`] — MAX31856 thermocouple amplifier (raw temperature + faults),
//!   a faithful port of the Adafruit driver `kiln/hardware.py` relies on.
#![cfg_attr(not(test), no_std)]

pub mod max31856;
pub mod platform;

pub use max31856::{Faults, Max31856, ThermocoupleType};
pub use platform::{SsrOutput, TempSensor, Watchdog};
