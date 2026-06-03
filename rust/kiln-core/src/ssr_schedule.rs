//! Time-proportional SSR duty scheduling — the pure decision half of
//! `kiln/hardware.py`'s `SSRController` (everything except `pin.value()`).
//!
//! A solid-state relay must not be switched at high frequency, so the kiln drives
//! it with *slow PWM*: each `cycle_time` window is ON for `duty%` of its length
//! and OFF for the rest. This module owns that decision; flipping the actual GPIO
//! (and any multi-SSR stagger) stays in `kiln-hal::ssr`.
//!
//! Faithful-port details, all mirrored branch-for-branch from the reference
//! (validated by `tests/replay_ssr_schedule.rs`):
//!
//! - **Minimum on-time floor.** Any non-zero request is floored to
//!   [`MIN_SSR_OUTPUT`] so the relay still gets a usable pulse (5 % of a 20 s
//!   cycle is a 1 s minimum), while an exact `0` stays fully off.
//! - **Mid-cycle duty lock.** The requested duty can change at any time, but the
//!   *applied* duty is latched at the start of each cycle so a moving setpoint
//!   can't chatter the relay mid-window; the new value takes effect next cycle.
//! - **Single-cycle advance.** Like the reference, [`update`](SsrSchedule::update)
//!   advances the cycle boundary by exactly one `cycle_time` per call even if more
//!   than one elapsed — if the loop ever falls badly behind, the window re-aligns
//!   over the next few ticks rather than fast-forwarding.
//!
//! As elsewhere in `kiln-core`, **time is injected**: pass a monotonic
//! millisecond clock (`now_ms`) instead of reading `time.ticks_ms()`, so the
//! schedule is deterministic and host-testable.

/// Floor applied to any non-zero duty request, in percent — `MIN_SSR_OUTPUT`.
/// Guarantees a minimum relay on-time (5 % × cycle) once the PID asks for heat.
pub const MIN_SSR_OUTPUT: f64 = 5.0;

/// Time-proportional duty scheduler for a solid-state relay.
///
/// Construct with the cycle period and the current clock, push the requested
/// duty with [`set_output`](Self::set_output), then call [`update`](Self::update)
/// frequently (the reference runs it at 10 Hz) to get the ON/OFF the relay should
/// hold right now.
#[derive(Debug, Clone)]
pub struct SsrSchedule {
    cycle_time_ms: u32,
    duty_cycle: f64,
    duty_cycle_locked: f64,
    cycle_start_ms: u64,
}

impl SsrSchedule {
    /// Create a scheduler with a `cycle_time_s`-second window, seeding the cycle
    /// start from `now_ms`. Both the requested and locked duty start at `0`
    /// (relay off), matching the `SSRController` constructor.
    ///
    /// `cycle_time_s` is truncated to whole milliseconds exactly as the reference
    /// (`int(cycle_time * 1000)`).
    pub fn new(cycle_time_s: f64, now_ms: u64) -> Self {
        Self {
            cycle_time_ms: (cycle_time_s * 1000.0) as u32,
            duty_cycle: 0.0,
            duty_cycle_locked: 0.0,
            cycle_start_ms: now_ms,
        }
    }

    /// Set the requested output percentage. A request `> 0` is clamped to
    /// `[MIN_SSR_OUTPUT, 100]`; an exact `0` (or negative) requests full off.
    /// The change only reaches the relay at the next cycle boundary (see the
    /// mid-cycle lock in [`update`](Self::update)).
    pub fn set_output(&mut self, percent: f64) {
        if percent > 0.0 {
            // max(MIN_SSR_OUTPUT, min(100.0, percent))
            let capped = if percent < 100.0 { percent } else { 100.0 };
            self.duty_cycle = if capped > MIN_SSR_OUTPUT {
                capped
            } else {
                MIN_SSR_OUTPUT
            };
        } else {
            self.duty_cycle = 0.0;
        }
    }

    /// Advance the schedule to `now_ms` and return whether the relay should be ON.
    ///
    /// At a cycle boundary the requested duty is latched and the boundary moves
    /// forward by one `cycle_time`; within a cycle the relay is ON for the first
    /// `duty%` of the window. `now_ms` must be monotonic non-decreasing.
    pub fn update(&mut self, now_ms: u64) -> bool {
        let mut elapsed = now_ms.saturating_sub(self.cycle_start_ms);

        // New cycle: lock the duty (prevents mid-cycle relay chatter) and step the
        // boundary forward by exactly one cycle, like the reference.
        if elapsed >= self.cycle_time_ms as u64 {
            self.duty_cycle_locked = self.duty_cycle;
            self.cycle_start_ms = self.cycle_start_ms.wrapping_add(self.cycle_time_ms as u64);
            elapsed = now_ms.saturating_sub(self.cycle_start_ms);
        }

        // ON for the first `duty%` of the window, using the LOCKED duty.
        // `int(...)` truncation toward zero (duty_locked and cycle_time are >= 0).
        let on_time_ms = ((self.duty_cycle_locked / 100.0) * self.cycle_time_ms as f64) as u64;
        elapsed < on_time_ms
    }

