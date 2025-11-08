# main_safe.py
# Safe wrapper around main.py with comprehensive error catching and logging
#
# This version adds:
# - Immediate LED initialization for visual feedback
# - File-based error logging that survives crashes
# - Exception catching at every critical stage
# - LED blink patterns for different boot stages
#
# To use:
#   1. Backup current main.py: mv main.py main_original.py
#   2. Copy this to main.py: cp main_safe.py main.py
#   3. Copy main_original.py content into the "ORIGINAL MAIN CODE" section below

import time
from machine import Pin
import sys

# ============================================================================
# EARLY INITIALIZATION - Before any other imports
# ============================================================================

# Initialize LED FIRST - before anything can fail
LED = Pin("LED", Pin.OUT)
LED.off()

ERROR_LOG = '/boot_error.log'
STAGE_LOG = '/boot_stages.log'

def write_log(filepath, message, timestamp=True):
    """Write to log file with optional timestamp"""
    try:
        with open(filepath, 'a') as f:
            if timestamp:
                t = time.time()
                f.write(f"[{t:.3f}] {message}\n")
            else:
                f.write(f"{message}\n")
    except Exception as e:
        # Logging failed - try LED indication
        blink_error_pattern()

def clear_logs():
    """Clear log files at boot start"""
    try:
        with open(STAGE_LOG, 'w') as f:
            f.write("=== BOOT STAGE LOG ===\n")
            f.write(f"Time: {time.time()}\n\n")
    except:
        pass

    try:
        with open(ERROR_LOG, 'w') as f:
            f.write("=== BOOT ERROR LOG ===\n")
            f.write(f"Time: {time.time()}\n\n")
    except:
        pass

def blink_stage(stage_num):
    """Blink LED to indicate boot stage (1-9 blinks)"""
    for _ in range(stage_num):
        LED.on()
        time.sleep(0.15)
        LED.off()
        time.sleep(0.15)
    time.sleep(0.5)

def blink_error_pattern():
    """Fast continuous blinking indicates error"""
    for _ in range(20):
        LED.on()
        time.sleep(0.05)
        LED.off()
        time.sleep(0.05)

def log_exception(stage, exception):
    """Log exception with full details"""
    error_msg = f"STAGE {stage} FAILED: {exception}\n"
    error_msg += f"  Type: {type(exception).__name__}\n"
    error_msg += f"  Args: {exception.args}\n"

    # Try to get traceback
    try:
        import io
        buf = io.StringIO()
        sys.print_exception(exception, buf)
        error_msg += f"  Traceback:\n{buf.getvalue()}\n"
    except:
        error_msg += "  (Could not generate traceback)\n"

    write_log(ERROR_LOG, error_msg, timestamp=True)
    write_log(STAGE_LOG, f"STAGE {stage}: FAILED - {exception}", timestamp=True)
    print(error_msg)

# ============================================================================
# START BOOT SEQUENCE
# ============================================================================

clear_logs()
write_log(STAGE_LOG, "Boot sequence started", timestamp=True)

# Stage 1: LED test
write_log(STAGE_LOG, "STAGE 1: LED initialization", timestamp=True)
blink_stage(1)
write_log(STAGE_LOG, "STAGE 1: SUCCESS", timestamp=True)

# ============================================================================
# STAGE 2: Core imports
# ============================================================================
try:
    write_log(STAGE_LOG, "STAGE 2: Core imports (asyncio, _thread, config)", timestamp=True)
    blink_stage(2)

    import asyncio
    import _thread
    import config

    write_log(STAGE_LOG, "STAGE 2: SUCCESS", timestamp=True)

except Exception as e:
    log_exception(2, e)
    blink_error_pattern()
    raise

# ============================================================================
# STAGE 3: Server imports
# ============================================================================
try:
    write_log(STAGE_LOG, "STAGE 3: Server imports", timestamp=True)
    blink_stage(3)

    from server import web_server
    from server.wifi_manager import WiFiManager
    from server.status_receiver import get_status_receiver
    from server.data_logger import DataLogger

    write_log(STAGE_LOG, "STAGE 3: SUCCESS", timestamp=True)

except Exception as e:
    log_exception(3, e)
    blink_error_pattern()
    raise

# ============================================================================
# STAGE 4: Kiln imports
# ============================================================================
try:
    write_log(STAGE_LOG, "STAGE 4: Kiln imports", timestamp=True)
    blink_stage(4)

    from kiln.control_thread import start_control_thread

    write_log(STAGE_LOG, "STAGE 4: SUCCESS", timestamp=True)

except Exception as e:
    log_exception(4, e)
    blink_error_pattern()
    raise

