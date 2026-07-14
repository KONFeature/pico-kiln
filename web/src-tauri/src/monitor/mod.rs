//! Background kiln monitor.
//!
//! A single in-process poller (the "supervisor") owns all kiln state: it polls
//! the firmware `/api/status` endpoint, records a rolling temperature history,
//! detects state transitions, and fires local notifications on error / complete
//! / connection loss. The frontend reads this state through Tauri commands and
//! live events instead of polling the Pico itself, so there is exactly one
//! poller hitting the (2-connection-limited) controller.
//!
//! When the kiln becomes active (RUNNING / TUNING / a profile scheduled), the
//! frontend promotes monitoring to the `tauri-plugin-background-service`
//! foreground service, which keeps the OS process (and therefore this same
//! supervisor task) alive while the app is backgrounded. See `service.rs`.

pub mod commands;
pub mod service;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;

/// Rolling history retention window (4 hours).
const HISTORY_MAX_AGE_MS: i64 = 4 * 60 * 60 * 1000;
/// Hard cap on retained samples (defensive; ~2880 at 5s over 4h).
const HISTORY_MAX_POINTS: usize = 6000;
/// Grace period a "connection lost while active" must persist before alerting.
const CONN_LOST_ALERT_MS: i64 = 10 * 60 * 1000;
/// Persist the history file at most this often.
const HISTORY_PERSIST_INTERVAL_MS: i64 = 30 * 1000;
/// HTTP request timeout for a status poll.
const POLL_TIMEOUT_MS: u64 = 10_000;

/// A single temperature sample served to the frontend live chart.
#[derive(Debug, Clone, Serialize)]
pub struct HistoryPoint {
    /// Wall-clock time, epoch milliseconds (monotonic across the buffer).
    pub t: i64,
    pub temp: f64,
    /// Setpoint; `None` when idle / natural cooling (SSR off).
    pub target: Option<f64>,
    pub state: String,
}

/// Health snapshot that drives the "service not running" toast in the app.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MonitoringStatus {
    /// The supervisor poll loop is alive and has a configured URL.
    pub running: bool,
    /// Kiln is in an active state (RUNNING / TUNING / scheduled) — the
    /// foreground service should be running.
    pub active: bool,
    /// Last status poll succeeded.
    pub reachable: bool,
    pub url: Option<String>,
    pub last_error: Option<String>,
    /// Epoch ms of the last successful poll.
    pub last_ok: Option<i64>,
}

struct MonitorState {
    url: Option<String>,
    /// Raw `/api/status` JSON from the last successful poll, passed through to
    /// the frontend verbatim (it already models `KilnStatus`).
    latest: Option<Value>,
    last_state: Option<String>,
    is_active: bool,
    reachable: bool,
    last_error: Option<String>,
    last_ok: Option<i64>,
    /// Epoch ms of the first failure in the current outage (for the 10-min
    /// connection-lost alert). Cleared on any success.
    first_fail: Option<i64>,
    conn_lost_alerted: bool,
    history: Vec<HistoryPoint>,
    last_persist: i64,
}

impl MonitorState {
    fn new() -> Self {
        Self {
            url: None,
            latest: None,
            last_state: None,
            is_active: false,
            reachable: false,
            last_error: None,
            last_ok: None,
            first_fail: None,
            conn_lost_alerted: false,
            history: Vec::new(),
            last_persist: 0,
        }
    }
}

struct Inner {
    state: Mutex<MonitorState>,
    /// Set while the foreground-service keepalive is engaged.
    fgs_active: AtomicBool,
    http: reqwest::Client,
    data_dir: Mutex<Option<std::path::PathBuf>>,
}

