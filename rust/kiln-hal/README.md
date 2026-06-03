# kiln-hal

Device drivers for the pico-kiln controller — the thin I/O layer between
`kiln-core`'s decisions and the physical kiln.

- **`#![no_std]`, one dependency** (`embedded-hal` 1.0). Drivers are generic over
  `SpiDevice` / `OutputPin`, so the same code runs on the RP2350 and against mock
  buses under `cargo test`.
- **I/O only.** Every control decision — temperature filtering, SSR duty
  scheduling, state — stays in `kiln-core`. These drivers just move bytes and
  flip pins.

## What's here

| Module | Drives | Ports |
|--------|--------|-------|
| `max31856` | MAX31856 thermocouple amplifier (SPI): config, continuous auto-conversion, 19-bit temperature, fault register | the Adafruit MAX31856 driver `kiln/hardware.py` relies on |
| `ssr` | a solid-state relay (GPIO), active-high, with an off-on-drop safety guard | the pin actuation in `kiln/hardware.py:SSRController` |

The `max31856` register map, init sequence (assert on all faults + open-circuit
detection), continuous-conversion flow (set the mains notch + hardware averaging,
then `start_autoconverting`; reads are non-blocking and return `0.0` until the
first conversion settles), and temperature unpack (`LSB = 2^-7 °C`) mirror the
Adafruit library exactly, so the Rust readings match what the MicroPython
controller was calibrated against after the thermocouple-filtering rework.

## Safety posture

`Ssr::new` drives the pin **low** (relay off) before anything else, and a `Drop`
guard drives it low again — a dropped/panicked/torn-down `Ssr` de-energises the
kiln. The time-proportional duty cycle, the locked duty, and the minimum on-time
floor are *not* here; they are pure logic in `kiln-core::ssr_schedule`, which
decides the on/off this driver applies.

## Test it

```bash
cd rust
cargo test -p kiln-hal
```

12 tests using `embedded-hal-mock`: they assert the exact SPI transactions the
driver issues (address byte then read/write, single chip-select per access),
decode known positive/negative temperatures and fault bits, check
`start_autoconverting` preserves the other config bits and that reads are `0.0`
before the first conversion, validate the `MAINS_FREQUENCY` / `THERMOCOUPLE_AVERAGING`
config mapping (and its fallback to the kiln defaults) down to the AVGSEL / notch
register writes, and verify the SSR starts low and the drop guard fires.

> No system C linker? See `../TESTING.md §5` for the static-musl + `rust-lld`
> recipe that runs the suite with only the Rust toolchain.

## Cross-compile for the RP2350

```bash
cargo build -p kiln-hal --lib --target thumbv8m.main-none-eabihf
```

`embedded-hal-mock` is a dev-dependency only, so it never reaches the firmware
build.

## Where this sits

See `../ARCHITECTURE.md`. `kiln-hal` depends only on `embedded-hal`; the firmware
composes it with `kiln-core` (wrapping the raw thermocouple reading in the core
filter, feeding the core's SSR schedule into this driver). Concrete RP2350
peripherals are bound in `kiln-control` / `kiln-firmware`, not here.
