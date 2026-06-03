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
use crate::{Core0Periphs, Core1Periphs, LcdPeriphs};

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
/// `embassy_rp` feed path.
///
/// Reloads the watchdog down-counter to the maximum LOAD so the brief flash-park
/// spin cannot trip a reset before Core 0 finishes the flash write; the normal
/// per-tick driver feed (with the configured timeout) takes back over once the
/// loop resumes. `pac::…as_ptr()` is a `const fn` (a compile-time address) and
/// the `write_volatile` inlines, so this touches only the register, never flash.
/// Harmless when `ENABLE_WATCHDOG=false` (the counter is simply not armed).
#[link_section = ".data.ram_func"]
#[inline(never)]
pub fn raw_watchdog_feed() {
    const WATCHDOG_LOAD_MASK: u32 = 0x00ff_ffff; // LOAD is a 24-bit field
    let load = rp_pac::WATCHDOG.load().as_ptr() as *mut u32;
    unsafe { core::ptr::write_volatile(load, WATCHDOG_LOAD_MASK) };
}

/// Bitmask of the GPIOs (bank 0) wired to SSRs, so the RAM-resident
/// [`raw_ssr_off`] can de-energise *every* configured relay without reading the
/// runtime config (which it cannot, from its XIP-down context). Defaults to the
/// reference's single PIN_15; [`build_kiln_io`] overwrites it once the configured
/// `SSR_PIN` list is known. A plain `AtomicU32` load is a single `ldr` (no flash
/// call), so reading it stays safe with XIP down.
static SSR_PIN_MASK: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(1 << 15);

