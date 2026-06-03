#!/usr/bin/env python3
"""Generate golden PID data from the REAL MicroPython controller (kiln/pid.py).

This imports the production `kiln/pid.py` unchanged (via a tiny `micropython`
shim) and drives it through a deterministic scenario that exercises every
branch of the controller: proportional response, integral wind-up, derivative
kicks, output saturation (high) and a step-down that forces negative error and
integral unwind.

The emitted CSV is consumed by `tests/replay_pid.rs`, which replays the exact
same (setpoint, measured, time) inputs through the Rust port and asserts the
outputs match. Floats are written with `repr()` so they round-trip bit-exactly
through f64 parsing on the Rust side.

Run from anywhere:
    python3 rust/kiln-core/tools/gen_pid_golden.py
"""

import csv
import importlib.util
import os
import sys
import types

# --- Locate repo root relative to this file: rust/kiln-core/tools/<this> ---
THIS = os.path.abspath(__file__)
REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(THIS))))
PID_PATH = os.path.join(REPO_ROOT, "kiln", "pid.py")
OUT_PATH = os.path.join(
    REPO_ROOT, "rust", "kiln-core", "tests", "fixtures", "pid_golden.csv"
)

# --- Inject a minimal `micropython` shim so pid.py imports unchanged ---
# pid.py uses `import micropython` and the `@micropython.native` decorator.
if "micropython" not in sys.modules:
    mp = types.ModuleType("micropython")
    mp.native = lambda f: f          # no-op decorator on CPython
    mp.viper = lambda f: f
    mp.const = lambda x: x
    sys.modules["micropython"] = mp


def load_reference_pid():
    """Load the production PID class straight from kiln/pid.py by file path.

    Loading by path (not `import kiln.pid`) avoids executing kiln/__init__.py,
    which pulls in hardware-only modules unavailable on CPython.
    """
    spec = importlib.util.spec_from_file_location("pid_ref", PID_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.PID


# Deterministic dt jitter (seconds). Mirrors real-world ~1 Hz loop wobble so the
# d_term (error_delta/dt) and integral (error*dt) paths are tested with dt != 1.
DT_PATTERN = [1.0, 0.97, 1.04, 0.99, 1.02]


def setpoint_at(t):
    """Firing-like setpoint: ramp up, hold, then a step DOWN.

    - 0..120 s : ramp 20 -> 200 C at 1.5 C/s  (controller saturates high)
    - 120..200 s: hold 200 C                  (steady-state, integral settles)
    - 200..240 s: step to 120 C               (negative error, output -> 0)
    """
    if t <= 120.0:
        return 20.0 + 1.5 * t
    if t <= 200.0:
        return 200.0
    return 120.0


def main():
    PID = load_reference_pid()

    # Config defaults from config.example.py
    pid = PID(kp=25.0, ki=0.14, kd=160.0, output_limits=(0, 100))

    # Simple first-order plant ONLY to produce varied, reproducible `measured`
    # values. The Rust test never sees the plant; it replays recorded inputs.
    ambient = 20.0
    temp = 20.0
    heat_per_pct = 0.03   # C/s per 1% SSR output
    cooling = 0.0015      # C/s per C above ambient

    rows = []
    t = 0.0
    n = 240
    sat_count = 0
    neg_err_count = 0

    for i in range(n):
        sp = setpoint_at(t)
        out = pid.update(sp, temp, current_time=t)
        s = pid.get_stats()

        rows.append(
            {
                "idx": i,
                "time_s": repr(t),
                "setpoint": repr(sp),
                "measured": repr(temp),
                "output": repr(out),
                "p_term": repr(s["p_term"]),
                "i_term": repr(s["i_term"]),
                "d_term": repr(s["d_term"]),
                "integral_frozen": int(bool(s["integral_frozen"])),
            }
        )

        if out >= 100.0 or out <= 0.0:
            sat_count += 1
        if (sp - temp) < 0.0:
            neg_err_count += 1

        # Advance plant with this step's output, then advance time by jittered dt.
        dt = DT_PATTERN[i % len(DT_PATTERN)]
        temp += (heat_per_pct * out - cooling * (temp - ambient)) * dt
        t += dt

    os.makedirs(os.path.dirname(OUT_PATH), exist_ok=True)
    fields = [
        "idx",
        "time_s",
        "setpoint",
        "measured",
        "output",
        "p_term",
        "i_term",
        "d_term",
        "integral_frozen",
    ]
    with open(OUT_PATH, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        w.writerows(rows)

    print(f"wrote {len(rows)} rows -> {os.path.relpath(OUT_PATH, REPO_ROOT)}")
    print(f"  reference: {os.path.relpath(PID_PATH, REPO_ROOT)}")
    print(f"  saturated samples (out at 0 or 100): {sat_count}")
    print(f"  negative-error samples: {neg_err_count}")
    out_vals = [float(r["output"]) for r in rows]
    print(f"  output range: {min(out_vals):.4f} .. {max(out_vals):.4f}")


if __name__ == "__main__":
    main()