impl Inner {
    /// Lock the state, recovering from poisoning so one panic can't
    /// permanently wedge the monitor.
    fn st(&self) -> std::sync::MutexGuard<'_, MonitorState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn dir(&self) -> std::sync::MutexGuard<'_, Option<std::path::PathBuf>> {
        self.data_dir.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[derive(Clone)]
pub struct Monitor {
    inner: Arc<Inner>,
}

/// A transition-derived alert to fire after releasing the state lock.
enum Alert {
    Error(String),
    Complete,
    ConnectionLost,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Active = something worth keeping the foreground service alive for.
fn compute_active(status: &Value) -> bool {
    let state = status.get("state").and_then(Value::as_str).unwrap_or("");
    if matches!(state, "RUNNING" | "TUNING") {
        return true;
    }
    // A pending scheduled profile keeps us watching even while IDLE.
    status
        .get("scheduled_profile")
        .map(|v| !v.is_null())
        .unwrap_or(false)
}

impl Monitor {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(POLL_TIMEOUT_MS))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(MonitorState::new()),
                fgs_active: AtomicBool::new(false),
                http,
                data_dir: Mutex::new(None),
            }),
        }
    }

    /// Wire up the app data dir and rehydrate persisted URL + history. Called
    /// once during `setup()`.
    pub fn attach<R: Runtime>(&self, app: &AppHandle<R>) {
        if let Ok(dir) = app.path().app_data_dir() {
            let _ = std::fs::create_dir_all(&dir);
            *self.inner.dir() = Some(dir.clone());
            // Restore URL.
            if let Ok(url) = std::fs::read_to_string(dir.join("kiln_url.txt")) {
                let url = url.trim().to_string();
                if !url.is_empty() {
                    self.inner.st().url = Some(url);
                }
            }
            // Restore history (best effort).
            if let Ok(csv) = std::fs::read_to_string(dir.join("kiln_history.csv")) {
                let points = parse_history_csv(&csv);
                self.inner.st().history = points;
            }
        }
    }

    pub fn set_fgs_active(&self, active: bool) {
        self.inner.fgs_active.store(active, Ordering::SeqCst);
    }

    pub fn is_kiln_active(&self) -> bool {
        self.inner.st().is_active
    }

    pub fn get_url(&self) -> Option<String> {
        self.inner.st().url.clone()
    }

    /// Update the polled URL. Resets history when the target changes (different
    /// kiln) and persists both to disk so a `START_STICKY` relaunch — which has
    /// no webview to re-supply the URL — can keep polling.
    pub fn set_url(&self, url: Option<String>) {
        let url = url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
        let dir = self.inner.dir().clone();
        {
            let mut st = self.inner.st();
            if st.url == url {
                return;
            }
            st.url = url.clone();
            // Different kiln → stale history.
            st.history.clear();
            st.latest = None;
            st.last_state = None;
            st.reachable = false;
            st.first_fail = None;
            st.conn_lost_alerted = false;
        }
        if let Some(dir) = dir {
            match &url {
                Some(u) => {
                    let _ = std::fs::write(dir.join("kiln_url.txt"), u);
                }
                None => {
                    let _ = std::fs::remove_file(dir.join("kiln_url.txt"));
                }
            }
            let _ = std::fs::remove_file(dir.join("kiln_history.csv"));
        }
    }

    pub fn snapshot_status(&self) -> Option<Value> {
        self.inner.st().latest.clone()
    }

    pub fn history(&self) -> Vec<HistoryPoint> {
        self.inner.st().history.clone()
    }

    pub fn monitoring_status(&self) -> MonitoringStatus {
        let st = self.inner.st();
        MonitoringStatus {
            running: st.url.is_some(),
            active: st.is_active,
            reachable: st.reachable,
            url: st.url.clone(),
            last_error: st.last_error.clone(),
            last_ok: st.last_ok,
        }
    }

    /// Poll interval for the current state.
    fn cadence(&self) -> Duration {
        let state = {
            let st = self.inner.st();
            st.latest
                .as_ref()
                .and_then(|v| v.get("state").and_then(Value::as_str))
                .map(str::to_string)
        };
        match state.as_deref() {
            Some("TUNING") => Duration::from_secs(2),
            Some("RUNNING") => Duration::from_secs(5),
            Some("ERROR") => Duration::from_secs(15),
            _ => Duration::from_secs(30),
        }
    }

    /// Trigger an immediate poll (used by the `refresh_kiln` command).
    pub async fn poll_now<R: Runtime>(&self, app: &AppHandle<R>) {
        self.poll_once(app).await;
    }

    /// One poll cycle. Never holds the state lock across the network await.
    async fn poll_once<R: Runtime>(&self, app: &AppHandle<R>) {
        let url = match self.get_url() {
            Some(u) => u,
            None => return,
        };

        let result = self
            .inner
            .http
            .get(format!("{}/api/status", url.trim_end_matches('/')))
            .send()
            .await;

        let outcome = match result {
            Ok(resp) if resp.status().is_success() => {
                resp.json::<Value>().await.map_err(|e| e.to_string())
            }
            Ok(resp) => Err(format!("HTTP {}", resp.status().as_u16())),
            Err(e) => Err(e.to_string()),
        };

        match outcome {
            Ok(status) => self.on_poll_ok(app, status),
            Err(err) => self.on_poll_err(app, err),
        }
    }

    fn on_poll_ok<R: Runtime>(&self, app: &AppHandle<R>, status: Value) {
        let ts = now_ms();
        let state_str = status
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN")
            .to_string();
        let is_active = compute_active(&status);

        let mut alerts: Vec<Alert> = Vec::new();
        let point = status_to_point(&status, ts, &state_str);
        let should_persist;

        {
            let mut st = self.inner.st();

            // Transition detection (only after we've seen a prior state).
            if let Some(prev) = st.last_state.clone() {
                if prev != state_str {
                    match state_str.as_str() {
                        "ERROR" => {
                            let msg = status
                                .get("error_message")
                                .or_else(|| status.get("error"))
                                .and_then(Value::as_str)
                                .unwrap_or("The kiln entered an error state.")
                                .to_string();
                            alerts.push(Alert::Error(msg));
                        }
                        "COMPLETE" => alerts.push(Alert::Complete),
                        _ => {}
                    }
                }
            }

            st.latest = Some(status.clone());
            st.last_state = Some(state_str.clone());
            st.is_active = is_active;
            st.reachable = true;
            st.last_ok = Some(ts);
            st.last_error = None;
            st.first_fail = None;
            st.conn_lost_alerted = false;

            push_history(&mut st.history, point.clone());

            let due = ts - st.last_persist >= HISTORY_PERSIST_INTERVAL_MS;
            should_persist = due;
            if due {
                st.last_persist = ts;
            }
        }

        if should_persist {
            self.persist_history();
        }

        let _ = app.emit("kiln://status", &status);
        let _ = app.emit("kiln://sample", &point);
        let _ = app.emit("kiln://monitoring", self.monitoring_status());

        self.fire_alerts(app, alerts);
    }

    fn on_poll_err<R: Runtime>(&self, app: &AppHandle<R>, err: String) {
        let ts = now_ms();
        let mut alerts: Vec<Alert> = Vec::new();
        {
            let mut st = self.inner.st();
            st.reachable = false;
            st.last_error = Some(err);
            if st.first_fail.is_none() {
                st.first_fail = Some(ts);
            }
            // Only alert when we lose a kiln that was actively firing.
            let outage = st.first_fail.map(|f| ts - f).unwrap_or(0);
            if st.is_active && !st.conn_lost_alerted && outage >= CONN_LOST_ALERT_MS {
                st.conn_lost_alerted = true;
                alerts.push(Alert::ConnectionLost);
            }
        }
        let _ = app.emit("kiln://monitoring", self.monitoring_status());
        self.fire_alerts(app, alerts);
    }

    fn fire_alerts<R: Runtime>(&self, app: &AppHandle<R>, alerts: Vec<Alert>) {
        for alert in alerts {
            let (title, body) = match alert {
                Alert::Error(msg) => ("Kiln error", msg),
                Alert::Complete => (
                    "Firing complete",
                    "The kiln has finished its firing schedule.".to_string(),
                ),
                Alert::ConnectionLost => (
                    "Kiln unreachable",
                    "Lost connection to the kiln for over 10 minutes while firing.".to_string(),
                ),
            };
            let _ = app.notification().builder().title(title).body(body).show();
        }
    }

    fn persist_history(&self) {
        let dir = self.inner.dir().clone();
        let Some(dir) = dir else {
            return;
        };
        let csv = {
            let st = self.inner.st();
            history_to_csv(&st.history)
        };
        // Keep the ~KBs write off the poll task's executor thread.
        tauri::async_runtime::spawn_blocking(move || {
            let _ = std::fs::write(dir.join("kiln_history.csv"), csv);
        });
    }

    /// Spawn the always-on supervisor poll loop. Runs for the life of the
    /// process; while the foreground service is engaged the OS keeps the
    /// process (and this task) alive in the background.
    pub fn spawn_supervisor<R: Runtime>(&self, app: AppHandle<R>) {
        let monitor = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                monitor.poll_once(&app).await;
                tokio::time::sleep(monitor.cadence()).await;
            }
        });
    }
}

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

