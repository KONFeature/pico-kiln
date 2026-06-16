# Debug Tools

This directory contains debugging tools for diagnosing standalone boot issues on the Raspberry Pi Pico 2.

## Problem

MicroPython code that works perfectly when run via `mpremote run main.py` may fail silently when the Pico boots standalone (powered without USB console). This happens because:

- REPL mode shows all exceptions and output
- Standalone mode has no console - crashes are completely silent
- LED initialization happens late in boot (if at all)

## Solution

This debug toolkit provides:

1. **Early LED feedback** - Blink patterns indicate boot progress
2. **File-based logging** - Errors are written to files that survive crashes
3. **Stage-by-stage monitoring** - Identify exactly where boot fails
4. **Easy mode switching** - Single command to enable/disable debug modes

## Quick Start

From the project root:

```bash
# Install minimal debug mode
./debug.sh --install-minimal

# Power cycle Pico and watch LED patterns

# Read logs
./debug.sh --logs

# Restore original when done
./debug.sh --restore
```

## Available Tools

### 1. debug_boot.py
Minimal diagnostic tool that tests each import stage with LED feedback.

**Use when:** Initial diagnosis of boot failures

**LED Patterns:**
- 1 blink: Boot started
- 2 blinks: Basic imports OK (asyncio, _thread, network)
- 3 blinks: Config loaded
- 4 blinks: Project imports OK (server/kiln modules)
- 5 blinks: All successful!
- Fast blink (10Hz): Error - check logs
- Slow blink (0.5Hz): Success

**Logs:** `/boot_debug.log`

### 2. main_safe.py
Full production wrapper with comprehensive error catching and logging.

**Use when:** Ongoing protection during development, or after identifying initial issue

**LED Patterns:**
- 1-9 blinks: Boot stages (infrastructure, Core 1, WiFi, etc.)
- Solid ON: System running normally
- Fast blink: Fatal error - check logs

**Logs:** `/boot_stages.log`, `/boot_error.log`

### 3. read_boot_logs.py
Python script to read debug logs from the Pico.

**Usage:**
```bash
python3 read_boot_logs.py           # Read once
python3 read_boot_logs.py --watch   # Continuous monitoring
```

## Debug Script Usage

The `debug.sh` script at the project root manages all debug modes:

```bash
# Install debug modes
./debug.sh --install-minimal    # Install minimal debug boot
./debug.sh --install-safe       # Install safe boot mode
./debug.sh --restore            # Restore original main.py

# Read logs
./debug.sh --logs               # Read logs once
./debug.sh --watch              # Watch logs in real-time
./debug.sh --clean              # Clear debug logs on Pico

# Manage backups
./debug.sh --list-backups       # List all backups
./debug.sh --status             # Show current status

# Help
./debug.sh --help               # Show all options
```

## Workflow

### Initial Diagnosis

1. **Install minimal debug:**
   ```bash
   ./debug.sh --install-minimal
   ```

2. **Power cycle Pico** (unplug/replug)

3. **Watch LED pattern** - count the blinks

4. **Read logs:**
   ```bash
   ./debug.sh --logs
   ```

5. **Identify issue** from logs (missing module, syntax error, etc.)

6. **Fix issue** in code

7. **Restore original:**
   ```bash
   ./debug.sh --restore
   ```

### Ongoing Protection

1. **Install safe mode:**
   ```bash
   ./debug.sh --install-safe
   ```

2. **Monitor during development:**
   ```bash
   ./debug.sh --watch
   ```

3. **Keep safe mode** as your main.py for built-in protection

## Common Issues

### WiFi Configuration Errors
**Symptom:** Boot hangs during WiFi connection
**Log shows:** Timeout or connection errors
**Fix:** Check `WIFI_SSID`, `WIFI_PASSWORD` in config.py

### Missing Module
**Symptom:** Stops at import stage (2-4 blinks)
**Log shows:** `ImportError: no module named 'xyz'`
**Fix:** Verify all files deployed: `mpremote fs ls`

