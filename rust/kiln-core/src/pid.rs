//! PID controller with conditional-integration anti-windup.
//!
//! Direct port of `kiln/pid.py`. The arithmetic is mirrored expression-for-
//! expression and left-to-right so the `f64` results match the reference
//! implementation bit-for-bit (validated by `tests/replay_pid.rs`).
//!
//! Anti-windup uses conditional integration (Åström & Hägglund): the integral
//! is frozen when the predicted P+I output is already saturated *in the same
//! direction as the error*, so it can still unwind when the error reverses.
//!
//! Pure `core` — no `std`, no `alloc`, no float intrinsics (only `+ - * /` and
//! comparisons), so it runs identically on the host and on the RP2350.

/// Clamp `x` to `[lo, hi]` using the same comparison order as Python's
/// `max(min(x, hi), lo)`. Avoids `f64::min`/`max` to stay dependency- and
/// libm-free for `no_std`.
#[inline]
fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    let m = if x < hi { x } else { hi }; // min(x, hi)
    if m > lo {
        m
    } else {
        lo
    } // max(m, lo)
}

/// Per-update diagnostics, mirroring the Python `pid.stats` dict fields used
/// downstream (status messages, tuning logs).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PidStats {
    pub dt: f32,
    pub setpoint: f32,
    pub measured: f32,
    pub error: f32,
    pub p_term: f32,
    pub i_term: f32,
    pub d_term: f32,
    pub output: f32,
    /// Output before clamping to the limits.
    pub output_raw: f32,
    /// True when the integral was held this step (anti-windup engaged).
    pub integral_frozen: bool,
}

/// PID controller. Construct with [`Pid::new`], drive with [`Pid::update`].
#[derive(Debug, Clone)]
pub struct Pid {
    pub kp: f32,
    pub ki: f32,
    pub kd: f32,
    out_min: f32,
    out_max: f32,

    prev_error: f32,
    integral: f32,
    // Absolute monotonic-seconds anchor stays f64; only the small `dt` derived
    // from it is narrowed to f32 (see `update`).
    prev_time: Option<f64>,

    stats: PidStats,
}

impl Pid {
    /// Create a controller with output clamped to `[out_min, out_max]`.
    ///
    /// Mirrors `PID(kp, ki, kd, output_limits=(out_min, out_max))`.
    pub fn new(kp: f32, ki: f32, kd: f32, out_min: f32, out_max: f32) -> Self {
        Self {
            kp,
            ki,
            kd,
            out_min,
            out_max,
            prev_error: 0.0,
            integral: 0.0,
            prev_time: None,
            stats: PidStats::default(),
        }
    }

    /// Compute the control output for `setpoint` vs `measured` at time `now`
    /// (seconds, monotonic). The first call uses `dt = 1.0` exactly as the
    /// reference does; thereafter `dt = now - prev_time`, floored at `0.001`.
    pub fn update(&mut self, setpoint: f32, measured: f32, now: f64) -> f32 {
        // --- dt (matches pid.py: first call -> 1.0; non-positive -> 0.001) ---
        // `now`/`prev_time` are large f64 monotonic seconds; the subtraction is
        // done in f64, then the small dt is narrowed to f32 — so the timestamp
        // magnitude never costs PID precision.
        let dt: f32 = match self.prev_time {
            None => 1.0,
            Some(prev) => {
                let d = now - prev;
                (if d <= 0.0 { 0.001 } else { d }) as f32
            }
        };

        let error = setpoint - measured;

        // Proportional.
        let p_term = self.kp * error;

        // Derivative (computed before the integral, as in the reference, so the
        // conditional-integration check can reason about P+I only).
        let error_delta = error - self.prev_error;
        let d_term = self.kd * (error_delta / dt);

        // Integral with conditional-integration anti-windup.
        let saturated_high;
        let saturated_low;
        if self.ki > 0.0 {
            let candidate_integral = self.integral + error * dt;
            let pi_candidate = p_term + self.ki * candidate_integral;

            saturated_high = pi_candidate >= self.out_max && error > 0.0;
            saturated_low = pi_candidate <= self.out_min && error < 0.0;

            if !(saturated_high || saturated_low) {
                self.integral = candidate_integral;
            }

            // Hard safety clamp on the accumulator itself.
            let integral_max = self.out_max / self.ki;
            let integral_min = self.out_min / self.ki;
            self.integral = clamp(self.integral, integral_min, integral_max);
        } else {
            self.integral = 0.0;
            saturated_high = false;
            saturated_low = false;
        }

        let i_term = self.ki * self.integral;

        let output_raw = p_term + i_term + d_term;
        let output = clamp(output_raw, self.out_min, self.out_max);

        self.prev_error = error;
        self.prev_time = Some(now);

        self.stats = PidStats {
            dt,
            setpoint,
            measured,
            error,
            p_term,
            i_term,
            d_term,
            output,
            output_raw,
            integral_frozen: saturated_high || saturated_low,
        };

        output
    }

