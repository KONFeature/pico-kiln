//! Core 0 logging glue: the global `log::Log` implementation, the bounded
//! drop-oldest channels both cores feed, the RAM ring (`/api/logs` snapshot), the
//! Core 0 drain task, and the unified flash flusher that persists both the diag
//! log and the CSV rows in one flash-paused window.
//!
//! Producers (either core) only `try_send` a pre-formatted line into
//! `LOG_CHANNEL` with drop-oldest on overflow — they never block on a queue,
//! never touch flash, and never take an *async* lock. The only synchronisation a
//! producer incurs is a short, O(1) critical section to read the wall clock for
//! the timestamp (the same cross-core CS the control loop already takes every
//! iteration). The single Core 0 drain task is the sole writer of the ring and the
//! sole feeder of the flash channel.
//!
//! FLASH FLUSH COALESCING. The diag log and the CSV run log share one writer
//! ([`flash_flush_task`]) so a periodic persist costs a single SSR pause + single
//! filesystem mount for both, instead of one each. The CSV logger
//! ([`crate::server::csv_logger_task`]) hands its steady-state rows off through
//! [`csv_push_row`]/[`csv_set_name`] and nudges the flusher with [`request_flush`];
//! its recovery-critical edge writes (header, RECOVERY row, terminal row) stay
//! direct.

use core::cell::{Cell, RefCell};
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, ThreadModeRawMutex};
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::channel::{Channel, TrySendError};
use embassy_sync::signal::Signal;

use crate::config::LogLevel;
use crate::server::Clock;

/// Bytes per formatted line. Longer messages are truncated (with a guaranteed
/// trailing newline).
pub const LINE_CAP: usize = 128;
/// Depth of the producer->drain channel.
const CHAN_CAP: usize = 32;
/// Depth of the drain->flash channel.
const FLASH_CHAN_CAP: usize = 32;
/// RAM ring size in bytes (the `/api/logs` snapshot).
pub const RING_CAP: usize = 8 * 1024;

/// One formatted log line.
pub type LogLine = heapless::String<LINE_CAP>;

/// Producer -> drain task. `CriticalSectionRawMutex` is REQUIRED: producers are on
/// BOTH cores (the Core 1 control loop logs too), so the send must take the
/// cross-core critical section.
pub(crate) static LOG_CHANNEL: Channel<CriticalSectionRawMutex, LogLine, CHAN_CAP> = Channel::new();
/// Drain task -> flash-writer task. Both ends are Core 0 tasks (the drain feeds, the
/// flusher receives), so a `ThreadModeRawMutex` is correct and skips the cross-core CS.
pub(crate) static FLASH_LOG_CHANNEL: Channel<ThreadModeRawMutex, LogLine, FLASH_CHAN_CAP> =
    Channel::new();
/// The live-tail snapshot ring (written only by the drain task; read by the snapshot
/// handler). Both accessors are Core 0 tasks on the same executor, so a
/// `ThreadModeRawMutex` is sound — and it MATTERS here: a `CriticalSectionRawMutex` would
/// hold the cross-core critical section for the full `RING_CAP` byte-by-byte
/// `snapshot` copy, stalling Core 1's per-tick wall-clock CS on every `/api/logs`
/// poll. `RefCell` interior mutability is sound for the same reason (single
/// executor, lock never held across an `.await`).
pub(crate) static LOG_RING: BlockingMutex<ThreadModeRawMutex, RefCell<kiln_log::Ring<RING_CAP>>> =
    BlockingMutex::new(RefCell::new(kiln_log::Ring::new()));
/// Whether flash persistence is enabled (the `LOG_TO_FLASH` knob).
pub(crate) static FLASH_ENABLED: AtomicBool = AtomicBool::new(false);
/// Late-bound wall clock (set once Core 0 builds it). Reads fall back to uptime.
/// `CriticalSectionRawMutex` is REQUIRED: read from BOTH cores (Core 1 stamps its own
/// log lines via [`now_secs`]).
static LOG_CLOCK: BlockingMutex<
    CriticalSectionRawMutex,
    Cell<Option<&'static (dyn Clock + Sync)>>,
