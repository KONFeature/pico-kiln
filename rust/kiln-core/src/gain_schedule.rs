//! Continuous PID gain scheduling — port of the thermal-compensation block in
//! `kiln/control_thread.py` (the init validation at lines 132-160 and the hot-loop
//! scaling at lines 585-606).
//!
//! Heat loss rises with temperature, so a single set of PID gains that is calm at
//! 100 °C is sluggish at 1200 °C. The controller compensates by scaling the base
//! gains with a simple linear law:
//!
//! ```text
//! g(T) = 1 + h · (T − T_ambient)
//! Kp = Kp_base · g(T),  Ki = Ki_base · g(T),  Kd = Kd_base · g(T)
//! ```
//!
//! `h` is the heat-loss coefficient (typical range `1e-4 .. 1e-2`); `h = 0`
//! disables scheduling and the gains stay at their base values.
//!
//! Two faithful details:
//!
//! - **Negative `h` is clamped to 0** (disabled) at construction, mirroring the
//!   reference's validation. The reference also *warns* when `h > 0.1`
//!   ("may cause instability") but proceeds unchanged — that advisory is
//!   presentation, so it is omitted here.
//! - **Change-threshold gate.** New gains are only emitted when at least one of
//!   `|ΔKp| > 0.01`, `|ΔKi| > 0.0001`, `|ΔKd| > 0.01` (absolute thresholds, so no
//!   division by zero). Below that the PID keeps its current gains, avoiding
//!   pointless churn every tick. [`update`](GainSchedule::update) returns
//!   [`Some`] exactly when the reference would call `pid.set_gains(...)`.
//!
//! This module is the pure decision; the caller gates it on the RUNNING state and
//! applies any returned [`Gains`] to the PID, exactly as the control loop does.

/// `|x|` without `std`/libm. Matches Python `abs` for finite values.
#[inline]
fn abs(x: f64) -> f64 {
    if x < 0.0 {
        -x
    } else {
        x
    }
}

/// Minimum `|ΔKp|` before new gains are emitted — `0.01`.
pub const KP_CHANGE_THRESHOLD: f64 = 0.01;
/// Minimum `|ΔKi|` before new gains are emitted — `0.0001`.
pub const KI_CHANGE_THRESHOLD: f64 = 0.0001;
/// Minimum `|ΔKd|` before new gains are emitted — `0.01`.
pub const KD_CHANGE_THRESHOLD: f64 = 0.01;

/// A PID gain triple.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gains {
    pub kp: f64,
    pub ki: f64,
    pub kd: f64,
}

impl Gains {
    /// Construct a gain triple.
    pub const fn new(kp: f64, ki: f64, kd: f64) -> Self {
        Self { kp, ki, kd }
    }
}

/// Continuous gain scheduler: holds the base gains and the thermal law, tracks
/// the gains last emitted, and decides when a temperature change warrants new
/// PID gains.
#[derive(Debug, Clone)]
pub struct GainSchedule {
    h: f64,
    t_ambient: f64,
    base: Gains,
    current: Gains,
}

impl GainSchedule {
    /// Create a scheduler from the `base` gains, heat-loss coefficient `h`, and
    /// ambient temperature `t_ambient` (°C). A negative `h` is clamped to `0`
    /// (scheduling disabled), matching the reference validation. The "current"
    /// gains seed to `base`, so the first [`update`](Self::update) at ambient is a
    /// no-op (returns [`None`]).
    pub fn new(base: Gains, h: f64, t_ambient: f64) -> Self {
        let h = if h < 0.0 { 0.0 } else { h };
        Self {
            h,
            t_ambient,
            base,
            current: base,
        }
    }

    /// Whether scheduling is active (`h > 0`).
    pub fn enabled(&self) -> bool {
        self.h > 0.0
    }

    /// The base (unscaled) gains.
    pub fn base(&self) -> Gains {
        self.base
    }

    /// The gains most recently emitted (what the PID is currently using).
    pub fn current(&self) -> Gains {
        self.current
    }

    /// The scale factor `g(T) = 1 + h·(T − T_ambient)` at `current_temp`.
    pub fn gain_scale(&self, current_temp: f64) -> f64 {
        1.0 + self.h * (current_temp - self.t_ambient)
    }

