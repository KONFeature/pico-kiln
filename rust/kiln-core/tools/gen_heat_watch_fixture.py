#!/usr/bin/env python3
"""Build kiln-core/tests/fixtures/heat_watch_real_logs.csv from real firing logs.

Unlike the other gen_*_golden.py tools this does not run Python code: the heat
watch is Rust-only. It condenses real device logs (scripts/logs/*.csv, which are
gitignored) into a compact committed trace so `tests/replay_heat_watch.rs` can
prove, on every `cargo test`, that the heating-response watchdog never trips on
a real firing and still trips when a fault is injected into one.

Output columns: seg,t_s,temp,target,output,mode
  seg     segment id; a new one per log and after every RECOVERY (a reboot: the
          controller resets the watch on ResumeProfile)
  t_s     monotonic seconds within the log: the controller's elapsed_seconds,
          stitched across the tuner's per-step restarts (the wall timestamp is
          unusable: right after a reboot the clock is not yet NTP-synced)
  mode    R = RUNNING (setpoint = target), T = TUNING (no setpoint)

Usage: python3 rust/kiln-core/tools/gen_heat_watch_fixture.py [log.csv ...]
"""
import csv
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
OUT = os.path.join(HERE, "..", "tests", "fixtures", "heat_watch_real_logs.csv")

DEFAULT_LOGS = [
    # Hottest firing on record (983 C), 3505 rows, no interruption.
    "scripts/logs/980-biscuit-cone06_2026-03-05_02-00-01.csv",
    # Firing with three power-outage recoveries (recovery holds at 100 %).
    "scripts/logs/biscuit_faience_adaptive_2025-11-09_09-28-31.csv",
    # HIGH_TEMP autotune: open-loop 100 % steps up to 626 C.
    "scripts/logs/tuning_2025-11-27_12-59-25.csv",
]

# Decimation: keep one row per this many seconds (the watch window is 900 s).
MIN_DT = {"R": 60.0, "T": 20.0}


def main(paths):
    rows_out = []
    seg = -1
    for path in paths:
        full = path if os.path.isabs(path) else os.path.join(REPO, path)
        seg += 1
        last_kept = None
        prev_state = None
        offset = 0.0
        prev_raw = None
        prev_t = 0.0
        for r in csv.DictReader(open(full)):
            state = r["state"]
            if state == "RECOVERY":
                if prev_state != "RECOVERY":
                    seg += 1
                    last_kept = None
                prev_state = state
                continue
            prev_state = state
            mode = {"RUNNING": "R", "TUNING": "T"}.get(state)
            if mode is None:
                continue
            raw = float(r["elapsed_seconds"])
            if prev_raw is not None and raw < prev_raw:
                # Tuner step boundary: elapsed restarts near 0.
                offset = prev_t + 1.0 - raw
            prev_raw = raw
            t = raw + offset
            prev_t = t
            if last_kept is not None and t - last_kept < MIN_DT[mode]:
                continue
            last_kept = t
            rows_out.append(
                "%d,%d,%.1f,%.1f,%d,%s"
                % (
                    seg,
                    t,
                    float(r["current_temp_c"]),
                    float(r["target_temp_c"] or 0.0),
                    round(float(r["ssr_output_percent"])),
                    mode,
                )
            )
    with open(OUT, "w") as f:
        f.write("seg,t_s,temp,target,output,mode\n")
        f.write("\n".join(rows_out) + "\n")
    print("wrote %d rows, %d segments -> %s" % (len(rows_out), seg + 1, OUT))


if __name__ == "__main__":
    main(sys.argv[1:] or DEFAULT_LOGS)
