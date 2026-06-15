# debug/debug_lcd.py
# LCD Debug Script - Tests LCD initialization with different configurations
#
# This script attempts to initialize the LCD with various settings to help
# diagnose LCD connection and encoding issues.
#
# Usage:
#   mpremote run debug/debug_lcd.py
#
# What it tests:
#   - I2C bus scanning
#   - Different I2C addresses (0x27, 0x3F)
#   - LCD initialization sequence
#   - Basic display operations (clear, print)
#   - Character encoding (ASCII, UTF-8)
#   - Different text lengths and special characters

import time
from machine import I2C, Pin
import sys

def log(message):
    """Print log message with timestamp"""
    timestamp = time.ticks_ms()
    print(f"[{timestamp:08d}] {message}")

def scan_i2c(i2c):
    """Scan I2C bus for devices"""
    log("Scanning I2C bus...")
    try:
        devices = i2c.scan()
        if devices:
            log(f"Found {len(devices)} I2C device(s):")
            for addr in devices:
                log(f"  - 0x{addr:02X}")
            return devices
        else:
            log("No I2C devices found!")
            return []
    except Exception as e:
        log(f"I2C scan failed: {e}")
        return []

def test_lcd_init(i2c, addr):
    """Test LCD initialization at specific address"""
    log(f"\n--- Testing LCD at address 0x{addr:02X} ---")

    try:
        # Import LCD driver
        from lib.lcd1602_i2c import LCD1602

        # Create LCD object
        log("Creating LCD object...")
        lcd = LCD1602(i2c, addr=addr)

        # Initialize LCD hardware (blocking version for debug)
        log("Initializing LCD hardware...")

        # Manual initialization with detailed logging
        log("  Step 1: Power-on delay (50ms)")
        time.sleep_ms(50)

        # Try to clear any garbage
        log("  Step 2: Clearing I2C line")
        try:
            i2c.writeto(addr, bytes([lcd.backlight]))
            time.sleep_ms(10)
            log("  I2C write successful")
        except Exception as e:
            log(f"  I2C write failed: {e}")
            raise

        # HD44780 initialization sequence
        log("  Step 3: HD44780 init sequence (4-bit mode)")
        lcd._write4bits(0x03 << 4)
        time.sleep_ms(5)
        lcd._write4bits(0x03 << 4)
        time.sleep_ms(5)
        lcd._write4bits(0x03 << 4)
        time.sleep_ms(1)
        lcd._write4bits(0x02 << 4)
        time.sleep_ms(1)
        log("  4-bit mode set")

        # Display initialization
        log("  Step 4: Display configuration")
        lcd._send_command(lcd.LCD_FUNCTIONSET | lcd.LCD_4BITMODE | lcd.LCD_2LINE | lcd.LCD_5x8DOTS)
        time.sleep_ms(1)
        lcd._send_command(lcd.LCD_DISPLAYCONTROL | lcd.LCD_DISPLAYON | lcd.LCD_CURSOROFF | lcd.LCD_BLINKOFF)
        time.sleep_ms(1)
        log("  Step 5: Clearing display")
        lcd.clear()
        lcd._send_command(lcd.LCD_ENTRYMODESET | lcd.LCD_ENTRYLEFT | lcd.LCD_ENTRYSHIFTDECREMENT)
        time.sleep_ms(2)

        log("LCD initialized successfully!")
        return lcd

    except Exception as e:
        log(f"LCD initialization failed: {e}")
        import sys
        sys.print_exception(e)
        return None

