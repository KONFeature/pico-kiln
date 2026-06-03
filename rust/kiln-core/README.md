# kiln-core

Hardware-free control logic for the pico-kiln controller — the safe, testable
heart of a planned MicroPython → Rust migration.

- **`#![no_std]`, zero dependencies.** The same code runs on the RP2350 and
  under `cargo test` on your laptop.
- **Equivalence-first.** Each module is validated against golden data captured
  from the original MicroPython implementation *before* any hardware is touched.

## What's here today

| Module | Ports | Status |
|--------|-------|--------|
| `pid`  | `kiln/pid.py` — PID with conditional-integration anti-windup | ✅ ported + replay-tested |

Planned next: `state` (state machine), `profile`, `rate_monitor`, `tuner`.

## Test it

```bash
cd rust
cargo test -p kiln-core
```

The headline test, `replay_pid.rs`, replays
`tests/fixtures/pid_golden.csv` — 240 samples spanning ramp / hold / step-down
(186 saturated, 112 negative-error) — through the Rust `Pid` and asserts every
`output`, `p_term`, `i_term`, `d_term`, and the `integral_frozen` flag match the
reference within `1e-6`.

## Regenerate the golden fixture

The fixture is produced by importing the **real** `kiln/pid.py` (via a tiny
`micropython` shim) and driving it through a deterministic scenario:

```bash
python3 rust/kiln-core/tools/gen_pid_golden.py
```

If you change `kiln/pid.py`, regenerate the fixture and re-run the test to keep
the port honest.

## Why this layering

`kiln-core` deliberately knows nothing about SPI, GPIO, WiFi, or allocation.
Those live in higher layers (`kiln-hal`, `firmware`) added later. Keeping the
control math pure is what lets us prove it correct off-device — the single
biggest risk-reducer for a safety-critical, fire-capable controller.
