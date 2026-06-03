# pico-kiln Rust architecture

End-state design for the MicroPython → Rust migration, and how every Rust crate
maps back to the Python it replaces.

The guiding rule: **the brain never touches the world.** All control decisions
live in a pure, host-testable core; the device drivers are generic over
`embedded-hal` traits; and the two halves (`kiln-control`, `kiln-app`) are
generic over the *platform*, not the chip. Exactly one crate — `kiln-firmware` —
knows that an RP2350 exists. This is what lets us prove the fire-relevant logic
correct off-device (see `TESTING.md`) **and** run the real control loop on a host.

---

## 1. Crate graph

```
                    kiln-core   (no_std, ZERO deps)
                    pure decisions + protocol types
                  ┌──────┬───────────────┬──────────┐
                  │      │               │          │
                  │      │ (drivers +    │          │
                  │      ▼  Platform     │          │
                  │   kiln-hal  traits)  │          │
                  │   embedded-hal drivers          │
                  │   Watchdog + driver bounds      │
                  │   ┌──────┴───────┐   │          │
                  ▼   ▼              ▼   ▼          ▼
            kiln-control          kiln-app      kiln-sim (optional, std)
            Core 1 safety loop    Core 2 UX     drives the REAL
            generic / platform    generic /     kiln-control loop
            (no embassy-rp)       platform      on host
                  │                   │
                  └─────────┬─────────┘
                            ▼
                      kiln-firmware  (bin, thumbv8m.main-none-eabihf)
                      THE ONLY embassy-rp / cyw43 crate: builds concrete
                      drivers + net Stack, injects them; owns init,
                      #[task] shims, channels, core dispatch
```

Dependencies point **inward only**. `kiln-core` depends on nothing. The two
halves talk to each other through exactly one thing: a channel carrying
`kiln-core::protocol` values, and they are **generic over the platform**: they
take already-constructed drivers + abstraction traits, never raw peripherals.
`kiln-firmware` does nothing but build the concrete RP2350 types and wire them
together — it is the single crate that names `embassy-rp` / `cyw43`.

**5 shipping crates + 1 optional host-sim** (the `Platform` traits live in
`kiln-hal`, so no extra crate).

| Crate | `no_std` | Role | Key deps |
|-------|:---:|------|----------|
| `kiln-core` | yes | All decision logic; the data model (`protocol`) | none |
| `kiln-hal` | yes | Device drivers over `embedded-hal` traits **+ `Platform` abstraction traits** (`Watchdog`, …) | `embedded-hal`, `embedded-storage`, `kiln-core` (shared types) |
| `kiln-control` | yes | **Core 1** real-time safety loop, **generic over platform** | `kiln-core`, `kiln-hal`, `embassy-sync/time/executor` (**no `embassy-rp`**) |
| `kiln-app` | yes | **Core 2** networking, web, logging, LCD, **generic over platform** | `kiln-core`, `kiln-hal`, `embassy-sync/time/executor`, `embassy-net` (`Stack`), `picoserve` (**no `embassy-rp`, no `cyw43`**) |
| `kiln-firmware` | yes | Binary shim + **only RP2350-aware crate**: init, build/inject drivers, cyw43+PIO bring-up, `#[task]` shims, core dispatch | `kiln-control`, `kiln-app`, `embassy-rp`, `cyw43`, `cyw43-pio` |
| `kiln-sim` | no | Dev-only: drive the **real `kiln-control` loop** against a thermal model | `kiln-core`, `kiln-hal`, `kiln-control` |

---

## 2. The safety boundary (why control/app are separate crates)

The most important invariant in a fire-capable controller is that the
**real-time safety loop cannot be starved or crashed by the application layer**.
The original code expresses this with a hard core split (Core 1 = control, Core 2
= web/WiFi) in `main.py` and `control_thread.py`.

We encode that same boundary in the *dependency graph*:

- `kiln-control` cannot `use` `picoserve` / `cyw43` / `embassy-net` — nor even
  `embassy-rp` — they aren't in its `Cargo.toml`. The networking stack physically
  cannot be pulled into the safety loop. That's a compile-time wall, not a
  convention.
- The only thing crossing the boundary is `kiln-core::protocol` over an
  `embassy-sync` channel — the same shape as today's command/status queues.
