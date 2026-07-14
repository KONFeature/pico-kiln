//! Tauri commands bridging the frontend to the background monitor.

use serde_json::Value;

use super::{HistoryPoint, Monitor, MonitoringStatus};

/// Set (or clear) the kiln base URL the supervisor polls. Called by the app on
/// launch and whenever the user changes the kiln address.
#[tauri::command]
pub fn set_kiln_url(monitor: tauri::State<'_, Monitor>, url: Option<String>) {
    monitor.set_url(url);
}

/// Latest `/api/status` snapshot the monitor has seen (raw firmware JSON), or
/// `null` before the first successful poll.
#[tauri::command]
pub fn get_kiln_status(monitor: tauri::State<'_, Monitor>) -> Option<Value> {
    monitor.snapshot_status()
}

/// The accumulated rolling temperature history (last 4 hours).
#[tauri::command]
pub fn get_kiln_history(monitor: tauri::State<'_, Monitor>) -> Vec<HistoryPoint> {
    monitor.history()
}

/// Monitor health, used to surface the "background monitoring not running"
/// toast in the app.
#[tauri::command]
pub fn monitoring_status(monitor: tauri::State<'_, Monitor>) -> MonitoringStatus {
    monitor.monitoring_status()
}

/// Kick off an immediate poll + short fast-poll burst, e.g. right after the
/// user issues a control command, so the UI reflects the new state quickly even
/// though the firmware takes a second or two to update.
#[tauri::command]
pub fn refresh_kiln(monitor: tauri::State<'_, Monitor>) {
    monitor.request_refresh();
}