/// RAM-safe emergency SSR de-energise for the panic handler — drives every
/// configured SSR GPIO low independent of any driver state.
///
/// Clears the output bits (de-energise) and asserts the output-enables so the
/// pins actively drive low even if `OE` was glitched. All SSR pins live in bank 0
/// (GPIO 0..31), so `gpio_out(0)` / `gpio_oe(0)`. Const-address register pokes +
/// one atomic load only — no flash access, safe to run with XIP down.
#[link_section = ".data.ram_func"]
#[inline(never)]
pub fn raw_ssr_off() {
    let mask = SSR_PIN_MASK.load(core::sync::atomic::Ordering::Relaxed);
    let out_clr = rp_pac::SIO.gpio_out(0).value_clr().as_ptr();
    let oe_set = rp_pac::SIO.gpio_oe(0).value_set().as_ptr();
    unsafe {
        core::ptr::write_volatile(out_clr, mask);
        core::ptr::write_volatile(oe_set, mask);
    }
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

use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::SPI0;
use embassy_rp::spi::{Config as SpiConfig, Phase, Polarity, Spi};
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use kiln_hal::max31856::{Averaging, NoiseFilter};

/// The concrete Core 1 SPI device: embassy-rp's blocking **SPI0** with a
/// GPIO chip-select, wrapped as an `embedded-hal` `SpiDevice` (it owns CS).
///
/// SPI0 (not SPI1): the MAX31856 wiring (`MAX31856_SPI_ID = 0` in the reference)
/// uses PIN_18/19/16 for SCK/MOSI/MISO, which are SPI0 function pins on the
/// RP2350 — `ClkPin<SPI1>` etc. are not implemented for them. CS (PIN_28) is
/// bit-banged by `ExclusiveDevice`, so its pin function is irrelevant.
pub type DeviceSpi =
    ExclusiveDevice<Spi<'static, SPI0, embassy_rp::spi::Blocking>, Output<'static>, NoDelay>;
/// Maximum simultaneously-driven SSRs (the config `MAX_SSR`). Also the size of
/// the reserved candidate-pin pool (`main.rs`'s `ssr_pool`: GPIO
/// 15/14/13/12/11/10/9/8/7/6, PIN_15 = the reference default — chosen to avoid
/// SPI0 16/18/19/28, I2C0 20/21, and the cyw43 pins 23–25/29). The RP2350 pinmux
/// is compile-time, so a runtime `SSR_PIN` number is honoured only if it is one
/// of these.
pub const MAX_SSR: usize = 10;

/// Build the Core 1 sensor / SSR / watchdog from `cfg`, ready for
/// [`kiln_control::Controller::new`]. The sensor is configured for the
/// `THERMOCOUPLE_TYPE` / `THERMOCOUPLE_AVERAGING` / `MAINS_FREQUENCY` from config;
/// the watchdog is [`MaybeWatchdog`] so `ENABLE_WATCHDOG` is honoured.
///
/// The SPI clock/MISO/MOSI/CS pins are fixed by the RP2350 pinmux (config carries
/// their numbers for documentation, not runtime re-routing). Config-write errors
/// (a transient boot-time SPI glitch) are swallowed rather than panicked: the
/// control loop's `temp_filter` already treats a faulting/unreadable sensor as a
/// fault and shuts the SSR, so a panic-reset loop here would be strictly worse.
///
/// SSRs: `SSR_PIN` (a single int or a list, up to [`MAX_SSR`]) is honoured by
/// selecting the matching pins from the reserved pool (see [`MAX_SSR`]) in
/// config order; unmatched numbers are skipped, and if none match it falls back
/// to the default PIN_15. `SSR_STAGGER_DELAY` spaces multi-relay turn-on/off
/// (inrush limiting). The chosen pins' bitmask is published to [`SSR_PIN_MASK`]
/// so the panic-handler [`raw_ssr_off`] de-energises exactly them.
pub fn build_kiln_io(
    p: Core1Periphs,
    cfg: &KilnConfig,
) -> (kiln_hal::Max31856<DeviceSpi>, ConfiguredSsr, MaybeWatchdog) {
    // SPI0 @ 1 MHz, MAX31856 = SPI mode 1 (CPOL=0 idle-low, CPHA=1 capture on the
    // second edge).
    let mut spi_cfg = SpiConfig::default();
    spi_cfg.frequency = 1_000_000;
    spi_cfg.polarity = Polarity::IdleLow;
    spi_cfg.phase = Phase::CaptureOnSecondTransition;
    let spi = Spi::new_blocking(p.spi, p.sck, p.mosi, p.miso, spi_cfg);
    // CS idle-high; `ExclusiveDevice` drives it low only for the duration of each
    // transaction. The pin error is `Infallible`, so `new_no_delay` cannot fail.
    let cs = Output::new(p.cs, Level::High);
    let dev = ExclusiveDevice::new_no_delay(spi, cs).unwrap();

    let mut sensor = kiln_hal::Max31856::new(dev);
    // init: mask off all fault asserts + open-circuit detection + the thermocouple
    // type; then hardware averaging and the mains notch; then START AUTOCONVERTING
    // (REQUIRED — hardware.py:84). Without it the chip stays one-shot and the LTCB
    // registers read 0, so the loop would see a constant 0 °C. Invalid config
    // values fall back to the kiln defaults (8 samples / 60 Hz), matching the
    // reference's `unwrap_or_default` behaviour.
    let _ = sensor.init(cfg.thermocouple_type);
    let _ = sensor.set_averaging(
        Averaging::from_samples(cfg.thermocouple_averaging).unwrap_or_default(),
    );
    let _ = sensor.set_noise_filter(NoiseFilter::from_hz(cfg.mains_frequency).unwrap_or_default());
    let _ = sensor.start_autoconverting();

    // Select the configured SSR pins from the reserved pool, in config order.
    let mut pool: [Option<(u8, Output<'static>)>; MAX_SSR] = p.ssr_pool.map(Some);
    let mut relays: heapless::Vec<Output<'static>, MAX_SSR> = heapless::Vec::new();
    let mut mask: u32 = 0;
    for &want in cfg.ssr_pin.as_slice() {
        if let Some(slot) = pool
            .iter_mut()
            .find(|s| matches!(s, Some((num, _)) if *num == want))
        {
            let (num, out) = slot.take().unwrap();
            let _ = relays.push(out);
            mask |= 1 << num;
        }
    }
    // Fallback: no configured pin matched the pool → use the default PIN_15.
    if relays.is_empty() {
        if let Some(slot) = pool
            .iter_mut()
            .find(|s| matches!(s, Some((15, _))))
        {
            let (num, out) = slot.take().unwrap();
            let _ = relays.push(out);
            mask |= 1 << num;
        }
    }
    // Publish the mask for the RAM-resident panic de-energise. (Unselected pool
    // `Output`s drop here, releasing their pins.)
    SSR_PIN_MASK.store(mask, core::sync::atomic::Ordering::Release);
    let ssr = ConfiguredSsr::new(relays, cfg.ssr_stagger_delay_ms());

    // Watchdog per ENABLE_WATCHDOG. When disabled, `p.watchdog` is simply dropped.
    let watchdog = if cfg.enable_watchdog {
        MaybeWatchdog::Enabled(RpWatchdog {
            inner: embassy_rp::watchdog::Watchdog::new(p.watchdog),
            timeout: Duration::from_millis(cfg.watchdog_timeout as u64),
        })
    } else {
        MaybeWatchdog::Disabled
    };

    (sensor, ssr, watchdog)
}

/// One or more SSRs driven as one logical output with staggered turn-on/off — a
/// runtime-sized analogue of [`kiln_hal::MultiSsr`] (whose relay count is a const
/// generic, which we cannot pick from runtime config). Same behaviour: on a
/// rising/falling edge each relay switches once its `i * stagger_ms` delay has
/// elapsed (inrush limiting), while [`force_off`](ConfiguredSsr::force_off) — the
/// emergency path — drops every relay at once. Backs the host-tested
/// `ssr_schedule` decisions exactly as `MultiSsr` does.
pub struct ConfiguredSsr {
    pins: heapless::Vec<Output<'static>, MAX_SSR>,
    on: [bool; MAX_SSR],
    stagger_ms: u64,
    logical_on: bool,
    rising_edge_ms: u64,
    falling_edge_ms: u64,
}

impl ConfiguredSsr {
    fn new(pins: heapless::Vec<Output<'static>, MAX_SSR>, stagger_ms: u64) -> Self {
        // `Output::new` already drove every pin low, so the relays start off.
        Self {
            pins,
            on: [false; MAX_SSR],
            stagger_ms,
            logical_on: false,
            rising_edge_ms: 0,
            falling_edge_ms: 0,
        }
    }
}

impl kiln_hal::platform::SsrOutput for ConfiguredSsr {
    // embassy-rp `Output` pin writes are infallible.
    type Error = core::convert::Infallible;

    fn apply(&mut self, on: bool, now_ms: u64) -> Result<(), Self::Error> {
        if on {
            if !self.logical_on {
                self.logical_on = true;
                self.rising_edge_ms = now_ms;
            }
            let elapsed = now_ms.saturating_sub(self.rising_edge_ms);
            for (i, pin) in self.pins.iter_mut().enumerate() {
                if !self.on[i] && elapsed >= (i as u64) * self.stagger_ms {
                    pin.set_high();
                    self.on[i] = true;
                }
            }
        } else {
            if self.logical_on {
                self.logical_on = false;
                self.falling_edge_ms = now_ms;
            }
            let elapsed = now_ms.saturating_sub(self.falling_edge_ms);
            for (i, pin) in self.pins.iter_mut().enumerate() {
                if self.on[i] && elapsed >= (i as u64) * self.stagger_ms {
                    pin.set_low();
                    self.on[i] = false;
                }
            }
        }
        Ok(())
    }

    fn force_off(&mut self) -> Result<(), Self::Error> {
        for pin in self.pins.iter_mut() {
            pin.set_low();
        }
        self.logical_on = false;
        self.on = [false; MAX_SSR];
        Ok(())
    }
}

impl Drop for ConfiguredSsr {
    fn drop(&mut self) {
        for pin in self.pins.iter_mut() {
            pin.set_low();
        }
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

// === Flash filesystem (littlefs2 over the RP2350 QSPI flash) =================
//
// One littlefs2 mount over the reserved top partition holds everything the
// reference kept on its MicroPython filesystem: `config.json` at the root,
// `profiles/*.json`, and `logs/*.csv`. The `Storage` trait is synchronous and
// littlefs2 is blocking C, so each call mounts (`mount_and_then`), runs the op,
// and unmounts — cheap at our op rate (a log row every >= 10 s; profiles/config
// rare) and free of any long-lived-handle lifetime juggling.
//
// DEVICE-VERIFICATION SURFACE: only `LfsFlash`'s three `blocking_*` calls touch
// hardware. They run the erase/program from RAM (embassy-rp) and, at runtime,
// are serialised against Core 1 by the flash handshake (write paths wrap
// `with_flash_paused`); reads are XIP-safe and skip it. Boot-time mount/format
// runs before the core split, so Core 1 is not yet alive to need pausing.

use core::cell::RefCell;

use embassy_rp::flash::{Blocking, Flash};
use embassy_rp::peripherals::FLASH;
use embassy_rp::Peri;
use littlefs2::fs::Filesystem;
use littlefs2::io::SeekFrom;
use littlefs2::path;
use littlefs2::path::{Path, PathBuf}; // the `path!` macro is exported at the crate root

/// Total QSPI flash on the Pico 2 W (RP2350A) — the bound embassy-rp's `Flash`
/// validates every access against.
const FLASH_TOTAL: usize = 4 * 1024 * 1024;
/// littlefs partition: the top 1536 KiB, above the 2560 KiB the linker may fill
/// (`memory.x`). Offsets are flash-relative (from 0x1000_0000), as embassy-rp's
/// `Flash` expects. `FS_BASE` is erase-sector aligned, so every littlefs offset
/// stays aligned once rebased.
const FS_BASE: u32 = 0x28_0000; // 2560 KiB
const FS_SIZE: usize = 0x18_0000; // 1536 KiB
/// RP2350 QSPI erase sector and program page.
const FLASH_ERASE: usize = 4096;
const FLASH_PAGE: usize = 256;

/// The littlefs block device: embassy-rp's blocking `Flash`, with every littlefs
/// offset rebased into the reserved partition.
pub struct LfsFlash {
    flash: Flash<'static, FLASH, Blocking, FLASH_TOTAL>,
}

impl littlefs2::driver::Storage for LfsFlash {
    type CACHE_SIZE = littlefs2::consts::U256;
    // Lookahead is counted in u64 words: 4 * 64 = 256 blocks scanned per pass,
    // comfortably covering the 384-block partition.
    type LOOKAHEAD_SIZE = littlefs2::consts::U4;
    const READ_SIZE: usize = FLASH_PAGE;
    const WRITE_SIZE: usize = FLASH_PAGE;
    const BLOCK_SIZE: usize = FLASH_ERASE;
    const BLOCK_COUNT: usize = FS_SIZE / FLASH_ERASE;
    // Migrate a block's data after this many erase cycles (-1 would disable
    // dynamic wear levelling). 500 is littlefs's common default.
    const BLOCK_CYCLES: isize = 500;

    fn read(&mut self, off: usize, buf: &mut [u8]) -> littlefs2::io::Result<usize> {
        self.flash
            .blocking_read(FS_BASE + off as u32, buf)
            .map_err(|_| littlefs2::io::Error::IO)?;
        Ok(buf.len())
    }

    fn write(&mut self, off: usize, data: &[u8]) -> littlefs2::io::Result<usize> {
        self.flash
            .blocking_write(FS_BASE + off as u32, data)
            .map_err(|_| littlefs2::io::Error::IO)?;
        Ok(data.len())
    }

    fn erase(&mut self, off: usize, len: usize) -> littlefs2::io::Result<usize> {
        let from = FS_BASE + off as u32;
        self.flash
            .blocking_erase(from, from + len as u32)
            .map_err(|_| littlefs2::io::Error::IO)?;
        Ok(len)
    }
}

/// Build a `profiles/<name>` / `logs/<name>` (or bare `config.json`) runtime
/// path; `None` if it overflows the path buffer.
fn fs_path(prefix: &str, name: &str) -> Option<PathBuf> {
    let mut s = heapless::String::<128>::new();
    s.push_str(prefix).ok()?;
    s.push_str(name).ok()?;
    PathBuf::try_from(s.as_str()).ok()
}

/// The `Directory` → littlefs path prefix.
fn dir_prefix(dir: Directory) -> &'static str {
    match dir {
        Directory::Profiles => "profiles/",
        Directory::Logs => "logs/",
    }
}

/// The `Directory` listing root.
fn dir_root(dir: Directory) -> &'static Path {
    match dir {
        Directory::Profiles => path!("profiles"),
        Directory::Logs => path!("logs"),
    }
}

/// A littlefs filename as `&str`, with the stored trailing NUL trimmed.
fn name_str(p: &Path) -> &str {
    let s = p.as_str_ref_with_trailing_nul();
    s.strip_suffix('\0').unwrap_or(s)
}

/// A chronologically-sortable key from a `{profile}_{YYYY-MM-DD}_{HH-MM-SS}.csv`
/// log filename -> `YYYYMMDDHHMMSS`, or 0 when the name has no timestamp.
/// littlefs keeps no mtime, so recovery's "most recent log" and the web file
/// list derive `modified` from the timestamp the logger already embeds.
fn filename_time_key(name: &str) -> u64 {
    let stem = name.strip_suffix(".csv").unwrap_or(name);
    let mut it = stem.rsplitn(3, '_');
    let time = it.next();
    let date = it.next();
    match (date, time) {
        (Some(d), Some(t)) => {
            let mut key = 0u64;
            for c in d.chars().chain(t.chars()) {
                if let Some(digit) = c.to_digit(10) {
                    key = key * 10 + digit as u64;
                }
            }
            key
        }
        _ => 0,
    }
}

/// The single littlefs mount, behind a `RefCell` because `mount_and_then` needs
/// `&mut` storage. Core 0 is single-threaded and these methods run to completion
/// without awaiting, so the borrow never overlaps.
pub struct FlashStorage {
    dev: RefCell<LfsFlash>,
}

impl FlashStorage {
    /// Run `write` between [`flash_handshake::request_pause`] and
    /// [`flash_handshake::release`] — the safety-critical wrapper around any
    /// flash program/erase (Core 1 de-energises the SSR and parks in RAM).
    fn with_flash_paused<R>(&self, write: impl FnOnce() -> R) -> R {
        flash_handshake::request_pause();
        let r = write();
        flash_handshake::release();
        r
    }

    /// Mount the filesystem, run `f`, unmount. Read-only ops call this directly;
    /// write ops wrap the call in [`with_flash_paused`]. Any littlefs error
    /// (mount, or `f`'s) collapses to [`StorageError`].
    fn with_fs<R>(
        &self,
        f: impl FnOnce(&Filesystem<'_, LfsFlash>) -> littlefs2::io::Result<R>,
    ) -> Result<R, StorageError> {
        let mut dev = self.dev.borrow_mut();
        Filesystem::mount_and_then(&mut *dev, f).map_err(|_| StorageError)
    }
}

impl kiln_app::server::Storage for FlashStorage {
    fn read_chunk(
        &self,
        dir: Directory,
        name: &str,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<usize, StorageError> {
        let path = fs_path(dir_prefix(dir), name).ok_or(StorageError)?;
        self.with_fs(|fs| {
            fs.open_file_and_then(&path, |file| {
                if offset > 0 {
                    file.seek(SeekFrom::Start(offset as u32))?;
                }
                file.read(buf)
            })
        })
    }

    fn size(&self, dir: Directory, name: &str) -> Option<u64> {
        let path = fs_path(dir_prefix(dir), name)?;
        self.with_fs(|fs| fs.metadata(&path).map(|m| m.len() as u64))
            .ok()
    }

    fn for_each(&self, dir: Directory, f: &mut dyn FnMut(&str, u64, u64)) {
        let _ = self.with_fs(|fs| {
            fs.read_dir_and_then(dir_root(dir), |rd| {
                for entry in rd {
                    let entry = entry?;
                    if !entry.file_type().is_file() {
                        continue; // skip "." and ".."
                    }
                    let name = name_str(entry.file_name());
                    f(name, entry.metadata().len() as u64, filename_time_key(name));
                }
                Ok(())
            })
        });
    }

    fn append(
        &self,
        dir: Directory,
        name: &str,
        bytes: &[u8],
        create: bool,
    ) -> Result<(), StorageError> {
        let path = fs_path(dir_prefix(dir), name).ok_or(StorageError)?;
        self.with_flash_paused(|| {
            self.with_fs(|fs| {
                fs.open_file_with_options_and_then(
                    |o| {
                        if create {
                            // New run: truncate and write the header.
                            o.write(true).create(true).truncate(true)
                        } else {
                            // Subsequent rows / a recovery resume: append.
                            o.write(true).create(true).append(true)
                        }
                    },
                    &path,
                    |file| {
                        file.write(bytes)?;
                        file.sync()
                    },
                )?;
                Ok(())
            })
        })
    }

    fn remove(&self, dir: Directory, name: &str) -> Result<(), StorageError> {
        let path = fs_path(dir_prefix(dir), name).ok_or(StorageError)?;
        self.with_flash_paused(|| self.with_fs(|fs| fs.remove(&path)))
    }

    fn remove_all(&self, dir: Directory) -> Result<(), StorageError> {
        self.with_flash_paused(|| {
            self.with_fs(|fs| {
                // Collect full paths first — removing during iteration is unsafe.
                let mut paths: heapless::Vec<PathBuf, 64> = heapless::Vec::new();
                fs.read_dir_and_then(dir_root(dir), |rd| {
                    for entry in rd {
                        let entry = entry?;
                        if entry.file_type().is_file() {
                            let _ = paths.push(PathBuf::from(entry.path()));
                        }
                    }
                    Ok(())
                })?;
                for p in &paths {
                    fs.remove(p)?;
                }
                Ok(())
            })
        })
    }

    fn upload_begin(&self) -> Result<(), StorageError> {
        self.with_flash_paused(|| {
            self.with_fs(|fs| {
                fs.open_file_with_options_and_then(
                    |o| o.write(true).create(true).truncate(true),
                    path!("upload.tmp"),
                    |_file| Ok(()),
                )
            })
        })
    }

    fn upload_write(&self, bytes: &[u8]) -> Result<(), StorageError> {
        self.with_flash_paused(|| {
            self.with_fs(|fs| {
                fs.open_file_with_options_and_then(
                    |o| o.write(true).append(true),
                    path!("upload.tmp"),
                    |file| {
                        file.write(bytes)?;
                        file.sync()
                    },
                )?;
                Ok(())
            })
        })
    }

    fn upload_commit(&self, dir: Directory, name: &str) -> Result<(), StorageError> {
        let dest = fs_path(dir_prefix(dir), name).ok_or(StorageError)?;
        self.with_flash_paused(|| self.with_fs(|fs| fs.rename(path!("upload.tmp"), &dest)))
    }

    fn upload_abort(&self) {
        let _ = self.with_flash_paused(|| self.with_fs(|fs| fs.remove(path!("upload.tmp"))));
    }

    fn static_asset(&self, name: &str) -> Option<&'static [u8]> {
        // The web UI is compiled into flash (the reference cached it in RAM).
        match name {
            "index.html" => Some(include_bytes!("../../../static/index.html")),
            "tuning.html" => Some(include_bytes!("../../../static/tuning.html")),
            _ => None,
        }
    }

    fn read_config(&self, buf: &mut [u8]) -> Result<usize, StorageError> {
        // Absent/unreadable config → 0 bytes, so `load_config` falls back to
        // KilnConfig::default() (the graceful-default boot the reference lacks).
        Ok(self
            .with_fs(|fs| fs.open_file_and_then(path!("config.json"), |file| file.read(buf)))
            .unwrap_or(0))
    }

    fn write_config(&self, bytes: &[u8]) -> Result<(), StorageError> {
        // Temp file + atomic rename, so a power loss mid-write can't truncate the
        // live config (matches the reference's write convention).
        self.with_flash_paused(|| {
            self.with_fs(|fs| {
                fs.open_file_with_options_and_then(
                    |o| o.write(true).create(true).truncate(true),
                    path!("config.tmp"),
                    |file| {
                        file.write(bytes)?;
                        file.sync()
                    },
                )?;
                fs.rename(path!("config.tmp"), path!("config.json"))
            })
        })
    }
}

// === LCD ====================================================================

use crate::lcd::Lcd1602;
use embassy_rp::i2c::I2c;
use embassy_rp::peripherals::I2C0;

/// The concrete RP2350 LCD bus: blocking I²C0.
type LcdI2c = I2c<'static, I2C0, embassy_rp::i2c::Blocking>;

/// Throttle between LCD renders — `LCDManager.run`'s 5 s update cadence (keeps
/// I²C traffic down to limit wire interference).
const LCD_RENDER_INTERVAL: Duration = Duration::from_secs(5);
/// Periodic hardware re-init to recover from wire interference (`reset_interval_sec`).
const LCD_RESET_INTERVAL: Duration = Duration::from_secs(300);
/// Disable the LCD after this many consecutive write failures
/// (`max_consecutive_errors`) — the web server / WiFi keep running.
const LCD_MAX_ERRORS: u8 = 3;

/// The character LCD status line (`server/lcd_manager.py`). The kiln-app
/// [`lcd_task`] calls [`show`] on every status change; the manager's cadence,
/// periodic reset, and error-backoff logic live here since `show` is the only
/// hook the [`Display`] trait exposes.
///
/// [`lcd_task`]: kiln_app::server::lcd_task
/// [`show`]: Display::show
pub struct LcdDisplay {
    lcd: Lcd1602<LcdI2c>,
    enabled: bool,
    rendered: bool,
    last_render: Instant,
    last_reset: Instant,
    errors: u8,
}

impl LcdDisplay {
    /// Format and write the two-line summary. Row 1 = current temp + state; row 2
    /// = target temp + SSR duty (or just SSR duty when idle) — `LCDManager.run`'s
    /// exact layout.
    fn render(&mut self, status: &Status) -> Result<(), embassy_rp::i2c::Error> {
        use core::fmt::Write;

        let mut row1 = heapless::String::<24>::new();
        let _ = write!(row1, "{:4.0}C {}", status.current_temp, state_label(status.state));
        self.lcd.print_row(0, &row1)?;

        let mut row2 = heapless::String::<24>::new();
        if status.target_temp > 0.0 {
            let _ = write!(
                row2,
                "Tgt:{:4.0}C {:3.0}%",
                status.target_temp, status.ssr_output
            );
        } else {
            let _ = write!(row2, "SSR: {:3.0}%", status.ssr_output);
        }
        self.lcd.print_row(1, &row2)?;
        Ok(())
    }
}

/// The web/CSV-canonical state label (`json.rs`), so the LCD matches the API.
fn state_label(state: KilnState) -> &'static str {
    match state {
        KilnState::Idle => "IDLE",
        KilnState::Running => "RUNNING",
        KilnState::Tuning => "TUNING",
        KilnState::Complete => "COMPLETE",
        KilnState::Error => "ERROR",
    }
}

impl Display for LcdDisplay {
    fn show(&mut self, status: &Status) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();

        // Periodic hardware reset to shrug off wire-interference corruption.
        if now.duration_since(self.last_reset) >= LCD_RESET_INTERVAL {
            self.last_reset = now;
            let _ = self.lcd.init(); // a failed reset just errors on the next render
        }

        // Throttle to the render cadence (but always render the first status).
        if self.rendered && now.duration_since(self.last_render) < LCD_RENDER_INTERVAL {
            return;
        }

        match self.render(status) {
            Ok(()) => {
                self.errors = 0;
                self.rendered = true;
                self.last_render = now;
            }
            Err(_) => {
                self.errors = self.errors.saturating_add(1);
                if self.errors == 2 {
                    let _ = self.lcd.init(); // emergency reset on repeated failure
                }
                if self.errors >= LCD_MAX_ERRORS {
                    // Give up — leave web/WiFi untouched (`LCDManager` disables).
                    self.enabled = false;
                }
            }
        }
    }
}

// === Core 0 setup ===========================================================

use static_cell::StaticCell;

use cyw43::{Control, JoinOptions, PowerManagementMode, ScanOptions};
use cyw43_pio::{PioSpi, RM2_CLOCK_DIVIDER};
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_net::{IpEndpoint, Ipv4Address, Ipv4Cidr, Stack, StaticConfigV4};
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::RoscRng;
use embassy_rp::dma::{self, Channel};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{self, Pio};
use sntpc::{NtpContext, NtpTimestampGenerator, NtpUdpSocket};

// PIO0 drives the cyw43 SPI; one DMA channel moves the transfers. Both their
// completion interrupts must be bound for the blocking-future drivers to wake.
bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>;
});

