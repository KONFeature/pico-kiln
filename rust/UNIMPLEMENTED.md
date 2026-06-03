# pico-kiln Rust port — Unimplemented inventory

**Date:** 2026-06-03
**Branch:** `feat/rust-kiln-core`
**Scope:** every stub / `unimplemented!()` / inert body remaining in the Rust port.

## TL;DR

All missing code lives in **one crate: `kiln-firmware`** (`src/platform.rs` +
`src/main.rs`). The logic crates (`kiln-core`, `kiln-control`, `kiln-app`,
`kiln-hal`) are complete and host-tested — `grep` for `unimplemented!`/`todo!`
finds nothing in them. What remains is **device-I/O wiring**, not business logic:
drivers (SPI/GPIO/cyw43/LCD), the flash filesystem, and two RAM-resident safety
pokes.

The `kiln-app` web/CSV/recovery/config consumers are all written against the
`Storage` / `Clock` / `Display` traits (`kiln-app/src/server.rs:54-117`); the
only `Storage` impl (`FlashStorage`) is currently entirely stubbed. Implementing
the trait bodies finishes the port without touching any consumer.

---

## Group A — hard `unimplemented!()` (panics at runtime)

| # | Symbol | Location | What it must do | Python source |
|---|---|---|---|---|
| A1 | `build_kiln_io()` | `platform.rs:144` | Construct SPI1 + CS + `Max31856` + `Ssr` pin(s) + `MaybeWatchdog` from `Core1Periphs` + cfg. Sequence: `init()` → `set_averaging()` → `set_noise_filter()` → **`start_autoconverting()`** (mandatory — without it the sensor reads a constant 0 °C). | `hardware.py:69-84` (`TemperatureSensor.__init__`) + `hardware.py:223-256` (`SSRController.__init__`) |
| A2 | `init_network()` | `platform.rs:388` | cyw43 firmware-blob load + PIO SPI + `embassy_net::Stack` + DHCP + `join_wpa2` retry loop. | `wifi_manager.py:68-137` (`connect`) |

These two **panic the firmware** the moment Core 1 / Core 0 reach them. Nothing
boots without them.

## Group B — `FlashStorage` methods return `Err`/`None` (flash FS not wired)

All of `impl Storage for FlashStorage` is a DEVICE stub (`platform.rs:275-348`).
Blocked on a flash-filesystem backend (see "Storage plan" below).

| # | Method | Location | Behaviour today | Python source |
|---|---|---|---|---|
| B1 | `read_chunk` | `platform.rs:276` | `Err` | `web_server.py` file_get / `data_logger` reads |
| B2 | `size` | `:287` | `None` | `os.stat` |
| B3 | `for_each` | `:292` | no-op | `os.listdir` |
| B4 | `append` | `:296` | `Err` | `data_logger.py` CSV row append |
| B5 | `remove` | `:309` | `Err` | `web_server` single delete |
| B6 | `remove_all` | `:313` | `Err` | bulk log delete |
| B7 | `upload_begin/write/commit/abort` | `:317-329` | `Err` | profile upload |
| B8 | `read_config` | `:340` | `Err` | `import config` — **so `config.json` never loads; the device silently runs `KilnConfig::default()`** |
| B9 | `write_config` | `:345` | `Err` | config PATCH persist |

Consequence today: no logging, no profile upload/list, no recovery (it reads
storage), no config override. `attempt_recovery()` (`platform.rs:434`) is fully
written but **inert** because its `for_each`/`size`/`read_chunk` return empty.

## Group C — safety-critical stubs that compile but do nothing 🔴

| # | Symbol | Location | Problem |
|---|---|---|---|
| C1 | `raw_ssr_off()` | `platform.rs:90` | **Empty body.** Called from the panic handler (`main.rs:324`) to de-energise the SSR on a crash. Currently does NOT drive the GPIO low → the relay can stay ON through a panic. Needs a `SIO.gpio_out_clr` register poke. |
| C2 | `raw_watchdog_feed()` | `platform.rs:80` | Reads a dummy volatile; does NOT poke `WATCHDOG.load`. RAM-resident feed used during the flash-park (`flash_handshake.rs`). Without it the watchdog trips mid-flash-write. |

These are the de-energise-on-failure path — highest priority despite being small.

## Group D — stub task bodies (loop, but do no work)

| # | Task | Location | Missing vs Python |
|---|---|---|---|
| D1 | `wifi_monitor_task` | `platform.rs:402` | Just a `Timer`. No link-status check / disconnect→wait 2 s→re-join. `wifi_manager.py:139-188` (`monitor`) |
| D2 | `ntp_task` | `platform.rs:527` | Just `Timer(3600 s)`. No `sntpc` UDP exchange → `set_unix_ms` never called → wall clock never syncs → CSV/recovery timestamps stay 0. `wifi_manager.py:42-66` (`sync_time_ntp`) |

## Group E — LCD entirely unported (optional / deferred, audit U9)

| Missing piece | Location / source |
|---|---|
| `LcdDisplay::show()` empty stub | `platform.rs:359` |
| HD44780/PCF8574 4-bit I²C driver | `lib/lcd1602_i2c.py` (whole file) |
| Manager loop: 2-line format, 5 s cadence, 300 s periodic HW reset, consecutive-error backoff + auto-disable, I²C scan, 500 ms init timeout | `server/lcd_manager.py` (whole file) |
| `lcd_task` not spawned | `main.rs:312-314` `TODO(LCD)` |

---

## Storage plan (decided)

The `Storage` trait is file-shaped, but the data splits cleanly by access
pattern. **Per-data-type backend, behind the one `FlashStorage` facade:**

| Data | Pattern | Backend |
|---|---|---|
| `config.json` | 1 small blob, rewrite | **sequential-storage `map`** (pure Rust) |
| `profiles/*.json` | named blobs, **write-once**, list, delete | **sequential-storage `map`**, key = name |
| `logs/*.csv` | named, **append-grown**, list, delete, crash-tail-read, **format-locked** (consumed by `scripts/plot_run.py`, `analyze_*`) | **littlefs2** (real files; needs ARM C toolchain) |

Rationale: config + profiles are key-value — no filesystem needed. Logs require
true named append-grown files whose `.csv` byte format must stay compatible with
the host analysis scripts; littlefs2 is its exact use case. Building a "file
index" on a KV store would reinvent a worse, untested filesystem — rejected.

`embassy_rp::flash::Flash` implements `embedded-storage` `NorFlash`, so both
backends consume it directly. The existing `flash_handshake` (Core-1 SSR-off +
RAM-park during XIP-down writes) applies to either backend.

### Blocker

littlefs2-sys compiles bundled C `littlefs`, needing `arm-none-eabi-gcc` +
freestanding libc headers. This machine has **neither**, so the littlefs2 logs
backend cannot cross-compile here until the toolchain is installed. The
sequential-storage (config + profiles) half is pure Rust and builds now.

---

## Priority order

1. **C1 + C2** — safety pokes; small, no deps.
2. **A1 `build_kiln_io`** — nothing reads temperature without it (drivers already done in `kiln-hal`).
3. **B (config + profiles)** — sequential-storage `map`; unblocks config override + profile upload/run/recovery. Pure Rust, verifiable now.
4. **B (logs)** — littlefs2 `append`/`read_chunk`/`for_each`/`remove`; gated on the ARM toolchain.
5. **A2 `init_network`** + **D2 `ntp_task`** + **D1 `wifi_monitor_task`** — network reachability + timestamps.
6. **E LCD** — optional, last.