> = BlockingMutex::new(Cell::new(None));

// === CSV ⇆ flusher handoff ==================================================
//
// The CSV logger accumulates steady-state rows here; [`flash_flush_task`] persists
// them coalesced with the diag log. Only Core 0 touches these (the CSV task pushes,
// the flusher copies+clears), so they use `ThreadModeRawMutex` — a `CriticalSectionRawMutex`
// would hold the cross-core CS for the O(n) `CSV_BUF_CAP` buffer copy, needlessly
// stalling Core 1.

/// CSV run-log filename cap (mirrors `Status::filename`).
const CSV_NAME_CAP: usize = 96;
/// RAM accumulator for un-flushed CSV rows. Sized to hold one flush interval at the
/// fastest cadence (TUNING: 120 s / 2 s = 60 rows × ~130 B ≈ 7.8 KB) with margin.
pub(crate) const CSV_BUF_CAP: usize = 8192;
/// Push past this fraction of [`CSV_BUF_CAP`] and the producer asks for an early
/// flush, leaving headroom so a single further row can never overflow before the
/// flusher (which runs on the next executor tick) drains it.
const CSV_HIGH_WATER: usize = CSV_BUF_CAP * 3 / 4;

/// Steady-state CSV rows awaiting flush (filled by [`csv_push_row`], drained by the
/// flusher). Not the recovery-critical edge writes — those stay direct.
static CSV_PENDING: BlockingMutex<ThreadModeRawMutex, RefCell<heapless::String<CSV_BUF_CAP>>> =
    BlockingMutex::new(RefCell::new(heapless::String::new()));
/// The active run-log filename the pending rows belong to.
static CSV_NAME: BlockingMutex<ThreadModeRawMutex, RefCell<heapless::String<CSV_NAME_CAP>>> =
    BlockingMutex::new(RefCell::new(heapless::String::new()));
/// CSV → flusher nudge: "flush soon" (run start, leaving, or high-water). Core-0-only
/// (CSV task signals, flusher waits) ⇒ `ThreadModeRawMutex`.
static FLUSH_NOW: Signal<ThreadModeRawMutex, ()> = Signal::new();
/// Flusher → CSV ack: signalled after every flush so [`flush_and_wait`] can block
/// the run-end path until the pending rows are durable. Core-0-only ⇒ `ThreadModeRawMutex`.
static FLUSH_DONE: Signal<ThreadModeRawMutex, ()> = Signal::new();

/// Append a rendered CSV row to the pending batch. Returns `true` once the buffer
/// has crossed the high-water mark, signalling the caller to [`request_flush`]. A
/// row that genuinely will not fit (buffer pathologically full) is dropped rather
/// than blocking the control-status task — but the high-water early flush makes
/// that unreachable in practice (one row per status tick, flusher runs each tick).
/// Latches once a CSV row has been dropped, so the warning fires once per
/// low-space episode (re-armed by a successful flush in `csv_clear_pending`)
/// instead of on every dropped row.
static CSV_DROP_WARNED: AtomicBool = AtomicBool::new(false);

pub(crate) fn csv_push_row(row: &str) -> bool {
    let (over, dropped) = CSV_PENDING.lock(|b| {
        let mut b = b.borrow_mut();
        let dropped = b.push_str(row).is_err();
        (b.len() >= CSV_HIGH_WATER, dropped)
    });
    // Surface a dropped firing-data row (buffer full ⇒ flushes aren't draining it,
    // i.e. the filesystem is full or writes are failing). Retention should make this
    // unreachable; if it fires, the run is losing data. Logged outside the lock.
    if dropped && !CSV_DROP_WARNED.swap(true, Ordering::Relaxed) {
        log::warn!(
            target: "kiln_app::logging",
            "CSV row DROPPED — pending buffer full (flash space / writes failing)"
        );
    }
    over
}

/// Record the run-log filename the pending rows belong to (set on the run-start
/// edge, before the first row is pushed).
pub(crate) fn csv_set_name(name: &str) {
    CSV_NAME.lock(|n| {
        let mut n = n.borrow_mut();
        n.clear();
        let _ = n.push_str(name);
    });
}