/// Total smoltcp socket slots: the web worker pool (TCP) + the NTP UDP socket +
/// the stack's internal DHCP and DNS sockets.
const NET_SOCKETS: usize = kiln_app::server::WEB_TASK_POOL_SIZE + 3;

/// Drives the cyw43 chip (SPI ioctls, event pump). cyw43 0.7: `Runner<'a, BUS>`
/// — two generics, `BUS = SpiBus<PWR, SPI>` (no third `Cyw43439` param; that is
/// the newer cyw43 ≥0.8).
#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, cyw43::SpiBus<Output<'static>, PioSpi<'static, PIO0, 0>>>,
) -> ! {
    runner.run().await
}

/// Drives the `embassy-net` stack (smoltcp poll loop, DHCP).
#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

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

/// The cyw43 GPIO the on-board status LED hangs off (Pico W / 2 W: the LED is on
/// the wireless chip, not an RP2350 pin), driven via `Control::gpio_set`.
const STATUS_LED_GPIO: u8 = 0;

/// Build a static `embassy-net` v4 config from `WIFI_STATIC_IP` / `WIFI_SUBNET` /
/// `WIFI_GATEWAY` / `WIFI_DNS`. `None` if any field is absent or unparseable, so
/// the caller falls back to DHCP — `wifi_manager.connect` applies a static IP only
/// when all four are present.
fn static_config(config: &KilnConfig) -> Option<StaticConfigV4> {
    let ip: Ipv4Address = config.wifi_static_ip.as_ref()?.as_str().parse().ok()?;
    let mask: Ipv4Address = config.wifi_subnet.as_ref()?.as_str().parse().ok()?;
    let gateway: Ipv4Address = config.wifi_gateway.as_ref()?.as_str().parse().ok()?;
    let dns: Ipv4Address = config.wifi_dns.as_ref()?.as_str().parse().ok()?;

    // Dotted subnet mask (255.255.255.0) → CIDR prefix length.
    let prefix = mask.octets().iter().map(|b| b.count_ones()).sum::<u32>() as u8;
    // `StaticConfigV4.dns_servers` is an embassy-net (heapless 0.9) Vec.
    let mut dns_servers: heapless_v09::Vec<Ipv4Address, 3> = heapless_v09::Vec::new();
    let _ = dns_servers.push(dns);
    Some(StaticConfigV4 {
        address: Ipv4Cidr::new(ip, prefix),
        gateway: Some(gateway),
        dns_servers,
    })
}

