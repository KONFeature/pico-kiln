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
//!
//! **Liveness (Rust-only hardening).** The chip's own fault register cannot see
//! the two failures that make it lie *plausibly*: a dead/unpowered chip or a
//! broken MISO line (the RP2350 pad pull-down makes every register read `0x00`,
//! so SR says "no fault" and LTC decodes to a valid-looking 0.0 °C), and a
//! brown-out reset (CR0 returns to one-shot mode, so the LTC registers freeze or
//! read 0). A loop that trusts those readings heats at full power forever. When
//! configured through [`Max31856::configure`], the driver remembers the register
//! values it wrote and, on every [`TempSensor::has_fault`](crate::TempSensor),
//! reads them back: a mismatch is reported as a fault and the chip is
//! re-configured, and [`TempSensor::read_temperature`](crate::TempSensor) refuses
//! the all-zero "no conversion yet" register until a real conversion lands.

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

/// Read-back comparison masks: only bits that hold their written value. CR0 bit 6
/// (1SHOT) and bit 1 (FAULTCLR) self-clear; CR1 bit 7 and MASK bits 7:6 are
/// reserved, so their read value is not specified by the datasheet.
const CR0_CHECK: u8 = 0b1011_1101;
const CR1_CHECK: u8 = 0b0111_1111;
const MASK_CHECK: u8 = 0b0011_1111;

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

/// Driver error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error<E> {
    /// The underlying SPI bus failed.
    Spi(E),
    /// The configuration registers do not read back what [`Max31856::configure`]
    /// wrote: a dead/unpowered chip, a broken MISO line, or a chip reset.
    ConfigMismatch,
    /// No conversion has completed since (re)configuration: the linearised
    /// temperature registers still hold their all-zero reset value.
    NotReady,
}

/// The full register configuration [`Max31856::configure`] applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SensorConfig {
    pub thermocouple: ThermocoupleType,
    pub averaging: Averaging,
    pub noise_filter: NoiseFilter,
}

/// The register values written by [`Max31856::configure`], kept for read-back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Expected {
    cr0: u8,
    cr1: u8,
    mask: u8,
}

impl Expected {
    fn from_config(cfg: SensorConfig) -> Self {
        let notch = match cfg.noise_filter {
            NoiseFilter::Hz50 => CR0_50HZ,
            NoiseFilter::Hz60 => 0,
        };
        Expected {
            // Continuous conversion + open-circuit detection + mains notch.
            cr0: CR0_AUTOCONVERT | CR0_OCFAULT0 | notch,
            cr1: cfg.averaging.bits() | cfg.thermocouple.bits(),
            // Assert the FAULT pin on every fault (Adafruit's `MASK = 0`).
            mask: 0x00,
        }
    }

    /// True when a `[CR0, CR1, MASK]` read-back matches on every defined bit.
    fn matches(&self, regs: [u8; 3]) -> bool {
        regs[0] & CR0_CHECK == self.cr0 & CR0_CHECK
            && regs[1] & CR1_CHECK == self.cr1 & CR1_CHECK
            && regs[2] & MASK_CHECK == self.mask & MASK_CHECK
    }
}

/// MAX31856 driver bound to an SPI device (the device owns chip-select).
pub struct Max31856<SPI> {
    spi: SPI,
    /// Set by [`configure`](Self::configure): the registers to verify each read.
    expected: Option<Expected>,
    /// True from (re)configuration until the first completed conversion.
    settling: bool,
}

impl<SPI: SpiDevice> Max31856<SPI> {
    /// Wrap an SPI device. Does no I/O; call [`Max31856::configure`] (or the
    /// legacy [`Max31856::init`] sequence) before reading.
    pub fn new(spi: SPI) -> Self {
        Self {
            spi,
            expected: None,
            settling: false,
        }
    }

    /// Apply the whole configuration in one go and arm the read-back liveness
    /// check: MASK, then CR1 (type + averaging), then CR0 last (notch +
    /// open-circuit detection + continuous conversion), so conversion starts with
    /// everything else in place. The final register state equals the Adafruit
    /// `init` → `averaging` → `noise_rejection` → `start_autoconverting` sequence
    /// the reference runs.
    ///
    /// Returns [`Error::ConfigMismatch`] if the registers do not read back — the
    /// chip is absent or not talking. The check stays armed either way, so a dead
    /// sensor keeps reporting faults to the control loop instead of `0.0 °C`.
    pub fn configure(&mut self, cfg: SensorConfig) -> Result<(), Error<SPI::Error>> {
        let exp = Expected::from_config(cfg);
        self.expected = Some(exp);
        self.write_expected(exp)?;
        if self.config_matches(exp)? {
            Ok(())
        } else {
            Err(Error::ConfigMismatch)
        }
    }

