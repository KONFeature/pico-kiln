# pico-kiln Rust architecture

End-state design for the MicroPython → Rust migration, and how every Rust crate
maps back to the Python it replaces.

The guiding rule: **the brain never touches the world.** All control decisions
live in a pure, host-testable core; only the outermost crates know that an
RP2350, a thermocouple, or WiFi exist. This is what lets us prove the
fire-relevant logic correct off-device (see `TESTING.md`).

---

## 1. Crate graph

```
                    kiln-core   (no_std, ZERO deps)
                    pure decisions + protocol types
                  ┌──────┬───────────────┬──────────┐
                  │      │               │          │
                  │      │ (drivers)     │          │
                  │      ▼               │          │
                  │   kiln-hal           │          │
                  │   embedded-hal       │          │
                  │   device drivers     │          │
                  │   ┌──────┴───────┐   │          │
                  ▼   ▼              ▼   ▼          ▼
            kiln-control          kiln-app      kiln-sim (optional, std)
            [Core 1: safety]      [Core 2: UX]  host thermal simulation
                  │                   │
                  └─────────┬─────────┘
                            ▼
                      kiln-firmware  (bin, thumbv8m.main-none-eabihf)
                      shim: init + peripheral split + channels + core dispatch
```

Dependencies point **inward only**. `kiln-core` depends on nothing. The two
halves talk to each other through exactly one thing: a channel carrying
`kiln-core::protocol` values. `kiln-firmware` does nothing but wire it together.

**5 shipping crates + 1 optional host-sim.**

| Crate | `no_std` | Role | Key deps |
|-------|:---:|------|----------|
| `kiln-core` | yes | All decision logic; the data model (`protocol`) | none |
| `kiln-hal` | yes | Device drivers over `embedded-hal` traits | `embedded-hal`, `kiln-core` (shared types) |
| `kiln-control` | yes | **Core 1** real-time safety loop | `kiln-core`, `kiln-hal`, `embassy-rp/sync/time/executor` |
| `kiln-app` | yes | **Core 2** networking, web, logging, LCD | `kiln-core`, `kiln-hal`, `embassy-*`, `embassy-net`, `cyw43`, `picoserve` |
| `kiln-firmware` | yes | Binary shim: init, split peripherals, dispatch cores | `kiln-control`, `kiln-app`, `embassy-rp/executor` |
| `kiln-sim` | no | Dev-only: run core+hal mocks against a thermal model | `kiln-core`, `kiln-hal` |

---

## 2. The safety boundary (why control/app are separate crates)

The most important invariant in a fire-capable controller is that the
**real-time safety loop cannot be starved or crashed by the application layer**.
The original code expresses this with a hard core split (Core 1 = control, Core 2
= web/WiFi) in `main.py` and `control_thread.py`.

We encode that same boundary in the *dependency graph*:

- `kiln-control` cannot `use` `picoserve` / `cyw43` / `embassy-net` — they aren't
  in its `Cargo.toml`. The networking stack physically cannot be pulled into the
  safety loop. That's a compile-time wall, not a convention.
- The only thing crossing the boundary is `kiln-core::protocol` over an
  `embassy-sync` channel — the same shape as today's command/status queues.
- Names describe **responsibility**, not core number: if a task ever migrates
  between cores, no crate gets renamed. Core affinity is decided once, at
  dispatch time, in `kiln-firmware`.

Backstop: the hardware watchdog (`WDT`, `control_thread.py:373`) lives with
`kiln-control` and resets the chip if the safety loop ever hangs; `panic = abort`
plus an SSR-off-on-drop guard in `kiln-hal` ensure a fault de-energises the kiln.

---

## 3. Crate-by-crate, with Python sources

### `kiln-core` — the brain (status: 10/11 modules done)

Pure logic, `#![no_std]`, zero deps, time injected as `now: f64` / `now_ms: u64`,
no strings (errors are typed enums). Already ported and equivalence-tested:

