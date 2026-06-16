#!/usr/bin/env python3
"""Generate golden temperature-filter traces from the REAL kiln/hardware.py.

`hardware.py`'s `TemperatureSensor` mixes SPI I/O (via adafruit_max31856) with the
pure conditioning logic we port to `kiln-core::temp_filter`: median spike
rejection, range checks, consecutive-fault counting with cold-start tolerance,
and a window re-seed after a dropout. To exercise just that logic on CPython we:

  1. install a `micropython` shim (const/native),
  2. inject a fake `time` whose ticks_diff() forces __init__'s "wait for first
     conversion" poll to give up immediately, so the sensor starts UNINITIALISED
     with an empty window (matching `TempFilter::new`) instead of pre-seeding,
  3. inject a fake `adafruit_max31856.MAX31856` whose `.fault` and
     `.unpack_temperature()` we script per step,
  4. load hardware.py by file path and drive `TemperatureSensor.read()`.

Each step is a reading (`R,<raw>`) or a fault (`F`); the recorded result is the
returned filtered temperature, or `ERR:NotInitialized` / `ERR:EmergencyShutdown`
when read() raises. The Rust replay feeds the same script through `TempFilter`.

Run:  python3 rust/kiln-core/tools/gen_temp_filter_golden.py
"""

import csv
import importlib.util
import os
import sys
import types

THIS = os.path.abspath(__file__)
REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(THIS))))
SRC = os.path.join(REPO_ROOT, "python", "kiln", "hardware.py")
OUT = os.path.join(REPO_ROOT, "rust", "kiln-core", "tests", "fixtures", "temp_filter_golden.csv")

OFFSET = 0.0
WINDOW = 3
TEMP_MIN_RANGE = -50.0  # mirror hardware.py, for coverage accounting only
TEMP_MAX_RANGE = 1500.0


class FakeSensor:
    """Stand-in for adafruit_max31856.MAX31856 with scriptable temp + faults."""

    def __init__(self, spi, cs_pin, thermocouple_type=None):
        self.noise_rejection = None
        self.averaging = None
        self.next_temp = 0.0
        self.next_fault = {}

    def start_autoconverting(self):
        pass

    def unpack_temperature(self):
        return self.next_temp

    @property
    def fault(self):
        return self.next_fault


class ThermocoupleType:
    B, E, J, K, N, R, S, T = 0, 1, 2, 3, 4, 5, 6, 7


def load_temperature_sensor():
    mp = types.ModuleType("micropython")
    mp.native = lambda f: f
    mp.const = lambda x: x
    sys.modules["micropython"] = mp

    faketime = types.ModuleType("time")
    faketime.ticks_ms = lambda: 0
    faketime.ticks_diff = lambda a, b: 2000  # >= 1500 so the init poll bails at once
    faketime.sleep_ms = lambda *_: None
    faketime.sleep = lambda *_: None
    sys.modules["time"] = faketime

    ada = types.ModuleType("adafruit_max31856")
    ada.MAX31856 = FakeSensor
    ada.ThermocoupleType = ThermocoupleType
    sys.modules["adafruit_max31856"] = ada

    spec = importlib.util.spec_from_file_location("kiln_hardware_ref", SRC)
    m = importlib.util.module_from_spec(spec)
    # hardware.py uses the bare `@micropython.native` decorator but only does
    # `from micropython import const`; on-device the compiler treats the decorator
    # specially. Pre-bind the name so the class body executes on CPython.
    m.micropython = mp
    spec.loader.exec_module(m)
    return m.TemperatureSensor


# Scenario: (kind, raw). kind "R" = reading at raw; "F" = fault register tripped.
# Designed to hit every branch of the reference read():
#   - fault before init -> NotInitialized, then a good reading recovers
#   - median warmup (even/odd windows) + an isolated spike that is rejected
#   - transient faults returning last-good, a window re-seed at >= WINDOW faults
#   - out-of-range readings funnelled to the fault path
#   - a climb that pushes max_recorded >= 100 (cold->hot), then the lower hot
#     fault budget tripping EmergencyShutdown
def build_script():
    script = [("F", None)]  # NotInitialized
    script += [("R", 25.0), ("R", 26.0), ("R", 24.0)]  # warmup: 25, 25.5, 25
    script += [("R", 100.0), ("R", 25.0), ("R", 26.0), ("R", 27.0)]  # spike rejected + flushed
    script += [("F", None), ("F", None), ("F", None)]  # transient faults -> last-good, re-seed
    script += [("R", 30.0), ("R", 31.0), ("R", 32.0)]  # recover
    script += [("R", 5000.0), ("R", -100.0)]  # out-of-range -> fault path
    script += [("R", 33.0)]  # recover again
    script += [("R", 500.0), ("R", 510.0), ("R", 520.0)]  # climb hot (max_recorded >= 100)
    script += [("F", None)] * 20  # hot budget is 20 -> EmergencyShutdown on the 20th
    return script


def main():
    TemperatureSensor = load_temperature_sensor()
    sensor = TemperatureSensor(
        spi=None,
        cs_pin=None,
        thermocouple_type=ThermocoupleType.K,
        offset=OFFSET,
        mains_frequency=60,
        averaging=8,
        median_window=WINDOW,
    )
    assert not sensor.initialized, "init should leave the sensor uninitialised"

    rows = []
    saw = {"not_init": 0, "shutdown": 0, "last_good": 0}
    for kind, raw in build_script():
        if kind == "F":
            sensor.sensor.next_fault = {"open_tc": True}
        else:
            sensor.sensor.next_fault = {}
            sensor.sensor.next_temp = raw

        try:
            out = sensor.read()
            result = repr(out)
            if kind == "F" or raw < TEMP_MIN_RANGE or raw > TEMP_MAX_RANGE:
                saw["last_good"] += 1  # fault/out-of-range step that still returned
        except Exception as e:
            msg = str(e)
            if "failed to initialize" in msg:
                result, key = "ERR:NotInitialized", "not_init"
            elif "EMERGENCY SHUTDOWN" in msg:
                result, key = "ERR:EmergencyShutdown", "shutdown"
            else:
                raise AssertionError(f"unexpected error: {msg}")
            saw[key] += 1

        rows.append({"kind": kind, "input": "" if raw is None else repr(raw), "result": result})

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", newline="") as f:
        f.write(f"# temp_filter|offset={OFFSET!r}|window={WINDOW}\n")
        w = csv.DictWriter(f, fieldnames=["kind", "input", "result"])
        w.writeheader()
        w.writerows(rows)

    assert saw["not_init"] == 1, saw
    assert saw["shutdown"] == 1, saw
    assert saw["last_good"] >= 5, saw
    print(f"wrote {len(rows)} rows -> {os.path.relpath(OUT, REPO_ROOT)}")
    print(f"  reference: {os.path.relpath(SRC, REPO_ROOT)}  (window={WINDOW}, offset={OFFSET})")
    print(f"  coverage: NotInitialized=1, EmergencyShutdown=1, last-good returns={saw['last_good']}")


if __name__ == "__main__":
    main()
