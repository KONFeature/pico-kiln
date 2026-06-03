//! HD44780 1602 character LCD over a PCF8574 I²C backpack — a port of
//! `lib/lcd1602_i2c.py`.
//!
//! Generic over `embedded_hal::i2c::I2c`, so the same code drives the RP2350
//! blocking I²C (and could be exercised against a mock bus). The HD44780 timing
//! delays are blocking busy-waits (`embassy_time::block_for`): at the 100 kHz
//! default each one-byte transfer already takes ~0.2 ms, comfortably exceeding
//! the >450 ns enable-pulse and >37 µs execution minimums, so only the power-on
//! and clear delays need explicit waits.

use embassy_time::{block_for, Duration};
use embedded_hal::i2c::I2c;

// PCF8574 → HD44780 control bits (the `PINMAP_STANDARD` of the reference).
const RS: u8 = 0x01; // register select: 1 = data, 0 = command
const EN: u8 = 0x04; // enable strobe
const BACKLIGHT: u8 = 0x08; // backlight on

// HD44780 commands and flags.
const CMD_CLEAR: u8 = 0x01;
const CMD_ENTRY_MODE: u8 = 0x04;
const CMD_DISPLAY_CTRL: u8 = 0x08;
const CMD_FUNCTION_SET: u8 = 0x20;
const CMD_SET_DDRAM: u8 = 0x80;
const ENTRY_LEFT: u8 = 0x02;
const DISPLAY_ON: u8 = 0x04;
const TWO_LINE: u8 = 0x08;

/// A 16×2 character LCD on a PCF8574 backpack.
pub struct Lcd1602<I> {
    i2c: I,
    addr: u8,
}

impl<I: I2c> Lcd1602<I> {
    /// Wrap an I²C bus and the backpack address. Does no I/O; call [`init`] first.
    ///
    /// [`init`]: Lcd1602::init
    pub fn new(i2c: I, addr: u8) -> Self {
        Self { i2c, addr }
    }

    /// Run the HD44780 power-on init into 4-bit / 2-line / display-on mode. A
    /// NACK (no device on the bus) surfaces as `Err`, so the caller can disable
    /// the LCD. Safe to call again to re-init after a wire-interference glitch.
    pub fn init(&mut self) -> Result<(), I::Error> {
        block_for(Duration::from_millis(50)); // settle >40 ms after power-on
        self.write_raw(BACKLIGHT)?;
        block_for(Duration::from_millis(10));

        // 4-bit init handshake — works from either the 4- or 8-bit power-up state.
        self.write_nibble(0x30)?;
        block_for(Duration::from_millis(5));
        self.write_nibble(0x30)?;
        block_for(Duration::from_millis(5));
        self.write_nibble(0x30)?;
        block_for(Duration::from_millis(1));
        self.write_nibble(0x20)?; // switch to 4-bit
        block_for(Duration::from_millis(1));

        self.command(CMD_FUNCTION_SET | TWO_LINE)?; // 4-bit, 2-line, 5x8 dots
        self.command(CMD_DISPLAY_CTRL | DISPLAY_ON)?; // display on, cursor/blink off
        self.clear()?;
        self.command(CMD_ENTRY_MODE | ENTRY_LEFT)?; // left-to-right, no shift
        Ok(())
    }

    /// Clear the display (the HD44780 needs up to 1.52 ms; wait 5 ms).
    pub fn clear(&mut self) -> Result<(), I::Error> {
        self.command(CMD_CLEAR)?;
        block_for(Duration::from_millis(5));
        Ok(())
    }

    /// Write `text` left-aligned on `row` (0 or 1), truncated/space-padded to 16
    /// columns so it overwrites whatever was there (`LCD1602.print`).
    pub fn print_row(&mut self, row: u8, text: &str) -> Result<(), I::Error> {
        let base = if row == 0 { 0x00 } else { 0x40 };
        self.command(CMD_SET_DDRAM | base)?;
        let mut written = 0u8;
        for b in text.bytes().take(16) {
            self.data(b)?;
            written += 1;
        }
        for _ in written..16 {
            self.data(b' ')?;
        }
        Ok(())
    }

    fn command(&mut self, value: u8) -> Result<(), I::Error> {
        self.send(value, 0)
    }
    fn data(&mut self, value: u8) -> Result<(), I::Error> {
        self.send(value, RS)
    }

    // One byte as two 4-bit nibbles, high then low (4-bit interface).
    fn send(&mut self, value: u8, mode: u8) -> Result<(), I::Error> {
        self.write_nibble(mode | (value & 0xF0))?;
        self.write_nibble(mode | ((value << 4) & 0xF0))?;
        Ok(())
    }

    // Latch one nibble (already in the high bits) with an enable pulse.
    fn write_nibble(&mut self, data: u8) -> Result<(), I::Error> {
        let byte = data | BACKLIGHT;
        self.write_raw(byte & !EN)?;
        self.write_raw(byte | EN)?;
        self.write_raw(byte & !EN)?;
        Ok(())
    }

    fn write_raw(&mut self, byte: u8) -> Result<(), I::Error> {
        self.i2c.write(self.addr, &[byte])
    }
}
