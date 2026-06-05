//! The Core 0 embassy glue: the picoserve router + task set that replaces
//! `server/web_server.py`, `data_logger.py`, the status receiver, and the
//! recovery listener, plus the WiFi/NTP/LCD monitors from `main.py`.
//!
//! VERIFICATION STATUS. Unlike the rest of this crate (host-tested), this module
//! binds `picoserve`/`embassy-net`/`embassy-sync` and compiles only for the
//! target — it is behind the `embassy` feature and never enters the host test
//! build. The split is deliberate: every handler's *decisions* (validation,
//! serialization, parsing, CSV/recovery formatting, command selection) are made
//! by the verified pure modules ([`crate::api`], [`crate::json`], [`crate::csv`],
//! [`crate::profile_json`], [`crate::recovery_io`]); this file only wires them to
//! picoserve and to the device. The remaining device surface — flash, the cyw43
//! chip, the LCD, the wall clock — is reached solely through the [`Storage`],
//! [`Clock`], and [`Display`] traits, whose `embassy-rp`/`cyw43`/littlefs
//! implementations live in `kiln-firmware`; this crate names none of them.
//!
//! Boundaries honoured: cross-core channels use [`CriticalSectionRawMutex`]; the
//! `#[embassy_executor::task]`s are concrete (never generic).

use core::fmt::Write as _;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Channel, Sender};
use embassy_sync::signal::Signal;
use embassy_sync::watch::Watch;
use kiln_core::profile::Profile;
use kiln_core::protocol::{Command, ProfileName, Status};

use crate::api::{self, Directory};
use crate::config::KilnConfig;
use crate::{config, csv, html, json, profile_json};

/// Command-queue depth (Core 0 → Core 1). The reference held 10, but there each
/// slot was a tiny dict reference; here a profile-bearing `Command` (`RunProfile`
/// / `ResumeProfile` / `ScheduleProfile`) embeds a full `Profile` (~2.3 KB), so
/// every slot costs that much *static* RAM. Commands are user actions drained one
/// per control tick (≈1 Hz), so a shallow queue is ample — 3 keeps a small burst
/// margin while cutting the channel from ~24.5 KB to ~7.4 KB.
pub const COMMAND_DEPTH: usize = 3;
/// Latest-status broadcast consumers: web pollers + CSV logger + LCD + recovery.
pub const STATUS_CONSUMERS: usize = 4;
/// picoserve worker pool size for the *primary* interface (WiFi STA) — the
/// reference's `MAX_CONCURRENT_CONNECTIONS`.
pub const WEB_TASK_POOL_SIZE: usize = api::MAX_CONCURRENT_CONNECTIONS;

/// Web workers per *secondary* interface (USB-NCM, fallback SoftAP). Provisioning
/// and USB file access are single-user, so one connection each is enough — and a
/// picoserve worker future is ~84 KB, so each extra worker is the dominant RAM
/// cost (see the RAM notes in [[rust-reliability-sprint]]).
pub const SECONDARY_WEB_WORKERS: usize = 1;

/// Total `web_task` instances that can be live at once = the macro `pool_size`.
/// Worst case is configured boot: STA (`WEB_TASK_POOL_SIZE`) + USB-NCM
/// (`SECONDARY_WEB_WORKERS`). Unconfigured boot uses fewer (SoftAP 1 + NCM 1).
pub const WEB_TASK_POOL_TOTAL: usize = WEB_TASK_POOL_SIZE + SECONDARY_WEB_WORKERS;

/// Core 0 → Core 1 command channel (typed [`Command`], no heap).
pub type CommandChannel = Channel<CriticalSectionRawMutex, Command, COMMAND_DEPTH>;
/// Core 1 → Core 0 latest-status broadcast (`Watch` keeps the latest value, as
/// the reference's status cache is "latest wins").
pub type StatusWatch = Watch<CriticalSectionRawMutex, Status, STATUS_CONSUMERS>;
/// The sender half handed to web handlers.
pub type CommandSender = Sender<'static, CriticalSectionRawMutex, Command, COMMAND_DEPTH>;
/// One-shot reboot request (`/api/reboot`); the firmware waits on it then resets.
pub type RebootSignal = Signal<CriticalSectionRawMutex, ()>;

/// Wall-clock source — real Unix seconds. The control loop runs on monotonic
/// `embassy-time`; this is the NTP-disciplined wall clock (`sntpc` updates an
/// offset in the firmware) used only for status timestamps, CSV rows, and log
/// filenames. Mirrors the reference's `time.time()`.
pub trait Clock {
    /// Current Unix time in seconds, or `None` before the first NTP sync.
    fn unix_seconds(&self) -> Option<i64>;
}

/// Opaque storage failure (absent file, out of space, I/O error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageError;

/// The flash filesystem (`profiles/`, `logs/`) and the compiled-in static assets,
/// behind a trait so this crate never names littlefs. Methods are synchronous —
/// littlefs is blocking and flash reads are fast, exactly as the reference's
/// `open`/`os` calls are blocking.
pub trait Storage {
    /// Read up to `buf.len()` bytes of `dir/name` from `offset`, returning the
    /// count (0 at EOF). Streams downloads and reads small files whole.
    fn read_chunk(
        &self,
        dir: Directory,
        name: &str,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<usize, StorageError>;
    /// Size of `dir/name`, or `None` if absent.
    fn size(&self, dir: Directory, name: &str) -> Option<u64>;
    /// Invoke `f(name, size, modified)` for each entry of `dir`.
    fn for_each(&self, dir: Directory, f: &mut dyn FnMut(&str, u64, u64));
    /// Remove `dir/name`.
    fn remove(&self, dir: Directory, name: &str) -> Result<(), StorageError>;
    /// Remove every file in `dir` (logs bulk delete).
    fn remove_all(&self, dir: Directory) -> Result<(), StorageError>;
    /// Append `bytes` to `dir/name`; when `create` is set, truncate first (a new
    /// run's header). Used by the CSV logger.
    fn append(
        &self,
        dir: Directory,
        name: &str,
        bytes: &[u8],
        create: bool,
    ) -> Result<(), StorageError>;
    /// Begin a streamed upload to a scratch file (clears any prior scratch).
    fn upload_begin(&self) -> Result<(), StorageError>;
    /// Append a streamed-upload chunk.
    fn upload_write(&self, bytes: &[u8]) -> Result<(), StorageError>;
    /// Atomically move the completed scratch upload to `dir/name`.
    fn upload_commit(&self, dir: Directory, name: &str) -> Result<(), StorageError>;
    /// Discard an aborted/partial upload.
    fn upload_abort(&self);
    /// A compiled-in static asset (`index.html`, …), if present.
    fn static_asset(&self, name: &str) -> Option<&'static [u8]>;
    /// Read the whole `config.json` root file into `buf`, returning the byte
    /// count (0 if absent). A dedicated path, not a [`Directory`] entry, so the
    /// public `/api/files` routes can never address or overwrite it.
    fn read_config(&self, buf: &mut [u8]) -> Result<usize, StorageError>;
    /// Overwrite `config.json` atomically (the `POST /api/config` persist).
    fn write_config(&self, bytes: &[u8]) -> Result<(), StorageError>;
    /// Read the `active_run` pointer (the live firing's log filename) into `buf`,
    /// returning the byte count (0 if absent). A dedicated root path, not a
    /// [`Directory`] entry, so `/api/files` can never touch it. This is the
    /// authoritative "which run was firing at power loss" marker crash recovery
    /// reads — independent of any wall-clock/filename timestamp.
    fn read_active_run(&self, buf: &mut [u8]) -> Result<usize, StorageError>;
    /// Record the live firing's log filename (atomic). Set on the IDLE→RUNNING
    /// edge by the CSV logger.
    fn write_active_run(&self, name: &[u8]) -> Result<(), StorageError>;
    /// Clear the pointer (idempotent if absent). Written on a clean run end, and
    /// on a *declined* recovery so a stale interrupted run is considered at most
    /// once and can never be auto-resumed on a later boot.
    fn clear_active_run(&self);
}

/// The character LCD status line (`main.py`'s monitor), behind a trait so the
/// driver stays in the firmware.
pub trait Display {
    /// Render the two-line summary for `status`.
    fn show(&mut self, status: &Status);
}

/// Shared handler state. All fields are `Copy`, so `AppState: Copy` and
/// picoserve's blanket `FromRef<Self> for Self` applies.
#[derive(Clone, Copy)]
pub struct AppState {
    pub commands: CommandSender,
    pub status: &'static StatusWatch,
    pub clock: &'static dyn Clock,
    pub storage: &'static dyn Storage,
    pub reboot: &'static RebootSignal,
    /// The boot-time config, served by `GET /api/config` and used as the merge
    /// base for `POST /api/config` (read-only after boot; edits apply on reboot).
    pub config: &'static KilnConfig,
}

impl AppState {
    /// The latest published status, or the idle default before Core 1's first
    /// publish (the reference seeds its cache with the idle template).
    pub fn latest(&self) -> Status {
        self.status.try_get().unwrap_or_else(Status::idle)
    }

