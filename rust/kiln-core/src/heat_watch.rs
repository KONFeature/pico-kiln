//! Heating-response watchdog: "we are pushing (near) full power and the measured
//! temperature is not going up".
//!
//! Rust-only safety layer (no Python counterpart). Every other guard trusts the
//! thermocouple: over-temperature compares the reading to `MAX_TEMP`, stall
//! detection compares the reading's rate to the ramp, the temperature filter only
//! rejects out-of-range values. A probe that reads **low but plausible** defeats
//! all of them at once — a shorted or degraded thermocouple (it reports the
//! junction nearest the controller), a probe pulled out of the chamber, the wrong
//! thermocouple type, or a reading frozen below the setpoint — and the controller
//! then heats at 100 % indefinitely while believing the kiln is cold: the classic
//! kiln-fire failure. Ramps were partly covered by the stall check, but holds, the
//! recovery hold and the open-loop autotuner had no check at all.
//!
//! The physics it relies on: an electric kiln at full power heats measurably until
//! it nears its top temperature. Measured on this kiln (30 s logs, 15-min windows
//! at >= 95 % output): median ~145 °C/h at 500 °C, ~93 °C/h at 750 °C, ~54 °C/h at
//! 950 °C. "At least [`HeatWatchConfig::min_rise`] °C per
//! [`HeatWatchConfig::window_s`]" (default 5 °C / 15 min = 20 °C/h) is far below
//! that, so failing it means the reading or the heating is wrong — or the kiln is
//! at its ceiling and cannot reach the setpoint (extrapolated ~1100 °C for that
//! kiln) — and all of these warrant de-energising.
//!
//! Arming: the watch only runs while the controller is short of its setpoint by
//! more than [`HeatWatchConfig::error_band`] (always, for open-loop tuning). A
//! kiln tracking its profile, holding at target, or cooling never arms it. The
//! band is the stall check's arrival band (10 °C); a reading frozen *within* the
//! band of a hold target is the one case this cannot see (the independent
//! hardware over-temperature cutoff is the backstop). The
//! verdict at the end of a window also requires the **average** commanded output
//! over the window to be at least [`HeatWatchConfig::min_output`]: a low-power
//! window is not a heating failure, it just restarts. Any rise of `min_rise`
//! restarts the window from the new temperature, so a kiln that is genuinely
//! heating never trips it. NaN inputs fail safe (count as demanding / not rising).

/// Thresholds for [`HeatWatch`]. Defaults are deliberately loose (see module docs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeatWatchConfig {
    /// Window length in seconds within which the temperature must rise by
    /// [`min_rise`](Self::min_rise). `<= 0` disables the watch.
    pub window_s: f32,
    /// Minimum rise (°C) expected per window while demanding heat.
    pub min_rise: f32,
    /// The watch arms only while `setpoint - temp` exceeds this (°C).
    pub error_band: f32,
    /// Minimum average commanded output (%) over the window for a no-rise window
    /// to count as a heating failure.
    pub min_output: f32,
}

impl Default for HeatWatchConfig {
    fn default() -> Self {
        Self {
            window_s: 900.0,
            min_rise: 5.0,
            error_band: 10.0,
            min_output: 90.0,
        }
    }
}

/// A failed window: the temperature rose only `rise` °C in `window_s` seconds
/// at an average of `avg_output` % power.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoHeatResponse {
    pub rise: f32,
    pub window_s: f32,
    pub avg_output: f32,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    start_ms: i64,
    start_temp: f32,
    output_sum: f32,
    samples: u32,
}

/// The watch itself. Feed it once per control tick with [`HeatWatch::update`].
#[derive(Debug, Clone)]
pub struct HeatWatch {
    cfg: HeatWatchConfig,
    window: Option<Window>,
}

impl HeatWatch {
    pub fn new(cfg: HeatWatchConfig) -> Self {
        Self { cfg, window: None }
    }

    /// Disarm (call whenever the controller stops heating under supervision:
    /// idle, error, a new run or tuning session).
    pub fn reset(&mut self) {
        self.window = None;
    }

    /// True while a window is open (diagnostics / tests).
    pub fn armed(&self) -> bool {
        self.window.is_some()
    }

