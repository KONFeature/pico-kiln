# pico-kiln — MicroPython firmware (original implementation)

The original kiln controller firmware, running on MicroPython on the Raspberry
Pi Pico 2 W. The current/primary firmware is now the Rust implementation under
[`../rust/`](../rust); this MicroPython version is kept here.

Run all commands in this guide from the `python/` directory unless noted.

## Architecture

The Pico 2's dual cores are split by concern:

- **Core 1** — time-critical control: reads the MAX31856 thermocouple (SPI),
  runs the PID loop with thermal-model gain scheduling, drives the SSR, executes
  firing profiles.
- **Core 2** — web interface and monitoring: serves the UI, accepts profile
  uploads, reports temperature / SSR / program state, logs to CSV, and runs the
  multi-mode PID auto-tuner.

Full design notes: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Hardware

- Raspberry Pi Pico 2 W (RP2350) with MicroPython
- MAX31856 thermocouple board (SPI) + K-type (or other supported) thermocouple
- SSR for kiln control (default GPIO 15, configurable)
- WiFi (Pico 2 W) for the web interface

## Setup & deploy

```bash
# 1. Configure (WiFi credentials, pins, PID/thermal model)
cp config.example.py config.py

# 2. Compile to .mpy bytecode (optional but faster on device)
./compile.sh                 # development build
./compile.sh --production    # minified + prints stripped

# 3. Deploy to the Pico over USB (mpremote)
./deploy.sh                  # deploys build/ if present, else source .py
./deploy.sh --clean          # wipe existing dirs on the Pico first

# 4. Sync firing profiles (from the shared ../profiles dir)
./sync_profiles.sh

# 5. Reset the Pico, watch the serial console for its IP, open http://<ip>
```

Other helpers: `./debug.sh` (interactive session), `./dump_logs.sh` (download
logs into `../scripts/logs/`), `./clean_logs.sh` (clear logs on the Pico).

Requirements for the host-side tooling: `pip install -r requirements.txt`
(`mpremote`, `mpy-cross`, `python-minifier` as needed).

## Structure

```
python/
├── boot.py                 # MicroPython boot config
├── main.py                 # Entry point — asyncio setup + init
├── config.example.py       # Config template (copy to config.py)
├── kiln/                   # Core 1: control
│   ├── control_thread.py   # Main control loop
│   ├── state.py            # Controller state machine
│   ├── pid.py              # PID with anti-windup
│   ├── scheduler.py        # Profile scheduling
│   ├── tuner.py            # Multi-mode PID auto-tuning
│   ├── rate_monitor.py     # Rolling rate / stall detection
│   ├── profile.py          # Firing-profile management
│   ├── hardware.py         # Thermocouple + SSR abstraction
│   └── comms.py            # Inter-core communication
├── server/                 # Core 2: web server, logging, recovery
├── lib/                    # MicroPython libs (MAX31856, LCD1602, ...)
├── static/  -> ../static   # Shared web assets (lives at repo root)
├── debug/                  # Standalone hardware/boot debug scripts
├── docs/                   # Firmware docs (tuning, thermal model, etc.)
├── compile.sh deploy.sh debug.sh dump_logs.sh clean_logs.sh sync_profiles.sh
└── remove_prints.py        # Build helper: strip print() for production
```

Shared with the Rust firmware and kept at the repo root: `../profiles/` (firing
profiles), `../static/` (embedded UI), `../scripts/` (offline analysis).

## First-time tuning

Tune before running real firings:

1. Open the Tuning page in the web UI, pick **SAFE** mode (first run), start it.
2. After it completes, analyze on your laptop (from the repo root):
   ```bash
   python scripts/analyze_tuning.py scripts/logs/tuning_*.csv
   python scripts/generate_thermal_model_config.py
   ```
3. Paste the generated `THERMAL_MODEL` into `config.py`, restart.

Tuning modes: **SAFE** (30–45 min), **STANDARD** (1–2 h), **THOROUGH** (3–4 h).
Full guide: [`docs/TUNING.md`](docs/TUNING.md).

## Gain scheduling (thermal model)

Temperature-range-specific PID gains in `config.py`:

```python
THERMAL_MODEL = [
    {'temp_min': 0,   'temp_max': 300,  'kp': 25.0, 'ki': 180.0, 'kd': 160.0},
    {'temp_min': 300, 'temp_max': 700,  'kp': 20.0, 'ki': 150.0, 'kd': 120.0},
    {'temp_min': 700, 'temp_max': 9999, 'kp': 15.0, 'ki': 100.0, 'kd': 80.0},
]
```

See [`docs/THERMAL_MODEL.md`](docs/THERMAL_MODEL.md) /
[`docs/THERMAL_MODEL_QUICK_START.md`](docs/THERMAL_MODEL_QUICK_START.md), and
[`docs/RATE_CONTROL.md`](docs/RATE_CONTROL.md) for rate control / stall detection.

## API endpoints

```
GET  /api/status            # temp, SSR state, PID gains, program status
GET  /api/info              # version, hardware info, uptime
GET  /api/profiles          # list profiles
POST /api/profiles/upload   # upload a profile
DELETE /api/profiles/<name> # delete a profile
POST /api/profiles/start    # start firing a profile
POST /api/profiles/stop     # stop current profile
POST /api/tuning/start      # start auto-tuning (mode, max_temp)
POST /api/tuning/stop       # stop tuning
GET  /api/tuning/status     # tuning progress / results
POST /api/pid/set           # set PID gains manually
GET  /api/logs              # list log files
```

## Docs

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — system design
- [`docs/TUNING.md`](docs/TUNING.md) — PID auto-tuning guide
- [`docs/THERMAL_MODEL.md`](docs/THERMAL_MODEL.md) — thermal modeling / gain scheduling
- [`docs/RATE_CONTROL.md`](docs/RATE_CONTROL.md) — rolling rate control & stall detection
- [`docs/DELAYED_START.md`](docs/DELAYED_START.md) — delayed-start firing
- [`docs/VALIDATION_REPORT.md`](docs/VALIDATION_REPORT.md) — validation notes
