#!/usr/bin/env python3
"""Generate golden state-machine traces from the REAL kiln/state.py.

state.py does `from kiln.rate_monitor import TempHistory` and `from micropython
import const`, and reads `time.time()` internally. To run it on CPython without
the hardware package we:

  1. install a `micropython` shim (const/native),
  2. pre-register a stub `kiln` package + the real `kiln.rate_monitor` in
     sys.modules so the relative import resolves WITHOUT running kiln/__init__.py,
  3. load state.py as `kiln.state`,
  4. replace its `time` with a controllable fake clock.

Then we drive the real KilnController through three scenarios (run progression
with NTP jumps, stall detection, crash recovery) and record per-update outputs.

Each fixture is self-describing:
    # config|max_temp,rmw,rri,sci,scf,smst
    # profile|<steps enc: kind,target,rate,min,dur ; ...>
    # op|run|pre_run_temp|run_now
    # op|resume|elapsed|last_logged|current|step_index|now   (alt)
    idx,now,current_temp,state,target,step_index,recovering,rate
    <rows...>

Run:  python3 rust/kiln-core/tools/gen_state_golden.py
"""

import importlib.util
import os
import sys
import types

THIS = os.path.abspath(__file__)
REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(THIS))))
KILN_DIR = os.path.join(REPO_ROOT, "python", "kiln")
FIX_DIR = os.path.join(REPO_ROOT, "rust", "kiln-core", "tests", "fixtures")
KIND = {"ramp": "r", "hold": "h", "cooling": "c"}


class FakeClock:
    def __init__(self):
        self.t = 0.0

    def time(self):
        return self.t


def load_state_module(clock):
    # 1. micropython shim
    mp = types.ModuleType("micropython")
    mp.native = lambda f: f
    mp.const = lambda x: x
    sys.modules["micropython"] = mp

    # 2. stub kiln package + real rate_monitor (by path) so the relative import
    #    in state.py resolves without executing kiln/__init__.py.
    pkg = types.ModuleType("kiln")
    pkg.__path__ = []
    sys.modules["kiln"] = pkg
    spec = importlib.util.spec_from_file_location(
        "kiln.rate_monitor", os.path.join(KILN_DIR, "rate_monitor.py")
    )
    rate = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(rate)
    sys.modules["kiln.rate_monitor"] = rate

    # 3. load state.py as kiln.state
    spec = importlib.util.spec_from_file_location(
        "kiln.state", os.path.join(KILN_DIR, "state.py")
    )
    state = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(state)

    # 4. controllable clock
    faketime = types.ModuleType("time")
    faketime.time = clock.time
    faketime.localtime = __import__("time").localtime
    faketime.sleep = lambda *_: None
    state.time = faketime
    return state


def enc_steps(steps):
    def g(s, k):
        v = s.get(k)
        return repr(v) if v is not None else ""

    return ";".join(
        f"{KIND[s['type']]},{g(s,'target_temp')},{g(s,'desired_rate')},{g(s,'min_rate')},{g(s,'duration')}"
        for s in steps
    )


def make_config(state_mod, max_temp, rmw, rri, sci, scf, smst):
    return types.SimpleNamespace(
        MAX_TEMP=max_temp,
        RATE_MEASUREMENT_WINDOW=rmw,
        RATE_RECORDING_INTERVAL=rri,
        STALL_CHECK_INTERVAL=sci,
        STALL_CONSECUTIVE_FAILS=scf,
        STALL_MIN_STEP_TIME=smst,
    )


