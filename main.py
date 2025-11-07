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
from server import web_server
from server.wifi_manager import WiFiManager
from server.status_receiver import get_status_receiver
from server.data_logger import DataLogger

# Import control thread
from kiln.control_thread import start_control_thread

def format_timestamp(timestamp):
    """Format timestamp for error log file"""
    try:
        # MicroPython's localtime returns (year, month, day, hour, min, sec, weekday, yearday)
        t = time.localtime(timestamp)
        return f"{t[0]}-{t[1]:02d}-{t[2]:02d} {t[3]:02d}:{t[4]:02d}:{t[5]:02d}"
    except:
        # Fallback if localtime not available
        return f"{int(timestamp)}"

async def error_logger_loop(error_log):
    """
    Async loop that periodically flushes errors from queue to log file

    Runs on Core 2 to avoid blocking Core 1's control loop with I/O operations.

    Args:
        error_log: ErrorLog instance shared with Core 1
    """
    print("[Error Logger] Starting error logger loop")
    error_file = '/errors.log'
    flush_interval = 10  # Flush every 10 seconds

    while True:
        try:
            # Get all pending errors from queue
            errors, dropped_count = error_log.get_errors()

            if errors or dropped_count > 0:
                # Write to file
                try:
                    with open(error_file, 'a') as f:
                        # Report dropped errors if any
                        if dropped_count > 0:
                            timestamp_str = format_timestamp(time.time())
                            f.write(f"[{timestamp_str}] [ErrorLog] WARNING: {dropped_count} errors dropped due to full queue\n")

                        # Write all errors
                        for error in errors:
                            timestamp_str = format_timestamp(error['timestamp'])
                            f.write(f"[{timestamp_str}] [{error['source']}] {error['message']}\n")

                    if errors:
                        print(f"[Error Logger] Flushed {len(errors)} errors to {error_file}")

                except Exception as e:
                    print(f"[Error Logger] Failed to write error log: {e}")
                    # Don't crash the logger - just skip this flush and try again later

        except Exception as e:
            print(f"[Error Logger] Error in logger loop: {e}")
            # Don't crash - keep running

        # Wait before next flush
        await asyncio.sleep(flush_interval)

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
    from kiln.comms import ThreadSafeQueue, ErrorLog

    # Command queue: Core 2 -> Core 1
    # Small queue since commands are infrequent
    command_queue = ThreadSafeQueue(maxsize=10)

    # Status queue: Core 1 -> Core 2
    # Larger queue to buffer status updates
    status_queue = ThreadSafeQueue(maxsize=100)

    # Error log: Core 1 -> Core 2
    # Queue for cross-core error logging
    error_log = ErrorLog(max_queue_size=50)

    print("[Main] Communication queues created")

    # Start control thread on Core 1
    print("[Main] Starting control thread on Core 1...")
    _thread.start_new_thread(start_control_thread, (command_queue, status_queue, config, error_log))
    print("[Main] Control thread started")

    # Give control thread time to initialize hardware
    await asyncio.sleep(2)

    # Initialize status receiver (singleton) for Core 2
    print("[Main] Initializing status receiver...")
    status_receiver = get_status_receiver()
    status_receiver.initialize(status_queue)
    
    # Initialize LCD manager (optional hardware) - defer hardware init until after WiFi
    print("[Main] Initializing LCD manager...")
    from server.lcd_manager import initialize_lcd_manager
    lcd_manager = initialize_lcd_manager(config, command_queue)

    # Register LCD as status listener early (will queue updates until LCD ready)
    if lcd_manager and lcd_manager.enabled:
        status_receiver.register_listener(lcd_manager.update_status)

    # Initialize and register data logger
    print("[Main] Initializing data logger...")
    data_logger = DataLogger(config.LOGS_DIR, config.LOGGING_INTERVAL)
    status_receiver.register_listener(data_logger.on_status_update)

    # Initialize WiFi manager (early, so recovery can use it for NTP callbacks)
    print("[Main] Initializing WiFi manager...")
    wifi_mgr = WiFiManager(config.WIFI_SSID, config.WIFI_PASSWORD)

    # Initialize and register recovery listener (with wifi_mgr for NTP retry)
    print("[Main] Initializing recovery listener...")
    from server.recovery import RecoveryListener
    recovery_listener = RecoveryListener(command_queue, data_logger, config, wifi_mgr)
    recovery_listener.set_status_receiver(status_receiver)
    status_receiver.register_listener(recovery_listener.on_status_update)
    print("[Main] Recovery listener will check on first valid temperature reading")

    # Start status receiver
    print("[Main] Starting status receiver...")
    receiver_task = asyncio.create_task(status_receiver.run())

    # Connect to WiFi FIRST (minimal interference from other operations)
    # Note: WiFi manager was initialized earlier to allow recovery NTP callbacks
    print("[Main] Connecting to WiFi...")
    ip_address = await wifi_mgr.connect(timeout=30)

    if not ip_address:
        print("[Main] WARNING: WiFi connection failed!")
        print("[Main] System will continue without WiFi")
        print("[Main] - Control thread is still running")
        print("[Main] - WiFi monitor will keep trying to connect")
        ip_address = "N/A"

    # Initialize LCD hardware AFTER WiFi to avoid timing interference
    if lcd_manager and lcd_manager.enabled:
        print("[Main] Initializing LCD hardware...")
        await lcd_manager.initialize_hardware(timeout_ms=500)

    # Update LCD with WiFi status
    if lcd_manager and lcd_manager.enabled:
        if ip_address != "N/A":
            lcd_manager.set_wifi_status(True, ip_address)
        else:
            lcd_manager.set_wifi_status(False, None)

    # Pre-load HTML files into cache (after WiFi to avoid interference)
    print("[Main] Pre-loading HTML files into cache...")
    from server.html_cache import get_html_cache
    html_cache = get_html_cache()
    html_cache.preload({
        'index': 'static/index.html',
        'tuning': 'static/tuning.html'
    })

    # Pre-load profiles into cache (after WiFi to avoid interference)
    print("[Main] Pre-loading profiles into cache...")
    from server.profile_cache import get_profile_cache
    profile_cache = get_profile_cache()
    profile_cache.preload(config.PROFILES_DIR)

    # Start async tasks on Core 2
    print("[Main] Starting web server...")
    server_task = asyncio.create_task(web_server.start_server(command_queue))

    print("[Main] Starting WiFi monitor...")
    wifi_task = asyncio.create_task(wifi_mgr.monitor())

    print("[Main] Starting error logger...")
    error_logger_task = asyncio.create_task(error_logger_loop(error_log))
    
    # Start LCD manager if enabled
    lcd_task = None
    if lcd_manager and lcd_manager.enabled:
        print("[Main] Starting LCD manager...")
        lcd_task = asyncio.create_task(lcd_manager.run())

    print("=" * 50)
    print(f"System ready!")
    print(f"Core 1: Control thread (temp, PID, SSR)")
    lcd_status = " + LCD display" if (lcd_manager and lcd_manager.enabled) else ""
    print(f"Core 2: Web server + WiFi + Status receiver + Data logger + Error logger{lcd_status}")
    if ip_address != "N/A":
        print(f"Access web interface at: http://{ip_address}")
    else:
        print(f"Web interface unavailable (no WiFi)")
        print(f"REPL/USB should be responsive")
    print("=" * 50)

    # Run all async tasks on Core 2
    tasks = [receiver_task, server_task, wifi_task, error_logger_task]
    if lcd_task:
        tasks.append(lcd_task)
    await asyncio.gather(*tasks)

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
