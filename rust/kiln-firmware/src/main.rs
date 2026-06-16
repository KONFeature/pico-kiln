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
mod stack;

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
/// 16 KiB: measured high-water hit 7188 B on the old 8 KiB stack (~492 B from the
/// MSPLIM guard trip) and jumped +3 KiB mid-run — too thin for the safety-critical
/// control core. Cost: +8 KiB `.bss`, taken from Core 0's ~60 KiB stack headroom.
/// See PICOSERVE_RAM.md ("MEASURED — real high-water").
const CORE1_STACK_BYTES: u32 = 16384;
static CORE1_STACK: StaticCell<Stack<{ CORE1_STACK_BYTES as usize }>> = StaticCell::new();
static EXECUTOR1: StaticCell<Executor> = StaticCell::new();
static EXECUTOR0: StaticCell<Executor> = StaticCell::new();

#[cortex_m_rt::entry]
fn main() -> ! {
    // Arm the Core 0 stack-limit guard FIRST, before any deep frame: a stack
    // overflow now traps as a HardFault at the boundary instead of smashing .bss
    // (see stack.rs). Debug builds additionally paint the free stack so the
    // high-water task can report how deep each route really goes.
    stack::arm_guard();
    #[cfg(feature = "stack-debug")]
    stack::paint_current();

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
    // Surface any panic/hardfault captured by the previous boot (see FaultRecord).
    report_prior_fault();
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
    // Core 1's stack is this static, not the `_stack_end` region — guard + paint
    // it by its own base. Paint the full span now, while it is still entirely free
    // (Core 1 has not started). `spawn_core1` moves `stack`, so grab its base first.
    let core1_bottom = stack as *const _ as u32;
    #[cfg(feature = "stack-debug")]
    stack::paint_range(core1_bottom, core1_bottom + CORE1_STACK_BYTES);
    spawn_core1(p.CORE1, stack, move || {
        // MSPLIM is banked per-core: arm Core 1's guard on Core 1.
        stack::arm_guard_at(core1_bottom);
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
        );
        #[cfg(feature = "stack-debug")]
        spawner.spawn(stack_highwater_task(core1_bottom).unwrap());
    });
}