/// Nudge [`flash_flush_task`] to flush now (next executor tick).
pub(crate) fn request_flush() {
    FLUSH_NOW.signal(());
}

/// Discard any pending CSV rows (the run-start reset — mirrors the reference's
/// `buf.clear()` at the start of a run, so a failed prior flush can't bleed rows
/// into the next run's file).
pub(crate) fn csv_reset() {
    CSV_PENDING.lock(|b| b.borrow_mut().clear());
}

/// Request a flush and await its completion. Used on the run-end edge so the
/// terminal row (already pushed) is durable, and `CSV_PENDING` is drained, BEFORE
/// the caller clears `active_run` and processes the next run. The terminal row is
/// pushed before this call, so whichever flush wakes us necessarily included it;
/// the leading `reset` drops any stale completion from an earlier flush. The flush
/// reuses the flusher's own scratch — no extra buffer here.
pub(crate) async fn flush_and_wait() {
    FLUSH_DONE.reset();
    FLUSH_NOW.signal(());
    FLUSH_DONE.wait().await;
}

/// Take and clear the pending CSV rows into `scratch` for a flush. Returns the
/// filename if there were rows to write. The caller MUST NOT clear on a failed
/// write — see [`csv_clear_pending`]. Copies under the lock (no `.await`), so the
/// CSV task cannot interleave between this and a later [`csv_clear_pending`].
fn csv_take_into(scratch: &mut heapless::String<CSV_BUF_CAP>, name: &mut heapless::String<CSV_NAME_CAP>) -> bool {
    scratch.clear();
    let has_rows = CSV_PENDING.lock(|b| {
        let b = b.borrow();
        let _ = scratch.push_str(b.as_str());
        !b.is_empty()
    });
    if has_rows {
        CSV_NAME.lock(|n| {
            name.clear();
            let _ = name.push_str(n.borrow().as_str());
        });
    }
    has_rows
}

/// Clear the pending CSV rows after a successful flush. Safe to call only when no
/// `.await` has run since [`csv_take_into`] (so no new rows have been pushed).
fn csv_clear_pending() {
    CSV_PENDING.lock(|b| b.borrow_mut().clear());
    // A successful drain re-arms the drop warning for any later low-space episode.
    CSV_DROP_WARNED.store(false, Ordering::Relaxed);
}

/// The global logger (a unit struct; all state lives in the statics above).
struct KilnLogger;

static LOGGER: KilnLogger = KilnLogger;

fn level_to_filter(level: LogLevel) -> log::LevelFilter {
    match level {
        LogLevel::Off => log::LevelFilter::Off,
        LogLevel::Error => log::LevelFilter::Error,
        LogLevel::Warn => log::LevelFilter::Warn,
        LogLevel::Info => log::LevelFilter::Info,
        LogLevel::Debug => log::LevelFilter::Debug,
    }
}

/// Current time in seconds for line stamping: wall-clock when a synced clock is
/// registered, else monotonic uptime.
fn now_secs() -> i64 {
    let wall = LOG_CLOCK.lock(|c| c.get()).and_then(|c| c.unix_seconds());
    wall.unwrap_or_else(|| embassy_time::Instant::now().as_secs() as i64)
}

/// Push a finished line into `LOG_CHANNEL`, dropping the OLDEST queued line if the
/// channel is full (keeps the most recent lines flowing to the live tail). Never
/// blocks — safe to call from the Core 1 control loop.
fn push_line(line: LogLine) {
    if let Err(TrySendError::Full(line)) = LOG_CHANNEL.try_send(line) {
        let _ = LOG_CHANNEL.try_receive();
        let _ = LOG_CHANNEL.try_send(line);
    }
}

impl log::Log for KilnLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let mut line = LogLine::new();
        let secs = now_secs();
        let res = kiln_log::format_line(
            &mut line,
            secs,
            record.level().as_str(),
            kiln_log::tag_of(record.target()),
            *record.args(),
        );
        // On capacity overflow the trailing newline may have been dropped;
        // guarantee one so the ring/SSE record boundaries stay intact.
        if res.is_err() && !line.as_str().ends_with('\n') {
            if line.len() == LINE_CAP {
                line.pop();
            }
            let _ = line.push('\n');
        }
        push_line(line);
    }

    fn flush(&self) {}
}

