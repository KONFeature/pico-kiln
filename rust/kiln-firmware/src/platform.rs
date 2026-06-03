//! The device boundary: concrete RP2350 implementations of the `kiln_app` traits
//! ([`Storage`], [`Clock`], [`Display`]) and the `kiln_hal` [`Watchdog`], plus the
//! peripheral builders and the Core 0 setup helpers.
//!
//! VERIFICATION STATUS. Everything here is RP2350-specific and device-verified.
//! Two kinds of code live in this file, kept distinct on purpose:
//!
//! - **Architectural integrations** that are concrete and reviewable: the flash
//!   handshake wrapping every flash *write* ([`FlashStorage`]), the monotonic +
//!   NTP-offset wall clock ([`NtpClock`]), the crash-recovery orchestration that
//!   drives the host-tested `recovery_io`/`recovery` deciders
//!   ([`attempt_recovery`]), and the picoserve timeouts ([`web_config`]).
//! - **Driver bodies** that need the hardware to validate — the cyw43 firmware
//!   load + PIO SPI, the littlefs mount and file ops, the `sntpc` exchange, the
//!   LCD I2C writes, and the raw watchdog/GPIO register pokes. These are marked
//!   `DEVICE` and sketch the intended calls; they are the only unreviewable part.

use core::cell::Cell;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_time::{Duration, Instant};
use kiln_app::api::Directory;
use kiln_app::config::KilnConfig;
use kiln_app::server::{AppState, Clock, Display, RebootSignal, Storage, StorageError};
use kiln_control::ControlParams;
use kiln_core::protocol::{Command, ProfileName, Status};
use kiln_core::state::KilnState;

use crate::flash_handshake;
use crate::{Core0Periphs, Core1Periphs};

// === Watchdog ===============================================================

/// The RP2350 hardware watchdog (`ENABLE_WATCHDOG`, 8 s in `config.example.py`).
/// embassy-rp 0.10's `Watchdog::feed(timeout)` reloads the counter with a fresh
/// period on every pet, so the configured timeout is stored and replayed.
pub struct RpWatchdog {
    inner: embassy_rp::watchdog::Watchdog,
    timeout: Duration,
}

impl kiln_hal::platform::Watchdog for RpWatchdog {
    fn start(&mut self, timeout_ms: u32) {
        self.timeout = Duration::from_millis(timeout_ms as u64);
        self.inner.start(self.timeout);
    }
    fn feed(&mut self) {
        self.inner.feed(self.timeout);
    }
}

/// Honours `ENABLE_WATCHDOG` at runtime with a single concrete type: the Core 1
/// `#[task]` cannot be generic, so a config flag can't pick the `W` type. When
/// disabled this never arms the hardware (`start`/`feed` are no-ops), matching
/// `ENABLE_WATCHDOG = false` (the reference default).
pub enum MaybeWatchdog {
    Enabled(RpWatchdog),
    Disabled,
}

impl kiln_hal::platform::Watchdog for MaybeWatchdog {
    fn start(&mut self, timeout_ms: u32) {
        if let MaybeWatchdog::Enabled(w) = self {
            w.start(timeout_ms);
        }
    }
    fn feed(&mut self) {
        if let MaybeWatchdog::Enabled(w) = self {
            w.feed();
        }
    }
}

/// RAM-resident raw watchdog feed for [`flash_handshake::park_until_idle`] — it
/// runs while flash (XIP) is disabled, so it cannot call the flash-resident
/// `embassy_rp` feed path. DEVICE: writes the watchdog LOAD register directly.
#[link_section = ".data.ram_func"]
#[inline(never)]
pub fn raw_watchdog_feed() {
    // DEVICE: `embassy_rp::pac::WATCHDOG.load().write_value(...)` with the same
    // reload the driver uses; a register poke, no flash access.
    unsafe { core::ptr::read_volatile(&0u32) };
}

/// RAM-safe emergency SSR de-energise for the panic handler. DEVICE: drives the
/// SSR GPIO low via the SIO register, independent of any driver state.
#[link_section = ".data.ram_func"]
#[inline(never)]
pub fn raw_ssr_off() {
    // DEVICE: `embassy_rp::pac::SIO.gpio_out_clr(0).write(|w| w.set_gpio_out_clr(1 << SSR_PIN))`.
}

