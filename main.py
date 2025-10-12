# main.py
# Entry point for pico-kiln controller

import asyncio
import network
import time
from machine import Pin
import config
import web_server

# Global state - shared between modules
# This is a lightweight container for hardware and component references
class State:
    def __init__(self):
        # Hardware pins
        self.ssr_pin = None
        self.status_led = None

        # Network
        self.wifi = None
        self.ip_address = None

        # Kiln components (initialized in setup)
        self.temp_sensor = None
        self.ssr_controller = None
        self.pid = None
        self.controller = None  # This holds the actual state (temps, program, etc.)

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
    """Initialize hardware pins and kiln components"""
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

    # Initialize MAX31856 SPI interface
    print(f"Initializing MAX31856 on SPI{config.MAX31856_SPI_ID}")
    from machine import SPI
    from wrapper import DigitalInOut, SPIWrapper
    from kiln import TemperatureSensor, SSRController, PID, KilnController

    # Setup SPI for MAX31856
    spi = SPIWrapper(
        SPI(
            config.MAX31856_SPI_ID,
            baudrate=1000000,
            sck=Pin(config.MAX31856_SCK_PIN),
            mosi=Pin(config.MAX31856_MOSI_PIN),
            miso=Pin(config.MAX31856_MISO_PIN),
        )
    )

    cs_pin = DigitalInOut(Pin(config.MAX31856_CS_PIN, Pin.OUT))

    # Initialize temperature sensor
    state.temp_sensor = TemperatureSensor(
        spi, cs_pin, offset=config.THERMOCOUPLE_OFFSET
    )

    # Initialize SSR controller
    state.ssr_controller = SSRController(
        state.ssr_pin, cycle_time=config.SSR_CYCLE_TIME
    )

    # Initialize PID controller
    state.pid = PID(
        kp=config.PID_KP,
        ki=config.PID_KI,
        kd=config.PID_KD,
        output_limits=(0, 100)
    )

    # Initialize kiln controller
    state.controller = KilnController(
        max_temp=config.MAX_TEMP,
        max_temp_error=config.MAX_TEMP_ERROR
    )

    print("All hardware initialized successfully")

async def main_loop():
    """
    Main control loop - runs every control interval

    This implements the single-core async control strategy:
    1. Read temperature from MAX31856
    2. Update kiln controller state machine
    3. Calculate PID output if running
    4. Set SSR duty cycle
    5. Update SSR state multiple times per interval for time-proportional control
    """
    print("Starting main loop...")

    # Import kiln state for convenience
    from kiln.state import KilnState

    while True:
        try:
            # 1. Read temperature
            current_temp = state.temp_sensor.read()

            # 2. Update controller state and get target temperature
            target_temp = state.controller.update(current_temp)

            # 3. Calculate PID output
            if state.controller.state == KilnState.RUNNING:
                # PID control active
                ssr_output = state.pid.update(target_temp, current_temp)
            else:
                # Not running - turn off SSR
                ssr_output = 0
                state.pid.reset()

            state.controller.ssr_output = ssr_output
            state.ssr_controller.set_output(ssr_output)

            # 4. Safety check: force SSR off in error state
            if state.controller.state == KilnState.ERROR:
                state.ssr_controller.force_off()
                print(f"ERROR STATE: {state.controller.error_message}")

            # Log status
            if state.controller.state != KilnState.IDLE:
                elapsed = state.controller.get_elapsed_time()
                print(f"[{elapsed:.0f}s] State:{state.controller.state} Temp:{current_temp:.1f}°C Target:{target_temp:.1f}°C SSR:{ssr_output:.1f}%")

            # 5. Update SSR state multiple times during control interval
            # This provides better time-proportional control resolution
            update_count = int(config.TEMP_READ_INTERVAL / 0.1)  # 10 Hz updates
            for _ in range(update_count):
                state.ssr_controller.update()
                await asyncio.sleep(0.1)

        except Exception as e:
            print(f"Control loop error: {e}")
            # Emergency shutdown on error
            if state.ssr_controller:
                state.ssr_controller.force_off()
            if state.controller:
                state.controller.set_error(str(e))
            await asyncio.sleep(1)

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
