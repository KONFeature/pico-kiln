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

/// Command-queue depth (Core 0 → Core 1) — the reference command queue holds 10.
pub const COMMAND_DEPTH: usize = 10;
/// Latest-status broadcast consumers: web pollers + CSV logger + LCD + recovery.
pub const STATUS_CONSUMERS: usize = 4;
/// picoserve worker pool size — the reference's `MAX_CONCURRENT_CONNECTIONS`.
pub const WEB_TASK_POOL_SIZE: usize = api::MAX_CONCURRENT_CONNECTIONS;

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
    pub elapsed_seconds: f64,
}

/// CSV logging — the `data_logger.py` half that owns timing and the file handle.
/// Subscribes to the status broadcast and writes a row through [`Storage`] when
/// the interval has elapsed (`LOGGING_INTERVAL` normally, 2 s while TUNING), using
/// the verified [`csv`] formatters. On the IDLE→RUNNING/TUNING edge it starts a
/// new file (header) — or, for a crash-recovery resume, **appends** to the
/// interrupted run's file (no header) and writes a one-shot `RECOVERY` event row
/// (`data_logger.log_recovery_event`). Forces a final terminal-state row on the
/// way out — `data_logger.update`.
#[embassy_executor::task]
pub async fn csv_logger_task(
    status: &'static StatusWatch,
    storage: &'static dyn Storage,
    clock: &'static dyn Clock,
    config: &'static KilnConfig,
    mut recovery: Option<RecoveryLog>,
) -> ! {
    use kiln_core::state::KilnState;
    let mut rx = status.receiver().unwrap();
    let mut logging = false;
    let mut filename = heapless::String::<96>::new();
    let mut last_log: i64 = 0;
    let mut prev_state = KilnState::Idle;

    loop {
        let s = rx.changed().await;
        let now = clock.unix_seconds().unwrap_or(0);
        let active = matches!(s.state, KilnState::Running | KilnState::Tuning);

        if active && !logging {
            if let Some(rec) = recovery.take() {
                // Recovery resume: continue the interrupted run's file (append,
                // no header) and write the one-shot RECOVERY marker row using the
                // resume elapsed + live temps/SSR/rate (data_logger.py 201-264).
                filename.clear();
                let _ = filename.push_str(&rec.filename);
                let mut row = heapless::String::<256>::new();
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
                }
            } else if let Some(name) = s.profile_name {
                filename.clear();
                let _ = csv::write_log_filename(&mut filename, name.as_str(), now);
                if storage
                    .append(Directory::Logs, &filename, csv::HEADER.as_bytes(), true)
                    .is_ok()
                {
                    logging = true;
                    last_log = 0;
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
            if leaving || (now - last_log) >= interval {
                let mut row = heapless::String::<256>::new();
                let _ = csv::write_row(&mut row, &s);
                let _ = storage.append(Directory::Logs, &filename, row.as_bytes(), false);
                last_log = now;
            }
            if leaving {
                logging = false;
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
    /// Working buffer for reading a profile file to parse (profiles are a few KB).
    const PROFILE_READ_CAP: usize = 8192;
    /// Max profiles rendered into the index `{profiles_list}` / collected for a
    /// bulk log delete. The flash holds far fewer; extras are silently dropped.
    const MAX_PROFILES: usize = 32;
    /// Max length of a profile/log filename held on the stack.
    const NAME_CAP: usize = 64;

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
                    post(|s: State<AppState>| async move { enqueue(&s.0, Command::Stop) })
                        .options(cors_preflight),
                )
                .route(
                    "/api/clear-error",
                    post(|s: State<AppState>| async move { enqueue(&s.0, Command::ClearError) })
                        .options(cors_preflight),
                )
                .route(
                    "/api/shutdown",
                    post(|s: State<AppState>| async move { enqueue(&s.0, Command::Shutdown) })
                        .options(cors_preflight),
                )
                .route(
                    "/api/scheduled/cancel",
                    post(
                        |s: State<AppState>| async move { enqueue(&s.0, Command::CancelScheduled) },
                    )
                    .options(cors_preflight),
                )
                .route(
                    "/api/tuning/stop",
                    post(|s: State<AppState>| async move { enqueue(&s.0, Command::StopTuning) })
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

    async fn status_json(State(state): State<AppState>) -> impl IntoResponse {
        let mut body = heapless::String::<2048>::new();
        let _ = json::write_status_json(&mut body, &state.latest());
        ApiResponse::json(StatusCode::OK, body)
    }

    async fn scheduled_json(State(state): State<AppState>) -> impl IntoResponse {
        let mut body = heapless::String::<2048>::new();
        let _ = json::write_scheduled_endpoint(&mut body, &state.latest());
        ApiResponse::json(StatusCode::OK, body)
    }

    /// Enqueue a typed command, returning the reference's `{"success": …}`
    /// envelope (200, or 500 when the queue is full) — `_send_command`.
    fn enqueue(state: &AppState, cmd: Command) -> ApiResponse {
        if state.commands.try_send(cmd).is_ok() {
            ApiResponse::static_json(StatusCode::OK, "{\"success\":true}")
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
        let mut body = heapless::String::<2048>::new();
        let _ = state.config.write_json(&mut body);
        ApiResponse::json(StatusCode::OK, body)
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
        let name = match api::json_get_str(body.as_str(), "profile") {
            Some(n) if !n.is_empty() => n,
            _ => return ApiResponse::error(StatusCode::BAD_REQUEST, "Profile name required"),
        };
        match load_profile(&state, name) {
            Ok((profile, parsed)) => enqueue(&state, Command::RunProfile { profile, parsed }),
            Err(resp) => resp,
        }
    }

    async fn schedule(State(state): State<AppState>, body: JsonBody) -> impl IntoResponse {
        let name = api::json_get_str(body.as_str(), "profile");
        let start = api::json_get_f64(body.as_str(), "start_time");
        if !api::schedule_fields_present(name, start) {
            return ApiResponse::error(StatusCode::BAD_REQUEST, "profile and start_time required");
        }
        let (name, start) = (name.unwrap(), start.unwrap());
        if !api::start_time_in_future(start, state.now() as f64) {
            return ApiResponse::error(StatusCode::BAD_REQUEST, "start_time must be in the future");
        }
        match load_profile(&state, name) {
            Ok((profile, parsed)) => enqueue(
                &state,
                Command::ScheduleProfile {
                    profile,
                    parsed,
                    start_time: start as u64,
                },
            ),
            Err(resp) => resp,
        }
    }

    async fn tuning_start(State(state): State<AppState>, body: JsonBody) -> impl IntoResponse {
        let mode = match api::parse_tuning_mode(
            api::json_get_str(body.as_str(), "mode").unwrap_or("STANDARD"),
        ) {
            Some(m) => m,
            None => return ApiResponse::error(StatusCode::BAD_REQUEST, api::INVALID_MODE_MESSAGE),
        };
        let max_temp = api::json_get_f64(body.as_str(), "max_temp");
        if !api::max_temp_valid(max_temp) {
            return ApiResponse::error(StatusCode::BAD_REQUEST, api::MAX_TEMP_RANGE_MESSAGE);
        }
        enqueue(&state, Command::StartTuning { mode, max_temp })
    }

    /// Read `profiles/{name}.json` and parse it (Core 0 owns the FS and ships the
    /// `parsed` profile), reproducing run/schedule's not-found → 404 and
    /// parse-error → 400 paths.
    fn load_profile(state: &AppState, name: &str) -> Result<(ProfileName, Profile), ApiResponse> {
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
        let n = state
            .storage
            .read_chunk(Directory::Profiles, &path, 0, &mut buf)
            .map_err(|_| ApiResponse::error(StatusCode::NOT_FOUND, "Profile not found"))?;
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
        let directory = match file_guard(&state, &dir, None) {
            Ok(d) => d,
            Err(resp) => return resp,
        };
        let mut body = heapless::String::<2048>::new();
        let _ = write!(
            body,
            "{{\"success\":true,\"directory\":\"{}\",\"files\":[",
            directory.as_str()
        );
        let mut count = 0usize;
        state
            .storage
            .for_each(directory, &mut |name, size, modified| {
                if count > 0 {
                    let _ = body.push(',');
                }
                let _ = write!(
                    body,
                    "{{\"name\":\"{}\",\"size\":{},\"modified\":{}}}",
                    name, size, modified
                );
                count += 1;
            });
        let _ = write!(body, "],\"count\":{}}}", count);
        ApiResponse::json(StatusCode::OK, body)
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
            UploadOutcome::Ok => {
                if state.storage.upload_commit(directory, &file).is_ok() {
                    ApiResponse::static_json(StatusCode::OK, "{\"success\":true}")
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
    /// `handle_api_files_delete_all`). Idle-gated and logs-only; returns
    /// `{success, deleted_count, deleted_files:[...]}`. The file list is capped at
    /// `MAX_PROFILES` for the response, but `remove_all` clears the whole dir.
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
        // Snapshot names (capped) for the response, count all, then clear the dir.
        let mut names: heapless::Vec<heapless::String<NAME_CAP>, MAX_PROFILES> =
            heapless::Vec::new();
        let mut total = 0usize;
        state
            .storage
            .for_each(directory, &mut |name, _size, _modified| {
                total += 1;
                if names.len() < names.capacity() {
                    let mut s = heapless::String::<NAME_CAP>::new();
                    if s.push_str(name).is_ok() {
                        let _ = names.push(s);
                    }
                }
            });
        if state.storage.remove_all(directory).is_err() {
            return ApiResponse::error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete files");
        }
        let mut body = heapless::String::<2048>::new();
        let _ = write!(
            body,
            "{{\"success\":true,\"deleted_count\":{},\"deleted_files\":[",
            total
        );
        for (i, name) in names.iter().enumerate() {
            if i > 0 {
                let _ = body.push(',');
            }
            let _ = write!(body, "\"{}\"", name);
        }
        let _ = body.push_str("]}");
        ApiResponse::json(StatusCode::OK, body)
    }

    async fn page_index(State(state): State<AppState>) -> impl IntoResponse {
        let bytes = match state.storage.static_asset("index.html") {
            Some(b) => b,
            None => return ApiResponse::error_text(StatusCode::NOT_FOUND, "Not found"),
        };
        // Fill the `{profiles_list}` placeholder the way `main.py` prerendered it.
        // If absent (unexpected), fall back to serving the bytes verbatim.
        match html::split_profiles_placeholder(bytes) {
            None => ApiResponse::Asset {
                bytes,
                content_type: "text/html",
            },
            Some((pre, post)) => {
                let mut names: heapless::Vec<heapless::String<NAME_CAP>, MAX_PROFILES> =
                    heapless::Vec::new();
                state
                    .storage
                    .for_each(Directory::Profiles, &mut |name, _size, _modified| {
                        if let Some(stem) = html::profile_display_name(name) {
                            let mut s = heapless::String::<NAME_CAP>::new();
                            if s.push_str(stem).is_ok() {
                                let _ = names.push(s);
                            }
                        }
                    });
                ApiResponse::Index(IndexBody { pre, post, names })
            }
        }
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

    /// Every handler returns one of these; `IntoResponse` is implemented once,
    /// adding the CORS headers the reference attaches to every response.
    pub(super) enum ApiResponse {
        Json {
            status: StatusCode,
            body: heapless::String<2048>,
        },
        StaticJson {
            status: StatusCode,
            body: &'static str,
        },
        Text {
            status: StatusCode,
            body: &'static str,
        },
        Asset {
            bytes: &'static [u8],
            content_type: &'static str,
        },
        Download(Download),
        Index(IndexBody),
    }

    impl ApiResponse {
        fn json(status: StatusCode, body: heapless::String<2048>) -> Self {
            ApiResponse::Json { status, body }
        }
        fn static_json(status: StatusCode, body: &'static str) -> Self {
            ApiResponse::StaticJson { status, body }
        }
        fn error_text(status: StatusCode, body: &'static str) -> Self {
            ApiResponse::Text { status, body }
        }
        /// A `{"success": false, "error": "..."}` envelope with `status`.
        fn error(status: StatusCode, message: &str) -> Self {
            let mut body = heapless::String::<2048>::new();
            let _ = write!(body, "{{\"success\":false,\"error\":\"{}\"}}", message);
            ApiResponse::Json { status, body }
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
                ApiResponse::Json { status, body } => {
                    Response::new(status, Typed(body, "application/json"))
                        .with_headers(CORS)
                        .write_to(connection, response_writer)
                        .await
                }
                ApiResponse::StaticJson { status, body } => {
                    Response::new(status, Typed(body, "application/json"))
                        .with_headers(CORS)
                        .write_to(connection, response_writer)
                        .await
                }
                ApiResponse::Text { status, body } => {
                    Response::new(status, Typed(body, "text/plain"))
                        .with_headers(CORS)
                        .write_to(connection, response_writer)
                        .await
                }
                ApiResponse::Asset {
                    bytes,
                    content_type,
                } => {
                    Response::new(StatusCode::OK, Typed(bytes, content_type))
                        .with_headers(CORS)
                        .write_to(connection, response_writer)
                        .await
                }
                // Streamed bodies have a size not worth buffering, so they use a
                // chunked (`Transfer-Encoding: chunked`) response; the Content-Type
                // comes from each `Chunks` impl.
                ApiResponse::Download(d) => {
                    ChunkedResponse::new(StorageChunks(d))
                        .into_response()
                        .with_headers(CORS)
                        .write_to(connection, response_writer)
                        .await
                }
                ApiResponse::Index(body) => {
                    ChunkedResponse::new(body)
                        .into_response()
                        .with_headers(CORS)
                        .write_to(connection, response_writer)
                        .await
                }
            }
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
            // Streams `html::render_profiles_list` (the host-tested spec)
            // fragment-for-fragment, so no full-page buffer is needed. Keep these
            // byte fragments in lockstep with that function.
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

    /// A picoserve chunked body that streams a file from [`Storage`] in 1 KiB
    /// chunks (the reference's `FILE_CHUNK_SIZE`), so peak RAM stays flat
    /// regardless of file size.
    struct StorageChunks(Download);

    impl Chunks for StorageChunks {
        fn content_type(&self) -> &'static str {
            content_type_for(&self.0.name)
        }

        async fn write_chunks<W: picoserve::io::Write>(
            self,
            mut chunk_writer: ChunkWriter<W>,
        ) -> Result<ChunksWritten, W::Error> {
            let d = self.0;
            let mut offset = 0u64;
            let mut buf = [0u8; api::FILE_CHUNK_SIZE];
            while offset < d.size {
                let n = match d.storage.read_chunk(d.dir, &d.name, offset, &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                chunk_writer.write_chunk(&buf[..n]).await?;
                offset += n as u64;
            }
            chunk_writer.finalize().await
        }
    }

    /// A buffered JSON command body (`json.loads(body)` in the reference), capped
    /// at `MAX_JSON_BODY`.
    pub(super) struct JsonBody(pub heapless::Vec<u8, BODY_CAP>);

    impl<'r, S> picoserve::extract::FromRequest<'r, S> for JsonBody {
        type Rejection = core::convert::Infallible;
        async fn from_request<R: picoserve::io::Read>(
            _state: &'r S,
            _parts: picoserve::request::RequestParts<'r>,
            request_body: picoserve::request::RequestBody<'r, R>,
        ) -> Result<Self, Self::Rejection> {
            let mut v = heapless::Vec::new();
            if let Ok(bytes) = request_body.read_all().await {
                let take = bytes.len().min(BODY_CAP);
                let _ = v.extend_from_slice(&bytes[..take]);
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
        Ok,
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
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => return UploadOutcome::Ok,
                Ok(n) => {
                    if state.storage.upload_write(&buf[..n]).is_err() {
                        state.storage.upload_abort();
                        return UploadOutcome::Failed;
                    }
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
    /// each worker serves on port 80. picoserve 0.18 replaces the free
    /// `listen_and_serve_with_state` function with the `Server` builder.
    #[embassy_executor::task(pool_size = WEB_TASK_POOL_SIZE)]
    pub async fn web_task(
        id: usize,
        stack: embassy_net::Stack<'static>,
        app: &'static AppRouter,
        config: &'static picoserve::Config,
    ) -> ! {
        let mut tcp_rx = [0u8; 1024];
        let mut tcp_tx = [0u8; 1024];
        let mut http = [0u8; 2048];
        loop {
            let _ = picoserve::Server::new(app, config, &mut http)
                .listen_and_serve(id, stack, 80, &mut tcp_rx, &mut tcp_tx)
                .await;
        }
    }
}

pub use web::{make_app, web_task, AppRouter};
