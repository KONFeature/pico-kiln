# lib/lcd1602_i2c.py
# I2C LCD 1602 display driver for MicroPython
#
# This driver supports the common PCF8574-based I2C backpack for 1602 LCD displays

import time

class LCD1602:
    """
    Driver for 1602 LCD display with I2C backpack (PCF8574)
    
    Supports 16 columns x 2 rows display with I2C interface.
    """
    
    # Commands
    LCD_CLEARDISPLAY = 0x01
    LCD_RETURNHOME = 0x02
    LCD_ENTRYMODESET = 0x04
    LCD_DISPLAYCONTROL = 0x08
    LCD_CURSORSHIFT = 0x10
    LCD_FUNCTIONSET = 0x20
    LCD_SETCGRAMADDR = 0x40
    LCD_SETDDRAMADDR = 0x80
    
    # Flags for display entry mode
    LCD_ENTRYRIGHT = 0x00
    LCD_ENTRYLEFT = 0x02
    LCD_ENTRYSHIFTINCREMENT = 0x01
    LCD_ENTRYSHIFTDECREMENT = 0x00
    
    # Flags for display on/off control
    LCD_DISPLAYON = 0x04
    LCD_DISPLAYOFF = 0x00
    LCD_CURSORON = 0x02
    LCD_CURSOROFF = 0x00
    LCD_BLINKON = 0x01
    LCD_BLINKOFF = 0x00
    
    # Flags for function set
    LCD_8BITMODE = 0x10
    LCD_4BITMODE = 0x00
    LCD_2LINE = 0x08
    LCD_1LINE = 0x00
    LCD_5x10DOTS = 0x04
    LCD_5x8DOTS = 0x00
    
    # Flags for backlight control
    LCD_BACKLIGHT = 0x08
    LCD_NOBACKLIGHT = 0x00

    # Pin mapping presets for different PCF8574 modules
    # Format: (Rs, Rw, En, Backlight)
    PINMAP_STANDARD = {
        'Rs': 0b00000001,  # P0
        'Rw': 0b00000010,  # P1
        'En': 0b00000100,  # P2
        'BL': 0b00001000,  # P3
    }

    PINMAP_ALTERNATE = {
        'Rs': 0b00000001,  # P0
        'Rw': 0b00000010,  # P1
        'En': 0b00000100,  # P2
        'BL': 0b00001000,  # P3 (inverted logic)
        'BL_INVERTED': True
    }

    def __init__(self, i2c, addr=0x27, cols=16, rows=2, pinmap=None):
        """
        Initialize LCD display

        Args:
            i2c: MicroPython I2C object
            addr: I2C address (default 0x27, some use 0x3F)
            cols: Number of columns (default 16)
            rows: Number of rows (default 2)
            pinmap: Pin mapping dict (default: PINMAP_STANDARD)
        """
        self.i2c = i2c
        self.addr = addr
        self.cols = cols
        self.rows = rows

        # Set up pin mapping
        if pinmap is None:
            pinmap = self.PINMAP_STANDARD

        self.Rs = pinmap['Rs']
        self.Rw = pinmap['Rw']
        self.En = pinmap['En']
        self.backlight_bit = pinmap['BL']
        self.backlight_inverted = pinmap.get('BL_INVERTED', False)

        # Set backlight state (on by default)
        if self.backlight_inverted:
            self.backlight = 0x00  # Active low
        else:
            self.backlight = self.backlight_bit  # Active high

        # Initialize display - wait for power-on
        time.sleep_ms(50)  # Wait >40ms after power on

        # Try to ensure we're in a known state - send 0x00 to clear any garbage
        try:
            self.i2c.writeto(self.addr, bytes([self.backlight]))
            time.sleep_ms(10)
        except:
            pass

        # Put LCD into 4-bit mode (HD44780 initialization sequence)
        # This sequence works regardless of whether LCD is in 4-bit or 8-bit mode
        self._write4bits(0x03 << 4)
        time.sleep_ms(5)  # Wait >4.1ms
        self._write4bits(0x03 << 4)
        time.sleep_ms(5)  # Wait >100us (using 5ms to be safe)
        self._write4bits(0x03 << 4)
        time.sleep_ms(1)
        self._write4bits(0x02 << 4)  # Switch to 4-bit mode
        time.sleep_ms(1)

        # Display initialization with proper delays
        self._send_command(self.LCD_FUNCTIONSET | self.LCD_4BITMODE | self.LCD_2LINE | self.LCD_5x8DOTS)
        time.sleep_ms(1)
        self._send_command(self.LCD_DISPLAYCONTROL | self.LCD_DISPLAYON | self.LCD_CURSOROFF | self.LCD_BLINKOFF)
        time.sleep_ms(1)
        self.clear()
        self._send_command(self.LCD_ENTRYMODESET | self.LCD_ENTRYLEFT | self.LCD_ENTRYSHIFTDECREMENT)
        time.sleep_ms(2)
    
    def _write4bits(self, data):
        """
        Write 4 bits to I2C with proper enable pulse

        The HD44780 requires:
        1. Data setup (with E=0)
        2. E high pulse (min 450ns)
        3. E low (min 500ns total cycle)
        """
        try:
            # Ensure data is in upper nibble (bits 4-7)
            # Lower bits contain control signals
            byte_data = data | self.backlight

            # Step 1: Set data with E=0 (data setup)
            self.i2c.writeto(self.addr, bytes([byte_data & ~self.En]))
            time.sleep_us(1)  # Data setup time (>60ns, use 1us)

            # Step 2: Set E=1 (latch data)
            self.i2c.writeto(self.addr, bytes([byte_data | self.En]))
            time.sleep_us(1)  # Enable pulse width (>450ns, use 1us)

            # Step 3: Set E=0 (complete cycle)
            self.i2c.writeto(self.addr, bytes([byte_data & ~self.En]))
            time.sleep_us(50)  # Command execution time (>37us, use 50us)

        except OSError:
            pass  # Silently fail if I2C error
    
    def _send_command(self, cmd):
        """Send command to LCD"""
        self._send_byte(cmd, 0)  # Rs=0 for commands

    def _send_data(self, data):
        """Send data to LCD"""
        self._send_byte(data, self.Rs)  # Rs=1 for data
    
    def _send_byte(self, data, mode):
        """Send byte to LCD in 4-bit mode"""
        high_bits = mode | (data & 0xF0)
        low_bits = mode | ((data << 4) & 0xF0)
        self._write4bits(high_bits)
        self._write4bits(low_bits)
    
    def clear(self):
        """Clear display"""
        self._send_command(self.LCD_CLEARDISPLAY)
        time.sleep_ms(5)  # Clear needs up to 1.52ms, use 5ms to be safe

    def home(self):
        """Return cursor to home position"""
        self._send_command(self.LCD_RETURNHOME)
        time.sleep_ms(5)  # Home needs up to 1.52ms, use 5ms to be safe
    
    def set_cursor(self, col, row):
        """
        Set cursor position
        
        Args:
            col: Column (0-15)
            row: Row (0-1)
        """
        row_offsets = [0x00, 0x40, 0x14, 0x54]
        if row >= self.rows:
            row = self.rows - 1
        self._send_command(self.LCD_SETDDRAMADDR | (col + row_offsets[row]))
    
    def write_string(self, text):
        """
        Write string to display at current cursor position
        
        Args:
            text: String to display
        """
        for char in text:
            self._send_data(ord(char))
    
    def print(self, text, row=0):
        """
        Print text on specified row (centered or left-aligned)

        Args:
            text: Text to display
            row: Row number (0 or 1)
        """
        # Convert to string if needed (e.g., if int or float passed)
        text = str(text)
        # Truncate if too long
        text = text[:self.cols]
        # Pad with spaces to clear previous content (MicroPython-compatible)
        text = text + ' ' * (self.cols - len(text))
        self.set_cursor(0, row)
        self.write_string(text)
    
    def backlight_on(self):
        """Turn backlight on"""
        if self.backlight_inverted:
            self.backlight = 0x00  # Active low
        else:
            self.backlight = self.backlight_bit  # Active high
        try:
            self.i2c.writeto(self.addr, bytes([self.backlight]))
        except OSError:
            pass

    def backlight_off(self):
        """Turn backlight off"""
        if self.backlight_inverted:
            self.backlight = self.backlight_bit  # Active low (set bit to turn off)
        else:
            self.backlight = 0x00  # Active high (clear bit to turn off)
        try:
            self.i2c.writeto(self.addr, bytes([self.backlight]))
        except OSError:
            pass
    
    def display_on(self):
        """Turn display on"""
        self._send_command(self.LCD_DISPLAYCONTROL | self.LCD_DISPLAYON | self.LCD_CURSOROFF | self.LCD_BLINKOFF)
    
    def display_off(self):
        """Turn display off"""
        self._send_command(self.LCD_DISPLAYCONTROL | self.LCD_DISPLAYOFF)