| Rust module | Python source | Symbols |
|-------------|---------------|---------|
| `pid` ✅ | `kiln/pid.py` | `PID` |
| `rate_monitor` ✅ | `kiln/rate_monitor.py` | `TempHistory` |
| `scheduler` ✅ | `kiln/scheduler.py` | `ScheduledProfileQueue` |
| `profile` ✅ | `kiln/profile.py` | `Profile` |
| `state` ✅ | `kiln/state.py` | `KilnState`, `KilnController` |
| `tuner` ✅ | `kiln/tuner.py` | `ZieglerNicholsTuner`, `TuningStage` |
| `temp_filter` ✅ | `kiln/hardware.py:114-197` | median spike-rejection, consecutive-fault counting, cold-start tolerance, range validation, window re-seed (the software half of `TemperatureSensor.read()`, post MAX31856 rework — **median, not EMA**) |
| `ssr_schedule` ✅ | `kiln/hardware.py:209-318` | time-proportional duty calc, mid-cycle duty **lock**, `MIN_SSR_OUTPUT` floor, single-cycle advance (everything in `SSRController` except `pin.value()`); time injected as `now_ms` |
| `gain_schedule` ✅ | `kiln/control_thread.py:132-160,585-606` | continuous gain scaling `g(T) = 1 + h·(T − T_ambient)`, `h<0`→disabled validation, + change-threshold gate (`Some(Gains)` exactly when the reference calls `pid.set_gains`) |
| `protocol` ✅ | `kiln/comms.py:191-506` | `MessageType` + `CommandMessage` → `enum Command` (tags 1..=10 preserved); `StatusMessage` templates → `Copy struct Status` (typed `KilnState`/`KilnError`/`StepKind`, no dicts, no heap strings; filenames in a bounded `ProfileName`) |

Still to extract — pure logic currently tangled with I/O in the Python, and
belongs in core, not the HAL:

| Rust module (planned) | Python source | What moves |
|-----------------------|---------------|-----------|
| `recovery` (decision) | `server/recovery.py:131-243` | `RecoveryInfo` / `check_recovery` math: is a resume warranted, at what step/elapsed (operates on already-parsed values) |

### `kiln-hal` — the hands (status: `max31856` + `ssr` built; `lcd` planned)

Thin drivers generic over `embedded-hal` 1.0 traits (`SpiDevice`, `OutputPin`),
so they run against mocks on the host. Return raw readings; `kiln-control` wraps
the core filters around them.

| Rust module | Python source | What it does |
|-------------|---------------|--------------|
| `max31856` ✅ | `kiln/hardware.py:24-205` + `adafruit_max31856` | configure (notch + averaging), `start_autoconverting`, non-blocking 19-bit read → raw °C, decoded `Faults` |
| `ssr` ✅ | `kiln/hardware.py:209-318` (`pin.value()` calls) | GPIO on/off; SSR-off-on-`Drop` safety guard |
| `lcd` (optional) | `server/lcd_manager.py` (hardware init/draw) | display driver |

### `kiln-control` — Core 1 real-time loop (status: planned)

The orchestration that today is `ControlThread`. One `embassy` task; reads
sensor → core filters → state → PID → SSR every tick; drains the command
channel; publishes status; feeds the watchdog.

| Concern | Python source |
|---------|---------------|
| control loop / tick | `kiln/control_thread.py` `run` / `control_loop_iteration` (521-672) |
| tuning loop | `kiln/control_thread.py` `tuning_loop_iteration` (438-519) |
| command dispatch | `kiln/control_thread.py` `handle_command` (222-371) |
| profile load + retry | `kiln/control_thread.py` `load_profile_with_retry` (187-220) |
| watchdog feed | `kiln/control_thread.py:373-381` (`WDT`) |
| thread entry | `kiln/control_thread.py` `start_control_thread` (679-694) |

### `kiln-app` — Core 2 application layer (status: planned)

Everything user-facing and best-effort. Multiple `embassy` tasks on Core 0.

| Concern | Python source |
|---------|---------------|
| HTTP server + REST API | `server/web_server.py` (→ `picoserve` routes) |
| status fan-out to listeners | `server/status_receiver.py` (`StatusReceiver`) |
| CSV data logging (→ flash) | `server/data_logger.py` (`DataLogger`) |
| recovery I/O (read last log) | `server/recovery.py` `_find_most_recent_log` / `_parse_last_log_entry` (244-330) |
| WiFi connect / monitor / NTP | `server/wifi_manager.py` + `main.py` `wifi_connect_background` / `ntp_sync_background` |
| LCD render loop | `server/lcd_manager.py` (`LCDManager.run`) |
| HTML / profile caches | `server/html_cache.py`, `server/profile_cache.py` |

### `kiln-firmware` — the shim (status: planned)

The only crate that calls `embassy_rp::init`, owns the channel `static`s, and
chooses which core runs what. Maps directly to `main.py`'s boot stages.

| Concern | Python source |
|---------|---------------|
| boot sequence / peripheral init | `main.py` `main()`, `boot.py` |
| create command/status channels + flags | `main.py:188-199` |
| launch control on Core 1 | `main.py:210-213` (`_thread.start_new_thread`) → `spawn_core1` |
| run app on Core 0 | `main.py:382-384` (`asyncio.run(main)`) |

### `kiln-sim` — host simulation (status: optional, new)