    /// Current Unix time, or 0 before NTP sync (matching the reference, which
    /// formats whatever `time.time()` returns).
    fn now(&self) -> i64 {
        self.clock.unix_seconds().unwrap_or(0)
    }

    /// Whether the wall clock has synced at least once. Run-triggering endpoints
    /// gate on this so every firing/tuning log gets a real dated filename (the
    /// kiln is reachable only over WiFi, so NTP is available by then anyway).
    fn clock_synced(&self) -> bool {
        self.clock.unix_seconds().is_some()
    }
}

/// Read `dir/name` whole into `buf` with the reference's transient-glitch retry
/// (`control_thread.load_profile_with_retry`: 3 attempts, 0.5 s/1.0 s backoff),
/// returning the byte count. `None` only if every attempt failed — a single
/// flash read glitch no longer aborts a run/recovery. The caller checks
/// existence first, so a genuinely-absent file is a fast 404, not 3 retries.
pub async fn read_file_with_retry(
    storage: &dyn Storage,
    dir: Directory,
    name: &str,
    buf: &mut [u8],
) -> Option<usize> {
    const MAX_ATTEMPTS: u32 = 3;
    for attempt in 0..MAX_ATTEMPTS {
        if let Ok(n) = storage.read_chunk(dir, name, 0, buf) {
            return Some(n);
        }
        if attempt + 1 < MAX_ATTEMPTS {
            // Exponential backoff: 0.5 s, then 1.0 s (`0.5 * (attempt + 1)`).
            embassy_time::Timer::after(embassy_time::Duration::from_millis(
                500 * (attempt as u64 + 1),
            ))
            .await;
        }
    }
    None
}

// === Tasks ==================================================================

/// A crash-recovery resume hand-off for the CSV logger: the *existing* log file
/// to continue (append to, no new header) and the resume elapsed for the one-shot
/// `RECOVERY` event row. Built by `kiln-firmware`'s `attempt_recovery` and passed
/// once into [`csv_logger_task`]; `None` for a normal boot. Mirrors
/// `data_logger.set_recovery_context` + `log_recovery_event`.
pub struct RecoveryLog {
    /// The interrupted run's log filename (no directory prefix).
    pub filename: heapless::String<96>,
    /// `recovery_info.elapsed_seconds` — the elapsed written in the RECOVERY row.
    pub elapsed_seconds: f32,
}

/// Defer the flash write of buffered CSV rows by at most this long (monotonic
/// seconds). One flash program per *flush* instead of one per *row* — far fewer
/// flash-pause events (each one de-energises the SSR and busy-spins Core 0; see
/// the flash handshake), which matters most during TUNING (a row every 2 s) — and
/// far less flash wear. Trade-off: a power cut loses up to this much logged data.
/// That is safe: crash recovery resumes from the last *flushed* row, and its
/// safety gate is the current-vs-logged **temperature delta**, not the exact
/// elapsed (`kiln_core::recovery`), so a slightly stale resume still gates
/// correctly and the controller re-derives the step from temp.
const CSV_FLUSH_INTERVAL_S: u64 = 120;
/// One rendered CSV row's upper bound (matches the per-row render `String`).
const CSV_ROW_CAP: usize = 256;
/// RAM accumulator for un-flushed rows. Sized to hold one flush interval at the
/// fastest cadence (TUNING: 120 s / 2 s = 60 rows × ~130 B ≈ 7.8 KB) with margin;
/// a row that would overflow forces an early flush, so nothing is ever dropped.
const CSV_BUF_CAP: usize = 8192;

/// CSV logging — the `data_logger.py` half that owns timing and the file handle.
/// Subscribes to the status broadcast and writes a row through [`Storage`] when
/// the interval has elapsed (`LOGGING_INTERVAL` normally, 2 s while TUNING), using
/// the verified [`csv`] formatters. On the IDLE→RUNNING/TUNING edge it starts a
/// new file (header) — or, for a crash-recovery resume, **appends** to the
/// interrupted run's file (no header) and writes a one-shot `RECOVERY` event row
/// (`data_logger.log_recovery_event`). Forces a final terminal-state row on the
/// way out — `data_logger.update`.
///
/// Also owns the `active_run` recovery pointer (see [`Storage::write_active_run`]):
/// it records the live firing's log filename on the IDLE→RUNNING edge and clears
/// it on a clean exit, so crash recovery can identify *which* run was interrupted
/// without depending on log-filename timestamps. Tuning runs do not set it (they
/// are not recovery-eligible).
#[embassy_executor::task]
pub async fn csv_logger_task(
    status: &'static StatusWatch,
    storage: &'static dyn Storage,
    clock: &'static dyn Clock,
    config: &'static KilnConfig,
    mut recovery: Option<RecoveryLog>,
) -> ! {
    use embassy_time::Instant;
    use kiln_core::state::KilnState;
    let mut rx = status.receiver().unwrap();
    let mut logging = false;
    let mut filename = heapless::String::<96>::new();
    let mut last_log: i64 = 0;
    // Rows accumulate here and flush to flash in batches (see CSV_FLUSH_INTERVAL_S).
    let mut buf = heapless::String::<CSV_BUF_CAP>::new();
    // Monotonic, so the flush cadence holds even before NTP sync. `None` until the
    // first flush of a run, which is taken promptly so the file gains a RUNNING
    // row early and recovery stays possible from near the start.
    let mut last_flush: Option<Instant> = None;
    let mut prev_state = KilnState::Idle;

    loop {
        let s = rx.changed().await;
        let now = clock.unix_seconds().unwrap_or(0);
        let active = matches!(s.state, KilnState::Running | KilnState::Tuning);

        if active && !logging {
            buf.clear();
            if let Some(rec) = recovery.take() {
                // Recovery resume: continue the interrupted run's file (append,
                // no header) and write the one-shot RECOVERY marker row using the
                // resume elapsed + live temps/SSR/rate (data_logger.py 201-264).
                // Written straight through (not buffered) so the marker is durable.
                filename.clear();
                let _ = filename.push_str(&rec.filename);
                let mut row = heapless::String::<CSV_ROW_CAP>::new();
                let _ = csv::write_recovery_event_row(
                    &mut row,
                    s.timestamp,
                    rec.elapsed_seconds,
                    s.current_temp,
                    s.target_temp,
                    s.ssr_output,
                    s.measured_rate,
                );
                if storage
                    .append(Directory::Logs, &filename, row.as_bytes(), false)
                    .is_ok()
                {
                    logging = true;
                    last_log = 0;
                    last_flush = None;
                }
            } else {
                // Pick the log stem: a fixed "tuning" while TUNING (the
                // controller sets no profile_name then, so a profile-only branch
                // would log nothing and `analyze_tuning.py` would have no data),
                // else the running profile's filename. Mirrors
                // `data_logger.on_status_update`'s two start-logging branches. The
                // header is written straight through (create + truncate) so the
                // file exists immediately; rows then batch into `buf`.
                let stem = if s.state == KilnState::Tuning {
                    Some("tuning")
                } else {
                    s.profile_name.as_ref().map(|n| n.as_str())
                };
                if let Some(stem) = stem {
                    filename.clear();
                    let _ = csv::write_log_filename(&mut filename, stem, now);
                    if storage
                        .append(Directory::Logs, &filename, csv::HEADER.as_bytes(), true)
                        .is_ok()
                    {
                        logging = true;
                        last_log = 0;
                        last_flush = None;
                        // Mark this as the live firing for crash recovery — but
                        // only a real RUN; tuning is not recovery-eligible.
                        if s.state == KilnState::Running {
                            let _ = storage.write_active_run(filename.as_bytes());
                        }
                    }
                }
            }
        }

        if logging {
            let interval = if s.state == KilnState::Tuning {
                2
            } else {
                config.logging_interval as i64
            };
            let leaving = matches!(prev_state, KilnState::Running | KilnState::Tuning) && !active;

            // Accumulate a row at the logging cadence (and a final row on the way
            // out). Cadence/granularity are unchanged — only the flash write is
            // deferred. If the next row would overflow the buffer, flush first so
            // a row is never dropped.
            if leaving || (now - last_log) >= interval {
                let mut row = heapless::String::<CSV_ROW_CAP>::new();
                let _ = csv::write_row(&mut row, &s);
                if buf.len() + row.len() > buf.capacity()
                    && storage
                        .append(Directory::Logs, &filename, buf.as_bytes(), false)
                        .is_ok()
                {
                    buf.clear();
                    last_flush = Some(Instant::now());
                }
                let _ = buf.push_str(&row);
                last_log = now;
            }

            // Flush the batch on the way out, or once the defer window elapses.
            // `last_flush == None` (the first batch of a run) flushes promptly so
            // the file holds a RUNNING row early and recovery stays possible.
            let due = match last_flush {
                None => true,
                Some(t) => Instant::now().duration_since(t).as_secs() >= CSV_FLUSH_INTERVAL_S,
            };
            if !buf.is_empty()
                && (leaving || due)
                && storage
                    .append(Directory::Logs, &filename, buf.as_bytes(), false)
                    .is_ok()
            {
                buf.clear();
                last_flush = Some(Instant::now());
            }

            if leaving {
                logging = false;
                // Clean run/tuning end → no interrupted run to recover. Clearing
                // here (and on a declined recovery) is what stops a stale run from
                // ever being auto-resumed on a later boot. Idempotent if absent.
                storage.clear_active_run();
            }
        }
        prev_state = s.state;
    }
}

/// LCD task — the `main.py` status-line monitor; renders the latest status on
/// each change through the firmware-provided [`Display`].
#[embassy_executor::task]
pub async fn lcd_task(status: &'static StatusWatch, display: &'static mut dyn Display) -> ! {
    let mut rx = status.receiver().unwrap();
    loop {
        let s = rx.changed().await;
        display.show(&s);
    }
}

// === picoserve router =======================================================
//
// `web` holds the picoserve-specific wiring (the device-verification surface).
// The route set mirrors `web_server.py`'s `ROUTES` 1:1; each handler delegates
// its decisions to the verified modules.

mod web {
    // `ApiResponse` (the handler response type) is also used as the early-return
    // error in `load_profile`/`file_guard`, so several `Result`s have a large
    // `Err` variant. That is intentional — it is the response, and boxing it is
    // not free in `no_std` — so the `result_large_err` lint is silenced here.
    #![allow(clippy::result_large_err)]

