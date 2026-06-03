//! Thermocouple reading conditioning and fault tolerance.
//!
//! Sans-IO port of the *software* half of `kiln/hardware.py`'s
//! `TemperatureSensor` after the MAX31856 filtering rework. The chip itself runs
//! in continuous mode and applies the SINC + notch + AVGSEL filtering (that lives
//! in `kiln-hal`); this module is the part that does **not** touch SPI:
//!
//! - a small **median** window that rejects isolated SSR/EMI spikes without the
//!   lag a moving average would add (deliberately not an EMA),
//! - range validation against `[-50, 1500] °C`,
//! - consecutive-fault counting with a higher tolerance during cold start
//!   (S-type thermocouples are noisy at low mV), and
//! - a window re-seed after a sustained dropout so the median recovers from
//!   fresh samples instead of blending pre-fault values.
//!
//! Feed it [`TempFilter::push_reading`] when the sensor returned a value, or
//! [`TempFilter::push_fault`] when the fault register tripped or the read failed.
//! The arithmetic mirrors the reference left-to-right (validated by
//! `tests/replay_temp_filter.rs`).

const MAX_CONSECUTIVE_FAULTS: u32 = 20;
const COLD_START_FAULT_LIMIT: u32 = 40;
const COLD_START_TEMP_THRESHOLD: f64 = 100.0;
const TEMP_MIN_RANGE: f64 = -50.0;
const TEMP_MAX_RANGE: f64 = 1500.0;

/// A read that the filter cannot recover from; the control loop must shut down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempError {
    /// A fault arrived before any valid reading — the sensor never initialised.
    NotInitialized,
    /// Too many consecutive faults; emergency shutdown.
    EmergencyShutdown,
}

/// Median spike-rejection filter with fault tolerance. `CAP` is the maximum
/// median window; the effective window is set at construction.
#[derive(Debug, Clone)]
pub struct TempFilter<const CAP: usize> {
    offset: f64,
    window: usize,
    samples: [f64; CAP],
    len: usize,
    last_good: f64,
    initialized: bool,
    fault_count: u32,
    max_recorded: f64,
}

impl<const CAP: usize> TempFilter<CAP> {
    /// Create a filter that adds `offset` °C to each raw reading and takes the
    /// median over `median_window` samples (clamped to `1..=CAP`; `1` disables
    /// software filtering). Mirrors the `TemperatureSensor` constructor with no
    /// valid first reading yet (uninitialised, empty window).
    pub fn new(offset: f64, median_window: usize) -> Self {
        let window = if median_window < 1 {
            1
        } else if median_window > CAP {
            CAP
        } else {
            median_window
        };
        Self {
            offset,
            window,
            samples: [0.0; CAP],
            len: 0,
            last_good: 0.0,
            initialized: false,
            fault_count: 0,
            max_recorded: 0.0,
        }
    }

    /// Feed a raw sensor reading (°C, before offset). Returns the median-filtered
    /// temperature, or — if the reading is out of range — the recovered last-good
    /// value (treated as a fault), or an error if the fault budget is exhausted.
    pub fn push_reading(&mut self, raw: f64) -> Result<f64, TempError> {
        if raw < TEMP_MIN_RANGE || raw > TEMP_MAX_RANGE {
            return self.fault();
        }

        let temp = raw + self.offset;
        self.push_sample(temp);
        let filtered = self.median();

        if filtered > self.max_recorded {
            self.max_recorded = filtered;
        }
        self.initialized = true;
        self.fault_count = 0;
        self.last_good = filtered;
        Ok(filtered)
    }

    /// Record a sensor fault (fault register tripped, or the read failed).
    /// Returns the last-good temperature while the fault budget lasts, then an
    /// error: [`TempError::NotInitialized`] if no valid reading ever arrived,
    /// otherwise [`TempError::EmergencyShutdown`].
    pub fn push_fault(&mut self) -> Result<f64, TempError> {
        self.fault()
    }

    /// Last successfully filtered temperature (°C). Meaningful once initialised.
    pub fn last_good(&self) -> f64 {
        self.last_good
    }

    /// Whether a valid reading has ever been seen.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Current consecutive-fault count (reset to 0 by a good reading).
    pub fn fault_count(&self) -> u32 {
        self.fault_count
    }

