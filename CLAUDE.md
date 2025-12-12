# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a kiln controller designed to run on MicroPython (Raspberry Pi Pico 2 W). The project is inspired by [jbruce12000/kiln-controller](https://github.com/jbruce12000/kiln-controller) but reimplemented for MicroPython to simplify integration with the MAX31856 thermocouple board.

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
├── main.py              # Entry point
├── boot.py              # MicroPython boot configuration
├── config.example.py    # Configuration template (copy to config.py)
├── kiln/                # Core kiln control modules (MicroPython)
│   ├── control_thread.py  # Main control loop
│   ├── pid.py            # PID controller
│   ├── tuner.py          # PID auto-tuning
│   ├── hardware.py       # Hardware abstraction (thermocouple, SSR)
│   ├── profile.py        # Firing profile management
│   ├── scheduler.py      # Profile scheduling
│   ├── state.py          # Shared state management
│   └── comms.py          # Inter-core communication
├── server/              # Web server modules (MicroPython)
│   ├── web_server.py     # HTTP server
│   ├── wifi_manager.py   # WiFi connection management
│   ├── data_logger.py    # Temperature/event logging
│   └── recovery.py       # Crash recovery
├── lib/                 # MicroPython libraries
│   ├── adafruit_max31856.py  # Thermocouple driver
│   └── lcd1602_i2c.py        # LCD display driver
├── web/                 # React web application (desktop/mobile)
│   ├── src/             # React + TypeScript source
│   └── src-tauri/       # Tauri desktop/mobile app config
├── scripts/             # Python 3 analysis & utility scripts
│   ├── plot_run.py      # Plot firing run data
│   ├── analyze_pid_performance.py  # PID tuning analysis
│   ├── analyze_heat_loss.py        # Thermal analysis
│   └── analyzer/        # Shared analysis modules
├── profiles/            # Firing profile definitions (JSON)
├── debug/               # Debug and test scripts
└── static/              # Static web assets for embedded server
```

## Build & Deploy Commands

### Pico 2 Deployment
```bash
# Compile Python to .mpy bytecode (faster execution)
./compile.sh                    # Development build
./compile.sh --production       # Production build (minified, optimized)

# Deploy to Pico 2 via USB
./deploy.sh                     # Deploy compiled or source files
./deploy.sh --clean             # Clean deploy (removes existing files first)

# Debug and logging
./debug.sh                      # Interactive debugging session
./dump_logs.sh                  # Download logs from Pico
./clean_logs.sh                 # Clear logs on Pico

# Profile management
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

1. Copy `config.example.py` to `config.py`
2. Configure WiFi credentials, PID parameters, and hardware pins
3. Deploy to Pico 2

## Development Guidelines

### MicroPython Code (kiln/, server/, lib/)
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

- `ARCHITECTURE.md` - Detailed system architecture
- `TUNING.md` - PID tuning guide
- `ADAPTIVE_CONTROL.md` - Adaptive control system documentation
- `THERMAL_MODEL.md` - Thermal model documentation
- `web/README.md` - Web application details
