# main.py
# Entry point for pico-kiln controller
#
# Multi-threaded architecture:
# - Core 1: Control thread (temperature reading, PID, SSR control)
# - Core 2: Web server + WiFi management (this main thread)
#
# Communication via ThreadSafeQueue (bidirectional)

import asyncio
import network
import time
import _thread
from machine import Pin
import config
import web_server

# Import control thread
from kiln.control_thread import start_control_thread

# Global WiFi state
wifi = None
status_led = None

async def wait_for_wifi_connection(timeout=30):
    """
    Wait for WiFi connection with LED blinking and timeout

    Args:
        timeout: Maximum time to wait (seconds)

    Returns:
        True if connected, False if timeout
    """
    global wifi, status_led

    start_time = time.time()
    led_state = False

    while not wifi.isconnected():
        # Check timeout
        if time.time() - start_time > timeout:
            if status_led:
                status_led.off()
            return False

        # Blink LED (non-blocking)
        if status_led:
            led_state = not led_state
            status_led.value(1 if led_state else 0)

        # Yield to event loop
        await asyncio.sleep(0.5)

    # Connected successfully
    if status_led:
        status_led.on()
    return True

async def connect_wifi_async(timeout=30):
    """
    Connect to WiFi network with AP selection (async, non-blocking)

    Args:
        timeout: Maximum time to wait for connection (seconds)

    Returns:
        IP address if connected, None if timeout/failure
    """
    global wifi, status_led

    print("[Main] Scanning for WiFi networks...")
    wifi = network.WLAN(network.STA_IF)
    wifi.active(True)

    # Scan and find best AP with matching SSID
    try:
        networks = wifi.scan()
    except Exception as e:
        print(f"[Main] WiFi scan failed: {e}")
        return None

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
        print(f"[Main] Connecting to {config.WIFI_SSID} (RSSI: {best_rssi}, BSSID: {best_bssid.hex()})")
        wifi.connect(config.WIFI_SSID, config.WIFI_PASSWORD, bssid=best_bssid)
    else:
        print(f"[Main] Connecting to {config.WIFI_SSID} (no specific AP)")
        wifi.connect(config.WIFI_SSID, config.WIFI_PASSWORD)

    # Wait for connection (non-blocking with timeout)
    print(f"[Main] Waiting for WiFi connection (timeout: {timeout}s)...")
    connected = await wait_for_wifi_connection(timeout)

    if not connected:
        print(f"[Main] WiFi connection timeout after {timeout}s")
        return None

    # Connected!
    status = wifi.ifconfig()
    ip_address = status[0]
    print(f"[Main] WiFi connected!")
    print(f"[Main] IP Address: {status[0]}")
    print(f"[Main] Netmask: {status[1]}")
    print(f"[Main] Gateway: {status[2]}")
    print(f"[Main] DNS: {status[3]}")

    return ip_address

async def wifi_monitor():
    """Monitor WiFi connection and reconnect if disconnected"""
    global wifi, status_led

    while True:
        if wifi and not wifi.isconnected():
            print("[Main] WiFi disconnected, reconnecting...")
            if status_led:
                status_led.off()

            # Attempt reconnection
            wifi.disconnect()
            wifi.active(True)
            wifi.connect(config.WIFI_SSID, config.WIFI_PASSWORD)

            # Wait for connection (with timeout)
            timeout = 30  # 30 second timeout for reconnection
            connected = await wait_for_wifi_connection(timeout)

            if connected:
                # Successfully reconnected
                status = wifi.ifconfig()
                print("[Main] WiFi reconnected")
                print(f"[Main] IP Address: {status[0]}")
            else:
                # Reconnection failed
                print(f"[Main] WiFi reconnection timeout after {timeout}s")
                print("[Main] Will retry in 60 seconds...")
                await asyncio.sleep(60)

        await asyncio.sleep(5)  # Check every 5 seconds

def setup_status_led():
    """Initialize status LED (onboard LED on Core 2)"""
    global status_led

    print("[Main] Initializing status LED...")

    # Setup status LED (onboard LED)
    try:
        status_led = Pin("LED", Pin.OUT)
        status_led.off()
        print("[Main] Status LED initialized")
    except:
        print("[Main] Status LED not available (not a Pico W?)")
        status_led = None

async def main():
    """
    Main entry point for multi-threaded kiln controller

    Architecture:
    - Core 1: Control thread (dedicated hardware control)
    - Core 2: This thread (WiFi, web server, network operations)
    """
    print("=" * 50)
    print("Pico Kiln Controller Starting (Multi-threaded)")
    print("=" * 50)

    # Setup status LED (Core 2 only)
    setup_status_led()

    # Create communication queues for inter-thread communication
    print("[Main] Creating communication queues...")
    from kiln.comms import ThreadSafeQueue

    # Command queue: Core 2 -> Core 1
    # Small queue since commands are infrequent
    command_queue = ThreadSafeQueue(maxsize=10)

    # Status queue: Core 1 -> Core 2
    # Larger queue to buffer status updates
    status_queue = ThreadSafeQueue(maxsize=100)

    print("[Main] Communication queues created")

    # Start control thread on Core 1
    print("[Main] Starting control thread on Core 1...")
    _thread.start_new_thread(start_control_thread, (command_queue, status_queue, config))
    print("[Main] Control thread started")

    # Give control thread time to initialize hardware
    await asyncio.sleep(2)

    # Connect to WiFi (Core 2) - non-blocking with timeout
    print("[Main] Attempting WiFi connection...")
    ip_address = await connect_wifi_async(timeout=30)

    if ip_address:
        print(f"[Main] WiFi connected successfully: {ip_address}")
    else:
        print("[Main] WARNING: WiFi connection failed!")
        print("[Main] System will continue without WiFi")
        print("[Main] - Control thread is still running")
        print("[Main] - WiFi monitor will keep trying to connect")
        ip_address = "N/A"

    # Start async tasks on Core 2
    print("[Main] Starting web server...")
    server_task = asyncio.create_task(web_server.start_server(command_queue, status_queue))

    print("[Main] Starting WiFi monitor...")
    wifi_task = asyncio.create_task(wifi_monitor())

    print("=" * 50)
    print(f"System ready!")
    print(f"Core 1: Control thread (temp, PID, SSR)")
    print(f"Core 2: Web server + WiFi")
    if ip_address != "N/A":
        print(f"Access web interface at: http://{ip_address}")
    else:
        print(f"Web interface unavailable (no WiFi)")
        print(f"REPL/USB should be responsive")
    print("=" * 50)

    # Run all async tasks on Core 2
    await asyncio.gather(server_task, wifi_task)

if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("\n[Main] Keyboard interrupt received")
        print("[Main] Shutting down gracefully...")

        # Note: Control thread will handle SSR shutdown automatically
        # The exception will propagate and both threads will terminate
        print("[Main] Control thread should turn off SSR automatically")
        print("[Main] Shutdown complete")

    except Exception as e:
        print(f"[Main] Fatal error: {e}")
        print("[Main] Emergency shutdown - control thread should have turned off SSR")
        raise