No direct Python equivalent (replaces the "Simulating Kiln Behavior" note in the
project README). A `std` binary that drives the same `kiln-core` control logic
the Core 1 loop runs — sensor→filter→state→PID→SSR — against a software thermal
model using mock `kiln-hal` implementations. End-to-end firing runs on a laptop,
no hardware. (It re-wires the tick rather than importing `kiln-control`, which is
`embassy-rp`-bound; keeping that orchestration thin is what makes this cheap.)

---

## 4. The concurrency boundary: `comms.py` → `embassy-sync`

`kiln/comms.py` is **not** ported as logic — it's reimplemented with embassy
primitives in the firmware/halves. The data *shapes* it defines move to
`kiln-core::protocol`.

| `kiln/comms.py` | Rust equivalent | Lives in |
|-----------------|-----------------|----------|
| `ThreadSafeQueue` (command) | `Channel<CriticalSectionRawMutex, Command, N>` | firmware owns static; halves get endpoints |
| `ThreadSafeQueue` (status) | `Channel` or `Watch<Status>` | firmware static |
| `ReadyFlag` (120-161) | `Signal` | firmware static |
| `QuietMode` (164-188) | `Signal` / `AtomicBool` | firmware static |
| `StatusCache` (560-638) | `Watch` / `Mutex<Status>` | `kiln-app` |
| `QueueHelper.put_nowait/get_nowait` (508-558) | `try_send` / `try_receive` | call sites |
| `MessageType` + `CommandMessage` (191-342) | `enum Command` | `kiln-core::protocol` |
| `StatusMessage` templates (344-506) | `struct Status` | `kiln-core::protocol` |
| `state_to_string` (204-231) | serialise at the web boundary only | `kiln-app` |

---

## 5. How the shim wires it (canonical embassy-rp multicore)

```rust
// kiln-firmware: the ONLY place that calls init / spawn_core1.
let p = embassy_rp::init(Default::default());
let (control_p, app_p) = split_peripherals(p);   // SPI/GPIO/WDT  vs  PIO/DMA/I2C/FLASH

// Cross-core channels MUST use CriticalSectionRawMutex (see gotcha 3).
static CMD:    Channel<CriticalSectionRawMutex, Command, 8> = Channel::new();
static STATUS: Watch<CriticalSectionRawMutex,   Status,  4> = Watch::new();

// Core 1: the real-time safety loop.
spawn_core1(p.CORE1, CORE1_STACK.init(Stack::new()), move || {
    CONTROL_EXEC.init(Executor::new())
        .run(|s| kiln_control::run(s, control_p, CMD.receiver(), STATUS.sender()));
});

// Core 0: the application layer.
EXEC0.init(Executor::new())
    .run(|s| kiln_app::run(s, app_p, CMD.sender(), STATUS.receiver()));
```

Each half exposes a single entry fn — `run(spawner, its_peripherals,
channel_endpoints)`. The halves never see the `static`s, never call `init`, and
never reference each other.

---

## 6. Gotchas (none fatal, all real)

1. **Crate boundaries don't enforce core affinity.** Code in `kiln-control` is
   not *automatically* on Core 1 — the shim's `spawn_core1` placement is what
   pins it. Enforced by "the shim is the only spawner." Keep a comment there.
2. **Both halves are RP2350-specific.** Each binds concrete peripherals
   (control → SPI/GPIO/WDT; app → PIO+DMA for cyw43, I2C for LCD, flash), so both
   depend on `embassy-rp`. Chip portability stays in `kiln-core`/`kiln-hal`,
   which is where we wanted it anyway.
3. **Cross-core channels need `CriticalSectionRawMutex`.** `NoopRawMutex` /
   `ThreadModeRawMutex` are single-executor only and will misbehave across cores.

---

## 7. Build & test targets

| Action | Command |
|--------|---------|
| Host unit + replay tests | see `TESTING.md §5` (static-musl + `rust-lld`) |
| Cross-compile a crate | `cargo build -p kiln-core --lib --target thumbv8m.main-none-eabihf` |
| Final firmware image | `cargo build -p kiln-firmware --release` (RP2350 target) |

`kiln-core` and `kiln-hal` are host-testable directly. `kiln-control` logic is
covered transitively (it's mostly `kiln-core` calls) and end-to-end by
`kiln-sim`.

---

## 8. Status & roadmap

- ✅ `kiln-core`: 10 modules ported, 54 unit + 11 replay tests green.
- ✅ `kiln-hal`: `max31856` + `ssr` drivers over `embedded-hal`, 12 tests green.
- ⏭ `kiln-core`: extract `recovery` (the last decision module).
- ⏭ `kiln-control` + `kiln-app`: embassy tasks; `kiln-firmware` shim.
- ⏭ `kiln-sim`: optional host thermal-model harness.

> Workspace `members` in `rust/Cargo.toml` lists `kiln-core` + `kiln-hal`; add
> each remaining crate as it lands.