/// Install the global logger. Call once, before the core split. The wall clock is
/// bound later via [`set_clock`]; until then lines carry an uptime timestamp.
pub fn init(level: LogLevel, flash_enabled: bool) {
    FLASH_ENABLED.store(flash_enabled, Ordering::Relaxed);
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(level_to_filter(level));
}

/// Bind the wall clock once Core 0 has built it (sharpens line timestamps once
/// NTP syncs). Safe to call from Core 0 after [`init`].
pub fn set_clock(clock: &'static (dyn Clock + Sync)) {
    LOG_CLOCK.lock(|c| c.set(Some(clock)));
}

/// Core 0 task: drain `LOG_CHANNEL` and fan each line out to the RAM ring (the
/// `/api/logs` snapshot) and, when enabled, the flash channel. The sole writer of
/// the ring.
#[embassy_executor::task]
pub async fn log_drain_task() -> ! {
    loop {
        let line = LOG_CHANNEL.receive().await;

        // Ring (snapshot). Lock held without `.await`, so no reentrancy.
        LOG_RING.lock(|r| r.borrow_mut().push(line.as_bytes()));

        // Flash: drop-oldest, never blocks the drain.
        if FLASH_ENABLED.load(Ordering::Relaxed) {
            if let Err(TrySendError::Full(line)) = FLASH_LOG_CHANNEL.try_send(line) {
                let _ = FLASH_LOG_CHANNEL.try_receive();
                let _ = FLASH_LOG_CHANNEL.try_send(line);
            }
        }
    }
}

// === Unified flash flusher ==================================================

use crate::api::Directory;
use crate::server::{BatchWrite, Storage};
use crate::timefmt::write_iso;
use embassy_futures::select::{select, Either};

/// Diag-line batch buffer. Lines (≤[`LINE_CAP`]) accumulate here between flushes.
/// Sized at 8 KiB (≈64 lines) so a verbose/debug log burst fills it slowly: the
/// high-water early flush — which forces an SSR-pausing flash write — fires roughly
/// every ~48 lines instead of ~12, so noisy logging perturbs the control loop far
/// less. The [`FLUSH_INTERVAL_S`] timer still bounds how stale a quiet log gets, and
/// the RAM ring (`/api/logs`) keeps the live view, so the only cost of buffering
/// more is a larger power-cut loss window of (droppable) diagnostic lines.
const DIAG_BUF_CAP: usize = 8 * 1024;
/// Push past this and the diag side asks for a flush this iteration. With ≤128 B
/// lines and a same-iteration flush, the buffer peaks at `DIAG_HIGH_WATER + one
/// line` (< `DIAG_BUF_CAP`), so a line is never dropped to overflow.
const DIAG_HIGH_WATER: usize = DIAG_BUF_CAP * 3 / 4;
/// Unified periodic flush cadence (seconds). Both the diag log and the CSV rows
/// persist on this interval (matched to the CSV defer window) — the RAM ring gives
/// the live view, so flash only needs to survive a reboot, exactly like CSV. Was
/// 2 s for diag alone, which fired the SSR-pause/flash handshake 60× more than CSV.
const FLUSH_INTERVAL_S: u64 = 120;

/// Build the `diag-NNNNNN.log` name for a suffix.
fn diag_name(suffix: u32) -> heapless::String<24> {
    let mut s = heapless::String::new();
    let _ = core::fmt::Write::write_fmt(&mut s, format_args!("diag-{:06}.log", suffix));
    s
}

/// Parse the numeric suffix from a `diag-NNNNNN.log` name.
fn parse_suffix(name: &str) -> Option<u32> {
    name.strip_prefix("diag-")?
        .strip_suffix(".log")?
        .parse()
        .ok()
}

