//! MAX31856 universal thermocouple amplifier driver.
//!
//! Faithful port of the Adafruit CircuitPython MAX31856 driver that
//! `kiln/hardware.py` builds on: same register map, same init (assert on all
//! faults, open-circuit detection), and the same 19-bit linearised-temperature
//! unpack (`LSB = 2^-7 °C`). Generic over `embedded_hal::spi::SpiDevice`, so it
//! runs on the RP2350 and against a mock bus under `cargo test`.
//!
//! Matched to how the controller runs the chip after the filtering rework: set
//! the mains notch and hardware averaging, then [`Max31856::start_autoconverting`]
//! so the chip free-runs its SINC + notch + AVGSEL filter. Reads are then
//! non-blocking register fetches of the latest result (no one-shot, no ~160 ms
//! busy-wait); the registers read `0.0` until the first conversion settles.
//!
//! The median spike-rejection, fault tolerance, and range checks that wrap this
//! sensor in `hardware.py` are *not* here — that is pure decision logic and lives
//! in `kiln-core` (`temp_filter`). This driver returns raw readings and faults.

use embedded_hal::spi::{Operation, SpiDevice};

const REG_CR0: u8 = 0x00;
const REG_CR1: u8 = 0x01;
const REG_MASK: u8 = 0x02;
const REG_LTCBH: u8 = 0x0C;
const REG_SR: u8 = 0x0F;

const CR0_AUTOCONVERT: u8 = 0x80;
const CR0_ONESHOT: u8 = 0x40;
const CR0_OCFAULT0: u8 = 0x10;
const CR0_50HZ: u8 = 0x01;

/// °C per LSB of the 19-bit linearised thermocouple register (`2^-7`).
const THERM_LSB: f32 = 0.007_812_5;

/// Thermocouple type, written to the low nibble of config register CR1. Values
/// match the MAX31856 datasheet / Adafruit `ThermocoupleType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThermocoupleType {
    B,
    E,
    J,
    #[default]
    K,
    N,
    R,
    S,
    T,
    /// Voltage mode, gain 8.
    G8,
    /// Voltage mode, gain 32.
    G32,
}

impl ThermocoupleType {
    fn bits(self) -> u8 {
        match self {
            Self::B => 0b0000,
            Self::E => 0b0001,
            Self::J => 0b0010,
            Self::K => 0b0011,
            Self::N => 0b0100,
            Self::R => 0b0101,
            Self::S => 0b0110,
            Self::T => 0b0111,
            Self::G8 => 0b1000,
            Self::G32 => 0b1100,
        }
    }
}

/// Samples averaged per conversion (CR1 bits 4-6). More averaging trades
/// conversion time for noise rejection. `Default` is the kiln's
/// `THERMOCOUPLE_AVERAGING` default of 8 samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Averaging {
    S1,
    S2,
    S4,
    #[default]
    S8,
    S16,
}

impl Averaging {
    /// Map a `THERMOCOUPLE_AVERAGING` config value (1, 2, 4, 8, 16) to its AVGSEL
    /// setting; `None` for any other count. Mirrors `VALID_AVERAGING` in
    /// `hardware.py` — pair with `.unwrap_or_default()` for the same fallback to 8.
    pub fn from_samples(samples: u8) -> Option<Self> {
        match samples {
            1 => Some(Self::S1),
            2 => Some(Self::S2),
            4 => Some(Self::S4),
            8 => Some(Self::S8),
            16 => Some(Self::S16),
            _ => None,
        }
    }

    fn bits(self) -> u8 {
        match self {
            Self::S1 => 0x00,
            Self::S2 => 0x10,
            Self::S4 => 0x20,
            Self::S8 => 0x30,
            Self::S16 => 0x40,
        }
    }
}

/// Mains-frequency notch filter for the ADC (CR0 bit 0). Pick the local mains
/// frequency to reject its hum; must be set before conversions. `Default` is the
/// kiln's `MAINS_FREQUENCY` default of 60 Hz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NoiseFilter {
    Hz50,
    #[default]
    Hz60,
}

impl NoiseFilter {
    /// Map a `MAINS_FREQUENCY` config value (50 or 60) to its notch setting;
    /// `None` otherwise. Mirrors the check in `hardware.py` — pair with
    /// `.unwrap_or_default()` for the same fallback to 60 Hz.
    pub fn from_hz(hz: u16) -> Option<Self> {
        match hz {
            50 => Some(Self::Hz50),
            60 => Some(Self::Hz60),
            _ => None,
        }
    }
}

/// Decoded fault status register (0x0F). Mirrors the booleans Adafruit's
/// `fault` dict exposes, which `hardware.py` reduces with `any(...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Faults {
    pub cj_range: bool,
    pub tc_range: bool,
    pub cj_high: bool,
    pub cj_low: bool,
    pub tc_high: bool,
    pub tc_low: bool,
    /// Over-voltage / under-voltage on an input.
    pub voltage: bool,
    /// Thermocouple open circuit (disconnected probe).
    pub open_tc: bool,
}

