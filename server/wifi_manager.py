# server/wifi_manager.py
# Simple WiFi management for pico-kiln
# Handles connection to best available AP, automatic reconnection, and NTP time sync

import asyncio
import network
import time
from machine import Pin

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

    def __init__(self, ssid, password, status_led_pin="LED"):
        """
        Initialize WiFi manager

        Args:
            ssid: WiFi network name
            password: WiFi password
            status_led_pin: Pin for status LED (default: "LED")
        """
        self.ssid = ssid
        self.password = password
        self.wlan = None

        # Initialize status LED
        self.status_led = Pin(status_led_pin, Pin.OUT)
        self.status_led.off()

    def _find_best_ap(self):
        """Scan and find the best AP for our SSID"""
        try:
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
                return best_bssid, best_rssi

        except Exception as e:
            print(f"[WiFi] Scan failed: {e}")

        return None, None

    def sync_time_ntp(self):
        """
        Synchronize time with NTP server (simple version)

        Returns:
            True if successful, False otherwise
        """
        if ntptime is None:
            print("[WiFi] NTP sync skipped - ntptime module not available")
            return False

        try:
            print("[WiFi] Syncing time with NTP server...")
            ntptime.settime()

            # Print synchronized time
            local_time = time.localtime()
            time_str = f"{local_time[0]:04d}-{local_time[1]:02d}-{local_time[2]:02d} {local_time[3]:02d}:{local_time[4]:02d}:{local_time[5]:02d}"
            print(f"[WiFi] Time synchronized: {time_str} UTC")
            return True

        except Exception as e:
            print(f"[WiFi] NTP sync failed: {e}")
            return False

    async def connect(self, timeout=30):
        """Connect to the best available AP"""
        print(f"[WiFi] Connecting to {self.ssid}...")

        # Initialize WLAN if needed
        if not self.wlan:
            self.wlan = network.WLAN(network.STA_IF)

        self.wlan.active(True)

        # Find and connect to best AP
        best_bssid, best_rssi = self._find_best_ap()

        if best_bssid:
            print(f"[WiFi] Found AP with RSSI {best_rssi}, connecting...")
            self.wlan.connect(self.ssid, self.password, bssid=best_bssid)
        else:
            print(f"[WiFi] No specific AP found, connecting to {self.ssid}...")
            self.wlan.connect(self.ssid, self.password)

        # Wait for connection
        start_time = time.time()
        led_state = False

        while not self.wlan.isconnected():
            if time.time() - start_time > timeout:
                print(f"[WiFi] Connection timeout after {timeout}s")
                self.status_led.off()
                return None

            # Blink LED while connecting
            led_state = not led_state
            self.status_led.value(1 if led_state else 0)

            await asyncio.sleep(0.5)

        # Connected!
        self.status_led.on()

        ip = self.wlan.ifconfig()[0]
        print(f"[WiFi] Connected! IP: {ip}")

        # Sync time with NTP server
        self.sync_time_ntp()

        return ip

    async def monitor(self, check_interval=5):
        """Monitor connection and reconnect if dropped"""
        while True:
            await asyncio.sleep(check_interval)

            if self.wlan and not self.wlan.isconnected():
                print("[WiFi] Connection lost, reconnecting...")
                self.status_led.off()

                # Reconnect
                self.wlan.disconnect()
                await self.connect(timeout=30)
