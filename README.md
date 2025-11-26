# pico-kiln

A sophisticated kiln controller running on MicroPython for the Raspberry Pi Pico 2, featuring advanced PID auto-tuning with thermal modeling and gain scheduling.

Greatly inspired by: https://github.com/jbruce12000/kiln-controller

## Overview

This project implements a professional-grade kiln temperature controller using the Raspberry Pi Pico 2's dual-core architecture:

- **Core 1**: Handles time-critical kiln control operations
  - Reads temperature from MAX31856 thermocouple board (SPI)
  - Runs PID control algorithm with adaptive gain scheduling
  - Controls SSR (Solid State Relay) based on PID output
  - Executes firing profiles with precise temperature control

- **Core 2**: Provides web interface and monitoring
  - Serves web interface for real-time monitoring and control
  - Allows uploading and managing kiln firing programs
  - Displays current temperature, SSR status, and program state
  - Logs temperature data to CSV files
  - Provides multi-mode PID auto-tuning system

## Key Features

### 🔥 Firing Profile Management
- Upload custom firing profiles (JSON format)
- Multi-segment temperature ramps and holds
- Real-time progress monitoring
- Program recovery after power loss

### 🎯 Advanced PID Auto-Tuning
- **Three tuning modes** for different needs:
  - **SAFE** (30-45 min): Quick safety verification for new kilns
  - **STANDARD** (1-2 hours): Balanced characterization for good PID data
  - **THOROUGH** (3-4 hours): Comprehensive thermal modeling across full range
- Step-based sequences with plateau detection
- Automatic data logging for offline analysis

### 🧠 Thermal Modeling & Gain Scheduling
- **Multi-method PID calculation**: Ziegler-Nichols, Cohen-Coon, AMIGO, Lambda tuning
- **Temperature-range-specific PID parameters**: Different gains for LOW/MID/HIGH temps
- Automatic gain switching based on current temperature
- Significantly improved control across 0-1300°C range
- Reduced overshoot and faster settling

### 📊 Comprehensive Analysis Tools
- Advanced thermal characterization (dead time, time constant, heat loss modeling)
- Multiple PID calculation methods with recommendations
- Test quality assessment (EXCELLENT/GOOD/POOR scoring)
- Beautiful terminal reports and JSON export
- Auto-generation of config snippets

## Hardware Requirements

- Raspberry Pi Pico 2 (RP2350) with MicroPython
- MAX31856 thermocouple board (SPI interface)
- SSR (Solid State Relay) for kiln control
- K-type thermocouple (or other supported types)
- WiFi connectivity (Pico 2 W) for web interface

## Quick Start

### 1. Hardware Setup

1. Flash MicroPython on the Pico 2: https://micropython.org/download/RPI_PICO2_W/
2. Wire the MAX31856 board to the Pico 2's SPI pins
3. Connect the SSR to GPIO 15 (or configure in config.py)
4. Install thermocouple in kiln

### 2. Software Setup

```bash
# 1. Configure the project
cp config.example.py config.py
# Edit config.py with your WiFi credentials and pin settings

# 2. Copy all files to the Pico 2's filesystem
# (Main Python files, kiln/ directory, server/ directory, static/ directory)

# 3. Reset the Pico 2
# Watch serial console for IP address

# 4. Access web interface
# Navigate to http://<pico-ip-address>
```

### 3. First-Time Tuning

**Before running any firing programs, tune your kiln:**

1. Navigate to the Tuning page in web interface
2. Select **SAFE mode** (recommended for first run)
3. Click "Start Tuning" and wait 30-45 minutes
4. Analyze results on your laptop:
   ```bash
   python analyze_tuning.py logs/tuning_YYYY-MM-DD_HH-MM-SS.csv
   python generate_thermal_model_config.py
   ```
5. Copy generated THERMAL_MODEL to config.py
6. Restart controller

**See [TUNING.md](TUNING.md) for complete guide**

## Project Structure

