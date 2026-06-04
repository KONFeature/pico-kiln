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

    let executor0 = EXECUTOR0.init(Executor::new());
    executor0.run(|spawner| {
        spawner.spawn(
            core0_main(
                core0_periphs,
                lcd_periphs,
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
    Timer::after(Duration::from_millis(500)).await;

    let (sensor, ssr, watchdog) = platform::build_kiln_io(p, config);
    let mut controller = Controller::new(
        sensor,
        ssr,
        watchdog,
        platform::control_params_from(config),
        Instant::now().as_millis(),
    );

    // Core 1 hardware is constructed — release Core 0's boot gate (ReadyFlag).
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
            Timer::after(Duration::from_secs(1)).await;
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

/// Core 0 setup task: bring up cyw43 → an `embassy-net` `Stack`, mount flash,
/// wait for Core 1's ready handshake, run crash recovery, build the picoserve app
/// with shared state, and spawn the worker pool plus the CSV-logging,
/// WiFi-monitor, NTP, and reboot tasks. Mirrors `main.py`'s asyncio startup, minus
/// the control thread (now Core 1) and the LCD task (deferred — see U9).
#[embassy_executor::task]
async fn core0_main(
    p: Core0Periphs,
    lcd: LcdPeriphs,
    storage: &'static FlashStorage,
    config: &'static KilnConfig,
    commands: kiln_app::server::CommandSender,
    status: &'static StatusWatch,
    reboot: &'static RebootSignal,
) {
    // SAFETY: `core0_main` runs as a task on Core 0's embassy executor, which is
    // exactly the precondition embassy-executor 0.10 requires for this call.
    let spawner = unsafe { embassy_executor::Spawner::for_current_executor() }.await;

    // --- Network-independent bring-up first ---------------------------------
    // Build the wall clock and shared AppState. None of this needs WiFi, so crash
    // recovery + control + logging come up even if the AP is slow or absent (a
    // power blip mid-firing must resume the firing regardless of connectivity;
    // Core 1 already runs independently).
    let clock: &'static NtpClock = platform::init_clock();
    let state = AppState {
        commands,
        status,
        clock,
        storage,
        reboot,
        config,
    };

    // Gate on Core 1 hardware-ready (`main.py:235-242`, ReadyFlag). If Core 1
    // never signals within 20 s the kiln is unsafe to operate — reset and retry.
    // WiFi-independent, and kept ahead of the (potentially slow) network bring-up.
    if with_timeout(Duration::from_secs(20), READY.wait())
        .await
        .is_err()
    {
        cortex_m::peripheral::SCB::sys_reset();
    }

    // Crash recovery — BEFORE the network. It needs only flash + Core 1's live
    // temperature (published from the first tick now that QuietMode is gone), never
    // the network: the run to resume is the `active_run` pointer and the decision
    // is temp-delta gated (kiln_core::recovery), not time/NTP based. Returns the
    // interrupted run's log file so the CSV logger continues it (append + RECOVERY
    // row) rather than starting fresh.
    let recovery = platform::attempt_recovery(&state).await;

    // Start CSV logging now (flash only, no network) so a resumed firing — or one
    // started the instant the web comes up — is logged from its first row.
    spawner.spawn(
        kiln_app::server::csv_logger_task(status, storage, clock, config, recovery).unwrap(),
    );

    // --- Network bring-up ---------------------------------------------------
    // cyw43 firmware blobs + PIO SPI + embassy-net stack + WiFi join + DHCP.
    // Returns the Stack the web workers serve on and the cyw43 `Control` the WiFi
    // monitor re-joins with. Everything below this line genuinely needs the network.
    let (stack, control) = platform::init_network(&spawner, p, config).await;

    // `make_app` bakes the shared `AppState` into the router; picoserve 0.18 then
    // serves the stateless `Router<P>` aliased as `AppRouter`, stored in a
    // StaticCell so the `#[task]` web_task stays non-generic. (`AppState` is
    // `Copy`, so recovery's borrow above leaves `state` usable here.)
    static APP: StaticCell<kiln_app::server::AppRouter> = StaticCell::new();
    let app: &'static _ = APP.init(kiln_app::server::make_app(state));
    let web_cfg = platform::web_config();

    // embassy-executor 0.10: `#[task]` fns return `Result<SpawnToken, _>` and
    // `Spawner::spawn` returns `()`, so each token is unwrapped before spawning.
    for id in 0..kiln_app::server::WEB_TASK_POOL_SIZE {
        spawner.spawn(
            kiln_app::server::web_task(id, stack, app, web_cfg, config.web_server_port).unwrap(),
        );
    }
    // WiFi reconnect monitor (`wifi_manager.monitor`): re-join on link failure.
    // Takes the `Control` handle (moved here — nothing else uses it) plus the
    // SSID/password, which live in the `'static` config.
    spawner.spawn(
        platform::wifi_monitor_task(
            control,
            stack,
            config.wifi_ssid.as_str(),
            config.wifi_password.as_str(),
        )
        .unwrap(),
    );
    // LCD status line (`server/lcd_manager.py`), optional. Built only when
    // `LCD_ENABLED`; a missing/mis-wired backpack disables it without affecting
    // the web server or WiFi. The kiln-app `lcd_task` renders each status change
    // through the firmware `Display`.
    if config.lcd_enabled {
        if let Some(display) = platform::init_display(lcd, config) {
            spawner.spawn(kiln_app::server::lcd_task(status, display).unwrap());
        }
    }
    spawner.spawn(platform::ntp_task(clock, stack).unwrap());
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
