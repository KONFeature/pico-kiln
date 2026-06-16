# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

A kiln controller for the Raspberry Pi Pico 2 (RP2350), inspired by [jbruce12000/kiln-controller](https://github.com/jbruce12000/kiln-controller).

The firmware exists in two implementations:
- **`rust/`** — current/primary firmware (Rust + Embassy). See `rust/ARCHITECTURE.md`.
- **`python/`** — original MicroPython implementation (this was the whole project before the Rust port). See `python/README.md`.

`web/` is the React/Tauri app; `scripts/`, `profiles/`, and `static/` are shared by both firmwares (analysis tools, firing profiles, embedded web assets).

## Hardware Target

- **Platform**: Raspberry Pi Pico 2 W (dual-core RP2350)
- **Runtime**: MicroPython
- **Temperature Sensor**: MAX31856 thermocouple board (SPI)
- **Output**: SSR (Solid State Relay) control
- **Display**: LCD1602 I2C display (optional)

## Architecture

The system uses both cores of the Pico 2:

### Core 1: Kiln Control (Primary)
- Temperature reading from MAX31856
- PID control algorithm with adaptive tuning
- SSR toggling based on control decisions
- Bare-metal, time-critical operations

### Core 2: Web Interface & Monitoring
- Web server for monitoring and control
- Program upload capability (kiln firing schedules)
- Real-time state monitoring (current temp, SSR status, program progress)
- Data logging and recovery

## Project Structure

```
pico-kiln/
├── rust/                # Current firmware — Rust + Embassy (RP2350). See rust/ARCHITECTURE.md
├── python/              # Original firmware — MicroPython
│   ├── main.py / boot.py / config.example.py
│   ├── kiln/            # Core 1: control (control_thread, pid, tuner, scheduler,
│   │                    #         rate_monitor, profile, hardware, state, comms)
│   ├── server/          # Core 2: web_server, wifi_manager, data_logger, recovery, ...
│   ├── lib/             # MicroPython libraries (adafruit_max31856, lcd1602_i2c, ...)
│   ├── debug/           # Hardware/boot debug scripts
│   ├── docs/            # Firmware docs (tuning, thermal model, rate control, ...)
│   └── *.sh             # compile / deploy / debug / dump_logs / clean_logs / sync_profiles
├── web/                 # React web app (desktop/mobile) — src/ + src-tauri/
├── scripts/             # Python 3 analysis tools (SHARED) — plot_run, analyze_*, analyzer/
├── profiles/            # Firing profiles, JSON (SHARED by both firmwares)
└── static/              # Embedded web assets (SHARED — baked by rust, served by python)
```

## Build & Deploy Commands

### Rust firmware (in `rust/`)
Current firmware. Build/flash/test instructions: `rust/ARCHITECTURE.md`, `rust/TESTING.md`. Run cargo from `rust/`.

### MicroPython firmware (in `python/`)
Run these from the `python/` directory:
```bash
# Compile Python to .mpy bytecode (faster execution)
./compile.sh                    # Development build
./compile.sh --production       # Production build (minified, optimized)

# Deploy to Pico 2 via USB
./deploy.sh                     # Deploy compiled or source files
./deploy.sh --clean             # Clean deploy (removes existing files first)

# Debug and logging
./debug.sh                      # Interactive debugging session
./dump_logs.sh                  # Download logs from Pico (into ../scripts/logs/)
./clean_logs.sh                 # Clear logs on Pico

# Profile management (uploads from the shared ../profiles dir)
./sync_profiles.sh              # Sync profiles to Pico
```

### Web Application (in `web/` directory)
```bash
cd web

# Development
bun install                     # Install dependencies (or npm install)
bun run dev                     # Start dev server on port 3000

# Testing & Linting
bun run test                    # Run Vitest tests
bun run lint                    # Run Biome linter
bun run check                   # Run Biome checks
bun run format                  # Format code with Biome

# Production build
bun run build                   # Build for web
bun run tauri:build             # Build desktop app
bun run tauri:android:build     # Build Android app
```

### Analysis Scripts (Python 3)
```bash
# Plot a firing run
python scripts/plot_run.py logs/run_*.csv

# Analyze PID performance
python scripts/analyze_pid_performance.py logs/run_*.csv

# Analyze heat loss characteristics
python scripts/analyze_heat_loss.py logs/run_*.csv
```

## Configuration

- **MicroPython firmware**: copy `python/config.example.py` to `python/config.py`, set WiFi credentials, PID parameters, and hardware pins, then deploy.
- **Rust firmware**: `rust/config.json` (`KilnConfig`), with the same UPPER_SNAKE keys as `config.py`. See `AGENTS.md` → "Rust workspace".

## Development Guidelines

### MicroPython Code (python/kiln/, python/server/, python/lib/)
- Must be MicroPython-compatible (no standard library features unavailable in MicroPython)
- Keep memory usage minimal (Pico has limited RAM)
- Use `const()` for constant values
- Avoid dynamic imports where possible

### Web Application (web/)
- React 19 with TypeScript
- TanStack Router for routing
- TanStack Query for data fetching
- Tailwind CSS v4 for styling
- Biome for linting/formatting (not ESLint/Prettier)

### Analysis Scripts (scripts/)
- Standard Python 3 with matplotlib, numpy, pandas
- Used for offline analysis and PID tuning

## Key Documentation

- `rust/ARCHITECTURE.md` - Rust firmware design + Rust↔Python mapping
- `python/README.md` - MicroPython firmware setup/deploy guide
- `python/docs/ARCHITECTURE.md` - MicroPython system architecture
- `python/docs/TUNING.md` - PID tuning guide
- `python/docs/RATE_CONTROL.md` - Rolling rate control and stall detection
- `python/docs/THERMAL_MODEL.md` - Thermal model documentation
- `web/README.md` - Web application details
