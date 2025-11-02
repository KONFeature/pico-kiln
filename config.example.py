# config.example.py
# Configuration template for pico-kiln controller
# Copy this file to config.py and update with your settings

# === Hardware Pin Configuration ===

# MAX31856 Thermocouple SPI Pins
MAX31856_SPI_ID = 0  # SPI bus ID (0 or 1)
MAX31856_SCK_PIN = 18   # SPI Clock
MAX31856_MOSI_PIN = 19  # SPI MOSI (Master Out Slave In)
MAX31856_MISO_PIN = 16  # SPI MISO (Master In Slave Out)
MAX31856_CS_PIN = 28    # Chip Select

# SSR Control Pin(s)
# Single SSR (backward compatible):
# SSR_PIN = 15
# Multiple SSRs (staggered power-on to prevent inrush current):
# SSR_PIN = [15, 16, 17]
SSR_PIN = 15  # GPIO pin(s) for controlling the Solid State Relay(s)

# === WiFi Configuration ===
WIFI_SSID = "your_wifi_ssid"
WIFI_PASSWORD = "your_wifi_password"

# === Web Server Configuration ===
WEB_SERVER_PORT = 80
WEB_SERVER_HOST = "0.0.0.0"

# === Control Parameters ===
# Temperature reading interval (seconds)
TEMP_READ_INTERVAL = 1.0

# PID update interval (seconds)
PID_UPDATE_INTERVAL = 1.0

# === PID Parameters ===
# These should be tuned for your specific kiln
# Default values are conservative starting points
# Use auto-tuning utility to determine optimal values for your kiln
PID_KP = 25.0      # Proportional gain
PID_KI = 180.0     # Integral gain (inverse time constant)
PID_KD = 160.0     # Derivative gain

# === SSR Control ===
# Time-proportional control cycle time (seconds)
#
# This is the time window for PWM control of the SSR.
# For example, at 30% duty cycle with 20s cycle time:
#   - SSR is ON for 6 seconds
#   - SSR is OFF for 14 seconds
#   - Total: 3 switches per minute
#
# Why 20 seconds is appropriate for kilns:
# - Kilns have huge thermal mass and respond slowly to power changes
# - Short cycles (2s) = 30 switches/minute → excessive SSR wear + light flickering
# - Long cycles (20-30s) = 3 switches/minute → minimal wear + no flickering
# - Temperature precision is NOT affected (kiln responds over minutes, not seconds)
#
# Recommended values:
# - Small kilns (test kilns): 20 seconds
# - Medium/large kilns: 20-30 seconds
# - DO NOT use values < 10 seconds (causes flickering and SSR wear)
#
SSR_CYCLE_TIME = 20.0

# Stagger delay between multiple SSRs (seconds)
# When using multiple SSRs (SSR_PIN as list), this delay is applied between
# each SSR state change to prevent large inrush current draw.
# Recommended: 0.01 seconds (10ms) per SSR
# With 10ms delay, up to 10 SSRs can be supported within the 0.1s update window
# Set to 0 to disable staggering (not recommended for multiple SSRs)
SSR_STAGGER_DELAY = 0.01

# === Safety Limits ===
# Maximum safe temperature (°C)
MAX_TEMP = 1300

# Maximum temperature error before triggering safety shutdown (°C)
# If actual temp deviates from target by more than this, stop firing
MAX_TEMP_ERROR = 50

# === Temperature Settings ===
# Temperature units: "c" for Celsius, "f" for Fahrenheit
TEMP_UNITS = "c"

# Thermocouple type
# The MAX31856 supports various thermocouple types (B, E, J, K, N, R, S, T)
# Import ThermocoupleType from adafruit_max31856 to specify your thermocouple type
# Common types:
#   - ThermocoupleType.K: Type K (default) - General purpose, -270°C to 1372°C
#   - ThermocoupleType.R: Type R - High temp platinum, 0°C to 1768°C (ceramics/kilns)
#   - ThermocoupleType.S: Type S - High temp platinum, 0°C to 1768°C (ceramics/kilns)
#   - ThermocoupleType.J: Type J - Iron-constantan, -210°C to 1200°C
#   - ThermocoupleType.T: Type T - Copper-constantan, -270°C to 400°C
#
# Example configuration:
from adafruit_max31856 import ThermocoupleType
THERMOCOUPLE_TYPE = ThermocoupleType.K  # Change to match your thermocouple

# Thermocouple calibration offset (°C)
# Add this value to all temperature readings for calibration
THERMOCOUPLE_OFFSET = 0.0

# === Profile Settings ===
# Directory for storing firing profiles
PROFILES_DIR = "profiles"