// === Config (config.json → KilnConfig) ======================================

/// Read `config.json` from flash at boot and parse it — the runtime replacement
/// for the `config.py` the MicroPython build `import`ed. Any failure (absent
/// file, malformed JSON, non-UTF-8) falls back to [`KilnConfig::default`] so the
/// kiln always boots. The parse/fallback is the host-tested `kiln_app::config`;
/// only the flash read ([`FlashStorage::read_config`]) is device I/O.
pub fn load_config(storage: &'static FlashStorage) -> &'static KilnConfig {
    static CONFIG: StaticCell<KilnConfig> = StaticCell::new();
    let mut buf = [0u8; 4096];
    let cfg = match storage.read_config(&mut buf) {
        Ok(n) if n > 0 => core::str::from_utf8(&buf[..n])
            .ok()
            .and_then(|text| kiln_app::config::parse(text).ok())
            .unwrap_or_default(),
        _ => KilnConfig::default(),
    };
    CONFIG.init(cfg)
}

/// Map the loaded [`KilnConfig`] to the Core 1 [`ControlParams`] — pure data, so
/// the safety/PID/timing knobs the control loop reads come from one place.
pub fn control_params_from(cfg: &KilnConfig) -> ControlParams {
    ControlParams {
        controller: cfg.controller_config(),
        pid_base: cfg.pid_base(),
        thermal_h: cfg.thermal_h,
        thermal_t_ambient: cfg.thermal_t_ambient,
        ssr_cycle_time_s: cfg.ssr_cycle_time,
        thermocouple_offset: cfg.thermocouple_offset,
        median_window: cfg.temp_median_window,
        status_update_interval_ms: cfg.status_update_interval_ms(),
        watchdog_timeout_ms: cfg.watchdog_timeout,
        temp_read_interval_ms: cfg.temp_read_interval_ms(),
        ssr_update_interval_ms: cfg.ssr_update_interval_ms(),
    }
}

// === Core 1 kiln I/O ========================================================

/// Build the Core 1 sensor / SSR / watchdog from `cfg`, ready for
/// [`kiln_control::Controller::new`]. The sensor is configured for the
/// `THERMOCOUPLE_TYPE` / `THERMOCOUPLE_AVERAGING` / `MAINS_FREQUENCY` from config;
/// the watchdog is [`MaybeWatchdog`] so `ENABLE_WATCHDOG` is honoured.
///
/// DEVICE: the SPI/CS construction and the SSR pin(s). The SPI clock/MISO/MOSI/CS
/// pins are fixed by the RP2350 pinmux (config carries them for documentation and
/// validation, not runtime re-routing); the SSR GPIO(s) come from `SSR_PIN` via a
/// degraded `AnyPin`, and `SSR_STAGGER_DELAY` feeds `MultiSsr` when more than one
/// is listed.
pub fn build_kiln_io(
    _p: Core1Periphs,
    _cfg: &KilnConfig,
) -> (
    kiln_hal::Max31856<DeviceSpi>,
    kiln_hal::Ssr<DevicePin>,
    MaybeWatchdog,
) {
    // DEVICE: embassy_rp::spi::Spi::new(SPI1, sck, mosi, miso, cfg{1MHz, mode1});
    // wrap with embedded_hal_bus ExclusiveDevice(cs); Max31856::new(spi_dev) then
    // init(cfg.thermocouple_type) + set_averaging(Averaging::from_samples(
    // cfg.thermocouple_averaging)) + set_noise_filter(NoiseFilter::from_hz(
    // cfg.mains_frequency)) + start_autoconverting(). The last call is REQUIRED
    // (hardware.py:84): without it the MAX31856 stays in one-shot mode and the
    // LTCB registers read 0, so read_temperature() returns a constant 0 °C and the
    // control loop never sees real temperature (it will read 0 until the first
    // conversion completes regardless). Ssr::new(Output::new(cfg.ssr_pin[0]
    // .degrade())). MaybeWatchdog per cfg.enable_watchdog.
    unimplemented!("DEVICE: construct SPI + CS + SSR pin(s) + watchdog from Core1Periphs + cfg")
}

/// DEVICE placeholder types for the concrete embassy-rp SPI device / pin the
/// kiln-hal drivers are generic over. They are uninhabited (no value is ever
/// constructed — [`build_kiln_io`] is `unimplemented!()`); the trait impls below
/// exist only so `Max31856<DeviceSpi>` / `Ssr<DevicePin>` satisfy the
/// `embedded-hal` bounds the `Controller` methods require, keeping the crate
/// type-checkable until the real SPI/GPIO wiring lands. Every body is
/// unreachable.
pub enum DeviceSpi {}
pub enum DevicePin {}

impl embedded_hal::spi::ErrorType for DeviceSpi {
    type Error = core::convert::Infallible;
}

impl embedded_hal::spi::SpiDevice<u8> for DeviceSpi {
    fn transaction(
        &mut self,
        _operations: &mut [embedded_hal::spi::Operation<'_, u8>],
    ) -> Result<(), Self::Error> {
        match *self {}
    }
}

impl embedded_hal::digital::ErrorType for DevicePin {
    type Error = core::convert::Infallible;
}

impl embedded_hal::digital::OutputPin for DevicePin {
    fn set_low(&mut self) -> Result<(), Self::Error> {
        match *self {}
    }
    fn set_high(&mut self) -> Result<(), Self::Error> {
        match *self {}
    }
}

// === Wall clock (Oracle Q4) =================================================

/// Milliseconds to add to the monotonic clock to get Unix time, set by NTP. A
/// critical-section `Cell` rather than `AtomicU64`: ARMv8-M has no native 64-bit
/// atomic, and this is read from both cores.
static WALL_OFFSET_MS: BlockingMutex<CriticalSectionRawMutex, Cell<i64>> =
    BlockingMutex::new(Cell::new(0));
/// Set once NTP has synced; before that the clock reports "unknown".
static WALL_SYNCED: BlockingMutex<CriticalSectionRawMutex, Cell<bool>> =
    BlockingMutex::new(Cell::new(false));

/// Monotonic-`embassy-time`-plus-NTP-offset wall clock. The control loop times
/// with the monotonic `Instant`; this is only for status timestamps, CSV rows,
/// and log filenames (the reference's `time.time()`).
pub struct NtpClock;

impl NtpClock {
    fn unix_ms() -> Option<i64> {
        if WALL_SYNCED.lock(|s| s.get()) {
            Some(Instant::now().as_millis() as i64 + WALL_OFFSET_MS.lock(|o| o.get()))
        } else {
            None
        }
    }

    /// Wall-clock seconds as `f64` for the Core 1 control loop (`0.0` pre-sync).
    pub fn unix_seconds_f64() -> f64 {
        Self::unix_ms().map(|ms| ms as f64 / 1000.0).unwrap_or(0.0)
    }

    /// Record the NTP-derived Unix time, computing the offset from the current
    /// monotonic clock.
    fn set_unix_ms(unix_ms: i64) {
        WALL_OFFSET_MS.lock(|o| o.set(unix_ms - Instant::now().as_millis() as i64));
        WALL_SYNCED.lock(|s| s.set(true));
    }
}

impl Clock for NtpClock {
    fn unix_seconds(&self) -> Option<i64> {
        Self::unix_ms().map(|ms| ms / 1000)
    }
}

// === Flash filesystem (Oracle Q2) ===========================================

/// littlefs over the RP2350 flash. Every flash *write* is wrapped in the
/// [`flash_handshake`] so Core 1 de-energises the SSR and parks while XIP is
/// down; reads are plain XIP and need no handshake.
///
/// NOTE: the `littlefs2` dependency is currently commented out in `Cargo.toml`
/// (its `littlefs2-sys` build compiles the bundled C `littlefs`, which needs a
/// freestanding ARM C toolchain + libc headers this environment lacks). Until
/// that is restored, every method below is an `unimplemented!()` / error DEVICE
/// stub and this struct holds no real filesystem handle — re-enable the dep and
/// fill in the bodies together.
pub struct FlashStorage {
    // DEVICE: the mounted littlefs Filesystem + a CriticalSectionRawMutex (writes
    // are sync and do not await, but the lock guards cross-task reentry).
    _private: (),
}

impl FlashStorage {
    /// Run `write` between [`flash_handshake::request_pause`] and
    /// [`flash_handshake::release`] — the safety-critical wrapper around any
    /// flash program/erase.
    fn with_flash_paused<R>(&self, write: impl FnOnce() -> R) -> R {
        flash_handshake::request_pause();
        let r = write();
        flash_handshake::release();
        r
    }
}

impl kiln_app::server::Storage for FlashStorage {
    fn read_chunk(
        &self,
        _dir: Directory,
        _name: &str,
        _offset: u64,
        _buf: &mut [u8],
    ) -> Result<usize, StorageError> {
        // DEVICE: littlefs open + seek(offset) + read into buf (XIP read; no handshake).
        Err(StorageError)
    }

    fn size(&self, _dir: Directory, _name: &str) -> Option<u64> {
        // DEVICE: littlefs metadata size (XIP read; no handshake).
        None
    }

    fn for_each(&self, _dir: Directory, _f: &mut dyn FnMut(&str, u64, u64)) {
        // DEVICE: littlefs dir iter → f(name, size, mtime-attr) (XIP read).
    }

    fn append(
        &self,
        _dir: Directory,
        _name: &str,
        _bytes: &[u8],
        _create: bool,
    ) -> Result<(), StorageError> {
        self.with_flash_paused(|| {
            // DEVICE: littlefs open (truncate if create else append), write, sync.
            Err(StorageError)
        })
    }

    fn remove(&self, _dir: Directory, _name: &str) -> Result<(), StorageError> {
        self.with_flash_paused(|| Err(StorageError)) // DEVICE: littlefs remove
    }

    fn remove_all(&self, _dir: Directory) -> Result<(), StorageError> {
        self.with_flash_paused(|| Err(StorageError)) // DEVICE: iterate + remove
    }

    fn upload_begin(&self) -> Result<(), StorageError> {
        self.with_flash_paused(|| Err(StorageError)) // DEVICE: truncate scratch
    }
    fn upload_write(&self, _bytes: &[u8]) -> Result<(), StorageError> {
        self.with_flash_paused(|| Err(StorageError)) // DEVICE: append scratch
    }
    fn upload_commit(&self, _dir: Directory, _name: &str) -> Result<(), StorageError> {
        self.with_flash_paused(|| Err(StorageError)) // DEVICE: rename scratch → dir/name
    }
    fn upload_abort(&self) {
        let _ = self.with_flash_paused(|| -> Result<(), StorageError> { Ok(()) });
        // DEVICE: remove scratch
    }

    fn static_asset(&self, name: &str) -> Option<&'static [u8]> {
        // The web UI is compiled into flash (the reference cached it in RAM).
        match name {
            "index.html" => Some(include_bytes!("../../../static/index.html")),
            "tuning.html" => Some(include_bytes!("../../../static/tuning.html")),
            _ => None,
        }
    }

    fn read_config(&self, _buf: &mut [u8]) -> Result<usize, StorageError> {
        // DEVICE: littlefs open "/config.json" + read into buf (XIP read; no handshake).
        Err(StorageError)
    }

    fn write_config(&self, _bytes: &[u8]) -> Result<(), StorageError> {
        // DEVICE: littlefs write "/config.json" via a temp file + atomic rename.
        self.with_flash_paused(|| Err(StorageError))
    }
}

// === LCD ====================================================================

/// The character LCD status line (`main.py`). DEVICE: the I2C HD44780 writes.
pub struct LcdDisplay {
    _private: (),
}

impl Display for LcdDisplay {
    fn show(&mut self, _status: &Status) {
        // DEVICE: format a two-line summary (state, temp/target) and write it
        // over I2C. Presentation only — no control decision here.
    }
}

// === Core 0 setup ===========================================================

use static_cell::StaticCell;

/// picoserve timeouts (`web_server.py` connection limits), built once.
pub fn web_config() -> &'static picoserve::Config {
    static CONFIG: StaticCell<picoserve::Config> = StaticCell::new();
    CONFIG.init(
        picoserve::Config::new(picoserve::Timeouts {
            start_read_request: Duration::from_secs(5),
            persistent_start_read_request: Duration::from_secs(1),
            read_request: Duration::from_secs(1),
            write: Duration::from_secs(1),
        })
        .keep_connection_alive(),
    )
}

