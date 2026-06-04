//! Temperature-rate monitor — port of `kiln/rate_monitor.py` (`TempHistory`).
//!
//! A fixed-capacity ring buffer of `(time, temp)` samples that computes a
//! heating/cooling rate in °C/hour over a window. The rate uses the same
//! "newest sample vs. the sample closest to `window` seconds ago" method as the
//! reference, including its first-wins tie-breaking for `max`/`min`.
//!
//! `no_std`, zero-dependency, no float intrinsics (manual `abs`).

/// `|x|` without pulling in `std`/libm. Matches Python `abs` for finite values.
#[inline]
fn abs(x: f32) -> f32 {
    if x < 0.0 {
        -x
    } else {
        x
    }
}

/// Fixed-capacity temperature history. `CAP` mirrors the Python
/// `TempHistory(capacity=...)` argument; the kiln controller uses `CAP = 60`.
#[derive(Debug, Clone)]
pub struct TempHistory<const CAP: usize> {
    buf: [(f32, f32); CAP],
    len: usize,
    write_index: usize,
}

impl<const CAP: usize> Default for TempHistory<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAP: usize> TempHistory<CAP> {
    /// Create an empty history.
    pub const fn new() -> Self {
        Self {
            buf: [(0.0, 0.0); CAP],
            len: 0,
            write_index: 0,
        }
    }

    /// Add a `(timestamp, temp)` sample. Once full, overwrites the oldest write
    /// slot exactly as the reference circular buffer does.
    pub fn add(&mut self, timestamp: f32, temp: f32) {
        if self.len < CAP {
            self.buf[self.len] = (timestamp, temp);
            self.len += 1;
        } else {
            self.buf[self.write_index] = (timestamp, temp);
            self.write_index = (self.write_index + 1) % CAP;
        }
    }

    /// Temperature rate over `window_seconds`, in °C/hour
    /// (positive = heating, negative = cooling). Returns `0.0` with fewer than
    /// two samples or a zero time span.
    pub fn get_rate(&self, window_seconds: f32) -> f32 {
        if self.len < 2 {
            return 0.0;
        }
        let active = &self.buf[..self.len];

        // recent = max by timestamp (first max wins, matching Python `max`).
        let mut recent = active[0];
        for &r in &active[1..] {
            if r.0 > recent.0 {
                recent = r;
            }
        }
        let (recent_time, recent_temp) = recent;
        let target_time = recent_time - window_seconds;

        // old = among samples with time <= recent_time, the one closest to
        // target_time (first min wins, matching Python `min`).
        let mut old: Option<(f32, f32)> = None;
        for &r in active {
            if r.0 <= recent_time {
                match old {
                    None => old = Some(r),
                    Some(o) => {
                        if abs(r.0 - target_time) < abs(o.0 - target_time) {
                            old = Some(r);
                        }
                    }
                }
            }
        }
        let (old_time, old_temp) = match old {
            Some(o) => o,
            None => return 0.0,
        };

        let dt_hours = (recent_time - old_time) / 3600.0;
        if dt_hours == 0.0 {
            return 0.0;
        }
        (recent_temp - old_temp) / dt_hours
    }

    /// Drop all samples (used on step transitions).
    pub fn clear(&mut self) {
        self.len = 0;
        self.write_index = 0;
    }

    /// Number of stored samples.
    #[cfg(test)]
    pub fn size(&self) -> usize {
        self.len
    }

    /// Whether the buffer is at capacity. Used by the golden replay test
    /// (`tests/replay_rate.rs`).
    pub fn is_full(&self) -> bool {
        self.len >= CAP
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_single_sample_rate_is_zero() {
        let mut h = TempHistory::<60>::new();
        assert_eq!(h.get_rate(600.0), 0.0);
        h.add(0.0, 20.0);
        assert_eq!(h.get_rate(600.0), 0.0);
        assert_eq!(h.size(), 1);
    }

    #[test]
    fn linear_ramp_rate_is_degrees_per_hour() {
        let mut h = TempHistory::<60>::new();
        // +1 C every 10 s over 600 s => 360 C/h.
        for i in 0..=60 {
            h.add((i as f32) * 10.0, 20.0 + i as f32);
        }
        let rate = h.get_rate(600.0);
        assert!((rate - 360.0).abs() < 1e-2, "rate={rate}");
    }

    #[test]
    fn ring_overwrites_when_full() {
        let mut h = TempHistory::<4>::new();
        for i in 0..6 {
            h.add(i as f32, i as f32);
        }
        assert!(h.is_full());
        assert_eq!(h.size(), 4);
    }

    #[test]
    fn clear_resets() {
        let mut h = TempHistory::<8>::new();
        h.add(0.0, 20.0);
        h.add(10.0, 30.0);
        h.clear();
        assert_eq!(h.size(), 0);
        assert_eq!(h.get_rate(600.0), 0.0);
    }
}