- Names describe **responsibility**, not core number: if a task ever migrates
  between cores, no crate gets renamed. Core affinity is decided once, at
  dispatch time, in `kiln-firmware`.

Backstop: the watchdog *feed* (`WDT`, `control_thread.py:373`) lives with
`kiln-control` (it holds an `impl kiln_hal::Watchdog`; `kiln-firmware` supplies
the concrete RP2350 `WATCHDOG`) and resets the chip if the safety loop ever
hangs; `panic = abort` plus an SSR-off-on-drop guard in `kiln-hal` ensure a fault
de-energises the kiln.

---

## 3. Crate-by-crate, with Python sources

### `kiln-core` — the brain (status: 11/11 modules done ✅)

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
| `recovery` ✅ | `server/recovery.py:156-241` | `check_recovery` decision (`LastLogEntry` → `RecoveryDecision`/`RecoveryReason`): was the last logged state RUNNING + is the current temp within delta, with the resume params echoed through; operates on already-parsed values |

Every decision module is now extracted — the only recovery code left in the
Python is genuine I/O (find the most recent log, read its last line, split the
columns), which belongs in `kiln-app`, not core:

| Stays in `kiln-app` | Python source | Why |
|---------------------|---------------|-----|
| recovery I/O | `server/recovery.py` `_find_most_recent_log` / `_parse_last_log_entry` (244-355) | filesystem scan + CSV parse; feeds the parsed `LastLogEntry` into `kiln-core::recovery::check_recovery` |

### `kiln-hal` — the hands (status: `max31856` + `ssr` + `platform` traits + `MultiSsr` done ✅; `lcd` planned)