/// Bring up cyw43 → an `embassy-net` `Stack`, join WiFi, and run DHCP. DEVICE:
/// firmware-blob load, PIO SPI, the cyw43 + net runner tasks, then `join_wpa2`
/// **in a retry loop** (disconnect → wait 2 s → re-join) until it succeeds, so a
/// transient initial-join failure recovers — `wifi_manager.connect`'s retry,
/// which cyw43's built-in link auto-reconnect (drops only) does not cover.
pub async fn init_network(
    _spawner: &embassy_executor::Spawner,
    _p: &Core0Periphs,
) -> embassy_net::Stack<'static> {
    unimplemented!("DEVICE: cyw43 init + embassy_net stack + WiFi join (retry until up)")
}

/// WiFi reconnect monitor — the steady-state half of `wifi_manager.monitor`
/// (`wifi_manager.py:139-180`): every few seconds check the link and, if it is
/// down, re-join. cyw43 auto-reconnects dropped links, but the explicit
/// disconnect→reconnect on a *failed* state is what the reference adds; mirror it
/// here so a kiln on a flaky AP keeps its web/NTP reachable. DEVICE: the cyw43
/// `control` handle for the status read + re-join.
#[embassy_executor::task]
pub async fn wifi_monitor_task(_stack: embassy_net::Stack<'static>) -> ! {
    loop {
        // DEVICE: if the link is down (STAT_NO_AP_FOUND/CONNECT_FAIL equivalent),
        // control.join_wpa2(ssid, pw) again (disconnect → wait 2 s → join).
        embassy_time::Timer::after(Duration::from_secs(5)).await;
    }
}