/// Scan for `ssid` and return the strongest matching AP's RSSI (dBm), or `None`
/// if it is not visible — `wifi_manager.connect`'s scan-for-best-AP. NOTE: cyw43
/// 0.7's `join` takes no BSSID (`JoinOptions` has no such field), so the strongest
/// BSSID is *identified, not pinned*; the firmware associates by SSID. The scan
/// therefore serves as a presence gate (and picks the strongest, as asked).
async fn scan_strongest(control: &mut Control<'static>, ssid: &str) -> Option<i16> {
    // Scan all APs and filter by SSID bytes in the loop, rather than setting
    // `ScanOptions.ssid` — that field is a cyw43-version `heapless::String`, which
    // differs from this crate's heapless; filtering here avoids the mismatch and
    // behaves identically.
    let mut best: Option<i16> = None;
    let mut scanner = control.scan(ScanOptions::default()).await;
    while let Some(bss) = scanner.next().await {
        let len = bss.ssid_len as usize;
        if len <= bss.ssid.len()
            && &bss.ssid[..len] == ssid.as_bytes()
            && best.is_none_or(|r| bss.rssi > r)
        {
            best = Some(bss.rssi);
        }
    }
    best
}

/// Bring up cyw43 → an `embassy-net` `Stack`, join WiFi, and configure IP. Loads
/// the firmware/CLM/nvram blobs, builds the PIO SPI, spawns the cyw43 + net runner
/// tasks, scans for the AP (strongest), then joins **in a retry loop** (wait 2 s →
/// re-join) until the first association succeeds — `wifi_manager.connect`'s "keep
/// trying" behaviour, which cyw43's link auto-reconnect (drops only) does not
/// cover. The on-board LED blinks while connecting and goes solid once up. Returns
/// the `Stack` plus the `Control` handle the [`wifi_monitor_task`] needs to re-join.
///
/// IP: a static config (`WIFI_STATIC_IP` + subnet/gateway/dns) is honoured when
/// all four are set, else DHCP. The blobs are vendored under `cyw43-firmware/`
/// (Infineon Permissive Binary License); `aligned_bytes!` resolves their paths
/// relative to this file.
pub async fn init_network(
    spawner: &embassy_executor::Spawner,
    p: Core0Periphs,
    config: &'static KilnConfig,
) -> (Stack<'static>, Control<'static>) {
    let fw = cyw43::aligned_bytes!("../cyw43-firmware/43439A0.bin");
    let clm = cyw43::aligned_bytes!("../cyw43-firmware/43439A0_clm.bin");
    let nvram = cyw43::aligned_bytes!("../cyw43-firmware/nvram_rp2040.bin");

    // PIO-driven SPI to the on-board CYW43439 (fixed Pico 2 W wiring). The RM2
    // module needs the slower `RM2_CLOCK_DIVIDER` (embassy #3960).
    let pwr = Output::new(p.wl_pwr, Level::Low);
    let cs = Output::new(p.wl_cs, Level::High);
    let mut pio = Pio::new(p.pio, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        RM2_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        p.wl_dio,
        p.wl_clk,
        Channel::new(p.dma, Irqs),
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw, nvram).await;
    spawner.spawn(cyw43_task(runner).unwrap());

    control.init(clm).await;
    control
        .set_power_management(PowerManagementMode::PowerSave)
        .await;

    // Static IP when WIFI_STATIC_IP+subnet+gateway+dns are all set, else DHCP.
    // Seed the TCP/UDP RNG from the ring oscillator.
    let mut rng = RoscRng;
    let seed = rng.next_u64();
    let net_config = match static_config(config) {
        Some(static_v4) => embassy_net::Config::ipv4_static(static_v4),
        None => embassy_net::Config::dhcpv4(Default::default()),
    };
    static RES: StaticCell<embassy_net::StackResources<NET_SOCKETS>> = StaticCell::new();
    let (stack, net_runner) = embassy_net::new(
        net_device,
        net_config,
        RES.init(embassy_net::StackResources::new()),
        seed,
    );
    spawner.spawn(net_task(net_runner).unwrap());

    let ssid = config.wifi_ssid.as_str();
    let password = config.wifi_password.as_str();

    // Scan for the AP first (bounded so a flaky scan can't block boot): confirms
    // it is visible and picks the strongest, blinking the LED while we look.
    let mut led = false;
    for _ in 0..5 {
        if scan_strongest(&mut control, ssid).await.is_some() {
            break;
        }
        led = !led;
        control.gpio_set(STATUS_LED_GPIO, led).await;
        embassy_time::Timer::after(Duration::from_secs(1)).await;
    }

    // Join, blinking, until the first association sticks.
    while control
        .join(ssid, JoinOptions::new(password.as_bytes()))
        .await
        .is_err()
    {
        led = !led;
        control.gpio_set(STATUS_LED_GPIO, led).await;
        embassy_time::Timer::after(Duration::from_secs(2)).await;
    }
    stack.wait_config_up().await;
    control.gpio_set(STATUS_LED_GPIO, true).await; // solid on once connected

    (stack, control)
}

