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
use embassy_rp::multicore::{spawn_core1, Stack};
use embassy_rp::peripherals::CORE1;
use embassy_time::{Duration, Instant, Timer};
use static_cell::StaticCell;

use kiln_app::config::KilnConfig;
use kiln_app::server::{AppState, CommandChannel, RebootSignal, StatusWatch};
use kiln_control::Controller;

mod flash_handshake;
mod platform;

use platform::{FlashStorage, LcdDisplay, NtpClock};

/// Core 1 ↔ Core 0 channels and shared signals. `CriticalSectionRawMutex` is
/// mandatory: these are touched from both cores.
static COMMANDS: StaticCell<CommandChannel> = StaticCell::new();
static STATUS: StaticCell<StatusWatch> = StaticCell::new();
static REBOOT: StaticCell<RebootSignal> = StaticCell::new();

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
    let storage = platform::init_storage();
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
            spawner
                .spawn(control_task(core1_periphs, config, cmd_rx, status_tx))
                .unwrap()
        });
    });

    // Core 0: web + logging + recovery + WiFi + NTP + LCD.
    let executor0 = EXECUTOR0.init(Executor::new());
    executor0.run(|spawner| {
        spawner
            .spawn(core0_main(
                Core0Periphs::from(p),
                storage,
                config,
                commands.sender(),
                status,
                reboot,
            ))
            .unwrap()
    });
}

/// Peripherals handed to Core 1 (the kiln I/O only).
struct Core1Periphs {
    spi: embassy_rp::peripherals::SPI1,
    sck: embassy_rp::peripherals::PIN_18,
    mosi: embassy_rp::peripherals::PIN_19,
    miso: embassy_rp::peripherals::PIN_16,
    cs: embassy_rp::peripherals::PIN_28,
    ssr: embassy_rp::peripherals::PIN_15,
    watchdog: embassy_rp::peripherals::WATCHDOG,
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
    let (sensor, ssr, watchdog) = platform::build_kiln_io(p, config);
    let mut controller = Controller::new(
        sensor,
        ssr,
        watchdog,
        platform::control_params_from(config),
        Instant::now().as_millis(),
    );

    let (sub_ticks, sub_ms) = controller.timing();
    loop {
        let cmd = commands.try_receive().ok();
        let now_ms = Instant::now().as_millis();
        let outcome = controller.iterate(cmd, now_ms, NtpClock::unix_seconds_f64());

        if let Some(snapshot) = outcome.publish {
            status.send(snapshot);
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

/// Peripherals handed to Core 0 (network + flash + LCD).
struct Core0Periphs {
    // cyw43 (WiFi) pins, flash, I2C for the LCD, etc. Elided to the fields the
    // platform builders consume; see `platform::build_*`.
    raw: embassy_rp::Peripherals,
}

impl From<embassy_rp::Peripherals> for Core0Periphs {
    fn from(raw: embassy_rp::Peripherals) -> Self {
        Self { raw }
    }
}

/// Core 0 setup task: bring up cyw43 → an `embassy-net` `Stack`, mount flash,
/// build the picoserve app with shared state, and spawn the worker pool plus the
/// CSV-logging, LCD, WiFi-join, NTP, and reboot tasks. Mirrors `main.py`'s
/// asyncio startup, minus the control thread (now Core 1).
#[embassy_executor::task]
async fn core0_main(
    p: Core0Periphs,
    storage: &'static FlashStorage,
    config: &'static KilnConfig,
    commands: kiln_app::server::CommandSender,
    status: &'static StatusWatch,
    reboot: &'static RebootSignal,
) {
    let spawner = embassy_executor::Spawner::for_current_executor().await;

    // WiFi + network stack (cyw43 firmware blobs + PIO SPI), then DHCP. Returns
    // the `embassy-net` Stack the web workers serve on.
    let stack = platform::init_network(&spawner, &p).await;

    // The world, behind the kiln_app traits. Storage (mounted in `main`) routes
    // flash writes through the flash handshake; the clock is sntpc-disciplined;
    // the LCD is the firmware driver.
    let clock: &'static NtpClock = platform::init_clock();
    let display: &'static mut LcdDisplay = platform::init_display(&p);

    let state = AppState {
        commands,
        status,
        clock,
        storage,
        reboot,
        config,
    };

    // Crash recovery: parse the most recent log and, if interrupted mid-firing
    // within the safe temperature delta, resume. Uses kiln_app::recovery_io +
    // kiln_core::recovery (both host-tested); the resume Command is parsed here
    // on Core 0 and shipped to Core 1.
    platform::attempt_recovery(&state).await;

    // `AppRouter` (TAIT) names the router type so it stores in a StaticCell and
    // keeps the `#[task]` web_task non-generic.
    static APP: StaticCell<picoserve::Router<kiln_app::server::AppRouter, AppState>> =
        StaticCell::new();
    let app: &'static _ = APP.init(kiln_app::server::make_app());
    let config = platform::web_config();

    for id in 0..kiln_app::server::WEB_TASK_POOL_SIZE {
        spawner
            .spawn(kiln_app::server::web_task(id, stack, app, config, state))
            .unwrap();
    }
    spawner
        .spawn(kiln_app::server::csv_logger_task(status, storage, clock))
        .unwrap();
    spawner
        .spawn(kiln_app::server::lcd_task(status, display))
        .unwrap();
    spawner.spawn(platform::ntp_task(clock, stack)).unwrap();
    spawner.spawn(platform::reboot_task(reboot)).unwrap();
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