def test_lcd_operations(lcd):
    """Test various LCD operations"""
    if not lcd:
        log("No LCD to test (initialization failed)")
        return False

    log("\n--- Testing LCD Operations ---")

    try:
        # Test 1: Simple text
        log("Test 1: Simple ASCII text")
        lcd.clear()
        time.sleep_ms(100)
        lcd.print("Hello, World!", row=0)
        lcd.print("Test 1 OK", row=1)
        log("  Simple text displayed")
        time.sleep(2)

        # Test 2: Numbers
        log("Test 2: Numbers and symbols")
        lcd.clear()
        time.sleep_ms(100)
        lcd.print("Temp: 123.4C", row=0)
        lcd.print("SSR: 75% ON", row=1)
        log("  Numbers displayed")
        time.sleep(2)

        # Test 3: Full line (16 chars)
        log("Test 3: Full line (16 chars)")
        lcd.clear()
        time.sleep_ms(100)
        lcd.print("1234567890123456", row=0)
        lcd.print("ABCDEFGHIJKLMNOP", row=1)
        log("  Full line displayed")
        time.sleep(2)

        # Test 4: Special characters
        log("Test 4: Special characters")
        lcd.clear()
        time.sleep_ms(100)
        lcd.print("Chars: !@#$%^&*", row=0)
        lcd.print("()_+-=[]{}:;", row=1)
        log("  Special chars displayed")
        time.sleep(2)

        # Test 5: Degree symbol and common characters
        log("Test 5: Degree symbol (0xDF)")
        lcd.clear()
        time.sleep_ms(100)
        # Degree symbol is 0xDF in HD44780 character set
        lcd.set_cursor(0, 0)
        lcd.write_string("Temp: 25")
        lcd._send_data(0xDF)  # Degree symbol
        lcd.write_string("C")
        lcd.print("Encoding OK", row=1)
        log("  Degree symbol displayed")
        time.sleep(2)

        # Test 6: Backlight control
        log("Test 6: Backlight control")
        lcd.print("Backlight OFF", row=0)
        lcd.print("in 1 sec...", row=1)
        time.sleep(1)
        lcd.backlight_off()
        log("  Backlight OFF")
        time.sleep(2)
        lcd.backlight_on()
        log("  Backlight ON")
        lcd.clear()
        lcd.print("Backlight ON", row=0)
        time.sleep(2)

        # Test 7: Cursor positioning
        log("Test 7: Cursor positioning")
        lcd.clear()
        time.sleep_ms(100)
        lcd.set_cursor(0, 0)
        lcd.write_string("Row 0, Col 0")
        lcd.set_cursor(0, 1)
        lcd.write_string("Row 1, Col 0")
        log("  Cursor positioning OK")
        time.sleep(2)

        # Test 8: Rapid updates
        log("Test 8: Rapid updates (5 iterations)")
        for i in range(5):
            lcd.clear()
            time.sleep_ms(10)
            lcd.print(f"Count: {i+1}/5", row=0)
            lcd.print(f"Time: {time.ticks_ms()}", row=1)
            log(f"  Update {i+1}/5")
            time.sleep_ms(500)

        log("\n=== All LCD tests PASSED! ===")
        return True

    except Exception as e:
        log(f"LCD operation test failed: {e}")
        import sys
        sys.print_exception(e)
        return False

