//! `config.json` → [`KilnConfig`] — the single source of truth for every tunable
//! the controller reads, ported from the flat `config.py` module the MicroPython
//! build `import`ed at boot. Same knobs, same names: each JSON key is the exact
//! `config.py` identifier (`PID_KP_BASE`, `SSR_CYCLE_TIME`, `WIFI_SSID`, ...), so
//! a `config.py` maps to a `config.json` one-to-one.
//!
//! Loading model (the device has no module system to `import`): `kiln-firmware`
//! reads `config.json` from the littlefs flash at boot and calls
//! [`evaluate_boot`]. A missing, unreadable, unparseable or out-of-range file no
//! longer silently becomes [`KilnConfig::default`]: the kiln still boots (network
//! up, so the config can be fixed) but **firing is locked** until a valid config
//! is saved and the board rebooted — the defaults (thermocouple type K, ...) can
//! be lethally wrong for a given kiln (an S probe read as K shows ~236 °C at a
//! real 1000 °C). Every value is range-checked by [`KilnConfig::validate`], at
//! boot and on `POST /api/config`. [`parse`] is a *partial*
//! override — only the keys present are applied, exactly like editing a sparse
//! `config.py` — and unknown keys are ignored for forward compatibility.
//!
//! Like `profile_json`, this is a hand-rolled, panic-safe, allocation-free reader
//! (no `serde`) so the pure layer stays dependency-free and host-testable.
//! [`KilnConfig::write_json`] re-emits the whole document for `GET /api/config`.

use core::fmt::{self, Write};

use kiln_core::gain_schedule::Gains;
use kiln_core::heat_watch::HeatWatchConfig;
use kiln_core::state::{ControllerConfig, FiringLock, RATE_HISTORY_CAP};
use kiln_core::temp_filter::MAX_MEDIAN_WINDOW;
use kiln_hal::max31856::Averaging;
use kiln_hal::ThermocoupleType;

/// Maximum SSR relays the `SSR_PIN` list accepts — `config.example.py` notes the
/// stagger logic "supports up to 10 SSRs".
pub const MAX_SSR: usize = 10;

/// Capacity for the network string fields (SSID/password/IP literals).
pub const STR_CAP: usize = 64;

/// The GPIOs `kiln-firmware` can drive as SSR outputs. The RP2350 pinmux is
/// fixed at compile time, so the firmware pre-builds exactly these (its
/// `ssr_pool`, same order) and `SSR_PIN` must pick from them.
pub const SSR_PIN_POOL: [u8; MAX_SSR] = [15, 14, 13, 12, 11, 10, 9, 8, 7, 6];

/// Longest watchdog period the RP2350 supports: its 24-bit LOAD counter at
/// 1 µs/tick is 16.78 s, and embassy-rp panics above that (a boot loop).
pub const WATCHDOG_TIMEOUT_MAX_MS: u32 = 16_000;

/// A bounded, owned string — the pure layer's stand-in for `heapless::String`
/// (kept here so `config` adds no dependency, matching `profile_json`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedStr<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> FixedStr<N> {
    /// Copy `s` in, erroring if it does not fit in `N` bytes.
    pub fn from_text(s: &str) -> Result<Self, ConfigError> {
        let b = s.as_bytes();
        if b.len() > N {
            return Err(ConfigError::StringTooLong);
        }
        let mut buf = [0u8; N];
        buf[..b.len()].copy_from_slice(b);
        Ok(Self { buf, len: b.len() })
    }

    /// Borrow as `&str` (always valid UTF-8: only ever filled from `&str`).
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> Default for FixedStr<N> {
    fn default() -> Self {
        Self {
            buf: [0u8; N],
            len: 0,
        }
    }
}

/// Network string alias.
pub type Str = FixedStr<STR_CAP>;

/// One or more SSR GPIO numbers — `SSR_PIN` accepts a bare int (single relay) or
/// a list (staggered multi-relay). Stored fixed-capacity, no allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinList {
    pins: [u8; MAX_SSR],
    len: usize,
}

impl PinList {
    /// A single-relay list (the `SSR_PIN = 15` case).
    pub fn single(pin: u8) -> Self {
        let mut pins = [0u8; MAX_SSR];
        pins[0] = pin;
        Self { pins, len: 1 }
    }

    fn empty() -> Self {
        Self {
            pins: [0u8; MAX_SSR],
            len: 0,
        }
    }

    fn push(&mut self, pin: u8) -> Result<(), ConfigError> {
        if self.len >= MAX_SSR {
            return Err(ConfigError::TooManySsrPins);
        }
        self.pins[self.len] = pin;
        self.len += 1;
        Ok(())
    }

    /// The configured pins, in order.
    pub fn as_slice(&self) -> &[u8] {
        &self.pins[..self.len]
    }
}

/// `TEMP_UNITS` — display units. Carried for fidelity; like the reference, the
/// control loop itself is always Celsius (the global is presentation only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TempUnits {
    #[default]
    Celsius,
    Fahrenheit,
}

/// Why parsing `config.json` failed. A bad file is non-fatal — the caller falls
/// back to [`KilnConfig::default`] — but the typed cause aids `POST /api/config`
/// validation messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// Malformed JSON (the reference's `json.loads` raising).
    Syntax,
    /// A string value exceeded its fixed capacity.
    StringTooLong,
    /// `SSR_PIN` list longer than [`MAX_SSR`].
    TooManySsrPins,
    /// An enumerated value (`THERMOCOUPLE_TYPE`, `TEMP_UNITS`) was unrecognised,
    /// or an integer field got a fractional / negative / oversized number.
    BadValue,
}

/// What is wrong with a configuration value (see [`Invalid`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Problem {
    /// The JSON could not be parsed at (or before) this key.
    Parse(ConfigError),
    /// Outside the inclusive safe range, or not finite.
    Range { min: f64, max: f64 },
    /// Not one of the listed values.
    OneOf(&'static str),
    /// Breaks a rule (possibly relating it to [`Invalid::other`]).
    Rule(&'static str),
}

/// A rejected configuration: the offending key and why. `Display` renders a
/// short, ASCII, quote-free message suitable for an HTTP error body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Invalid {
    /// The offending key (`None` for a document-level syntax error).
    pub key: Option<&'static str>,
    /// For a cross-key rule, the other key involved.
    pub other: Option<&'static str>,
    pub problem: Problem,
}

impl Invalid {
    fn range(key: &'static str, min: f64, max: f64) -> Self {
        Invalid {
            key: Some(key),
            other: None,
            problem: Problem::Range { min, max },
        }
    }
    fn one_of(key: &'static str, list: &'static str) -> Self {
        Invalid {
            key: Some(key),
            other: None,
            problem: Problem::OneOf(list),
        }
    }
    fn rule(key: &'static str, other: Option<&'static str>, rule: &'static str) -> Self {
        Invalid {
            key: Some(key),
            other,
            problem: Problem::Rule(rule),
        }
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(key) = self.key else {
            return f.write_str("malformed JSON");
        };
        match self.problem {
            Problem::Parse(ConfigError::StringTooLong) => write!(f, "{key} is too long"),
            Problem::Parse(ConfigError::TooManySsrPins) => {
                write!(f, "{key} lists more than {MAX_SSR} pins")
            }
            Problem::Parse(_) => write!(f, "{key} has an invalid value or type"),
            Problem::Range { min, max } => write!(f, "{key} must be between {min} and {max}"),
            Problem::OneOf(list) => write!(f, "{key} must be one of {list}"),
            Problem::Rule(rule) => write!(f, "{key} {rule}"),
        }
    }
}

/// Check `v` lies in `[min, max]` (rejects NaN/inf).
fn in_range(key: &'static str, v: f64, min: f64, max: f64) -> Result<(), Invalid> {
    if v >= min && v <= max {
        Ok(())
    } else {
        Err(Invalid::range(key, min, max))
    }
}

/// Dotted-quad IPv4 literal (`a.b.c.d`, each 0-255, no leading `+`/spaces).
fn is_ipv4(s: &str) -> bool {
    let mut parts = 0;
    for part in s.split('.') {
        parts += 1;
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        if part.parse::<u16>().map_or(true, |v| v > 255) {
            return false;
        }
    }
    parts == 4
}

/// Diagnostic log verbosity, mapped to `log::LevelFilter` by `kiln-app::logging`.
/// Kept as a local enum so the pure config layer pulls in no `log` dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    /// Parse the `LOG_LEVEL` string; `None` for an unknown value.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "off" => LogLevel::Off,
            "error" => LogLevel::Error,
            "warn" => LogLevel::Warn,
            "info" => LogLevel::Info,
            "debug" => LogLevel::Debug,
            _ => return None,
        })
    }

    /// The lowercase wire string (for `write_json`).
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Off => "off",
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
        }
    }
}

/// Every tunable the firmware reads, flattened to the `config.py` names. Edit one
/// place, in JSON; [`KilnConfig::default`] is the `config.example.py` baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct KilnConfig {
    // --- Hardware (MAX31856 SPI + SSR pins) ---
    pub max31856_spi_id: u8,
    pub max31856_sck_pin: u8,
    pub max31856_mosi_pin: u8,
    pub max31856_miso_pin: u8,
    pub max31856_cs_pin: u8,
    pub ssr_pin: PinList,

    // --- Network ---
    pub wifi_ssid: Str,
    pub wifi_password: Str,
    pub wifi_static_ip: Option<Str>,
    pub wifi_subnet: Option<Str>,
    pub wifi_gateway: Option<Str>,
    pub wifi_dns: Option<Str>,
    pub web_server_host: Str,
    pub web_server_port: u16,

    // --- Temperature sensor ---
    pub thermocouple_type: ThermocoupleType,
    pub temp_units: TempUnits,
    pub thermocouple_offset: f64,
    pub mains_frequency: u16,
    pub thermocouple_averaging: u8,
    pub temp_median_window: usize,

    // --- Control-loop timing (seconds; watchdog in ms) ---
    pub temp_read_interval: f64,
    pub pid_update_interval: f64,
    pub status_update_interval: f64,
    pub ssr_update_interval: f64,

    // --- PID + gain scheduling ---
    pub pid_kp_base: f64,
    pub pid_ki_base: f64,
    pub pid_kd_base: f64,
    pub thermal_h: f64,
    pub thermal_t_ambient: f64,

    // --- SSR power control ---
    pub ssr_cycle_time: f64,
    pub ssr_stagger_delay: f64,

    // --- Safety + rate/stall ---
    pub max_temp: f64,
    pub stall_check_interval: f64,
    pub stall_consecutive_fails: u32,
    pub stall_min_step_time: f64,
    pub stall_rate_ratio: f64,
    pub stall_arrival_band: f64,
    pub rate_measurement_window: f64,
    pub rate_recording_interval: f64,

    // --- Heating-response watchdog (Rust-only; see kiln_core::heat_watch) ---
    pub heat_watch_window: f64,
    pub heat_watch_min_rise: f64,

    // --- Logging + recovery + watchdog ---
    pub logging_interval: u32,
    pub max_recovery_temp_delta: f64,
    pub enable_watchdog: bool,
    pub watchdog_timeout: u32,
    pub log_level: LogLevel,
    pub log_to_flash: bool,

    // --- LCD (disabled unless any `LCD_I2C_*` key is present) ---
    pub lcd_enabled: bool,
    pub lcd_i2c_id: u8,
    pub lcd_i2c_scl: u8,
    pub lcd_i2c_sda: u8,
    pub lcd_i2c_freq: u32,
    pub lcd_i2c_addr: u8,
}