/// Persist the pending CSV rows and the buffered diag lines in ONE flash-paused +
/// single-mount window (the coalescing point). Diag size accounting advances only
/// on a successful diag write. CSV rows are cleared only on a successful write (so
/// a failed flush retries them — recovery depends on it); the diag buffer is always
/// cleared (diagnostic data, droppable, and clearing prevents it wedging on a
/// failed write).
///
/// All synchronous: no `.await` runs between [`csv_take_into`] and [`csv_clear_pending`],
/// so the CSV task cannot push a row in between and lose it to the clear.
fn flush_all(
    storage: &'static dyn Storage,
    suffix: u32,
    diag_buf: &mut heapless::String<DIAG_BUF_CAP>,
    active_size: &mut u32,
    csv_scratch: &mut heapless::String<CSV_BUF_CAP>,
    csv_name: &mut heapless::String<CSV_NAME_CAP>,
) {
    let has_csv = csv_take_into(csv_scratch, csv_name);
    let write_diag = !diag_buf.is_empty();
    let diag_len = diag_buf.len() as u32;
    let diag_name_s = diag_name(suffix);

    // Scoped so `writes` (which borrows the buffers) is dropped — releasing those
    // borrows — before the post-write `diag_buf.clear()` / accounting below.
    let ok = {
        let mut writes: heapless::Vec<BatchWrite, 2> = heapless::Vec::new();
        if has_csv {
            let _ = writes.push(BatchWrite {
                dir: Directory::Logs,
                name: csv_name.as_str(),
                bytes: csv_scratch.as_bytes(),
                create: false,
            });
        }
        if write_diag {
            let _ = writes.push(BatchWrite {
                dir: Directory::Diag,
                name: diag_name_s.as_str(),
                bytes: diag_buf.as_bytes(),
                create: false,
            });
        }
        !writes.is_empty() && storage.write_batch(&writes).is_ok()
    };

    if ok {
        if has_csv {
            csv_clear_pending();
        }
        if write_diag {
            *active_size += diag_len;
        }
    }
    // Diag lines are diagnostic/droppable: cleared even on a failed write so the
    // buffer can't wedge. CSV rows are kept on failure (above) — never dropped here.
    diag_buf.clear();
}

/// Create a fresh `diag-NNNNNN.log` with a one-line ISO/uptime header (truncating
/// any pre-existing file at that name). Resets `active_size` to 0 first, then to
/// the header length on a successful write — so a FAILED open leaves `active_size`
/// at 0 and cannot make `should_rotate` re-fire every loop and spin the suffix.
fn open_new_diag_file(
    storage: &'static dyn Storage,
    clock: &'static dyn Clock,
    suffix: u32,
    active_size: &mut u32,
) {
    // Reset unconditionally: on a write failure the new file does not exist, so
    // its "active size" is 0 — never the prior file's size (which would re-trigger
    // rotation immediately and spin `suffix` forward).
    *active_size = 0;
    let name = diag_name(suffix);
    let mut header = heapless::String::<64>::new();
    let _ = header.push_str("# diag ");
    match clock.unix_seconds() {
        Some(secs) => {
            let _ = write_iso(&mut header, secs);
        }
        None => {
            let _ = core::fmt::Write::write_fmt(
                &mut header,
                format_args!("uptime+{}s", embassy_time::Instant::now().as_secs()),
            );
        }
    }
    let _ = header.push('\n');
    if storage
        .append(Directory::Diag, &name, header.as_bytes(), true)
        .is_ok()
    {
        *active_size = header.len() as u32;
    }
}

/// How many diag files to keep. The eviction order is diag-first, so beyond a size
/// deficit we *also* force the diag count down to this — bounding the many-small-
/// files pile-up that a size-only target can't reclaim (each boot opens one diag
/// file; a crash/short-boot loop leaves dozens of sub-rotation files whose total
/// size stays under the free target, so a pure size prune never fires). 8 files ≈
/// up to 512 KiB of recent diag retained for post-mortem.
const MAX_DIAG_FILES: usize = 8;