impl Faults {
    fn from_sr(sr: u8) -> Self {
        Self {
            cj_range: sr & 0x80 != 0,
            tc_range: sr & 0x40 != 0,
            cj_high: sr & 0x20 != 0,
            cj_low: sr & 0x10 != 0,
            tc_high: sr & 0x08 != 0,
            tc_low: sr & 0x04 != 0,
            voltage: sr & 0x02 != 0,
            open_tc: sr & 0x01 != 0,
        }
    }

    /// True if any fault bit is set (the `hardware.py` shutdown trigger).
    pub fn any(&self) -> bool {
        self.cj_range
            || self.tc_range
            || self.cj_high
            || self.cj_low
            || self.tc_high
            || self.tc_low
            || self.voltage
            || self.open_tc
    }
}

/// Driver error: the only failure mode is the underlying SPI bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error<E> {
    Spi(E),
}

/// MAX31856 driver bound to an SPI device (the device owns chip-select).
pub struct Max31856<SPI> {
    spi: SPI,
}

impl<SPI: SpiDevice> Max31856<SPI> {
    /// Wrap an SPI device. Does no I/O; call [`Max31856::init`] before reading.
    pub fn new(spi: SPI) -> Self {
        Self { spi }
    }

    /// Configure the amplifier: assert on all faults, enable open-circuit
    /// detection, and select the thermocouple type. Mirrors Adafruit's
    /// `MAX31856.__init__`.
    pub fn init(&mut self, tc: ThermocoupleType) -> Result<(), Error<SPI::Error>> {
        self.write_register(REG_MASK, 0x00)?;
        self.write_register(REG_CR0, CR0_OCFAULT0)?;
        self.set_thermocouple_type(tc)
    }

    /// Set the thermocouple type, preserving the other CR1 bits (averaging).
    pub fn set_thermocouple_type(
        &mut self,
        tc: ThermocoupleType,
    ) -> Result<(), Error<SPI::Error>> {
        let cr1 = self.read_register_u8(REG_CR1)?;
        self.write_register(REG_CR1, (cr1 & 0xF0) | tc.bits())
    }

    /// Set the per-conversion averaging, preserving the thermocouple type.
    pub fn set_averaging(&mut self, avg: Averaging) -> Result<(), Error<SPI::Error>> {
        let cr1 = self.read_register_u8(REG_CR1)?;
        self.write_register(REG_CR1, (cr1 & 0b1000_1111) | avg.bits())
    }

    /// Select the mains-frequency notch filter.
    pub fn set_noise_filter(&mut self, filter: NoiseFilter) -> Result<(), Error<SPI::Error>> {
        let cr0 = self.read_register_u8(REG_CR0)?;
        let cr0 = match filter {
            NoiseFilter::Hz50 => cr0 | CR0_50HZ,
            NoiseFilter::Hz60 => cr0 & !CR0_50HZ,
        };
        self.write_register(REG_CR0, cr0)
    }

    /// Put the chip into continuous (auto) conversion: it free-runs a conversion
    /// every ~100 ms applying its SINC + notch + AVGSEL filter. Configure the
    /// noise filter and averaging *before* calling this. Afterwards the registers
    /// read `0.0` until the first conversion settles, then
    /// [`Max31856::read_temperature`] is a non-blocking fetch of the latest value.
    pub fn start_autoconverting(&mut self) -> Result<(), Error<SPI::Error>> {
        let cr0 = self.read_register_u8(REG_CR0)?;
        self.write_register(REG_CR0, (cr0 & !CR0_ONESHOT) | CR0_AUTOCONVERT)
    }

    /// Read and decode the linearised thermocouple temperature in °C from the
    /// most recent conversion. 19-bit two's-complement, `LSB = 2^-7 °C`.
    pub fn read_temperature(&mut self) -> Result<f32, Error<SPI::Error>> {
        let mut buf = [0u8; 3];
        self.read_register(REG_LTCBH, &mut buf)?;

        let combined =
            ((buf[0] as u32) << 11) | ((buf[1] as u32) << 3) | ((buf[2] as u32) >> 5);
        let mut value = combined as i32;
        if value & 0x0004_0000 != 0 {
            value -= 0x0008_0000; // sign-extend the 19-bit field
        }
        Ok(value as f32 * THERM_LSB)
    }

    /// Read the decoded fault status register.
    pub fn faults(&mut self) -> Result<Faults, Error<SPI::Error>> {
        Ok(Faults::from_sr(self.read_register_u8(REG_SR)?))
    }

    fn read_register(&mut self, reg: u8, buf: &mut [u8]) -> Result<(), Error<SPI::Error>> {
        self.spi
            .transaction(&mut [Operation::Write(&[reg & 0x7F]), Operation::Read(buf)])
            .map_err(Error::Spi)
    }

    fn read_register_u8(&mut self, reg: u8) -> Result<u8, Error<SPI::Error>> {
        let mut b = [0u8; 1];
        self.read_register(reg, &mut b)?;
        Ok(b[0])
    }

