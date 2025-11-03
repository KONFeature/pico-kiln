# server/lcd_manager.py
# LCD Display Manager for 1602 I2C LCD
#
# Manages display output and button input for optional LCD interface.
# Runs on Core 2 to avoid interfering with control thread.

import asyncio
import time
from machine import I2C, Pin

class Screen:
    """Screen identifiers"""
    WIFI = "wifi"
    STATE = "state"
    TEMP = "temp"
    PROFILE = "profile"
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
    - Status updates from kiln controller
    """
    
    def __init__(self, config, command_queue):
        """
        Initialize LCD manager
        
        Args:
            config: Configuration object with LCD settings
            command_queue: Queue for sending commands to control thread
        """
        self.config = config
        self.command_queue = command_queue
        
        # Check if LCD is enabled in config
        self.enabled = hasattr(config, 'LCD_I2C_SCL') and hasattr(config, 'LCD_I2C_SDA')
        
        if not self.enabled:
            print("[LCD] LCD not configured, display disabled")
            self.lcd = None
            self.btn_next = None
            self.btn_select = None
            return
        
        # Initialize hardware
        try:
            # Initialize I2C
            i2c = I2C(
                config.LCD_I2C_ID,
                scl=Pin(config.LCD_I2C_SCL),
                sda=Pin(config.LCD_I2C_SDA),
                freq=config.LCD_I2C_FREQ
            )
            
            # Initialize LCD
            from lib.lcd1602_i2c import LCD1602
            self.lcd = LCD1602(i2c, addr=config.LCD_I2C_ADDR)
            print(f"[LCD] Display initialized at I2C address 0x{config.LCD_I2C_ADDR:02x}")
            
            # Initialize buttons if configured
            self.btn_next = None
            self.btn_select = None
            
            if hasattr(config, 'LCD_BTN_NEXT_PIN'):
                self.btn_next = Pin(config.LCD_BTN_NEXT_PIN, Pin.IN, Pin.PULL_UP)
                print(f"[LCD] Next button configured on pin {config.LCD_BTN_NEXT_PIN}")
            
            if hasattr(config, 'LCD_BTN_SELECT_PIN'):
                self.btn_select = Pin(config.LCD_BTN_SELECT_PIN, Pin.IN, Pin.PULL_UP)
                print(f"[LCD] Select button configured on pin {config.LCD_BTN_SELECT_PIN}")
            
        except Exception as e:
            print(f"[LCD] Failed to initialize LCD hardware: {e}")
            print("[LCD] Display disabled")
            self.lcd = None
            self.btn_next = None
            self.btn_select = None
            self.enabled = False
            return
        
        # Screen state
        self.current_screen = Screen.WIFI
        self.screen_order = [Screen.WIFI, Screen.STATE, Screen.TEMP, Screen.PROFILE, Screen.STOP]
        
        # Button debouncing
        self.btn_next_last_press = 0
        self.btn_select_last_press = 0
        self.btn_debounce_ms = 300  # 300ms debounce
        
        # Cached status data
        self.cached_status = {}
        self.wifi_ip = None
        self.wifi_connected = False
        
        # Initialization tracking
        self.init_steps_completed = []
        
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
    
    def set_wifi_status(self, connected, ip_address=None):
        """
        Update WiFi connection status
        
        Args:
            connected: True if WiFi connected
            ip_address: IP address if connected
        """
        self.wifi_connected = connected
        self.wifi_ip = ip_address
        
        # Update display if we're on WiFi screen
        if self.current_screen == Screen.WIFI:
            self._render_current_screen()
    
    def update_status(self, status):
        """
        Update cached status from control thread
        
        Args:
            status: Status dictionary from kiln controller
        """
        self.cached_status = status
        
        # Update display if we're on a status screen
        if self.current_screen != Screen.STOP_CONFIRM:
            self._render_current_screen()
    
    def _render_current_screen(self):
        """Render the current screen"""
        if not self.enabled or not self.lcd:
            return
        
        try:
            if self.current_screen == Screen.WIFI:
                self._render_wifi_screen()
            elif self.current_screen == Screen.STATE:
                self._render_state_screen()
            elif self.current_screen == Screen.TEMP:
                self._render_temp_screen()
            elif self.current_screen == Screen.PROFILE:
                self._render_profile_screen()
            elif self.current_screen == Screen.STOP:
                self._render_stop_screen()
            elif self.current_screen == Screen.STOP_CONFIRM:
                self._render_stop_confirm_screen()
        except Exception as e:
            print(f"[LCD] Error rendering screen: {e}")
    
    def _render_wifi_screen(self):
        """Render WiFi status screen"""
        if self.wifi_connected and self.wifi_ip:
            self.lcd.print("WiFi: Connected", row=0)
            # Truncate IP if too long
            ip_text = self.wifi_ip[:16]
            self.lcd.print(ip_text, row=1)
        else:
            self.lcd.print("WiFi: Not Conn.", row=0)
            self.lcd.print("", row=1)
    
    def _render_state_screen(self):
        """Render current state screen"""
        state = self.cached_status.get('state', 'UNKNOWN')
        
        # Show state on first line
        self.lcd.print(f"State: {state[:9]}", row=0)
        
        # Show additional info on second line based on state
        if state == 'RUNNING':
            is_recovering = self.cached_status.get('is_recovering', False)
            if is_recovering:
                self.lcd.print("(Recovering)", row=1)
            else:
                step = self.cached_status.get('current_step', 0)
                total = self.cached_status.get('total_steps', 0)
                self.lcd.print(f"Step {step}/{total}", row=1)
        elif state == 'TUNING':
            self.lcd.print("Auto-tuning PID", row=1)
        elif state == 'ERROR':
            self.lcd.print("Check Web UI", row=1)
        elif state == 'COMPLETE':
            self.lcd.print("Profile Done!", row=1)
        else:
            self.lcd.print("", row=1)
    
    def _render_temp_screen(self):
        """Render temperature screen"""
        current = self.cached_status.get('current_temp', 0.0)
        target = self.cached_status.get('target_temp', 0.0)
        
        # Format temperatures to fit: "Cur: 999C"
        self.lcd.print(f"Cur: {current:.0f}C", row=0)
        
        if target > 0:
            self.lcd.print(f"Tgt: {target:.0f}C", row=1)
        else:
            self.lcd.print("Tgt: --", row=1)
    
    def _render_profile_screen(self):
        """Render profile/tuning method screen"""
        state = self.cached_status.get('state', 'IDLE')
        
        if state == 'RUNNING':
            profile_name = self.cached_status.get('profile', 'Unknown')
            # Truncate profile name if too long
            self.lcd.print("Profile:", row=0)
            self.lcd.print(profile_name[:16], row=1)
        elif state == 'TUNING':
            tuning_mode = self.cached_status.get('tuning_mode', 'Unknown')
            self.lcd.print("Tuning Mode:", row=0)
            self.lcd.print(tuning_mode[:16], row=1)
        else:
            self.lcd.print("Profile:", row=0)
            self.lcd.print("None", row=1)
    
    def _render_stop_screen(self):
        """Render stop menu screen"""
        state = self.cached_status.get('state', 'IDLE')
        
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
    
    def _handle_next_button(self):
        """Handle next button press"""
        # Move to next screen
        current_index = self.screen_order.index(self.current_screen)
        next_index = (current_index + 1) % len(self.screen_order)
        self.current_screen = self.screen_order[next_index]
        
        print(f"[LCD] Screen changed to: {self.current_screen}")
        self._render_current_screen()
    
    def _handle_select_button(self):
        """Handle select button press"""
        if self.current_screen == Screen.STOP:
            # Check if something is running
            state = self.cached_status.get('state', 'IDLE')
            if state in ['RUNNING', 'TUNING']:
                # Show confirmation screen
                self.current_screen = Screen.STOP_CONFIRM
                self._render_current_screen()
        
        elif self.current_screen == Screen.STOP_CONFIRM:
            # Confirmed - send stop command
            print("[LCD] Stop confirmed, sending stop command")
            from kiln.comms import CommandMessage, QueueHelper
            command = CommandMessage.stop()
            
            if QueueHelper.put_nowait(self.command_queue, command):
                print("[LCD] Stop command sent successfully")
                # Show feedback
                self.lcd.print("Stopping...", row=0)
                self.lcd.print("", row=1)
                # Return to state screen after short delay
                asyncio.create_task(self._return_to_state_screen_delayed())
            else:
                print("[LCD] Failed to send stop command (queue full)")
                self.lcd.print("Stop Failed", row=0)
                self.lcd.print("Queue full", row=1)
                asyncio.create_task(self._return_to_state_screen_delayed())
    
    async def _return_to_state_screen_delayed(self):
        """Return to state screen after 2 seconds"""
        await asyncio.sleep(2)
        self.current_screen = Screen.STATE
        self._render_current_screen()
    
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
        - Updates display periodically
        - Checks button presses
        """
        if not self.enabled:
            print("[LCD] LCD manager not enabled, exiting")
            return
        
        print("[LCD] Starting LCD update loop")
        
        # Show initial screen
        self._render_current_screen()
        
        # Main loop: update display and check buttons
        while True:
            try:
                # Check buttons
                self._check_buttons()
                
                # Update display (refresh current screen)
                # This ensures display stays in sync with status updates
                self._render_current_screen()
                
                # Run at ~10Hz for responsive button handling
                await asyncio.sleep(0.1)
                
            except Exception as e:
                print(f"[LCD] Error in update loop: {e}")
                await asyncio.sleep(1)  # Back off on errors


def get_lcd_manager():
    """Get singleton LCD manager instance"""
    global _lcd_manager_instance
    if '_lcd_manager_instance' not in globals():
        _lcd_manager_instance = None
    return _lcd_manager_instance


def initialize_lcd_manager(config, command_queue):
    """
    Initialize global LCD manager instance
    
    Args:
        config: Configuration object
        command_queue: Command queue for control thread
    
    Returns:
        LCDManager instance (or None if disabled)
    """
    global _lcd_manager_instance
    _lcd_manager_instance = LCDManager(config, command_queue)
    return _lcd_manager_instance