    /// Recompute scaled gains for `current_temp`. Returns [`Some`] new [`Gains`]
    /// when scheduling is enabled *and* at least one gain moved past its change
    /// threshold (the caller should then apply them to the PID); otherwise
    /// [`None`], leaving the current gains in place.
    pub fn update(&mut self, current_temp: f64) -> Option<Gains> {
        if self.h <= 0.0 {
            return None;
        }
        let scale = 1.0 + self.h * (current_temp - self.t_ambient);
        let kp = self.base.kp * scale;
        let ki = self.base.ki * scale;
        let kd = self.base.kd * scale;

        if abs(kp - self.current.kp) > KP_CHANGE_THRESHOLD
            || abs(ki - self.current.ki) > KI_CHANGE_THRESHOLD
            || abs(kd - self.current.kd) > KD_CHANGE_THRESHOLD
        {
            self.current = Gains { kp, ki, kd };
            Some(self.current)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Realistic base gains from config.example.py defaults.
    fn base() -> Gains {
        Gains::new(25.0, 0.18, 160.0)
    }

    #[test]
    fn disabled_when_h_is_zero_or_negative() {
        let mut z = GainSchedule::new(base(), 0.0, 25.0);
        assert!(!z.enabled());
        assert_eq!(z.update(1000.0), None); // never emits when disabled

        // Negative h is clamped to 0 (disabled), not left negative.
        let mut neg = GainSchedule::new(base(), -0.01, 25.0);
        assert!(!neg.enabled());
        assert_eq!(neg.update(1000.0), None);
        assert_eq!(neg.current(), base());
    }

    #[test]
    fn no_update_at_ambient() {
        // At T = T_ambient the scale is 1.0, so gains equal base == current.
        let mut g = GainSchedule::new(base(), 0.001, 25.0);
        assert!(g.enabled());
        assert_eq!(g.update(25.0), None);
        assert_eq!(g.current(), base());
    }

    #[test]
    fn scales_gains_above_threshold() {
        let h = 0.001;
        let t = 525.0; // ΔT = 500 -> scale = 1 + 0.001*500 = 1.5
        let mut g = GainSchedule::new(base(), h, 25.0);
        let out = g.update(t).expect("large ΔT must emit new gains");
        assert!((g.gain_scale(t) - 1.5).abs() < 1e-12);
        assert!((out.kp - 25.0 * 1.5).abs() < 1e-9);
        assert!((out.ki - 0.18 * 1.5).abs() < 1e-9);
        assert!((out.kd - 160.0 * 1.5).abs() < 1e-9);
        assert_eq!(g.current(), out); // current tracks what was emitted
    }

    #[test]
    fn tiny_change_below_all_thresholds_is_suppressed() {
        let h = 0.001;
        let mut g = GainSchedule::new(base(), h, 25.0);
        // Jump to 525 (scale 1.5) and accept the new gains.
        let _ = g.update(525.0).unwrap();
        let after_jump = g.current();
        // A 0.0001 °C nudge changes Kp by 25*0.001*1e-4 = 2.5e-6 « 0.01,
        // Ki by 1.8e-8 « 1e-4, Kd by 1.6e-5 « 0.01 -> all under threshold.
        assert_eq!(g.update(525.0001), None);
        assert_eq!(g.current(), after_jump); // unchanged
    }

    #[test]
    fn ki_threshold_can_trigger_independently_of_kp_kd() {
        // Synthetic bases where Ki dominates: a scale change can move Ki past
        // 1e-4 while Kp and Kd stay under 0.01, exercising the OR-gate's Ki arm.
        let synth = Gains::new(0.001, 50.0, 0.001);
        let h = 1e-7;
        let mut g = GainSchedule::new(synth, h, 25.0);
        // ΔT = 100 -> scale = 1 + 1e-7*100 = 1.00001, Δscale = 1e-5.
        // ΔKi = 50 * 1e-5 = 5e-4 > 1e-4 ; ΔKp = ΔKd = 1e-8 « 0.01.
        let out = g.update(125.0).expect("Ki change alone must emit");
        assert!((out.ki - 50.0 * 1.00001).abs() < 1e-9);
    }

    #[test]
    fn successive_small_steps_accumulate_until_threshold() {
        // current tracks the LAST emitted gains, so sub-threshold steps that
        // accumulate past the threshold eventually emit — matching the reference
        // comparing against _current_kp, not the base. Isolate Kp (Ki=Kd=0) so it
        // is the only binding gain; with realistic bases the large Kd dominates
        // the gate and would trip first.
        let h = 0.001;
        let kp_only = Gains::new(25.0, 0.0, 0.0);
        let mut g = GainSchedule::new(kp_only, h, 25.0);
        // Kp moves 25*0.001 = 0.025 per °C. A 0.5 °C step -> ΔKp 0.0125 > 0.01 -> emit.
        let a = g.update(25.5).expect("0.5C step exceeds Kp threshold");
        assert!((a.kp - 25.0 * (1.0 + h * 0.5)).abs() < 1e-9);
        // A further 0.3 °C step (ΔKp 0.0075 vs the last emit) -> suppressed.
        assert_eq!(g.update(25.8), None);
        // One more 0.3 °C (0.6 total from last emit) -> ΔKp 0.015 > 0.01 -> emit.
        let b = g
            .update(26.1)
            .expect("accumulated change exceeds threshold");
        assert!((b.kp - 25.0 * (1.0 + h * 1.1)).abs() < 1e-9);
    }
}
