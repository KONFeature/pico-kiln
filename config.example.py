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
