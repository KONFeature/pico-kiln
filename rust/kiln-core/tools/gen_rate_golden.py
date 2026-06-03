#!/usr/bin/env python3
"""Generate golden rate data from the REAL kiln/rate_monitor.py (TempHistory).

Drives the production circular buffer through adds, a mid-sequence clear, ring
overflow (>capacity), and queries the rate over several windows so the Rust port
is checked against the reference's exact endpoint-selection and tie-breaking.

`rate_monitor.py` has no imports, so it's loaded by file path directly.

Run:  python3 rust/kiln-core/tools/gen_rate_golden.py
"""

import csv
import importlib.util
import os

THIS = os.path.abspath(__file__)
REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(THIS))))
SRC = os.path.join(REPO_ROOT, "kiln", "rate_monitor.py")
OUT = os.path.join(REPO_ROOT, "rust", "kiln-core", "tests", "fixtures", "rate_golden.csv")

WINDOWS = [60.0, 120.0, 600.0]
CAPACITY = 60


def load_temp_history():
    spec = importlib.util.spec_from_file_location("rate_ref", SRC)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m.TempHistory


def temp_at(i):
    # Deterministic, slightly non-linear so rates vary. Pure arithmetic.
    return 20.0 + i * 1.5 + (i % 7) * 0.7 - (i % 5) * 0.4


def main():
    TempHistory = load_temp_history()
    h = TempHistory(capacity=CAPACITY)

    rows = []
    n = 100
    clear_at = 35  # exercise a mid-sequence clear (rates -> 0 afterwards)

    for i in range(n):
        if i == clear_at:
            h.clear()
            op, t, temp = "clear", "", ""
        else:
            t = i * 10.0  # 10 s sampling, monotonic timestamps
            temp = temp_at(i)
            h.add(t, temp)
            op = "add"

        rates = [h.get_rate(w) for w in WINDOWS]
        rows.append(
            {
                "idx": i,
                "op": op,
                "time": repr(t) if t != "" else "",
                "temp": repr(temp) if temp != "" else "",
                "rate60": repr(rates[0]),
                "rate120": repr(rates[1]),
                "rate600": repr(rates[2]),
            }
        )

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    fields = ["idx", "op", "time", "temp", "rate60", "rate120", "rate600"]
    with open(OUT, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fields)
        w.writeheader()
        w.writerows(rows)

    nonzero = sum(1 for r in rows if float(r["rate600"]) != 0.0)
    print(f"wrote {len(rows)} rows -> {os.path.relpath(OUT, REPO_ROOT)}")
    print(f"  reference: {os.path.relpath(SRC, REPO_ROOT)}  (capacity={CAPACITY})")
    print(f"  overflow tested: {n - 1} adds vs capacity {CAPACITY}")
    print(f"  rows with non-zero rate600: {nonzero}")


if __name__ == "__main__":
    main()
