//! Foreground-service keepalive.
//!
//! On Android, `startService()` runs this in the app's own process/Tokio
//! runtime and keeps that process alive (`START_STICKY`) with a persistent
//! notification while the app is backgrounded. The actual polling is done by
//! the always-on supervisor in `mod.rs`; this service only needs to (a) exist
//! to hold the process open, and (b) stop itself once the kiln settles back to
//! an idle state so the persistent notification goes away.

use std::time::Duration;

use async_trait::async_trait;
use tauri::Runtime;
use tauri_plugin_background_service::{BackgroundService, ServiceContext, ServiceError};

use super::Monitor;

/// How long the kiln must stay non-active before the foreground service
/// demotes itself (covers the COMPLETE → IDLE tail).
const DEMOTE_GRACE: Duration = Duration::from_secs(60);
const CHECK_INTERVAL: Duration = Duration::from_secs(15);

pub struct KilnMonitorService {
    monitor: Monitor,
}

impl KilnMonitorService {
    pub fn new(monitor: Monitor) -> Self {
        Self { monitor }
    }
}

#[async_trait]
impl<R: Runtime> BackgroundService<R> for KilnMonitorService {
    async fn init(&mut self, _ctx: &ServiceContext<R>) -> Result<(), ServiceError> {
        self.monitor.set_fgs_active(true);
        Ok(())
    }

    async fn run(&mut self, ctx: &ServiceContext<R>) -> Result<(), ServiceError> {
        let mut idle_since: Option<std::time::Instant> = None;

        loop {
            tokio::select! {
                _ = ctx.shutdown.cancelled() => break,
                _ = tokio::time::sleep(CHECK_INTERVAL) => {
                    if self.monitor.is_kiln_active() {
                        idle_since = None;
                    } else {
                        let since = idle_since.get_or_insert_with(std::time::Instant::now);
                        if since.elapsed() >= DEMOTE_GRACE {
                            // Idle settled — end run() to tear down the foreground
                            // service. The supervisor keeps polling in-process.
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

impl Drop for KilnMonitorService {
    fn drop(&mut self) {
        self.monitor.set_fgs_active(false);
    }
}