/// Free space down to `target_free` bytes (caller passes the already-known `free`,
/// so this does not re-query) by deleting the most sacrificial files first: all
/// diag files (oldest→newest, **excluding** `active_suffix`'s live file), then the
/// oldest CSV runs (**excluding** the active run — the recovery pointer's file,
/// which the live firing and crash recovery depend on). Independently of the size
/// deficit it also evicts oldest diag down to [`MAX_DIAG_FILES`] (see there). The
/// deletes run as ONE batched, SSR-paused window ([`Storage::remove_batch`]). Best
/// effort: if evicting everything prunable still falls short (e.g. a single huge
/// active run), it frees what it can; the caller's write may then fail and a
/// dropped CSV row is surfaced by [`csv_push_row`].
fn prune_to_free(storage: &dyn Storage, free: u64, target_free: u64, active_suffix: Option<u32>) {
    // 0 when there is headroom — the count cap below can still have work to do.
    let deficit = target_free.saturating_sub(free).min(u32::MAX as u64) as u32;

    // The active run is the protected file (recovery pointer = source of truth).
    let mut abuf = [0u8; CSV_NAME_CAP];
    let alen = storage.read_active_run(&mut abuf).unwrap_or(0);
    let active = core::str::from_utf8(&abuf[..alen]).unwrap_or("").trim();

    // Diag names are tiny (`diag-NNNNNN.log`, ≤14 B), so scan generously — 64 covers
    // any realistic count; a pathological crash-boot pile beyond it is mopped up,
    // with progress, on the next pass. Logs: one CSV per firing, so 32 is ample.
    const MAX_DIAG_SCAN: usize = 64;
    const MAX_LOG_SCAN: usize = 32;
    const MAX_PRUNE_TOTAL: usize = MAX_DIAG_SCAN + MAX_LOG_SCAN;

    let active_diag = active_suffix.map(diag_name);
    let mut diag: heapless::Vec<(heapless::String<16>, u32), MAX_DIAG_SCAN> = heapless::Vec::new();
    storage.for_each(Directory::Diag, &mut |name, size, _modified| {
        if active_diag.as_deref() == Some(name) {
            return; // never evict the file currently being appended
        }
        let mut n = heapless::String::new();
        if n.push_str(name).is_ok() {
            let _ = diag.push((n, size as u32));
        }
    });
    diag.sort_unstable_by(|a, b| a.0.cmp(&b.0)); // zero-padded suffix ⇒ lexical = age

    let mut logs: heapless::Vec<(heapless::String<CSV_NAME_CAP>, u32, u64), MAX_LOG_SCAN> =
        heapless::Vec::new();
    storage.for_each(Directory::Logs, &mut |name, size, modified| {
        if name == active {
            return; // never evict the live / recoverable run
        }
        let mut n = heapless::String::new();
        if n.push_str(name).is_ok() {
            let _ = logs.push((n, size as u32, modified));
        }
    });
    logs.sort_unstable_by_key(|e| e.2); // ascending filename time-key = oldest first

    // Eviction-ordered sizes (diag, then logs) → how many to drop for the deficit.
    let mut sizes: heapless::Vec<u32, MAX_PRUNE_TOTAL> = heapless::Vec::new();
    for (_, s) in diag.iter() {
        let _ = sizes.push(*s);
    }
    for (_, s, _) in logs.iter() {
        let _ = sizes.push(*s);
    }
    // Take the larger of: enough to cover the size deficit, and enough to bring the
    // diag count to MAX_DIAG_FILES. Both count from the front (diag-first), so the
    // count floor is always a valid prefix.
    let size_k = kiln_log::evict_count(&sizes, deficit);
    let count_k = diag.len().saturating_sub(MAX_DIAG_FILES);
    let k = size_k.max(count_k);
    if k == 0 {
        return;
    }

    let mut batch: heapless::Vec<(Directory, &str), MAX_PRUNE_TOTAL> = heapless::Vec::new();
    for (n, _) in diag.iter().take(k) {
        let _ = batch.push((Directory::Diag, n.as_str()));
    }
    if k > diag.len() {
        for (n, _, _) in logs.iter().take(k - diag.len()) {
            let _ = batch.push((Directory::Logs, n.as_str()));
        }
    }
    storage.remove_batch(&batch);
}

