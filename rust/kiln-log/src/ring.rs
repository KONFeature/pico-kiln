//! A fixed-capacity byte ring of newline-delimited records — the live-tail
//! snapshot buffer. Whole records are pushed; on overflow the oldest bytes are
//! dropped. `snapshot` returns a clean, line-aligned, valid-UTF-8 view.

/// Fixed-capacity byte ring. `N` is the buffer size in bytes.
pub struct Ring<const N: usize> {
    buf: [u8; N],
    /// Index of the oldest byte.
    start: usize,
    /// Number of valid bytes currently stored (<= N).
    len: usize,
    /// True once any byte has been overwritten (the buffer has wrapped). Distinct
    /// from `len == N`: a buffer filled exactly to capacity without overflow is
    /// NOT wrapped, and all its bytes are whole records.
    wrapped: bool,
}

impl<const N: usize> Ring<N> {
    pub const fn new() -> Self {
        Self {
            buf: [0u8; N],
            start: 0,
            len: 0,
            wrapped: false,
        }
    }

    /// Append `bytes`, dropping oldest bytes if it would overflow. A record
    /// longer than `N` keeps only its last `N` bytes.
    pub fn push(&mut self, bytes: &[u8]) {
        for &b in bytes {
            let pos = (self.start + self.len) % N;
            self.buf[pos] = b;
            if self.len == N {
                // Full: advance start, overwriting the oldest byte.
                self.start = (self.start + 1) % N;
                self.wrapped = true;
            } else {
                self.len += 1;
            }
        }
    }

    /// Copy the most-recent whole lines into `out` and return the byte count.
    ///
    /// Guarantees the result is **line-aligned** (starts and ends on record
    /// boundaries) and therefore valid UTF-8 given UTF-8 records:
    /// - if the buffer has wrapped, the partially-overwritten leading record is
    ///   dropped;
    /// - the trailing bytes are truncated to the last newline, so an `out`
    ///   smaller than the stored content yields whole lines only.
    ///
    /// Returns `0` when no complete line is available (e.g. a wrapped buffer
    /// holding a single record longer than `N`, or an `out` too small for even
    /// one line). When `out` is smaller than the stored content, the EARLIEST
    /// whole lines that fit are returned.
    pub fn snapshot(&self, out: &mut [u8]) -> usize {
        // Linearize oldest-first into out, truncated to out's capacity.
        let cap = out.len().min(self.len);
        for i in 0..cap {
            out[i] = self.buf[(self.start + i) % N];
        }

        // Drop a partial leading record only if the buffer has actually wrapped.
        let begin = if self.wrapped {
            match out[..cap].iter().position(|&b| b == b'\n') {
                Some(nl) => nl + 1,
                None => return 0, // no complete line present
            }
        } else {
            0
        };

        // Keep whole lines only: end at the last newline in the remaining window.
        let end = match out[begin..cap].iter().rposition(|&b| b == b'\n') {
            Some(rel) => begin + rel + 1,
            None => return 0,
        };

        out.copy_within(begin..end, 0);
        end - begin
    }
}

impl<const N: usize> Default for Ring<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_snapshot_below_capacity() {
        let mut r: Ring<64> = Ring::new();
        r.push(b"hello\n");
        r.push(b"world\n");
        let mut out = [0u8; 64];
        let n = r.snapshot(&mut out);
        assert_eq!(&out[..n], b"hello\nworld\n");
    }

    #[test]
    fn exact_capacity_fill_keeps_all_lines() {
        // total pushed == N with NO overflow: nothing must be dropped.
        let mut r: Ring<12> = Ring::new();
        r.push(b"hello\nworld\n"); // exactly 12 bytes
        let mut out = [0u8; 12];
        let n = r.snapshot(&mut out);
        assert_eq!(&out[..n], b"hello\nworld\n");
    }

    #[test]
    fn wrap_drops_oldest_and_partial_leading_line() {
        // Capacity 16: four 5-byte records (20 bytes) -> wraps.
        let mut r: Ring<16> = Ring::new();
        r.push(b"aaaa\n");
        r.push(b"bbbb\n");
        r.push(b"cccc\n");
        r.push(b"dddd\n");
        let mut out = [0u8; 16];
        let n = r.snapshot(&mut out);
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert_eq!(s, "bbbb\ncccc\ndddd\n");
    }

    #[test]
    fn wrapped_with_no_complete_line_returns_zero() {
        // A single record longer than N leaves no newline in the ring.
        let mut r: Ring<8> = Ring::new();
        r.push(b"abcdefghij"); // 10 bytes, no newline, wraps
        let mut out = [0u8; 8];
        let n = r.snapshot(&mut out);
        assert_eq!(n, 0);
    }

    #[test]
    fn small_out_returns_only_whole_lines() {
        // out smaller than the single stored line -> no complete line fits.
        let mut r: Ring<64> = Ring::new();
        r.push(b"line-one\n"); // 9 bytes, newline at index 8
        let mut out = [0u8; 4];
        let n = r.snapshot(&mut out);
        assert_eq!(n, 0);
    }
}