def test_encoding_variants(lcd):
    """Test different text encoding approaches"""
    if not lcd:
        return

    log("\n--- Testing Encoding Variants ---")

    test_strings = [
        ("ASCII only", "Hello World"),
        ("Numbers", "Temperature: 1234.5"),
        ("Special ASCII", "!@#$%^&*()_+-="),
        ("Degree (chr)", f"Temp: 25{chr(0xDF)}C"),  # Using chr()
        ("Degree (bytes)", None),  # Will handle separately
        ("UTF-8 Degree", "Temp: 25°C"),  # UTF-8 degree symbol
        ("UTF-8 Mixed", "25°C • 100% • ±5"),
    ]

    for name, text in test_strings:
        try:
            log(f"Testing: {name}")
            lcd.clear()
            time.sleep_ms(100)

            if text is None and "bytes" in name:
                # Test using direct byte writing
                lcd.set_cursor(0, 0)
                lcd.write_string("Temp: 25")
                lcd._send_data(0xDF)
                lcd.write_string("C")
                lcd.print("(via bytes)", row=1)
            elif text:
                # Convert UTF-8 to LCD character set if needed
                try:
                    # Try direct print first
                    lcd.print(text, row=0)
                    lcd.print(f"({name[:14]})", row=1)
                except Exception as e:
                    log(f"  Direct print failed: {e}")
                    # Try encoding as latin-1 (which maps to HD44780 for basic chars)
                    try:
                        encoded = text.encode('latin-1', errors='ignore').decode('latin-1')
                        lcd.print(encoded, row=0)
                        lcd.print(f"({name[:14]})", row=1)
                        log(f"  Latin-1 encoding worked")
                    except Exception as e2:
                        log(f"  Latin-1 encoding also failed: {e2}")

            log(f"  {name}: OK")
            time.sleep(2)

        except Exception as e:
            log(f"  {name}: FAILED - {e}")
            time.sleep(1)

def main():
    """Main debug routine"""

    print("=" * 50)
    log("LCD Debug Script Starting")
    print("=" * 50)

    try:
        # Configuration from config.py
        I2C_ID = 1
        SCL_PIN = 27
        SDA_PIN = 26
        I2C_FREQ = 100000
        COMMON_ADDRESSES = [0x27, 0x3F]  # Most common LCD I2C addresses

        log(f"Configuration:")
        log(f"  I2C Bus: {I2C_ID}")
        log(f"  SCL Pin: GP{SCL_PIN}")
        log(f"  SDA Pin: GP{SDA_PIN}")
        log(f"  Frequency: {I2C_FREQ} Hz")

        # Initialize I2C
        log("\nInitializing I2C bus...")
        i2c = I2C(I2C_ID, scl=Pin(SCL_PIN), sda=Pin(SDA_PIN), freq=I2C_FREQ)
        log("I2C initialized")

        # Scan I2C bus
        devices = scan_i2c(i2c)

        if not devices:
            log("\nERROR: No I2C devices found!")
            log("Check wiring:")
            log(f"  - SCL -> GP{SCL_PIN}")
            log(f"  - SDA -> GP{SDA_PIN}")
            log("  - VCC -> 5V or 3.3V")
            log("  - GND -> GND")
            return

        # Try each detected address
        lcd = None
        for addr in devices:
            if addr in COMMON_ADDRESSES:
                log(f"\nAttempting LCD init at detected address 0x{addr:02X}")
                lcd = test_lcd_init(i2c, addr)
                if lcd:
                    break

        # If no LCD found at detected addresses, try all common addresses
        if not lcd:
            log("\nLCD not initialized at detected addresses")
            log("Trying all common LCD addresses...")
            for addr in COMMON_ADDRESSES:
                if addr not in devices:
                    log(f"\nAttempting LCD init at 0x{addr:02X} (not detected in scan)")
                    lcd = test_lcd_init(i2c, addr)
                    if lcd:
                        break

        if not lcd:
            log("\nERROR: Failed to initialize LCD at any address")
            log("Possible issues:")
            log("  - Wrong I2C address (try 0x27 or 0x3F)")
            log("  - Faulty LCD backpack")
            log("  - Bad connection")
            log("  - LCD requires 5V (check power)")
            return

        # Run operation tests
        success = test_lcd_operations(lcd)

        if success:
            # Run encoding tests
            test_encoding_variants(lcd)

            # Final success message
            lcd.clear()
            lcd.print("Debug Complete!", row=0)
            print("\n" + "=" * 50)
            log("LCD Debug Complete - All tests passed!")
            print("=" * 50)
        else:
            log("\nSome tests failed - check output for details")

    except Exception as e:
        log(f"\nFATAL ERROR: {e}")
        import sys
        sys.print_exception(e)

    print("\nDebug script finished")

if __name__ == "__main__":
    main()