Thin drivers generic over `embedded-hal` 1.0 traits (`SpiDevice`, `OutputPin`),
so they run against mocks on the host. Return raw readings; `kiln-control` wraps
the core filters around them. This crate is also the natural home for the
`Platform` abstraction traits the two halves are generic over (a small set —
e.g. `Watchdog` — plus each half's driver bounds, not one god-trait): it's
`no_std`, already depends on `embedded-hal`, and is already a dependency of both
halves, so the traits cost no extra crate.

| Rust module | Python source | What it does |
|-------------|---------------|--------------|
| `max31856` ✅ | `kiln/hardware.py:24-205` + `adafruit_max31856` | configure (notch + averaging), `start_autoconverting`, non-blocking 19-bit read → raw °C, decoded `Faults` |
| `ssr` ✅ | `kiln/hardware.py:209-318` (`pin.value()` calls) | GPIO on/off; SSR-off-on-`Drop` safety guard |
| `lcd` (optional) | `server/lcd_manager.py` (hardware init/draw) | display driver (over `embedded-hal` I2C) |
| `platform` (traits) ✅ | — (new; replaces direct `embassy-rp` use in the halves) | the abstractions the halves are generic over — `Watchdog`, `TempSensor`, `SsrOutput` (+ `NoopWatchdog`); `kiln-firmware` implements them for the RP2350 |
| `MultiSsr<P, N>` ✅ | `kiln/hardware.py` (staggered multi-relay turn-on) | non-blocking inrush-staggered `SsrOutput` over N relays, `now_ms` injected |

### `kiln-control` — Core 1 real-time loop (status: done ✅ — sync `Controller` + async `run`, host-tested + thumbv8m)

The orchestration that today is `ControlThread`. One `embassy` task; reads
sensor → core filters → state → PID → SSR every tick; drains the command
channel; publishes status; feeds the watchdog.

**Platform-generic.** Its entry point takes *constructed* drivers
(`Max31856<Spi>`, `Ssr<Pin>`) plus an `impl kiln_hal::Watchdog` and the channel
endpoints — never raw peripherals. It therefore needs only the portable embassy
crates (`embassy-time` / `-sync` / `-executor`), **not** `embassy-rp`; the
concrete peripherals are built in `kiln-firmware` and injected. (This is also
what lets `kiln-sim` run the real loop — see below.)

| Concern | Python source |
|---------|---------------|
| control loop / tick | `kiln/control_thread.py` `run` / `control_loop_iteration` (521-672) |
| tuning loop | `kiln/control_thread.py` `tuning_loop_iteration` (438-519) |
| command dispatch | `kiln/control_thread.py` `handle_command` (222-371) |
| profile load + retry | `kiln/control_thread.py` `load_profile_with_retry` (187-220) |
| watchdog feed | `kiln/control_thread.py:373-381` (`WDT`) |
| thread entry | `kiln/control_thread.py` `start_control_thread` (679-694) |

### `kiln-app` — Core 2 application layer (status: pure modules done ✅ — json/csv/recovery_io/api/profile_json/timefmt/errors/tuning_names host-tested; embassy glue `server.rs` written, device-only)

Everything user-facing and best-effort. Multiple `embassy` tasks on Core 0.

**Platform-generic.** It takes a *running* `embassy_net::Stack<'_>`, an
`impl embedded_storage` flash handle (CSV logging), and an `embedded-hal` I2C for
the LCD — so it names neither `embassy-rp` nor `cyw43`. `kiln-firmware` owns
WiFi-chip bring-up (the cyw43 firmware blob, PIO+DMA) and hands the finished
`Stack` across; the genuinely chip-specific link layer stays out of `kiln-app`.
(`picoserve` and `embassy-net` are themselves portable, name notwithstanding.)

| Concern | Python source |
|---------|---------------|
| HTTP server + REST API | `server/web_server.py` (→ `picoserve` routes) |
| status fan-out to listeners | `server/status_receiver.py` (`StatusReceiver`) |
| CSV data logging (→ flash) | `server/data_logger.py` (`DataLogger`) |
| recovery I/O (read last log) | `server/recovery.py` `_find_most_recent_log` / `_parse_last_log_entry` (244-330) |
| WiFi connect / monitor / NTP | `server/wifi_manager.py` + `main.py` `wifi_connect_background` / `ntp_sync_background` |
| LCD render loop | `server/lcd_manager.py` (`LCDManager.run`) |
| HTML / profile caches | `server/html_cache.py`, `server/profile_cache.py` |

### `kiln-firmware` — the shim (status: written, device-only — excluded from the host workspace; compiles on thumbv8m with `memory.x` + cyw43 blobs)

**The only RP2350-aware crate.** It alone calls `embassy_rp::init`, runs
`bind_interrupts!`, owns the channel `static`s, loads the cyw43 firmware blob
(PIO+DMA), **builds every concrete driver and injects it** into the two halves,
implements `kiln-hal`'s `Platform` traits for the chip, and chooses which core
runs what. It also owns the concrete `#[embassy_executor::task]` shims that
monomorphise the halves' generic `run<P>` (see Gotcha 4). Maps directly to
`main.py`'s boot stages.

| Concern | Python source |
|---------|---------------|
| boot sequence / peripheral init | `main.py` `main()`, `boot.py` |
| create command/status channels + flags | `main.py:188-199` |
| launch control on Core 1 | `main.py:210-213` (`_thread.start_new_thread`) → `spawn_core1` |
| run app on Core 0 | `main.py:382-384` (`asyncio.run(main)`) |

### `kiln-sim` — host simulation (status: optional, new)

No direct Python equivalent (replaces the "Simulating Kiln Behavior" note in the
project README). A `std` binary that runs the kiln control logic against a
software thermal model using mock `kiln-hal` implementations — end-to-end firing
on a laptop, no hardware.

Because `kiln-control` is now platform-generic (not `embassy-rp`-bound), the sim
**can import and drive the real `kiln-control` loop** — using `embassy-time`'s
`std` driver and host mocks for the sensor / SSR / watchdog — instead of
duplicating the tick wiring. The host harness then exercises the *actual*
shipping orchestration, not a copy of it: a direct payoff of the generification.

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
// kiln-firmware: the ONLY place that names embassy_rp / cyw43 / spawn_core1.
let p = embassy_rp::init(Default::default());

// Build the CONCRETE RP2350 drivers HERE, behind kiln-hal's generic traits.
let sensor = Max31856::new(spi_device);          // embedded-hal SpiDevice
let ssr    = Ssr::new(p.PIN_15.degrade());        // embedded-hal OutputPin
let wdt    = Rp2350Watchdog::new(p.WATCHDOG);     // impl kiln_hal::Watchdog
let net    = bring_up_cyw43(/* PIO + DMA + fw blob */).await;  // embassy_net::Stack

// Cross-core channels MUST use CriticalSectionRawMutex (see gotcha 3).
static CMD:    Channel<CriticalSectionRawMutex, Command, 8> = Channel::new();
static STATUS: Watch<CriticalSectionRawMutex,   Status,  4> = Watch::new();

// Core 1: the real-time safety loop. The halves expose a generic `run<P>`; the
// concrete types injected HERE monomorphise it (embassy #[task] can't be
// generic — Gotcha 4), so all chip knowledge stays in this crate.
spawn_core1(p.CORE1, CORE1_STACK.init(Stack::new()), move || {
    CONTROL_EXEC.init(Executor::new())
        .run(|s| kiln_control::run(s, sensor, ssr, wdt, CMD.receiver(), STATUS.sender()));
});

// Core 0: the application layer — handed a *running* Stack, not raw peripherals.
EXEC0.init(Executor::new())
    .run(|s| kiln_app::run(s, net, CMD.sender(), STATUS.receiver()));
```

Each half exposes a single **generic** entry fn — `run<P>(spawner,
constructed_drivers, channel_endpoints)`. The halves never see the `static`s,
never call `init`, never name `embassy-rp` / `cyw43`, and never reference each
other. Porting to a new chip means writing one new `kiln-firmware` (plus the
`Platform` impl); the halves recompile untouched.

---

## 6. Gotchas (none fatal, all real)

1. **Crate boundaries don't enforce core affinity.** Code in `kiln-control` is
   not *automatically* on Core 1 — the shim's `spawn_core1` placement is what
   pins it. Enforced by "the shim is the only spawner." Keep a comment there.
2. **Only `kiln-firmware` is RP2350-specific — by construction, not accident.**
   The halves are generic over a `Platform` (drivers + `Watchdog` from
   `kiln-hal`, a running `embassy_net::Stack`, `embedded-storage` flash); the
   firmware constructs every concrete driver and injects it, and owns the WiFi
   link layer (cyw43 blob, PIO+DMA). You don't get chip portability for free —
   you get a single, clean place to pay for it. (An RP2040 move barely benefits:
   still `embassy-rp`, plus the `thumbv6m` atomics caveat. The payoff is real
   only for a different chip family — or the host-sim win above.)
3. **Cross-core channels need `CriticalSectionRawMutex`.** `NoopRawMutex` /
   `ThreadModeRawMutex` are single-executor only and will misbehave across cores.
4. **`#[embassy_executor::task]` can't be generic.** So a half's generic
   `run<P>` (and any task it spawns) can't itself be a `#[task]`. Resolve it in
   `kiln-firmware`: keep `run<P>` a plain generic `async fn` and let the firmware
   own the concrete `#[task]` shims that instantiate `P = RP2350` (erasing to
   `dyn` only where a spawned sub-task needs a fixed signature). This *reinforces*
   "the shim is the only wirer."

---

## 7. Build & test targets

| Action | Command |
|--------|---------|
| Host unit + replay tests | see `TESTING.md §5` (static-musl + `rust-lld`) |
| Cross-compile a crate | `cargo build -p kiln-core --lib --target thumbv8m.main-none-eabihf` |
| Final firmware image | `cargo build -p kiln-firmware --release` (RP2350 target) |

`kiln-core` and `kiln-hal` are host-testable directly. Because `kiln-control` is
platform-generic, `kiln-sim` can run **the real loop** on the host (via
`embassy-time`'s `std` driver), so it's covered both transitively (mostly
`kiln-core` calls) and end-to-end.

---

## 8. Status & roadmap

- ✅ `kiln-core`: **all 11 decision modules ported**, 60 unit + 12 replay tests green.
- ✅ `kiln-hal`: `max31856` + `ssr` drivers over `embedded-hal`, 12 tests green.
- ⏭ `kiln-hal`: add the `Platform` abstraction traits (`Watchdog`, the per-half
  driver bounds) the halves will be generic over.
- ⏭ `kiln-control` + `kiln-app`: platform-generic embassy tasks; `kiln-firmware`
  shim builds + injects the concrete RP2350 types.
- ⏭ `kiln-sim`: optional host harness — drive the real `kiln-control` loop via
  `embassy-time`'s `std` driver.

> **Design note (this revision):** the halves were re-scoped from
> "RP2350-specific" to "platform-generic"; only `kiln-firmware` names
> `embassy-rp` / `cyw43`. Decided before the outer crates were written, so there
> is no retrofit cost. See §6.2 and §6.4.

> Workspace `members` in `rust/Cargo.toml` lists `kiln-core` + `kiln-hal`; add
> each remaining crate as it lands.
