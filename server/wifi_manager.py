# server/wifi_manager.py
# Simple WiFi management for pico-kiln
# Handles connection to best available AP, automatic reconnection, and NTP time sync

import asyncio
import network
import time
from machine import Pin
from micropython import const

# Performance: const() declaration for AP scan caching
AP_CACHE_TTL = const(120)  # AP scan cache lifetime in seconds (2 minutes)

try:
    import ntptime
except ImportError:
    ntptime = None
    print("[WiFi] Warning: ntptime module not available, time sync disabled")


class WiFiManager:
    """
    Simplified WiFi manager - connects to best AP and maintains connection

    Features:
    - Automatic NTP time synchronization after connection
    - Connection monitoring and auto-reconnection
    - Status LED indication
    """

    def __init__(self, config, status_led_pin="LED"):
        """
        Initialize WiFi manager

        Args:
            config: Configuration object with WiFi settings
            status_led_pin: Pin for status LED (default: "LED")
        """
        self.ssid = config.WIFI_SSID
        self.password = config.WIFI_PASSWORD

        # Optional static IP configuration
        self.static_ip = getattr(config, 'WIFI_STATIC_IP', None)
        self.subnet = getattr(config, 'WIFI_SUBNET', None)
        self.gateway = getattr(config, 'WIFI_GATEWAY', None)
        self.dns = getattr(config, 'WIFI_DNS', None)

        self.wlan = None
        self.time_synced = False  # Track if NTP sync was successful
        self.ntp_sync_callbacks = []  # Callbacks to invoke when NTP syncs

        # AP scan cache for faster reconnections
        self.cached_bssid = None
        self.cached_rssi = None
        self.cache_timestamp = 0
        self.cache_ttl = AP_CACHE_TTL

        # Initialize status LED
        self.status_led = Pin(status_led_pin, Pin.OUT)
        self.status_led.off()

    def _update_lcd_error(self, error):
        """
        Update LCD with WiFi error (if LCD is available)

        Args:
            error: Error status code (int) or error message (str)
        """
        try:
            from server.lcd_manager import get_lcd_manager
            lcd_manager = get_lcd_manager()
            if lcd_manager and lcd_manager.enabled:
                # Format error message
                if isinstance(error, int):
                    error_msg = f"Status: {error}"
                else:
                    error_msg = str(error)
                lcd_manager.set_wifi_status(False, error=error_msg)
        except Exception as e:
            # Don't crash WiFi manager if LCD update fails
            print(f"[WiFi] Failed to update LCD with error: {e}")

    def _find_best_ap(self, use_cache=True):
        """
        Scan and find the best AP for our SSID

        Args:
            use_cache: If True, return cached BSSID if still valid (default: True)

        Returns:
            Tuple of (bssid, rssi) or (None, None) if not found
        """
        # Check cache first (if enabled and valid)
        if use_cache and self.cached_bssid:
            cache_age = time.time() - self.cache_timestamp
            if cache_age < self.cache_ttl:
                print(f"[WiFi] Using cached AP (age: {cache_age:.1f}s, RSSI: {self.cached_rssi})")
                return self.cached_bssid, self.cached_rssi

        # Cache miss or expired - perform scan
        try:
            print("[WiFi] Scanning for APs...")
            networks = self.wlan.scan()
            best_bssid = None
            best_rssi = -100

            for net in networks:
                ssid = net[0].decode()
                bssid = net[1]
                rssi = net[3]

                if ssid == self.ssid and rssi > best_rssi:
                    best_rssi = rssi
                    best_bssid = bssid

            if best_bssid:
                # Update cache
                self.cached_bssid = best_bssid
                self.cached_rssi = best_rssi
                self.cache_timestamp = time.time()
                print(f"[WiFi] Best AP found: RSSI {best_rssi} (cached for {self.cache_ttl}s)")
                return best_bssid, best_rssi

        except Exception as e:
            print(f"[WiFi] Scan failed: {e}")

        return None, None

    def register_ntp_sync_callback(self, callback):
        """
        Register a callback to be invoked when NTP sync completes

        The callback will be invoked once when NTP sync succeeds.
        Use this for operations that need accurate time (like recovery retry).

        Args:
            callback: Function to call when NTP syncs (no arguments)
        """
        if callback not in self.ntp_sync_callbacks:
            self.ntp_sync_callbacks.append(callback)

    def sync_time_ntp(self, max_attempts=3):
        """
        Synchronize time with NTP server with retry logic

        Implements retry with exponential backoff to handle transient NTP server issues.
        Sets self.time_synced flag on success for recovery system validation.
        Invokes registered callbacks on successful sync.

        Args:
            max_attempts: Maximum number of retry attempts (default: 3)

        Returns:
            True if successful, False otherwise
        """
        if ntptime is None:
            print("[WiFi] NTP sync skipped - ntptime module not available")
            self.time_synced = False
            return False

        for attempt in range(max_attempts):
            try:
                print(f"[WiFi] Syncing time with NTP server (attempt {attempt + 1}/{max_attempts})...")
                ntptime.settime()

                # Print synchronized time
                local_time = time.localtime()
                time_str = f"{local_time[0]:04d}-{local_time[1]:02d}-{local_time[2]:02d} {local_time[3]:02d}:{local_time[4]:02d}:{local_time[5]:02d}"
                print(f"[WiFi] Time synchronized: {time_str} UTC")

                self.time_synced = True

                # Invoke NTP sync callbacks
                self._invoke_ntp_callbacks()

                return True

            except Exception as e:
                if attempt < max_attempts - 1:
                    backoff_time = 1.0 * (attempt + 1)  # 1s, 2s
                    print(f"[WiFi] NTP sync attempt {attempt + 1} failed: {e}, retrying in {backoff_time:.1f}s...")
                    time.sleep(backoff_time)
                else:
                    print(f"[WiFi] NTP sync failed after {max_attempts} attempts: {e}")
                    self.time_synced = False

        return False

    def _invoke_ntp_callbacks(self):
        """
        Invoke all registered NTP sync callbacks

        Callbacks are invoked once and then cleared to prevent multiple invocations.
        Errors in callbacks are caught to prevent blocking NTP sync completion.
        """
        if not self.ntp_sync_callbacks:
            return

        print(f"[WiFi] Invoking {len(self.ntp_sync_callbacks)} NTP sync callback(s)...")

        for callback in self.ntp_sync_callbacks:
            try:
                callback()
            except Exception as e:
                print(f"[WiFi] Error in NTP sync callback: {e}")

        # Clear callbacks after invoking (one-time use)
        self.ntp_sync_callbacks.clear()

    async def connect(self, timeout=30, use_cache=True):
        """
        Connect to the best available AP using MicroPython's built-in features

        Args:
            timeout: Connection timeout in seconds (default: 30)
            use_cache: Use cached AP BSSID if available (default: True)

        Returns:
            IP address if successful, None if failed
        """
        print(f"[WiFi] Connecting to {self.ssid}...")

        # Initialize WLAN if needed
        if not self.wlan:
            self.wlan = network.WLAN(network.STA_IF)

        # Activate WiFi interface
        was_active = self.wlan.active()
        self.wlan.active(True)

        if not was_active:
            # Configure WiFi for maximum reliability (first time only)
            print("[WiFi] Configuring WiFi hardware...")

            # PM_NONE: Disable power saving for maximum reliability
            # Note: reconnects parameter not supported on Pico 2W
            self.wlan.config(pm=network.WLAN.PM_NONE)
            print("[WiFi] Power management: PM_NONE (max reliability)")

            # Set static IP if configured (faster than DHCP)
            if self.static_ip and self.subnet and self.gateway and self.dns:
                print(f"[WiFi] Configuring static IP: {self.static_ip}")
                self.wlan.ifconfig((self.static_ip, self.subnet, self.gateway, self.dns))
            else:
                print("[WiFi] Using DHCP for IP address")

        # Find and connect to best AP (with optional caching)
        best_bssid, best_rssi = self._find_best_ap(use_cache=use_cache)

        if best_bssid:
            print(f"[WiFi] Connecting to AP (RSSI {best_rssi}dBm)...")
            self.wlan.connect(self.ssid, self.password, bssid=best_bssid)
        else:
            print(f"[WiFi] No specific AP found, connecting to {self.ssid}...")
            self.wlan.connect(self.ssid, self.password)

        # Wait for connection
        start_time = time.time()
        led_state = False

        while True:
            status = self.wlan.status()

            # Check if connection failed (status < 0) or succeeded (status >= 3)
            if status < 0:
                print(f"[WiFi] Connection failed with status {status}")
                self.status_led.off()
                # Update LCD with error status
                self._update_lcd_error(status)
                return None
            elif status >= 3:
                # Connected!
                break

            # Check timeout
            if time.time() - start_time > timeout:
                print(f"[WiFi] Connection timeout after {timeout}s (status: {status})")
                self.status_led.off()
                # Update LCD with timeout error
                self._update_lcd_error(f"Timeout (st:{status})")
                return None

            # Blink LED while connecting
            led_state = not led_state
            self.status_led.value(1 if led_state else 0)

            await asyncio.sleep(0.5)

        # Connected!
        self.status_led.on()

        ip = self.wlan.ifconfig()[0]
        print(f"[WiFi] Connected! IP: {ip}")

        # NTP sync moved to background task - don't block here
        print("[WiFi] Connection complete (NTP sync will happen in background)")

        return ip

    async def monitor(self, check_interval=5):
        """
        Monitor connection and reconnect if dropped

        Implements retry logic with progressive backoff to handle transient
        WiFi issues without crashing the monitor task.
        """
        reconnect_failures = 0
        max_consecutive_failures = 10

        while True:
            await asyncio.sleep(check_interval)

            if self.wlan and not self.wlan.isconnected():
                try:
                    print("[WiFi] Connection lost, reconnecting...")
                    self.status_led.off()

                    # Attempt reconnection
                    self.wlan.disconnect()
                    ip_address = await self.connect(timeout=30)

                    # Success - update LCD and reset failure counter
                    if ip_address:
                        from server.lcd_manager import get_lcd_manager
                        lcd_manager = get_lcd_manager()
                        if lcd_manager and lcd_manager.enabled:
                            lcd_manager.set_wifi_status(True, ip_address)

                        reconnect_failures = 0
                    else:
                        # Connection failed, increment counter
                        reconnect_failures += 1
                        print(f"[WiFi] Reconnect attempt failed ({reconnect_failures}/{max_consecutive_failures})")

                except Exception as e:
                    reconnect_failures += 1
                    print(f"[WiFi] Reconnect failed ({reconnect_failures}/{max_consecutive_failures}): {e}")

                # Check if we've hit max failures
                if reconnect_failures >= max_consecutive_failures:
                    # Too many failures - wait longer before next attempt
                    print("[WiFi] Max reconnect failures reached - waiting 60s before retry")
                    reconnect_failures = 0  # Reset to try again later
                    await asyncio.sleep(60)  # Extended wait