```
pico-kiln/
├── config.py                      # Hardware and WiFi configuration (user-created)
├── config.example.py              # Configuration template
├── main.py                        # Entry point - asyncio setup and initialization
│
├── kiln/                          # Core kiln control (runs on Core 1)
│   ├── __init__.py
│   ├── control_thread.py          # Main control loop
│   ├── state.py                   # Controller state machine
│   ├── pid.py                     # PID controller with anti-windup
│   ├── pid_scheduler.py           # Temperature-based gain scheduling
│   ├── tuner.py                   # Multi-mode PID auto-tuning
│   ├── profile.py                 # Firing profile management
│   ├── comms.py                   # Inter-thread communication
│   ├── max31856.py                # MAX31856 thermocouple driver
│   └── ssr.py                     # SSR control with PWM
│
├── server/                        # Web server (runs on Core 2)
│   ├── __init__.py
│   ├── web_server.py              # HTTP server and API endpoints
│   ├── data_logger.py             # CSV data logging
│   └── recovery.py                # Program recovery after power loss
│
├── static/                        # Web interface assets
│   ├── index.html                 # Main dashboard
│   ├── tuning.html                # PID tuning interface
│   ├── profiles.html              # Profile management
│   └── styles.css                 # Shared styles
│
├── profiles/                      # Firing profile storage (JSON)
│
├── logs/                          # Temperature data logs (CSV)
│
├── scripts/                       # Analysis scripts (run on laptop)
│   ├── analyze_tuning.py          # PID tuning data analysis
│   ├── analyze_final_climb.py     # Final 100°C climb rate analysis for pottery
│   ├── analyze_heat_loss.py       # Heat loss and energy efficiency analysis
│   ├── generate_thermal_model_config.py  # Config snippet generator
│   ├── plot_run.py                # Visualize kiln run data
│   └── compare_runs.py            # Compare multiple runs
│
├── README.md                      # This file
├── TUNING.md                      # Complete tuning guide
├── THERMAL_MODEL.md               # Thermal modeling documentation
├── THERMAL_MODEL_QUICK_START.md   # Quick reference
├── CLAUDE.md                      # Development guidance for AI assistants
└── feedback.md                    # Development notes
```

## Web Interface

### Main Dashboard (`/`)
- Real-time temperature display
- Current firing program status
- SSR output percentage and state
- Elapsed time and progress
- Program start/stop controls

### Tuning Interface (`/tuning.html`)
- Mode selection (SAFE/STANDARD/THOROUGH)
- Real-time tuning progress with step indicators
- Plateau detection status
- Current PID gains display
- Download tuning data

### Profile Management (`/profiles.html`)
- Upload firing profiles (JSON)
- View existing profiles
- Delete profiles
- Profile validation

## API Endpoints

### Status & Information
- `GET /api/status` - System status (temperature, SSR state, PID gains, program status)
- `GET /api/info` - System information (version, hardware info, uptime)

### Profile Management
- `GET /api/profiles` - List all firing profiles
- `POST /api/profiles/upload` - Upload new profile
- `DELETE /api/profiles/<name>` - Delete profile
- `POST /api/profiles/start` - Start firing profile
- `POST /api/profiles/stop` - Stop current profile

### PID Tuning
- `POST /api/tuning/start` - Start auto-tuning (with mode and max_temp parameters)
- `POST /api/tuning/stop` - Stop tuning
- `GET /api/tuning/status` - Tuning progress and results

### Configuration
- `POST /api/pid/set` - Update PID gains manually
- `GET /api/logs` - List available log files

## Temperature-Range-Specific PID (Gain Scheduling)

The controller supports **thermal modeling** for improved control across wide temperature ranges:

```python
# In config.py:
THERMAL_MODEL = [
    {'temp_min': 0, 'temp_max': 300, 'kp': 25.0, 'ki': 180.0, 'kd': 160.0},
    {'temp_min': 300, 'temp_max': 700, 'kp': 20.0, 'ki': 150.0, 'kd': 120.0},
    {'temp_min': 700, 'temp_max': 9999, 'kp': 15.0, 'ki': 100.0, 'kd': 80.0}
]
```

**Benefits:**
- ✅ Reduced overshoot during temperature ramps
- ✅ Faster settling at target temperatures
- ✅ Better control across 0-1300°C range
- ✅ Automatically switches gains based on current temperature

**See [THERMAL_MODEL.md](THERMAL_MODEL.md) for complete guide**

## Offline Analysis Tools

### analyze_tuning.py
Analyzes tuning data and calculates optimal PID parameters using multiple methods:

```bash
python analyze_tuning.py logs/tuning_2025-10-21_11-32-41.csv

# Show only specific method
python analyze_tuning.py logs/tuning_2025-10-21_11-32-41.csv --method amigo
```

**Features:**
- Multi-phase detection (heating, cooling, plateau)
- Thermal model fitting (dead time, time constant, heat loss)
- 4 PID calculation methods (Ziegler-Nichols, Cohen-Coon, AMIGO, Lambda)
- Temperature-range-specific PID parameters
- Test quality assessment
- Beautiful terminal reports + JSON export

