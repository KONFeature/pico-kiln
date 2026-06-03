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

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_executor::Executor;
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

/// Status-publish suppression during the WiFi-connect phase (`main.py`'s
/// `QuietMode`). Starts quiet so Core 1 doesn't publish while WiFi bring-up needs
/// the CPU; cleared once the network is up. Read on Core 1, written on Core 0.
static QUIET: AtomicBool = AtomicBool::new(true);

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
        spi: p.SPI1,
        sck: p.PIN_18,
        mosi: p.PIN_19,
        miso: p.PIN_16,
        cs: p.PIN_28,
        ssr: p.PIN_15,
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

    let executor0 = EXECUTOR0.init(Executor::new());
    executor0.run(|spawner| {
        spawner.spawn(
            core0_main(
                core0_periphs,
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
    spi: Peri<'static, embassy_rp::peripherals::SPI1>,
    sck: Peri<'static, embassy_rp::peripherals::PIN_18>,
    mosi: Peri<'static, embassy_rp::peripherals::PIN_19>,
    miso: Peri<'static, embassy_rp::peripherals::PIN_16>,
    cs: Peri<'static, embassy_rp::peripherals::PIN_28>,
    ssr: Peri<'static, embassy_rp::peripherals::PIN_15>,
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
        let outcome = controller.iterate(cmd, now_ms, NtpClock::unix_seconds_f64());

        // QuietMode: suppress status sends during the WiFi-connect phase.
        if let Some(snapshot) = outcome.publish {
            if !QUIET.load(Ordering::Acquire) {
                status.send(snapshot);
            }
        }

        if outcome.faulted {
            Timer::after(Duration::from_secs(1)).await;
            continue;
        }

        for _ in 0..sub_ticks {
            // Flash handshake: if Core 0 is about to program flash, de-energise
            // the SSR now (flash still live) and park in RAM until it is done.
            if flash_handshake::pause_requested() {
                let _ = controller.force_ssr_off();
                flash_handshake::park_until_idle(platform::raw_watchdog_feed);
            }
            Timer::after(Duration::from_millis(sub_ms)).await;
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
/// The `platform::init_network` builder that consumes these is still an
/// `unimplemented!` DEVICE stub, so the fields are not read yet.
#[allow(dead_code)]
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

/// Core 0 setup task: bring up cyw43 → an `embassy-net` `Stack`, mount flash,
/// wait for Core 1's ready handshake, run crash recovery, build the picoserve app
/// with shared state, and spawn the worker pool plus the CSV-logging,
/// WiFi-monitor, NTP, and reboot tasks. Mirrors `main.py`'s asyncio startup, minus
/// the control thread (now Core 1) and the LCD task (deferred — see U9).
#[embassy_executor::task]
async fn core0_main(
    p: Core0Periphs,
    storage: &'static FlashStorage,
    config: &'static KilnConfig,
    commands: kiln_app::server::CommandSender,
    status: &'static StatusWatch,
    reboot: &'static RebootSignal,
) {
    // SAFETY: `core0_main` runs as a task on Core 0's embassy executor, which is
    // exactly the precondition embassy-executor 0.10 requires for this call.
    let spawner = unsafe { embassy_executor::Spawner::for_current_executor() }.await;

    // WiFi + network stack (cyw43 firmware blobs + PIO SPI), then DHCP. Returns
    // the `embassy-net` Stack the web workers serve on.
    let stack = platform::init_network(&spawner, &p).await;

    // WiFi is up — let Core 1 resume publishing status (clear QuietMode).
    QUIET.store(false, Ordering::Release);

    // The world, behind the kiln_app traits. Storage (mounted in `main`) routes
    // flash writes through the flash handshake; the clock is sntpc-disciplined.
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
    // never signals within 20 s the kiln is unsafe to operate — reset and retry
    // the boot rather than serving the web/recovery stack against a dead control
    // core.
    if with_timeout(Duration::from_secs(20), READY.wait())
        .await
        .is_err()
    {
        cortex_m::peripheral::SCB::sys_reset();
    }

    // Crash recovery: parse the most recent log and, if interrupted mid-firing
    // within the safe temperature delta, resume. Uses kiln_app::recovery_io +
    // kiln_core::recovery (both host-tested); the resume Command is parsed here on
    // Core 0 and shipped to Core 1. Returns the interrupted run's log file (if
    // any) so the CSV logger continues it (append + RECOVERY row) rather than
    // starting a fresh file.
    let recovery = platform::attempt_recovery(&state).await;

    // `make_app` bakes the shared `AppState` into the router; picoserve 0.18
    // then serves the stateless `Router<P>` aliased as `AppRouter`, which names
    // the router type so it stores in a StaticCell and keeps the `#[task]`
    // web_task non-generic. (`AppState` is `Copy`, so the copy into `make_app`
    // leaves `state` usable.)
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
    spawner.spawn(
        kiln_app::server::csv_logger_task(status, storage, clock, config, recovery).unwrap(),
    );
    // WiFi reconnect monitor (`wifi_manager.monitor`): re-join on link failure.
    spawner.spawn(platform::wifi_monitor_task(stack).unwrap());
    // NOTE: the LCD task is intentionally NOT spawned — the LCD driver
    // (`lcd1602_i2c` + `lcd_manager`) is not yet ported. See MIGRATION_AUDIT.md U9.
    // TODO(LCD): port the driver, then spawn `lcd_task(status, display)` here.
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