impl Default for KilnConfig {
    fn default() -> Self {
        Self {
            max31856_spi_id: 0,
            max31856_sck_pin: 18,
            max31856_mosi_pin: 19,
            max31856_miso_pin: 16,
            max31856_cs_pin: 28,
            ssr_pin: PinList::single(15),

            wifi_ssid: Str::default(),
            wifi_password: Str::default(),
            wifi_static_ip: None,
            wifi_subnet: None,
            wifi_gateway: None,
            wifi_dns: None,
            web_server_host: Str::from_text("0.0.0.0").unwrap(),
            web_server_port: 80,

            thermocouple_type: ThermocoupleType::K,
            temp_units: TempUnits::Celsius,
            thermocouple_offset: 0.0,
            mains_frequency: 60,
            thermocouple_averaging: 8,
            temp_median_window: 3,

            temp_read_interval: 1.0,
            pid_update_interval: 1.0,
            status_update_interval: 5.0,
            ssr_update_interval: 0.1,

            pid_kp_base: 25.0,
            pid_ki_base: 0.14,
            pid_kd_base: 160.0,
            thermal_h: 0.0,
            thermal_t_ambient: 25.0,

            ssr_cycle_time: 20.0,
            ssr_stagger_delay: 0.01,

            max_temp: 1300.0,
            stall_check_interval: 60.0,
            stall_consecutive_fails: 3,
            stall_min_step_time: 600.0,
            stall_rate_ratio: 0.8,
            stall_arrival_band: 10.0,
            rate_measurement_window: 600.0,
            rate_recording_interval: 10.0,

            heat_watch_window: 900.0,
            heat_watch_min_rise: 5.0,

            logging_interval: 30,
            max_recovery_temp_delta: 30.0,
            // Default ON: the watchdog is the only backstop that resets the chip
            // if Core 1 hangs/panics mid-firing (the control loop feeds it every
            // ~temp_read_interval = 1 s, well inside the 8 s timeout). The
            // reference shipped it off; we default it on for unattended firing.
            enable_watchdog: true,
            watchdog_timeout: 8000,
            // Default INFO: keeps the boot/init trail and run events without the
            // per-tick DEBUG chatter that churns the diag flash. Raise to "debug"
            // via LOG_LEVEL when diagnosing a specific issue.
            log_level: LogLevel::Info,
            log_to_flash: true,

            lcd_enabled: false,
            lcd_i2c_id: 0,
            lcd_i2c_scl: 21,
            lcd_i2c_sda: 20,
            lcd_i2c_freq: 100_000,
            lcd_i2c_addr: 0x27,
        }
    }
}

impl KilnConfig {
    /// The WiFi placeholder that `config.example.json` ships (asserted verbatim by
    /// `config_example_json_is_valid`).
    const WIFI_SSID_PLACEHOLDER: &'static str = "your_wifi_ssid";

    /// True when WiFi station credentials are usable: a non-empty SSID that is not
    /// the shipped placeholder. Drives the firmware boot choice between STA and the
    /// provisioning SoftAP (`core0_main`).
    pub fn wifi_is_configured(&self) -> bool {
        let ssid = self.wifi_ssid.as_str();
        !ssid.is_empty() && ssid != Self::WIFI_SSID_PLACEHOLDER
    }

    /// The safety/rate/stall subset, as the `kiln-core` controller wants it.
    pub fn controller_config(&self) -> ControllerConfig {
        ControllerConfig {
            max_temp: self.max_temp as f32,
            rate_measurement_window: self.rate_measurement_window as f32,
            rate_recording_interval: self.rate_recording_interval as f32,
            stall_check_interval: self.stall_check_interval as f32,
            stall_consecutive_fails: self.stall_consecutive_fails,
            stall_min_step_time: self.stall_min_step_time as f32,
            stall_rate_ratio: self.stall_rate_ratio as f32,
            stall_arrival_band: self.stall_arrival_band as f32,
        }
    }

    /// The heating-response watchdog thresholds (`HEAT_WATCH_*`; the arming band
    /// and power threshold stay at the `kiln-core` defaults).
    pub fn heat_watch_config(&self) -> HeatWatchConfig {
        HeatWatchConfig {
            window_s: self.heat_watch_window as f32,
            min_rise: self.heat_watch_min_rise as f32,
            ..HeatWatchConfig::default()
        }
    }

    /// Range-check every value the firmware acts on. The first problem is
    /// returned. Run at boot (a failure locks firing, see [`evaluate_boot`]) and
    /// on every `POST /api/config` (a failure rejects the save).
    ///
    /// The limits are safety and hardware limits, not taste: e.g.
    /// `WATCHDOG_TIMEOUT` above 16 s panics the RP2350 driver, a
    /// `TEMP_READ_INTERVAL` near the watchdog period resets the chip mid-firing,
    /// voltage-mode `THERMOCOUPLE_TYPE`s (G8/G32) do not produce temperatures,
    /// `THERMAL_H` above 0.01 multiplies every PID gain by >14 at 1300 °C.
    pub fn validate(&self) -> Result<(), Invalid> {
        // --- Hardware ---
        // The MAX31856 bus is hardwired in kiln-firmware (SPI0, SCK 18, MOSI 19,
        // MISO 16, CS 28): any other value would be saved and silently ignored.
        for (key, value, fixed) in [
            ("MAX31856_SPI_ID", self.max31856_spi_id, (0, "must be 0 (fixed by the firmware wiring)")),
            ("MAX31856_SCK_PIN", self.max31856_sck_pin, (18, "must be 18 (fixed by the firmware wiring)")),
            ("MAX31856_MOSI_PIN", self.max31856_mosi_pin, (19, "must be 19 (fixed by the firmware wiring)")),
            ("MAX31856_MISO_PIN", self.max31856_miso_pin, (16, "must be 16 (fixed by the firmware wiring)")),
            ("MAX31856_CS_PIN", self.max31856_cs_pin, (28, "must be 28 (fixed by the firmware wiring)")),
        ] {
            if value != fixed.0 {
                return Err(Invalid::rule(key, None, fixed.1));
            }
        }
        let pins = self.ssr_pin.as_slice();
        if pins.is_empty() {
            return Err(Invalid::rule("SSR_PIN", None, "must list at least one pin"));
        }
        for (i, pin) in pins.iter().enumerate() {
            if !SSR_PIN_POOL.contains(pin) {
                return Err(Invalid::one_of("SSR_PIN", "GPIO 6 to 15"));
            }
            if pins[..i].contains(pin) {
                return Err(Invalid::rule("SSR_PIN", None, "must not repeat a pin"));
            }
        }

        // --- Network ---
        if self.wifi_ssid.as_str().len() > 32 {
            return Err(Invalid::rule("WIFI_SSID", None, "must be at most 32 bytes"));
        }
        let pw = self.wifi_password.as_str();
        let printable = |b: u8| (0x20..=0x7e).contains(&b);
        let pw_ok = pw.is_empty()
            || ((8..=63).contains(&pw.len()) && pw.bytes().all(printable))
            || (pw.len() == 64 && pw.bytes().all(|b| b.is_ascii_hexdigit()));
        if !pw_ok {
            return Err(Invalid::rule(
                "WIFI_PASSWORD",
                None,
                "must be empty or 8-63 printable ASCII chars",
            ));
        }
        for (key, ip) in [
            ("WIFI_STATIC_IP", &self.wifi_static_ip),
            ("WIFI_SUBNET", &self.wifi_subnet),
            ("WIFI_GATEWAY", &self.wifi_gateway),
            ("WIFI_DNS", &self.wifi_dns),
        ] {
            if let Some(s) = ip {
                if !s.as_str().is_empty() && !is_ipv4(s.as_str()) {
                    return Err(Invalid::rule(key, None, "must be an IPv4 address or null"));
                }
            }
        }
        in_range("WEB_SERVER_PORT", self.web_server_port as f64, 1.0, 65535.0)?;

        // --- Temperature sensor ---
        if matches!(
            self.thermocouple_type,
            ThermocoupleType::G8 | ThermocoupleType::G32
        ) {
            return Err(Invalid::one_of(
                "THERMOCOUPLE_TYPE",
                "B, E, J, K, N, R, S or T",
            ));
        }
        in_range("THERMOCOUPLE_OFFSET", self.thermocouple_offset, -50.0, 50.0)?;
        if !matches!(self.mains_frequency, 50 | 60) {
            return Err(Invalid::one_of("MAINS_FREQUENCY", "50 or 60"));
        }
        if Averaging::from_samples(self.thermocouple_averaging).is_none() {
            return Err(Invalid::one_of("THERMOCOUPLE_AVERAGING", "1, 2, 4, 8 or 16"));
        }
        in_range(
            "TEMP_MEDIAN_WINDOW",
            self.temp_median_window as f64,
            1.0,
            MAX_MEDIAN_WINDOW as f64,
        )?;

        // --- Control-loop timing ---
        in_range("TEMP_READ_INTERVAL", self.temp_read_interval, 0.1, 10.0)?;
        in_range("PID_UPDATE_INTERVAL", self.pid_update_interval, 0.1, 10.0)?;
        in_range("STATUS_UPDATE_INTERVAL", self.status_update_interval, 1.0, 60.0)?;
        in_range("SSR_UPDATE_INTERVAL", self.ssr_update_interval, 0.05, 5.0)?;
        if self.ssr_update_interval > self.temp_read_interval {
            return Err(Invalid::rule(
                "SSR_UPDATE_INTERVAL",
                Some("TEMP_READ_INTERVAL"),
                "must not exceed TEMP_READ_INTERVAL",
            ));
        }

        // --- PID + gain scheduling ---
        in_range("PID_KP_BASE", self.pid_kp_base, 0.0, 10_000.0)?;
        in_range("PID_KI_BASE", self.pid_ki_base, 0.0, 1_000.0)?;
        in_range("PID_KD_BASE", self.pid_kd_base, 0.0, 100_000.0)?;
        // h <= 0.01 and T_amb in [-20, 40] keep the gain scale 1 + h(T - T_amb)
        // positive over the whole accepted reading range [-50, 1500] °C.
        in_range("THERMAL_H", self.thermal_h, 0.0, 0.01)?;
        in_range("THERMAL_T_AMBIENT", self.thermal_t_ambient, -20.0, 40.0)?;

        // --- SSR ---
        in_range("SSR_CYCLE_TIME", self.ssr_cycle_time, 1.0, 120.0)?;
        in_range("SSR_STAGGER_DELAY", self.ssr_stagger_delay, 0.0, 5.0)?;

        // --- Safety + rate/stall ---
        in_range("MAX_TEMP", self.max_temp, 100.0, 1400.0)?;
        in_range("STALL_CHECK_INTERVAL", self.stall_check_interval, 10.0, 3600.0)?;
        in_range(
            "STALL_CONSECUTIVE_FAILS",
            self.stall_consecutive_fails as f64,
            1.0,
            20.0,
        )?;
        in_range("STALL_MIN_STEP_TIME", self.stall_min_step_time, 0.0, 7200.0)?;
        in_range("STALL_RATE_RATIO", self.stall_rate_ratio, 0.0, 1.0)?;
        in_range("STALL_ARRIVAL_BAND", self.stall_arrival_band, 0.0, 50.0)?;
        in_range(
            "RATE_MEASUREMENT_WINDOW",
            self.rate_measurement_window,
            60.0,
            7200.0,
        )?;
        in_range("RATE_RECORDING_INTERVAL", self.rate_recording_interval, 1.0, 600.0)?;
        if self.rate_recording_interval >= self.rate_measurement_window {
            return Err(Invalid::rule(
                "RATE_RECORDING_INTERVAL",
                Some("RATE_MEASUREMENT_WINDOW"),
                "must be less than RATE_MEASUREMENT_WINDOW",
            ));
        }
        // The rate history keeps RATE_HISTORY_CAP samples (60, as in the
        // reference): a longer window would silently measure a shorter span.
        if self.rate_measurement_window
            > self.rate_recording_interval * RATE_HISTORY_CAP as f64
        {
            return Err(Invalid::rule(
                "RATE_MEASUREMENT_WINDOW",
                Some("RATE_RECORDING_INTERVAL"),
                "must be at most 60 x RATE_RECORDING_INTERVAL",
            ));
        }
        in_range("HEAT_WATCH_WINDOW", self.heat_watch_window, 300.0, 3600.0)?;
        in_range("HEAT_WATCH_MIN_RISE", self.heat_watch_min_rise, 1.0, 50.0)?;

        // --- Logging + recovery + watchdog ---
        in_range("LOGGING_INTERVAL", self.logging_interval as f64, 1.0, 600.0)?;
        in_range(
            "MAX_RECOVERY_TEMP_DELTA",
            self.max_recovery_temp_delta,
            0.0,
            200.0,
        )?;
        in_range(
            "WATCHDOG_TIMEOUT",
            self.watchdog_timeout as f64,
            1000.0,
            WATCHDOG_TIMEOUT_MAX_MS as f64,
        )?;
        // The loop feeds the watchdog once per TEMP_READ_INTERVAL tick: keep a 2x
        // margin so a slow tick never resets the chip mid-firing.
        if self.enable_watchdog
            && self.temp_read_interval * 1000.0 * 2.0 > self.watchdog_timeout as f64
        {
            return Err(Invalid::rule(
                "TEMP_READ_INTERVAL",
                Some("WATCHDOG_TIMEOUT"),
                "must be at most half of WATCHDOG_TIMEOUT",
            ));
        }

        // --- LCD ---
        if self.lcd_enabled {
            // Also hardwired (I2C0, SDA 20, SCL 21); checked only when used.
            for (key, value, fixed) in [
                ("LCD_I2C_ID", self.lcd_i2c_id, (0, "must be 0 (fixed by the firmware wiring)")),
                ("LCD_I2C_SCL", self.lcd_i2c_scl, (21, "must be 21 (fixed by the firmware wiring)")),
                ("LCD_I2C_SDA", self.lcd_i2c_sda, (20, "must be 20 (fixed by the firmware wiring)")),
            ] {
                if value != fixed.0 {
                    return Err(Invalid::rule(key, None, fixed.1));
                }
            }
            in_range("LCD_I2C_FREQ", self.lcd_i2c_freq as f64, 10_000.0, 1_000_000.0)?;
            in_range("LCD_I2C_ADDR", self.lcd_i2c_addr as f64, 0.0, 127.0)?;
        }
        Ok(())
    }

