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

# SSR Control Pin
SSR_PIN = 15  # GPIO pin for controlling the Solid State Relay

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
# Longer cycle = less SSR switching, but less precise control
SSR_CYCLE_TIME = 2.0

# === Safety Limits ===
# Maximum safe temperature (°C)
MAX_TEMP = 1300

# Maximum temperature error before triggering safety shutdown (°C)
# If actual temp deviates from target by more than this, stop firing
MAX_TEMP_ERROR = 50

# === Temperature Settings ===
# Temperature units: "c" for Celsius, "f" for Fahrenheit
TEMP_UNITS = "c"

# Thermocouple calibration offset (°C)
# Add this value to all temperature readings for calibration
THERMOCOUPLE_OFFSET = 0.0

# === Profile Settings ===
# Directory for storing firing profiles
PROFILES_DIR = "profiles"

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
