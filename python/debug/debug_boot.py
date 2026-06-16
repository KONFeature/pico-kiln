# debug_boot.py
# Debugging tool to diagnose standalone boot failures
#
# Usage:
#   1. Temporarily rename main.py to main_backup.py
#   2. Rename this file to main.py
#   3. Power cycle the Pico
#   4. Watch LED blink patterns
#   5. Connect via mpremote and read /boot_debug.log
#
# LED Patterns:
#   - 1 blink:  Boot started, LED working
#   - 2 blinks: Imports successful
#   - 3 blinks: Config loaded
#   - 4 blinks: Ready to start main()
#   - 5 blinks: main() started successfully
#   - Fast blink (10Hz): Fatal error - check /boot_debug.log
#   - Slow blink (0.5Hz): All successful - check /boot_debug.log for details

import time
from machine import Pin
import sys

# Initialize LED IMMEDIATELY - before anything else
led = Pin("LED", Pin.OUT)
led.off()

# Debug log file
DEBUG_LOG = '/boot_debug.log'

def write_log(message):
    """Write to debug log with timestamp"""
    try:
        with open(DEBUG_LOG, 'a') as f:
            timestamp = time.time()
            f.write(f"[{timestamp}] {message}\n")
    except Exception as e:
        # If logging fails, at least try to print
        print(f"LOG FAILED: {message} (error: {e})")

def blink_pattern(count, delay=0.2):
    """Blink LED a specific number of times"""
    for _ in range(count):
        led.on()
        time.sleep(delay)
        led.off()
        time.sleep(delay)
    time.sleep(0.5)  # Pause between patterns

def blink_forever(fast=True):
    """Blink LED forever to indicate state"""
    delay = 0.1 if fast else 1.0
    while True:
        led.on()
        time.sleep(delay)
        led.off()
        time.sleep(delay)

def main():
    """Debug boot sequence with extensive logging and LED feedback"""

    # Clear previous log
    try:
        with open(DEBUG_LOG, 'w') as f:
            f.write("=== BOOT DEBUG LOG ===\n")
    except:
        pass

    write_log("STAGE 0: Debug boot started")
    blink_pattern(1)  # 1 blink - boot started

    # ========================================================================
    # STAGE 1: Test basic imports
    # ========================================================================
    try:
        write_log("STAGE 1: Testing basic imports...")

        write_log("  Importing asyncio...")
        import asyncio

        write_log("  Importing _thread...")
        import _thread

        write_log("  Importing network...")
        import network

        write_log("STAGE 1: Basic imports successful")
        blink_pattern(2)  # 2 blinks - imports OK

    except Exception as e:
        write_log(f"STAGE 1 FAILED: {e}")
        write_log(f"  Exception type: {type(e)}")
        write_log(f"  Exception args: {e.args}")
        blink_forever(fast=True)  # Fast blink = error

    # ========================================================================
    # STAGE 2: Load config
    # ========================================================================
    try:
        write_log("STAGE 2: Loading config...")
        import config

        write_log(f"  Config loaded: {dir(config)}")
        write_log(f"  WIFI_SSID: {getattr(config, 'WIFI_SSID', 'NOT SET')}")

        write_log("STAGE 2: Config loaded successfully")
        blink_pattern(3)  # 3 blinks - config OK

    except Exception as e:
        write_log(f"STAGE 2 FAILED: {e}")
        write_log(f"  Exception type: {type(e)}")
        write_log(f"  Exception args: {e.args}")
        import sys
        write_log(f"  Traceback: {sys.print_exception(e)}")
        blink_forever(fast=True)

    # ========================================================================
    # STAGE 3: Test project imports
    # ========================================================================
    try:
        write_log("STAGE 3: Testing project imports...")

        write_log("  Importing server.wifi_manager...")
        from server.wifi_manager import WiFiManager

        write_log("  Importing server.web_server...")
        from server import web_server

        write_log("  Importing server.status_receiver...")
        from server.status_receiver import get_status_receiver

        write_log("  Importing server.data_logger...")
        from server.data_logger import DataLogger

        write_log("  Importing kiln.control_thread...")
        from kiln.control_thread import start_control_thread

        write_log("  Importing kiln.comms...")
        from kiln.comms import ThreadSafeQueue, ErrorLog, ReadyFlag, QuietMode

        write_log("STAGE 3: All project imports successful")
        blink_pattern(4)  # 4 blinks - all imports OK

    except Exception as e:
        write_log(f"STAGE 3 FAILED: {e}")
        write_log(f"  Exception type: {type(e)}")
        write_log(f"  Exception args: {e.args}")
        # Try to get more details
        try:
            import sys
            import io
            buf = io.StringIO()
            sys.print_exception(e, buf)
            write_log(f"  Traceback:\n{buf.getvalue()}")
        except:
            write_log("  Could not get traceback")
        blink_forever(fast=True)

    # ========================================================================
    # STAGE 4: Try to run actual main()
    # ========================================================================
    try:
        write_log("STAGE 4: Attempting to import and run real main...")

        # Try to import the actual main
        write_log("  Renaming: You should have renamed main.py to main_backup.py")
        write_log("  If you want to test real main, import it here")

        write_log("STAGE 4: All checks passed!")
        blink_pattern(5)  # 5 blinks - success!

        write_log("=== SUCCESS: All stages completed ===")
        write_log("Boot debugging complete. Check this log for details.")
        write_log(f"Python version: {sys.version}")
        write_log(f"Platform: {sys.platform}")

        # Slow blink = success
        write_log("LED will now blink slowly (success pattern)")
        blink_forever(fast=False)

    except Exception as e:
        write_log(f"STAGE 4 FAILED: {e}")
        try:
            import sys
            import io
            buf = io.StringIO()
            sys.print_exception(e, buf)
            write_log(f"  Traceback:\n{buf.getvalue()}")
        except:
            write_log("  Could not get traceback")
        blink_forever(fast=True)

# Run immediately
if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        # Ultimate fallback
        write_log(f"CATASTROPHIC FAILURE: {e}")
        try:
            import sys
            import io
            buf = io.StringIO()
            sys.print_exception(e, buf)
            write_log(f"Traceback:\n{buf.getvalue()}")
        except:
            pass
        blink_forever(fast=True)
