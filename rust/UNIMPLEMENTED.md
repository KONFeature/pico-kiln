# pico-kiln Rust port — Unimplemented inventory

**Date:** 2026-06-03
**Branch:** `feat/rust-kiln-core`
**Scope:** every stub / `unimplemented!()` / inert body remaining in the Rust port.

> **Update 2026-06-03 — Group B (flash storage) is DONE.** `FlashStorage` is now
> a real littlefs2 mount over the reserved top 1.5 MiB of flash (config +
> profiles + logs), cross-compiles clean for `thumbv8m`, and is clippy-clean. The
> ARM toolchain blocker is resolved (see Storage plan). Groups **A, C, D, E**
> remain.

## TL;DR

All missing code lives in **one crate: `kiln-firmware`** (`src/platform.rs` +
`src/main.rs`). The logic crates (`kiln-core`, `kiln-control`, `kiln-app`,
`kiln-hal`) are complete and host-tested — `grep` for `unimplemented!`/`todo!`
finds nothing in them. What remains is **device-I/O wiring**, not business logic:
drivers (SPI/GPIO/cyw43/LCD), the network/NTP bring-up, and two RAM-resident
safety pokes. (The flash filesystem — Group B — is now implemented.)

The `kiln-app` web/CSV/recovery/config consumers are all written against the
`Storage` / `Clock` / `Display` traits (`kiln-app/src/server.rs:54-117`), so each
remaining backend slots in behind a trait without touching any consumer — as the
now-complete `FlashStorage` (Group B) did.

---

## Group A — hard `unimplemented!()` (panics at runtime)

| # | Symbol | Location | What it must do | Python source |
|---|---|---|---|---|
| A1 | `build_kiln_io()` | `platform.rs:144` | Construct SPI1 + CS + `Max31856` + `Ssr` pin(s) + `MaybeWatchdog` from `Core1Periphs` + cfg. Sequence: `init()` → `set_averaging()` → `set_noise_filter()` → **`start_autoconverting()`** (mandatory — without it the sensor reads a constant 0 °C). | `hardware.py:69-84` (`TemperatureSensor.__init__`) + `hardware.py:223-256` (`SSRController.__init__`) |
| A2 | `init_network()` | `platform.rs:618` | cyw43 firmware-blob load + PIO SPI + `embassy_net::Stack` + DHCP + `join_wpa2` retry loop. | `wifi_manager.py:68-137` (`connect`) |

These two **panic the firmware** the moment Core 1 / Core 0 reach them. Nothing
boots without them.

## Group B — `FlashStorage` flash filesystem ✅ DONE

All of `impl Storage for FlashStorage` (`read_chunk`/`size`/`for_each`/`append`/
`remove`/`remove_all`/`upload_*`/`read_config`/`write_config`) is now backed by a
real **littlefs2 0.7** mount over the reserved top 1.5 MiB partition — one engine
for `config.json`, `profiles/*.json`, and `logs/*.csv`. Each call mounts
(`mount_and_then`), runs the op, unmounts; write paths go through the existing
`flash_handshake`. `attempt_recovery()` is now live (its `for_each`/`size`/
`read_chunk` return real data); `config.json` loads (B8 fixed → no more silent
`KilnConfig::default()`). Cross-compiles clean for `thumbv8m`, clippy-clean.

littlefs keeps no mtime, so `for_each`'s `modified` is derived from the
timestamp the logger embeds in each filename (`filename_time_key`), preserving
recovery's "most recent log" ordering and the web file-list dates.

## Group C — safety-critical stubs that compile but do nothing 🔴

| # | Symbol | Location | Problem |
|---|---|---|---|
| C1 | `raw_ssr_off()` | `platform.rs:90` | **Empty body.** Called from the panic handler (`main.rs:324`) to de-energise the SSR on a crash. Currently does NOT drive the GPIO low → the relay can stay ON through a panic. Needs a `SIO.gpio_out_clr` register poke. |
| C2 | `raw_watchdog_feed()` | `platform.rs:80` | Reads a dummy volatile; does NOT poke `WATCHDOG.load`. RAM-resident feed used during the flash-park (`flash_handshake.rs`). Without it the watchdog trips mid-flash-write. |