    /// Base PID gains as the gain scheduler's triple.
    pub fn pid_base(&self) -> Gains {
        Gains::new(
            self.pid_kp_base as f32,
            self.pid_ki_base as f32,
            self.pid_kd_base as f32,
        )
    }

    /// Outer control-tick period in milliseconds.
    pub fn temp_read_interval_ms(&self) -> u64 {
        (self.temp_read_interval * 1000.0) as u64
    }

    /// Status publish cadence in milliseconds.
    pub fn status_update_interval_ms(&self) -> u64 {
        (self.status_update_interval * 1000.0) as u64
    }

    /// SSR sub-tick period in milliseconds.
    pub fn ssr_update_interval_ms(&self) -> u64 {
        (self.ssr_update_interval * 1000.0) as u64
    }

    /// SSR stagger delay in milliseconds (multi-relay inrush spacing).
    pub fn ssr_stagger_delay_ms(&self) -> u64 {
        (self.ssr_stagger_delay * 1000.0) as u64
    }

    /// Serialize the full config as JSON (the `GET /api/config` body).
    pub fn write_json<W: Write>(&self, w: &mut W) -> fmt::Result {
        w.write_char('{')?;
        let mut j = Fields { w, first: true };

        j.int("MAX31856_SPI_ID", self.max31856_spi_id as u64)?;
        j.int("MAX31856_SCK_PIN", self.max31856_sck_pin as u64)?;
        j.int("MAX31856_MOSI_PIN", self.max31856_mosi_pin as u64)?;
        j.int("MAX31856_MISO_PIN", self.max31856_miso_pin as u64)?;
        j.int("MAX31856_CS_PIN", self.max31856_cs_pin as u64)?;
        j.pins("SSR_PIN", &self.ssr_pin)?;

        j.string("WIFI_SSID", self.wifi_ssid.as_str())?;
        j.string("WIFI_PASSWORD", self.wifi_password.as_str())?;
        j.opt("WIFI_STATIC_IP", &self.wifi_static_ip)?;
        j.opt("WIFI_SUBNET", &self.wifi_subnet)?;
        j.opt("WIFI_GATEWAY", &self.wifi_gateway)?;
        j.opt("WIFI_DNS", &self.wifi_dns)?;
        j.string("WEB_SERVER_HOST", self.web_server_host.as_str())?;
        j.int("WEB_SERVER_PORT", self.web_server_port as u64)?;

        j.raw("THERMOCOUPLE_TYPE", tc_str(self.thermocouple_type))?;
        j.raw("TEMP_UNITS", units_str(self.temp_units))?;
        j.float("THERMOCOUPLE_OFFSET", self.thermocouple_offset)?;
        j.int("MAINS_FREQUENCY", self.mains_frequency as u64)?;
        j.int("THERMOCOUPLE_AVERAGING", self.thermocouple_averaging as u64)?;
        j.int("TEMP_MEDIAN_WINDOW", self.temp_median_window as u64)?;

        j.float("TEMP_READ_INTERVAL", self.temp_read_interval)?;
        j.float("PID_UPDATE_INTERVAL", self.pid_update_interval)?;
        j.float("STATUS_UPDATE_INTERVAL", self.status_update_interval)?;
        j.float("SSR_UPDATE_INTERVAL", self.ssr_update_interval)?;

        j.float("PID_KP_BASE", self.pid_kp_base)?;
        j.float("PID_KI_BASE", self.pid_ki_base)?;
        j.float("PID_KD_BASE", self.pid_kd_base)?;
        j.float("THERMAL_H", self.thermal_h)?;
        j.float("THERMAL_T_AMBIENT", self.thermal_t_ambient)?;

        j.float("SSR_CYCLE_TIME", self.ssr_cycle_time)?;
        j.float("SSR_STAGGER_DELAY", self.ssr_stagger_delay)?;

        j.float("MAX_TEMP", self.max_temp)?;
        j.float("STALL_CHECK_INTERVAL", self.stall_check_interval)?;
        j.int(
            "STALL_CONSECUTIVE_FAILS",
            self.stall_consecutive_fails as u64,
        )?;
        j.float("STALL_MIN_STEP_TIME", self.stall_min_step_time)?;
        j.float("STALL_RATE_RATIO", self.stall_rate_ratio)?;
        j.float("STALL_ARRIVAL_BAND", self.stall_arrival_band)?;
        j.float("RATE_MEASUREMENT_WINDOW", self.rate_measurement_window)?;
        j.float("RATE_RECORDING_INTERVAL", self.rate_recording_interval)?;
        j.float("HEAT_WATCH_WINDOW", self.heat_watch_window)?;
        j.float("HEAT_WATCH_MIN_RISE", self.heat_watch_min_rise)?;

        j.int("LOGGING_INTERVAL", self.logging_interval as u64)?;
        j.float("MAX_RECOVERY_TEMP_DELTA", self.max_recovery_temp_delta)?;
        j.boolean("ENABLE_WATCHDOG", self.enable_watchdog)?;
        j.int("WATCHDOG_TIMEOUT", self.watchdog_timeout as u64)?;
        j.string("LOG_LEVEL", self.log_level.as_str())?;
        j.boolean("LOG_TO_FLASH", self.log_to_flash)?;

        if self.lcd_enabled {
            j.int("LCD_I2C_ID", self.lcd_i2c_id as u64)?;
            j.int("LCD_I2C_SCL", self.lcd_i2c_scl as u64)?;
            j.int("LCD_I2C_SDA", self.lcd_i2c_sda as u64)?;
            j.int("LCD_I2C_FREQ", self.lcd_i2c_freq as u64)?;
            j.int("LCD_I2C_ADDR", self.lcd_i2c_addr as u64)?;
        }

        j.w.write_char('}')
    }
}

/// Every key [`parse`] recognises, as `'static` names for error messages (and
/// to keep [`KilnConfig::write_json`] in sync, see the tests).
const KEYS: &[&str] = &[
    "MAX31856_SPI_ID",
    "MAX31856_SCK_PIN",
    "MAX31856_MOSI_PIN",
    "MAX31856_MISO_PIN",
    "MAX31856_CS_PIN",
    "SSR_PIN",
    "WIFI_SSID",
    "WIFI_PASSWORD",
    "WIFI_STATIC_IP",
    "WIFI_SUBNET",
    "WIFI_GATEWAY",
    "WIFI_DNS",
    "WEB_SERVER_HOST",
    "WEB_SERVER_PORT",
    "THERMOCOUPLE_TYPE",
    "TEMP_UNITS",
    "THERMOCOUPLE_OFFSET",
    "MAINS_FREQUENCY",
    "THERMOCOUPLE_AVERAGING",
    "TEMP_MEDIAN_WINDOW",
    "TEMP_READ_INTERVAL",
    "PID_UPDATE_INTERVAL",
    "STATUS_UPDATE_INTERVAL",
    "SSR_UPDATE_INTERVAL",
    "PID_KP_BASE",
    "PID_KI_BASE",
    "PID_KD_BASE",
    "THERMAL_H",
    "THERMAL_T_AMBIENT",
    "SSR_CYCLE_TIME",
    "SSR_STAGGER_DELAY",
    "MAX_TEMP",
    "STALL_CHECK_INTERVAL",
    "STALL_CONSECUTIVE_FAILS",
    "STALL_MIN_STEP_TIME",
    "STALL_RATE_RATIO",
    "STALL_ARRIVAL_BAND",
    "RATE_MEASUREMENT_WINDOW",
    "RATE_RECORDING_INTERVAL",
    "HEAT_WATCH_WINDOW",
    "HEAT_WATCH_MIN_RISE",
    "LOGGING_INTERVAL",
    "MAX_RECOVERY_TEMP_DELTA",
    "ENABLE_WATCHDOG",
    "WATCHDOG_TIMEOUT",
    "LOG_LEVEL",
    "LOG_TO_FLASH",
    "LCD_I2C_ID",
    "LCD_I2C_SCL",
    "LCD_I2C_SDA",
    "LCD_I2C_FREQ",
    "LCD_I2C_ADDR",
];