    use super::*;
    use picoserve::extract::State;
    use picoserve::response::chunked::{ChunkWriter, ChunkedResponse, Chunks, ChunksWritten};
    use picoserve::response::{Content, IntoResponse, StatusCode};
    use picoserve::routing::{get, parse_path_segment, post};

    /// Cap on a buffered command/JSON body (`MAX_JSON_BODY`); profiles are small.
    const BODY_CAP: usize = api::MAX_JSON_BODY;
    /// Working buffer for reading a profile file to parse. Profiles are small —
    /// the shipped ones are <1 KB and a full `MAX_STEPS` (16) profile is ~1.5 KB —
    /// so 2 KiB is 2-3× headroom. This buffer lives on the stack of `load_profile`,
    /// which is held across an await inside every web-worker future, so its size
    /// is multiplied by the worker pool (web RAM).
    const PROFILE_READ_CAP: usize = 2048;
    /// Max profiles rendered into the index `{profiles_list}` / collected for a
    /// bulk log delete. The flash holds far fewer; extras are silently dropped.
    const MAX_PROFILES: usize = 32;
    /// Max length of a profile/log filename held on the stack.
    const NAME_CAP: usize = 64;
    /// Packed-arena byte cap for the streamed `/api/files` listing, and the max
    /// entries it holds. Sized ≈ the old single-`String` build so peak RAM is
    /// unchanged, but the body is streamed and always closes as valid JSON.
    /// Entries past either bound are dropped at an object boundary.
    const FILE_LIST_ARENA: usize = 2048;
    const FILE_LIST_MAX: usize = 48;
    /// Upper bound on one rendered `{"name":..,"size":..,"modified":..}` object:
    /// a `NAME_CAP` (64) name + two 20-digit `u64`s + ~32 bytes of framing.
    const FILE_ENTRY_CAP: usize = 160;
    /// Cap on a dynamic success-toast message (`{"success":true,"message":"…"}`).
    /// Owned inside the tiny `ApiResponse::Message`; the longest is
    /// `"Started profile: <name>"`. Held per route slot, so kept small.
    const MSG_CAP: usize = 96;
    /// Per-response JSON render buffer used inside `write_to`. picoserve's serve
    /// future holds several of these un-overlapped (the recursive router + the
    /// `Response`/`Typed` move chain), so the size is multiplied across the worker.
    /// The largest rendered body measured is the config (~1 KB; status ~0.7 KB), so
    /// 1.5 KiB keeps a safe margin while trimming the chain.
    const RENDER_CAP: usize = 1536;

    /// Router-construction props implementing picoserve 0.18's [`AppBuilder`].
    /// Using the trait (rather than a bare module-level `type Foo = impl ...`)
    /// gives the opaque router type a single defining use inside `build_app`, so
    /// the `#[embassy_executor::task]` [`web_task`] — which must name the router
    /// type — does not form a type-resolution cycle with the alias. The shared
    /// [`AppState`] is baked in via `with_state`, leaving the stateless
    /// `Router<P>` that picoserve 0.18's `Server` serves.
    pub struct AppProps {
        pub state: AppState,
    }

    impl picoserve::AppBuilder for AppProps {
        type PathRouter = impl picoserve::routing::PathRouter;

