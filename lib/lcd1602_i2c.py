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
    
    # Enable bit
    En = 0b00000100  # Enable bit
    Rw = 0b00000010  # Read/Write bit
    Rs = 0b00000001  # Register select bit
    
    def __init__(self, i2c, addr=0x27, cols=16, rows=2):
        """
        Initialize LCD display
        
        Args:
            i2c: MicroPython I2C object
            addr: I2C address (default 0x27, some use 0x3F)
            cols: Number of columns (default 16)
            rows: Number of rows (default 2)
        """
        self.i2c = i2c
        self.addr = addr
        self.cols = cols
        self.rows = rows
        self.backlight = self.LCD_BACKLIGHT
        
        # Initialize display
        time.sleep(0.05)  # Wait for LCD to power up
        
        # Put LCD into 4-bit mode
        self._write4bits(0x03 << 4)
        time.sleep(0.005)
        self._write4bits(0x03 << 4)
        time.sleep(0.005)
        self._write4bits(0x03 << 4)
        time.sleep(0.001)
        self._write4bits(0x02 << 4)
        
        # Display initialization
        self._send_command(self.LCD_FUNCTIONSET | self.LCD_4BITMODE | self.LCD_2LINE | self.LCD_5x8DOTS)
        self._send_command(self.LCD_DISPLAYCONTROL | self.LCD_DISPLAYON | self.LCD_CURSOROFF | self.LCD_BLINKOFF)
        self.clear()
        self._send_command(self.LCD_ENTRYMODESET | self.LCD_ENTRYLEFT | self.LCD_ENTRYSHIFTDECREMENT)
        time.sleep(0.002)
    
    def _write4bits(self, data):
        """Write 4 bits to I2C"""
        try:
            self.i2c.writeto(self.addr, bytes([data | self.backlight]))
            self._pulse_enable(data)
        except OSError:
            pass  # Silently fail if I2C error
    
    def _pulse_enable(self, data):
        """Pulse the enable bit"""
        try:
            self.i2c.writeto(self.addr, bytes([data | self.En | self.backlight]))
            time.sleep(0.0001)
            self.i2c.writeto(self.addr, bytes([(data & ~self.En) | self.backlight]))
            time.sleep(0.0001)
        except OSError:
            pass
    
    def _send_command(self, cmd):
        """Send command to LCD"""
        self._send_byte(cmd, self.Rs)
    
    def _send_data(self, data):
        """Send data to LCD"""
        self._send_byte(data, self.Rs | 0x01)
    
    def _send_byte(self, data, mode):
        """Send byte to LCD in 4-bit mode"""
        high_bits = mode | (data & 0xF0)
        low_bits = mode | ((data << 4) & 0xF0)
        self._write4bits(high_bits)
        self._write4bits(low_bits)
    
    def clear(self):
        """Clear display"""
        self._send_command(self.LCD_CLEARDISPLAY)
        time.sleep(0.002)
    
    def home(self):
        """Return cursor to home position"""
        self._send_command(self.LCD_RETURNHOME)
        time.sleep(0.002)
    
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
        # Truncate if too long
        text = text[:self.cols]
        # Pad with spaces to clear previous content
        text = text.ljust(self.cols)
        self.set_cursor(0, row)
        self.write_string(text)
    
    def backlight_on(self):
        """Turn backlight on"""
        self.backlight = self.LCD_BACKLIGHT
        try:
            self.i2c.writeto(self.addr, bytes([self.backlight]))
        except OSError:
            pass
    
    def backlight_off(self):
        """Turn backlight off"""
        self.backlight = self.LCD_NOBACKLIGHT
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