fn push_history(history: &mut Vec<HistoryPoint>, point: HistoryPoint) {
    // Enforce a strictly-increasing timeline: drop non-advancing samples so a
    // backward clock adjustment can't corrupt the cutoff-based eviction below.
    if let Some(last) = history.last() {
        if point.t <= last.t {
            return;
        }
    }
    history.push(point);
    let cutoff = now_ms() - HISTORY_MAX_AGE_MS;
    if let Some(idx) = history.iter().position(|p| p.t >= cutoff) {
        if idx > 0 {
            history.drain(0..idx);
        }
    }
    if history.len() > HISTORY_MAX_POINTS {
        let overflow = history.len() - HISTORY_MAX_POINTS;
        history.drain(0..overflow);
    }
}

fn status_to_point(status: &Value, ts: i64, state: &str) -> HistoryPoint {
    let temp = status
        .get("current_temp")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let target = status
        .get("target_temp")
        .and_then(Value::as_f64)
        .filter(|t| *t > 0.0);
    HistoryPoint {
        t: ts,
        temp,
        target,
        state: state.to_string(),
    }
}

fn history_to_csv(history: &[HistoryPoint]) -> String {
    let mut out = String::from("t_ms,temp,target,state\n");
    for p in history {
        let target = p.target.map(|t| t.to_string()).unwrap_or_default();
        out.push_str(&format!("{},{},{},{}\n", p.t, p.temp, target, p.state));
    }
    out
}

fn parse_history_csv(csv: &str) -> Vec<HistoryPoint> {
    let mut points = Vec::new();
    for line in csv.lines().skip(1) {
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() < 4 {
            continue;
        }
        let Ok(t) = cols[0].parse::<i64>() else {
            continue;
        };
        let temp = cols[1].parse::<f64>().unwrap_or(0.0);
        let target = cols[2].parse::<f64>().ok();
        points.push(HistoryPoint {
            t,
            temp,
            target,
            state: cols[3].to_string(),
        });
    }
    // Drop anything already outside the retention window.
    let cutoff = now_ms() - HISTORY_MAX_AGE_MS;
    points.retain(|p| p.t >= cutoff);
    points
}