# === Adaptive Rate Control ===
# Automatically adjust ramp rates if kiln cannot maintain desired rate
# This allows profiles to complete successfully even if kiln capacity is insufficient
#
# ADAPTATION_ENABLED: Enable/disable adaptive control
# ADAPTATION_CHECK_INTERVAL: How often to check if adaptation is needed (seconds)
# ADAPTATION_MIN_STEP_TIME: Minimum time in step before allowing first adaptation (seconds)
# ADAPTATION_MIN_TIME_BETWEEN: Minimum time between adaptations to avoid oscillation (seconds)
# ADAPTATION_TEMP_ERROR_THRESHOLD: Trigger adaptation if behind schedule by this many °C
# ADAPTATION_RATE_THRESHOLD: Trigger if actual rate < (current_rate * threshold)
# ADAPTATION_REDUCTION_FACTOR: Reduce rate to (measured_rate * factor) when adapting
# RATE_MEASUREMENT_WINDOW: Time window for measuring actual rate (seconds)
# RATE_RECORDING_INTERVAL: How often to record temp for rate calculation (seconds)
#
ADAPTATION_ENABLED = True
ADAPTATION_CHECK_INTERVAL = 60  # Check every minute
ADAPTATION_MIN_STEP_TIME = 600  # Wait 10 minutes before first adaptation
ADAPTATION_MIN_TIME_BETWEEN = 300  # Wait 5 minutes between adaptations
ADAPTATION_TEMP_ERROR_THRESHOLD = 20  # Trigger if 20°C behind schedule
ADAPTATION_RATE_THRESHOLD = 0.85  # Trigger if actual < 85% of target
ADAPTATION_REDUCTION_FACTOR = 0.95  # Reduce to 95% of measured rate
RATE_MEASUREMENT_WINDOW = 600  # Measure rate over 10 minutes
RATE_RECORDING_INTERVAL = 10  # Record temperature every 10 seconds

# === Data Logging Settings ===
# Directory for storing kiln run data logs (CSV files)
LOGS_DIR = "logs"

# Logging interval (seconds) - how often to write data to CSV
# Lower values = more data points but more memory usage
# Default: 30 seconds (saves ~120 data points for a 1-hour firing)
LOGGING_INTERVAL = 30

# === Program Recovery Settings ===
# Automatic program recovery after unexpected reboot/crash
# If a kiln program was running when the device rebooted, it will attempt
# to resume automatically if the conditions below are met
#
# MAX_RECOVERY_DURATION: Maximum time since last log entry to attempt recovery (seconds)
# If more time has passed, recovery is considered unsafe and program is abandoned
# Default: 300 seconds (5 minutes)
MAX_RECOVERY_DURATION = 300

# MAX_RECOVERY_TEMP_DELTA: Maximum temperature deviation from last logged value (°C)
# If current temperature differs by more than this, recovery is considered unsafe
# Default: 30°C
MAX_RECOVERY_TEMP_DELTA = 30

# === Watchdog Timer Settings ===
# Hardware watchdog timer for automatic recovery from control loop hangs
#
# The watchdog runs on Core 1 (control thread) and monitors the critical control loop.
# If the control loop hangs or crashes, the watchdog will automatically reset the Pico
# after WATCHDOG_TIMEOUT milliseconds. The recovery system will then automatically
# resume the interrupted kiln program (if within recovery time window).
#
# ENABLE_WATCHDOG: Set to True to enable watchdog protection
# WATCHDOG_TIMEOUT: Time in milliseconds before watchdog resets (default: 8000ms = 8s)
#
# How it works:
# 1. Control loop iteration completes successfully → watchdog is fed
# 2. Control loop hangs or crashes → watchdog NOT fed
# 3. After WATCHDOG_TIMEOUT ms → Pico resets automatically
# 4. On boot → Recovery system detects interrupted program
# 5. If within MAX_RECOVERY_DURATION → Program resumes automatically
#
# Safety notes:
# - Watchdog timeout (8000ms) must be longer than control loop interval (1000ms)
# - Current settings: 8x safety margin (8s timeout / 1s loop = 8x)
# - Core 2 failures (web server, WiFi) do NOT trigger watchdog - kiln keeps firing
# - Only critical control loop hangs will cause reset
#
# When to enable:
# ✓ After thorough testing of your kiln profiles
# ✓ For unattended operation (no laptop connected)
# ✓ When MAX_RECOVERY_DURATION and MAX_RECOVERY_TEMP_DELTA are properly configured
#
# When to disable:
# ✗ During development and debugging
# ✗ When connected to REPL/laptop (watchdog prevents debugging)
# ✗ If control loop legitimately takes >8 seconds (increase WATCHDOG_TIMEOUT instead)
#
ENABLE_WATCHDOG = False
WATCHDOG_TIMEOUT = 8000  # milliseconds (8 seconds)