/// WiFi reconnect monitor — the steady-state half of `wifi_manager.monitor`
/// (`wifi_manager.py:139-180`). Parks until the link drops, then re-joins with a
/// 2 s backoff until it sticks and DHCP reconfigures. cyw43 auto-reconnects a
/// *dropped* link, but a hard failure (wrong key / AP gone) needs this explicit
/// re-join — what the reference adds — so a kiln on a flaky AP stays reachable.
/// Drives the on-board status LED: solid on while connected, off + blink while
/// reconnecting (`wifi_manager`'s LED feedback).
#[embassy_executor::task]
pub async fn wifi_monitor_task(
    mut control: Control<'static>,
    stack: Stack<'static>,
    ssid: &'static str,
    password: &'static str,
) -> ! {
    loop {
        stack.wait_link_up().await;
        control.gpio_set(STATUS_LED_GPIO, true).await; // connected: solid on
        stack.wait_link_down().await;
        control.gpio_set(STATUS_LED_GPIO, false).await; // dropped: off

        let mut led = false;
        while control
            .join(ssid, JoinOptions::new(password.as_bytes()))
            .await
            .is_err()
        {
            led = !led;
            control.gpio_set(STATUS_LED_GPIO, led).await;
            embassy_time::Timer::after(Duration::from_secs(2)).await;
        }
        stack.wait_config_up().await;
    }
}

