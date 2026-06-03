# kiln-core

Hardware-free control logic for the pico-kiln controller — the safe, testable
heart of a planned MicroPython → Rust migration.

- **`#![no_std]`, zero dependencies.** The same code runs on the RP2350 and
  under `cargo test` on your laptop.
- **Equivalence-first.** Each module is validated against golden data captured
  from the original MicroPython implementation *before* any hardware is touched.

## What's here today

Every hardware-free module of the MicroPython controller is ported and
equivalence-tested:

| Module | Ports | Equivalence check |
|--------|-------|-------------------|
| `pid` | `kiln/pid.py` — PID with conditional-integration anti-windup | golden replay (240 samples) |
| `rate_monitor` | `kiln/rate_monitor.py` — `TempHistory` ring buffer + rate | golden replay (overflow + clear) |
| `scheduler` | `kiln/scheduler.py` — delayed-start queue (generic payload) | unit tests |
| `profile` | `kiln/profile.py` — step model + duration/progress | golden replay (5 profiles) |
| `state` | `kiln/state.py` — firing state machine | golden replay (run / stall / recovery) |
| `tuner` | `kiln/tuner.py` — Ziegler-Nichols auto-tuner (4 modes) | golden replay (safe / standard / error) |

The concurrency layer (`comms.py` queues, `_thread`) is intentionally **not**
here — it maps to `embassy-sync` channels in the firmware crate, not to portable
logic.

## Test it

```bash
cd rust
cargo test -p kiln-core
```

30 unit tests plus 9 golden-replay tests. Each `replay_*.rs` feeds inputs
captured from the **real** MicroPython module back through the Rust port and
asserts the outputs match within `1e-6`. For example `replay_pid.rs` replays
`tests/fixtures/pid_golden.csv` — 240 samples spanning ramp / hold / step-down
(186 saturated, 112 negative-error) — checking every `output`, `p_term`,
`i_term`, `d_term`, and the `integral_frozen` flag.

> No system C linker? See `../TESTING.md §5` for the static-musl + `rust-lld`
> recipe that runs the whole suite with only the Rust toolchain.

## Regenerate the golden fixtures

Each module has a generator under `tools/` that imports the **real** Python
module (via a `micropython` shim / stubbed `kiln` package where needed) and
drives it through a deterministic scenario:

```bash
python3 rust/kiln-core/tools/gen_pid_golden.py
python3 rust/kiln-core/tools/gen_rate_golden.py
python3 rust/kiln-core/tools/gen_profile_golden.py
python3 rust/kiln-core/tools/gen_state_golden.py
python3 rust/kiln-core/tools/gen_tuner_golden.py
```

If you change a `kiln/*.py` module, regenerate its fixture and re-run the tests
to keep the port honest.

## Why this layering

`kiln-core` deliberately knows nothing about SPI, GPIO, WiFi, or allocation.
Those live in higher layers (`kiln-hal`, `firmware`) added later. Keeping the
control math pure is what lets us prove it correct off-device — the single
biggest risk-reducer for a safety-critical, fire-capable controller.