        // The path set and methods match `web_server.py`. Every `/api/*` route
        // also answers `OPTIONS` with 200 + CORS so the browser's cross-origin
        // preflight succeeds — the reference returned 200/CORS for any OPTIONS
        // (`web_server.py:780-782`); without it the hosted web app's
        // POST/PUT/DELETE-with-JSON would fail at preflight.
        fn build_app(self) -> picoserve::Router<Self::PathRouter> {
            picoserve::Router::new()
                .route("/", get(page_index))
                .route("/index.html", get(page_index))
                .route("/tuning", get(page_tuning))
                .route("/tuning.html", get(page_tuning))
                .route("/api/status", get(status_json).options(cors_preflight))
                .route(
                    "/api/tuning/status",
                    get(status_json).options(cors_preflight),
                )
                .route(
                    "/api/scheduled",
                    get(scheduled_json).options(cors_preflight),
                )
                .route(
                    "/api/stop",
                    post(|s: State<AppState>| async move {
                        enqueue(&s.0, Command::Stop, "Profile stopped")
                    })
                    .options(cors_preflight),
                )
                .route(
                    "/api/clear-error",
                    post(|s: State<AppState>| async move {
                        enqueue(&s.0, Command::ClearError, "Error cleared, returned to idle")
                    })
                    .options(cors_preflight),
                )
                .route(
                    "/api/shutdown",
                    post(|s: State<AppState>| async move {
                        enqueue(
                            &s.0,
                            Command::Shutdown,
                            "System shutdown: SSR off, program stopped",
                        )
                    })
                    .options(cors_preflight),
                )
                .route(
                    "/api/scheduled/cancel",
                    post(|s: State<AppState>| async move {
                        enqueue(
                            &s.0,
                            Command::CancelScheduled,
                            "Cancelled scheduled profile",
                        )
                    })
                    .options(cors_preflight),
                )
                .route(
                    "/api/tuning/stop",
                    post(|s: State<AppState>| async move {
                        enqueue(&s.0, Command::StopTuning, "Tuning stopped")
                    })
                    .options(cors_preflight),
                )
                .route("/api/reboot", post(reboot).options(cors_preflight))
                .route(
                    "/api/config",
                    get(config_get).post(config_post).options(cors_preflight),
                )
                .route("/api/run", post(run).options(cors_preflight))
                .route("/api/schedule", post(schedule).options(cors_preflight))
                .route(
                    "/api/tuning/start",
                    post(tuning_start).options(cors_preflight),
                )
                .route(
                    ("/api/files", parse_path_segment()),
                    get(files_list).options(cors_preflight_dir),
                )
                .route(
                    ("/api/files", parse_path_segment(), parse_path_segment()),
                    get(file_get)
                        .put(file_put)
                        .delete(file_delete)
                        .options(cors_preflight_file),
                )
                // Bake the shared state in: picoserve 0.18 serves a stateless
                // `Router<P>`, with the state carried inside the router itself.
                .with_state(self.state)
        }
    }

    /// The concrete router type, named so it can live in a `StaticCell` and so
    /// [`web_task`] stays non-generic (an `#[embassy_executor::task]` cannot be
    /// generic). `picoserve::AppRouter<Props>` resolves to `Router<P, ()>`.
    pub type AppRouter = picoserve::AppRouter<AppProps>;

    /// Build the picoserve router with `state` baked in.
    pub fn make_app(state: AppState) -> AppRouter {
        use picoserve::AppBuilder as _;
        AppProps { state }.build_app()
    }

    /// CORS preflight responder: 200 with an empty body; the CORS headers are
    /// added by [`ApiResponse`]'s `IntoResponse` (the `Text` arm), matching the
    /// reference's blanket OPTIONS handler. picoserve 0.18 requires a method
    /// handler to accept the route's path parameters, so the three arities get
    /// distinct (parameter-ignoring) entry points.
    async fn cors_preflight() -> impl IntoResponse {
        ApiResponse::error_text(StatusCode::OK, "")
    }

    async fn cors_preflight_dir(_dir: heapless::String<16>) -> impl IntoResponse {
        ApiResponse::error_text(StatusCode::OK, "")
    }

    async fn cors_preflight_file(
        _params: (heapless::String<16>, heapless::String<64>),
    ) -> impl IntoResponse {
        ApiResponse::error_text(StatusCode::OK, "")
    }

    // The JSON-body responses carry only their *inputs*; the bytes are rendered in
    // `ApiResponse::write_to` (a single future → one buffer per connection, not one
    // per route). See the `ApiResponse` enum.
    async fn status_json(State(state): State<AppState>) -> impl IntoResponse {
        ApiResponse::Status(state)
    }

    async fn scheduled_json(State(state): State<AppState>) -> impl IntoResponse {
        ApiResponse::Scheduled(state)
    }

    /// Enqueue a typed command, returning the reference's
    /// `{"success":true,"message":"…"}` envelope (200), or the static
    /// queue-full 500 — `_send_command`. The per-command `ok_message` matches
    /// the reference's `_send_command(... ok_message ...)` text so the web app's
    /// success toast reads identically.
    fn enqueue(state: &AppState, cmd: Command, ok_message: &str) -> ApiResponse {
        if state.commands.try_send(cmd).is_ok() {
            let mut msg = heapless::String::<MSG_CAP>::new();
            let _ = msg.push_str(ok_message); // owns the (possibly dynamic) toast text
            ApiResponse::Message(msg)
        } else {
            ApiResponse::static_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                "{\"success\":false,\"error\":\"Command queue full, please retry\"}",
            )
        }
    }

    async fn reboot(State(state): State<AppState>) -> impl IntoResponse {
        state.reboot.signal(());
        ApiResponse::static_json(
            StatusCode::OK,
            "{\"success\":true,\"message\":\"Rebooting Pico...\"}",
        )
    }

    async fn config_get(State(state): State<AppState>) -> impl IntoResponse {
        ApiResponse::Config(state)
    }