fn static_key(key: &str) -> Option<&'static str> {
    KEYS.iter().copied().find(|k| *k == key)
}

/// Parse a `config.json` document, applying each present key over the defaults
/// (the boot / whole-file case). Syntax/type check only: call
/// [`KilnConfig::validate`] (or use [`evaluate_boot`] / [`apply_patch`]) before
/// acting on the values.
pub fn parse(json: &str) -> Result<KilnConfig, ConfigError> {
    parse_over(KilnConfig::default(), json)
}

/// Apply the present keys of `json` over `base`, leaving absent keys untouched —
/// the `PATCH`-style merge `POST /api/config` uses so a sparse body edits only
/// the named knobs of the running config rather than resetting the rest.
pub fn parse_over(base: KilnConfig, json: &str) -> Result<KilnConfig, ConfigError> {
    parse_over_skipping(base, json, &[]).map_err(|inv| match inv.problem {
        Problem::Parse(e) => e,
        _ => ConfigError::Syntax,
    })
}

/// [`parse_over`] that reports the offending key, ignoring every key in `skip`
/// (used to drop invalid keys when sanitising a boot config).
fn parse_over_skipping(base: KilnConfig, json: &str, skip: &[&str]) -> Result<KilnConfig, Invalid> {
    let doc = |e: ConfigError| Invalid {
        key: None,
        other: None,
        problem: Problem::Parse(e),
    };
    let mut cfg = base;
    let mut r = Reader::new(json);
    r.skip_ws();
    r.expect(b'{').map_err(doc)?;
    r.skip_ws();
    if r.try_consume(b'}') {
        return Ok(cfg);
    }
    loop {
        r.skip_ws();
        let key = r.parse_string().map_err(doc)?;
        // Keys are matched undecoded: an escaped spelling of a known key (e.g.
        // `THERMOCOUPLE_\u0054YPE`) would silently be ignored and its default
        // kept. No real key needs an escape, so refuse them outright.
        if key.contains('\\') {
            return Err(doc(ConfigError::Syntax));
        }
        r.skip_ws();
        r.expect(b':').map_err(doc)?;
        r.skip_ws();
        let applied = if skip.contains(&key) {
            r.skip_value(0)
        } else {
            apply_key(&mut cfg, key, &mut r)
        };
        applied.map_err(|e| Invalid {
            key: static_key(key),
            other: None,
            problem: Problem::Parse(e),
        })?;
        r.skip_ws();
        if r.try_consume(b',') {
            continue;
        }
        r.expect(b'}').map_err(doc)?;
        break;
    }
    r.skip_ws();
    if r.peek().is_some() {
        return Err(doc(ConfigError::Syntax)); // trailing garbage after the object
    }
    Ok(cfg)
}

/// `POST /api/config`: merge the sparse `body` over `base` and validate the
/// result. `Err` carries the message for the 400 response.
pub fn apply_patch(base: KilnConfig, body: &str) -> Result<KilnConfig, Invalid> {
    let merged = parse_over_skipping(base, body, &[])?;
    merged.validate()?;
    Ok(merged)
}

/// Keys a save must state explicitly while firing is LOCKED (see
/// [`apply_patch_confirming`]): the ones whose built-in default can be lethally
/// wrong for the kiln actually wired up (a type-S probe read as type K, a relay
/// on another pin, the mains notch, the over-temperature limit).
pub const LOCK_CONFIRM_KEYS: [&str; 4] =
    ["THERMOCOUPLE_TYPE", "SSR_PIN", "MAINS_FREQUENCY", "MAX_TEMP"];

/// `POST /api/config` while firing is locked: [`apply_patch`], but the body must
/// itself name every [`LOCK_CONFIRM_KEYS`] key. While locked the merge base is
/// the built-in defaults (no/unreadable file) or the file minus its rejected
/// keys, so a sparse save (say, only the WiFi SSID) would otherwise write those
/// defaults to flash and unlock firing on them at the next boot.
pub fn apply_patch_confirming(base: KilnConfig, body: &str) -> Result<KilnConfig, Invalid> {
    for key in LOCK_CONFIRM_KEYS {
        if !has_top_level_key(body, key)? {
            return Err(Invalid::rule(
                key,
                None,
                "must be set explicitly while firing is locked",
            ));
        }
    }
    apply_patch(base, body)
}

/// Does the JSON object `json` have the top-level key `wanted`?
fn has_top_level_key(json: &str, wanted: &str) -> Result<bool, Invalid> {
    let doc = |e: ConfigError| Invalid {
        key: None,
        other: None,
        problem: Problem::Parse(e),
    };
    let mut r = Reader::new(json);
    r.skip_ws();
    r.expect(b'{').map_err(doc)?;
    r.skip_ws();
    if r.try_consume(b'}') {
        return Ok(false);
    }
    let mut found = false;
    loop {
        r.skip_ws();
        let key = r.parse_string().map_err(doc)?;
        found |= key == wanted;
        r.skip_ws();
        r.expect(b':').map_err(doc)?;
        r.skip_value(0).map_err(doc)?;
        r.skip_ws();
        if r.try_consume(b',') {
            continue;
        }
        r.expect(b'}').map_err(doc)?;
        return Ok(found);
    }
}

/// How `config.json` came off flash at boot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConfigSource<'a> {
    /// No file (fresh board, or littlefs was reformatted after a mount error).
    Missing,
    /// The file exists but could not be read, or is not UTF-8.
    Unreadable,
    /// The file's text.
    Text(&'a str),
}

/// The boot decision: the config to run with, and whether firing is locked.
#[derive(Debug, Clone, PartialEq)]
pub struct BootConfig {
    /// The file when it is valid. Otherwise the file with every offending key
    /// reset to its default (so WiFi still joins and Core 1 runs on safe timing),
    /// or the plain defaults when the file is missing or unparseable.
    pub config: KilnConfig,
    /// `Some` = heating must stay locked until a valid config is saved.
    pub lock: Option<FiringLock>,
    /// The first problem found, for the boot log.
    pub issue: Option<Invalid>,
}

/// Bound on the keys dropped while sanitising an invalid boot config.
const MAX_DROPPED_KEYS: usize = 16;

/// Decide the boot configuration (pure; `kiln-firmware`'s `load_config` only
/// does the flash read). Policy, chosen for safety: any doubt about the file
/// locks firing — a missing file too, because after a silent littlefs reformat
/// the built-in defaults (thermocouple K, default PID) would run a kiln that is
/// wired differently.
pub fn evaluate_boot(src: ConfigSource<'_>) -> BootConfig {
    let locked = |lock, issue| BootConfig {
        config: KilnConfig::default(),
        lock: Some(lock),
        issue,
    };
    let text = match src {
        ConfigSource::Missing => return locked(FiringLock::ConfigMissing, None),
        ConfigSource::Unreadable => return locked(FiringLock::ConfigUnreadable, None),
        ConfigSource::Text(t) => t,
    };
    // Parse, validate, and on any per-key problem (a wrong type as much as an
    // out-of-range value) drop that key, plus the other key of a cross-key
    // rule, and retry, so the remaining keys (WiFi above all) still apply.
    let mut dropped: [&str; MAX_DROPPED_KEYS] = [""; MAX_DROPPED_KEYS];
    let mut n = 0;
    let mut first: Option<Invalid> = None;
    loop {
        let issue = match parse_over_skipping(KilnConfig::default(), text, &dropped[..n]) {
            Ok(cfg) => match cfg.validate() {
                Ok(()) => {
                    return BootConfig {
                        config: cfg,
                        lock: first.map(|_| FiringLock::ConfigInvalid),
                        issue: first,
                    }
                }
                Err(issue) => issue,
            },
            // Document-level syntax error: nothing can be salvaged.
            Err(issue) if issue.key.is_none() => {
                return locked(FiringLock::ConfigUnreadable, Some(first.unwrap_or(issue)));
            }
            Err(issue) => issue,
        };
        let first_issue = *first.get_or_insert(issue);
        let before = n;
        for key in [issue.key, issue.other].into_iter().flatten() {
            if n < MAX_DROPPED_KEYS && !dropped[..n].contains(&key) {
                dropped[n] = key;
                n += 1;
            }
        }
        if n == before {
            // No progress possible: fall back to the defaults entirely.
            return locked(FiringLock::ConfigInvalid, Some(first_issue));
        }
    }
}