/// Debug-only (`stack-debug`): periodically log both cores' stack high-water so a
/// LAN-traffic session ranks the real per-route peak. Lands in /api/logs + diag
/// flash. Compiled out of the shipped image.
#[cfg(feature = "stack-debug")]
#[embassy_executor::task]
async fn stack_highwater_task(core1_bottom: u32) -> ! {
    loop {
        Timer::after(Duration::from_secs(30)).await;
        stack::report_highwater(core1_bottom, CORE1_STACK_BYTES);
    }
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

    // --- USB-NCM: the wired escape hatch — OFF to maximise the Core 0 stack ------
    // The USB-NCM worker + its net stack are ~84 KB+ of `.bss`, stolen from the
    // stack picoserve's deep serve poll needs (~249 KB peak). With WiFi now stable,
    // turn USB off so all reclaimed RAM goes to the stack. Flip back to `true` only
    // after confirming the larger `.bss` still clears the serve-poll peak (and
    // restore WEB_TASK_POOL_TOTAL = WEB_TASK_POOL_SIZE + SECONDARY_WEB_WORKERS).
    const USE_USB_NCM: bool = false;
    if USE_USB_NCM {
        // Comes up whenever the cable is enumerated, serving the same router as WiFi,
        // so config + files + logs are reachable over USB regardless of WiFi/Core 1.
        let usb_stack = platform::init_usb_ncm(&spawner, usb);
        for i in 0..kiln_app::server::SECONDARY_WEB_WORKERS {
            let id = kiln_app::server::WEB_TASK_POOL_SIZE + i;
            spawner.spawn(kiln_app::server::web_task(id, usb_stack, app, web_cfg, port).unwrap());
        }
        log::info!(target: "boot", "usb-ncm up at 192.168.7.1 ({} workers); web/logs reachable over USB", kiln_app::server::SECONDARY_WEB_WORKERS);
    }

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
        //
        // DIAGNOSTIC: the monitor is the ONLY thing issuing cyw43 control ioctls
        // after boot. cyw43 0.7's runner `panic!`s on any non-zero ioctl status
        // (runner.rs:714), which halts Core 0 (USB + WiFi both die). On a WPA3-SAE
        // link a rekey/flap makes the monitor's re-`join()` ioctl error → panic.
        // Flipped off to confirm: if STA stops dying, that path is the culprit and
        // the durable fix is patching cyw43 to not panic on an ioctl error.
        const ENABLE_WIFI_MONITOR: bool = false;
        if ENABLE_WIFI_MONITOR {
            spawner.spawn(
                platform::wifi_monitor_task(
                    control,
                    sta_stack,
                    config.wifi_ssid.as_str(),
                    config.wifi_password.as_str(),
                )
                .unwrap(),
            );
        } else {
            // Drop the handle so nothing issues control ioctls post-boot; cyw43's
            // own firmware auto-reconnects a dropped link without driver ioctls.
            core::mem::drop(control);
        }
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

// === Fault capture (diagnostic) ============================================
// A silent panic/hardfault on Core 0 halts the executor — USB *and* WiFi die with
// no log, because the buffered diag never flushes. Worse, halting does NOT reset:
// a live Core 1 keeps feeding the watchdog, so the chip just hangs. To find the
// culprit we stash the fault site in a `.uninit` RAM cell (untouched by the boot
// `.bss` zeroing, so it survives a warm reset), force an immediate reset, and
// surface it on the next boot via `report_prior_fault` → /api/logs. For a
// hardfault, `arm-none-eabi-addr2line -e <elf> <pc>` maps the PC to the code.
const FAULT_MAGIC: u32 = 0xF00D_BEEF;

/// Stack-scan backtrace depth (code-range return addresses captured at fault).
const BT_DEPTH: usize = 8;

#[repr(C)]
struct FaultRecord {
    magic: u32,
    kind: u32, // 1 = panic, 2 = hardfault
    line: u32,
    pc: u32,       // hardfault: faulting instruction (0 = branch-to-null)
    lr: u32,       // hardfault: link register (call site — often 0 here)
    cfsr: u32,     // hardfault: SCB.CFSR fault-status bits
    cpuid: u32,    // which core faulted (0 = Core 0 / net+web, 1 = Core 1 / control)
    sp: u32,       // hardfault: stack pointer at fault (near .bss top ⇒ stack overflow)
    file_ptr: u32, // panic: &str into flash (.rodata) — stable across a warm reset
    file_len: u32,
    bt: [u32; BT_DEPTH], // hardfault: code-range words scanned off the faulting stack
}

#[link_section = ".uninit.FAULT"]
static mut FAULT_MARKER: core::mem::MaybeUninit<FaultRecord> = core::mem::MaybeUninit::uninit();

#[allow(clippy::too_many_arguments)]
fn record_fault(
    kind: u32,
    file_ptr: u32,
    file_len: u32,
    line: u32,
    pc: u32,
    lr: u32,
    cfsr: u32,
    cpuid: u32,
    sp: u32,
    bt: [u32; BT_DEPTH],
) {
    // Raw pointer (not `&mut FAULT_MARKER`) to stay clear of the static_mut_refs lint.
    let p = core::ptr::addr_of_mut!(FAULT_MARKER) as *mut FaultRecord;
    unsafe {
        p.write(FaultRecord {
            magic: FAULT_MAGIC,
            kind,
            line,
            pc,
            lr,
            cfsr,
            cpuid,
            sp,
            file_ptr,
            file_len,
            bt,
        });
    }
}

/// Log + clear any fault stashed by the previous boot. Call once in `main`, right
/// after the logger is installed, so it lands in the `/api/logs` ring.
fn report_prior_fault() {
    let p = core::ptr::addr_of_mut!(FAULT_MARKER) as *mut FaultRecord;
    unsafe {
        if core::ptr::addr_of!((*p).magic).read() != FAULT_MAGIC {
            return;
        }
        let kind = core::ptr::addr_of!((*p).kind).read();
        let line = core::ptr::addr_of!((*p).line).read();
        let pc = core::ptr::addr_of!((*p).pc).read();
        let lr = core::ptr::addr_of!((*p).lr).read();
        let cfsr = core::ptr::addr_of!((*p).cfsr).read();
        let cpuid = core::ptr::addr_of!((*p).cpuid).read();
        let sp = core::ptr::addr_of!((*p).sp).read();
        let file_ptr = core::ptr::addr_of!((*p).file_ptr).read();
        let file_len = core::ptr::addr_of!((*p).file_len).read();
        let mut bt = [0u32; BT_DEPTH];
        for (i, slot) in bt.iter_mut().enumerate() {
            *slot = core::ptr::addr_of!((*p).bt[i]).read();
        }
        core::ptr::addr_of_mut!((*p).magic).write(0); // clear so a clean boot won't re-report
        let file = if file_ptr != 0 && file_len > 0 && file_len < 256 {
            core::str::from_utf8(core::slice::from_raw_parts(file_ptr as *const u8, file_len as usize))
                .unwrap_or("?")
        } else {
            "?"
        };
        match kind {
            1 => log::error!(target: "fault", "RECOVERED FROM PANIC at {}:{}", file, line),
            2 => {
                // CFSR bit 20 (UFSR.STKOF) = the MSPLIM stack-limit guard tripped:
                // a stack overflow, escalated to HardFault (see stack.rs).
                let cause = if cfsr & 0x0010_0000 != 0 { " [STACK OVERFLOW]" } else { "" };
                log::error!(
                    target: "fault",
                    "RECOVERED FROM HARDFAULT{} core={} pc=0x{:08x} lr=0x{:08x} cfsr=0x{:08x} sp=0x{:08x}",
                    cause, cpuid, pc, lr, cfsr, sp
                );
                // Second line: the stack-scan backtrace (addr2line each). Split out
                // because the combined line would exceed the 128-byte log line cap.
                log::error!(
                    target: "fault",
                    "fault bt: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
                    bt[0], bt[1], bt[2], bt[3], bt[4], bt[5], bt[6], bt[7]
                );
            }
            other => log::error!(target: "fault", "RECOVERED FROM fault kind={} pc=0x{:08x} lr=0x{:08x}", other, pc, lr),
        }
    }
}

/// SCB Configurable Fault Status Register — the fault-cause bits.
const SCB_CFSR: *const u32 = 0xE000_ED28 as *const u32;
/// RP2350 SIO CPUID register — reads 0 on Core 0, 1 on Core 1.
const SIO_CPUID: *const u32 = 0xD000_0000 as *const u32;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    // Free the handler from the MSPLIM guard before it touches the stack (see
    // stack::disarm_guard): a panic raised near the limit must not re-trip it.
    stack::disarm_guard();
    // De-energise the kiln (RAM-safe raw GPIO write), stash the panic site, then
    // reset immediately — do NOT rely on the watchdog, which a live Core 1 keeps
    // feeding (the chip would otherwise hang here forever). The SSR pins reinit to
    // Level::Low on the reboot, so the relay stays off across the reset.
    platform::raw_ssr_off();
    let (fp, fl, line) = match info.location() {
        Some(loc) => (loc.file().as_ptr() as u32, loc.file().len() as u32, loc.line()),
        None => (0, 0, 0),
    };
    let cpuid = unsafe { core::ptr::read_volatile(SIO_CPUID) };
    record_fault(1, fp, fl, line, 0, 0, 0, cpuid, 0, [0; BT_DEPTH]);
    cortex_m::peripheral::SCB::sys_reset()
}