    fn write_register(&mut self, reg: u8, val: u8) -> Result<(), Error<SPI::Error>> {
        self.spi.write(&[reg | 0x80, val]).map_err(Error::Spi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_hal_mock::eh1::spi::{Mock as SpiMock, Transaction as Spi};

    fn read_reg(reg: u8, ret: &[u8]) -> [Spi<u8>; 4] {
        [
            Spi::transaction_start(),
            Spi::write_vec(vec![reg & 0x7F]),
            Spi::read_vec(ret.to_vec()),
            Spi::transaction_end(),
        ]
    }

    fn write_reg(reg: u8, val: u8) -> [Spi<u8>; 3] {
        [
            Spi::transaction_start(),
            Spi::write_vec(vec![reg | 0x80, val]),
            Spi::transaction_end(),
        ]
    }

    #[test]
    fn decodes_positive_temperature() {
        // 25.0 °C -> combined 3200 -> register 0x019000.
        let expect = read_reg(REG_LTCBH, &[0x01, 0x90, 0x00]);
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());

        assert_eq!(dev.read_temperature().unwrap(), 25.0);
        spi.done();
    }

    #[test]
    fn decodes_negative_temperature() {
        // -1.0 °C -> combined -128 (0x7FF80) -> register 0xFFF000.
        let expect = read_reg(REG_LTCBH, &[0xFF, 0xF0, 0x00]);
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());

        assert_eq!(dev.read_temperature().unwrap(), -1.0);
        spi.done();
    }

    #[test]
    fn init_writes_mask_cr0_and_type() {
        let mut expect = Vec::new();
        expect.extend(write_reg(REG_MASK, 0x00));
        expect.extend(write_reg(REG_CR0, CR0_OCFAULT0));
        expect.extend(read_reg(REG_CR1, &[0x00])); // current CR1
        expect.extend(write_reg(REG_CR1, 0x03)); // K-type into low nibble
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());

        dev.init(ThermocoupleType::K).unwrap();
        spi.done();
    }

    #[test]
    fn set_type_preserves_high_nibble() {
        let mut expect = Vec::new();
        expect.extend(read_reg(REG_CR1, &[0x40])); // averaging bits set
        expect.extend(write_reg(REG_CR1, 0x46)); // keep 0x40, set S-type (0x06)
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());

        dev.set_thermocouple_type(ThermocoupleType::S).unwrap();
        spi.done();
    }

    #[test]
    fn config_values_validate_and_fall_back_to_kiln_defaults() {
        assert_eq!(Averaging::from_samples(1), Some(Averaging::S1));
        assert_eq!(Averaging::from_samples(8), Some(Averaging::S8));
        assert_eq!(Averaging::from_samples(16), Some(Averaging::S16));
        assert_eq!(Averaging::from_samples(3), None);
        assert_eq!(Averaging::from_samples(3).unwrap_or_default(), Averaging::S8);

        assert_eq!(NoiseFilter::from_hz(50), Some(NoiseFilter::Hz50));
        assert_eq!(NoiseFilter::from_hz(60), Some(NoiseFilter::Hz60));
        assert_eq!(NoiseFilter::from_hz(55), None);
        assert_eq!(NoiseFilter::from_hz(55).unwrap_or_default(), NoiseFilter::Hz60);
    }

    #[test]
    fn set_averaging_writes_avgsel_preserving_type() {
        let mut expect = Vec::new();
        expect.extend(read_reg(REG_CR1, &[0x03])); // K-type already set
        expect.extend(write_reg(REG_CR1, 0x33)); // keep 0x03, AVGSEL=8 -> 0x30
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());

        dev.set_averaging(Averaging::S8).unwrap();
        spi.done();
    }

    #[test]
    fn set_noise_filter_writes_mains_bit() {
        let mut expect = Vec::new();
        expect.extend(read_reg(REG_CR0, &[CR0_OCFAULT0]));
        expect.extend(write_reg(REG_CR0, CR0_OCFAULT0 | CR0_50HZ)); // 50 Hz sets bit 0
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());

        dev.set_noise_filter(NoiseFilter::Hz50).unwrap();
        spi.done();
    }

    #[test]
    fn faults_decode_and_reduce() {
        let expect = read_reg(REG_SR, &[0x01]); // open circuit
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());

        let f = dev.faults().unwrap();
        assert!(f.open_tc);
        assert!(f.any());
        spi.done();

        let expect = read_reg(REG_SR, &[0x00]);
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());
        assert!(!dev.faults().unwrap().any());
        spi.done();
    }

    #[test]
    fn start_autoconverting_sets_cmode_and_clears_oneshot() {
        let mut expect = Vec::new();
        expect.extend(read_reg(REG_CR0, &[CR0_OCFAULT0 | CR0_ONESHOT]));
        expect.extend(write_reg(REG_CR0, CR0_OCFAULT0 | CR0_AUTOCONVERT));
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());

        dev.start_autoconverting().unwrap();
        spi.done();
    }

    #[test]
    fn registers_read_zero_before_first_conversion() {
        // In auto mode the LTCB registers read 0 until the first conversion
        // settles; the driver returns 0.0 and the caller (init) polls for nonzero.
        let expect = read_reg(REG_LTCBH, &[0x00, 0x00, 0x00]);
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());

        assert_eq!(dev.read_temperature().unwrap(), 0.0);
        spi.done();
    }
}
