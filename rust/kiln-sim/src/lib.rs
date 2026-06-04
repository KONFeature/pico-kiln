//! Host simulator for the pico-kiln controller.
//!
//! [`simulate`] runs the **real** [`kiln_control::Controller`] — the exact code
//! that ships on Core 1 — against a software [`ThermalModel`], using the host
//! mocks for the sensor/SSR/watchdog and a simulated clock. Each tick: feed the
//! model temperature to the sensor, run one real control iteration plus its 10 Hz
//! SSR sub-ticks, measure the resulting duty, and integrate the model forward.
//! This exercises the actual sensor→state→PID→SSR→watchdog path end to end, so a
//! green firing here is strong evidence the shipping loop is correct — no
//! hardware required.

use kiln_control::mock::{CountingWatchdog, MockSensor, MockSsr};
use kiln_control::{ControlParams, Controller};
use kiln_core::profile::Profile;
use kiln_core::protocol::{Command, ProfileName};
use kiln_core::state::KilnState;

/// A first-order lumped thermal model: the temperature rises with applied power
/// and bleeds heat to ambient. `dT = dt·(duty·heat_rate − loss_coeff·(T−ambient))`.
#[derive(Debug, Clone, Copy)]
pub struct ThermalModel {
    pub temp: f64,
    pub ambient: f64,
    /// °C/s at 100 % duty (the heater authority).
    pub heat_rate: f64,
    /// Per-second heat-loss coefficient (sets the equilibrium temperature).
    pub loss_coeff: f64,
}

impl ThermalModel {
    /// A well-insulated kiln: ~3 °C/min at full power, equilibrium ~525 °C, so a
    /// mid-range hold settles at a comfortable partial duty. Slow enough to suit
    /// the kiln-tuned PID gains.
    pub fn kiln() -> Self {
        ThermalModel {
            temp: 25.0,
            ambient: 25.0,
            heat_rate: 0.05,
            loss_coeff: 0.0001,
        }
    }

    /// Integrate one step of `dt_s` seconds at the given `duty` fraction (0..=1).
    pub fn step(&mut self, duty: f64, dt_s: f64) {
        let delta = dt_s * (duty * self.heat_rate - self.loss_coeff * (self.temp - self.ambient));
        self.temp += delta;
    }
}

/// One recorded simulation step.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub t_s: f64,
    pub temp: f64,
    pub target: f64,
    pub ssr: f64,
    pub state: KilnState,
}

/// The full simulation trace plus the watchdog feed count (one per clean tick).
pub struct SimResult {
    pub samples: Vec<Sample>,
    pub watchdog_feeds: u32,
}

/// Run `profile` (named `name`) through the real control loop for `ticks` outer
/// iterations against `model`, returning the trace.
pub fn simulate(name: &str, profile: Profile, ticks: u32, mut model: ThermalModel) -> SimResult {
    let params = ControlParams::default();
    let dt_s = params.temp_read_interval_ms as f64 / 1000.0;
    let mut c = Controller::new(
        MockSensor::new(model.temp as f32),
        MockSsr::new(),
        CountingWatchdog::new(),
        params,
        0,
    );
    let (sub_ticks, sub_ms) = c.timing();

    // Warm-up idle tick so the loop has read the sensor before the run command,
    // exactly as the real loop is already reading temperature when one arrives.
    c.sensor_mut().set_temp(model.temp as f32);
    c.iterate(None, 0, 0);

    let mut now_ms = params.temp_read_interval_ms;
    // Host time axis (Sample.t_s) stays f64; the controller takes integer Unix
    // seconds for its wall clock.
    let mut wall = now_ms as f64 / 1000.0;
    c.sensor_mut().set_temp(model.temp as f32);
    c.iterate(
        Some(Command::RunProfile {
            profile: ProfileName::new(name).unwrap(),
            parsed: profile,
        }),
        now_ms,
        (now_ms / 1000) as i64,
    );

    let mut samples = Vec::with_capacity(ticks as usize);
    for _ in 0..ticks {
        now_ms += params.temp_read_interval_ms;
        wall = now_ms as f64 / 1000.0;

        c.sensor_mut().set_temp(model.temp as f32);
        let _ = c.iterate(None, now_ms, (now_ms / 1000) as i64);

        let mut on_count = 0u32;
        for s in 0..sub_ticks {
            let sub_now = now_ms + (s as u64) * sub_ms;
            if c.ssr_subtick(sub_now).unwrap() {
                on_count += 1;
            }
        }
        let duty = on_count as f64 / sub_ticks as f64;
        model.step(duty, dt_s);

        let snap = c.snapshot(now_ms, (now_ms / 1000) as i64);
        samples.push(Sample {
            t_s: wall,
            temp: model.temp,
            target: snap.target_temp as f64,
            ssr: snap.ssr_output as f64,
            state: snap.state,
        });
    }

    SimResult {
        samples,
        watchdog_feeds: c.watchdog().feeds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiln_core::profile::Step;

    #[test]
    fn closed_loop_reaches_and_holds_setpoint() {
        let profile = Profile::new(&[Step::hold(150.0, 1.0e9)]).unwrap();
        let r = simulate("holdtest", profile, 8_000, ThermalModel::kiln());

        let last = r.samples.last().unwrap();
        assert_eq!(
            last.state,
            KilnState::Running,
            "hold step must keep running"
        );
        assert!(
            (last.temp - 150.0).abs() < 10.0,
            "closed loop should settle near the 150C setpoint, got {:.1}",
            last.temp
        );

        // Every clean tick fed the watchdog: warm-up + run + `ticks`.
        assert!(r.watchdog_feeds >= 8_000, "watchdog fed each clean tick");

        // It actually climbed under strong heat (not stuck), and then throttled.
        let max_ssr = r.samples.iter().map(|s| s.ssr).fold(0.0_f64, f64::max);
        assert!(
            max_ssr > 50.0,
            "PID should command strong heat while climbing, peak was {:.1}%",
            max_ssr
        );
        let peak_temp = r.samples.iter().map(|s| s.temp).fold(0.0_f64, f64::max);
        assert!(
            peak_temp <= 200.0,
            "a sane controller must not wildly overshoot, peak {:.1}",
            peak_temp
        );
    }
}
