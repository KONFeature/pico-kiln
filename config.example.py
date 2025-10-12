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
