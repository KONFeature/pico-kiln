# config.example.py
# Configuration template for pico-kiln controller
# Copy this file to config.py and update with your settings

# ============================================================================
# HARDWARE CONFIGURATION
# ============================================================================

# MAX31856 Thermocouple (SPI Interface)
MAX31856_SPI_ID = 0      # SPI bus ID (0 or 1)
MAX31856_SCK_PIN = 18    # SPI Clock
MAX31856_MOSI_PIN = 19   # SPI MOSI (Master Out Slave In)
MAX31856_MISO_PIN = 16   # SPI MISO (Master In Slave Out)
MAX31856_CS_PIN = 28     # Chip Select

# SSR Control Pin(s)
# Single SSR:   SSR_PIN = 15
# Multiple SSRs (for high-power kilns): SSR_PIN = [15, 16, 17]
SSR_PIN = 15

# ============================================================================
# NETWORK CONFIGURATION
# ============================================================================

WIFI_SSID = "your_wifi_ssid"
WIFI_PASSWORD = "your_wifi_password"

# Static IP Configuration (optional - leave commented for DHCP)
# Setting static IP can speed up connection by ~1-2 seconds
# Uncomment and configure all 4 values to enable:
# WIFI_STATIC_IP = "192.168.1.100"      # Static IP address for Pico
# WIFI_SUBNET = "255.255.255.0"         # Subnet mask
# WIFI_GATEWAY = "192.168.1.1"          # Router gateway address
# WIFI_DNS = "8.8.8.8"                  # DNS server (Google DNS or router IP)

# Web Server
WEB_SERVER_HOST = "0.0.0.0"
WEB_SERVER_PORT = 80

# ============================================================================
# TEMPERATURE SENSOR CONFIGURATION
# ============================================================================

# Thermocouple type - MAX31856 supports: B, E, J, K, N, R, S, T
# Common types:
#   K: General purpose, -270°C to 1372°C (most common)
#   R/S: High-temp platinum, 0°C to 1768°C (ceramics/kilns)
from adafruit_max31856 import ThermocoupleType
THERMOCOUPLE_TYPE = ThermocoupleType.K

# Temperature units: "c" (Celsius) or "f" (Fahrenheit)
TEMP_UNITS = "c"

# Calibration offset added to all readings (°C)
THERMOCOUPLE_OFFSET = 0.0

# ============================================================================
# CONTROL LOOP TIMING
# ============================================================================

TEMP_READ_INTERVAL = 1.0   # Temperature sensor read interval (seconds)
PID_UPDATE_INTERVAL = 1.0  # PID calculation interval (seconds)

# ============================================================================
# PID CONTROL PARAMETERS
# ============================================================================

# Base PID gains - tune these for your specific kiln
# Use the auto-tuning utility: python scripts/analyze_tuning.py
PID_KP_BASE = 25.0   # Proportional gain
PID_KI_BASE = 0.14   # Integral gain (converted from time constant)
PID_KD_BASE = 160.0  # Derivative gain

# Continuous Gain Scheduling (optional)
# Compensates for increased heat loss at higher temperatures
# Set THERMAL_H = 0 to disable (constant PID gains)
#
# How to generate:
#   1. Run full-range PID tuning
#   2. Run: python scripts/analyze_tuning.py logs/tuning_*.csv
#   3. Copy THERMAL_* and PID_*_BASE values from output
THERMAL_H = 0.0          # Heat loss coefficient (0 = disabled)
THERMAL_T_AMBIENT = 25.0 # Ambient temperature (°C)

# ============================================================================
# SSR POWER CONTROL
# ============================================================================

# Time-proportional control cycle time (seconds)
# Example: 30% duty cycle with 20s cycle → ON 6s, OFF 14s (3 switches/min)
#
# Why 20 seconds for kilns:
#   - Kilns have huge thermal mass (respond over minutes, not seconds)
#   - Short cycles (2s) → 30 switches/min → SSR wear + light flickering
#   - Long cycles (20s) → 3 switches/min → minimal wear, no flickering
#   - Temperature precision NOT affected by cycle length
#
# Recommended: 20s (small kilns) to 30s (large kilns)
# DO NOT use < 10s (causes flickering and SSR wear)
SSR_CYCLE_TIME = 20.0

# Stagger delay between multiple SSRs (seconds)
# Prevents inrush current when using multiple SSRs (SSR_PIN as list)
# Recommended: 0.01s (10ms) per SSR, supports up to 10 SSRs
SSR_STAGGER_DELAY = 0.01