    /// Emergency stop: zero both the requested and locked duty. The caller must
    /// de-energise the relay immediately (this only updates the schedule state;
    /// it does not wait for the next [`update`](Self::update)).
    pub fn force_off(&mut self) {
        self.duty_cycle = 0.0;
        self.duty_cycle_locked = 0.0;
    }

    /// The requested duty (may differ from what's applied until the next cycle).
    pub fn duty_cycle(&self) -> f64 {
        self.duty_cycle
    }

    /// The duty currently being applied (latched at the last cycle boundary).
    pub fn duty_cycle_locked(&self) -> f64 {
        self.duty_cycle_locked
    }

    /// Configured cycle period in milliseconds (`int(cycle_time * 1000)`).
    pub fn cycle_time_ms(&self) -> u32 {
        self.cycle_time_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_output_floors_nonzero_and_clamps() {
        let mut s = SsrSchedule::new(2.0, 0);
        s.set_output(0.0);
        assert_eq!(s.duty_cycle(), 0.0); // exact zero stays fully off
        s.set_output(3.0);
        assert_eq!(s.duty_cycle(), MIN_SSR_OUTPUT); // floored to 5%
        s.set_output(5.0);
        assert_eq!(s.duty_cycle(), 5.0); // exactly the floor
        s.set_output(42.5);
        assert_eq!(s.duty_cycle(), 42.5); // passed through
        s.set_output(150.0);
        assert_eq!(s.duty_cycle(), 100.0); // clamped to 100
        s.set_output(-10.0);
        assert_eq!(s.duty_cycle(), 0.0); // negative -> off
    }

    #[test]
    fn duty_is_locked_at_cycle_start_not_mid_cycle() {
        // 2 s cycle. Request 50% and lock it at t=0's first boundary.
        let mut s = SsrSchedule::new(2.0, 0);
        s.set_output(50.0);
        // First update at/after a full cycle locks 50%.
        assert!(s.update(2000)); // boundary: lock 50%, elapsed 0 < 1000 -> ON
        assert_eq!(s.duty_cycle_locked(), 50.0);

        // Mid-cycle change to 100% must NOT take effect until the next boundary.
        s.set_output(100.0);
        assert_eq!(s.duty_cycle(), 100.0); // requested updated
        assert!(s.update(2500)); // still in the locked 50% cycle, elapsed 500 < 1000
        assert_eq!(s.duty_cycle_locked(), 50.0); // unchanged
        assert!(!s.update(3500)); // elapsed 1500 >= on_time 1000 -> OFF, still 50% locked
        assert_eq!(s.duty_cycle_locked(), 50.0);

        // Next boundary (t=4000) latches the new 100%.
        assert!(s.update(4000));
        assert_eq!(s.duty_cycle_locked(), 100.0);
    }

    #[test]
    fn time_proportional_on_then_off_within_cycle() {
        let mut s = SsrSchedule::new(2.0, 0);
        s.set_output(25.0);
        // Lock 25% at the first boundary: on_time = 0.25 * 2000 = 500 ms.
        assert!(s.update(2000)); // elapsed 0 < 500 -> ON
        assert_eq!(s.duty_cycle_locked(), 25.0);
        assert!(s.update(2499)); // 499 < 500 -> ON
        assert!(!s.update(2500)); // 500 < 500 is false -> OFF
        assert!(!s.update(3999)); // remainder of cycle -> OFF
    }

    #[test]
    fn zero_duty_is_always_off_full_duty_always_on() {
        let mut off = SsrSchedule::new(2.0, 0);
        off.set_output(0.0);
        assert!(!off.update(2000));
        assert!(!off.update(2001));

        let mut full = SsrSchedule::new(2.0, 0);
        full.set_output(100.0);
        assert!(full.update(2000)); // on_time = cycle; ON across the whole window
        assert!(full.update(3999));
        assert!(full.update(4000)); // next boundary re-locks 100% (0 elapsed < on_time) -> still ON
    }

    #[test]
    fn force_off_zeroes_requested_and_locked() {
        let mut s = SsrSchedule::new(2.0, 0);
        s.set_output(80.0);
        s.update(2000); // lock 80%
        assert_eq!(s.duty_cycle_locked(), 80.0);
        s.force_off();
        assert_eq!(s.duty_cycle(), 0.0);
        assert_eq!(s.duty_cycle_locked(), 0.0);
        assert!(!s.update(2500)); // off for the rest of the cycle
    }

    #[test]
    fn cycle_time_truncates_to_whole_milliseconds() {
        // 0.0015 s -> int(1.5) = 1 ms, mirroring the reference constructor.
        let s = SsrSchedule::new(0.0015, 0);
        assert_eq!(s.cycle_time_ms(), 1);
    }
}
