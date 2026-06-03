#!/usr/bin/env python3
"""Generate golden SSR time-proportional traces from the REAL kiln/hardware.py.

`hardware.py`'s `SSRController` mixes GPIO actuation (`pin.value()`) with the pure
scheduling logic we port to `kiln-core::ssr_schedule`: the minimum on-time floor,
the mid-cycle duty lock, and the single-cycle-advance time-proportional window.
To exercise just that logic on CPython we:

  1. install a `micropython` shim (const/native),
  2. inject a fake `time` whose ticks_ms() reads a controllable global clock and
     whose ticks_diff/ticks_add are plain integer subtract/add (so the injected
     `now_ms` drives the schedule exactly like the Rust port's `now_ms`),
  3. inject a fake GPIO pin whose `.value()` we record,
  4. load hardware.py by file path and drive `SSRController.set_output()` /
     `.update()` / `.force_off()`.

Each step is one of:
  - `S,<percent>`   set_output(percent); record the resulting `duty_cycle`
  - `U,<now_ms>`    advance the clock and update(); record `is_on` (any pin high)
                    and the locked `duty_cycle_locked`
  - `F`             force_off(); record the locked duty (0)

The Rust replay feeds the same script through `SsrSchedule` and asserts the ON/OFF
decision and the locked duty match.

Run:  python3 rust/kiln-core/tools/gen_ssr_schedule_golden.py
"""

import csv
import importlib.util
import os
import sys
import types

THIS = os.path.abspath(__file__)
REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(THIS))))
SRC = os.path.join(REPO_ROOT, "kiln", "hardware.py")
OUT = os.path.join(REPO_ROOT, "rust", "kiln-core", "tests", "fixtures", "ssr_schedule_golden.csv")

CYCLE_TIME = 2.0  # seconds -> 2000 ms window
MIN_SSR_OUTPUT = 5.0  # mirror hardware.py, for coverage accounting only

# Controllable monotonic clock the fake `time` reads.
CLOCK = {"ms": 0}


class FakePin:
    """Stand-in for machine.Pin: records the last driven value."""

    def __init__(self):
        self.state = 0

    def value(self, v):
        self.state = v


def load_ssr_controller():
    mp = types.ModuleType("micropython")
    mp.native = lambda f: f
    mp.const = lambda x: x
    sys.modules["micropython"] = mp

    faketime = types.ModuleType("time")
    faketime.ticks_ms = lambda: CLOCK["ms"]
    faketime.ticks_diff = lambda a, b: a - b
    faketime.ticks_add = lambda a, b: a + b
    faketime.sleep = lambda *_: None
    faketime.sleep_ms = lambda *_: None
    sys.modules["time"] = faketime

    spec = importlib.util.spec_from_file_location("kiln_hardware_ref_ssr", SRC)
    m = importlib.util.module_from_spec(spec)
    # hardware.py uses the bare `@micropython.native` decorator but only does
    # `from micropython import const`; pre-bind the name so the class body runs.
    m.micropython = mp
    spec.loader.exec_module(m)
    return m.SSRController