These are the de-energise-on-failure path — highest priority despite being small.

## Group D — stub task bodies (loop, but do no work)

| # | Task | Location | Missing vs Python |
|---|---|---|---|
| D1 | `wifi_monitor_task` | `platform.rs:632` | Just a `Timer`. No link-status check / disconnect→wait 2 s→re-join. `wifi_manager.py:139-188` (`monitor`) |
| D2 | `ntp_task` | `platform.rs:780` | Just `Timer(3600 s)`. No `sntpc` UDP exchange → `set_unix_ms` never called → wall clock never syncs → CSV/recovery timestamps stay 0. `wifi_manager.py:42-66` (`sync_time_ntp`) |

## Group E — LCD entirely unported (optional / deferred, audit U9)

| Missing piece | Location / source |
|---|---|
| `LcdDisplay::show()` empty stub | `platform.rs:589` |
| HD44780/PCF8574 4-bit I²C driver | `lib/lcd1602_i2c.py` (whole file) |
| Manager loop: 2-line format, 5 s cadence, 300 s periodic HW reset, consecutive-error backoff + auto-disable, I²C scan, 500 ms init timeout | `server/lcd_manager.py` (whole file) |
| `lcd_task` not spawned | `main.rs:312-314` `TODO(LCD)` |

---

## Storage plan (implemented)

Once the ARM C toolchain was fixed, the seq-storage-for-config/profiles split
lost its only rationale (avoiding the C dep), so **littlefs2 handles all three
data types** — simplest, one engine, fully synchronous (matches the `Storage`
trait directly), one flash region, named files, no per-profile size cap.

| Data | littlefs path | Notes |
|---|---|---|
| `config.json` | `/config.json` | temp-file + atomic rename on write; absent → `KilnConfig::default()` |
| `profiles/*.json` | `/profiles/<name>` | upload via scratch `upload.tmp` + rename |
| `logs/*.csv` | `/logs/<name>` | append-grown; `.csv` byte format unchanged (host `scripts/` compatible) |

Layout: `memory.x` caps the linker's FLASH at 2560 KiB; the top 1536 KiB
(offset `0x280000..0x400000`) is the littlefs partition (`FS_BASE`/`FS_SIZE` in
`platform.rs`). `LfsFlash` rebases every littlefs offset into it over embassy-rp's
blocking `Flash` (`READ/WRITE_SIZE=256`, `BLOCK_SIZE=4096`, `BLOCK_CYCLES=500`).
Writes run through `flash_handshake`; boot-time mount/format runs pre-split (Core 1
not yet alive, so no handshake).

### Blocker — RESOLVED

littlefs2-sys compiles bundled C `littlefs`, needing an `arm-none-eabi` GCC **with
a C library** (newlib `stdint.h`). The Homebrew `arm-none-eabi-gcc` formula ships
**no libc** (the "stdint.h: No such file" failure). Fixed by the ARM toolchain
(`gcc-arm-embedded` cask / Arm GNU Toolchain, which bundles newlib); point cc-rs
at it via `CC_thumbv8m_main_none_eabihf` if it is not first on PATH. See
`kiln-firmware/.cargo/config.toml`.

---

## Priority order (remaining)

1. **C1 + C2** — safety pokes; small, no deps.
2. **A1 `build_kiln_io`** — nothing reads temperature without it (drivers already done in `kiln-hal`).
3. **A2 `init_network`** + **D2 `ntp_task`** + **D1 `wifi_monitor_task`** — network reachability + timestamps.
4. **E LCD** — optional, last.

~~B (flash storage)~~ — ✅ done (this pass).