def drive(state_mod, clock, cfg_tuple, steps, op, inputs):
    """Run one scenario, returning fixture lines."""
    cfg = make_config(state_mod, *cfg_tuple)
    controller = state_mod.KilnController(cfg)
    profile = types.SimpleNamespace(name="golden", steps=steps)

    header = [
        f"# config|{','.join(repr(x) for x in cfg_tuple)}",
        f"# profile|{enc_steps(steps)}",
    ]

    if op[0] == "run":
        _, pre_run_temp, run_now = op
        controller.current_temp = pre_run_temp
        clock.t = run_now
        controller.run_profile(profile)
        header.append(f"# op|run|{repr(pre_run_temp)}|{repr(run_now)}")
    elif op[0] == "resume":
        _, elapsed, llt, cur, step_index, now = op
        clock.t = now
        controller.resume_profile(profile, elapsed, llt, cur, step_index)
        header.append(
            "# op|resume|"
            + "|".join(
                repr(x) if x is not None else "" for x in (elapsed, llt, cur, step_index, now)
            )
        )
    else:
        raise ValueError(op)

    lines = list(header)
    lines.append("idx,now,current_temp,state,target,step_index,recovering,rate")
    rmw = cfg_tuple[1]
    for i, (now, temp) in enumerate(inputs):
        clock.t = now
        out = controller.update(temp)
        rate = controller.temp_history.get_rate(rmw)
        recovering = 1 if controller.recovery_target_temp is not None else 0
        lines.append(
            f"{i},{repr(now)},{repr(temp)},{int(controller.state)},{repr(out)},"
            f"{controller.current_step_index},{recovering},{repr(rate)}"
        )
    return lines, controller


def scenario_run(state_mod, clock):
    cfg = (1300.0, 600.0, 10.0, 60.0, 3, 600.0)
    steps = [
        {"type": "ramp", "target_temp": 200, "desired_rate": 600},
        {"type": "hold", "target_temp": 200, "duration": 100},
        {"type": "cooling", "target_temp": 100},
    ]
    inputs = []
    now, temp = 1000.0, 20.0
    while temp < 210.0:  # ramp phase -> completes when temp >= 200
        inputs.append((now, temp))
        now += 1.0
        temp += 4.0
    for k in range(120):  # hold phase (dur 100) with NTP jumps injected
        if k == 50:
            now -= 5.0  # backward jump  -> delta<0  -> clamp 1.0
        if k == 80:
            now += 500.0  # forward jump -> delta>60 -> clamp 1.0
        inputs.append((now, 200.0))
        now += 1.0
    temp = 200.0
    while temp > 88.0:  # cooling phase -> completes when temp <= 100
        inputs.append((now, temp))
        now += 1.0
        temp -= 4.0
    for _ in range(3):  # a few post-complete rows
        inputs.append((now, 88.0))
        now += 1.0
    return cfg, steps, ("run", 20.0, 1000.0), inputs


def scenario_stall(state_mod, clock):
    # Short thresholds so a flat ramp stalls quickly.
    cfg = (1300.0, 10.0, 1.0, 2.0, 2, 4.0)
    steps = [{"type": "ramp", "target_temp": 500, "desired_rate": 100, "min_rate": 50}]
    inputs = [(float(t), 100.0) for t in range(0, 12)]  # flat temp -> rate 0 < 50
    return cfg, steps, ("run", 100.0, 0.0), inputs


def scenario_recovery(state_mod, clock):
    cfg = (1300.0, 600.0, 10.0, 60.0, 3, 600.0)
    steps = [
        {"type": "ramp", "target_temp": 600, "desired_rate": 100},
        {"type": "hold", "target_temp": 600, "duration": 600},
    ]
    # Resume 30 min in, lost 50C (300 logged vs 250 now) -> recovery hold @300.
    op = ("resume", 1800.0, 300.0, 250.0, None, 5000.0)
    inputs = []
    now, temp = 5000.0, 250.0
    while temp < 306.0:  # climb past 299 -> recovery exits
        inputs.append((now, temp))
        now += 1.0
        temp += 4.0
    for _ in range(5):  # a few rows of normal running after recovery
        inputs.append((now, temp))
        now += 1.0
        temp += 1.0
    return cfg, steps, op, inputs


def main():
    os.makedirs(FIX_DIR, exist_ok=True)
    scenarios = {
        "state_run_golden.csv": scenario_run,
        "state_stall_golden.csv": scenario_stall,
        "state_recovery_golden.csv": scenario_recovery,
    }
    for fname, builder in scenarios.items():
        clock = FakeClock()
        state_mod = load_state_module(clock)
        cfg, steps, op, inputs = builder(state_mod, clock)
        lines, controller = drive(state_mod, clock, cfg, steps, op, inputs)
        path = os.path.join(FIX_DIR, fname)
        with open(path, "w") as f:
            f.write("\n".join(lines) + "\n")
        final = int(controller.state)
        print(f"wrote {len(inputs)} rows -> {os.path.relpath(path, REPO_ROOT)} (final state={final})")


if __name__ == "__main__":
    main()
