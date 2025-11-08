# main.py
# Entry point for pico-kiln controller
#
# Multi-threaded architecture with optimized boot sequence:
# - Core 1: Control thread (temperature reading, PID, SSR control)
# - Core 2: Web server + WiFi management (this main thread)
#
# Boot optimization: Core 1 starts first for immediate temperature monitoring,
# WiFi connects in parallel with reduced interference via "quiet mode"

import asyncio
import time
import _thread
import gc
import config
from server import web_server
from server.wifi_manager import WiFiManager
from server.status_receiver import get_status_receiver
from server.data_logger import DataLogger
from micropython import const

# Import control thread
from kiln.control_thread import start_control_thread

# Performance: const() declarations for boot sequence timing
LOG_FLUSH_INTERVAL = const(10)  # Seconds between log flushes
WIFI_CONNECT_TIMEOUT = const(15)  # WiFi connection timeout in seconds


def print_memory_info(label=""):
    """Print current memory status for debugging"""
    gc.collect()  # Collect garbage first for accurate reading
    free = gc.mem_free()
    alloc = gc.mem_alloc()
    total = free + alloc
    used_pct = (alloc / total * 100) if total > 0 else 0

    prefix = f"[RAM {label}]" if label else "[RAM]"
    print(f"{prefix} Free: {free:,} bytes ({free/1024:.1f} KB) | Allocated: {alloc:,} bytes ({alloc/1024:.1f} KB) | Used: {used_pct:.1f}%")


def format_timestamp(timestamp):
    """Format timestamp for error log file"""
    try:
        t = time.localtime(timestamp)
        return f"{t[0]}-{t[1]:02d}-{t[2]:02d} {t[3]:02d}:{t[4]:02d}:{t[5]:02d}"
    except:
        return f"{int(timestamp)}"


def log_error_to_file(source, message):
    """
    Log error directly to errors.log file

    Used for critical errors that occur outside the error_log queue system
    (e.g., main thread initialization failures, fatal exceptions)
    """
    try:
        timestamp_str = format_timestamp(time.time())
        with open('/errors.log', 'a') as f:
            f.write(f"[{timestamp_str}] [{source}] {message}\n")
    except Exception as e:
        print(f"[Error Logger] Failed to write error to file: {e}")


async def error_logger_loop(error_log):
    """
    Async loop that periodically flushes errors from queue to log file

    Runs on Core 2 to avoid blocking Core 1's control loop with I/O operations.
    Includes log rotation to prevent disk space exhaustion.
    Also logs periodic memory stats for runtime debugging.
    """
    print("[Error Logger] Starting error logger loop")
    error_file = '/errors.log'
    flush_interval = LOG_FLUSH_INTERVAL
    memory_log_counter = 0
    memory_log_frequency = 6  # Log memory every 6 flushes (every 60s if flush_interval=10s)

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

            # Periodic memory logging
            memory_log_counter += 1
            if memory_log_counter >= memory_log_frequency:
                print_memory_info("Runtime")
                memory_log_counter = 0

        except Exception as e:
            print(f"[Error Logger] Error in logger loop: {e}")

        await asyncio.sleep(flush_interval)


async def wifi_connect_background(wifi_mgr, timeout=15):
    """
    Connect to WiFi in background with smart timeout

    First boot uses longer timeout (15s) to handle cold WiFi hardware.
    Uses AP scan caching for faster reconnections.
    """
    try:
        print(f"[WiFi Background] Starting connection (timeout: {timeout}s)...")
        ip_address = await wifi_mgr.connect(timeout=timeout, use_cache=False)

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
    except Exception as e:
        error_msg = f"WiFi background task error: {e}"
        print(f"[WiFi Background] {error_msg}")
        log_error_to_file("WiFi", error_msg)
        return None


async def ntp_sync_background(wifi_mgr):
    """
    Sync NTP time in background after WiFi connects

    Waits for WiFi connection, then syncs time with retry logic.
    Recovery system will use file mtime until NTP syncs successfully.
    """
    try:
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
    except Exception as e:
        error_msg = f"NTP sync error: {e}"
        print(f"[NTP Background] {error_msg}")
        log_error_to_file("NTP", error_msg)
        return False


