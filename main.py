# main.py
# Entry point for pico-kiln controller
#
# Multi-threaded architecture:
# - Core 1: Control thread (temperature reading, PID, SSR control)
# - Core 2: Web server + WiFi management (this main thread)
#
# Communication via ThreadSafeQueue (bidirectional)

import asyncio
import time
import _thread
import config
import web_server
from wifi_manager import WiFiManager

# Import control thread
from kiln.control_thread import start_control_thread

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

    # Initialize WiFi manager
    wifi_mgr = WiFiManager(config.WIFI_SSID, config.WIFI_PASSWORD)

    # Connect to WiFi (Core 2)
    ip_address = await wifi_mgr.connect(timeout=30)

    if not ip_address:
        print("[Main] WARNING: WiFi connection failed!")
        print("[Main] System will continue without WiFi")
        print("[Main] - Control thread is still running")
        print("[Main] - WiFi monitor will keep trying to connect")
        ip_address = "N/A"

    # Start async tasks on Core 2
    print("[Main] Starting web server...")
    server_task = asyncio.create_task(web_server.start_server(command_queue, status_queue))

    print("[Main] Starting WiFi monitor...")
    wifi_task = asyncio.create_task(wifi_mgr.monitor())

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