fn apply_key(cfg: &mut KilnConfig, key: &str, r: &mut Reader) -> Result<(), ConfigError> {
    match key {
        "MAX31856_SPI_ID" => cfg.max31856_spi_id = r.parse_u8()?,
        "MAX31856_SCK_PIN" => cfg.max31856_sck_pin = r.parse_u8()?,
        "MAX31856_MOSI_PIN" => cfg.max31856_mosi_pin = r.parse_u8()?,
        "MAX31856_MISO_PIN" => cfg.max31856_miso_pin = r.parse_u8()?,
        "MAX31856_CS_PIN" => cfg.max31856_cs_pin = r.parse_u8()?,
        // SSR_PIN is an int (single relay) or an array (staggered multi-relay).
        "SSR_PIN" => {
            r.skip_ws();
            cfg.ssr_pin = if r.peek() == Some(b'[') {
                r.parse_pin_array()?
            } else {
                PinList::single(r.parse_u8()?)
            };
        }

        "WIFI_SSID" => cfg.wifi_ssid = r.parse_text()?,
        "WIFI_PASSWORD" => cfg.wifi_password = r.parse_text()?,
        "WIFI_STATIC_IP" => cfg.wifi_static_ip = r.parse_opt_string()?,
        "WIFI_SUBNET" => cfg.wifi_subnet = r.parse_opt_string()?,
        "WIFI_GATEWAY" => cfg.wifi_gateway = r.parse_opt_string()?,
        "WIFI_DNS" => cfg.wifi_dns = r.parse_opt_string()?,
        "WEB_SERVER_HOST" => cfg.web_server_host = r.parse_text()?,
        "WEB_SERVER_PORT" => cfg.web_server_port = r.parse_uint(u16::MAX as u64)? as u16,

        "THERMOCOUPLE_TYPE" => cfg.thermocouple_type = parse_tc(r.parse_string()?)?,
        "TEMP_UNITS" => cfg.temp_units = parse_units(r.parse_string()?)?,
        "THERMOCOUPLE_OFFSET" => cfg.thermocouple_offset = r.parse_number()?,
        "MAINS_FREQUENCY" => cfg.mains_frequency = r.parse_uint(u16::MAX as u64)? as u16,
        "THERMOCOUPLE_AVERAGING" => cfg.thermocouple_averaging = r.parse_u8()?,
        "TEMP_MEDIAN_WINDOW" => cfg.temp_median_window = r.parse_uint(u16::MAX as u64)? as usize,

        "TEMP_READ_INTERVAL" => cfg.temp_read_interval = r.parse_number()?,
        "PID_UPDATE_INTERVAL" => cfg.pid_update_interval = r.parse_number()?,
        "STATUS_UPDATE_INTERVAL" => cfg.status_update_interval = r.parse_number()?,
        "SSR_UPDATE_INTERVAL" => cfg.ssr_update_interval = r.parse_number()?,

        "PID_KP_BASE" => cfg.pid_kp_base = r.parse_number()?,
        "PID_KI_BASE" => cfg.pid_ki_base = r.parse_number()?,
        "PID_KD_BASE" => cfg.pid_kd_base = r.parse_number()?,
        "THERMAL_H" => cfg.thermal_h = r.parse_number()?,
        "THERMAL_T_AMBIENT" => cfg.thermal_t_ambient = r.parse_number()?,

        "SSR_CYCLE_TIME" => cfg.ssr_cycle_time = r.parse_number()?,
        "SSR_STAGGER_DELAY" => cfg.ssr_stagger_delay = r.parse_number()?,

        "MAX_TEMP" => cfg.max_temp = r.parse_number()?,
        "STALL_CHECK_INTERVAL" => cfg.stall_check_interval = r.parse_number()?,
        "STALL_CONSECUTIVE_FAILS" => {
            cfg.stall_consecutive_fails = r.parse_uint(u32::MAX as u64)? as u32
        }
        "STALL_MIN_STEP_TIME" => cfg.stall_min_step_time = r.parse_number()?,
        "STALL_RATE_RATIO" => cfg.stall_rate_ratio = r.parse_number()?,
        "STALL_ARRIVAL_BAND" => cfg.stall_arrival_band = r.parse_number()?,
        "RATE_MEASUREMENT_WINDOW" => cfg.rate_measurement_window = r.parse_number()?,
        "RATE_RECORDING_INTERVAL" => cfg.rate_recording_interval = r.parse_number()?,
        "HEAT_WATCH_WINDOW" => cfg.heat_watch_window = r.parse_number()?,
        "HEAT_WATCH_MIN_RISE" => cfg.heat_watch_min_rise = r.parse_number()?,

        "LOGGING_INTERVAL" => cfg.logging_interval = r.parse_uint(u32::MAX as u64)? as u32,
        "MAX_RECOVERY_TEMP_DELTA" => cfg.max_recovery_temp_delta = r.parse_number()?,
        "ENABLE_WATCHDOG" => cfg.enable_watchdog = r.parse_bool()?,
        "WATCHDOG_TIMEOUT" => cfg.watchdog_timeout = r.parse_uint(u32::MAX as u64)? as u32,
        "LOG_LEVEL" => {
            cfg.log_level = LogLevel::parse(r.parse_string()?).ok_or(ConfigError::BadValue)?
        }
        "LOG_TO_FLASH" => cfg.log_to_flash = r.parse_bool()?,

        "LCD_I2C_ID" => {
            cfg.lcd_enabled = true;
            cfg.lcd_i2c_id = r.parse_u8()?;
        }
        "LCD_I2C_SCL" => {
            cfg.lcd_enabled = true;
            cfg.lcd_i2c_scl = r.parse_u8()?;
        }
        "LCD_I2C_SDA" => {
            cfg.lcd_enabled = true;
            cfg.lcd_i2c_sda = r.parse_u8()?;
        }
        "LCD_I2C_FREQ" => {
            cfg.lcd_enabled = true;
            cfg.lcd_i2c_freq = r.parse_uint(u32::MAX as u64)? as u32;
        }
        "LCD_I2C_ADDR" => {
            cfg.lcd_enabled = true;
            cfg.lcd_i2c_addr = r.parse_u8()?;
        }

        // Unknown / removed keys (PROFILES_DIR, LOGS_DIR, ...) are ignored, as
        // `dict.get` would skip them — forward/backward compatibility.
        _ => r.skip_value(0)?,
    }
    Ok(())
}

fn parse_tc(s: &str) -> Result<ThermocoupleType, ConfigError> {
    Ok(match s {
        "B" => ThermocoupleType::B,
        "E" => ThermocoupleType::E,
        "J" => ThermocoupleType::J,
        "K" => ThermocoupleType::K,
        "N" => ThermocoupleType::N,
        "R" => ThermocoupleType::R,
        "S" => ThermocoupleType::S,
        "T" => ThermocoupleType::T,
        "G8" => ThermocoupleType::G8,
        "G32" => ThermocoupleType::G32,
        _ => return Err(ConfigError::BadValue),
    })
}

fn tc_str(t: ThermocoupleType) -> &'static str {
    match t {
        ThermocoupleType::B => "B",
        ThermocoupleType::E => "E",
        ThermocoupleType::J => "J",
        ThermocoupleType::K => "K",
        ThermocoupleType::N => "N",
        ThermocoupleType::R => "R",
        ThermocoupleType::S => "S",
        ThermocoupleType::T => "T",
        ThermocoupleType::G8 => "G8",
        ThermocoupleType::G32 => "G32",
    }
}

fn parse_units(s: &str) -> Result<TempUnits, ConfigError> {
    match s {
        "c" | "C" => Ok(TempUnits::Celsius),
        "f" | "F" => Ok(TempUnits::Fahrenheit),
        _ => Err(ConfigError::BadValue),
    }
}

fn units_str(u: TempUnits) -> &'static str {
    match u {
        TempUnits::Celsius => "c",
        TempUnits::Fahrenheit => "f",
    }
}

/// Bounded recursion guard for `skip_value` on ignored keys.
const MAX_DEPTH: u32 = 16;

/// A panic-safe cursor over the JSON bytes — the same shape as `profile_json`'s
/// reader, kept local so each parser stays self-contained.
struct Reader<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            s,
            b: s.as_bytes(),
            i: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if matches!(c, b' ' | b'\t' | b'\n' | b'\r') {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), ConfigError> {
        if self.peek() == Some(byte) {
            self.i += 1;
            Ok(())
        } else {
            Err(ConfigError::Syntax)
        }
    }

    fn try_consume(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn parse_string(&mut self) -> Result<&'a str, ConfigError> {
        self.expect(b'"')?;
        let start = self.i;
        while let Some(c) = self.peek() {
            match c {
                b'\\' => self.i += 2,
                b'"' => {
                    let slice = self.s.get(start..self.i).ok_or(ConfigError::Syntax)?;
                    self.i += 1;
                    return Ok(slice);
                }
                _ => self.i += 1,
            }
        }
        Err(ConfigError::Syntax)
    }

    /// A string value, with its JSON escapes DECODED into a [`FixedStr`]. (Keys
    /// and enum values use the raw [`parse_string`](Self::parse_string) slice,
    /// which is only ever compared.) Decoding matters for free-text values such
    /// as the WiFi password: [`write_json_str`] escapes `"` and `\\` on save, so a
    /// raw slice would grow by one backslash per save until it overflowed.
    fn parse_text(&mut self) -> Result<Str, ConfigError> {
        let raw = self.parse_string()?;
        let mut out = [0u8; STR_CAP];
        let mut len = 0usize;
        let mut push = |bytes: &[u8]| -> Result<(), ConfigError> {
            let end = len + bytes.len();
            if end > STR_CAP {
                return Err(ConfigError::StringTooLong);
            }
            out[len..end].copy_from_slice(bytes);
            len = end;
            Ok(())
        };
        let mut chars = raw.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                let mut b = [0u8; 4];
                push(c.encode_utf8(&mut b).as_bytes())?;
                continue;
            }
            let decoded = match chars.next().ok_or(ConfigError::Syntax)? {
                '"' => '"',
                '\\' => '\\',
                '/' => '/',
                'b' => '\u{8}',
                'f' => '\u{c}',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                'u' => {
                    let hi = hex4(&mut chars)?;
                    let code = if (0xD800..0xDC00).contains(&hi) {
                        // High surrogate: must be followed by `\uDC00..DFFF`.
                        if chars.next() != Some('\\') || chars.next() != Some('u') {
                            return Err(ConfigError::Syntax);
                        }
                        let lo = hex4(&mut chars)?;
                        if !(0xDC00..0xE000).contains(&lo) {
                            return Err(ConfigError::Syntax);
                        }
                        0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                    } else {
                        hi
                    };
                    match char::from_u32(code).ok_or(ConfigError::Syntax)? {
                        // A NUL would truncate the string for anything C-like
                        // downstream (WiFi SSID/password); no value needs one.
                        '\0' => return Err(ConfigError::BadValue),
                        c => c,
                    }
                }
                _ => return Err(ConfigError::Syntax),
            };
            let mut b = [0u8; 4];
            push(decoded.encode_utf8(&mut b).as_bytes())?;
        }
        let text = core::str::from_utf8(&out[..len]).map_err(|_| ConfigError::Syntax)?;
        FixedStr::from_text(text)
    }

    /// A string, or JSON `null` → `None` (DHCP/unset for the optional IP fields).
    fn parse_opt_string(&mut self) -> Result<Option<Str>, ConfigError> {
        self.skip_ws();
        if self.peek() == Some(b'n') {
            self.skip_literal(b"null")?;
            Ok(None)
        } else {
            Ok(Some(self.parse_text()?))
        }
    }

    /// A non-negative whole number no larger than `max` (an integer field).
    /// Fractions, negatives and out-of-type values are [`ConfigError::BadValue`]
    /// instead of being silently truncated / saturated by an `as` cast.
    fn parse_uint(&mut self, max: u64) -> Result<u64, ConfigError> {
        let v = self.parse_number()?;
        if v >= 0.0 && v <= max as f64 && (v as u64) as f64 == v {
            Ok(v as u64)
        } else {
            Err(ConfigError::BadValue)
        }
    }

    fn parse_u8(&mut self) -> Result<u8, ConfigError> {
        Ok(self.parse_uint(u8::MAX as u64)? as u8)
    }

    fn parse_number(&mut self) -> Result<f64, ConfigError> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E') {
                self.i += 1;
            } else {
                break;
            }
        }
        self.s
            .get(start..self.i)
            .and_then(|t| t.parse::<f64>().ok())
            .ok_or(ConfigError::Syntax)
    }

    fn parse_bool(&mut self) -> Result<bool, ConfigError> {
        match self.peek() {
            Some(b't') => {
                self.skip_literal(b"true")?;
                Ok(true)
            }
            Some(b'f') => {
                self.skip_literal(b"false")?;
                Ok(false)
            }
            _ => Err(ConfigError::Syntax),
        }
    }

    fn parse_pin_array(&mut self) -> Result<PinList, ConfigError> {
        self.expect(b'[')?;
        let mut list = PinList::empty();
        self.skip_ws();
        if self.try_consume(b']') {
            return Ok(list);
        }
        loop {
            self.skip_ws();
            list.push(self.parse_u8()?)?;
            self.skip_ws();
            if self.try_consume(b',') {
                continue;
            }
            self.expect(b']')?;
            return Ok(list);
        }
    }

    fn skip_value(&mut self, depth: u32) -> Result<(), ConfigError> {
        if depth > MAX_DEPTH {
            return Err(ConfigError::Syntax);
        }
        self.skip_ws();
        match self.peek().ok_or(ConfigError::Syntax)? {
            b'"' => {
                self.parse_string()?;
                Ok(())
            }
            b'{' => self.skip_container(depth, b'}'),
            b'[' => self.skip_container(depth, b']'),
            b't' => self.skip_literal(b"true"),
            b'f' => self.skip_literal(b"false"),
            b'n' => self.skip_literal(b"null"),
            c if c.is_ascii_digit() || c == b'-' => {
                self.parse_number()?;
                Ok(())
            }
            _ => Err(ConfigError::Syntax),
        }
    }

    fn skip_container(&mut self, depth: u32, close: u8) -> Result<(), ConfigError> {
        self.i += 1; // opening bracket
        self.skip_ws();
        if self.try_consume(close) {
            return Ok(());
        }
        loop {
            self.skip_ws();
            if close == b'}' {
                self.parse_string()?;
                self.skip_ws();
                self.expect(b':')?;
            }
            self.skip_value(depth + 1)?;
            self.skip_ws();
            if self.try_consume(b',') {
                continue;
            }
            self.expect(close)?;
            return Ok(());
        }
    }

    fn skip_literal(&mut self, lit: &[u8]) -> Result<(), ConfigError> {
        if self.b.get(self.i..self.i + lit.len()) == Some(lit) {
            self.i += lit.len();
            Ok(())
        } else {
            Err(ConfigError::Syntax)
        }
    }
}