    /// Reset runtime state (preserves gains). Mirrors `PID.reset()`.
    pub fn reset(&mut self) {
        self.prev_error = 0.0;
        self.integral = 0.0;
        self.prev_time = None;
        self.stats = PidStats::default();
    }

    /// Update gains on the fly with bumpless transfer for `ki` changes
    /// (preserves `i_term` so the output doesn't jump). Mirrors
    /// `PID.set_gains()`. `None` leaves a gain unchanged.
    pub fn set_gains(&mut self, kp: Option<f32>, ki: Option<f32>, kd: Option<f32>) {
        let old_ki = self.ki;
        if let Some(v) = kp {
            self.kp = v;
        }
        if let Some(v) = ki {
            self.ki = v;
        }
        if let Some(v) = kd {
            self.kd = v;
        }

        if self.ki > 0.0 && old_ki > 0.0 && self.ki != old_ki {
            self.integral *= old_ki / self.ki;
        } else if self.ki == 0.0 {
            self.integral = 0.0;
        }
    }

    /// Diagnostics from the most recent [`update`](Self::update). Used by the
    /// golden replay test (`tests/replay_pid.rs`).
    pub fn stats(&self) -> PidStats {
        self.stats
    }

    /// Current integral accumulator (for inspection/tests).
    #[cfg(test)]
    pub fn integral(&self) -> f32 {
        self.integral
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_call_uses_unit_dt_and_zero_error_gives_zero() {
        let mut pid = Pid::new(25.0, 0.14, 160.0, 0.0, 100.0);
        let out = pid.update(20.0, 20.0, 0.0);
        assert_eq!(out, 0.0);
        assert_eq!(pid.stats().dt, 1.0);
        assert!(!pid.stats().integral_frozen);
    }

    #[test]
    fn positive_error_drives_output_up_and_clamps() {
        let mut pid = Pid::new(25.0, 0.14, 160.0, 0.0, 100.0);
        let out = pid.update(200.0, 20.0, 0.0);
        assert_eq!(out, 100.0); // huge error saturates high
        assert!(pid.stats().output_raw > 100.0);
    }

    #[test]
    fn anti_windup_freezes_integral_when_saturated_high() {
        let mut pid = Pid::new(25.0, 0.14, 160.0, 0.0, 100.0);
        pid.update(500.0, 20.0, 0.0);
        let frozen = pid.update(500.0, 21.0, 1.0);
        assert!(frozen >= 99.0);
        assert!(
            pid.stats().integral_frozen,
            "integral should be held while saturated high"
        );
    }

    #[test]
    fn negative_dt_is_floored() {
        let mut pid = Pid::new(1.0, 0.0, 0.0, -1000.0, 1000.0);
        pid.update(10.0, 0.0, 5.0);
        pid.update(10.0, 0.0, 5.0); // dt would be 0 -> floored to 0.001
        assert_eq!(pid.stats().dt, 0.001);
    }

    #[test]
    fn set_gains_bumpless_preserves_i_term() {
        let mut pid = Pid::new(10.0, 0.2, 0.0, 0.0, 100.0);
        // Build up some integral within range.
        pid.update(50.0, 49.0, 0.0);
        pid.update(50.0, 49.5, 1.0);
        let i_before = pid.stats().i_term;
        pid.set_gains(None, Some(0.4), None);
        // i_term = ki * integral should be preserved across the ki change.
        let approx = (pid.ki * pid.integral() - i_before).abs() < 1e-9;
        assert!(
            approx,
            "i_term not preserved: {} vs {}",
            pid.ki * pid.integral(),
            i_before
        );
    }

    #[test]
    fn reset_clears_state() {
        let mut pid = Pid::new(25.0, 0.14, 160.0, 0.0, 100.0);
        pid.update(200.0, 20.0, 0.0);
        pid.reset();
        assert_eq!(pid.integral(), 0.0);
        // After reset the next call is a "first call" again -> dt 1.0.
        pid.update(100.0, 90.0, 99.0);
        assert_eq!(pid.stats().dt, 1.0);
    }
}
