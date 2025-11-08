# server/lcd_manager.py
# LCD Display Manager for 1602 I2C LCD
#
# Manages display output and button input for optional LCD interface.
# Runs on Core 2 to avoid interfering with control thread.

import asyncio
import time
from machine import I2C, Pin
from micropython import const

# Performance: const() declaration for button debouncing
BUTTON_DEBOUNCE_MS = const(300)  # Button debounce time in milliseconds

class Screen:
    """Screen identifiers"""
    WIFI = "wifi"
    STATE = "state"
    TEMP = "temp"
    PROFILE = "profile"
    RATE = "rate"
    TUNING_DETAIL = "tuning_detail"
    STOP = "stop"
    STOP_CONFIRM = "stop_confirm"


class LCDManager:
    """
    Manages LCD display and button input for kiln controller

    Features:
    - Multiple screens (WiFi status, state, temp, profile, stop)
    - Button navigation (next screen, select)
    - Initialization status messages
    - Optional hardware (gracefully handles missing LCD/buttons)
    - Direct reads from StatusCache (no listener pattern)
    """

    def __init__(self, config, command_queue, status_receiver):
        """
        Initialize LCD manager

        Args:
            config: Configuration object with LCD settings
            command_queue: Queue for sending commands to control thread
            status_receiver: StatusReceiver instance for reading cached status
        """
        self.config = config
        self.command_queue = command_queue
        self.status_receiver = status_receiver
        
        # Check if LCD is enabled in config
        self.enabled = hasattr(config, 'LCD_I2C_SCL') and hasattr(config, 'LCD_I2C_SDA')
        
        if not self.enabled:
            print("[LCD] LCD not configured, display disabled")
            self.lcd = None
            self.btn_next = None
            self.btn_select = None
            return

        # Hardware components (initialized separately via initialize_hardware)
        self.lcd = None
        self.btn_next = None
        self.btn_select = None

        # Screen state
        self.current_screen = Screen.WIFI
        self.screen_order = [
            Screen.WIFI, 
            Screen.STATE, 
            Screen.TEMP, 
            Screen.PROFILE, 
            Screen.RATE,
            Screen.STOP
        ]
        
        # Dynamic screen visibility (some screens only shown in certain states)
        self.always_visible_screens = [Screen.WIFI, Screen.STATE, Screen.TEMP, Screen.STOP]
        
        # Button debouncing
        self.btn_next_last_press = 0
        self.btn_select_last_press = 0
        self.btn_debounce_ms = BUTTON_DEBOUNCE_MS

        # WiFi status (not in StatusCache)
        self.wifi_ip = None
        self.wifi_connected = False
        self.wifi_error = None  # WiFi error message (if any)

        # Initialization tracking
        self.init_steps_completed = []

    async def initialize_hardware(self, timeout_ms=500):
        """
        Initialize LCD hardware (async, non-blocking)

        Uses asyncio.sleep() to avoid blocking the event loop during initialization.
        Has timeout protection to prevent system freeze if LCD hardware fails.

        Args:
            timeout_ms: Timeout in milliseconds (default: 500ms)

        Returns:
            True if initialization successful, False otherwise
        """
        if not self.enabled:
            return False

        print(f"[LCD] Initializing hardware with {timeout_ms}ms timeout...")

        try:
            # Initialize I2C
            i2c = I2C(
                self.config.LCD_I2C_ID,
                scl=Pin(self.config.LCD_I2C_SCL),
                sda=Pin(self.config.LCD_I2C_SDA),
                freq=self.config.LCD_I2C_FREQ
            )

            # Test I2C bus by scanning for device
            try:
                devices = i2c.scan()
                if self.config.LCD_I2C_ADDR not in devices:
                    print(f"[LCD] WARNING: Device not found at 0x{self.config.LCD_I2C_ADDR:02x}")
                    print(f"[LCD] Found devices: {[hex(d) for d in devices]}")
                    raise Exception(f"LCD not found on I2C bus at 0x{self.config.LCD_I2C_ADDR:02x}")
                print(f"[LCD] Device detected at 0x{self.config.LCD_I2C_ADDR:02x}")
            except Exception as e:
                print(f"[LCD] I2C bus scan failed: {e}")
                raise

            # Create LCD object (does not initialize hardware yet)
            from lib.lcd1602_i2c import LCD1602
            self.lcd = LCD1602(i2c, addr=self.config.LCD_I2C_ADDR)

            # Initialize LCD hardware with timeout using asyncio.wait_for
            await asyncio.wait_for(
                self.lcd.initialize(),
                timeout_ms / 1000.0  # Convert to seconds
            )

            print(f"[LCD] Display initialized successfully")

            # Initialize buttons if configured
            if hasattr(self.config, 'LCD_BTN_NEXT_PIN'):
                self.btn_next = Pin(self.config.LCD_BTN_NEXT_PIN, Pin.IN, Pin.PULL_UP)
                print(f"[LCD] Next button configured on pin {self.config.LCD_BTN_NEXT_PIN}")

            if hasattr(self.config, 'LCD_BTN_SELECT_PIN'):
                self.btn_select = Pin(self.config.LCD_BTN_SELECT_PIN, Pin.IN, Pin.PULL_UP)
                print(f"[LCD] Select button configured on pin {self.config.LCD_BTN_SELECT_PIN}")

            return True

        except asyncio.TimeoutError:
            print(f"[LCD] CRITICAL: Hardware initialization TIMED OUT after {timeout_ms}ms")
            print("[LCD] This may indicate I2C bus issues or wrong address")
            print("[LCD] Display disabled - system will continue without LCD")
            self.lcd = None
            self.btn_next = None
            self.btn_select = None
            self.enabled = False
            return False

        except Exception as e:
            print(f"[LCD] CRITICAL: Failed to initialize LCD hardware: {e}")
            print("[LCD] Display disabled - system will continue without LCD")
            self.lcd = None
            self.btn_next = None
            self.btn_select = None
            self.enabled = False
            return False

    def show_init_message(self, message):
        """
        Show initialization message during startup
        
        Args:
            message: Short message to display (max 16 chars per line)
        """
        if not self.enabled or not self.lcd:
            return
        
        try:
            self.lcd.clear()
            self.lcd.print("Initializing...", row=0)
            self.lcd.print(message[:16], row=1)
            self.init_steps_completed.append(message)
        except Exception as e:
            print(f"[LCD] Error showing init message: {e}")
    
    def set_wifi_status(self, connected, ip_address=None, error=None):
        """
        Update WiFi connection status and re-render if on WiFi screen

        Args:
            connected: True if WiFi connected
            ip_address: IP address if connected
            error: Error message if connection failed (e.g., "Status: -1")
        """
        self.wifi_connected = connected
        self.wifi_ip = ip_address
        self.wifi_error = error
        # Re-render if currently showing WiFi screen
        if self.current_screen == Screen.WIFI:
            asyncio.create_task(self._render_current_screen())
    
    def _render_current_screen_sync(self):
        """Render the current screen (synchronous, can block)"""
        if not self.enabled or not self.lcd:
            return

        if self.current_screen == Screen.WIFI:
            self._render_wifi_screen()
        elif self.current_screen == Screen.STATE:
            self._render_state_screen()
        elif self.current_screen == Screen.TEMP:
            self._render_temp_screen()
        elif self.current_screen == Screen.PROFILE:
            self._render_profile_screen()
        elif self.current_screen == Screen.RATE:
            self._render_rate_screen()
        elif self.current_screen == Screen.TUNING_DETAIL:
            self._render_tuning_detail_screen()
        elif self.current_screen == Screen.STOP:
            self._render_stop_screen()
        elif self.current_screen == Screen.STOP_CONFIRM:
            self._render_stop_confirm_screen()

    async def _render_current_screen(self):
        """Render the current screen with timeout protection"""
        if not self.enabled or not self.lcd:
            return

        try:
            # Wrap render in timeout - if it takes >200ms, something is wrong
            await asyncio.wait_for(
                self._render_sync_as_async(),
                timeout=0.2  # 200ms timeout
            )
        except asyncio.TimeoutError:
            print(f"[LCD] Render timeout on screen {self.current_screen}")
            raise Exception("LCD render timeout")
        except Exception as e:
            print(f"[LCD] Error rendering screen: {e}")
            raise

    async def _render_sync_as_async(self):
        """Wrapper to run synchronous render in async context"""
        self._render_current_screen_sync()
        await asyncio.sleep(0)  # Yield to event loop
    
    def _render_wifi_screen(self):
        """Render WiFi status screen"""
        if self.wifi_connected and self.wifi_ip:
            self.lcd.print("WiFi: Connected", row=0)
            # Truncate IP if too long
            ip_text = self.wifi_ip[:16]
            self.lcd.print(ip_text, row=1)
        elif self.wifi_error:
            # Show error status
            self.lcd.print("WiFi: Failed", row=0)
            self.lcd.print(self.wifi_error[:16], row=1)
        else:
            self.lcd.print("WiFi: Not Conn.", row=0)
            self.lcd.print("", row=1)
    
    def _render_state_screen(self):
        """Render current state screen"""
        state = self.status_receiver.get_status_field('state', 'UNKNOWN')

        # Show state on first line
        self.lcd.print(f"State: {state[:9]}", row=0)

        # Show additional info on second line based on state
        if state == 'RUNNING':
            is_recovering = self.status_receiver.get_status_field('is_recovering', False)
            if is_recovering:
                # Show recovery target and distance
                recovery_target = self.status_receiver.get_status_field('recovery_target_temp')
                current_temp = self.status_receiver.get_status_field('current_temp', 0.0)
                if recovery_target:
                    temp_diff = recovery_target - current_temp
                    self.lcd.print(f"Recov:{recovery_target:.0f}C {temp_diff:+.0f}", row=1)
                else:
                    self.lcd.print("(Recovering)", row=1)
            else:
                step = self.status_receiver.get_status_field('current_step', 0)
                total = self.status_receiver.get_status_field('total_steps', 0)
                self.lcd.print(f"Step {step}/{total}", row=1)
        elif state == 'TUNING':
            self.lcd.print("Auto-tuning PID", row=1)
        elif state == 'ERROR':
            # Show actual error message (truncated to fit)
            error_msg = self.status_receiver.get_status_field('error', 'Unknown error')
            self.lcd.print(error_msg[:16], row=1)
        elif state == 'COMPLETE':
            self.lcd.print("Profile Done!", row=1)
        else:
            self.lcd.print("", row=1)
    
    def _render_temp_screen(self):
        """Render temperature screen with SSR output"""
        current = self.status_receiver.get_status_field('current_temp', 0.0)
        target = self.status_receiver.get_status_field('target_temp', 0.0)
        ssr_output = self.status_receiver.get_status_field('ssr_output', 0.0)

        # Format temperatures to fit: "Cur: 999C"
        self.lcd.print(f"Cur:{current:4.0f}C", row=0)

        if target > 0:
            # Show target and SSR output on second line
            self.lcd.print(f"Tgt:{target:4.0f}C {ssr_output:.0f}%", row=1)
        else:
            self.lcd.print(f"Tgt: -- SSR:{ssr_output:.0f}%", row=1)
    
    def _render_profile_screen(self):
        """Render profile/tuning method screen with time estimates"""
        state = self.status_receiver.get_status_field('state', 'IDLE')

        if state == 'RUNNING':
            profile_name = self.status_receiver.get_status_field('profile', 'Unknown')
            elapsed = self.status_receiver.get_status_field('elapsed', 0)
            remaining = self.status_receiver.get_status_field('remaining', 0)
            progress = self.status_receiver.get_status_field('progress', 0)

            # First line: Profile name (truncated)
            self.lcd.print(profile_name[:16], row=0)

            # Second line: Progress and time
            if remaining > 0:
                # Format time: hours and minutes
                remain_hours = int(remaining / 3600)
                remain_mins = int((remaining % 3600) / 60)
                if remain_hours > 0:
                    time_str = f"{remain_hours}h{remain_mins:02d}m"
                else:
                    time_str = f"{remain_mins}m"
                self.lcd.print(f"{progress:3.0f}% {time_str:>9}", row=1)
            else:
                self.lcd.print(f"Progress: {progress:.0f}%", row=1)

        elif state == 'TUNING':
            tuning_mode = self.status_receiver.get_status_field('tuning_mode', 'Unknown')
            self.lcd.print("Tuning Mode:", row=0)
            self.lcd.print(tuning_mode[:16], row=1)
        else:
            self.lcd.print("Profile:", row=0)
            self.lcd.print("None", row=1)
    
    def _render_stop_screen(self):
        """Render stop menu screen"""
        state = self.status_receiver.get_status_field('state', 'IDLE')

        if state in ['RUNNING', 'TUNING']:
            self.lcd.print("Stop Program?", row=0)
            self.lcd.print("Press Select", row=1)
        else:
            self.lcd.print("Stop Program", row=0)
            self.lcd.print("(Not running)", row=1)
    
    def _render_stop_confirm_screen(self):
        """Render stop confirmation screen"""
        self.lcd.print("Are you sure?", row=0)
        self.lcd.print("Press Select", row=1)
    
    def _render_rate_screen(self):
        """Render rate monitoring screen (only during RUNNING)"""
        state = self.status_receiver.get_status_field('state', 'IDLE')

        if state != 'RUNNING':
            self.lcd.print("Rate Monitor", row=0)
            self.lcd.print("(Not running)", row=1)
            return

        # Show rate information with adaptation count
        desired_rate = self.status_receiver.get_status_field('desired_rate', 0)
        actual_rate = self.status_receiver.get_status_field('actual_rate', 0)
        current_rate = self.status_receiver.get_status_field('current_rate', 0)
        adaptation_count = self.status_receiver.get_status_field('adaptation_count', 0)

        # First line: Desired vs Actual with warning if significantly behind
        rate_warning = ""
        if desired_rate > 0 and actual_rate < desired_rate * 0.85:
            rate_warning = "!"
        self.lcd.print(f"D:{desired_rate:3.0f} A:{actual_rate:3.0f}{rate_warning}", row=0)

        # Second line: Current (adapted) rate and adaptation count
        if adaptation_count > 0:
            self.lcd.print(f"Now:{current_rate:3.0f} Ad:{adaptation_count}", row=1)
        else:
            self.lcd.print(f"Current:{current_rate:3.0f}", row=1)
    

    def _render_tuning_detail_screen(self):
        """Render detailed tuning progress screen"""
        state = self.status_receiver.get_status_field('state', 'IDLE')

        if state != 'TUNING':
            self.lcd.print("Tuning Detail", row=0)
            self.lcd.print("(Not tuning)", row=1)
            return

        # Show tuning progress details
        # Note: These fields would need to be added to tuning status
        tuning_step = self.status_receiver.get_status_field('tuning_step', 'Unknown')
        tuning_progress = self.status_receiver.get_status_field('tuning_progress', '?/?')
        current_temp = self.status_receiver.get_status_field('current_temp', 0.0)

        # First line: Step and progress
        self.lcd.print(f"{tuning_step[:10]}{tuning_progress:>6}", row=0)

        # Second line: Current temperature
        self.lcd.print(f"Temp: {current_temp:.0f}C", row=1)
    
    def _handle_next_button(self):
        """Handle next button press - change screen and render immediately"""
        # Move to next screen, skipping screens not relevant to current state
        state = self.status_receiver.get_status_field('state', 'IDLE')

        current_index = self.screen_order.index(self.current_screen)
        attempts = 0
        max_attempts = len(self.screen_order)

        while attempts < max_attempts:
            next_index = (current_index + 1 + attempts) % len(self.screen_order)
            next_screen = self.screen_order[next_index]

            # Check if screen is relevant
            if next_screen in self.always_visible_screens:
                self.current_screen = next_screen
                break
            elif next_screen == Screen.PROFILE and state in ['RUNNING', 'TUNING']:
                self.current_screen = next_screen
                break
            elif next_screen == Screen.RATE and state == 'RUNNING':
                self.current_screen = next_screen
                break
            elif next_screen == Screen.TUNING_DETAIL and state == 'TUNING':
                self.current_screen = next_screen
                break

            attempts += 1

        print(f"[LCD] Screen changed to: {self.current_screen}")
        # Render the new screen immediately
        asyncio.create_task(self._render_current_screen())
    
    def _handle_select_button(self):
        """Handle select button press - refresh data and re-render"""
        if self.current_screen == Screen.STOP:
            # Check if something is running
            state = self.status_receiver.get_status_field('state', 'IDLE')
            if state in ['RUNNING', 'TUNING']:
                # Show confirmation screen
                self.current_screen = Screen.STOP_CONFIRM
                asyncio.create_task(self._render_current_screen())
        elif self.current_screen == Screen.STOP_CONFIRM:
            # Confirmed - send stop command
            print("[LCD] Stop confirmed, sending stop command")
            from kiln.comms import CommandMessage, QueueHelper
            command = CommandMessage.stop()

            if QueueHelper.put_nowait(self.command_queue, command):
                print("[LCD] Stop command sent successfully")
            else:
                print("[LCD] Failed to send stop command (queue full)")

            # Return to state screen after short delay (will render there)
            asyncio.create_task(self._return_to_state_screen_delayed())
        else:
            # On any other screen, select button refreshes the display
            print(f"[LCD] Refreshing screen: {self.current_screen}")
            asyncio.create_task(self._render_current_screen())
    
    async def _return_to_state_screen_delayed(self):
        """Return to state screen after 2 seconds"""
        await asyncio.sleep(2)
        self.current_screen = Screen.STATE
        await self._render_current_screen()
    
    def _check_buttons(self):
        """Check button states and handle presses (with debouncing)"""
        if not self.enabled:
            return
        
        current_time = time.ticks_ms()
        
        # Check next button (active low with pull-up)
        if self.btn_next and self.btn_next.value() == 0:
            if time.ticks_diff(current_time, self.btn_next_last_press) > self.btn_debounce_ms:
                self.btn_next_last_press = current_time
                self._handle_next_button()
        
        # Check select button (active low with pull-up)
        if self.btn_select and self.btn_select.value() == 0:
            if time.ticks_diff(current_time, self.btn_select_last_press) > self.btn_debounce_ms:
                self.btn_select_last_press = current_time
                self._handle_select_button()
    
    async def run(self):
        """
        Main LCD update loop

        Runs on Core 2 as an async task.
        - Renders initial screen
        - Checks button presses at 10Hz (buttons trigger renders directly)
        """
        if not self.enabled:
            print("[LCD] LCD manager not enabled, exiting")
            return

        # Wait for LCD hardware to be initialized
        while not self.lcd:
            await asyncio.sleep(0.1)

        print("[LCD] Starting LCD update loop")

        # Show initial screen
        try:
            await self._render_current_screen()
        except Exception as e:
            print(f"[LCD] Failed to render initial screen: {e}")

        # Main loop: check buttons at 10Hz (buttons trigger renders directly)
        consecutive_errors = 0
        max_consecutive_errors = 3

        while True:
            try:
                # Check buttons (responsive at 10Hz)
                # Button handlers will queue render tasks as needed
                self._check_buttons()

                # Reset error counter on successful loop iteration
                consecutive_errors = 0

                # Run at ~10Hz for responsive button handling
                await asyncio.sleep(0.1)

            except Exception as e:
                consecutive_errors += 1
                print(f"[LCD] Error in update loop ({consecutive_errors}/{max_consecutive_errors}): {e}")

                # If too many consecutive errors, disable LCD to prevent crash loop
                if consecutive_errors >= max_consecutive_errors:
                    print(f"[LCD] CRITICAL: Disabling LCD after {max_consecutive_errors} errors")
                    print(f"[LCD] Last error: {e}")
                    print(f"[LCD] Web server and WiFi should remain functional")
                    self.enabled = False
                    self.lcd = None
                    return

                await asyncio.sleep(1)  # Back off on errors


def get_lcd_manager():
    """Get singleton LCD manager instance"""
    global _lcd_manager_instance
    if '_lcd_manager_instance' not in globals():
        _lcd_manager_instance = None
    return _lcd_manager_instance


def initialize_lcd_manager(config, command_queue, status_receiver):
    """
    Initialize global LCD manager instance

    Args:
        config: Configuration object
        command_queue: Command queue for control thread
        status_receiver: StatusReceiver instance for reading cached status

    Returns:
        LCDManager instance (or None if disabled)
    """
    global _lcd_manager_instance
    _lcd_manager_instance = LCDManager(config, command_queue, status_receiver)
    return _lcd_manager_instance