/// Core 0 task: the single owner of periodic flash writes. Persists diag lines to
/// `diag/diag-NNNNNN.log` (rotating by size) AND the CSV logger's steady-state
/// rows, coalesced into one flash-paused window per flush. Keeps the filesystem
/// above [`kiln_log::RUN_FREE_TARGET`] free by pruning oldest diag then oldest
/// non-active runs ([`prune_to_free`]) so a write never hits a full disk mid-run;
/// boot reclaims to [`kiln_log::BOOT_FREE_TARGET`]. Wakes on a diag line, a CSV
/// [`request_flush`] nudge, or the [`FLUSH_INTERVAL_S`] timer. Writes no diag while
/// `LOG_TO_FLASH` is false (the drain never forwards then) but still flushes CSV.
#[embassy_executor::task]
pub async fn flash_flush_task(storage: &'static dyn Storage, clock: &'static dyn Clock) -> ! {
    use embassy_time::{with_timeout, Duration, Instant};

    // --- Boot prune + active-file selection ---------------------------------
    // Highest diag suffix on disk: the active file opens at `max+1`, which is fresh
    // and collision-free because prune only ever removes oldest (lower) suffixes.
    let mut max_seen: Option<u32> = None;
    storage.for_each(Directory::Diag, &mut |name, _size, _modified| {
        if let Some(suf) = parse_suffix(name) {
            max_seen = Some(max_seen.map_or(suf, |m| m.max(suf)));
        }
    });
    // Reclaim generously at boot (idle — SSR off, so no flash-handshake cost): drop
    // oldest diag first, then oldest non-active runs, until BOOT_FREE_TARGET is free.
    // No active diag yet (opened below), so nothing is suffix-protected; this is also
    // where the cross-boot diag count cap (MAX_DIAG_FILES) gets applied.
    let free = storage.available_bytes().unwrap_or(u64::MAX);
    prune_to_free(storage, free, kiln_log::BOOT_FREE_TARGET as u64, None);

    let mut suffix = max_seen.map(|m| m + 1).unwrap_or(0);
    let mut active_size: u32 = 0;
    open_new_diag_file(storage, clock, suffix, &mut active_size);

    // --- Flush loop ----------------------------------------------------------
    let mut diag_buf = heapless::String::<DIAG_BUF_CAP>::new();
    let mut csv_scratch = heapless::String::<CSV_BUF_CAP>::new();
    let mut csv_name = heapless::String::<CSV_NAME_CAP>::new();
    // Monotonic anchor for the periodic cadence. The timeout each iteration is the
    // *remaining* time to the next due flush (not a fresh interval), so a steady
    // stream of diag lines — each completing the `select` before the interval — can
    // never starve the periodic flush and strand buffered CSV rows past the window.
    let mut last_flush = Instant::now();
    let interval = Duration::from_secs(FLUSH_INTERVAL_S);

    loop {
        // Wake on: a diag line, a CSV flush nudge, or the remaining interval.
        let remaining = interval
            .checked_sub(Instant::now().duration_since(last_flush))
            .unwrap_or(Duration::from_secs(0));
        let mut flush = false;
        match with_timeout(
            remaining,
            select(FLASH_LOG_CHANNEL.receive(), FLUSH_NOW.wait()),
        )
        .await
        {
            // Interval reached with nothing pending to wake us.
            Err(_) => flush = true,
            // A diag line arrived: buffer it; flush this iteration past high-water.
            // (≤128 B lines + same-iteration flush ⇒ never overflows DIAG_BUF_CAP.)
            Ok(Either::First(line)) => {
                let _ = diag_buf.push_str(&line);
                if diag_buf.len() >= DIAG_HIGH_WATER {
                    flush = true;
                }
            }
            // CSV producer asked to flush (run start / leaving / high-water).
            Ok(Either::Second(())) => flush = true,
        }

        // A line/nudge that landed at or past the window is also due.
        if !flush && Instant::now().duration_since(last_flush) >= interval {
            flush = true;
        }

        if flush {
            // Keep the run target free before writing so an append never hits a full
            // filesystem mid-run. `available_bytes` is a littlefs `lfs_fs_size` full
            // block traverse, not O(1) — but it is read-only (XIP-safe, no SSR pause)
            // and runs at most once per flush (≥120 s apart on Core 0), so the cost is
            // negligible. Hysteresis: prune only once free drops below the low-water
            // TRIGGER, but reclaim all the way to the higher TARGET — so a prune buys a
            // big margin (~14 flushes) instead of firing nearly every flush (which a
            // trigger == target would). The prune (diag-first, then oldest non-active
            // runs) is batched into one SSR-paused window and handed the `free` we just
            // read so it does not re-query. On a query error, u64::MAX ⇒ no prune (fail
            // safe — never delete on uncertainty).
            let free = storage.available_bytes().unwrap_or(u64::MAX);
            if free < kiln_log::RUN_PRUNE_TRIGGER as u64 {
                prune_to_free(storage, free, kiln_log::RUN_FREE_TARGET as u64, Some(suffix));
            }
            flush_all(
                storage,
                suffix,
                &mut diag_buf,
                &mut active_size,
                &mut csv_scratch,
                &mut csv_name,
            );
            last_flush = Instant::now();
            // Wake any run-end path blocked in `flush_and_wait`.
            FLUSH_DONE.signal(());
        }

        // Rotate to a fresh file once the active one is large enough.
        if kiln_log::should_rotate(active_size) {
            suffix += 1;
            open_new_diag_file(storage, clock, suffix, &mut active_size);
        }
    }
}