    /// Poll for the first conversion after [`configure`](Self::configure):
    /// `Ok(true)` once the temperature registers hold a real result. The firmware
    /// calls this at boot before starting the control loop (the reference polls
    /// up to 1.5 s for a nonzero reading for the same reason).
    pub fn first_conversion_ready(&mut self) -> Result<bool, Error<SPI::Error>> {
        match self.checked_temperature() {
            Ok(_) => Ok(true),
            Err(Error::NotReady) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn write_expected(&mut self, exp: Expected) -> Result<(), Error<SPI::Error>> {
        self.settling = true;
        self.write_register(REG_MASK, exp.mask)?;
        self.write_register(REG_CR1, exp.cr1)?;
        self.write_register(REG_CR0, exp.cr0)
    }

    /// Burst-read CR0, CR1, MASK (consecutive addresses, auto-increment) and
    /// compare them against what was written.
    fn config_matches(&mut self, exp: Expected) -> Result<bool, Error<SPI::Error>> {
        let mut regs = [0u8; 3];
        self.read_register(REG_CR0, &mut regs)?;
        Ok(exp.matches(regs))
    }

    /// The liveness half of [`TempSensor::has_fault`](crate::TempSensor): `Ok(true)`
    /// if the configuration no longer reads back (after re-writing it, so a
    /// brown-out-reset chip recovers on its next conversion). Always `Ok(false)`
    /// when the driver was not set up through [`configure`](Self::configure).
    fn config_lost(&mut self) -> Result<bool, Error<SPI::Error>> {
        let Some(exp) = self.expected else {
            return Ok(false);
        };
        if self.config_matches(exp)? {
            return Ok(false);
        }
        // Best effort: if the bus is dead the write fails too and the next
        // read-back reports the fault again.
        let _ = self.write_expected(exp);
        Ok(true)
    }

    /// The latest temperature, refusing the all-zero register while settling.
    fn checked_temperature(&mut self) -> Result<f32, Error<SPI::Error>> {
        let raw = self.read_ltc_raw()?;
        if self.settling {
            if raw == 0 {
                return Err(Error::NotReady);
            }
            self.settling = false;
        }
        Ok(decode_ltc(raw))
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
    pub fn set_thermocouple_type(&mut self, tc: ThermocoupleType) -> Result<(), Error<SPI::Error>> {
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
    ///
    /// This is the raw register fetch (no settling check); the control loop goes
    /// through the [`TempSensor`](crate::TempSensor) impl, which adds it.
    pub fn read_temperature(&mut self) -> Result<f32, Error<SPI::Error>> {
        Ok(decode_ltc(self.read_ltc_raw()?))
    }

    /// The 19-bit LTC field, undecoded (`0` = the post-reset register value).
    fn read_ltc_raw(&mut self) -> Result<u32, Error<SPI::Error>> {
        let mut buf = [0u8; 3];
        self.read_register(REG_LTCBH, &mut buf)?;
        Ok(((buf[0] as u32) << 11) | ((buf[1] as u32) << 3) | ((buf[2] as u32) >> 5))
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

/// Decode the 19-bit two's-complement LTC field to °C (`LSB = 2^-7 °C`).
fn decode_ltc(combined: u32) -> f32 {
    let mut value = combined as i32;
    if value & 0x0004_0000 != 0 {
        value -= 0x0008_0000; // sign-extend the 19-bit field
    }
    value as f32 * THERM_LSB
}

impl<SPI: SpiDevice> crate::platform::TempSensor for Max31856<SPI> {
    type Error = Error<SPI::Error>;

    /// Fault register OR a failed configuration read-back (dead chip, broken
    /// MISO, chip reset). The read-back is a single 3-byte burst per call.
    fn has_fault(&mut self) -> Result<bool, Self::Error> {
        if self.config_lost()? {
            return Ok(true);
        }
        Ok(self.faults()?.any())
    }

    /// The latest temperature; [`Error::NotReady`] (a fault to the caller) until
    /// the first conversion after (re)configuration has landed.
    fn read_temperature(&mut self) -> Result<f32, Self::Error> {
        self.checked_temperature()
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
        assert_eq!(
            Averaging::from_samples(3).unwrap_or_default(),
            Averaging::S8
        );

        assert_eq!(NoiseFilter::from_hz(50), Some(NoiseFilter::Hz50));
        assert_eq!(NoiseFilter::from_hz(60), Some(NoiseFilter::Hz60));
        assert_eq!(NoiseFilter::from_hz(55), None);
        assert_eq!(
            NoiseFilter::from_hz(55).unwrap_or_default(),
            NoiseFilter::Hz60
        );
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

    // ---- liveness (configure / read-back / settling) ----------------------

    const S_CFG: SensorConfig = SensorConfig {
        thermocouple: ThermocoupleType::S,
        averaging: Averaging::S16,
        noise_filter: NoiseFilter::Hz50,
    };
    // CR0 = autoconvert | OCFAULT0 | 50 Hz ; CR1 = AVGSEL16 | S ; MASK = 0.
    const S_CR0: u8 = 0x91;
    const S_CR1: u8 = 0x46;

    fn configure_ok() -> Vec<Spi<u8>> {
        let mut e = Vec::new();
        e.extend(write_reg(REG_MASK, 0x00));
        e.extend(write_reg(REG_CR1, S_CR1));
        e.extend(write_reg(REG_CR0, S_CR0));
        e.extend(read_reg(REG_CR0, &[S_CR0, S_CR1, 0x00]));
        e
    }

    #[test]
    fn configure_writes_mask_cr1_then_cr0_and_verifies() {
        let expect = configure_ok();
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());
        dev.configure(S_CFG).unwrap();
        spi.done();
    }

    #[test]
    fn configure_reports_a_dead_chip() {
        // MISO pulled low (unpowered chip / broken wire): everything reads 0x00.
        let mut expect = Vec::new();
        expect.extend(write_reg(REG_MASK, 0x00));
        expect.extend(write_reg(REG_CR1, S_CR1));
        expect.extend(write_reg(REG_CR0, S_CR0));
        expect.extend(read_reg(REG_CR0, &[0x00, 0x00, 0x00]));
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());
        assert_eq!(dev.configure(S_CFG), Err(Error::ConfigMismatch));
        spi.done();
    }

    #[test]
    fn readback_ignores_reserved_and_self_clearing_bits() {
        let exp = Expected::from_config(S_CFG);
        // 1SHOT/FAULTCLR set in CR0, reserved CR1 bit 7 and MASK bits 7:6 set.
        assert!(exp.matches([S_CR0 | 0x42, S_CR1 | 0x80, 0xC0]));
        // CMODE dropped (chip reset to one-shot) is a mismatch.
        assert!(!exp.matches([S_CR0 & !CR0_AUTOCONVERT, S_CR1, 0x00]));
        // Power-on-reset defaults (CR0=0x00, CR1=0x03, MASK=0xFF) are a mismatch.
        assert!(!exp.matches([0x00, 0x03, 0xFF]));
    }

    #[test]
    fn dead_bus_is_a_fault_not_zero_degrees() {
        // Configured OK, then the chip dies: SR reads 0x00 ("no fault") and LTC
        // reads 0 (a plausible 0.0 C), the case the fault register cannot see.
        // The read-back must flag it and re-write the config.
        let mut expect = configure_ok();
        // First conversion lands: 25.0 C.
        expect.extend(read_reg(REG_LTCBH, &[0x01, 0x90, 0x00]));
        // Next tick: read-back all-zero -> fault, config re-written.
        expect.extend(read_reg(REG_CR0, &[0x00, 0x00, 0x00]));
        expect.extend(write_reg(REG_MASK, 0x00));
        expect.extend(write_reg(REG_CR1, S_CR1));
        expect.extend(write_reg(REG_CR0, S_CR0));
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());
        dev.configure(S_CFG).unwrap();
        assert!(dev.first_conversion_ready().unwrap());
        assert!(crate::TempSensor::has_fault(&mut dev).unwrap());
        spi.done();
    }

    #[test]
    fn healthy_chip_checks_readback_then_status() {
        let mut expect = configure_ok();
        expect.extend(read_reg(REG_CR0, &[S_CR0, S_CR1, 0x00]));
        expect.extend(read_reg(REG_SR, &[0x00]));
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());
        dev.configure(S_CFG).unwrap();
        assert!(!crate::TempSensor::has_fault(&mut dev).unwrap());
        spi.done();
    }

    #[test]
    fn zero_register_is_not_ready_until_first_conversion() {
        let mut expect = configure_ok();
        expect.extend(read_reg(REG_LTCBH, &[0x00, 0x00, 0x00])); // pending
        expect.extend(read_reg(REG_LTCBH, &[0x01, 0x90, 0x00])); // 25.0 C
        expect.extend(read_reg(REG_LTCBH, &[0x00, 0x00, 0x00])); // a real 0.0 C
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());
        dev.configure(S_CFG).unwrap();
        assert_eq!(
            crate::TempSensor::read_temperature(&mut dev),
            Err(Error::NotReady)
        );
        assert_eq!(crate::TempSensor::read_temperature(&mut dev), Ok(25.0));
        // Once settled, an all-zero register is a genuine 0.0 C reading.
        assert_eq!(crate::TempSensor::read_temperature(&mut dev), Ok(0.0));
        spi.done();
    }

    #[test]
    fn legacy_init_path_has_no_readback() {
        // Without `configure`, has_fault is the plain status-register check.
        let expect = read_reg(REG_SR, &[0x00]);
        let mut spi = SpiMock::new(&expect);
        let mut dev = Max31856::new(spi.clone());
        assert!(!crate::TempSensor::has_fault(&mut dev).unwrap());
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