/// Four hex digits of a `\uXXXX` escape.
fn hex4(chars: &mut core::str::Chars<'_>) -> Result<u32, ConfigError> {
    let mut v = 0u32;
    for _ in 0..4 {
        let d = chars
            .next()
            .and_then(|c| c.to_digit(16))
            .ok_or(ConfigError::Syntax)?;
        v = v * 16 + d;
    }
    Ok(v)
}

/// Comma/quote bookkeeping for [`KilnConfig::write_json`].
struct Fields<'a, W: Write> {
    w: &'a mut W,
    first: bool,
}

impl<W: Write> Fields<'_, W> {
    fn key(&mut self, k: &str) -> fmt::Result {
        if !self.first {
            self.w.write_char(',')?;
        }
        self.first = false;
        self.w.write_char('"')?;
        self.w.write_str(k)?;
        self.w.write_str("\":")
    }

    fn int(&mut self, k: &str, v: u64) -> fmt::Result {
        self.key(k)?;
        write!(self.w, "{v}")
    }

    fn float(&mut self, k: &str, v: f64) -> fmt::Result {
        self.key(k)?;
        write!(self.w, "{v}")
    }

    fn boolean(&mut self, k: &str, v: bool) -> fmt::Result {
        self.key(k)?;
        self.w.write_str(if v { "true" } else { "false" })
    }

    fn string(&mut self, k: &str, v: &str) -> fmt::Result {
        self.key(k)?;
        write_json_str(self.w, v)
    }

    fn opt(&mut self, k: &str, v: &Option<Str>) -> fmt::Result {
        self.key(k)?;
        match v {
            Some(s) => write_json_str(self.w, s.as_str()),
            None => self.w.write_str("null"),
        }
    }

    fn raw(&mut self, k: &str, v: &str) -> fmt::Result {
        self.key(k)?;
        self.w.write_char('"')?;
        self.w.write_str(v)?;
        self.w.write_char('"')
    }

    fn pins(&mut self, k: &str, v: &PinList) -> fmt::Result {
        self.key(k)?;
        self.w.write_char('[')?;
        for (i, p) in v.as_slice().iter().enumerate() {
            if i > 0 {
                self.w.write_char(',')?;
            }
            write!(self.w, "{p}")?;
        }
        self.w.write_char(']')
    }
}

