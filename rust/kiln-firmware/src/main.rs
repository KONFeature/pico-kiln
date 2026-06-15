//! RP2350 (Pico 2 W) binary shim — the only crate that names `embassy-rp` /
//! `cyw43`, and the only place the two cores and the world are wired together.
//! Ports the boot/dispatch role of `main.py` (which started the control thread on
//! one core and the asyncio web stack on the other).
//!
//! VERIFICATION STATUS. This crate compiles only for `thumbv8m.main-none-eabihf`
//! with a `memory.x` + a probe runner + the cyw43 firmware blobs, so it is
//! excluded from the host workspace and is **not** built/tested in CI-on-host.
//! It is the integration layer: the safety- and behaviour-critical logic it
//! drives ([`kiln_control::Controller`], the `kiln_app` web/CSV/recovery modules,
//! the `kiln_hal` drivers) is host-tested in those crates. What lives here is the
//! wiring + the RP2350 specifics (peripheral split, the Core 1/Core 0 dispatch,
//! the flash handshake, the cyw43/littlefs/sntpc/LCD bindings).
//!
//! THE SPLIT (the user's hard requirement, mirroring the reference's dual-core
//! design): **Core 1 only controls the kiln** — it owns the sensor, the PID, the
//! SSR, and the watchdog, and runs nothing else. **Core 0 does everything else**
//! — the web server, CSV logging, recovery, WiFi, NTP, and the LCD. The cores
//! talk only through two `CriticalSectionRawMutex` channels (commands down,
//! status up) and the [`flash_handshake`] flag.

#![no_std]
#![no_main]

use embassy_executor::Executor;
use embassy_futures::select::select;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::multicore::{spawn_core1, Stack};
use embassy_rp::Peri;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{with_timeout, Duration, Instant, Timer};
use static_cell::StaticCell;

use kiln_app::config::KilnConfig;
use kiln_app::server::{AppState, CommandChannel, RebootSignal, StatusWatch};
use kiln_control::Controller;

mod dhcp;
mod flash_handshake;
mod lcd;
mod platform;

use platform::{FlashStorage, NtpClock};

/// Core 1 ↔ Core 0 channels and shared signals. `CriticalSectionRawMutex` is
/// mandatory: these are touched from both cores.
static COMMANDS: StaticCell<CommandChannel> = StaticCell::new();
static STATUS: StaticCell<StatusWatch> = StaticCell::new();
static REBOOT: StaticCell<RebootSignal> = StaticCell::new();

/// Core 1 → Core 0 "hardware ready" handshake (`main.py`'s `ReadyFlag`): Core 1
/// signals once its sensor/SSR/watchdog are built; Core 0 refuses to bring up the
/// app until then, and resets if it never comes — "unsafe to operate".
static READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Core 1's dedicated stack and executor (kept off Core 0's executor entirely).
static CORE1_STACK: StaticCell<Stack<8192>> = StaticCell::new();
static EXECUTOR1: StaticCell<Executor> = StaticCell::new();
static EXECUTOR0: StaticCell<Executor> = StaticCell::new();

