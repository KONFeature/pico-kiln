//! Control-loop parameters — the `config.py` values the loop needs that are not
//! already in [`ControllerConfig`]. Defaults match `config.example.py`.

use kiln_core::gain_schedule::Gains;
use kiln_core::state::ControllerConfig;

/// Status publish cadence — `STATUS_UPDATE_INTERVAL = 5 s`, in milliseconds.
pub const DEFAULT_STATUS_INTERVAL_MS: u64 = 5_000;

/// Tunable parameters injected into [`Controller::new`](crate::Controller::new).
#[derive(Debug, Clone, Copy)]
pub struct ControlParams {
    /// Safety / rate / stall config (max temp, windows, stall thresholds).
    pub controller: ControllerConfig,
    /// Base PID gains (`PID_KP_BASE` / `PID_KI_BASE` / `PID_KD_BASE`).
    pub pid_base: Gains,
    /// Heat-loss coefficient for gain scheduling (`THERMAL_H`; 0 disables).
    pub thermal_h: f64,
    /// Ambient reference for gain scheduling (`THERMAL_T_AMBIENT`).
    pub thermal_t_ambient: f64,
    /// SSR time-proportional cycle period (`SSR_CYCLE_TIME`), seconds.
    pub ssr_cycle_time_s: f64,
    /// Calibration offset added to every reading (`THERMOCOUPLE_OFFSET`).
    pub thermocouple_offset: f64,
    /// Software median window (`TEMP_MEDIAN_WINDOW`); clamped to the filter cap.
    pub median_window: usize,
    /// Status publish cadence in milliseconds (`STATUS_UPDATE_INTERVAL`).
    pub status_update_interval_ms: u64,
    /// Watchdog timeout (`WATCHDOG_TIMEOUT`), milliseconds.
    pub watchdog_timeout_ms: u32,
    /// Outer control-tick period (`TEMP_READ_INTERVAL`), milliseconds.
    pub temp_read_interval_ms: u64,
    /// SSR sub-tick period (`SSR_UPDATE_INTERVAL`), milliseconds (10 Hz default).
    pub ssr_update_interval_ms: u64,
}

impl Default for ControlParams {
    fn default() -> Self {
        ControlParams {
            controller: ControllerConfig::default(),
            pid_base: Gains::new(25.0, 0.14, 160.0),
            thermal_h: 0.0,
            thermal_t_ambient: 25.0,
            ssr_cycle_time_s: 20.0,
            thermocouple_offset: 0.0,
            median_window: 3,
            status_update_interval_ms: DEFAULT_STATUS_INTERVAL_MS,
            watchdog_timeout_ms: 8_000,
            temp_read_interval_ms: 1_000,
            ssr_update_interval_ms: 100,
        }
    }
}