/// Mount littlefs and return the shared [`FlashStorage`]. Called from `main()`
/// BEFORE the core split so [`load_config`] can read `config.json` and hand both
/// cores their config. Since Core 1 is not yet alive, the format/mount writes
/// run without the flash handshake (which would deadlock waiting for a PARK that
/// never comes); Core 0 erases its own flash safely because embassy-rp runs the
/// erase/program from RAM.
pub fn init_storage(flash: Peri<'static, FLASH>) -> &'static FlashStorage {
    static STORAGE: StaticCell<FlashStorage> = StaticCell::new();
    let mut dev = LfsFlash {
        flash: Flash::new_blocking(flash),
    };
    // Probe the filesystem; format on first boot or after a corrupting power loss.
    if Filesystem::mount_and_then(&mut dev, |_fs| Ok(())).is_err() {
        let _ = Filesystem::format(&mut dev);
    }
    // Ensure the profiles/ and logs/ directories exist (idempotent across boots).
    let _ = Filesystem::mount_and_then(&mut dev, |fs| {
        let _ = fs.create_dir(path!("profiles"));
        let _ = fs.create_dir(path!("logs"));
        Ok(())
    });
    STORAGE.init(FlashStorage {
        dev: RefCell::new(dev),
    })
}

pub fn init_clock() -> &'static NtpClock {
    static CLOCK: StaticCell<NtpClock> = StaticCell::new();
    CLOCK.init(NtpClock)
}

