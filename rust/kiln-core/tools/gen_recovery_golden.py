#!/usr/bin/env python3
"""Generate the golden recovery-decision trace from the REAL server/recovery.py.

`recovery.py:check_recovery` mixes filesystem I/O (`_find_most_recent_log`,
`_parse_last_log_entry`) with the pure decision we port to
`kiln-core::recovery`: was the last logged state RUNNING, and is the current
temperature still within `max_temp_delta` of the last logged temperature. The
I/O half stays in `kiln-app`; to exercise just the decision on CPython we:

  1. load recovery.py by file path (it imports only `os` + `gc`, both stdlib),
  2. monkeypatch `_find_most_recent_log` to hand back a valid-looking filename
     (so the in-body `profile_name` split succeeds and never short-circuits),
  3. monkeypatch `_parse_last_log_entry` to return scripted, already-parsed
     fields per scenario,
  4. call the real `check_recovery` and record `can_recover`, the echoed resume
     parameters, and a typed code for `recovery_reason`.

The Rust replay (`tests/replay_recovery.rs`) feeds the same parsed fields through
`kiln_core::recovery::check_recovery` and asserts the same outputs.

Run:  python3 rust/kiln-core/tools/gen_recovery_golden.py
"""

import importlib.util
import os
import sys

THIS = os.path.abspath(__file__)
REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(THIS))))
SRC = os.path.join(REPO_ROOT, "server", "recovery.py")
OUT = os.path.join(REPO_ROOT, "rust", "kiln-core", "tests", "fixtures", "recovery_golden.csv")

# A filename the in-body profile-name split accepts: rsplit('_', 2) -> 3 parts.
FAKE_LOG = "logs/Golden_2025-01-01_00-00-00.csv"


def load_recovery():
    spec = importlib.util.spec_from_file_location("recovery_ref", SRC)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


# Scenario = (state, last_temp, last_target, elapsed, step_index, current_temp, max_delta).
# step_index of None encodes a blank CSV column. Designed to hit every branch of
# the ported decision: RUNNING within / at / beyond delta (both signs), each
# non-RUNNING state, the zero-delta edge, and step_index passthrough (Some/None).
SCENARIOS = [
    ("RUNNING", 300.0, 305.0, 1800.0, 5, 298.0, 15.0),     # within delta -> recover
    ("RUNNING", 300.0, 305.0, 1800.0, 5, 285.0, 15.0),     # |dev| == delta -> recover (strict >)
    ("RUNNING", 300.0, 305.0, 1800.0, 5, 250.0, 15.0),     # current below, beyond -> reject
    ("RUNNING", 300.0, 305.0, 1800.0, 5, 400.0, 15.0),     # current above, beyond -> reject
    ("COMPLETE", 300.0, 305.0, 1800.0, 5, 300.0, 15.0),    # terminal state -> reject
    ("ERROR", 300.0, 305.0, 1800.0, 5, 300.0, 15.0),       # terminal state -> reject
    ("IDLE", 300.0, 305.0, 1800.0, 5, 300.0, 15.0),        # not running -> reject
    ("TUNING", 300.0, 305.0, 1800.0, 5, 300.0, 15.0),      # not a firing run -> reject
    ("RUNNING", 300.0, 305.0, 1800.0, None, 300.0, 15.0),  # blank step_index passthrough
    ("RUNNING", 300.0, 305.0, 1800.0, 5, 300.0, 0.0),      # zero delta, exact -> recover
    ("RUNNING", 300.0, 305.0, 1800.0, 5, 300.1, 0.0),      # zero delta, tiny dev -> reject
    ("RUNNING", 750.5, 760.0, 7200.0, 2, 748.0, 10.0),     # hot run, within -> recover
    ("RUNNING", 25.0, 30.0, 0.0, 0, 25.0, 20.0),           # start of run -> recover
]


def reason_code(reason):
    if reason.startswith("Recovery OK"):
        return "OK"
    if "not RUNNING" in reason:
        return "NOT_RUNNING"
    if "deviated too much" in reason:
        return "TEMP_DEVIATION"
    raise AssertionError(f"unexpected reason string: {reason!r}")


def main():
    rec = load_recovery()
    rec._find_most_recent_log = lambda logs_dir: FAKE_LOG

    rows = []
    saw = {"OK": 0, "NOT_RUNNING": 0, "TEMP_DEVIATION": 0, "none_step": 0}
    for (state, last_temp, last_target, elapsed, step_index, current_temp, max_delta) in SCENARIOS:
        rec._parse_last_log_entry = lambda log_file, _e=(
            elapsed, last_temp, last_target, state, step_index
        ): {
            "elapsed": _e[0],
            "current_temp": _e[1],
            "target_temp": _e[2],
            "state": _e[3],
            "step_index": _e[4],
        }

        info = rec.check_recovery("/fake/logs", current_temp, max_delta)
        code = reason_code(info.recovery_reason)
        saw[code] += 1
        if step_index is None:
            saw["none_step"] += 1

        # Sanity: the echoed resume parameters must be the parsed inputs.
        assert info.elapsed_seconds == elapsed, info.elapsed_seconds
        assert info.last_temp == last_temp, info.last_temp
        assert info.last_target_temp == last_target, info.last_target_temp
        assert info.step_index == step_index, info.step_index

        rows.append(
            f"{state},{repr(last_temp)},{repr(last_target)},{repr(elapsed)},"
            f"{'' if step_index is None else step_index},"
            f"{repr(current_temp)},{repr(max_delta)},"
            f"{1 if info.can_recover else 0},{code}"
        )

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", newline="") as f:
        f.write("# recovery\n")
        f.write("state,last_temp,last_target,elapsed,step_index,current_temp,max_delta,can_recover,reason\n")
        f.write("\n".join(rows) + "\n")

    assert saw["OK"] >= 3, saw
    assert saw["NOT_RUNNING"] >= 4, saw
    assert saw["TEMP_DEVIATION"] >= 3, saw
    assert saw["none_step"] >= 1, saw
    print(f"wrote {len(rows)} rows -> {os.path.relpath(OUT, REPO_ROOT)}")
    print(f"  reference: {os.path.relpath(SRC, REPO_ROOT)}")
    print(f"  coverage: OK={saw['OK']}, NOT_RUNNING={saw['NOT_RUNNING']}, "
          f"TEMP_DEVIATION={saw['TEMP_DEVIATION']}, blank-step={saw['none_step']}")


if __name__ == "__main__":
    main()
