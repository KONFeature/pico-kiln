#!/usr/bin/env python3
"""Generate golden tuner traces from the REAL kiln/tuner.py.

tuner.py only imports `time`, so we just load it by file path and patch its
clock. An adaptive driver inspects the tuner's current step to feed inputs that
trigger each completion path (target reach, plateau, timeout, cooling), then
records the EXACT (now, temp) inputs and outputs so the Rust test can replay
them deterministically.

Fixture header:
    # mode|SAFE
    # max_temp|200.0
    # start_now|0.0
    idx,now,current_temp,ssr,continue,stage,step_index

Run:  python3 rust/kiln-core/tools/gen_tuner_golden.py
"""

import importlib.util
import os
import sys
import types

THIS = os.path.abspath(__file__)
REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(THIS))))
SRC = os.path.join(REPO_ROOT, "python", "kiln", "tuner.py")
FIX_DIR = os.path.join(REPO_ROOT, "rust", "kiln-core", "tests", "fixtures")

STAGE = {"running": 0, "complete": 1, "error": 2}


class FakeClock:
    def __init__(self):
        self.t = 0.0

    def time(self):
        return self.t


def load_tuner(clock):
    spec = importlib.util.spec_from_file_location("tuner_ref", SRC)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    faketime = types.ModuleType("time")
    faketime.time = clock.time
    faketime.localtime = __import__("time").localtime
    m.time = faketime
    return m


def adaptive_drive(tuner_mod, clock, mode, max_temp, start_now, start_temp, cap=400):
    """Drive the real tuner, choosing inputs based on the current step."""
    tuner = tuner_mod.ZieglerNicholsTuner(mode=mode, max_temp=max_temp)
    clock.t = start_now
    tuner.start()

    rows = []
    now = start_now
    temp = start_temp

    for i in range(cap):
        step = tuner.current_step
        if step.plateau_detect:
            now += 60.0  # advance one plateau-check interval; hold temp flat
        elif step.target_temp is not None and step.ssr_percent > 0:
            now += 5.0  # heating toward absolute target
            if temp < step.target_temp + 2.0:
                temp += 5.0
        elif step.target_temp is not None and step.ssr_percent == 0:
            now += 5.0  # cooling toward peak - target
            temp -= 5.0
        else:
            # timeout-only step: first update starts it, then jump past timeout
            if step.start_time is None:
                now += 5.0
            else:
                now = step.start_time + step.timeout + 1.0

        clock.t = now
        out, cont = tuner.update(temp)
        rows.append((now, temp, out, cont, STAGE[tuner.stage], tuner.current_step_index))
        if not cont:
            break

    return tuner, rows


def manual_drive(tuner_mod, clock, mode, max_temp, start_now, inputs):
    """Drive with an explicit (now, temp) input list (for the error case)."""
    tuner = tuner_mod.ZieglerNicholsTuner(mode=mode, max_temp=max_temp)
    clock.t = start_now
    tuner.start()
    rows = []
    for now, temp in inputs:
        clock.t = now
        out, cont = tuner.update(temp)
        rows.append((now, temp, out, cont, STAGE[tuner.stage], tuner.current_step_index))
        if not cont:
            break
    return tuner, rows


def write_fixture(fname, mode, max_temp, start_now, rows):
    lines = [
        f"# mode|{mode}",
        f"# max_temp|{repr(max_temp)}",
        f"# start_now|{repr(start_now)}",
        "idx,now,current_temp,ssr,continue,stage,step_index",
    ]
    for i, (now, temp, out, cont, stage, step_idx) in enumerate(rows):
        lines.append(f"{i},{repr(now)},{repr(temp)},{repr(out)},{1 if cont else 0},{stage},{step_idx}")
    path = os.path.join(FIX_DIR, fname)
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")
    return path


def main():
    os.makedirs(FIX_DIR, exist_ok=True)

    # SAFE: target-reach -> timeout -> cooling -> COMPLETE
    clock = FakeClock()
    mod = load_tuner(clock)
    t, rows = adaptive_drive(mod, clock, "SAFE", 200, 0.0, 20.0)
    p = write_fixture("tuner_safe_golden.csv", "SAFE", 200, 0.0, rows)
    print(f"wrote {len(rows)} rows -> {os.path.relpath(p, REPO_ROOT)} (final stage={t.stage}, steps={len(t.steps)})")

    # STANDARD: plateau x3 + timeout x3 -> COMPLETE
    clock = FakeClock()
    mod = load_tuner(clock)
    t, rows = adaptive_drive(mod, clock, "STANDARD", 900, 0.0, 20.0)
    p = write_fixture("tuner_standard_golden.csv", "STANDARD", 900, 0.0, rows)
    print(f"wrote {len(rows)} rows -> {os.path.relpath(p, REPO_ROOT)} (final stage={t.stage}, steps={len(t.steps)})")

    # ERROR: exceed max_temp
    clock = FakeClock()
    mod = load_tuner(clock)
    t, rows = manual_drive(mod, clock, "SAFE", 100, 0.0, [(1.0, 50.0), (2.0, 150.0)])
    p = write_fixture("tuner_error_golden.csv", "SAFE", 100, 0.0, rows)
    print(f"wrote {len(rows)} rows -> {os.path.relpath(p, REPO_ROOT)} (final stage={t.stage})")


if __name__ == "__main__":
    main()