# ============================================================================
# STAGE 5: Copy original main() function here
# ============================================================================

# Import the constants from original main
from micropython import const
ERROR_LOG_FLUSH_INTERVAL = const(10)
WIFI_CONNECT_TIMEOUT = const(15)

def format_timestamp(timestamp):
    """Format timestamp for error log file"""
    try:
        t = time.localtime(timestamp)
        return f"{t[0]}-{t[1]:02d}-{t[2]:02d} {t[3]:02d}:{t[4]:02d}:{t[5]:02d}"
    except:
        return f"{int(timestamp)}"

async def error_logger_loop(error_log):
    """
    Async loop that periodically flushes errors from queue to log file

    Runs on Core 2 to avoid blocking Core 1's control loop with I/O operations.
    """
    print("[Error Logger] Starting error logger loop")
    error_file = '/errors.log'
    flush_interval = ERROR_LOG_FLUSH_INTERVAL

    while True:
        try:
            errors, dropped_count = error_log.get_errors()

            if errors or dropped_count > 0:
                try:
                    with open(error_file, 'a') as f:
                        if dropped_count > 0:
                            timestamp_str = format_timestamp(time.time())
                            f.write(f"[{timestamp_str}] [ErrorLog] WARNING: {dropped_count} errors dropped due to full queue\n")

                        for error in errors:
                            timestamp_str = format_timestamp(error['timestamp'])
                            f.write(f"[{timestamp_str}] [{error['source']}] {error['message']}\n")

                    if errors:
                        print(f"[Error Logger] Flushed {len(errors)} errors to {error_file}")

                except Exception as e:
                    print(f"[Error Logger] Failed to write error log: {e}")

        except Exception as e:
            print(f"[Error Logger] Error in logger loop: {e}")

        await asyncio.sleep(flush_interval)

async def wifi_connect_background(wifi_mgr, timeout=15):
    """
    Connect to WiFi in background with smart timeout

    First boot uses longer timeout (15s) to handle cold WiFi hardware.
    Uses AP scan caching for faster reconnections.
    """
    print(f"[WiFi Background] Starting connection (timeout: {timeout}s)...")
    ip_address = await wifi_mgr.connect(timeout=timeout)

    if ip_address:
        print(f"[WiFi Background] Connected: {ip_address}")
        # Update LCD if available
        from server.lcd_manager import get_lcd_manager
        lcd_manager = get_lcd_manager()
        if lcd_manager and lcd_manager.enabled:
            lcd_manager.set_wifi_status(True, ip_address)
    else:
        print(f"[WiFi Background] Connection failed/timeout")
        print(f"[WiFi Background] Monitor will retry with cached AP")

    return ip_address

async def ntp_sync_background(wifi_mgr):
    """
    Sync NTP time in background after WiFi connects

    Waits for WiFi connection, then syncs time with retry logic.
    Recovery system will use file mtime until NTP syncs successfully.
    """
    # Wait for WiFi to connect first
    max_wait = 30  # Don't wait forever
    waited = 0
    while (not wifi_mgr.wlan or not wifi_mgr.wlan.isconnected()) and waited < max_wait:
        await asyncio.sleep(1)
        waited += 1

    if not wifi_mgr.wlan or not wifi_mgr.wlan.isconnected():
        print("[NTP Background] WiFi not connected, skipping NTP sync")
        return False

    print("[NTP Background] Starting time sync...")
    success = wifi_mgr.sync_time_ntp(max_attempts=3)

    if success:
        print("[NTP Background] Time synchronized")
    else:
        print("[NTP Background] Time sync failed (recovery will use file mtime)")

    return success

async def lcd_init_background(lcd_manager):
    """
    Initialize LCD hardware in background with retries

    LCD initialization can be slow or fail, so we do it in background
    to avoid blocking the critical boot path.
    """
    if not lcd_manager or not lcd_manager.enabled:
        return False

    max_attempts = 3
    for attempt in range(max_attempts):
        try:
            success = await lcd_manager.initialize_hardware(timeout_ms=500)
            if success:
                print(f"[LCD Background] Initialized successfully (attempt {attempt + 1})")
                return True
            else:
                print(f"[LCD Background] Init failed (attempt {attempt + 1}/{max_attempts})")
        except Exception as e:
            print(f"[LCD Background] Error on attempt {attempt + 1}: {e}")

        if attempt < max_attempts - 1:
            await asyncio.sleep(1)  # Wait before retry

    print("[LCD Background] Initialization failed after all attempts")
    return False

