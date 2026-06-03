//! The async Core 1 task — a thin `embassy` wrapper around [`Controller`].
//!
//! This is the default control loop: drain one command, run one [`iterate`], push
//! any status snapshot to the `Watch`, then either throttle (on a fault) or run
//! the 10 Hz SSR sub-ticks across the outer tick. The watchdog is fed inside
//! `iterate` on a clean pass, so nothing here feeds it.
//!
//! **Flash-pause caveat.** The cross-core flash-write handshake (Oracle Q1) — by
//! which Core 0 asks Core 1 to force the SSR off and spin in RAM while a CSV row
//! is flushed — is intentionally **not** here: it needs raw-register access that
//! is `kiln-firmware`'s job. A firmware that performs blocking flash writes
//! should drive Core 1 with its own loop built from the same public
//! [`Controller`] methods ([`Controller::iterate`], [`Controller::ssr_subtick`],
//! [`Controller::force_ssr_off`]), inserting the handshake into the sub-tick
//! loop. This default `run` suits the host sim and any firmware that avoids the
//! hazard (e.g. background-DMA flash).
//!
//! [`iterate`]: Controller::iterate

use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::channel::Receiver;
use embassy_sync::watch::Sender;
use embassy_time::{Duration, Instant, Timer};

use kiln_core::protocol::{Command, Status};
use kiln_hal::platform::{SsrOutput, TempSensor, Watchdog};

use crate::Controller;

/// Drive `controller` forever. `commands` is the Core 0 → Core 1 command queue;
/// `status` is the latest-value status broadcast; `wall_clock` returns Unix
/// wall-clock seconds (Core 0 keeps it NTP-synced via a shared offset). Time for
/// the control math comes from the monotonic `embassy_time::Instant`.
pub async fn run<M, S, O, W, const NCMD: usize, const NWATCH: usize>(
    mut controller: Controller<S, O, W>,
    commands: Receiver<'_, M, Command, NCMD>,
    status: Sender<'_, M, Status, NWATCH>,
    wall_clock: impl Fn() -> f64,
) where
    M: RawMutex,
    S: TempSensor,
    O: SsrOutput,
    W: Watchdog,
{
    let (sub_ticks, sub_ms) = controller.timing();
    loop {
        let cmd = commands.try_receive().ok();
        let now_ms = Instant::now().as_millis();
        let outcome = controller.iterate(cmd, now_ms, wall_clock());

        if let Some(snapshot) = outcome.publish {
            status.send(snapshot);
        }

        if outcome.faulted {
            Timer::after(Duration::from_secs(1)).await;
            continue;
        }

        for _ in 0..sub_ticks {
            Timer::after(Duration::from_millis(sub_ms)).await;
            let _ = controller.ssr_subtick(Instant::now().as_millis());
        }
    }
}
