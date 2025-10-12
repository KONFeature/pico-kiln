# main.py
# Entry point for pico-kiln controller

import asyncio
import network
import time
from machine import Pin
import config
import web_server

# Global state - shared between modules
class State:
    def __init__(self):
        # Hardware
        self.ssr_pin = None
        self.status_led = None

        # Temperature
        self.current_temp = 0.0
        self.target_temp = 0.0

        # Program
        self.current_program = None
        self.program_running = False

        # Network
        self.wifi = None
        self.ip_address = None

# Global state instance
state = State()

def connect_wifi():
    """Connect to WiFi network with AP selection"""
    print("Scanning for WiFi networks...")
    state.wifi = network.WLAN(network.STA_IF)
    state.wifi.active(True)

    # Scan and find best AP with matching SSID
    networks = state.wifi.scan()
    best_bssid = None
    best_rssi = -100

    for net in networks:
        ssid = net[0].decode()
        bssid = net[1]
        rssi = net[3]

        if ssid == config.WIFI_SSID and rssi > best_rssi:
            best_rssi = rssi
            best_bssid = bssid

    # Connect to best AP or default
    if best_bssid:
        print(f"Connecting to {config.WIFI_SSID} (RSSI: {best_rssi}, BSSID: {best_bssid.hex()})")
        state.wifi.connect(config.WIFI_SSID, config.WIFI_PASSWORD, bssid=best_bssid)
    else:
        print(f"Connecting to {config.WIFI_SSID} (no specific AP)")
        state.wifi.connect(config.WIFI_SSID, config.WIFI_PASSWORD)

    # Wait for connection with LED blink
    print("Waiting for WiFi connection...")
    while not state.wifi.isconnected():
        if state.status_led:
            state.status_led.on()
        time.sleep(0.5)
        if state.status_led:
            state.status_led.off()
        time.sleep(0.5)

    # Connected!
    status = state.wifi.ifconfig()
    state.ip_address = status[0]
    print(f"WiFi connected!")
    print(f"IP Address: {status[0]}")
    print(f"Netmask: {status[1]}")
    print(f"Gateway: {status[2]}")
    print(f"DNS: {status[3]}")

    if state.status_led:
        state.status_led.on()

    return status[0]

async def wifi_monitor():
    """Monitor WiFi connection and reconnect if disconnected"""
    while True:
        if not state.wifi.isconnected():
            print("WiFi disconnected, reconnecting...")
            if state.status_led:
                state.status_led.off()

            state.wifi.disconnect()
            state.wifi.active(True)
            state.wifi.connect(config.WIFI_SSID, config.WIFI_PASSWORD)

            # Wait for connection with blinking LED
            while not state.wifi.isconnected():
                if state.status_led:
                    state.status_led.on()
                await asyncio.sleep(0.5)
                if state.status_led:
                    state.status_led.off()
                await asyncio.sleep(0.5)

            status = state.wifi.ifconfig()
            state.ip_address = status[0]
            print("WiFi reconnected")
            print(f"IP Address: {status[0]}")

            if state.status_led:
                state.status_led.on()

        await asyncio.sleep(5)  # Check every 5 seconds

def setup_hardware():
    """Initialize hardware pins"""
    print("Initializing hardware...")

    # Setup status LED (onboard LED)
    try:
        state.status_led = Pin("LED", Pin.OUT)
        state.status_led.off()
        print("Status LED initialized")
    except:
        print("Status LED not available (not a Pico W?)")
        state.status_led = None

    # Setup SSR control pin
    state.ssr_pin = Pin(config.SSR_PIN, Pin.OUT)
    state.ssr_pin.value(0)  # Start with SSR off
    print(f"SSR pin initialized on GPIO {config.SSR_PIN}")

    # TODO: Initialize MAX31856 SPI interface
    print(f"MAX31856 SPI will use: SCK={config.MAX31856_SCK_PIN}, MOSI={config.MAX31856_MOSI_PIN}, MISO={config.MAX31856_MISO_PIN}, CS={config.MAX31856_CS_PIN}")

async def main_loop():
    """Main async loop - temperature control and monitoring"""
    print("Starting main loop...")

    while True:
        # TODO: Read temperature from MAX31856
        # TODO: Update PID controller
        # TODO: Control SSR based on PID output

        # Placeholder for now
        await asyncio.sleep(config.TEMP_READ_INTERVAL)

async def main():
    """Main entry point"""
    print("=" * 50)
    print("Pico Kiln Controller Starting...")
    print("=" * 50)

    # Setup hardware first (before WiFi so LED works)
    setup_hardware()

    # Connect to WiFi
    ip_address = connect_wifi()

    # Start all async tasks
    print("Starting web server...")
    server_task = asyncio.create_task(web_server.start_server(state))

    print("Starting WiFi monitor...")
    wifi_task = asyncio.create_task(wifi_monitor())

    print("Starting control loop...")
    control_task = asyncio.create_task(main_loop())

    print("=" * 50)
    print(f"System ready! Access web interface at: http://{ip_address}")
    print("=" * 50)

    # Run all tasks concurrently
    await asyncio.gather(server_task, wifi_task, control_task)

if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("\nShutting down...")
    except Exception as e:
        print(f"Error: {e}")
        raise
