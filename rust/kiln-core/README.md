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
| `temp_filter` | `kiln/hardware.py` — median spike-rejection + fault tolerance (the software half of `TemperatureSensor`) | golden replay (init / spike / faults / shutdown) |
| `ssr_schedule` | `kiln/hardware.py` — time-proportional SSR duty: min on-time floor, mid-cycle duty lock, single-cycle advance (the decision half of `SSRController`) | golden replay (floor / lock / on-off / fall-behind / force-off) |
| `gain_schedule` | `kiln/control_thread.py` — continuous gain scaling `g(T)=1+h·(T−T_ambient)` + change-threshold gate | unit tests |
| `protocol` | `kiln/comms.py` — `MessageType`/`CommandMessage` → `enum Command`, `StatusMessage` → typed `Status` | unit tests |

The concurrency layer (`comms.py` queues, `_thread`) is intentionally **not**
here — it maps to `embassy-sync` channels in the firmware crate. Only the message
*shapes* live here as `protocol`. The chip-side filtering (continuous conversion,
hardware averaging, mains notch) lives in `kiln-hal`; `temp_filter` is only the
spike-rejection + fault logic that wraps it, and `ssr_schedule` is the duty
decision that `kiln-hal::ssr` actuates.

## Test it

```bash
cd rust
cargo test -p kiln-core
```

54 unit tests plus 11 golden-replay tests. Each `replay_*.rs` feeds inputs
captured from the **real** MicroPython module back through the Rust port and
asserts the outputs match within a tight tolerance. For example `replay_pid.rs`
replays `tests/fixtures/pid_golden.csv` — 240 samples spanning ramp / hold /
step-down (186 saturated, 112 negative-error) — checking every `output`,
`p_term`, `i_term`, `d_term`, and the `integral_frozen` flag; `replay_temp_filter.rs`
replays a 40-step script through the median filter, exercising spike rejection,
last-good recovery, the cold→hot fault budget, and both fatal errors;
`replay_ssr_schedule.rs` replays a 26-step `set_output`/`update`/`force_off`
script through the time-proportional scheduler, pinning the min on-time floor,
the mid-cycle duty lock, and the single-cycle-advance "fall behind" quirk.

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
python3 rust/kiln-core/tools/gen_temp_filter_golden.py
python3 rust/kiln-core/tools/gen_ssr_schedule_golden.py
```

(`gain_schedule` and `protocol` are pure formula / data-shape ports with no
numeric reference trace, so they are covered by unit tests rather than a golden
fixture — same as `scheduler`.)

If you change a `kiln/*.py` module, regenerate its fixture and re-run the tests
to keep the port honest.

## Why this layering

`kiln-core` deliberately knows nothing about SPI, GPIO, WiFi, or allocation.
Those live in higher layers (`kiln-hal`, `firmware`) added later. Keeping the
control math pure is what lets us prove it correct off-device — the single
biggest risk-reducer for a safety-critical, fire-capable controller.