    fn fault(&mut self) -> Result<f64, TempError> {
        self.fault_count += 1;

        if self.fault_count as usize >= self.window {
            self.len = 0; // re-seed: drop stale samples after a sustained dropout
        }

        if !self.initialized {
            return Err(TempError::NotInitialized);
        }

        let limit = if self.max_recorded < COLD_START_TEMP_THRESHOLD {
            COLD_START_FAULT_LIMIT
        } else {
            MAX_CONSECUTIVE_FAULTS
        };
        if self.fault_count >= limit {
            return Err(TempError::EmergencyShutdown);
        }
        Ok(self.last_good)
    }

    fn push_sample(&mut self, temp: f64) {
        if self.len < self.window {
            self.samples[self.len] = temp;
            self.len += 1;
        } else {
            for i in 1..self.window {
                self.samples[i - 1] = self.samples[i];
            }
            self.samples[self.window - 1] = temp;
        }
    }

    fn median(&self) -> f64 {
        let n = self.len;
        let mut sorted = [0.0f64; CAP];
        sorted[..n].copy_from_slice(&self.samples[..n]);
        for i in 1..n {
            let mut j = i;
            while j > 0 && sorted[j - 1] > sorted[j] {
                sorted.swap(j - 1, j);
                j -= 1;
            }
        }
        let mid = n / 2;
        if n % 2 == 1 {
            sorted[mid]
        } else {
            0.5 * (sorted[mid - 1] + sorted[mid])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_handles_warmup_even_and_odd_windows() {
        let mut f = TempFilter::<3>::new(0.0, 3);
        assert_eq!(f.push_reading(25.0).unwrap(), 25.0); // [25] -> 25
        assert_eq!(f.push_reading(26.0).unwrap(), 25.5); // [25,26] -> mean
        assert_eq!(f.push_reading(24.0).unwrap(), 25.0); // [25,26,24] -> 25
        assert_eq!(f.push_reading(100.0).unwrap(), 26.0); // [26,24,100] -> 26 (spike rejected)
    }

    #[test]
    fn offset_is_applied_before_filtering() {
        let mut f = TempFilter::<3>::new(2.5, 1);
        assert_eq!(f.push_reading(20.0).unwrap(), 22.5);
    }

    #[test]
    fn out_of_range_reading_is_a_fault_returning_last_good() {
        let mut f = TempFilter::<3>::new(0.0, 3);
        f.push_reading(30.0).unwrap();
        // 1600 > 1500 -> treated as a fault, returns last good, counts up.
        assert_eq!(f.push_reading(1600.0).unwrap(), 30.0);
        assert_eq!(f.fault_count(), 1);
    }

    #[test]
    fn fault_before_first_reading_is_not_initialized() {
        let mut f = TempFilter::<3>::new(0.0, 3);
        assert_eq!(f.push_fault(), Err(TempError::NotInitialized));
    }

    #[test]
    fn cold_start_tolerates_more_faults_than_hot() {
        // Cold (max_recorded < 100): limit is 40.
        let mut cold = TempFilter::<3>::new(0.0, 3);
        cold.push_reading(50.0).unwrap();
        for _ in 0..39 {
            assert_eq!(cold.push_fault().unwrap(), 50.0);
        }
        assert_eq!(cold.push_fault(), Err(TempError::EmergencyShutdown));

        // Hot (max_recorded >= 100): limit drops to 20.
        let mut hot = TempFilter::<3>::new(0.0, 3);
        hot.push_reading(200.0).unwrap();
        for _ in 0..19 {
            assert_eq!(hot.push_fault().unwrap(), 200.0);
        }
        assert_eq!(hot.push_fault(), Err(TempError::EmergencyShutdown));
    }

    #[test]
    fn window_reseeds_after_sustained_dropout() {
        let mut f = TempFilter::<3>::new(0.0, 3);
        f.push_reading(10.0).unwrap();
        f.push_reading(12.0).unwrap();
        f.push_reading(14.0).unwrap(); // window full: [10,12,14]
                                       // 3 faults (>= window) clear the window mid-way.
        f.push_fault().unwrap();
        f.push_fault().unwrap();
        f.push_fault().unwrap();
        // Fresh reading is the only sample -> median is itself, no blend with 10..14.
        assert_eq!(f.push_reading(800.0).unwrap(), 800.0);
    }
}
