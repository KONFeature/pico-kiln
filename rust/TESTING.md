# Testing & verifying `kiln-core`

This document explains how to build, test, cross-compile, and (re)validate the
Rust control-logic crate against the original MicroPython implementation.

> **Current status (verified):** `kiln-core` compiles `no_std` for the RP2350
> (`thumbv8m.main-none-eabihf`) and the full test suite is **green — 39/39**
> (30 unit tests + 9 reference-replay tests) across all six ported modules:
> `pid`, `rate_monitor`, `scheduler`, `profile`, `state`, `tuner`. See
> [Results](#results-snapshot).

---

## 1. TL;DR

```bash
cd rust

# Standard run (needs a normal host C toolchain: gcc/clang + libc-dev)
cargo test

# Prove it builds for the actual Pico 2 chip (no host linker needed)
cargo build -p kiln-core --lib --target thumbv8m.main-none-eabihf

# Regenerate the golden fixture from the REAL kiln/pid.py
python3 kiln-core/tools/gen_pid_golden.py
```

If `cargo test` fails to **link** with an error mentioning `cc`, `crt1.o`, or
`-lc`, your box has no system C toolchain — jump to
[§5 No system linker](#5-running-tests-without-a-system-linker).

---

## 2. Prerequisites

| Need | For | Install |
|------|-----|---------|
| Rust stable (`cargo`, `rustc`) | everything | <https://rustup.rs> |
| A host linker (`gcc`/`clang` + libc headers) | running `cargo test` natively | `apt install build-essential` (Debian) |
| `thumbv8m.main-none-eabihf` target | cross-compiling for RP2350 | `rustup target add thumbv8m.main-none-eabihf` |
| Python 3 | regenerating the golden fixture | system Python (no pip packages needed) |

The crate itself has **zero Cargo dependencies** — nothing to download to build
or test the logic.

---

## 3. Running the test suite

```bash
cd rust
cargo test -p kiln-core
```

You should see three test binaries run:

- **unit tests** (`src/lib.rs`) — focused behaviour checks
- **`replay_pid`** (`tests/replay_pid.rs`) — the equivalence test (see §4)
- **doc-tests** — currently none

Run a single test with the usual filter, e.g.:

```bash
cargo test -p kiln-core replay_matches_reference_pid -- --nocapture
```

---

## 4. What the tests actually prove

The headline guarantee is **behavioural equivalence with the production
MicroPython PID**, established without any hardware:

```
kiln/pid.py  ──(tools/gen_pid_golden.py)──►  tests/fixtures/pid_golden.csv
                                                      │
                                                      ▼
                                   tests/replay_pid.rs replays the SAME
                                   (setpoint, measured, time) inputs through
                                   the Rust `Pid` and asserts every output
                                   matches within 1e-6.
```

- `tools/gen_pid_golden.py` imports the **real** `kiln/pid.py` unchanged (via a
  tiny in-memory `micropython` shim) and drives it through a deterministic
  scenario: a 1.5 °C/s ramp to 200 °C, an 80 s hold, then a step **down** to
  120 °C. This exercises proportional, integral, derivative, output saturation,
  and the conditional-integration **anti-windup** (both freeze directions).
- The fixture records `output`, `p_term`, `i_term`, `d_term` and the
  `integral_frozen` flag per step, with floats written via Python `repr()` so
  they round-trip **bit-exactly** through `f64` parsing.
- `replay_pid.rs` asserts each of those fields matches and also asserts the
  fixture is *meaningful* (>50 saturated samples, >20 negative-error samples) so
  the test can't silently pass on a flat curve.

The Rust `update()` mirrors the Python expression-for-expression and
left-to-right, so with identical `f64` inputs the results agree to a few ULP —
far inside the `1e-6` tolerance.

> **Float note.** The golden data is generated with CPython doubles (`f64`). The
> Rust port also uses `f64`, so the host test is an exact apples-to-apples
> check of the *algorithm*. On-device MicroPython may be built single-precision;
> that's a runtime-precision question, separate from this logic-equivalence
> proof.

---

## 5. Running tests without a system linker

Some sandboxes (like the one this was scaffolded in) have **no `cc`/`ld` and no
`libc6-dev`**, and no root to install them — so a normal `std` host test binary
can't be linked. You can still run the **entire suite** using Rust's bundled
`rust-lld` and a **statically-linked musl** target, which ships its own libc and
CRT objects (nothing from the system needed):

```bash
cd rust
rustup target add x86_64-unknown-linux-musl

# Put the toolchain's bundled rust-lld on PATH
export PATH="$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin:$PATH"

RUSTFLAGS="-C linker=rust-lld -C link-self-contained=yes" \
  cargo test --target x86_64-unknown-linux-musl
```

This is exactly how the current green result below was produced. It needs only
the Rust toolchain — no apt, no sudo, no system C library. The resulting test
binary is a self-contained static executable that runs on any Linux host.

> Tip for CI on minimal images: the simplest fix is usually
> `apt-get install -y build-essential` so plain `cargo test` works. Use the musl
> recipe when you can't install system packages.

---

## 6. Cross-compiling for the RP2350 (Pico 2 / Pico 2 W)

Proving the *same* code compiles for the target chip needs **no host linker**
(building a library produces an `.rlib`, not a linked executable):

```bash
cd rust
rustup target add thumbv8m.main-none-eabihf
cargo build -p kiln-core --lib --target thumbv8m.main-none-eabihf
```

- `thumbv8m.main-none-eabihf` is the Arm Cortex-M33 (hard-float) target for the
  RP2350. (A RISC-V `riscv32imac-unknown-none-elf` build will be relevant once
  we choose a core; the logic crate is core-agnostic.)
- This confirms the `#![no_std]`, allocation-free design holds: if it builds
  here, it builds on the device.

A full firmware *binary* (later, in a `firmware` crate) will additionally need
`flip-link`, `probe-rs`, and the cyw43 firmware blob — out of scope for the
logic crate.

---

## 7. Regenerating the golden fixture

Re-run this whenever you touch `kiln/pid.py` (or want a different scenario):

```bash
python3 rust/kiln-core/tools/gen_pid_golden.py
```

It prints a summary, e.g.:

```
wrote 240 rows -> rust/kiln-core/tests/fixtures/pid_golden.csv
  reference: kiln/pid.py
  saturated samples (out at 0 or 100): 186
  negative-error samples: 112
  output range: 0.0000 .. 100.0000
```

Then re-run the test suite. If `kiln/pid.py` changed behaviour, the fixture
changes and `replay_pid.rs` will (correctly) flag any divergence in the port.

The generator is dependency-free and reads `kiln/pid.py` by **file path**, so it
never triggers `kiln/__init__.py` (which imports hardware-only modules).

---

## 8. Interpreting failures

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| `cannot read fixture …pid_golden.csv` | fixture not generated | run the generator (§7) |
| link error: `cc`/`crt1.o`/`-lc` not found | no host C toolchain | install `build-essential`, or use §5 |
| `row N output: rust=… ref=…` assertion | port diverged from `kiln/pid.py`, or fixture stale | re-gen fixture (§7); if still failing, the Rust `update()` no longer mirrors the Python math |
| `row N integral_frozen` assertion | anti-windup branch differs | compare the `saturated_high/low` logic in `pid.rs` vs `pid.py` |

The tolerance lives at the top of `tests/replay_pid.rs` (`const TOL: f64 =
1e-6`). It should not need loosening; if it does, that's a signal the arithmetic
order drifted from the reference.

---

## 9. Layout

```
rust/
├── Cargo.toml                      # workspace (release profile tuned for size)
├── TESTING.md                      # this file
└── kiln-core/
    ├── Cargo.toml                  # zero-dependency, no_std lib
    ├── README.md
    ├── src/                        # one module per ported kiln/*.py
    │   ├── lib.rs                  # #![no_std], re-exports
    │   ├── pid.rs
    │   ├── rate_monitor.rs
    │   ├── scheduler.rs
    │   ├── profile.rs
    │   ├── state.rs
    │   └── tuner.rs
    ├── tests/
    │   ├── replay_pid.rs           # equivalence tests (replay fixtures)
    │   ├── replay_rate.rs
    │   ├── replay_profile.rs
    │   ├── replay_state.rs         # run / stall / recovery scenarios
    │   ├── replay_tuner.rs         # safe / standard / error scenarios
    │   └── fixtures/               # *_golden.csv generated from kiln/*.py
    └── tools/
        └── gen_*_golden.py         # fixture generators (import the real modules)
```

---

## 10. Status & adding the next equivalence test

All hardware-free modules are ported and equivalence-tested:

1. ✅ `pid` (`kiln/pid.py`) — PID with anti-windup
2. ✅ `rate_monitor` (`kiln/rate_monitor.py`) — rolling temp/rate window
3. ✅ `scheduler` (`kiln/scheduler.py`) — delayed-start queue
4. ✅ `profile` (`kiln/profile.py`) — step model + duration/progress
5. ✅ `state` (`kiln/state.py`) — firing state machine (ramp/hold/cooling,
   stall detection, recovery)
6. ✅ `tuner` (`kiln/tuner.py`) — Ziegler-Nichols auto-tune sequences

What's intentionally **not** in `kiln-core`: the concurrency layer (`comms.py`
queues, `_thread`) and anything touching hardware — those belong to the future
`kiln-hal` / `firmware` crates and map to `embassy` primitives.

**Pattern for each module (for reference / future tweaks):**

1. Write `tools/gen_<mod>_golden.py` that imports the real Python module (by file
   path, with a `micropython` shim / stubbed `kiln` package and a patched clock
   where needed) and emits a fixture covering the important branches.
2. Port the logic into `kiln-core/src/<mod>.rs` (`no_std`, inject time, mirror the
   math/branch order; manual `clamp`/`abs`, no float intrinsics; strings kept out
   of the core).
3. Add `tests/replay_<mod>.rs` that replays the fixture and asserts equivalence.
4. Verify with `cargo test` (or the §5 recipe) and the §6 cross-build.

This keeps every step provably faithful to the battle-tested MicroPython code —
the single biggest risk-reducer for a fire-capable controller.

---

## Results snapshot

Produced with the §5 recipe (this environment has no system linker):

```
running 30 tests        # src/lib.rs unit tests
... pid::tests (6) ... rate_monitor::tests (4) ... scheduler::tests (5) ...
... profile::tests (5) ... state::tests (5) ... tuner::tests (5) ...
test result: ok. 30 passed; 0 failed; ...

   Running tests/replay_pid.rs       test result: ok. 1 passed; ...
   Running tests/replay_rate.rs      test result: ok. 1 passed; ...
   Running tests/replay_profile.rs   test result: ok. 1 passed; ...
   Running tests/replay_state.rs     test result: ok. 3 passed; ...  # run/stall/recovery
   Running tests/replay_tuner.rs     test result: ok. 3 passed; ...  # safe/standard/error
```

Total: 30 unit + 9 replay = 39 tests green.

Cross-compile for RP2350:

```
$ cargo build -p kiln-core --lib --target thumbv8m.main-none-eabihf
    Finished `dev` profile [unoptimized + debuginfo] target(s)
```