# ============================================================================
# SAFETY LIMITS
# ============================================================================

MAX_TEMP = 1300          # Maximum safe temperature (°C)
MAX_TEMP_ERROR = 50      # Max deviation from target before emergency stop (°C)

# ============================================================================
# FIRING PROFILES
# ============================================================================

PROFILES_DIR = "profiles"  # Directory for storing firing profiles

# ============================================================================
# ADAPTIVE RATE CONTROL
# ============================================================================

# Automatically adjust ramp rates if kiln cannot maintain desired rate
# Allows profiles to complete successfully even with insufficient kiln capacity

ADAPTATION_ENABLED = True
ADAPTATION_CHECK_INTERVAL = 60           # Check interval (seconds)
ADAPTATION_MIN_STEP_TIME = 600           # Wait before first adaptation (seconds)
ADAPTATION_MIN_TIME_BETWEEN = 300        # Wait between adaptations (seconds)
ADAPTATION_TEMP_ERROR_THRESHOLD = 20     # Trigger if behind by this temp (°C)
ADAPTATION_RATE_THRESHOLD = 0.85         # Trigger if actual < 85% of target
ADAPTATION_REDUCTION_FACTOR = 0.95       # Reduce to 95% of measured rate
RATE_MEASUREMENT_WINDOW = 600            # Rate measurement window (seconds)
RATE_RECORDING_INTERVAL = 10             # Temp recording interval (seconds)

# ============================================================================
# DATA LOGGING
# ============================================================================

LOGS_DIR = "logs"       # Directory for CSV log files
LOGGING_INTERVAL = 30   # Log data every N seconds

# ============================================================================
# CRASH RECOVERY
# ============================================================================

# Automatic program recovery after unexpected reboot/crash
# Resumes interrupted kiln program if conditions are safe

MAX_RECOVERY_DURATION = 300   # Max time since last log to attempt recovery (seconds)
MAX_RECOVERY_TEMP_DELTA = 30  # Max temp deviation from last log for safe recovery (°C)

# ============================================================================
# WATCHDOG TIMER
# ============================================================================

# Hardware watchdog for automatic recovery from control loop hangs
# Monitors Core 1 (control thread) and resets Pico if control loop hangs
#
# How it works:
#   1. Control loop completes → watchdog is fed
#   2. Control loop hangs → watchdog NOT fed
#   3. After WATCHDOG_TIMEOUT → Pico resets
#   4. Recovery system resumes program (if within MAX_RECOVERY_DURATION)
#
# Safety notes:
#   - Timeout (8000ms) > control loop interval (1000ms) → 8x safety margin
#   - Core 2 failures (web, WiFi) do NOT trigger watchdog
#   - Only critical control loop hangs trigger reset
#
# When to enable:
#   ✓ After thorough testing
#   ✓ For unattended operation
#   ✓ When recovery parameters are properly configured
#
# When to disable:
#   ✗ During development/debugging
#   ✗ When connected to REPL/laptop
#   ✗ If control loop legitimately takes >8s (increase timeout instead)

ENABLE_WATCHDOG = False
WATCHDOG_TIMEOUT = 8000  # milliseconds (8 seconds)

# ============================================================================
# LCD DISPLAY (OPTIONAL)
# ============================================================================

# 1602 LCD display with I2C backpack (PCF8574)
# If these settings are not defined, LCD display will be disabled
#
# To enable the LCD display, uncomment and configure the following:

# I2C Configuration for LCD
# LCD_I2C_ID = 0           # I2C bus ID (0 or 1)
# LCD_I2C_SCL = 21         # I2C SCL pin
# LCD_I2C_SDA = 20         # I2C SDA pin
# LCD_I2C_FREQ = 100000    # I2C frequency (100kHz standard)
# LCD_I2C_ADDR = 0x27      # I2C address (0x27 or 0x3F common for PCF8574)

# Button Configuration (OPTIONAL)
# Buttons connect to ground (active low with internal pull-up)
# If not defined, display will work but no button navigation
# LCD_BTN_NEXT_PIN = 14    # Button to cycle through screens
# LCD_BTN_SELECT_PIN = 15  # Button to select/confirm actions

# Available screens (auto-cycle with NEXT button):
#   1. WiFi Status - Shows connection status and IP address
#   2. State - Shows current state (RUNNING, IDLE, TUNING, etc.)
#   3. Temperature - Shows current and target temperature
#   4. Profile - Shows active profile or tuning method
#   5. Stop - Allows stopping active program (requires SELECT confirmation)