### generate_thermal_model_config.py
Generates ready-to-paste config snippets from tuning results:

```bash
python generate_thermal_model_config.py

# Output:
# THERMAL_MODEL = [
#     {'temp_min': 0, 'temp_max': 300, 'kp': 25.0, 'ki': 180.0, 'kd': 160.0},
#     ...
# ]
```

### analyze_final_climb.py
Analyzes the final 100°C climb rate for pottery firing verification:

```bash
python scripts/analyze_final_climb.py logs/cone6_firing.csv

# Analyze last 120°C instead
python scripts/analyze_final_climb.py logs/glaze.csv --climb 120

# Save results to JSON
python scripts/analyze_final_climb.py logs/bisque.csv --output report.json
```

**Features:**
- Identifies the maximum temperature reached
- Calculates heating rate for the last 100°C (configurable)
- Detects and accounts for hold periods
- Compares rate against Orton cone chart ranges
- Essential for verifying cone equivalence

**Use Case:** After a firing, use this tool to verify the heating rate during the critical final climb. This rate determines which Orton cone chart column to use when comparing against actual cone behavior.

### analyze_heat_loss.py
Analyzes heat loss characteristics and energy efficiency:

```bash
python scripts/analyze_heat_loss.py logs/firing.csv --volume 50 --power 5000

# With custom ambient temperature
python scripts/analyze_heat_loss.py logs/firing.csv -v 50 -p 5000 --ambient 20

# Save detailed report
python scripts/analyze_heat_loss.py logs/firing.csv -v 50 -p 5000 -o heat_loss.json
```

**Features:**
- Calculates heat loss at full power (100% SSR)
- Analyzes cooling periods to estimate heat loss coefficient
- Shows energy efficiency at different temperatures
- Identifies insulation effectiveness
- Estimates power loss in watts

**Use Case:** Understand how much heating power is being lost to the environment at different temperatures. Higher heat loss at high temperatures indicates areas where improved insulation could save energy and improve firing performance.

## Documentation

- **[TUNING.md](TUNING.md)** - Complete PID auto-tuning guide
  - Multi-mode tuning system (SAFE/STANDARD/THOROUGH)
  - Step-by-step workflow
  - Troubleshooting

- **[THERMAL_MODEL.md](THERMAL_MODEL.md)** - Thermal modeling and gain scheduling
  - Architecture and design
  - Configuration guide
  - Testing recommendations

- **[THERMAL_MODEL_QUICK_START.md](THERMAL_MODEL_QUICK_START.md)** - Quick reference
  - 5-step setup process
  - Common configurations

- **[CLAUDE.md](CLAUDE.md)** - Development guidelines for AI assistants

## Development

### Testing Hardware Connections

```python
# On the Pico's serial console:
from kiln.max31856 import MAX31856
sensor = MAX31856(spi, cs_pin)
temp = sensor.read_temperature()
print(f"Temperature: {temp}°C")
```

### Simulating Kiln Behavior

The project includes simulation capabilities for testing control algorithms without real hardware.

### Contributing

Contributions are welcome! Key areas:
- Additional PID tuning methods
- Improved thermal modeling
- Web interface enhancements
- Documentation improvements

## Safety Notes

⚠️ **Important Safety Information:**

- This controller manages high-temperature equipment that can cause fires
- Always supervise kiln operation
- Ensure proper ventilation
- Test thoroughly before unattended operation
- Have fire suppression equipment nearby
- Follow all local electrical and fire safety codes
- Never exceed your kiln's rated temperature
- Use appropriate thermocouples rated for your max temperature

## License

This project is licensed under the **PolyForm Noncommercial License 1.0.0**.

**You are free to:**
- Use this software for personal, educational, and research projects
- Modify and distribute the software
- Study and learn from the code

**Restrictions:**
- No commercial use (individuals or corporations)
- See [LICENSE](LICENSE) for full terms

For commercial licensing inquiries, please contact the project maintainer.

## Acknowledgments

- Inspired by [jbruce12000/kiln-controller](https://github.com/jbruce12000/kiln-controller)
- PID tuning methods based on classical control theory (Ziegler-Nichols, Cohen-Coon, AMIGO)
- MicroPython community for excellent embedded Python support

## Support

For issues, questions, or contributions:
1. Check documentation (TUNING.md, THERMAL_MODEL.md)
2. Review troubleshooting sections
3. Open an issue with:
   - Controller logs
   - Tuning results (if applicable)
   - Description of the problem
   - Hardware specifications
