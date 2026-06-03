//! Delayed-start scheduler — port of `kiln/scheduler.py`
//! (`ScheduledProfileQueue`).
//!
//! Holds at most one pending item that should start at a future time. Two
//! deliberate departures from the MicroPython version, both pushing concerns to
//! the right layer:
//!
//! * **Time is injected** (`now: f64` seconds) instead of calling `time.time()`,
//!   so the logic is deterministic and host-testable.
//! * **Generic over the payload** `P` instead of hard-coding a filename
//!   `String`. The firmware can use a `heapless::String`, a profile index, etc.,
//!   keeping this crate `no_std` and allocation-free. ISO time formatting is
//!   presentation and lives in the caller.
//!
//! The locking in the original is unnecessary here — cross-core access is the
//! firmware's job (e.g. an `embassy_sync` mutex/channel).

/// A pending scheduled item.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Scheduled<P> {
    payload: P,
    start_time: f64,
    scheduled_at: f64,
}

/// Snapshot for status reporting, mirroring `get_status()` minus the
/// presentation-only ISO string.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScheduleStatus<'a, P> {
    pub payload: &'a P,
    pub start_time: f64,
    /// `int(max(0, start_time - now))` — whole seconds remaining, truncated
    /// toward zero exactly like the reference.
    pub seconds_until_start: u64,
}

/// Single-slot delayed-start queue.
#[derive(Debug, Clone, Default)]
pub struct ScheduledProfileQueue<P> {
    item: Option<Scheduled<P>>,
}

impl<P> ScheduledProfileQueue<P> {
    /// Create an empty queue.
    pub const fn new() -> Self {
        Self { item: None }
    }

    /// Schedule `payload` to start at `start_time` (seconds), given the current
    /// `now`. Returns `false` (rejecting) on a second schedule or any non-future
    /// start time; the API layer validates and reports those to the client.
    pub fn schedule(&mut self, payload: P, start_time: f64, now: f64) -> bool {
        if self.item.is_some() {
            return false;
        }
        if start_time <= now {
            return false;
        }
        self.item = Some(Scheduled {
            payload,
            start_time,
            scheduled_at: now,
        });
        true
    }

    /// Whether a scheduled item exists and its start time has arrived.
    pub fn can_consume(&self, now: f64) -> bool {
        match &self.item {
            Some(it) => now >= it.start_time,
            None => false,
        }
    }

    /// Take the scheduled payload if its start time has arrived, clearing the
    /// slot. Returns `None` if nothing is scheduled or it isn't due yet.
    pub fn consume(&mut self, now: f64) -> Option<P> {
        let due = matches!(&self.item, Some(it) if now >= it.start_time);
        if due {
            // Safe: `due` implies `item` is `Some`.
            return self.item.take().map(|it| it.payload);
        }
        None
    }

    /// Cancel any scheduled item. Returns `true` if something was cancelled.
    pub fn cancel(&mut self) -> bool {
        self.item.take().is_some()
    }

    /// Status snapshot, or `None` when nothing is scheduled.
    pub fn status(&self, now: f64) -> Option<ScheduleStatus<'_, P>> {
        let it = self.item.as_ref()?;
        let remaining = it.start_time - now;
        let remaining = if remaining < 0.0 { 0.0 } else { remaining };
        Some(ScheduleStatus {
            payload: &it.payload,
            start_time: it.start_time,
            seconds_until_start: remaining as u64, // truncates toward zero, == Python int()
        })
    }

    /// Whether anything is currently scheduled.
    #[cfg(test)]
    pub fn is_scheduled(&self) -> bool {
        self.item.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_rejects_non_future_start() {
        let mut q: ScheduledProfileQueue<&str> = ScheduledProfileQueue::new();
        assert!(!q.schedule("cone6.json", 100.0, 100.0));
        assert!(!q.schedule("cone6.json", 90.0, 100.0));
        assert!(!q.is_scheduled());
    }

    #[test]
    fn schedule_rejects_double_booking() {
        let mut q = ScheduledProfileQueue::new();
        assert!(q.schedule("a.json", 200.0, 100.0));
        assert!(!q.schedule("b.json", 300.0, 100.0));
    }

    #[test]
    fn can_consume_and_consume_respect_start_time() {
        let mut q = ScheduledProfileQueue::new();
        assert!(q.schedule("cone6.json", 200.0, 100.0));

        assert!(!q.can_consume(199.999));
        assert_eq!(q.consume(199.999), None);

        assert!(q.can_consume(200.0)); // exactly due
        let taken = q.consume(250.0);
        assert_eq!(taken, Some("cone6.json"));
        assert!(!q.is_scheduled(), "consume must clear the slot");
        assert_eq!(q.consume(300.0), None);
    }

    #[test]
    fn cancel_reports_whether_something_was_removed() {
        let mut q = ScheduledProfileQueue::new();
        assert!(!q.cancel());
        assert!(q.schedule("a.json", 200.0, 100.0));
        assert!(q.cancel());
        assert!(!q.is_scheduled());
    }

    #[test]
    fn status_reports_truncated_seconds_remaining() {
        let mut q = ScheduledProfileQueue::new();
        assert!(q.status(0.0).is_none());

        assert!(q.schedule("cone6.json", 3700.0, 100.0));
        let s = q.status(100.5).unwrap();
        assert_eq!(*s.payload, "cone6.json");
        assert_eq!(s.start_time, 3700.0);
        // 3700 - 100.5 = 3599.5 -> int() -> 3599
        assert_eq!(s.seconds_until_start, 3599);

        // Past due clamps to 0 (never negative), matching max(0, ...).
        let s2 = q.status(9999.0).unwrap();
        assert_eq!(s2.seconds_until_start, 0);
    }
}