/// Build the LCD on blocking I²C0 and run its power-on init. Returns `None` when
/// the device does not ACK (absent / mis-wired backpack), so the caller simply
/// does not spawn the LCD task and the kiln runs headless. The SDA/SCL pins are
/// fixed by the RP2350 pinmux (config carries their numbers for documentation);
/// only `LCD_I2C_FREQ` / `LCD_I2C_ADDR` are honoured at runtime.
pub fn init_display(p: LcdPeriphs, config: &KilnConfig) -> Option<&'static mut LcdDisplay> {
    let mut i2c_cfg = embassy_rp::i2c::Config::default();
    i2c_cfg.frequency = config.lcd_i2c_freq;
    let i2c = I2c::new_blocking(p.i2c, p.scl, p.sda, i2c_cfg);

    let mut lcd = Lcd1602::new(i2c, config.lcd_i2c_addr);
    if lcd.init().is_err() {
        return None; // no device / bus fault → run without the LCD
    }

    static DISPLAY: StaticCell<LcdDisplay> = StaticCell::new();
    let now = Instant::now();
    Some(DISPLAY.init(LcdDisplay {
        lcd,
        enabled: true,
        rendered: false,
        last_render: now,
        last_reset: now,
        errors: 0,
    }))
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
    let pn = kiln_app::server::read_file_with_retry(
        state.storage,
        Directory::Profiles,
        &fname,
        &mut pbuf,
    )
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

