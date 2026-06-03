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

impl<P: OutputPin> crate::platform::SsrOutput for Ssr<P> {
    type Error = P::Error;

    fn apply(&mut self, on: bool, _now_ms: u64) -> Result<(), Self::Error> {
        self.set(on)
    }

    fn force_off(&mut self) -> Result<(), Self::Error> {
        self.off()
    }
}

/// `N` solid-state relays driven as one logical output, with staggered turn-on.
///
/// Mirrors `SSRController` driving a *list* of pins: when the duty schedule asks
/// for ON, the relays energise one at a time spaced `stagger_ms` apart instead of
/// all at once, limiting inrush current on high-power kilns. The reference does
/// this with a blocking `sleep(stagger_delay)` between pins; here the stagger is
/// non-blocking — [`apply`](MultiSsr::apply) is called every 10 Hz sub-tick with
/// a monotonic `now_ms`, and each relay latches on once its delay has elapsed.
/// Turn-OFF is staggered the same way (`SSRController.update` spaces the
/// `pin.value(0)` calls by `stagger_delay`, `kiln/hardware.py:317-324`); only
/// [`force_off`](MultiSsr::force_off) — the emergency path — drops every relay at
/// once (no stagger, for safety, matching `SSRController.force_off`). A `Drop`
/// guard drives every pin low, matching [`Ssr`].
pub struct MultiSsr<P: OutputPin, const N: usize> {
    pins: [P; N],
    stagger_ms: u64,
    on: bool,
    rising_edge_ms: u64,
    falling_edge_ms: u64,
    pins_on: [bool; N],
}

impl<P: OutputPin, const N: usize> MultiSsr<P, N> {
    /// Wrap `N` pins and drive them all low (relays off) so the initial state is
    /// known. `stagger_ms` is the delay between successive relay turn-ons.
    pub fn new(mut pins: [P; N], stagger_ms: u64) -> Result<Self, P::Error> {
        for pin in &mut pins {
            pin.set_low()?;
        }
        Ok(Self {
            pins,
            stagger_ms,
            on: false,
            rising_edge_ms: 0,
            falling_edge_ms: 0,
            pins_on: [false; N],
        })
    }

    /// Last commanded logical state (`true` = the relays should be energised).
    pub fn is_on(&self) -> bool {
        self.on
    }

    fn all_off(&mut self) -> Result<(), P::Error> {
        for pin in &mut self.pins {
            pin.set_low()?;
        }
        self.on = false;
        self.pins_on = [false; N];
        Ok(())
    }
}

impl<P: OutputPin, const N: usize> crate::platform::SsrOutput for MultiSsr<P, N> {
    type Error = P::Error;

    fn apply(&mut self, on: bool, now_ms: u64) -> Result<(), Self::Error> {
        if on {
            // Rising edge: latch the moment so each relay turns on once its
            // stagger delay has elapsed.
            if !self.on {
                self.on = true;
                self.rising_edge_ms = now_ms;
            }
            let elapsed = now_ms.saturating_sub(self.rising_edge_ms);
            for i in 0..N {
                if !self.pins_on[i] && elapsed >= (i as u64) * self.stagger_ms {
                    self.pins[i].set_high()?;
                    self.pins_on[i] = true;
                }
            }
        } else {
            // Falling edge: latch the moment, then de-energise each relay once
            // its stagger delay has elapsed — the same spacing as turn-on
            // (`SSRController.update`, kiln/hardware.py:317-324). Emergency stops
            // go through `force_off`, which drops everything at once.
            if self.on {
                self.on = false;
                self.falling_edge_ms = now_ms;
            }
            let elapsed = now_ms.saturating_sub(self.falling_edge_ms);
            for i in 0..N {
                if self.pins_on[i] && elapsed >= (i as u64) * self.stagger_ms {
                    self.pins[i].set_low()?;
                    self.pins_on[i] = false;
                }
            }
        }
        Ok(())
    }

    fn force_off(&mut self) -> Result<(), Self::Error> {
        self.all_off()
    }
}

impl<P: OutputPin, const N: usize> Drop for MultiSsr<P, N> {
    fn drop(&mut self) {
        for pin in &mut self.pins {
            let _ = pin.set_low();
        }
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

    #[test]
    fn multi_ssr_staggers_turn_on_and_turn_off() {
        use crate::platform::SsrOutput;
        // Both edges are staggered by `stagger_ms` (= 10): pin 0 acts at the
        // edge, pin 1 one stagger later — mirroring SSRController.update.
        let expect0 = [
            Pin::set(PinState::Low),  // new()
            Pin::set(PinState::High), // apply(true, 0): pin 0 fires immediately
            Pin::set(PinState::Low),  // apply(false, 20): pin 0 off immediately
            Pin::set(PinState::Low),  // drop
        ];
        let expect1 = [
            Pin::set(PinState::Low),  // new()
            Pin::set(PinState::High), // apply(true, 10): pin 1 fires after stagger
            Pin::set(PinState::Low),  // apply(false, 30): pin 1 off after stagger
            Pin::set(PinState::Low),  // drop
        ];
        let mut p0 = PinMock::new(&expect0);
        let mut p1 = PinMock::new(&expect1);
        {
            let mut m = MultiSsr::<_, 2>::new([p0.clone(), p1.clone()], 10).unwrap();
            assert!(!m.is_on());
            m.apply(true, 0).unwrap(); // pin 0 only
            assert!(m.is_on());
            m.apply(true, 5).unwrap(); // pin 1 not yet (5 < 10): no transitions
            m.apply(true, 10).unwrap(); // pin 1 now fires
            m.apply(false, 20).unwrap(); // logical OFF + pin 0 de-energises now
            assert!(!m.is_on()); // logical state flips immediately
            m.apply(false, 25).unwrap(); // pin 1 not yet (25 - 20 = 5 < 10)
            m.apply(false, 30).unwrap(); // pin 1 de-energises after the stagger
        }
        p0.done();
        p1.done();
    }

    #[test]
    fn multi_ssr_force_off_drops_all_at_once() {
        use crate::platform::SsrOutput;
        // Emergency stop: no stagger, both relays drop together.
        let expect0 = [
            Pin::set(PinState::Low),  // new()
            Pin::set(PinState::High), // apply(true, 0)
            Pin::set(PinState::Low),  // force_off
            Pin::set(PinState::Low),  // drop
        ];
        let expect1 = [
            Pin::set(PinState::Low),  // new()
            Pin::set(PinState::High), // apply(true, 10)
            Pin::set(PinState::Low),  // force_off (immediate, no stagger)
            Pin::set(PinState::Low),  // drop
        ];
        let mut p0 = PinMock::new(&expect0);
        let mut p1 = PinMock::new(&expect1);
        {
            let mut m = MultiSsr::<_, 2>::new([p0.clone(), p1.clone()], 10).unwrap();
            m.apply(true, 0).unwrap();
            m.apply(true, 10).unwrap(); // both on
            m.force_off().unwrap(); // both off immediately
            assert!(!m.is_on());
        }
        p0.done();
        p1.done();
    }
}
