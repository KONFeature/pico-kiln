//! `config.json` → [`KilnConfig`] — the single source of truth for every tunable
//! the controller reads, ported from the flat `config.py` module the MicroPython
//! build `import`ed at boot. Same knobs, same names: each JSON key is the exact
//! `config.py` identifier (`PID_KP_BASE`, `SSR_CYCLE_TIME`, `WIFI_SSID`, ...), so
//! a `config.py` maps to a `config.json` one-to-one.
//!
//! Loading model (the device has no module system to `import`): `kiln-firmware`
//! reads `config.json` from the littlefs flash at boot and calls [`parse`]; a
//! missing or malformed file falls back to [`KilnConfig::default`] (the
//! `config.example.py` values) so the kiln always boots. [`parse`] is a *partial*
//! override — only the keys present are applied, exactly like editing a sparse
//! `config.py` — and unknown keys are ignored for forward compatibility.
//!
//! Like `profile_json`, this is a hand-rolled, panic-safe, allocation-free reader
//! (no `serde`) so the pure layer stays dependency-free and host-testable.
//! [`KilnConfig::write_json`] re-emits the whole document for `GET /api/config`.

use core::fmt::{self, Write};

use kiln_core::gain_schedule::Gains;
use kiln_core::state::ControllerConfig;
use kiln_hal::ThermocoupleType;

/// Maximum SSR relays the `SSR_PIN` list accepts — `config.example.py` notes the
/// stagger logic "supports up to 10 SSRs".
pub const MAX_SSR: usize = 10;