# Scenario hitting every branch of the reference set_output()/update():
#   - duty floor (3% -> 5%) and clamp (150% -> 100%); exact 0 -> off
#   - mid-cycle lock: change the request mid-window, confirm the applied duty
#     does not move until the next boundary
#   - time-proportional ON then OFF inside one cycle
#   - the single-cycle advance "fall behind" quirk (jump many cycles ahead;
#     the boundary advances by ONE cycle only)
#   - force_off zeroes the locked duty
def build_script():
    s = []
    # Start idle: a few updates before any boundary stay off (locked 0).
    s += [("U", 0), ("U", 500), ("U", 1999)]
    # Floor a tiny request and lock it at the first boundary (on_time = 5% = 100 ms).
    s += [("S", 3.0)]
    s += [("U", 2000)]              # boundary: lock 5%, elapsed 0 -> ON
    s += [("U", 2099)]              # 99 < 100 -> ON
    s += [("U", 2100)]              # 100 < 100 false -> OFF
    s += [("U", 3500)]              # remainder of cycle -> OFF
    # Request 50% mid-cycle; must NOT take effect until the next boundary.
    s += [("S", 50.0)]
    s += [("U", 3900)]             # still locked 5% (OFF)
    s += [("U", 4000)]             # boundary: lock 50%, on_time 1000 ms, elapsed 0 -> ON
    s += [("U", 4999)]             # 999 < 1000 -> ON
    s += [("U", 5000)]             # 1000 < 1000 false -> OFF
    s += [("U", 5999)]             # OFF
    # Clamp >100 and go full ON next cycle.
    s += [("S", 150.0)]
    s += [("U", 6000)]             # boundary: lock 100%, always ON
    s += [("U", 7999)]             # ON across the whole window
    # Fall behind by several cycles: boundary advances ONE cycle only.
    # cycle_start is 6000 here; jump to 16000. elapsed 10000 -> advance to 8000,
    # elapsed 8000; on_time (100%) = 2000 -> 8000 < 2000 false -> OFF (the quirk).
    s += [("U", 16000)]
    # A request of exactly 0 turns fully off next cycle.
    s += [("S", 0.0)]
    s += [("U", 18000)]            # boundary: lock 0%, always OFF
    s += [("U", 18500)]            # OFF
    # Bring it back up, then emergency force_off mid-window.
    s += [("S", 80.0)]
    s += [("U", 20000)]            # boundary: lock 80%, on_time 1600 ms, elapsed 0 -> ON
    s += [("U", 20500)]            # 500 < 1600 -> ON
    s += [("F", None)]             # force_off -> locked 0
    s += [("U", 21000)]            # locked 0 -> OFF
    return s


def main():
    SSRController = load_ssr_controller()
    CLOCK["ms"] = 0
    pin = FakePin()
    ssr = SSRController(pin, cycle_time=CYCLE_TIME)

    rows = []
    saw = {"floored": 0, "clamped": 0, "midcycle_hold": 0, "on": 0, "off": 0, "force_off": 0}
    last_locked = None

    for kind, arg in build_script():
        if kind == "S":
            ssr.set_output(arg)
            duty = ssr.duty_cycle
            if 0.0 < arg < MIN_SSR_OUTPUT and duty == MIN_SSR_OUTPUT:
                saw["floored"] += 1
            if arg > 100.0 and duty == 100.0:
                saw["clamped"] += 1
            rows.append({"kind": "S", "arg": repr(arg), "on": "", "duty": repr(duty)})
        elif kind == "U":
            CLOCK["ms"] = arg
            requested_before = ssr.duty_cycle
            locked_before = ssr.duty_cycle_locked
            ssr.update()
            state = ssr.get_state()
            is_on = 1 if state["is_on"] else 0
            locked = state["duty_cycle_locked"]
            saw["on" if is_on else "off"] += 1
            # mid-cycle hold: a request that differs from the still-locked duty
            if requested_before != locked_before and locked == locked_before:
                saw["midcycle_hold"] += 1
            rows.append({"kind": "U", "arg": str(arg), "on": str(is_on), "duty": repr(locked)})
            last_locked = locked
        elif kind == "F":
            ssr.force_off()
            saw["force_off"] += 1
            locked = ssr.duty_cycle_locked
            rows.append({"kind": "F", "arg": "", "on": "", "duty": repr(locked)})

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", newline="") as f:
        f.write(f"# ssr_schedule|cycle_time={CYCLE_TIME!r}|min_ssr_output={MIN_SSR_OUTPUT!r}\n")
        w = csv.DictWriter(f, fieldnames=["kind", "arg", "on", "duty"])
        w.writeheader()
        w.writerows(rows)

    assert saw["floored"] >= 1, saw
    assert saw["clamped"] >= 1, saw
    assert saw["midcycle_hold"] >= 1, saw
    assert saw["on"] >= 5 and saw["off"] >= 5, saw
    assert saw["force_off"] == 1, saw
    print(f"wrote {len(rows)} rows -> {os.path.relpath(OUT, REPO_ROOT)}")
    print(f"  reference: {os.path.relpath(SRC, REPO_ROOT)}  (cycle_time={CYCLE_TIME}s)")
    print(f"  coverage: {saw}")


if __name__ == "__main__":
    main()
