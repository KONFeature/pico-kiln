//! Solid-state-relay output.
//!
//! A thin actuation wrapper over an `embedded_hal::digital::OutputPin`,
//! active-high (high = relay energised), mirroring `kiln/hardware.py`'s
//! `pin.value(1)`. Construction drives the pin **low** so the relay starts off,
//! and a `Drop` guard drives it low again — a power-fail / panic de-energises
//! the kiln.
//!
//! The time-proportional duty-cycle scheduling (`set_output`, the locked duty,
//! the minimum on-time floor) is pure decision logic and lives in `kiln-core`
//! (`ssr_schedule`); this driver only flips the pin it is told to.

use embedded_hal::digital::OutputPin;

/// A single solid-state relay on one output pin.
pub struct Ssr<P: OutputPin> {
    pin: P,
    on: bool,
}

impl<P: OutputPin> Ssr<P> {
    /// Wrap a pin and drive it low (relay off) so the initial state is known.
    pub fn new(mut pin: P) -> Result<Self, P::Error> {
        pin.set_low()?;
        Ok(Self { pin, on: false })
    }

    /// Drive the relay on (high) or off (low).
    pub fn set(&mut self, on: bool) -> Result<(), P::Error> {
        if on {
            self.pin.set_high()?;
        } else {
            self.pin.set_low()?;
        }
        self.on = on;
        Ok(())
    }

    /// Energise the relay.
    pub fn on(&mut self) -> Result<(), P::Error> {
        self.set(true)
    }

    /// De-energise the relay.
    pub fn off(&mut self) -> Result<(), P::Error> {
        self.set(false)
    }

    /// Last commanded state (`true` = energised).
    pub fn is_on(&self) -> bool {
        self.on
    }
}

impl<P: OutputPin> Drop for Ssr<P> {
    fn drop(&mut self) {
        let _ = self.pin.set_low();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_hal_mock::eh1::digital::{Mock as PinMock, State as PinState, Transaction as Pin};

    #[test]
    fn starts_low_and_tracks_state() {
        let expect = [
            Pin::set(PinState::Low),  // new()
            Pin::set(PinState::High), // on()
            Pin::set(PinState::Low),  // off()
            Pin::set(PinState::Low),  // drop guard
        ];
        let mut pin = PinMock::new(&expect);
        {
            let mut ssr = Ssr::new(pin.clone()).unwrap();
            assert!(!ssr.is_on());
            ssr.on().unwrap();
            assert!(ssr.is_on());
            ssr.off().unwrap();
            assert!(!ssr.is_on());
        }
        pin.done();
    }

    #[test]
    fn drop_forces_off_even_after_on() {
        let expect = [
            Pin::set(PinState::Low),  // new()
            Pin::set(PinState::High), // on()
            Pin::set(PinState::Low),  // drop guard de-energises
        ];
        let mut pin = PinMock::new(&expect);
        {
            let mut ssr = Ssr::new(pin.clone()).unwrap();
            ssr.on().unwrap();
        }
        pin.done();
    }
}