#[cortex_m_rt::entry]
fn main() -> ! {
    let p = embassy_rp::init(Default::default());

    let commands = COMMANDS.init(CommandChannel::new());
    let status = STATUS.init(StatusWatch::new());
    let reboot = REBOOT.init(RebootSignal::new());

    // Config is global, read once at boot from flash — the runtime replacement
    // for the `config.py` the MicroPython build imported. Mount storage and parse
    // `config.json` before the split so both cores share the same `KilnConfig`.
    let storage = platform::init_storage(p.FLASH);
    let config = platform::load_config(storage);

    // Install the global logger BEFORE the core split so both cores can log from
    // the outset. The wall clock is bound later (in `core0_main`, once it exists);
    // until then lines carry an uptime timestamp.
    kiln_app::logging::init(config.log_level, config.log_to_flash);

    // First lines on the wire: what config actually took effect. These queue in the
    // log channel until Core 0 starts the drain task, then surface in /api/logs.
    log::info!(target: "boot", "pico-kiln starting");
    log::debug!(
        target: "boot",
        "config: log={} wifi_cfg={} watchdog={} ssr_pins={:?} tc_type={:?} lcd={}",
        config.log_level.as_str(),
        config.wifi_is_configured(),
        config.enable_watchdog,
        config.ssr_pin.as_slice(),
        config.thermocouple_type,
        config.lcd_enabled,
    );

    // Core 1: the control loop, and nothing else. It receives commands and
    // publishes status; it never touches the network or flash.
    let core1_periphs = Core1Periphs {
        spi: p.SPI0,
        sck: p.PIN_18,
        mosi: p.PIN_19,
        miso: p.PIN_16,
        cs: p.PIN_28,
        // Reserved SSR candidate pins, built as de-energised outputs;
        // `build_kiln_io` keeps the `SSR_PIN`-selected subset.
        ssr_pool: [
            (15, Output::new(p.PIN_15, Level::Low)),
            (14, Output::new(p.PIN_14, Level::Low)),
            (13, Output::new(p.PIN_13, Level::Low)),
            (12, Output::new(p.PIN_12, Level::Low)),
            (11, Output::new(p.PIN_11, Level::Low)),
            (10, Output::new(p.PIN_10, Level::Low)),
            (9, Output::new(p.PIN_9, Level::Low)),
            (8, Output::new(p.PIN_8, Level::Low)),
            (7, Output::new(p.PIN_7, Level::Low)),
            (6, Output::new(p.PIN_6, Level::Low)),
        ],
        watchdog: p.WATCHDOG,
    };
    let cmd_rx = commands.receiver();
    let status_tx = status.sender();
    let stack = CORE1_STACK.init(Stack::new());
    spawn_core1(p.CORE1, stack, move || {
        let executor1 = EXECUTOR1.init(Executor::new());
        executor1.run(|spawner| {
            // embassy-executor 0.10: a `#[task]` fn returns `Result<SpawnToken, _>`
            // (pool exhaustion is fallible) and `Spawner::spawn` returns `()`, so
            // the `unwrap` moves inside.
            spawner.spawn(control_task(core1_periphs, config, cmd_rx, status_tx).unwrap())
        });
    });

    // Core 0: web + logging + recovery + WiFi + NTP + LCD.
    // Core 0's peripherals — disjoint from Core 1's, taken individually (see
    // `Core0Periphs`). The fixed Pico 2 W cyw43 wiring (the flash went to
    // `init_storage` above, before the split).
    let core0_periphs = Core0Periphs {
        wl_pwr: p.PIN_23,
        wl_dio: p.PIN_24,
        wl_cs: p.PIN_25,
        wl_clk: p.PIN_29,
        pio: p.PIO0,
        dma: p.DMA_CH0,
    };

    // Optional LCD status line on I2C0 (SDA=PIN_20, SCL=PIN_21 — the config
    // defaults). Always taken from the peripherals; only used if `LCD_ENABLED`.
    let lcd_periphs = LcdPeriphs {
        i2c: p.I2C0,
        sda: p.PIN_20,
        scl: p.PIN_21,
    };

    // Core 0's non-cyw43 peripherals: the optional LCD plus the USB controller for
    // the always-on CDC-NCM provisioning interface. Bundled into `AuxPeriphs`
    // because `init_network` consumes `Core0Periphs` by value, so these travel
    // separately. `p.USB` is disjoint from the cyw43/LCD pins.
    let aux = AuxPeriphs {
        lcd: lcd_periphs,
        usb: p.USB,
    };

    let executor0 = EXECUTOR0.init(Executor::new());
    executor0.run(|spawner| {
        spawner.spawn(
            core0_main(
                core0_periphs,
                aux,
                storage,
                config,
                commands.sender(),
                status,
                reboot,
            )
            .unwrap(),
        )
    });
}

