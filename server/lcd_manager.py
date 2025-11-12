# server/lcd_manager.py
# LCD Display Manager for 1602 I2C LCD
#
# SIMPLIFIED: Display-only mode for production kilns
# - No buttons, no navigation, no complexity
# - Just shows: Current temp, target temp, SSR output, state
# - Updates every 2 seconds
# - Rock-solid for 15+ hour firings

import asyncio
from machine import I2C, Pin


class LCDManager:
    """
    Simplified LCD display manager - display only, no buttons
    
    Shows essential kiln info on a 16x2 LCD display:
    - Row 1: Current temp + state
    - Row 2: Target temp + SSR output
    
    Updates every 2 seconds. That's it.
    """

    def __init__(self, config, status_receiver):
        """
        Initialize LCD manager
        
        Args:
            config: Configuration object with LCD I2C settings
            status_receiver: StatusReceiver instance for reading cached status
        """
        self.config = config
        self.status_receiver = status_receiver
        
        # Check if LCD is enabled in config
        self.enabled = hasattr(config, 'LCD_I2C_SCL') and hasattr(config, 'LCD_I2C_SDA')
        
        if not self.enabled:
            print("[LCD] LCD not configured, display disabled")
            self.lcd = None
            return
        
        # Hardware component (initialized separately via initialize_hardware)
        self.lcd = None

    async def initialize_hardware(self, timeout_ms=500):
        """
        Initialize LCD hardware (async, non-blocking)
        
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
            devices = i2c.scan()
            if self.config.LCD_I2C_ADDR not in devices:
                print(f"[LCD] WARNING: Device not found at 0x{self.config.LCD_I2C_ADDR:02x}")
                print(f"[LCD] Found devices: {[hex(d) for d in devices]}")
                raise Exception(f"LCD not found on I2C bus at 0x{self.config.LCD_I2C_ADDR:02x}")
            
            print(f"[LCD] Device detected at 0x{self.config.LCD_I2C_ADDR:02x}")
            
            # Create LCD object
            from lib.lcd1602_i2c import LCD1602
            self.lcd = LCD1602(i2c, addr=self.config.LCD_I2C_ADDR)
            
            # Initialize LCD hardware with timeout
            await asyncio.wait_for(
                self.lcd.initialize(),
                timeout_ms / 1000.0  # Convert to seconds
            )
            
            print(f"[LCD] Display initialized successfully")
            return True
            
        except asyncio.TimeoutError:
            print(f"[LCD] CRITICAL: Hardware initialization TIMED OUT after {timeout_ms}ms")
            print("[LCD] Display disabled - system will continue without LCD")
            self.lcd = None
            self.enabled = False
            return False
            
        except Exception as e:
            print(f"[LCD] CRITICAL: Failed to initialize LCD hardware: {e}")
            print("[LCD] Display disabled - system will continue without LCD")
            self.lcd = None
            self.enabled = False
            return False

    async def run(self):
        """
        Main LCD update loop - display only
        
        Shows essential kiln information:
        - Row 1: Current temperature + state (e.g., "  25C IDLE")
        - Row 2: Target temperature + SSR output (e.g., "Tgt:800C  45%")
        
        Updates every 2 seconds for a good balance between responsiveness
        and reducing I2C bus traffic.
        """
        if not self.enabled:
            print("[LCD] LCD manager not enabled, exiting")
            return
        
        # Wait for LCD hardware to be initialized
        while not self.lcd:
            await asyncio.sleep(0.1)
        
        print("[LCD] Starting LCD update loop (display-only mode)")
        print("[LCD] Showing: Temp, State, Target, SSR")
        
        consecutive_errors = 0
        max_consecutive_errors = 3
        
        while True:
            try:
                # Check if LCD is still available (can be disabled during init retries)
                if not self.lcd or not self.enabled:
                    print("[LCD] LCD disabled, exiting update loop")
                    return
                
                # Get current status from cache
                state = self.status_receiver.get_status_field('state', 'IDLE')
                current_temp = self.status_receiver.get_status_field('current_temp', 0.0)
                target_temp = self.status_receiver.get_status_field('target_temp', 0.0)
                ssr_output = self.status_receiver.get_status_field('ssr_output', 0.0)
                
                # Row 1: Current temp + state
                # Format: "123C RUNNING" or "  25C IDLE"
                row1 = f"{current_temp:4.0f}C {state[:10]}"
                self.lcd.print(row1, row=0)
                
                # Small delay between row updates for reliability
                await asyncio.sleep(0.01)
                
                # Row 2: Target temp + SSR output
                # Format: "Tgt:800C  45%" or "SSR:   0%" (when no target)
                if target_temp > 0:
                    row2 = f"Tgt:{target_temp:4.0f}C {ssr_output:3.0f}%"
                else:
                    row2 = f"SSR: {ssr_output:3.0f}%"
                self.lcd.print(row2, row=1)
                
                # Reset error counter on successful update
                consecutive_errors = 0
                
                # Update every 2 seconds
                await asyncio.sleep(2.0)
                
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


# Singleton instance
_lcd_manager = None


def get_lcd_manager():
    """Get singleton LCD manager instance"""
    return _lcd_manager


def initialize_lcd_manager(config, status_receiver):
    """
    Initialize global LCD manager instance
    
    Args:
        config: Configuration object with LCD I2C settings
        status_receiver: StatusReceiver instance for reading cached status
    
    Returns:
        LCDManager instance
    """
    global _lcd_manager
    _lcd_manager = LCDManager(config, status_receiver)
    return _lcd_manager