    async fn config_post(State(state): State<AppState>, body: JsonBody) -> impl IntoResponse {
        let merged = match config::parse_over(state.config.clone(), body.as_str()) {
            Ok(c) => c,
            Err(_) => return ApiResponse::error(StatusCode::BAD_REQUEST, "Invalid configuration"),
        };
        let mut canonical = heapless::String::<2048>::new();
        if merged.write_json(&mut canonical).is_err() {
            return ApiResponse::error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Configuration too large",
            );
        }
        if state.storage.write_config(canonical.as_bytes()).is_ok() {
            ApiResponse::static_json(
                StatusCode::OK,
                "{\"success\":true,\"message\":\"Config saved. Reboot to apply.\"}",
            )
        } else {
            ApiResponse::error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to save config")
        }
    }

    async fn run(State(state): State<AppState>, body: JsonBody) -> impl IntoResponse {
        if !state.clock_synced() {
            return ApiResponse::error(StatusCode::SERVICE_UNAVAILABLE, api::CLOCK_NOT_SYNCED_MESSAGE);
        }
        let name = match api::json_get_str(body.as_str(), "profile") {
            Some(n) if !n.is_empty() => n,
            _ => return ApiResponse::error(StatusCode::BAD_REQUEST, "Profile name required"),
        };
        match load_profile(&state, name).await {
            Ok((profile, parsed)) => {
                let mut msg = heapless::String::<96>::new();
                let _ = write!(msg, "Started profile: {}", name);
                enqueue(&state, Command::RunProfile { profile, parsed }, &msg)
            }
            Err(resp) => resp,
        }
    }

    async fn schedule(State(state): State<AppState>, body: JsonBody) -> impl IntoResponse {
        if !state.clock_synced() {
            return ApiResponse::error(StatusCode::SERVICE_UNAVAILABLE, api::CLOCK_NOT_SYNCED_MESSAGE);
        }
        let name = api::json_get_str(body.as_str(), "profile");
        let start = api::json_get_f64(body.as_str(), "start_time");
        if !api::schedule_fields_present(name, start) {
            return ApiResponse::error(StatusCode::BAD_REQUEST, "profile and start_time required");
        }
        let (name, start) = (name.unwrap(), start.unwrap());
        if !api::start_time_in_future(start as i64, state.now()) {
            return ApiResponse::error(StatusCode::BAD_REQUEST, "start_time must be in the future");
        }
        match load_profile(&state, name).await {
            Ok((profile, parsed)) => {
                let mut msg = heapless::String::<96>::new();
                let _ = write!(msg, "Scheduled profile: {}", name);
                enqueue(
                    &state,
                    Command::ScheduleProfile {
                        profile,
                        parsed,
                        start_time: start as u64,
                    },
                    &msg,
                )
            }
            Err(resp) => resp,
        }
    }

    async fn tuning_start(State(state): State<AppState>, body: JsonBody) -> impl IntoResponse {
        if !state.clock_synced() {
            return ApiResponse::error(StatusCode::SERVICE_UNAVAILABLE, api::CLOCK_NOT_SYNCED_MESSAGE);
        }
        let mode_str = api::json_get_str(body.as_str(), "mode").unwrap_or("STANDARD");
        let mode = match api::parse_tuning_mode(mode_str) {
            Some(m) => m,
            None => return ApiResponse::error(StatusCode::BAD_REQUEST, api::INVALID_MODE_MESSAGE),
        };
        let max_temp = api::json_get_f64(body.as_str(), "max_temp");
        if !api::max_temp_valid(max_temp) {
            return ApiResponse::error(StatusCode::BAD_REQUEST, api::MAX_TEMP_RANGE_MESSAGE);
        }
        let mut msg = heapless::String::<64>::new();
        let _ = write!(msg, "Tuning started in {} mode", mode_str);
        enqueue(
            &state,
            Command::StartTuning {
                mode,
                max_temp: max_temp.map(|m| m as f32),
            },
            &msg,
        )
    }

    /// Read `profiles/{name}.json` and parse it (Core 0 owns the FS and ships the
    /// `parsed` profile), reproducing run/schedule's not-found → 404 and
    /// parse-error → 400 paths.
    async fn load_profile(
        state: &AppState,
        name: &str,
    ) -> Result<(ProfileName, Profile), ApiResponse> {
        let mut path = heapless::String::<80>::new();
        if write!(path, "{}.json", name).is_err() {
            return Err(ApiResponse::error(
                StatusCode::BAD_REQUEST,
                "Profile name too long",
            ));
        }
        if state.storage.size(Directory::Profiles, &path).is_none() {
            return Err(ApiResponse::error(
                StatusCode::NOT_FOUND,
                "Profile not found",
            ));
        }
        let mut buf = [0u8; PROFILE_READ_CAP];
        // The read retries transient flash glitches (load_profile_with_retry);
        // existence was just checked, so a `None` here is a hard read failure.
        let n = read_file_with_retry(state.storage, Directory::Profiles, &path, &mut buf)
            .await
            .ok_or_else(|| ApiResponse::error(StatusCode::NOT_FOUND, "Profile not found"))?;
        let text = core::str::from_utf8(&buf[..n]).map_err(|_| {
            ApiResponse::error(StatusCode::BAD_REQUEST, "Profile is not valid UTF-8")
        })?;
        let parsed = profile_json::parse_profile(text)
            .map_err(|_| ApiResponse::error(StatusCode::BAD_REQUEST, "Invalid profile"))?;
        let profile = ProfileName::new(&path)
            .map_err(|_| ApiResponse::error(StatusCode::BAD_REQUEST, "Profile name too long"))?;
        Ok((profile, parsed))
    }

    /// Shared file-op preconditions (`_file_guard`): IDLE (403), valid dir (400),
    /// and — when `file` is given — a safe filename (400).
    fn file_guard(
        state: &AppState,
        dir: &str,
        file: Option<&str>,
    ) -> Result<Directory, ApiResponse> {
        if !api::file_ops_allowed(state.latest().state) {
            return Err(ApiResponse::error(
                StatusCode::FORBIDDEN,
                "File operations not allowed while kiln is active. Stop the kiln first.",
            ));
        }
        let directory = Directory::parse(dir).ok_or_else(|| {
            ApiResponse::error(
                StatusCode::BAD_REQUEST,
                "Invalid directory. Must be 'profiles' or 'logs'",
            )
        })?;
        if let Some(f) = file {
            if !api::safe_filename(f) {
                return Err(ApiResponse::error(
                    StatusCode::BAD_REQUEST,
                    "Invalid filename",
                ));
            }
        }
        Ok(directory)
    }

    async fn files_list(
        dir: heapless::String<16>,
        State(state): State<AppState>,
    ) -> impl IntoResponse {
        // Guard here (cheap); the directory scan + streaming run in `write_to`.
        match file_guard(&state, &dir, None) {
            Ok(directory) => ApiResponse::FileList(state, directory),
            Err(resp) => resp,
        }
    }

    async fn file_get(
        (dir, file): (heapless::String<16>, heapless::String<64>),
        State(state): State<AppState>,
    ) -> impl IntoResponse {
        let directory = match file_guard(&state, &dir, Some(&file)) {
            Ok(d) => d,
            Err(resp) => return resp,
        };
        match state.storage.size(directory, &file) {
            Some(size) => ApiResponse::Download(Download {
                storage: state.storage,
                dir: directory,
                name: file,
                size,
            }),
            None => ApiResponse::error_text(StatusCode::NOT_FOUND, "File not found"),
        }
    }

    async fn file_put(
        (dir, file): (heapless::String<16>, heapless::String<64>),
        State(state): State<AppState>,
        upload: Upload,
    ) -> impl IntoResponse {
        let directory = match file_guard(&state, &dir, Some(&file)) {
            Ok(d) => d,
            Err(resp) => {
                state.storage.upload_abort();
                return resp;
            }
        };
        match upload.outcome {
            UploadOutcome::Ok(written) => {
                if state.storage.upload_commit(directory, &file).is_ok() {
                    // `{success, message, filename, size}` — the reference's upload
                    // envelope (the web app's ProfileEditor reads `filename`/`size`).
                    // Rendered in `write_to`; we carry only the name + byte count.
                    ApiResponse::Uploaded {
                        name: file,
                        written,
                    }
                } else {
                    ApiResponse::error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to write file")
                }
            }
            UploadOutcome::Missing => {
                ApiResponse::error(StatusCode::BAD_REQUEST, "Missing or invalid Content-Length")
            }
            UploadOutcome::TooLarge => {
                ApiResponse::error(StatusCode::PAYLOAD_TOO_LARGE, "File too large")
            }
            UploadOutcome::Failed => {
                ApiResponse::error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to write file")
            }
        }
    }

    async fn file_delete(
        (dir, file): (heapless::String<16>, heapless::String<64>),
        State(state): State<AppState>,
    ) -> impl IntoResponse {
        // The web app's "delete all logs" maps to DELETE /api/files/logs/all
        // (`handle_api_files_delete_all`). Route it before the single-file path so
        // "all" is never treated as a filename.
        if file == "all" {
            return file_delete_all(&state, &dir);
        }
        let directory = match file_guard(&state, &dir, Some(&file)) {
            Ok(d) => d,
            Err(resp) => return resp,
        };
        if state.storage.remove(directory, &file).is_ok() {
            ApiResponse::static_json(StatusCode::OK, "{\"success\":true}")
        } else {
            ApiResponse::error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete file")
        }
    }

    /// `DELETE /api/files/logs/all` — bulk-delete every log (`web_server.py`
    /// `handle_api_files_delete_all`). Idle-gated and logs-only. Returns
    /// `{success, deleted_count, deleted_files:[...]}`; the listing (capped at
    /// `MAX_PROFILES`) is collected, `remove_all` run, and the result streamed —
    /// all in `write_to`, so the names list never bloats the per-route response.
    fn file_delete_all(state: &AppState, dir: &str) -> ApiResponse {
        let directory = match file_guard(state, dir, None) {
            Ok(d) => d,
            Err(resp) => return resp,
        };
        if !api::bulk_delete_allowed(directory) {
            return ApiResponse::error(
                StatusCode::FORBIDDEN,
                "Bulk delete only allowed for logs directory",
            );
        }
        ApiResponse::DeleteAll(*state, directory)
    }

    async fn page_index(State(state): State<AppState>) -> impl IntoResponse {
        // The asset fetch, `{profiles_list}` placeholder split, and profile-list
        // collection all run in `write_to` (one buffer per connection).
        ApiResponse::Index(state)
    }

    async fn page_tuning(State(state): State<AppState>) -> impl IntoResponse {
        serve_asset(&state, "tuning.html")
    }

    fn serve_asset(state: &AppState, name: &str) -> ApiResponse {
        match state.storage.static_asset(name) {
            Some(bytes) => ApiResponse::Asset {
                bytes,
                content_type: "text/html",
            },
            None => ApiResponse::error_text(StatusCode::NOT_FOUND, "Not found"),
        }
    }

    fn content_type_for(name: &str) -> &'static str {
        if name.ends_with(".csv") {
            "text/csv"
        } else if name.ends_with(".json") {
            "application/json"
        } else {
            "text/plain"
        }
    }

    /// A streamed file download pulled from [`Storage`] in chunks.
    pub(super) struct Download {
        storage: &'static dyn Storage,
        dir: Directory,
        name: heapless::String<64>,
        size: u64,
    }

    /// Every handler returns one of these; `IntoResponse` renders it once (adding
    /// the CORS headers the reference attaches to every response).
    ///
    /// RAM NOTE: picoserve embeds each route's handler future **additively** in the
    /// per-worker serve future, so this type is replicated across every route slot.
    /// It therefore carries **only the inputs** a response needs — never a body
    /// buffer. All buffering (the JSON render `String`, the streamed-list
    /// collections) happens inside [`write_to`](IntoResponse::write_to), which is a
    /// *single* future — so those buffers cost one-per-connection, not one-per-route.
    /// Keep every variant small.
    pub(super) enum ApiResponse {
        /// `GET /api/status` — rendered from `state.latest()` in `write_to`.
        Status(AppState),
        /// `GET /api/scheduled` — rendered in `write_to`.
        Scheduled(AppState),
        /// `GET /api/config` — rendered from `state.config` in `write_to`.
        Config(AppState),
        /// The index page: asset + `{profiles_list}` split + streamed profile list,
        /// all built in `write_to`.
        Index(AppState),
        /// `GET /api/files/{dir}` — directory scanned + streamed in `write_to`.
        FileList(AppState, Directory),
        /// `DELETE /api/files/logs/all` — names collected, dir cleared, result
        /// streamed, all in `write_to`.
        DeleteAll(AppState, Directory),
        /// `{"success":true,"message":"<msg>"}` with an owned (dynamic) message.
        Message(heapless::String<MSG_CAP>),
        /// The upload-success envelope `{success,message,filename,size}`.
        Uploaded {
            name: heapless::String<64>,
            written: usize,
        },
        /// `{"success":false,"error":"<message>"}` envelope (the message is static).
        Fail {
            status: StatusCode,
            message: &'static str,
        },
        /// A pre-formed JSON literal, sent verbatim as `application/json`.
        Json {
            status: StatusCode,
            body: &'static str,
        },
        /// A plain-text body.
        Text {
            status: StatusCode,
            body: &'static str,
        },
        /// A compiled-in asset (HTML), served verbatim.
        Asset {
            bytes: &'static [u8],
            content_type: &'static str,
        },
        /// A streamed file download (refs + size only).
        Download(Download),
    }

    impl ApiResponse {
        /// A pre-formed JSON literal sent verbatim.
        fn static_json(status: StatusCode, body: &'static str) -> Self {
            ApiResponse::Json { status, body }
        }
        /// A plain-text body.
        fn error_text(status: StatusCode, body: &'static str) -> Self {
            ApiResponse::Text { status, body }
        }
        /// A `{"success":false,"error":"..."}` envelope (rendered in `write_to`).
        fn error(status: StatusCode, message: &'static str) -> Self {
            ApiResponse::Fail { status, message }
        }
    }

    const CORS: [(&str, &str); 3] = [
        ("Access-Control-Allow-Origin", "*"),
        (
            "Access-Control-Allow-Methods",
            "GET, POST, PUT, DELETE, OPTIONS",
        ),
        ("Access-Control-Allow-Headers", "Content-Type"),
    ];

    /// A [`Content`] wrapper that overrides the Content-Type. picoserve 0.18's
    /// `Response::new` derives the Content-Type from the body's `Content` impl
    /// (e.g. `&str` → `text/plain`); wrapping lets us set the exact type once,
    /// without emitting a duplicate `Content-Type` header.
    struct Typed<C: Content>(C, &'static str);

    impl<C: Content> Content for Typed<C> {
        fn content_type(&self) -> &'static str {
            self.1
        }
        fn content_length(&self) -> usize {
            self.0.content_length()
        }
        async fn write_content<W: picoserve::io::Write>(self, writer: W) -> Result<(), W::Error> {
            self.0.write_content(writer).await
        }
    }

    /// Send `body` as a CORS response with `content_type` and `status`. One place
    /// for every JSON/text/asset arm of [`ApiResponse::write_to`]; the render
    /// buffers it is handed are locals of `write_to`, so one per connection.
    async fn respond<R, W, C>(
        status: StatusCode,
        content_type: &'static str,
        body: C,
        connection: picoserve::response::Connection<'_, R>,
        response_writer: W,
    ) -> Result<picoserve::ResponseSent, W::Error>
    where
        R: picoserve::io::Read,
        W: picoserve::response::ResponseWriter<Error = R::Error>,
        C: Content,
    {
        picoserve::response::Response::new(status, Typed(body, content_type))
            .with_headers(CORS)
            .write_to(connection, response_writer)
            .await
    }

    /// Render each entry of `dir` into `body`'s packed arena (bounded; entries past
    /// the cap drop at an object boundary, so the JSON stays well-formed). Shared by
    /// the `FileList` response arm; runs inside `write_to`.
    fn collect_file_list(storage: &dyn Storage, dir: Directory, body: &mut FileList) {
        storage.for_each(dir, &mut |name, size, modified| {
            if body.lens.len() >= body.lens.capacity() {
                return;
            }
            let mut rec = heapless::String::<FILE_ENTRY_CAP>::new();
            // A name longer than fits (or huge numbers) overflows `rec`; skip that
            // entry rather than emit a half-written, malformed object.
            if write!(
                rec,
                "{{\"name\":\"{}\",\"size\":{},\"modified\":{}}}",
                name, size, modified
            )
            .is_err()
            {
                return;
            }
            if body.arena.len() + rec.len() <= body.arena.capacity() {
                let _ = body.arena.extend_from_slice(rec.as_bytes());
                let _ = body.lens.push(rec.len() as u16);
            }
        });
    }

    impl IntoResponse for ApiResponse {
        async fn write_to<
            R: picoserve::io::Read,
            W: picoserve::response::ResponseWriter<Error = R::Error>,
        >(
            self,
            connection: picoserve::response::Connection<'_, R>,
            response_writer: W,
        ) -> Result<picoserve::ResponseSent, W::Error> {
            use picoserve::response::Response;
            match self {
                // --- JSON bodies rendered into a single per-connection buffer ---
                ApiResponse::Status(state) => {
                    let mut b = heapless::String::<RENDER_CAP>::new();
                    let _ = json::write_status_json(&mut b, &state.latest());
                    respond(StatusCode::OK, "application/json", b, connection, response_writer).await
                }
                ApiResponse::Scheduled(state) => {
                    let mut b = heapless::String::<RENDER_CAP>::new();
                    let _ = json::write_scheduled_endpoint(&mut b, &state.latest());
                    respond(StatusCode::OK, "application/json", b, connection, response_writer).await
                }
                ApiResponse::Config(state) => {
                    let mut b = heapless::String::<RENDER_CAP>::new();
                    let _ = state.config.write_json(&mut b);
                    respond(StatusCode::OK, "application/json", b, connection, response_writer).await
                }
                ApiResponse::Message(msg) => {
                    let mut b = heapless::String::<RENDER_CAP>::new();
                    let _ = write!(b, "{{\"success\":true,\"message\":\"{}\"}}", msg);
                    respond(StatusCode::OK, "application/json", b, connection, response_writer).await
                }
                ApiResponse::Uploaded { name, written } => {
                    let mut b = heapless::String::<RENDER_CAP>::new();
                    let _ = write!(
                        b,
                        "{{\"success\":true,\"message\":\"Uploaded {}\",\"filename\":\"{}\",\"size\":{}}}",
                        name, name, written
                    );
                    respond(StatusCode::OK, "application/json", b, connection, response_writer).await
                }
                ApiResponse::Fail { status, message } => {
                    let mut b = heapless::String::<RENDER_CAP>::new();
                    let _ = write!(b, "{{\"success\":false,\"error\":\"{}\"}}", message);
                    respond(status, "application/json", b, connection, response_writer).await
                }
                // --- verbatim bodies ---
                ApiResponse::Json { status, body } => {
                    respond(status, "application/json", body, connection, response_writer).await
                }
                ApiResponse::Text { status, body } => {
                    respond(status, "text/plain", body, connection, response_writer).await
                }
                ApiResponse::Asset {
                    bytes,
                    content_type,
                } => {
                    respond(
                        StatusCode::OK,
                        content_type,
                        bytes,
                        connection,
                        response_writer,
                    )
                    .await
                }
                // A file download has a known size, so it goes out with a real
                // `Content-Length` (browser progress) and `Content-Disposition:
                // attachment` (the saved filename) — the reference's headers —
                // while still streaming the body in 1 KiB chunks. The
                // Content-Type/Length come from `StorageBody`'s `Content` impl.
                ApiResponse::Download(d) => {
                    let mut disposition = heapless::String::<96>::new();
                    let _ = write!(disposition, "attachment; filename=\"{}\"", d.name);
                    Response::new(StatusCode::OK, StorageBody(d))
                        .with_headers(CORS)
                        .with_header("Content-Disposition", disposition)
                        .write_to(connection, response_writer)
                        .await
                }
                // --- streamed bodies (collection runs here, once per connection) ---
                ApiResponse::Index(state) => {
                    let bytes = match state.storage.static_asset("index.html") {
                        Some(b) => b,
                        None => {
                            return respond(
                                StatusCode::NOT_FOUND,
                                "text/plain",
                                "Not found",
                                connection,
                                response_writer,
                            )
                            .await
                        }
                    };
                    // Fill the `{profiles_list}` placeholder as `main.py` prerendered
                    // it; serve the asset verbatim if the placeholder is absent.
                    match html::split_profiles_placeholder(bytes) {
                        None => {
                            respond(
                                StatusCode::OK,
                                "text/html",
                                bytes,
                                connection,
                                response_writer,
                            )
                            .await
                        }
                        Some((pre, post)) => {
                            let mut names: heapless::Vec<heapless::String<NAME_CAP>, MAX_PROFILES> =
                                heapless::Vec::new();
                            state.storage.for_each(
                                Directory::Profiles,
                                &mut |name, _size, _modified| {
                                    if let Some(stem) = html::profile_display_name(name) {
                                        if names.len() < names.capacity() {
                                            let mut s = heapless::String::new();
                                            if s.push_str(stem).is_ok() {
                                                let _ = names.push(s);
                                            }
                                        }
                                    }
                                },
                            );
                            ChunkedResponse::new(IndexBody { pre, post, names })
                                .into_response()
                                .with_headers(CORS)
                                .write_to(connection, response_writer)
                                .await
                        }
                    }
                }
                ApiResponse::FileList(state, dir) => {
                    let mut body = FileList {
                        directory: dir,
                        arena: heapless::Vec::new(),
                        lens: heapless::Vec::new(),
                    };
                    collect_file_list(state.storage, dir, &mut body);
                    ChunkedResponse::new(body)
                        .into_response()
                        .with_headers(CORS)
                        .write_to(connection, response_writer)
                        .await
                }
                ApiResponse::DeleteAll(state, dir) => {
                    // Collect the (capped) names BEFORE clearing, then clear, then
                    // stream the result. The destructive op runs here, but the route
                    // is idle-gated by `file_delete_all`, so it is safe.
                    let mut names: heapless::Vec<heapless::String<NAME_CAP>, MAX_PROFILES> =
                        heapless::Vec::new();
                    let mut total = 0usize;
                    state.storage.for_each(dir, &mut |name, _size, _modified| {
                        total += 1;
                        if names.len() < names.capacity() {
                            let mut s = heapless::String::new();
                            if s.push_str(name).is_ok() {
                                let _ = names.push(s);
                            }
                        }
                    });
                    if state.storage.remove_all(dir).is_err() {
                        let mut b = heapless::String::<RENDER_CAP>::new();
                        let _ = write!(
                            b,
                            "{{\"success\":false,\"error\":\"Failed to delete files\"}}"
                        );
                        return respond(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "application/json",
                            b,
                            connection,
                            response_writer,
                        )
                        .await;
                    }
                    ChunkedResponse::new(DeleteAllBody { names, total })
                        .into_response()
                        .with_headers(CORS)
                        .write_to(connection, response_writer)
                        .await
                }
            }
        }
    }

    /// The `GET /api/files/{dir}` listing, rendered once into a packed arena then
    /// streamed as a chunked response. `arena` holds the entry JSON objects
    /// back-to-back; `lens` records each object's byte length so [`write_chunks`]
    /// can slice and comma-join them without scanning for a separator. Bounded by
    /// [`FILE_LIST_ARENA`] / [`FILE_LIST_MAX`]; entries past either bound are
    /// dropped at an object boundary, so the emitted JSON is always well-formed.
    ///
    /// [`write_chunks`]: Chunks::write_chunks
    pub(super) struct FileList {
        directory: Directory,
        arena: heapless::Vec<u8, FILE_LIST_ARENA>,
        lens: heapless::Vec<u16, FILE_LIST_MAX>,
    }

    impl Chunks for FileList {
        fn content_type(&self) -> &'static str {
            "application/json"
        }

        async fn write_chunks<W: picoserve::io::Write>(
            self,
            mut chunk_writer: ChunkWriter<W>,
        ) -> Result<ChunksWritten, W::Error> {
            let mut prefix = heapless::String::<96>::new();
            let _ = write!(
                prefix,
                "{{\"success\":true,\"directory\":\"{}\",\"files\":[",
                self.directory.as_str()
            );
            chunk_writer.write_chunk(prefix.as_bytes()).await?;

            // Emit each pre-rendered object, comma-separated, slicing the arena by
            // the recorded lengths (kept in lockstep with the handler's render).
            let mut off = 0usize;
            for (i, &len) in self.lens.iter().enumerate() {
                if i > 0 {
                    chunk_writer.write_chunk(b",").await?;
                }
                let end = off + len as usize;
                chunk_writer.write_chunk(&self.arena[off..end]).await?;
                off = end;
            }

            let mut suffix = heapless::String::<24>::new();
            let _ = write!(suffix, "],\"count\":{}}}", self.lens.len());
            chunk_writer.write_chunk(suffix.as_bytes()).await?;
            chunk_writer.finalize().await
        }
    }

    /// `DELETE /api/files/logs/all` result, streamed: `{success, deleted_count,
    /// deleted_files:[...]}`. Holds the (capped) deleted names plus the full count;
    /// built and consumed inside `write_to`, so it never sits in a per-route slot.
    pub(super) struct DeleteAllBody {
        names: heapless::Vec<heapless::String<NAME_CAP>, MAX_PROFILES>,
        total: usize,
    }

    impl Chunks for DeleteAllBody {
        fn content_type(&self) -> &'static str {
            "application/json"
        }

        async fn write_chunks<W: picoserve::io::Write>(
            self,
            mut chunk_writer: ChunkWriter<W>,
        ) -> Result<ChunksWritten, W::Error> {
            let mut head = heapless::String::<80>::new();
            let _ = write!(
                head,
                "{{\"success\":true,\"deleted_count\":{},\"deleted_files\":[",
                self.total
            );
            chunk_writer.write_chunk(head.as_bytes()).await?;
            for (i, name) in self.names.iter().enumerate() {
                if i > 0 {
                    chunk_writer.write_chunk(b",").await?;
                }
                chunk_writer.write_chunk(b"\"").await?;
                chunk_writer.write_chunk(name.as_bytes()).await?;
                chunk_writer.write_chunk(b"\"").await?;
            }
            chunk_writer.write_chunk(b"]}").await?;
            chunk_writer.finalize().await
        }
    }

    /// `index.html` with its `{profiles_list}` placeholder filled in at request
    /// time (the boot-time prerender in `main.py` + `html_cache.py`). The static
    /// prefix/suffix bracket the rendered profile list, which is streamed in.
    pub(super) struct IndexBody {
        pre: &'static [u8],
        post: &'static [u8],
        names: heapless::Vec<heapless::String<NAME_CAP>, MAX_PROFILES>,
    }

    impl Chunks for IndexBody {
        fn content_type(&self) -> &'static str {
            "text/html"
        }

        async fn write_chunks<W: picoserve::io::Write>(
            self,
            mut chunk_writer: ChunkWriter<W>,
        ) -> Result<ChunksWritten, W::Error> {
            // Renders the `<ul>` profile list fragment-for-fragment (the same
            // markup `server/html_cache.py:render_profiles_list` produced), so no
            // full-page buffer is needed.
            chunk_writer.write_chunk(self.pre).await?;
            if self.names.is_empty() {
                chunk_writer
                    .write_chunk(b"<ul><li>No profiles found</li></ul>")
                    .await?;
            } else {
                chunk_writer.write_chunk(b"<ul>").await?;
                for name in &self.names {
                    let n = name.as_bytes();
                    chunk_writer.write_chunk(b"<li>").await?;
                    chunk_writer.write_chunk(n).await?;
                    chunk_writer
                        .write_chunk(b" <button onclick=\"startProfile('")
                        .await?;
                    chunk_writer.write_chunk(n).await?;
                    chunk_writer
                        .write_chunk(b"')\">Start</button></li>")
                        .await?;
                }
                chunk_writer.write_chunk(b"</ul>").await?;
            }
            chunk_writer.write_chunk(self.post).await?;
            chunk_writer.finalize().await
        }
    }

    /// A sized picoserve body that streams a file from [`Storage`] in 1 KiB
    /// chunks (the reference's `FILE_CHUNK_SIZE`), so peak RAM stays flat
    /// regardless of file size, while declaring `Content-Length` up front (the
    /// `size` from the `os.stat` taken in `file_get`).
    struct StorageBody(Download);

    impl Content for StorageBody {
        fn content_type(&self) -> &'static str {
            content_type_for(&self.0.name)
        }

        fn content_length(&self) -> usize {
            self.0.size as usize
        }

        async fn write_content<W: picoserve::io::Write>(
            self,
            mut writer: W,
        ) -> Result<(), W::Error> {
            let d = self.0;
            let mut offset = 0u64;
            let mut buf = [0u8; api::FILE_CHUNK_SIZE];
            while offset < d.size {
                let n = match d.storage.read_chunk(d.dir, &d.name, offset, &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                writer.write_all(&buf[..n]).await?;
                offset += n as u64;
            }
            Ok(())
        }
    }

    /// A buffered JSON command body (`json.loads(body)` in the reference), capped
    /// at `MAX_JSON_BODY`.
    pub(super) struct JsonBody(pub heapless::Vec<u8, BODY_CAP>);

    impl<'r, S> picoserve::extract::FromRequest<'r, S> for JsonBody {
        // A 413 on an oversized body, rather than silently truncating it.
        type Rejection = ApiResponse;
        async fn from_request<R: picoserve::io::Read>(
            _state: &'r S,
            _parts: picoserve::request::RequestParts<'r>,
            request_body: picoserve::request::RequestBody<'r, R>,
        ) -> Result<Self, Self::Rejection> {
            // Reject an oversized declared body before buffering it — the
            // reference's `content_length > MAX_JSON_BODY` 413 guard in
            // `handle_client`. JSON command bodies are tiny; buffering a hostile
            // Content-Length would risk the heap.
            if request_body.content_length() > BODY_CAP {
                return Err(ApiResponse::error_text(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "Body too large",
                ));
            }
            let mut v = heapless::Vec::new();
            match request_body.read_all().await {
                Ok(bytes) => {
                    // content_length <= BODY_CAP, so this never clips a valid body.
                    let take = bytes.len().min(BODY_CAP);
                    let _ = v.extend_from_slice(&bytes[..take]);
                }
                // The body did not fit the request buffer → too large (413). A
                // short/aborted body (EOF/IO) leaves `v` empty, and the handler
                // returns 400 on the missing field — matching the reference.
                Err(picoserve::request::ReadAllBodyError::BufferIsTooSmall { .. }) => {
                    return Err(ApiResponse::error_text(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "Body too large",
                    ));
                }
                Err(_) => {}
            }
            Ok(JsonBody(v))
        }
    }

    impl JsonBody {
        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.0).unwrap_or("")
        }
    }

    /// Outcome of streaming a PUT body to the upload scratch, classified by
    /// [`api::validate_upload_size`] against the `Content-Length`.
    pub(super) enum UploadOutcome {
        /// Streamed successfully; carries the number of bytes written (the
        /// reference's `written`, echoed as `size`).
        Ok(usize),
        Missing,
        TooLarge,
        Failed,
    }

    /// A streamed file upload (`handle_api_files_upload`): the body is written to
    /// the [`Storage`] scratch in chunks, never buffered whole, then committed by
    /// the handler once the destination path is validated.
    pub(super) struct Upload {
        pub outcome: UploadOutcome,
    }

    impl<'r> picoserve::extract::FromRequest<'r, AppState> for Upload {
        type Rejection = core::convert::Infallible;
        async fn from_request<R: picoserve::io::Read>(
            state: &'r AppState,
            parts: picoserve::request::RequestParts<'r>,
            request_body: picoserve::request::RequestBody<'r, R>,
        ) -> Result<Self, Self::Rejection> {
            let content_length = parts
                .headers()
                .get("Content-Length")
                .and_then(|v| v.as_str().ok())
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0);
            let outcome = match api::validate_upload_size(content_length) {
                api::UploadSize::Missing => UploadOutcome::Missing,
                api::UploadSize::TooLarge => UploadOutcome::TooLarge,
                api::UploadSize::Ok => stream_upload(state, request_body).await,
            };
            Ok(Upload { outcome })
        }
    }

    async fn stream_upload<R: picoserve::io::Read>(
        state: &AppState,
        request_body: picoserve::request::RequestBody<'_, R>,
    ) -> UploadOutcome {
        use picoserve::io::Read as _;
        if state.storage.upload_begin().is_err() {
            return UploadOutcome::Failed;
        }
        let mut reader = request_body.reader();
        let mut buf = [0u8; api::FILE_CHUNK_SIZE];
        let mut written = 0usize;
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => return UploadOutcome::Ok(written),
                Ok(n) => {
                    if state.storage.upload_write(&buf[..n]).is_err() {
                        state.storage.upload_abort();
                        return UploadOutcome::Failed;
                    }
                    written += n;
                }
                Err(_) => {
                    state.storage.upload_abort();
                    return UploadOutcome::Failed;
                }
            }
        }
    }

    /// The picoserve worker pool. `make_app(state)` bakes the shared state into
    /// the router, which is built once in the firmware and shared `&'static`;
    /// each worker serves on `port` (the configured `WEB_SERVER_PORT`, default
    /// 80 — editing it in `config.json` now takes effect). On a single-NIC
    /// embassy-net stack there is no per-host bind, so `WEB_SERVER_HOST` is
    /// inherently "all interfaces"; only the port is actionable. picoserve 0.18
    /// replaces the free `listen_and_serve_with_state` function with the
    /// `Server` builder.
    #[embassy_executor::task(pool_size = WEB_TASK_POOL_TOTAL)]
    pub async fn web_task(
        id: usize,
        stack: embassy_net::Stack<'static>,
        app: &'static AppRouter,
        config: &'static picoserve::Config,
        port: u16,
    ) -> ! {
        let mut tcp_rx = [0u8; 1024];
        let mut tcp_tx = [0u8; 1024];
        let mut http = [0u8; 2048];
        loop {
            let _ = picoserve::Server::new(app, config, &mut http)
                .listen_and_serve(id, stack, port, &mut tcp_rx, &mut tcp_tx)
                .await;
        }
    }
}

pub use web::{make_app, web_task, AppRouter};
