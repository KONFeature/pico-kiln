# server/wifi_manager.py
# Simplified WiFi management for pico-kiln
# Relies on MicroPython's built-in auto-retry and reconnection

import asyncio
import network
import time
from machine import Pin

try:
    import ntptime
except ImportError:
    ntptime = None
    print("[WiFi] Warning: ntptime module not available")


class WiFiManager:
    """
    Minimal WiFi manager - connect once, MicroPython handles the rest

    Features:
    - One-time connection (with optional best AP selection)
    - NTP time synchronization
    - Status monitoring for LED/LCD updates
    """

    def __init__(self, config, status_led_pin="LED"):
        self.ssid = config.WIFI_SSID
        self.password = config.WIFI_PASSWORD

        # Optional static IP
        self.static_ip = getattr(config, 'WIFI_STATIC_IP', None)
        self.subnet = getattr(config, 'WIFI_SUBNET', None)
        self.gateway = getattr(config, 'WIFI_GATEWAY', None)
        self.dns = getattr(config, 'WIFI_DNS', None)

        self.wlan = None
        self.time_synced = False
        self.status_led = Pin(status_led_pin, Pin.OUT)
        self.status_led.off()

    def sync_time_ntp(self, max_attempts=3):
        """Synchronize time with NTP server"""
        if ntptime is None:
            return False

        for attempt in range(max_attempts):
            try:
                print(f"[WiFi] Syncing time (attempt {attempt + 1}/{max_attempts})...")
                ntptime.settime()

                local_time = time.localtime()
                time_str = f"{local_time[0]:04d}-{local_time[1]:02d}-{local_time[2]:02d} {local_time[3]:02d}:{local_time[4]:02d}:{local_time[5]:02d}"
                print(f"[WiFi] Time synchronized: {time_str} UTC")

                self.time_synced = True
                return True

            except Exception as e:
                if attempt < max_attempts - 1:
                    print(f"[WiFi] NTP sync failed: {e}, retrying...")
                    time.sleep(1.0 * (attempt + 1))
                else:
                    print(f"[WiFi] NTP sync failed after {max_attempts} attempts: {e}")

        return False

    async def connect(self, timeout=30, scan_for_best_ap=True):
        """
        Connect to WiFi (called once at boot)

        Args:
            timeout: How long to wait for initial connection
            scan_for_best_ap: If True, scan and connect to strongest signal

        Returns:
            IP address if successful, None if timeout
        """
        print(f"[WiFi] Connecting to {self.ssid}...")

        # Initialize WLAN
        self.wlan = network.WLAN(network.STA_IF)
        self.wlan.active(True)

        # Configure for reliability
        self.wlan.config(pm=network.WLAN.PM_NONE)

        # Static IP if configured
        if self.static_ip and self.subnet and self.gateway and self.dns:
            print(f"[WiFi] Using static IP: {self.static_ip}")
            self.wlan.ifconfig((self.static_ip, self.subnet, self.gateway, self.dns))

        # Find best AP if requested
        bssid = None
        if scan_for_best_ap:
            try:
                print("[WiFi] Scanning for best AP...")
                networks = self.wlan.scan()
                best_rssi = -100

                for net in networks:
                    if net[0].decode() == self.ssid and net[3] > best_rssi:
                        best_rssi = net[3]
                        bssid = net[1]

                if bssid:
                    print(f"[WiFi] Found best AP (RSSI: {best_rssi}dBm)")
            except Exception as e:
                print(f"[WiFi] Scan failed: {e}, connecting anyway...")

        # Connect (MicroPython will retry forever)
        if bssid:
            self.wlan.connect(self.ssid, self.password, bssid=bssid)
        else:
            self.wlan.connect(self.ssid, self.password)

        # Wait for initial connection
        start_time = time.time()
        led_state = False

        while time.time() - start_time < timeout:
            if self.wlan.status() >= 3:
                # Connected!
                self.status_led.on()
                ip = self.wlan.ifconfig()[0]
                print(f"[WiFi] Connected! IP: {ip}")
                return ip

            # Blink LED while connecting
            led_state = not led_state
            self.status_led.value(1 if led_state else 0)
            await asyncio.sleep(0.5)

        # Timeout, but MicroPython keeps trying in background
        print(f"[WiFi] Timeout after {timeout}s, MicroPython will keep trying...")
        self.status_led.off()
        return None

    async def monitor(self, check_interval=5):
        """
        Monitor connection and update LED/LCD

        MicroPython auto-reconnects after successful connection, but if the initial
        connection fails, we need to manually retry by disconnect/reconnect.
        """
        was_connected = self.wlan.isconnected() if self.wlan else False

        while True:
            await asyncio.sleep(check_interval)

            if not self.wlan:
                continue

            status = self.wlan.status()
            is_connected = self.wlan.isconnected()

            # Check for connection failure states that require manual retry
            # STAT_WRONG_PASSWORD, STAT_NO_AP_FOUND, STAT_CONNECT_FAIL
            if status in (network.STAT_WRONG_PASSWORD, network.STAT_NO_AP_FOUND, network.STAT_CONNECT_FAIL):
                print(f"[WiFi] Connection failed (status={status}), retrying...")
                self.status_led.off()
                
                # Disconnect, wait, reconnect
                self.wlan.disconnect()
                await asyncio.sleep(2)
                self.wlan.connect(self.ssid, self.password)
                
                # Don't update was_connected here, let next iteration handle it
                continue

            # Connection state changed
            if is_connected != was_connected:
                if is_connected:
                    # Reconnected!
                    ip = self.wlan.ifconfig()[0]
                    print(f"[WiFi] Reconnected! IP: {ip}")
                    self.status_led.on()

                    # Re-sync time if needed
                    if not self.time_synced:
                        self.sync_time_ntp()
                else:
                    # Connection lost
                    print("[WiFi] Connection lost (auto-reconnecting...)")
                    self.status_led.off()

                was_connected = is_connected