/// Peripherals handed to Core 1 (the kiln I/O only). embassy-rp 0.10 hands out
/// each peripheral as a `Peri<'static, T>` handle rather than the bare singleton.
struct Core1Periphs {
    spi: Peri<'static, embassy_rp::peripherals::SPI0>,
    sck: Peri<'static, embassy_rp::peripherals::PIN_18>,
    mosi: Peri<'static, embassy_rp::peripherals::PIN_19>,
    miso: Peri<'static, embassy_rp::peripherals::PIN_16>,
    cs: Peri<'static, embassy_rp::peripherals::PIN_28>,
    /// Reserved candidate SSR outputs (already driven low). `build_kiln_io`
    /// keeps the ones `SSR_PIN` selects and drops the rest. GPIO
    /// 15/14/13/12/11/10/9/8/7/6 (PIN_15 = the reference default).
    ssr_pool: [(u8, Output<'static>); platform::MAX_SSR],
    watchdog: Peri<'static, embassy_rp::peripherals::WATCHDOG>,
}

/// The Core 1 control task — the same per-tick orchestration as
/// `kiln_control::run`, but with the [`flash_handshake`] check woven into the
/// sub-tick loop so a Core 0 CSV write cannot strand the SSR on across the XIP
/// stall. Built from the public [`Controller`] methods exactly as `run.rs`
/// documents a flash-aware firmware should.
#[embassy_executor::task]
async fn control_task(
    p: Core1Periphs,
    config: &'static KilnConfig,
    commands: embassy_sync::channel::Receiver<
        'static,
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        kiln_core::protocol::Command,
        { kiln_app::server::COMMAND_DEPTH },
    >,
    status: embassy_sync::watch::Sender<
        'static,
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        kiln_core::protocol::Status,
        { kiln_app::server::STATUS_CONSUMERS },
    >,
) -> ! {
    // Power/hardware settle before touching the thermocouple (`boot.py:26`) —
    // "especially important when the thermocouple is connected at boot".
    log::debug!(target: "ctrl", "core1: 500ms hardware settle");
    Timer::after(Duration::from_millis(500)).await;

    // build_kiln_io logs the sensor / SSR / watchdog detail it sets up.
    let (sensor, ssr, watchdog) = platform::build_kiln_io(p, config);
    let mut controller = Controller::new(
        sensor,
        ssr,
        watchdog,
        platform::control_params_from(config),
        Instant::now().as_millis(),
    );

    // Core 1 hardware is constructed — release Core 0's boot gate (ReadyFlag).
    log::info!(target: "ctrl", "core1: hardware ready, control loop starting");
    READY.signal(());

    let (sub_ticks, sub_ms) = controller.timing();
    loop {
        let cmd = commands.try_receive().ok();
        let now_ms = Instant::now().as_millis();
        let outcome = controller.iterate(cmd, now_ms, NtpClock::unix_seconds_i64());

        if let Some(snapshot) = outcome.publish {
            status.send(snapshot);
        }

        if outcome.faulted {
            // Throttle (the reference's sleep(1)) — but keep answering the flash
            // handshake while faulted. Core 0 may need to write *now* (e.g. the
            // terminal ERROR log row this fault just published), and the faulted
            // branch used to `continue` straight past the only place the handshake
            // is serviced, so such a write would spin Core 0 forever. Wake early on
            // a pause request and park (RAM-resident, raw watchdog feed); the SSR is
            // already de-energised by the faulted iterate. Between parks the
            // watchdog is still not fed, so a sustained sensor fault still resets
            // the chip as intended.
            let _ = select(
                Timer::after(Duration::from_secs(1)),
                flash_handshake::PAUSE_WAKE.wait(),
            )
            .await;
            if flash_handshake::pause_requested() {
                let _ = controller.force_ssr_off();
                flash_handshake::park_until_idle(platform::raw_watchdog_feed);
            }
            continue;
        }

        for _ in 0..sub_ticks {
            // Wait out the sub-tick, but wake the instant Core 0 asks to write
            // flash (PAUSE_WAKE) instead of only noticing at the next sub-tick
            // boundary — collapses Core 0's busy-spin in `request_pause` from up
            // to ~sub_ms down to the flash-op time itself.
            let _ = select(
                Timer::after(Duration::from_millis(sub_ms)),
                flash_handshake::PAUSE_WAKE.wait(),
            )
            .await;

            // Flash handshake: if Core 0 is about to program flash, de-energise
            // the SSR now (flash still live) and park in RAM until it is done.
            if flash_handshake::pause_requested() {
                let _ = controller.force_ssr_off();
                flash_handshake::park_until_idle(platform::raw_watchdog_feed);
            }

            let _ = controller.ssr_subtick(Instant::now().as_millis());
        }
    }
}

/// Peripherals handed to Core 0 (network + flash). A single `embassy-rp`
/// `Peripherals` cannot be split by storing the whole struct on one side: once
/// Core 1's fields are moved out, the remainder can no longer be moved or
/// borrowed wholesale. So Core 0 takes the *specific* peripherals it owns, which
/// are disjoint from Core 1's. These are the fixed Pico 2 W cyw43 wiring; the
/// flash went to `init_storage` (before the split) and the LCD I2C is omitted
/// until the LCD driver is ported (U9).
///
/// Consumed by value by `platform::init_network` (the pins are moved into the
/// cyw43 `Output`s and the PIO SPI).
struct Core0Periphs {
    // cyw43 WiFi: power-on, the PIO-driven SPI (data/cs/clk), the PIO block, and
    // a DMA channel — all hardwired to the CYW43 on the Pico 2 W.
    wl_pwr: Peri<'static, embassy_rp::peripherals::PIN_23>,
    wl_dio: Peri<'static, embassy_rp::peripherals::PIN_24>,
    wl_cs: Peri<'static, embassy_rp::peripherals::PIN_25>,
    wl_clk: Peri<'static, embassy_rp::peripherals::PIN_29>,
    pio: Peri<'static, embassy_rp::peripherals::PIO0>,
    dma: Peri<'static, embassy_rp::peripherals::DMA_CH0>,
}

/// The optional LCD status-line peripherals (I2C0 + its fixed Pico 2 W pins),
/// kept out of [`Core0Periphs`] because `init_network` consumes that by value.
/// Handed to `core0_main` and used only when `LCD_ENABLED`; otherwise dropped.
struct LcdPeriphs {
    i2c: Peri<'static, embassy_rp::peripherals::I2C0>,
    sda: Peri<'static, embassy_rp::peripherals::PIN_20>,
    scl: Peri<'static, embassy_rp::peripherals::PIN_21>,
}

/// Core 0 peripherals beyond the cyw43 radio: the optional LCD and the USB
/// controller (always-on CDC-NCM provisioning). Bundled so `core0_main` stays
/// within clippy's argument limit, and kept out of [`Core0Periphs`] because
/// `init_network`/`init_softap` consume that by value before these are used.
struct AuxPeriphs {
    lcd: LcdPeriphs,
    usb: Peri<'static, embassy_rp::peripherals::USB>,
}

/// Core 0 setup task: bring up cyw43 → an `embassy-net` `Stack`, mount flash,
/// wait for Core 1's ready handshake, run crash recovery, build the picoserve app
/// with shared state, and spawn the worker pool plus the CSV-logging,
/// WiFi-monitor, NTP, and reboot tasks. Mirrors `main.py`'s asyncio startup, minus
/// the control thread (now Core 1) and the LCD task (deferred — see U9).
#[embassy_executor::task]
async fn core0_main(
    p: Core0Periphs,
    aux: AuxPeriphs,
    storage: &'static FlashStorage,
    config: &'static KilnConfig,
    commands: kiln_app::server::CommandSender,
    status: &'static StatusWatch,
    reboot: &'static RebootSignal,
) {
    // SAFETY: `core0_main` runs as a task on Core 0's embassy executor, which is
    // exactly the precondition embassy-executor 0.10 requires for this call.
    let spawner = unsafe { embassy_executor::Spawner::for_current_executor() }.await;
    let AuxPeriphs { lcd, usb } = aux;

    // --- Serve-the-logs-no-matter-what bring-up -----------------------------
    // The boot order's overriding goal: the diagnosable surface (web + logs +
    // files) must come up before ANYTHING that can block or depend on Core 1, so
    // however boot goes wrong, the operator can still reach the logs. None of this
    // needs WiFi or Core 1.
    let clock: &'static NtpClock = platform::init_clock();
    let state = AppState {
        commands,
        status,
        clock,
        storage,
        reboot,
        config,
    };

    // Logging first: bind the wall clock and start the Core 0 drain + unified flash
    // flusher. The global logger was installed in `main` before the split, so lines
    // emitted during bring-up are already queued; the drain fans them to the RAM
    // ring (`/api/logs`) and — when LOG_TO_FLASH — the flash channel.
    kiln_app::logging::set_clock(clock);
    spawner.spawn(kiln_app::logging::log_drain_task().unwrap());
    spawner.spawn(kiln_app::logging::flash_flush_task(storage, clock).unwrap());

    // `make_app` bakes the shared `AppState` into the router; picoserve 0.18 then
    // serves the stateless `Router<P>` aliased as `AppRouter`, stored in a
    // StaticCell so the `#[task]` web_task stays non-generic. (`AppState` is `Copy`,
    // so `state` stays usable for recovery below.) One router, shared across stacks.
    static APP: StaticCell<kiln_app::server::AppRouter> = StaticCell::new();
    let app: &'static _ = APP.init(kiln_app::server::make_app(state));
    let web_cfg = platform::web_config();
    let port = config.web_server_port;

    // --- USB-NCM: always on (radio-independent), the wired escape hatch -------
    // Comes up whenever the cable is enumerated, serving the same router as WiFi,
    // so config + files + logs are reachable over USB regardless of WiFi or Core 1
    // state. Workers spawn BEFORE the radio branch (and before the READY wait) so
    // the wired UI is never starved by a slow/failed WiFi join or a dead Core 1.
    let usb_stack = platform::init_usb_ncm(&spawner, usb);
    for i in 0..kiln_app::server::SECONDARY_WEB_WORKERS {
        let id = kiln_app::server::WEB_TASK_POOL_SIZE + i;
        spawner.spawn(kiln_app::server::web_task(id, usb_stack, app, web_cfg, port).unwrap());
    }
    log::info!(target: "boot", "usb-ncm up at 192.168.7.1 ({} workers); web/logs reachable over USB", kiln_app::server::SECONDARY_WEB_WORKERS);

    // Gate on Core 1 hardware-ready (`main.py:235-242`, ReadyFlag) — bounded and
    // NON-FATAL. A ready Core 1 unlocks firing; if it never signals, the board
    // stays in DEGRADED MODE (web/logs/files up, start-firing endpoints reject)
    // instead of the old reset loop, which hid every Core-1 fault behind an
    // undiagnosable reboot. Safe either way: the SSR is de-energised (relays init
    // low and the control loop never ran), so nothing can be left energised.
    let ready = with_timeout(Duration::from_secs(20), READY.wait())
        .await
        .is_ok();
    if ready {
        log::info!(target: "boot", "core1 ready — firing enabled");
        kiln_app::server::set_controller_ready();
    } else {
        log::warn!(target: "boot", "core1 not ready after 20s — degraded mode: serving logs, firing disabled");
    }

    // Crash recovery + CSV logging — only with a live controller. Both need Core 1's
    // temperature; running recovery against a dead controller's idle 0 °C would
    // consume the active-run pointer and forfeit the resume. Skipping it in degraded
    // mode lets a transient Core-1 hang + watchdog reset recover the run on the next
    // healthy boot. Recovery returns the interrupted run's log file so the CSV logger
    // continues it (append + RECOVERY row) rather than starting fresh.
    let recovery = if ready {
        platform::attempt_recovery(&state).await
    } else {
        None
    };
    spawner.spawn(
        kiln_app::server::csv_logger_task(status, storage, clock, config, recovery).unwrap(),
    );

    // --- Radio: STA when configured, else the provisioning SoftAP ------------
    // cyw43 cannot do STA and AP at once. A configured board makes a bounded STA
    // connect attempt (`init_network`) then hands off to `wifi_monitor_task`, which
    // keeps retrying the join in the background — boot never blocks on WiFi, and
    // USB-NCM is the recovery path if the saved creds are wrong. An unconfigured
    // board serves the open setup AP instead.
    if config.wifi_is_configured() {
        log::info!(target: "boot", "wifi: STA mode, joining \"{}\"", config.wifi_ssid.as_str());
        let (sta_stack, control) = platform::init_network(&spawner, p, config).await;
        log::info!(target: "boot", "wifi: boot join {}", if sta_stack.is_link_up() { "up" } else { "pending (monitor retrying)" });
        // Primary interface: the full worker pool.
        for id in 0..kiln_app::server::WEB_TASK_POOL_SIZE {
            spawner.spawn(kiln_app::server::web_task(id, sta_stack, app, web_cfg, port).unwrap());
        }
        // WiFi reconnect monitor (`wifi_manager.monitor`) + NTP — STA only: there
        // is no upstream network in AP mode. The `Control` handle is moved into
        // the monitor (nothing else uses it after this).
        spawner.spawn(
            platform::wifi_monitor_task(
                control,
                sta_stack,
                config.wifi_ssid.as_str(),
                config.wifi_password.as_str(),
            )
            .unwrap(),
        );
        spawner.spawn(platform::ntp_task(clock, sta_stack).unwrap());
    } else {
        // Unconfigured: open SoftAP for first-time provisioning. No NTP (so
        // NTP-gated runs stay gated — fine, you're here to set WiFi, not fire).
        log::info!(target: "boot", "wifi: unconfigured — starting provisioning SoftAP");
        let ap_stack = platform::init_softap(&spawner, p).await;
        for id in 0..kiln_app::server::SECONDARY_WEB_WORKERS {
            spawner.spawn(kiln_app::server::web_task(id, ap_stack, app, web_cfg, port).unwrap());
        }
    }

    // LCD status line (`server/lcd_manager.py`), optional. Built only when
    // `LCD_ENABLED`; a missing/mis-wired backpack disables it without affecting
    // the web server or WiFi. The kiln-app `lcd_task` renders each status change
    // through the firmware `Display`. Works in either radio mode.
    if config.lcd_enabled {
        match platform::init_display(lcd, config) {
            Some(display) => {
                log::debug!(target: "boot", "lcd: initialised");
                spawner.spawn(kiln_app::server::lcd_task(status, display).unwrap());
            }
            None => log::warn!(target: "boot", "lcd: enabled but no device ACK — running headless"),
        }
    }
    spawner.spawn(platform::reboot_task(reboot).unwrap());
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // A panic on either core must de-energise the kiln. Force the SSR off via the
    // raw GPIO register (RAM-safe, driver-independent) then halt; the watchdog,
    // no longer fed, resets the chip into a clean state.
    platform::raw_ssr_off();
    loop {
        cortex_m::asm::wfe();
    }
}
