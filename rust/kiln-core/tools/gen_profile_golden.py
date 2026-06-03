#!/usr/bin/env python3
"""Generate golden profile data from the REAL kiln/profile.py.

For a set of profiles covering ramp/hold/cooling combinations (including default
rate, first-step cooling, and zero-duration cases), record the reference
`duration`, plus `get_progress`/`is_complete` at several elapsed points.

The steps are encoded into each row so the Rust test rebuilds the *exact* same
profile — a single source of truth, no hand-mirrored step lists to drift.

Row format (pipe-delimited so step commas don't collide):
    idx|steps|elapsed|duration|progress|is_complete
Steps: `;`-joined, each `kind,target,rate,min,dur` with empty = absent,
kind in {r,h,c}.

profile.py imports only json/gc (stdlib), so it's loaded by file path directly.

Run:  python3 rust/kiln-core/tools/gen_profile_golden.py
"""

import importlib.util
import os

THIS = os.path.abspath(__file__)
REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(THIS))))
SRC = os.path.join(REPO_ROOT, "kiln", "profile.py")
OUT = os.path.join(REPO_ROOT, "rust", "kiln-core", "tests", "fixtures", "profile_golden.csv")

KIND = {"ramp": "r", "hold": "h", "cooling": "c"}

PROFILES = [
    # ramp + hold + cooling(target)
    {
        "name": "cone6",
        "steps": [
            {"type": "ramp", "target_temp": 600, "desired_rate": 100, "min_rate": 80},
            {"type": "hold", "target_temp": 600, "duration": 600},
            {"type": "cooling", "target_temp": 100},
        ],
    },
    # ramp with DEFAULT rate (no desired_rate) -> 100 C/h
    {
        "name": "default_rate",
        "steps": [
            {"type": "ramp", "target_temp": 120},
            {"type": "hold", "target_temp": 120, "duration": 300},
        ],
    },
    # multi-ramp staircase
    {
        "name": "staircase",
        "steps": [
            {"type": "ramp", "target_temp": 200, "desired_rate": 150},
            {"type": "ramp", "target_temp": 500, "desired_rate": 80},
            {"type": "ramp", "target_temp": 1000, "desired_rate": 60},
            {"type": "hold", "target_temp": 1000, "duration": 1200},
        ],
    },
    # cooling with NO target (contributes no estimated duration)
    {
        "name": "natural_cool_end",
        "steps": [
            {"type": "ramp", "target_temp": 300, "desired_rate": 120},
            {"type": "cooling"},
        ],
    },
    # FIRST step cooling without target -> exercises start-temp default (20)
    {
        "name": "first_cool",
        "steps": [
            {"type": "cooling"},
            {"type": "hold", "target_temp": 50, "duration": 100},
        ],
    },
]


def enc_step(s):
    def g(key):
        v = s.get(key)
        return repr(v) if v is not None else ""

    return f"{KIND[s['type']]},{g('target_temp')},{g('desired_rate')},{g('min_rate')},{g('duration')}"


def enc_steps(steps):
    return ";".join(enc_step(s) for s in steps)


def main():
    spec = importlib.util.spec_from_file_location("profile_ref", SRC)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    Profile = m.Profile

    lines = ["idx|steps|elapsed|duration|progress|is_complete"]
    for idx, prof in enumerate(PROFILES):
        P = Profile(prof)
        dur = P.duration
        enc = enc_steps(prof["steps"])

        # Query points: fractions of duration plus fixed values and overrun.
        elapseds = [0.0, 1.0, dur * 0.25, dur * 0.5, dur * 0.9, dur, dur * 1.5]
        # de-dup while preserving order, guard duration==0
        seen = []
        for e in elapseds:
            if e not in seen:
                seen.append(e)

        for e in seen:
            prog = P.get_progress(e)
            comp = 1 if P.is_complete(e) else 0
            lines.append(f"{idx}|{enc}|{repr(e)}|{repr(dur)}|{repr(prog)}|{comp}")

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w") as f:
        f.write("\n".join(lines) + "\n")

    print(f"wrote {len(lines) - 1} rows ({len(PROFILES)} profiles) -> {os.path.relpath(OUT, REPO_ROOT)}")
    print(f"  reference: {os.path.relpath(SRC, REPO_ROOT)}")
    for idx, prof in enumerate(PROFILES):
        print(f"  [{idx}] {prof['name']}: duration={Profile(prof).duration:.1f}s")


if __name__ == "__main__":
    main()
