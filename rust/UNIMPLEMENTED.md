# pico-kiln Rust port — Unimplemented inventory

**Date:** 2026-06-03
**Branch:** `feat/rust-kiln-core`
**Scope:** every stub / `unimplemented!()` / inert body remaining in the Rust port.

> **Update 2026-06-03 — ALL GROUPS DONE.** Every stub in this inventory is now
> implemented and cross-compiles clean (zero warnings) for `thumbv8m`,
> clippy-clean; the host workspace still passes 172 tests. Done this pass:
> **A1** `build_kiln_io` (+ a real SPI1→SPI0 wiring-bug fix), **A2**
> `init_network` (full cyw43 bring-up, blobs vendored), **D1**
> `wifi_monitor_task`, **D2** `ntp_task` (sntpc over UDP), **E** the LCD
> (1602 driver + manager + task). Groups **B** (flash storage) and **C**
> (safety pokes) were done earlier. There are **no `unimplemented!()` left in
> the firmware.** What remains is device-on-hardware validation, not code.

## TL;DR

All code in this inventory lived in **one crate: `kiln-firmware`**
(`src/platform.rs` + `src/main.rs`, plus the new `src/lcd.rs`). The logic crates
(`kiln-core`, `kiln-control`, `kiln-app`, `kiln-hal`) were already complete and
host-tested. As of this pass the firmware is **fully implemented** — `grep` for
`unimplemented!`/`todo!` finds nothing anywhere in `rust/`. The device-I/O wiring
(sensor/SSR builder, cyw43/WiFi/NTP bring-up, LCD) is done; what is left is
on-hardware validation (the parts marked `DEVICE`, only checkable on a Pico 2 W).

The `kiln-app` web/CSV/recovery/config consumers are all written against the
`Storage` / `Clock` / `Display` traits (`kiln-app/src/server.rs:54-117`), so each
remaining backend slots in behind a trait without touching any consumer — as the
now-complete `FlashStorage` (Group B) did.

---

## Group A — hard `unimplemented!()` ✅ DONE

| # | Symbol | Implementation |
|---|---|---|
| A1 | `build_kiln_io()` | Blocking **SPI0** (not SPI1 — the old wiring named `p.SPI1`, but pins 18/19/16 are SPI0 functions and Python uses `MAX31856_SPI_ID = 0`; fixed) + `ExclusiveDevice` CS → `Max31856` with `init` → `set_averaging` → `set_noise_filter` → **`start_autoconverting`** (mandatory), `Ssr` on PIN_15, `MaybeWatchdog` per `ENABLE_WATCHDOG`. The `DeviceSpi`/`DevicePin` placeholder enums became concrete type aliases. |
| A2 | `init_network()` | Full cyw43 0.7 bring-up: vendored firmware blobs (`cyw43-firmware/`, Permissive Binary License) via `aligned_bytes!`, PIO0 SPI (`RM2_CLOCK_DIVIDER`) + one DMA channel, `cyw43::new` (5-arg, with nvram), `cyw43_task`/`net_task` spawned, `control.init`/PowerSave, embassy-net DHCP stack (ROSC-seeded), join in a 2 s-backoff retry loop, `wait_config_up`. Returns `(Stack, Control)`. |

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

## Group C — safety register pokes ✅ DONE

| # | Symbol | Location | Implementation |
|---|---|---|---|
| C1 | `raw_ssr_off()` | `platform.rs:99` | Drives SSR GPIO 15 low: `str 1<<15 → SIO gpio_out(0).value_clr` (`0xd000_0020`) + `gpio_oe(0).value_set` (`0xd000_0038`) so it actively drives low even if OE glitched. |
| C2 | `raw_watchdog_feed()` | `platform.rs:84` | `str 0x00ff_ffff → WATCHDOG.load` (`0x400d_8004`) — max 24-bit reload so the flash-park can't trip a reset; harmless when the watchdog is disabled. |

Both addresses come from `rp_pac` `const fn as_ptr()` (compile-time constants);
the `write_volatile`s inline, so each function is a self-contained register poke.
**Verified by disassembly of the release ELF:** both sit at RAM VMAs
(`0x2000_0000` / `0x2000_0014`) in `.data.ram_func`, contain **no `bl`/`blx`**
(no flash calls), and emit exactly the stores above — so they are safe to run
with XIP down (C2 during the flash-park, C1 from the panic handler). Needed a new
`rp-pac` direct dep: embassy-rp's `pac` re-export is `pub(crate)`.

## Group D — stub task bodies ✅ DONE

| # | Task | Implementation |
|---|---|---|
| D1 | `wifi_monitor_task` | Parks on the embassy-net link (`wait_link_up`/`wait_link_down`), re-joins with a 2 s backoff on a hard failure, waits for DHCP — `wifi_manager.monitor`'s explicit reconnect that cyw43's drop-only auto-reconnect doesn't cover. Takes the `Control` handle threaded from `init_network`. |
| D2 | `ntp_task` | `sntpc` over an embassy-net UDP socket (added the embassy-net `dns` feature): resolve `pool.ntp.org` (Cloudflare anycast fallback), 10 s-bounded `get_time`, convert NTP→Unix (−2208988800), `NtpClock::set_unix_ms` — unblocking CSV/recovery timestamps. Hourly re-sync, 60 s retry until the first sync. Custom `NtpUdpSocket`/`NtpTimestampGenerator` adapters bridge the proto-ipv4-only smoltcp conversions. |

## Group E — LCD ✅ DONE

| Piece | Implementation |
|---|---|
| HD44780/PCF8574 4-bit I²C driver | `src/lcd.rs` — `Lcd1602<I>` generic over `embedded_hal::i2c::I2c`, port of `lib/lcd1602_i2c.py`. |
| `LcdDisplay::show()` + manager | `platform.rs` — implements the kiln-app `Display` trait; since `lcd_task` only calls `show()` per status change, the `lcd_manager.py` logic (5 s render throttle, 300 s periodic reset, disable-after-3-errors) lives in `show()`. Exact 2-line layout; state label reuses the web/CSV-canonical strings. |
| I²C build + presence check | `init_display` builds blocking I²C0 (SDA=PIN_20/SCL=PIN_21) and runs power-on init; a NACK (no backpack) → `None`, kiln runs headless. |
| `lcd_task` spawned | `main.rs` — new `LcdPeriphs`; `core0_main` spawns kiln-app's `lcd_task` when `LCD_ENABLED` and the device is present. |

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

## Status — all complete

~~A1 `build_kiln_io`~~ ✅ · ~~A2 `init_network`~~ ✅ · ~~B flash storage~~ ✅ ·
~~C1/C2 safety pokes~~ ✅ · ~~D1 `wifi_monitor_task`~~ ✅ · ~~D2 `ntp_task`~~ ✅ ·
~~E LCD~~ ✅

The firmware has **no `unimplemented!()` / `todo!()` left**. The remaining
caveat is unchanged from the start: the `DEVICE`-marked driver bodies
(cyw43/SPI/I²C register traffic, the sntpc exchange) compile and are
type-checked but can only be *behaviourally* validated on a physical Pico 2 W
with the sensor/SSR/LCD wired — there is no host emulation for them.
