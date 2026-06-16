# pico-kiln firmware (Rust + Embassy)

The current/primary firmware for the kiln controller, targeting the Raspberry Pi
Pico 2 W (RP2350, dual Cortex-M33). Bare-metal, `no_std`, async via
[Embassy](https://embassy.dev/).

This is the practical entry point. For the full design and the Rust↔MicroPython
mapping see **[ARCHITECTURE.md](ARCHITECTURE.md)**; for the host test strategy see
**[TESTING.md](TESTING.md)**.

## Quick build & flash

```bash
cd rust
./scripts/deploy.sh           # build + flash a Pico in BOOTSEL mode
# or, if you just want the image:
./scripts/compile.sh          # produces kiln-firmware/target/.../release/kiln-firmware.uf2
```

Releases ship two images: `kiln-firmware.uf2` (plain `compile.sh`) and
`kiln-firmware-debug.uf2` (`compile.sh --debug` — the same release profile plus
the `stack-debug` feature: boot stack painting + periodic high-water logging).

To flash by hand: hold **BOOTSEL** while plugging in USB, then drop the `.uf2`
onto the `RPI-RP2` drive. Reflashing **preserves** the device's config, profiles,
and logs (they live in a separate flash partition — see [Memory layout](#memory-layout)).

First boot has no WiFi credentials; set them over USB or the setup AP — see
[ARCHITECTURE.md §6 (Provisioning)](ARCHITECTURE.md#6-provisioning-usb-ncm--fallback-softap).

### Prerequisites

| Need | Why | Install |
|------|-----|---------|
| Rust **nightly** | `kiln-app` uses TAIT (`impl-trait-in-assoc-type`) | `rustup toolchain install nightly` |
| `thumbv8m.main-none-eabihf` target | the RP2350 Cortex-M33 | `rustup target add thumbv8m.main-none-eabihf` |
| `picotool` | pack/flash the UF2 | `brew install picotool` |
| `arm-none-eabi-gcc` **with newlib** | `littlefs2-sys` compiles bundled C | [Arm GNU Toolchain](https://developer.arm.com/downloads/-/arm-gnu-toolchain-downloads) (Homebrew's formula ships no libc) |

`compile.sh` auto-finds the Arm toolchain; override with `ARM_GCC=/path/to/arm-none-eabi-gcc`.

## Architecture at a glance

Five shipping crates + one optional host sim. Dependencies point **inward only**;
`kiln-core` depends on nothing.

| Crate | Role |
|-------|------|
| `kiln-core` | Pure control logic + protocol types — `no_std`, zero deps, host-testable |
| `kiln-hal` | Device drivers over `embedded-hal` (MAX31856, SSR, LCD) + the `Platform` traits |
| `kiln-control` | **Core 1** real-time safety loop (read → filter → PID → SSR → watchdog) |
| `kiln-app` | **Core 0** app layer: web server, WiFi, CSV logging, LCD |
| `kiln-firmware` | The only RP2350-aware crate: init, build/inject drivers, dispatch the cores |
| `kiln-sim` | Optional `std` host harness — drives the *real* `kiln-control` loop vs a thermal model |

The two halves (`kiln-control`, `kiln-app`) are generic over the *platform*, not
the chip; only `kiln-firmware` names `embassy-rp`/`cyw43`. See ARCHITECTURE.md §1.

## Core isolation

The one invariant in a fire-capable controller: **the real-time safety loop
cannot be starved or crashed by the application layer.** It's enforced at
compile time, not by convention.

- **Core 1** runs `kiln-control` — the safety loop. Its `Cargo.toml` does not list
  `picoserve`/`cyw43`/`embassy-net`/`embassy-rp`, so the network stack physically
  cannot be pulled into it.
- **Core 0** runs `kiln-app` — web/WiFi/logging, all best-effort.
- The only thing crossing the boundary is `kiln-core::protocol` over an
  `embassy-sync` `Channel`/`Watch` (must be `CriticalSectionRawMutex` — cross-core).
- **Backstop:** `kiln-control` feeds the hardware watchdog; if the safety loop
  hangs the chip resets. `panic = "abort"` + an SSR-off-on-`Drop` guard ensure any
  fault de-energises the kiln.

Core *affinity* is decided once, at dispatch time, in `kiln-firmware`
(`spawn_core1`) — crate names describe responsibility, not core number.

## Memory layout

RP2350A: 4 MiB external QSPI flash (XIP at `0x1000_0000`), 520 KiB on-chip SRAM.
Defined in [`kiln-firmware/memory.x`](kiln-firmware/memory.x).

```
Flash (4 MiB @ 0x1000_0000)
├─ 0x000000 .. 0x280000   program image (2560 KiB)   ← linker FLASH region
│                          .start_block / .end_block IMAGE_DEF for the bootrom
└─ 0x280000 .. 0x400000   littlefs2 partition (1536 KiB)  ← config.json, profiles, run logs
                          (see platform.rs FS_BASE/FS_SIZE; preserved across reflash)

RAM (512 KiB @ 0x2000_0000)
├─ Core 1 stack: 16 KiB static (CORE1_STACK in main.rs)
└─ .data + .bss: statics, channels, executor pools, network buffers
```

Capping `FLASH` at 2560 KiB guarantees the image can never grow into the
filesystem region. Measure real RAM use with [`scripts/ram.sh`](#scripts).

## Scripts

In `rust/scripts/` (run from `rust/`):

| Script | Does |
|--------|------|
| `compile.sh` | `cargo build` (release) → ELF → UF2. `--debug` adds the `stack-debug` feature (same release profile + stack painting/high-water logging — **not** the dev profile). |
| `deploy.sh` | `compile.sh` then `picotool load -x` over USB (Pico in BOOTSEL). `--no-build` flashes an existing UF2. Preserves the littlefs partition. |
| `ram.sh` | Static RAM (`.data`+`.bss`) vs the 512 KiB budget + largest symbols. `--types` dumps the biggest type/future sizes. |
| `watch_faults.sh` | Polls the live device for captured faults (panic/hardfault/stack-overflow trap) and per-core stack high-water. |

## Testing

`kiln-core` and `kiln-hal` are host-testable directly (`cargo test`); `kiln-sim`
runs the real control loop on a laptop against a thermal model. Full recipe —
including the no-system-linker musl path and the MicroPython equivalence
fixtures — in **[TESTING.md](TESTING.md)**.

## Configuration

`kiln-firmware` reads `config.json` from the device's littlefs root at boot,
falling back to defaults on any error. Keys mirror the MicroPython
`config.example.py` UPPER_SNAKE names; see
[`config.example.json`](config.example.json). Edit live via `GET`/`POST`/`PATCH
/api/config` (changes persist to flash, apply on reboot).