/// Capacity for the network string fields (SSID/password/IP literals).
pub const STR_CAP: usize = 64;

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
    /// An enumerated value (`THERMOCOUPLE_TYPE`, `TEMP_UNITS`) was unrecognised.
    BadValue,
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
    pub rate_measurement_window: f64,
    pub rate_recording_interval: f64,

    // --- Logging + recovery + watchdog ---
    pub logging_interval: u32,
    pub max_recovery_temp_delta: f64,
    pub enable_watchdog: bool,
    pub watchdog_timeout: u32,

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
            rate_measurement_window: 600.0,
            rate_recording_interval: 10.0,

            logging_interval: 30,
            max_recovery_temp_delta: 30.0,
            enable_watchdog: false,
            watchdog_timeout: 8000,

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
    /// The safety/rate/stall subset, as the `kiln-core` controller wants it.
    pub fn controller_config(&self) -> ControllerConfig {
        ControllerConfig {
            max_temp: self.max_temp as f32,
            rate_measurement_window: self.rate_measurement_window as f32,
            rate_recording_interval: self.rate_recording_interval as f32,
            stall_check_interval: self.stall_check_interval as f32,
            stall_consecutive_fails: self.stall_consecutive_fails,
            stall_min_step_time: self.stall_min_step_time as f32,
        }
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
        j.float("RATE_MEASUREMENT_WINDOW", self.rate_measurement_window)?;
        j.float("RATE_RECORDING_INTERVAL", self.rate_recording_interval)?;

        j.int("LOGGING_INTERVAL", self.logging_interval as u64)?;
        j.float("MAX_RECOVERY_TEMP_DELTA", self.max_recovery_temp_delta)?;
        j.boolean("ENABLE_WATCHDOG", self.enable_watchdog)?;
        j.int("WATCHDOG_TIMEOUT", self.watchdog_timeout as u64)?;

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

/// Parse a `config.json` document, applying each present key over the defaults
/// (the boot / whole-file case).
pub fn parse(json: &str) -> Result<KilnConfig, ConfigError> {
    parse_over(KilnConfig::default(), json)
}

/// Apply the present keys of `json` over `base`, leaving absent keys untouched —
/// the `PATCH`-style merge `POST /api/config` uses so a sparse body edits only
/// the named knobs of the running config rather than resetting the rest.
pub fn parse_over(base: KilnConfig, json: &str) -> Result<KilnConfig, ConfigError> {
    let mut cfg = base;
    let mut r = Reader::new(json);
    r.skip_ws();
    r.expect(b'{')?;
    r.skip_ws();
    if r.try_consume(b'}') {
        return Ok(cfg);
    }
    loop {
        r.skip_ws();
        let key = r.parse_string()?;
        r.skip_ws();
        r.expect(b':')?;
        r.skip_ws();
        apply_key(&mut cfg, key, &mut r)?;
        r.skip_ws();
        if r.try_consume(b',') {
            continue;
        }
        r.expect(b'}')?;
        break;
    }
    Ok(cfg)
}

fn apply_key(cfg: &mut KilnConfig, key: &str, r: &mut Reader) -> Result<(), ConfigError> {
    match key {
        "MAX31856_SPI_ID" => cfg.max31856_spi_id = r.parse_number()? as u8,
        "MAX31856_SCK_PIN" => cfg.max31856_sck_pin = r.parse_number()? as u8,
        "MAX31856_MOSI_PIN" => cfg.max31856_mosi_pin = r.parse_number()? as u8,
        "MAX31856_MISO_PIN" => cfg.max31856_miso_pin = r.parse_number()? as u8,
        "MAX31856_CS_PIN" => cfg.max31856_cs_pin = r.parse_number()? as u8,
        // SSR_PIN is an int (single relay) or an array (staggered multi-relay).
        "SSR_PIN" => {
            r.skip_ws();
            cfg.ssr_pin = if r.peek() == Some(b'[') {
                r.parse_pin_array()?
            } else {
                PinList::single(r.parse_number()? as u8)
            };
        }

        "WIFI_SSID" => cfg.wifi_ssid = FixedStr::from_text(r.parse_string()?)?,
        "WIFI_PASSWORD" => cfg.wifi_password = FixedStr::from_text(r.parse_string()?)?,
        "WIFI_STATIC_IP" => cfg.wifi_static_ip = r.parse_opt_string()?,
        "WIFI_SUBNET" => cfg.wifi_subnet = r.parse_opt_string()?,
        "WIFI_GATEWAY" => cfg.wifi_gateway = r.parse_opt_string()?,
        "WIFI_DNS" => cfg.wifi_dns = r.parse_opt_string()?,
        "WEB_SERVER_HOST" => cfg.web_server_host = FixedStr::from_text(r.parse_string()?)?,
        "WEB_SERVER_PORT" => cfg.web_server_port = r.parse_number()? as u16,

        "THERMOCOUPLE_TYPE" => cfg.thermocouple_type = parse_tc(r.parse_string()?)?,
        "TEMP_UNITS" => cfg.temp_units = parse_units(r.parse_string()?)?,
        "THERMOCOUPLE_OFFSET" => cfg.thermocouple_offset = r.parse_number()?,
        "MAINS_FREQUENCY" => cfg.mains_frequency = r.parse_number()? as u16,
        "THERMOCOUPLE_AVERAGING" => cfg.thermocouple_averaging = r.parse_number()? as u8,
        "TEMP_MEDIAN_WINDOW" => cfg.temp_median_window = r.parse_number()? as usize,

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
        "STALL_CONSECUTIVE_FAILS" => cfg.stall_consecutive_fails = r.parse_number()? as u32,
        "STALL_MIN_STEP_TIME" => cfg.stall_min_step_time = r.parse_number()?,
        "RATE_MEASUREMENT_WINDOW" => cfg.rate_measurement_window = r.parse_number()?,
        "RATE_RECORDING_INTERVAL" => cfg.rate_recording_interval = r.parse_number()?,

        "LOGGING_INTERVAL" => cfg.logging_interval = r.parse_number()? as u32,
        "MAX_RECOVERY_TEMP_DELTA" => cfg.max_recovery_temp_delta = r.parse_number()?,
        "ENABLE_WATCHDOG" => cfg.enable_watchdog = r.parse_bool()?,
        "WATCHDOG_TIMEOUT" => cfg.watchdog_timeout = r.parse_number()? as u32,

        "LCD_I2C_ID" => {
            cfg.lcd_enabled = true;
            cfg.lcd_i2c_id = r.parse_number()? as u8;
        }
        "LCD_I2C_SCL" => {
            cfg.lcd_enabled = true;
            cfg.lcd_i2c_scl = r.parse_number()? as u8;
        }
        "LCD_I2C_SDA" => {
            cfg.lcd_enabled = true;
            cfg.lcd_i2c_sda = r.parse_number()? as u8;
        }
        "LCD_I2C_FREQ" => {
            cfg.lcd_enabled = true;
            cfg.lcd_i2c_freq = r.parse_number()? as u32;
        }
        "LCD_I2C_ADDR" => {
            cfg.lcd_enabled = true;
            cfg.lcd_i2c_addr = r.parse_number()? as u8;
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

    /// A string, or JSON `null` → `None` (DHCP/unset for the optional IP fields).
    fn parse_opt_string(&mut self) -> Result<Option<Str>, ConfigError> {
        self.skip_ws();
        if self.peek() == Some(b'n') {
            self.skip_literal(b"null")?;
            Ok(None)
        } else {
            Ok(Some(FixedStr::from_text(self.parse_string()?)?))
        }
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
            list.push(self.parse_number()? as u8)?;
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
        // Watchdog defaults OFF, like the reference.
        assert!(!c.enable_watchdog);
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