    /// Advance one tick. `setpoint` is the controller's target (`None` for the
    /// open-loop tuner, where the commanded output alone defines demand);
    /// `output` is the commanded duty in percent. Returns an error once a full
    /// window passes without the required rise at high average power.
    pub fn update(
        &mut self,
        now_ms: i64,
        temp: f32,
        output: f32,
        setpoint: Option<f32>,
    ) -> Result<(), NoHeatResponse> {
        if self.cfg.window_s.is_nan() || self.cfg.window_s <= 0.0 {
            self.window = None;
            return Ok(());
        }
        // Fail-safe comparisons: a NaN error counts as "short of setpoint".
        let short_of_setpoint = match setpoint {
            Some(sp) => {
                let deficit = sp - temp;
                deficit.is_nan() || deficit > self.cfg.error_band
            }
            None => true,
        };
        if !short_of_setpoint {
            self.window = None;
            return Ok(());
        }
        // A NaN output is accumulated as full power (fail safe).
        let out = if output.is_nan() { 100.0 } else { output };

        let w = match self.window.as_mut() {
            None => {
                self.window = Some(Window {
                    start_ms: now_ms,
                    start_temp: temp,
                    output_sum: out,
                    samples: 1,
                });
                return Ok(());
            }
            Some(w) => w,
        };

        // A window opened on a NaN reading could never see progress (every
        // comparison with NaN is false): reseed it from the first real reading.
        // NaN readings themselves keep accumulating (fail safe).
        let reseed = w.start_temp.is_nan() && !temp.is_nan();
        if reseed || temp - w.start_temp >= self.cfg.min_rise {
            // Progress: restart the window from here.
            *w = Window {
                start_ms: now_ms,
                start_temp: temp,
                output_sum: out,
                samples: 1,
            };
            return Ok(());
        }

        w.output_sum += out;
        w.samples += 1;
        let elapsed_s = (now_ms - w.start_ms) as f32 / 1000.0;
        if elapsed_s < self.cfg.window_s {
            return Ok(());
        }

        let avg_output = w.output_sum / w.samples as f32;
        let rise = temp - w.start_temp;
        if avg_output >= self.cfg.min_output {
            return Err(NoHeatResponse {
                rise,
                window_s: elapsed_s,
                avg_output,
            });
        }
        // Mostly low power: not a heating failure. Start a fresh window.
        *w = Window {
            start_ms: now_ms,
            start_temp: temp,
            output_sum: out,
            samples: 1,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch() -> HeatWatch {
        HeatWatch::new(HeatWatchConfig::default())
    }

    /// Drive `secs` one-second ticks starting at `t0` with `temp(t)`.
    fn run(
        w: &mut HeatWatch,
        t0: i64,
        secs: i64,
        output: f32,
        sp: Option<f32>,
        temp: impl Fn(i64) -> f32,
    ) -> Result<(), (i64, NoHeatResponse)> {
        for s in t0..t0 + secs {
            w.update(s * 1000, temp(s), output, sp).map_err(|e| (s, e))?;
        }
        Ok(())
    }

    #[test]
    fn frozen_reading_at_full_power_trips_after_one_window() {
        // Dead amplifier reading 0.0 during a 1050 C hold: PID pinned at 100 %.
        let mut w = watch();
        let (at, e) = run(&mut w, 0, 2000, 100.0, Some(1050.0), |_| 0.0).unwrap_err();
        assert_eq!(at, 900);
        assert_eq!(e.rise, 0.0);
        assert_eq!(e.avg_output, 100.0);
    }

    #[test]
    fn falling_reading_at_full_power_trips() {
        // Degrading thermocouple: reading sags while we pour power in.
        let mut w = watch();
        let r = run(&mut w, 0, 2000, 100.0, Some(1000.0), |s| 900.0 - s as f32 * 0.01);
        assert!(r.is_err());
    }

    #[test]
    fn slow_but_real_heating_never_trips() {
        // 25 C/h at 100 % near the top of a tired kiln: 6.25 C per 15 min.
        let mut w = watch();
        let r = run(&mut w, 0, 6 * 3600, 100.0, Some(1280.0), |s| {
            1200.0 + s as f32 * 25.0 / 3600.0
        });
        assert!(r.is_ok());
    }

    #[test]
    fn tracking_or_holding_never_arms() {
        let mut w = watch();
        // Holding at target with the PID at 95 %: error 0 < band.
        assert!(run(&mut w, 0, 3600, 95.0, Some(1250.0), |_| 1250.0).is_ok());
        assert!(!w.armed());
        // Cooling step: setpoint below temperature.
        assert!(run(&mut w, 3600, 3600, 0.0, Some(0.0), |_| 600.0).is_ok());
        assert!(!w.armed());
    }

    #[test]
    fn low_average_power_restarts_instead_of_tripping() {
        // Short of setpoint but only 40 % output (e.g. tuning plateau step).
        let mut w = watch();
        assert!(run(&mut w, 0, 4000, 40.0, None, |_| 300.0).is_ok());
    }

    #[test]
    fn open_loop_tuning_at_full_power_without_rise_trips() {
        let mut w = watch();
        assert!(run(&mut w, 0, 1000, 100.0, None, |_| 25.0).is_err());
    }

    #[test]
    fn leaving_the_band_disarms_and_rearms_fresh() {
        let mut w = watch();
        assert!(run(&mut w, 0, 800, 100.0, Some(500.0), |_| 100.0).is_ok());
        assert!(w.armed());
        // Reaches the band: disarm.
        w.update(800_000, 495.0, 100.0, Some(500.0)).unwrap();
        assert!(!w.armed());
        // Re-arms with a fresh 15-minute window.
        assert!(run(&mut w, 801, 899, 100.0, Some(600.0), |_| 495.0).is_ok());
    }

    #[test]
    fn nan_inputs_fail_safe() {
        let mut w = watch();
        // NaN temperature: never counts as a rise; NaN output counts as 100 %.
        let r = run(&mut w, 0, 1000, f32::NAN, Some(1000.0), |_| f32::NAN);
        assert!(r.is_err());
    }

    #[test]
    fn zero_window_disables() {
        let mut w = HeatWatch::new(HeatWatchConfig {
            window_s: 0.0,
            ..HeatWatchConfig::default()
        });
        assert!(run(&mut w, 0, 5000, 100.0, Some(1000.0), |_| 0.0).is_ok());
    }

    #[test]
    fn nan_reading_at_window_start_does_not_poison_the_window() {
        // A window opened on NaN reseeds from the first real reading, so a kiln
        // that then heats normally never trips...
        let mut w = HeatWatch::new(HeatWatchConfig::default());
        assert!(w.update(0, f32::NAN, 100.0, None).is_ok());
        for s in 1..3_600i64 {
            let t = 100.0 + s as f32 * 60.0 / 3600.0;
            assert!(w.update(s * 1_000, t, 100.0, None).is_ok(), "false trip at {s}s");
        }
        // ...while readings that stay NaN still fail safe.
        let mut w = HeatWatch::new(HeatWatchConfig::default());
        let mut tripped = false;
        for s in 0..1_000i64 {
            if w.update(s * 1_000, f32::NAN, 100.0, None).is_err() {
                tripped = true;
                break;
            }
        }
        assert!(tripped, "a NaN reading at full power must still trip");
    }

    #[test]
    fn arms_only_beyond_the_arrival_band() {
        // Within 10 C of the setpoint (tracking / holding) the watch stays
        // disarmed even at full power; just beyond it, a flat reading trips.
        let mut w = HeatWatch::new(HeatWatchConfig::default());
        for s in 0..3_600i64 {
            assert!(w.update(s * 1_000, 990.0, 100.0, Some(1000.0)).is_ok());
            assert!(!w.armed());
        }
        let mut w = HeatWatch::new(HeatWatchConfig::default());
        let mut tripped_at = None;
        for s in 0..3_600i64 {
            if w.update(s * 1_000, 989.0, 100.0, Some(1000.0)).is_err() {
                tripped_at = Some(s);
                break;
            }
        }
        assert_eq!(tripped_at, Some(900));
    }
}