/// Mount littlefs and return the shared [`FlashStorage`]. Called at boot before
/// the core split so [`load_config`] can read `config.json` and hand both cores
/// their config. DEVICE: flash driver + littlefs mount (format on first boot).
pub fn init_storage() -> &'static FlashStorage {
    static STORAGE: StaticCell<FlashStorage> = StaticCell::new();
    STORAGE.init(FlashStorage { _private: () })
}

pub fn init_clock() -> &'static NtpClock {
    static CLOCK: StaticCell<NtpClock> = StaticCell::new();
    CLOCK.init(NtpClock)
}

pub fn init_display(_p: &Core0Periphs) -> &'static mut LcdDisplay {
    static DISPLAY: StaticCell<LcdDisplay> = StaticCell::new();
    DISPLAY.init(LcdDisplay { _private: () })
}

/// Crash recovery (`server/recovery.py`): find the most recent profile log,
/// parse its last line, and — if the run was interrupted mid-firing within the
/// safe temperature delta — resume it. The *decisions* use the host-tested
/// `recovery_io` + `kiln_core::recovery`; only the directory scan and the file
/// read are device I/O. The resume profile is parsed here on Core 0 and shipped
/// to Core 1, like every other run.
pub async fn attempt_recovery(state: &AppState) -> Option<kiln_app::server::RecoveryLog> {
    use kiln_app::recovery_io;

    // Wait for the first valid (>= 20°C) temperature, as the reference does.
    let current_temp = loop {
        let s = state.latest();
        if s.current_temp >= 20.0 {
            break s.current_temp;
        }
        embassy_time::Timer::after(Duration::from_millis(500)).await;
    };

    // Most recent non-tuning .csv by mtime (DEVICE: the listdir + mtime sort;
    // the candidate filter is `recovery_io::is_recovery_candidate`).
    let mut newest: Option<(heapless::String<64>, u64)> = None;
    state
        .storage
        .for_each(Directory::Logs, &mut |name, _size, modified| {
            if recovery_io::is_recovery_candidate(name) {
                let newer = newest.as_ref().map(|(_, m)| modified > *m).unwrap_or(true);
                if newer {
                    let mut n = heapless::String::new();
                    if n.push_str(name).is_ok() {
                        newest = Some((n, modified));
                    }
                }
            }
        });
    let (log_name, _) = newest?;

    // Read the tail and decide.
    let mut buf = [0u8; 4096];
    let read = state
        .storage
        .size(Directory::Logs, &log_name)
        .and_then(|size| {
            let start = size.saturating_sub(buf.len() as u64);
            state
                .storage
                .read_chunk(Directory::Logs, &log_name, start, &mut buf)
                .ok()
        });
    let n = read?;
    let text = core::str::from_utf8(&buf[..n]).ok()?;
    let entry = recovery_io::last_log_entry_from_csv(text)?;

    let decision = kiln_core::recovery::check_recovery(
        &entry,
        current_temp,
        state.config.max_recovery_temp_delta,
    );
    if !decision.can_recover {
        return None;
    }

    // Profile name from the log filename (lowercased), then parse profiles/{name}.json.
    let stem = recovery_io::profile_stem(&log_name)?;
    let mut fname = heapless::String::<80>::new();
    if recovery_io::write_lowercase(&mut fname, stem).is_err() || fname.push_str(".json").is_err() {
        return None;
    }
    let mut pbuf = [0u8; 8192];
    state.storage.size(Directory::Profiles, &fname)?;
    // Same transient-glitch retry as the run/schedule load path
    // (control_thread.load_profile_with_retry): 3 attempts, 0.5 s/1.0 s backoff.
    let pn =
        kiln_app::server::read_file_with_retry(state.storage, Directory::Profiles, &fname, &mut pbuf)
            .await?;
    let ptext = core::str::from_utf8(&pbuf[..pn]).ok()?;
    let parsed = kiln_app::profile_json::parse_profile(ptext).ok()?;
    let profile = ProfileName::new(&fname).ok()?;

    let _ = state.commands.try_send(Command::ResumeProfile {
        profile,
        parsed,
        elapsed_seconds: decision.elapsed_seconds,
        last_logged_temp: Some(decision.last_temp),
        current_temp: Some(current_temp),
        step_index: decision.step_index,
    });

    // Hand the CSV logger the interrupted run's file so it appends (no new header)
    // and writes the one-shot RECOVERY event row — data_logger.set_recovery_context.
    let mut filename = heapless::String::<96>::new();
    let _ = filename.push_str(&log_name); // String<64> always fits in String<96>
    Some(kiln_app::server::RecoveryLog {
        filename,
        elapsed_seconds: decision.elapsed_seconds,
    })
}