/// NTP epoch (1900-01-01) → Unix epoch (1970-01-01), in seconds. `sntpc` reports
/// `NtpResult.seconds` in the NTP era, so subtract this to get Unix time.
const NTP_UNIX_DELTA: u64 = 2_208_988_800;
/// Fixed local UDP port for the NTP client (smoltcp rejects binding port 0).
const NTP_LOCAL_PORT: u16 = 50_123;

/// `sntpc` timestamp source. Seeds from the wall clock once synced (sharpens the
/// round-trip/offset math); 0 before first sync, which only perturbs the offset
/// estimate, not the absolute time read back from the server.
#[derive(Copy, Clone, Default)]
struct NtpTimestamps {
    secs: u64,
    micros: u32,
}

impl NtpTimestampGenerator for NtpTimestamps {
    fn init(&mut self) {
        match NtpClock::unix_ms() {
            Some(ms) if ms > 0 => {
                self.secs = (ms / 1000) as u64;
                self.micros = ((ms % 1000) * 1000) as u32;
            }
            _ => {
                self.secs = 0;
                self.micros = 0;
            }
        }
    }
    fn timestamp_sec(&self) -> u64 {
        self.secs
    }
    fn timestamp_subsec_micros(&self) -> u32 {
        self.micros
    }
}

/// An `embassy-net` UDP socket as an `sntpc::NtpUdpSocket`. Both trait methods
/// take `&self`, matching embassy-net's `send_to`/`recv_from`.
struct NtpSocket<'a>(UdpSocket<'a>);

impl NtpUdpSocket for NtpSocket<'_> {
    async fn send_to(&self, buf: &[u8], addr: core::net::SocketAddr) -> sntpc::Result<usize> {
        // proto-ipv4 only: `IpEndpoint: From<SocketAddr>` needs both protocols,
        // so convert from the V4 case (which is the ipv4-gated impl). The server
        // address is always V4 here (resolved A record / V4 fallback).
        let endpoint = match addr {
            core::net::SocketAddr::V4(v4) => IpEndpoint::from(v4),
            core::net::SocketAddr::V6(_) => return Err(sntpc::Error::Network),
        };
        self.0
            .send_to(buf, endpoint)
            .await
            .map_err(|_| sntpc::Error::Network)?;
        Ok(buf.len())
    }

    async fn recv_from(&self, buf: &mut [u8]) -> sntpc::Result<(usize, core::net::SocketAddr)> {
        let (n, meta) = self
            .0
            .recv_from(buf)
            .await
            .map_err(|_| sntpc::Error::Network)?;
        let ip: core::net::IpAddr = meta.endpoint.addr.into();
        Ok((n, core::net::SocketAddr::new(ip, meta.endpoint.port)))
    }
}

/// One NTP exchange: resolve `pool.ntp.org` (falling back to Cloudflare's anycast
/// NTP if DNS fails), query it over UDP with a 10 s deadline, and return Unix
/// seconds. `None` on any failure. Mirrors `wifi_manager.sync_time_ntp`.
async fn ntp_query(stack: Stack<'static>) -> Option<u64> {
    use core::net::{IpAddr, Ipv4Addr, SocketAddr};

    let server: IpAddr = match stack
        .dns_query("pool.ntp.org", embassy_net::dns::DnsQueryType::A)
        .await
    {
        Ok(addrs) if !addrs.is_empty() => addrs[0].into(),
        // Cloudflare anycast NTP — a stable fixed IP for networks without usable
        // DHCP DNS.
        _ => IpAddr::V4(Ipv4Addr::new(162, 159, 200, 123)),
    };

    let mut rx_meta = [PacketMetadata::EMPTY; 8];
    let mut rx_buf = [0u8; 256];
    let mut tx_meta = [PacketMetadata::EMPTY; 8];
    let mut tx_buf = [0u8; 256];
    let mut socket = UdpSocket::new(stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
    socket.bind(NTP_LOCAL_PORT).ok()?;

    let ntp = NtpSocket(socket);
    let ctx = NtpContext::new(NtpTimestamps::default());
    let server_addr = SocketAddr::new(server, 123);
    // Bound the recv: a silent server would otherwise wait forever.
    let result =
        embassy_time::with_timeout(Duration::from_secs(10), sntpc::get_time(server_addr, &ntp, ctx))
            .await
            .ok()?
            .ok()?;
    Some((result.seconds as u64).saturating_sub(NTP_UNIX_DELTA))
}

/// NTP task: sync the wall clock via `sntpc`, then re-sync hourly (retrying
/// sooner until the first sync lands). On success it calls [`NtpClock::set_unix_ms`],
/// which unblocks CSV/recovery timestamps.
#[embassy_executor::task]
pub async fn ntp_task(_clock: &'static NtpClock, stack: Stack<'static>) -> ! {
    loop {
        let synced = match ntp_query(stack).await {
            Some(unix) => {
                NtpClock::set_unix_ms(unix as i64 * 1000);
                true
            }
            None => false,
        };
        let wait_s = if synced { 3600 } else { 60 };
        embassy_time::Timer::after(Duration::from_secs(wait_s)).await;
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
