//! Lightweight mock platform implementations for host tests and `kiln-sim`.
//!
//! These satisfy the [`kiln_hal::platform`] traits with no hardware: the sensor
//! returns a settable temperature (so a thermal model can drive it), the SSR
//! records its commanded state, and the watchdog counts feeds (so a test can
//! prove the loop withholds the feed on an emergency).

use core::convert::Infallible;
use kiln_hal::platform::{SsrOutput, TempSensor, Watchdog};

/// A thermocouple whose reading (and fault state) the caller sets directly.
#[derive(Debug, Clone, Copy)]
pub struct MockSensor {
    pub raw_temp: f32,
    pub fault: bool,
}

impl MockSensor {
    pub fn new(raw_temp: f32) -> Self {
        MockSensor {
            raw_temp,
            fault: false,
        }
    }
    pub fn set_temp(&mut self, raw_temp: f32) {
        self.raw_temp = raw_temp;
    }
    #[cfg(test)]
    pub fn set_fault(&mut self, fault: bool) {
        self.fault = fault;
    }
}

impl TempSensor for MockSensor {
    type Error = Infallible;
    fn has_fault(&mut self) -> Result<bool, Infallible> {
        Ok(self.fault)
    }
    fn read_temperature(&mut self) -> Result<f32, Infallible> {
        Ok(self.raw_temp)
    }
}

/// An SSR that records how often it was forced off (the emergency path a test
/// asserts on). Commanded duty is measured by the sim via `ssr_subtick`'s
/// return value, not recorded here.
#[derive(Debug, Clone, Copy, Default)]
pub struct MockSsr {
    pub force_off_count: u32,
}

impl MockSsr {
    pub fn new() -> Self {
        MockSsr::default()
    }
}

impl SsrOutput for MockSsr {
    type Error = Infallible;
    fn apply(&mut self, _on: bool, _now_ms: u64) -> Result<(), Infallible> {
        Ok(())
    }
    fn force_off(&mut self) -> Result<(), Infallible> {
        self.force_off_count += 1;
        Ok(())
    }
}

/// A watchdog that records its arm timeout and counts feeds.
#[derive(Debug, Clone, Copy, Default)]
pub struct CountingWatchdog {
    pub started_ms: Option<u32>,
    pub feeds: u32,
}

impl CountingWatchdog {
    pub fn new() -> Self {
        CountingWatchdog::default()
    }
}

impl Watchdog for CountingWatchdog {
    fn start(&mut self, timeout_ms: u32) {
        self.started_ms = Some(timeout_ms);
    }
    fn feed(&mut self) {
        self.feeds += 1;
    }
}