/// Top of SRAM (RP2350: 512 KiB at 0x20000000) — the upper bound for the stack scan.
const RAM_TOP: usize = 0x2008_0000;

#[cortex_m_rt::exception]
unsafe fn HardFault(ef: &cortex_m_rt::ExceptionFrame) -> ! {
    // Disarm MSPLIM first: an overflow fault enters here with SP near the limit, so
    // the handler's own frame (stack scan + record) must be free to use the reserve
    // below it without re-tripping the guard into a nested fault → lockup.
    stack::disarm_guard();
    platform::raw_ssr_off();
    let cfsr = core::ptr::read_volatile(SCB_CFSR);
    let cpuid = core::ptr::read_volatile(SIO_CPUID);
    // pc/lr are null here (branch-to-null zeroes them), so walk the faulting stack
    // upward from the exception frame and collect words that look like code return
    // addresses (flash XIP range, Thumb bit set). addr2line these to find who called
    // through the null pointer.
    let mut bt = [0u32; BT_DEPTH];
    let sp = ef as *const cortex_m_rt::ExceptionFrame as usize;
    let mut addr = sp;
    let mut n = 0;
    while addr < RAM_TOP && n < BT_DEPTH {
        let v = core::ptr::read_volatile(addr as *const u32);
        if (0x1000_0000..0x1028_0000).contains(&v) && (v & 1) == 1 {
            bt[n] = v;
            n += 1;
        }
        addr += 4;
    }
    record_fault(2, 0, 0, 0, ef.pc() as u32, ef.lr() as u32, cfsr, cpuid, sp as u32, bt);
    cortex_m::peripheral::SCB::sys_reset()
}