async def lcd_init_background(lcd_manager):
    """
    Initialize LCD hardware in background with retries

    LCD initialization can be slow or fail, so we do it in background
    to avoid blocking the critical boot path.

    Retry strategy:
    - First 3 attempts: Quick retries (1s intervals)
    - Remaining attempts: Slow retries (3 minutes intervals)
    - Maximum 10 total attempts
    """
    try:
        if not lcd_manager or not lcd_manager.enabled:
            return False

        max_attempts = 10
        quick_retry_attempts = 3
        quick_retry_delay = 1  # seconds
        slow_retry_delay = 30  # 30 seconds

        for attempt in range(max_attempts):
            try:
                success = await lcd_manager.initialize_hardware(timeout_ms=500)
                if success:
                    print(f"[LCD Background] Initialized successfully (attempt {attempt + 1})")
                    return True
                else:
                    print(f"[LCD Background] Init failed (attempt {attempt + 1}/{max_attempts})")
            except Exception as e:
                error_msg = f"LCD init attempt {attempt + 1} error: {e}"
                print(f"[LCD Background] {error_msg}")
                if attempt == max_attempts - 1:
                    log_error_to_file("LCD", f"LCD initialization failed after {max_attempts} attempts")

            # Determine retry delay
            if attempt < max_attempts - 1:
                if attempt < quick_retry_attempts - 1:
                    # Quick retries for first few attempts
                    delay = quick_retry_delay
                    print(f"[LCD Background] Retrying in {delay}s...")
                else:
                    # Slow retries after quick attempts exhausted
                    delay = slow_retry_delay
                    print(f"[LCD Background] Retrying in {delay // 60} minutes...")

                await asyncio.sleep(delay)

        print("[LCD Background] Initialization failed after all attempts")
        return False
    except Exception as e:
        error_msg = f"LCD background task error: {e}"
        print(f"[LCD Background] {error_msg}")
        log_error_to_file("LCD", error_msg)
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
    # Install stdout capture FIRST (before any prints)
    from server.stdout_capture import install_print_capture
    stdout_capture = install_print_capture()

    print("=" * 50)
    print("Pico Kiln Controller - Optimized Boot")
    print("=" * 50)
    print("[Main] Stdout capture installed - logging to /stdout.log")
    print_memory_info("Boot Start")

    try:
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
        print_memory_info("Stage 1")

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
        else:
            print("[Main] Core 1 not ready after 20s - CRITICAL ERROR")
            print("[Main] System unsafe to operate - check hardware connections")
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
        print_memory_info("Stage 5")

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
        print_memory_info("Stage 8")

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

        # Error logger (with rotation)
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
        print(f"Core 2: Web + WiFi + Status + Data + Errors + Stdout{lcd_status}")
        if ip_address:
            print(f"Web interface: http://{ip_address}")
        else:
            print("Web interface: Unavailable (no WiFi)")
        print("Logs: /errors.log, /stdout.log (auto-rotate at 100KB)")
        print_memory_info("Boot Complete")
        print("=" * 50)

        # ========================================================================
        # Run all async tasks
        # ========================================================================
        tasks = [receiver_task, server_task, wifi_monitor_task, error_logger_task]
        if lcd_task:
            tasks.append(lcd_task)

        await asyncio.gather(*tasks)

    except Exception as e:
        # Log main thread errors to errors.log
        error_msg = f"Main thread error: {e}"
        print(f"[Main] {error_msg}")
        log_error_to_file("Main", error_msg)
        raise


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("\n[Main] Keyboard interrupt received")
        print("[Main] Shutting down gracefully...")
        print("[Main] Control thread will turn off SSR automatically")
        log_error_to_file("Main", "Keyboard interrupt - graceful shutdown")
        print("[Main] Shutdown complete")

    except Exception as e:
        error_msg = f"Fatal error: {e}"
        print(f"[Main] {error_msg}")
        print("[Main] Emergency shutdown - control thread should have turned off SSR")
        log_error_to_file("Main", error_msg)
        raise