/// Minimal JSON string escaping (the config values are plain, but stay well-formed).
fn write_json_str<W: Write>(w: &mut W, s: &str) -> fmt::Result {
    w.write_char('"')?;
    for c in s.chars() {
        match c {
            '"' => w.write_str("\\\"")?,
            '\\' => w.write_str("\\\\")?,
            c if (c as u32) < 0x20 => write!(w, "\\u{:04x}", c as u32)?,
            c => w.write_char(c)?,
        }
    }
    w.write_char('"')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_config_example() {
        let c = KilnConfig::default();
        // Hardware pins.
        assert_eq!(c.max31856_sck_pin, 18);
        assert_eq!(c.max31856_cs_pin, 28);
        assert_eq!(c.ssr_pin.as_slice(), &[15]);
        // PID + thermal.
        assert_eq!(c.pid_base(), Gains::new(25.0, 0.14, 160.0));
        assert_eq!(c.thermal_h, 0.0);
        assert_eq!(c.thermal_t_ambient, 25.0);
        // SSR + safety.
        assert_eq!(c.ssr_cycle_time, 20.0);
        assert_eq!(c.ssr_stagger_delay, 0.01);
        assert_eq!(c.max_temp, 1300.0);
        // Watchdog defaults ON (unattended-firing backstop for a hung Core 1).
        assert!(c.enable_watchdog);
        assert_eq!(c.watchdog_timeout, 8000);
        // Recovery + logging.
        assert_eq!(c.max_recovery_temp_delta, 30.0);
        assert_eq!(c.logging_interval, 30);
        // LCD disabled until configured.
        assert!(!c.lcd_enabled);
    }

    #[test]
    fn controller_config_matches_core_defaults() {
        let c = KilnConfig::default().controller_config();
        assert_eq!(c.max_temp, 1300.0);
        assert_eq!(c.rate_measurement_window, 600.0);
        assert_eq!(c.rate_recording_interval, 10.0);
        assert_eq!(c.stall_check_interval, 60.0);
        assert_eq!(c.stall_consecutive_fails, 3);
        assert_eq!(c.stall_min_step_time, 600.0);
        assert_eq!(c.stall_rate_ratio, 0.8);
    }

    #[test]
    fn ms_helpers_convert_seconds() {
        let c = KilnConfig::default();
        assert_eq!(c.temp_read_interval_ms(), 1000);
        assert_eq!(c.status_update_interval_ms(), 5000);
        assert_eq!(c.ssr_update_interval_ms(), 100);
        assert_eq!(c.ssr_stagger_delay_ms(), 10);
    }

    #[test]
    fn empty_object_is_defaults() {
        assert_eq!(parse("{}").unwrap(), KilnConfig::default());
        assert_eq!(parse("  { }  ").unwrap(), KilnConfig::default());
    }

    #[test]
    fn partial_override_changes_only_present_keys() {
        let c = parse(r#"{"PID_KP_BASE": 30.5, "MAX_TEMP": 1250}"#).unwrap();
        assert_eq!(c.pid_kp_base, 30.5);
        assert_eq!(c.max_temp, 1250.0);
        // Everything else is still the default.
        assert_eq!(c.pid_ki_base, 0.14);
        assert_eq!(c.ssr_cycle_time, 20.0);
    }

    #[test]
    fn device_overrides_including_thermocouple_apply() {
        // The shape of the shipped (gitignored) config.json device overrides:
        // the keys must be recognised (values take effect) and absent keys keep
        // their defaults. Guards the UPPER_SNAKE key names + the enum mapping.
        let c = parse(
            r#"{"THERMOCOUPLE_TYPE":"S","MAINS_FREQUENCY":50,
               "PID_KP_BASE":6.11,"PID_KI_BASE":0.0037,"PID_KD_BASE":42.534,
               "THERMAL_T_AMBIENT":10,"MAX_RECOVERY_TEMP_DELTA":60}"#,
        )
        .unwrap();
        assert_eq!(c.thermocouple_type, ThermocoupleType::S);
        assert_eq!(c.mains_frequency, 50);
        assert_eq!(c.pid_base(), Gains::new(6.11, 0.0037, 42.534));
        assert_eq!(c.thermal_t_ambient, 10.0);
        assert_eq!(c.max_recovery_temp_delta, 60.0);
        // Absent keys keep their defaults (sparse merge).
        assert_eq!(c.max_temp, 1300.0);
        assert_eq!(c.logging_interval, 30);
    }

    #[test]
    fn config_example_json_is_valid() {
        // The committed template must parse (the `_comment` key is skipped) and
        // its keys must be recognised — verified via the WiFi placeholder taking
        // effect; the rest of its values equal the built-in defaults.
        let c = parse(include_str!("../../config.example.json")).unwrap();
        assert_eq!(c.wifi_ssid.as_str(), "your_wifi_ssid");
        assert_eq!(c.mains_frequency, 60);
        assert_eq!(c.pid_base(), Gains::new(25.0, 0.14, 160.0));
    }

    #[test]
    fn parse_over_patches_a_running_config() {
        let base = KilnConfig {
            pid_kp_base: 30.0,
            max_temp: 1250.0,
            ..Default::default()
        };
        // A sparse PATCH touches only MAX_TEMP; the prior KP edit survives.
        let patched = parse_over(base, r#"{"MAX_TEMP": 1100}"#).unwrap();
        assert_eq!(patched.max_temp, 1100.0);
        assert_eq!(patched.pid_kp_base, 30.0);
    }

    #[test]
    fn ssr_pin_accepts_int_or_list() {
        assert_eq!(
            parse(r#"{"SSR_PIN": 17}"#).unwrap().ssr_pin.as_slice(),
            &[17]
        );
        let multi = parse(r#"{"SSR_PIN": [15, 16, 17]}"#).unwrap();
        assert_eq!(multi.ssr_pin.as_slice(), &[15, 16, 17]);
    }

    #[test]
    fn too_many_ssr_pins_errors() {
        let json = r#"{"SSR_PIN": [1,2,3,4,5,6,7,8,9,10,11]}"#;
        assert_eq!(parse(json), Err(ConfigError::TooManySsrPins));
    }

    #[test]
    fn parses_wifi_and_static_ip() {
        let json = r#"{"WIFI_SSID": "home", "WIFI_PASSWORD": "secret",
            "WIFI_STATIC_IP": "192.168.1.100", "WIFI_DNS": null}"#;
        let c = parse(json).unwrap();
        assert_eq!(c.wifi_ssid.as_str(), "home");
        assert_eq!(c.wifi_password.as_str(), "secret");
        assert_eq!(c.wifi_static_ip.as_ref().unwrap().as_str(), "192.168.1.100");
        assert_eq!(c.wifi_dns, None);
    }

    #[test]
    fn wifi_is_configured_truth_table() {
        let mut c = KilnConfig::default();
        // default SSID is empty
        assert!(
            !c.wifi_is_configured(),
            "empty SSID must read as unconfigured"
        );

        c.wifi_ssid = Str::from_text("your_wifi_ssid").unwrap();
        assert!(
            !c.wifi_is_configured(),
            "the config.example placeholder must read as unconfigured"
        );

        c.wifi_ssid = Str::from_text("home").unwrap();
        assert!(
            c.wifi_is_configured(),
            "a real SSID must read as configured"
        );
    }

    #[test]
    fn string_too_long_errors() {
        let mut json = String::from(r#"{"WIFI_SSID": ""#);
        for _ in 0..(STR_CAP + 1) {
            json.push('x');
        }
        json.push_str(r#""}"#);
        assert_eq!(parse(&json), Err(ConfigError::StringTooLong));
    }

    #[test]
    fn parses_thermocouple_type_and_units() {
        let c = parse(r#"{"THERMOCOUPLE_TYPE": "S", "TEMP_UNITS": "f"}"#).unwrap();
        assert_eq!(c.thermocouple_type, ThermocoupleType::S);
        assert_eq!(c.temp_units, TempUnits::Fahrenheit);
        assert_eq!(
            parse(r#"{"THERMOCOUPLE_TYPE": "Z"}"#),
            Err(ConfigError::BadValue)
        );
        assert_eq!(parse(r#"{"TEMP_UNITS": "k"}"#), Err(ConfigError::BadValue));
    }

    #[test]
    fn parses_watchdog_bool() {
        assert!(
            parse(r#"{"ENABLE_WATCHDOG": true}"#)
                .unwrap()
                .enable_watchdog
        );
        assert!(
            !parse(r#"{"ENABLE_WATCHDOG": false}"#)
                .unwrap()
                .enable_watchdog
        );
    }

    #[test]
    fn lcd_keys_enable_the_display() {
        let off = parse("{}").unwrap();
        assert!(!off.lcd_enabled);
        let on = parse(r#"{"LCD_I2C_ADDR": 63, "LCD_I2C_SDA": 20}"#).unwrap();
        assert!(on.lcd_enabled);
        assert_eq!(on.lcd_i2c_addr, 63);
        assert_eq!(on.lcd_i2c_sda, 20);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let c = parse(
            r#"{"PROFILES_DIR": "profiles", "FUTURE_FLAG": {"nested": [1,2,3]},
               "PID_KD_BASE": 99.0}"#,
        )
        .unwrap();
        assert_eq!(c.pid_kd_base, 99.0);
    }

    #[test]
    fn log_level_defaults_to_info_and_flash_on() {
        let c = KilnConfig::default();
        assert_eq!(c.log_level, LogLevel::Info);
        assert!(c.log_to_flash);
    }

    #[test]
    fn parse_overrides_log_keys() {
        let c = parse(r#"{"LOG_LEVEL": "debug", "LOG_TO_FLASH": false}"#).unwrap();
        assert_eq!(c.log_level, LogLevel::Debug);
        assert!(!c.log_to_flash);
    }

    #[test]
    fn parse_log_level_off() {
        let c = parse(r#"{"LOG_LEVEL": "off"}"#).unwrap();
        assert_eq!(c.log_level, LogLevel::Off);
    }

    #[test]
    fn bad_log_level_is_an_error() {
        assert!(parse(r#"{"LOG_LEVEL": "verbose"}"#).is_err());
    }

    #[test]
    fn write_json_round_trips_log_keys() {
        let mut c = KilnConfig::default();
        c.log_level = LogLevel::Warn;
        c.log_to_flash = false;
        let mut out = String::new();
        c.write_json(&mut out).unwrap();
        let reparsed = parse(&out).unwrap();
        assert_eq!(reparsed.log_level, LogLevel::Warn);
        assert!(!reparsed.log_to_flash);
    }

    #[test]
    fn malformed_json_errors_without_panicking() {
        for bad in [
            "",
            "{",
            "{\"PID_KP_BASE\"",
            "{\"PID_KP_BASE\":}",
            "{\"PID_KP_BASE\": 1,",
            "not json",
            "[]",
            "{\"ENABLE_WATCHDOG\": tru}",
            "{\"SSR_PIN\": [1,2,}",
        ] {
            assert!(parse(bad).is_err(), "expected Err for {bad:?}");
        }
    }

    #[test]
    fn default_roundtrips_through_json() {
        let c = KilnConfig::default();
        let mut s = String::new();
        c.write_json(&mut s).unwrap();
        assert_eq!(parse(&s).unwrap(), c);
        // The server serializes into a fixed 2048-byte buffer; keep it fitting.
        assert!(s.len() < 2048, "serialized config is {} bytes", s.len());
    }

    // ---- validation / boot policy (K2, H2) ------------------------------

    fn err_key(json: &str) -> Option<&'static str> {
        match apply_patch(KilnConfig::default(), json) {
            Ok(_) => None,
            Err(inv) => inv.key,
        }
    }

    #[test]
    fn defaults_and_shipped_examples_validate() {
        KilnConfig::default().validate().unwrap();
        parse(include_str!("../../config.example.json"))
            .unwrap()
            .validate()
            .unwrap();
        // The shape of the operator's real device config (S-type kiln).
        apply_patch(
            KilnConfig::default(),
            r#"{"WIFI_SSID":"Mesh","WIFI_PASSWORD":"CorrectHorse9",
               "THERMOCOUPLE_TYPE":"S","MAINS_FREQUENCY":50,"PID_KP_BASE":6.11,
               "PID_KI_BASE":0.0037,"PID_KD_BASE":42.534,"THERMAL_H":0.0,
               "THERMAL_T_AMBIENT":10.0,"MAX_RECOVERY_TEMP_DELTA":60.0}"#,
        )
        .unwrap();
    }

    #[test]
    fn audit_h2_values_are_rejected_with_their_key() {
        // Every value the audit got accepted over the network (H2).
        for (json, key) in [
            (r#"{"MAX_TEMP": 1e999}"#, "MAX_TEMP"),
            (r#"{"MAX_TEMP": -1}"#, "MAX_TEMP"),
            (r#"{"MAX_TEMP": 5000}"#, "MAX_TEMP"),
            (r#"{"WATCHDOG_TIMEOUT": 0}"#, "WATCHDOG_TIMEOUT"),
            (r#"{"WATCHDOG_TIMEOUT": 30000}"#, "WATCHDOG_TIMEOUT"),
            (r#"{"SSR_CYCLE_TIME": 0}"#, "SSR_CYCLE_TIME"),
            (r#"{"TEMP_READ_INTERVAL": 1e999}"#, "TEMP_READ_INTERVAL"),
            (r#"{"WEB_SERVER_PORT": 0}"#, "WEB_SERVER_PORT"),
            (r#"{"WEB_SERVER_PORT": 70000}"#, "WEB_SERVER_PORT"),
            (r#"{"WEB_SERVER_PORT": 80.5}"#, "WEB_SERVER_PORT"),
            (r#"{"LOGGING_INTERVAL": -5}"#, "LOGGING_INTERVAL"),
            (r#"{"THERMAL_H": 0.1}"#, "THERMAL_H"),
            (r#"{"THERMOCOUPLE_TYPE": "G8"}"#, "THERMOCOUPLE_TYPE"),
            (r#"{"THERMOCOUPLE_AVERAGING": 3}"#, "THERMOCOUPLE_AVERAGING"),
            (r#"{"MAINS_FREQUENCY": 55}"#, "MAINS_FREQUENCY"),
            (r#"{"TEMP_MEDIAN_WINDOW": 15}"#, "TEMP_MEDIAN_WINDOW"),
            (r#"{"SSR_PIN": 17}"#, "SSR_PIN"),
            (r#"{"SSR_PIN": [15, 15]}"#, "SSR_PIN"),
            (r#"{"SSR_PIN": []}"#, "SSR_PIN"),
            (r#"{"WIFI_PASSWORD": "short"}"#, "WIFI_PASSWORD"),
            (r#"{"WIFI_STATIC_IP": "192.168.1.300"}"#, "WIFI_STATIC_IP"),
            (r#"{"HEAT_WATCH_WINDOW": 0}"#, "HEAT_WATCH_WINDOW"),
            (r#"{"STALL_RATE_RATIO": 2}"#, "STALL_RATE_RATIO"),
        ] {
            assert_eq!(err_key(json), Some(key), "{json}");
        }
    }

    #[test]
    fn cross_key_rules() {
        // A 5 s tick cannot be fed by an 8 s watchdog with a 2x margin...
        assert_eq!(
            err_key(r#"{"TEMP_READ_INTERVAL": 5}"#),
            Some("TEMP_READ_INTERVAL")
        );
        // ...but is fine with a 12 s watchdog or with the watchdog off.
        assert_eq!(
            err_key(r#"{"TEMP_READ_INTERVAL": 5, "WATCHDOG_TIMEOUT": 12000}"#),
            None
        );
        assert_eq!(
            err_key(r#"{"TEMP_READ_INTERVAL": 5, "ENABLE_WATCHDOG": false}"#),
            None
        );
        assert_eq!(
            err_key(r#"{"SSR_UPDATE_INTERVAL": 2}"#),
            Some("SSR_UPDATE_INTERVAL")
        );
        assert_eq!(
            err_key(r#"{"RATE_RECORDING_INTERVAL": 600}"#),
            Some("RATE_RECORDING_INTERVAL")
        );
    }

    #[test]
    fn messages_are_short_ascii_and_name_the_key() {
        let inv = apply_patch(KilnConfig::default(), r#"{"MAX_TEMP": 5000}"#).unwrap_err();
        let mut m = String::new();
        write!(m, "{inv}").unwrap();
        assert_eq!(m, "MAX_TEMP must be between 100 and 1400");
        let inv = apply_patch(KilnConfig::default(), r#"{"MAX_TEMP": "hot"}"#).unwrap_err();
        m.clear();
        write!(m, "{inv}").unwrap();
        assert_eq!(m, "MAX_TEMP has an invalid value or type");
        let inv = apply_patch(KilnConfig::default(), r#"{"MAX_TEMP": 1"#).unwrap_err();
        m.clear();
        write!(m, "{inv}").unwrap();
        assert_eq!(m, "malformed JSON");
        // Every rule/range message fits the 96-byte HTTP message buffer with the
        // "Invalid config: " prefix.
        for k in KEYS {
            for p in [
                Problem::Range { min: -100_000.0, max: 1_000_000.0 },
                Problem::OneOf("B, E, J, K, N, R, S or T"),
                Problem::Rule("must be empty or 8-63 printable ASCII chars"),
                Problem::Rule("must be set explicitly while firing is locked"),
                Problem::Rule("must be at most 60 x RATE_RECORDING_INTERVAL"),
                Problem::Rule("must be 18 (fixed by the firmware wiring)"),
            ] {
                m.clear();
                let inv = Invalid { key: Some(k), other: None, problem: p };
                write!(m, "Invalid config: {inv}").unwrap();
                assert!(m.len() <= 96 && m.is_ascii() && !m.contains('"'), "{m}");
            }
        }
    }

    #[test]
    fn escaped_strings_decode_and_round_trip_stably() {
        // The audit's K2 growth bug: a password with a quote/backslash grew by a
        // backslash on every save (12, 16, 24, 40, then StringTooLong -> boot on
        // defaults). Decoding fixes it: save N times, still identical.
        let mut c = parse(r#"{"WIFI_PASSWORD": "pa\"ss\\w\u00f6rd\ud83d\ude00"}"#).unwrap();
        assert_eq!(c.wifi_password.as_str(), "pa\"ss\\w\u{f6}rd\u{1F600}");
        for _ in 0..10 {
            let mut out = String::new();
            c.write_json(&mut out).unwrap();
            let again = parse(&out).unwrap();
            assert_eq!(again, c);
            c = again;
        }
        // Malformed escapes are syntax errors, not panics.
        for bad in [
            r#"{"WIFI_SSID": "\x"}"#,
            r#"{"WIFI_SSID": "\u12"}"#,
            r#"{"WIFI_SSID": "\ud83d"}"#,
            r#"{"WIFI_SSID": "\ude00"}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn keys_table_matches_write_json() {
        let c = KilnConfig {
            lcd_enabled: true,
            ..KilnConfig::default()
        };
        let mut out = String::new();
        c.write_json(&mut out).unwrap();
        let mut emitted = 0;
        for k in KEYS {
            assert!(out.contains(&format!("\"{k}\":")), "{k} not emitted");
            emitted += 1;
        }
        assert_eq!(out.matches("\":").count(), emitted, "write_json emits a key missing from KEYS");
    }

    #[test]
    fn boot_missing_or_unreadable_locks_on_defaults() {
        let b = evaluate_boot(ConfigSource::Missing);
        assert_eq!(b.lock, Some(FiringLock::ConfigMissing));
        assert_eq!(b.config, KilnConfig::default());
        let b = evaluate_boot(ConfigSource::Unreadable);
        assert_eq!(b.lock, Some(FiringLock::ConfigUnreadable));
        let b = evaluate_boot(ConfigSource::Text(r#"{"THERMOCOUPLE_TYPE": "S","#));
        assert_eq!(b.lock, Some(FiringLock::ConfigUnreadable));
        assert_eq!(b.config, KilnConfig::default());
        assert!(b.issue.is_some());
    }

    #[test]
    fn boot_valid_file_is_unlocked() {
        let b = evaluate_boot(ConfigSource::Text(r#"{"THERMOCOUPLE_TYPE": "S"}"#));
        assert_eq!(b.lock, None);
        assert_eq!(b.config.thermocouple_type, ThermocoupleType::S);
    }

    #[test]
    fn boot_invalid_file_locks_but_keeps_the_valid_keys() {
        // WiFi stays usable (so the operator can reach the board to fix it) and
        // only the bad keys fall back; firing is locked.
        let b = evaluate_boot(ConfigSource::Text(
            r#"{"WIFI_SSID": "home", "THERMOCOUPLE_TYPE": "S", "MAX_TEMP": 5000,
                "TEMP_READ_INTERVAL": 5, "WATCHDOG_TIMEOUT": 8000}"#,
        ));
        assert_eq!(b.lock, Some(FiringLock::ConfigInvalid));
        assert_eq!(b.issue.unwrap().key, Some("MAX_TEMP"));
        assert_eq!(b.config.wifi_ssid.as_str(), "home");
        assert_eq!(b.config.thermocouple_type, ThermocoupleType::S);
        assert_eq!(b.config.max_temp, 1300.0);
        // The cross-key rule dropped both keys back to their defaults.
        assert_eq!(b.config.temp_read_interval, 1.0);
        assert_eq!(b.config.watchdog_timeout, 8000);
        b.config.validate().unwrap();
    }

    #[test]
    fn boot_type_error_keeps_the_other_keys() {
        // A wrong type in one key (here a quoted number) must not cost the WiFi
        // credentials: the key is dropped like an out-of-range one.
        let b = evaluate_boot(ConfigSource::Text(
            r#"{"WIFI_SSID": "home", "WIFI_PASSWORD": "hunter22", "MAINS_FREQUENCY": "60",
                "THERMOCOUPLE_TYPE": "S"}"#,
        ));
        assert_eq!(b.lock, Some(FiringLock::ConfigInvalid));
        assert_eq!(b.issue.unwrap().key, Some("MAINS_FREQUENCY"));
        assert_eq!(b.config.wifi_ssid.as_str(), "home");
        assert_eq!(b.config.wifi_password.as_str(), "hunter22");
        assert_eq!(b.config.thermocouple_type, ThermocoupleType::S);
        assert_eq!(b.config.mains_frequency, KilnConfig::default().mains_frequency);
        // A type error plus a range error: both dropped, still salvaged.
        let b = evaluate_boot(ConfigSource::Text(
            r#"{"WIFI_SSID": "home", "SSR_PIN": "15", "MAX_TEMP": 9999}"#,
        ));
        assert_eq!(b.lock, Some(FiringLock::ConfigInvalid));
        assert_eq!(b.config.wifi_ssid.as_str(), "home");
        b.config.validate().unwrap();
    }

    #[test]
    fn locked_save_must_confirm_the_hardware_keys() {
        // Audit review H1: on a locked board the merge base is the defaults
        // (type K, pin 15). A sparse save is refused, naming the first missing
        // key; a save that states them all goes through.
        let base = evaluate_boot(ConfigSource::Missing).config;
        let inv = apply_patch_confirming(base.clone(), r#"{"WIFI_SSID": "home"}"#).unwrap_err();
        assert_eq!(inv.key, Some("THERMOCOUPLE_TYPE"));
        let mut m = String::new();
        write!(m, "{inv}").unwrap();
        assert_eq!(m, "THERMOCOUPLE_TYPE must be set explicitly while firing is locked");
        let inv = apply_patch_confirming(
            base.clone(),
            r#"{"THERMOCOUPLE_TYPE": "S", "SSR_PIN": [15], "MAX_TEMP": 1300}"#,
        )
        .unwrap_err();
        assert_eq!(inv.key, Some("MAINS_FREQUENCY"));
        // Only a top-level key counts, not a nested or string occurrence.
        let inv = apply_patch_confirming(
            base.clone(),
            r#"{"X": {"MAINS_FREQUENCY": 50}, "Y": "MAINS_FREQUENCY",
                "THERMOCOUPLE_TYPE": "S", "SSR_PIN": [15], "MAX_TEMP": 1300}"#,
        )
        .unwrap_err();
        assert_eq!(inv.key, Some("MAINS_FREQUENCY"));
        let ok = apply_patch_confirming(
            base.clone(),
            r#"{"WIFI_SSID": "home", "THERMOCOUPLE_TYPE": "S", "SSR_PIN": [15],
                "MAINS_FREQUENCY": 50, "MAX_TEMP": 1300}"#,
        )
        .unwrap();
        assert_eq!(ok.thermocouple_type, ThermocoupleType::S);
        assert_eq!(ok.wifi_ssid.as_str(), "home");
        // Still validated after the confirmation check.
        let inv = apply_patch_confirming(
            base,
            r#"{"THERMOCOUPLE_TYPE": "S", "SSR_PIN": [15], "MAINS_FREQUENCY": 50,
                "MAX_TEMP": 5000}"#,
        )
        .unwrap_err();
        assert_eq!(inv.key, Some("MAX_TEMP"));
        // The normal (unlocked) path stays sparse.
        assert!(apply_patch(KilnConfig::default(), r#"{"WIFI_SSID": "home"}"#).is_ok());
    }

    #[test]
    fn rate_window_must_fit_the_history() {
        // 60 samples: 600 s at 10 s is the default; 600 s at 5 s would silently
        // measure only the last 300 s.
        assert_eq!(err_key(r#"{"RATE_MEASUREMENT_WINDOW": 600, "RATE_RECORDING_INTERVAL": 5}"#),
            Some("RATE_MEASUREMENT_WINDOW"));
        assert_eq!(err_key(r#"{"RATE_MEASUREMENT_WINDOW": 300, "RATE_RECORDING_INTERVAL": 5}"#), None);
    }

    #[test]
    fn fixed_wiring_keys_must_match_the_firmware() {
        assert_eq!(err_key(r#"{"MAX31856_SCK_PIN": 5}"#), Some("MAX31856_SCK_PIN"));
        assert_eq!(err_key(r#"{"MAX31856_SPI_ID": 1}"#), Some("MAX31856_SPI_ID"));
        assert_eq!(err_key(r#"{"MAX31856_CS_PIN": 28, "MAX31856_MISO_PIN": 16}"#), None);
        // LCD pins are only checked when the display is on (any LCD_* key
        // turns it on); with it off the defaults are never looked at.
        assert_eq!(err_key(r#"{"LCD_I2C_SCL": 5}"#), Some("LCD_I2C_SCL"));
        assert_eq!(err_key(r#"{"LCD_I2C_ADDR": 39}"#), None);
        let off = KilnConfig {
            lcd_i2c_scl: 5,
            ..KilnConfig::default()
        };
        assert!(!off.lcd_enabled);
        off.validate().unwrap();
    }

    #[test]
    fn escaped_keys_and_nul_are_refused() {
        // An escaped spelling of a real key would otherwise be ignored (keeping
        // the default type K).
        assert!(parse(r#"{"THERMOCOUPLE_\u0054YPE": "S"}"#).is_err());
        let b = evaluate_boot(ConfigSource::Text(r#"{"THERMOCOUPLE_\u0054YPE": "S"}"#));
        assert_eq!(b.lock, Some(FiringLock::ConfigUnreadable));
        assert_eq!(parse(r#"{"WIFI_SSID": "ho\u0000me"}"#).unwrap_err(), ConfigError::BadValue);
    }

    #[test]
    fn trailing_garbage_is_rejected() {
        assert!(parse(r#"{"MAX_TEMP": 1200} trailing"#).is_err());
        assert!(parse("{} \n").is_ok());
    }

    #[test]
    fn customized_config_roundtrips() {
        let json = r#"{
            "MAX31856_CS_PIN": 5,
            "SSR_PIN": [15, 16],
            "WIFI_SSID": "kiln-net",
            "WIFI_STATIC_IP": "10.0.0.5",
            "THERMOCOUPLE_TYPE": "R",
            "TEMP_UNITS": "f",
            "PID_KI_BASE": 0.2,
            "SSR_STAGGER_DELAY": 0.02,
            "ENABLE_WATCHDOG": true,
            "LCD_I2C_ADDR": 39
        }"#;
        let c = parse(json).unwrap();
        let mut s = String::new();
        c.write_json(&mut s).unwrap();
        // Re-parsing the serialized form yields an identical config.
        assert_eq!(parse(&s).unwrap(), c);
        // Spot-check a few of the round-tripped values.
        assert_eq!(c.ssr_pin.as_slice(), &[15, 16]);
        assert_eq!(c.thermocouple_type, ThermocoupleType::R);
        assert!(c.enable_watchdog);
        assert!(c.lcd_enabled);
    }
}