### Syntax Error
**Symptom:** Fast blink during imports
**Log shows:** `SyntaxError` with line number
**Fix:** Check syntax in the reported file

### Memory Error
**Symptom:** Random crashes, `MemoryError`
**Log shows:** `MemoryError` exception
**Fix:** Reduce queue sizes, imports, or cache sizes

### Hardware Timeout
**Symptom:** Stops at Stage 8 (Core 1 ready)
**Log shows:** "Core 1 not ready"
**Fix:** Check MAX31856, SSR connections and pin config

## Files

- `debug_boot.py` - Minimal diagnostic boot
- `main_safe.py` - Safe boot wrapper
- `read_boot_logs.py` - Log reader utility
- `BOOT_DEBUG_GUIDE.md` - Comprehensive debugging guide
- `README.md` - This file

## Backups

The debug script automatically creates backups in `.debug_backups/` when installing debug modes:

```bash
# List backups
./debug.sh --list-backups

# Restore latest backup
./debug.sh --restore
```

Backups are timestamped and include a `main_latest.py` for easy restoration.

## More Information

See `BOOT_DEBUG_GUIDE.md` for:
- Detailed LED pattern reference
- Advanced debugging techniques
- Prevention strategies
- Complete troubleshooting guide

### 4. debug_lcd.py
Comprehensive LCD diagnostic tool that tests initialization, display operations, and character encodings.

**Use when:** Troubleshooting LCD display issues, testing I2C connections, or verifying character encoding

**What it tests:**
- I2C bus scanning for connected devices
- LCD initialization at different addresses (0x27, 0x3F)
- Basic display operations (clear, print, cursor positioning)
- Character encodings (ASCII, special characters, degree symbols)
- Backlight control
- Rapid display updates

**Usage:**
```bash
# Run the debug script directly from REPL
mpremote run debug/debug_lcd.py
```

**Output:**
All diagnostic information is printed to the console with timestamps, showing:
- I2C configuration and scan results
- Step-by-step LCD initialization progress
- Test results for each operation (text display, encoding, etc.)
- Any errors with full stack traces

**Common Issues Diagnosed:**
- Wrong I2C address (script tests 0x27 and 0x3F)
- Faulty wiring (I2C bus scan shows connected devices)
- Character encoding problems (tests various encoding methods)
- LCD initialization failures (detailed step-by-step logging)
- Display update issues (tests rapid updates)

### 5. debug_thermocouple.py
Comprehensive MAX31856 thermocouple diagnostic tool that tests SPI initialization, sensor configuration, and temperature readings.

**Use when:** Troubleshooting thermocouple sensor issues, verifying SPI connections, or testing temperature accuracy

**What it tests:**
- SPI bus initialization with correct parameters
- MAX31856 sensor initialization and configuration
- Multiple temperature readings (default: 5 samples)
- Fault detection and reporting (open circuit, voltage, range faults, etc.)
- Cold junction (reference) temperature reading
- Sensor configuration verification (averaging, noise rejection, thresholds)
- Temperature stability analysis
- Different averaging settings (1, 2, 4, 8, 16 samples)

**Usage:**
```bash
# Run the debug script directly from REPL
mpremote run debug/debug_thermocouple.py
```

**Output:**
All diagnostic information is printed to the console with timestamps, showing:
- SPI configuration (bus, pins, baudrate)
- Sensor initialization progress
- Temperature readings with fault status
- Cold junction temperature
- Statistical analysis (average, min, max, range)
- Any faults or errors with details

**Common Issues Diagnosed:**
- SPI wiring problems (incorrect pin connections)
- Thermocouple not connected (open circuit fault)
- Wrong thermocouple type configuration
- Temperature out of range (sensor or wiring fault)
- Unstable readings (electrical noise, poor connections)
- Sensor configuration errors

## Tips

- **Always test standalone** after deploying changes
- **Use --watch** during active debugging for real-time feedback
- **Keep safe mode** enabled during development
- **Clean logs** periodically with `./debug.sh --clean`
- **Check status** with `./debug.sh --status` to verify current mode