// === Web endpoints ==========================================================

use picoserve::response::chunked::{ChunkWriter, ChunkedResponse, Chunks, ChunksWritten};
use picoserve::response::IntoResponse;

/// Lazy snapshot body, sent with **chunked transfer encoding** (no
/// `Content-Length`). Two reasons:
/// 1. The `RING_CAP` snapshot buffer is confined to the leaf `write_chunks` future.
///    It is held across the chunk-write awaits, but the serve future is an enum
///    sized to its largest arm (picoserve's ~77 KB request-reader/dispatch branch),
///    and the response-writer arm holding this buffer sits under that max — so it
///    costs no extra per-worker RAM. (A shared static instead would be pure added
///    `.bss`: measured +8 KB. An owned `Content` body, by contrast, WOULD duplicate
///    the copy across picoserve's nested router response future — that is what blows
///    the RAM budget, and chunking avoids it.)
/// 2. The ring is taken in ONE snapshot inside `write_chunks`, so there is no
///    `Content-Length`-vs-body mismatch (a `Content` impl would have to measure
///    and write in two separate snapshots, and a line arriving between them would
///    desync the length from the body and corrupt the keep-alive connection).
struct RingSnapshot;

impl Chunks for RingSnapshot {
    fn content_type(&self) -> &'static str {
        "text/plain; charset=utf-8"
    }

    async fn write_chunks<W: picoserve::io::Write>(
        self,
        mut chunk_writer: ChunkWriter<W>,
    ) -> Result<ChunksWritten, W::Error> {
        let mut buf = [0u8; RING_CAP];
        let n = LOG_RING.lock(|r| r.borrow().snapshot(&mut buf));
        // `write_chunk` is a no-op for an empty slice, so an empty ring just
        // produces the terminating chunk from `finalize`.
        chunk_writer.write_chunk(&buf[..n]).await?;
        chunk_writer.finalize().await
    }
}

/// `GET /api/logs` — plain-text snapshot of the RAM ring (the "what's on screen
/// now" view). The ring keeps records line-aligned and valid UTF-8. A client tails
/// the log by polling this on an interval.
///
/// CORS: unlike the `ApiResponse` endpoints (which add CORS via `respond`), this
/// chunked response must attach the headers itself, or a cross-origin browser
/// (the dev UI on `localhost:3000` hitting the device IP) blocks the GET for a
/// missing `Access-Control-Allow-Origin`. Mirrors `server::web::CORS` (kept local
/// because that const is private to `mod web`).
pub async fn logs_snapshot() -> impl IntoResponse {
    const CORS: [(&str, &str); 3] = [
        ("Access-Control-Allow-Origin", "*"),
        ("Access-Control-Allow-Methods", "GET, POST, PUT, DELETE, OPTIONS"),
        ("Access-Control-Allow-Headers", "Content-Type"),
    ];
    ChunkedResponse::new(RingSnapshot)
        .into_response()
        .with_headers(CORS)
}