/// NTP task: periodically sync the wall clock via `sntpc`. DEVICE: the UDP
/// exchange; on success it calls [`NtpClock::set_unix_ms`].
#[embassy_executor::task]
pub async fn ntp_task(_clock: &'static NtpClock, _stack: embassy_net::Stack<'static>) -> ! {
    loop {
        // DEVICE: sntpc::get_time(pool.ntp.org, socket) → NtpClock::set_unix_ms(unix_ms).
        let _ = NtpClock::set_unix_ms; // referenced; DEVICE call elided
        embassy_time::Timer::after(Duration::from_secs(3600)).await;
    }
}

/// Reboot task: wait for `/api/reboot`, drain the response, then reset. DEVICE:
/// `cortex_m::peripheral::SCB::sys_reset()`.
#[embassy_executor::task]
pub async fn reboot_task(reboot: &'static RebootSignal) -> ! {
    reboot.wait().await;
    embassy_time::Timer::after(Duration::from_millis(500)).await;
    cortex_m::peripheral::SCB::sys_reset()
}

/// Force-off helper used by the LCD/idle transitions if needed; kept with the
/// other RP2350 specifics. (Reserved.)
#[allow(dead_code)]
fn _state_is_active(s: KilnState) -> bool {
    matches!(s, KilnState::Running | KilnState::Tuning)
}