async def main():
    """
    Main entry point for multi-threaded kiln controller

    Optimized boot sequence:
    1. Core 1 starts FIRST (priority: temperature monitoring)
    2. WiFi connects in parallel during "quiet mode"
    3. Recovery check happens ASAP (~2-3s)
    4. Non-critical tasks (LCD, NTP) deferred to background
    """
    print("=" * 50)
    print("Pico Kiln Controller - Safe Boot Mode")
    print("=" * 50)

    # Mark that we've reached main()
    write_log(STAGE_LOG, "STAGE 5: Entering main()", timestamp=True)
    blink_stage(5)

    # ========================================================================
    # STAGE 1: Create communication infrastructure
    # ========================================================================
    print("[Main] Stage 1: Creating communication infrastructure...")
    from kiln.comms import ThreadSafeQueue, ErrorLog, ReadyFlag, QuietMode

    # Command queue: Core 2 -> Core 1
    command_queue = ThreadSafeQueue(maxsize=10)

    # Status queue: Core 1 -> Core 2
    status_queue = ThreadSafeQueue(maxsize=100)

    # Error log: Core 1 -> Core 2
    error_log = ErrorLog(max_queue_size=50)

    # Synchronization primitives
    ready_flag = ReadyFlag()
    quiet_mode = QuietMode()

    print("[Main] Infrastructure ready")
    write_log(STAGE_LOG, "STAGE 6: Infrastructure created", timestamp=True)
    blink_stage(6)

    # ========================================================================
    # STAGE 2: Start Core 1 IMMEDIATELY (quiet mode)
    # ========================================================================
    print("[Main] Stage 2: Starting Core 1 (quiet mode)...")
    quiet_mode.set_quiet(True)  # Suppress status updates during WiFi phase

    _thread.start_new_thread(
        start_control_thread,
        (command_queue, status_queue, config, error_log, ready_flag, quiet_mode)
    )
    print("[Main] Core 1 started (initializing hardware...)")
    write_log(STAGE_LOG, "STAGE 7: Core 1 started", timestamp=True)
    blink_stage(7)

    # ========================================================================
    # STAGE 3: Start status receiver and WiFi in parallel
    # ========================================================================
    print("[Main] Stage 3: Starting status receiver and WiFi...")

    # Status receiver starts immediately (ready for Core 1 updates)
    status_receiver = get_status_receiver()
    status_receiver.initialize(status_queue)
    receiver_task = asyncio.create_task(status_receiver.run())
    print("[Main] Status receiver running")

    # WiFi connects in background (15s timeout for cold hardware)
    wifi_mgr = WiFiManager(config)
    wifi_task = asyncio.create_task(wifi_connect_background(wifi_mgr, timeout=WIFI_CONNECT_TIMEOUT))
    print("[Main] WiFi connection started (background)")

    # ========================================================================
    # STAGE 4: Wait for Core 1 hardware initialization
    # ========================================================================
    print("[Main] Stage 4: Waiting for Core 1 ready signal...")
    core1_ready = await ready_flag.wait_ready(timeout=20.0)

    if core1_ready:
        print("[Main] Core 1 hardware ready")
        write_log(STAGE_LOG, "STAGE 8: Core 1 ready", timestamp=True)
        blink_stage(8)
    else:
        print("[Main] Core 1 not ready after 20s - CRITICAL ERROR")
        print("[Main] System unsafe to operate - check hardware connections")
        write_log(ERROR_LOG, "CRITICAL: Core 1 timeout", timestamp=True)
        raise Exception("Core 1 initialization timeout")

    # ========================================================================
    # STAGE 5: Wait for WiFi (or timeout) - end of quiet mode
    # ========================================================================
    print("[Main] Stage 5: Waiting for WiFi connection...")
    try:
        ip_address = await asyncio.wait_for(wifi_task, timeout=16)
    except asyncio.TimeoutError:
        ip_address = None
        print("[Main] WiFi timeout - continuing without network")

    # Exit quiet mode - Core 1 can now send status updates
    quiet_mode.set_quiet(False)
    print("[Main] Quiet mode ended - Core 1 active")

    # Small delay to let first status update flow
    await asyncio.sleep(0.2)

    # ========================================================================
    # STAGE 6: Register all listeners and check recovery
    # ========================================================================
    print("[Main] Stage 6: Registering listeners and checking recovery...")

    # Create LCD manager (reads directly from StatusCache, no listener needed)
    from server.lcd_manager import initialize_lcd_manager
    lcd_manager = initialize_lcd_manager(config, command_queue, status_receiver)

    # Register data logger
    data_logger = DataLogger(config.LOGS_DIR, config.LOGGING_INTERVAL)
    status_receiver.register_listener(data_logger.on_status_update)
    print("[Main] Data logger registered")

    # Register recovery listener
    from server.recovery import RecoveryListener
    recovery_listener = RecoveryListener(command_queue, data_logger, config, wifi_mgr)
    recovery_listener.set_status_receiver(status_receiver)
    status_receiver.register_listener(recovery_listener.on_status_update)
    print("[Main] Recovery listener registered (will check on first temp)")

    # ========================================================================
    # STAGE 7: Start background tasks
    # ========================================================================
    print("[Main] Stage 7: Starting background tasks...")

    # Start NTP sync in background
    ntp_task = asyncio.create_task(ntp_sync_background(wifi_mgr))
    print("[Main] NTP sync started (background)")

    # Start LCD hardware init in background
    lcd_init_task = None
    if lcd_manager and lcd_manager.enabled:
        lcd_init_task = asyncio.create_task(lcd_init_background(lcd_manager))
        print("[Main] LCD init started (background)")

    # ========================================================================
    # STAGE 8: Preload caches
    # ========================================================================
    print("[Main] Stage 8: Preloading caches...")

    # HTML cache
    from server.html_cache import get_html_cache
    html_cache = get_html_cache()
    html_cache.preload({
        'index': 'static/index.html',
        'tuning': 'static/tuning.html'
    })
    print("[Main] HTML cache preloaded")

    # Profile cache
    from server.profile_cache import get_profile_cache
    profile_cache = get_profile_cache()
    profile_cache.preload(config.PROFILES_DIR)
    print("[Main] Profile cache preloaded")

    # ========================================================================
    # STAGE 9: Start async services
    # ========================================================================
    print("[Main] Stage 9: Starting async services...")

    # Web server
    server_task = asyncio.create_task(web_server.start_server(command_queue))
    print("[Main] Web server started")

    # WiFi monitor (auto-reconnect)
    wifi_monitor_task = asyncio.create_task(wifi_mgr.monitor())
    print("[Main] WiFi monitor started")

    # Error logger
    error_logger_task = asyncio.create_task(error_logger_loop(error_log))
    print("[Main] Error logger started")

    # LCD manager
    lcd_task = None
    if lcd_manager and lcd_manager.enabled:
        lcd_task = asyncio.create_task(lcd_manager.run())
        print("[Main] LCD manager started")

    # Update LCD with WiFi status
    if lcd_manager and lcd_manager.enabled and ip_address:
        lcd_manager.set_wifi_status(True, ip_address)

    # ========================================================================
    # BOOT COMPLETE
    # ========================================================================
    print("=" * 50)
    print("System Ready!")
    print("Core 1: Control thread (temp, PID, SSR)")
    lcd_status = " + LCD" if (lcd_manager and lcd_manager.enabled) else ""
    print(f"Core 2: Web + WiFi + Status + Data + Errors{lcd_status}")
    if ip_address:
        print(f"Web interface: http://{ip_address}")
    else:
        print("Web interface: Unavailable (no WiFi)")
    print("=" * 50)

    write_log(STAGE_LOG, "STAGE 9: Boot complete - system ready!", timestamp=True)
    blink_stage(9)

    # Solid LED on = system running
    LED.on()

    # ========================================================================
    # Run all async tasks
    # ========================================================================
    tasks = [receiver_task, server_task, wifi_monitor_task, error_logger_task]
    if lcd_task:
        tasks.append(lcd_task)

    await asyncio.gather(*tasks)

# ============================================================================
# ENTRY POINT with full exception handling
# ============================================================================
if __name__ == "__main__":
    try:
        write_log(STAGE_LOG, "Starting asyncio.run(main())", timestamp=True)
        asyncio.run(main())

    except KeyboardInterrupt:
        write_log(STAGE_LOG, "Keyboard interrupt", timestamp=True)
        print("\n[Main] Keyboard interrupt received")
        print("[Main] Shutting down gracefully...")
        print("[Main] Control thread will turn off SSR automatically")
        print("[Main] Shutdown complete")

    except Exception as e:
        write_log(ERROR_LOG, "FATAL ERROR in main:", timestamp=True)
        log_exception("MAIN", e)
        print(f"[Main] Fatal error: {e}")
        print("[Main] Emergency shutdown - control thread should have turned off SSR")
        print(f"[Main] Check logs: {ERROR_LOG} and {STAGE_LOG}")

        # Fast blink forever to indicate fatal error
        blink_error_pattern()
        while True:
            LED.on()
            time.sleep(0.1)
            LED.off()
            time.sleep(0.1)
