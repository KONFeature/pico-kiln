//! Cross-core message types — the typed data model from `kiln/comms.py`.
//!
//! `comms.py`'s *concurrency* (the `ThreadSafeQueue`, `ReadyFlag`, `QuietMode`)
//! is reimplemented with `embassy-sync` primitives in the firmware crates; only
//! the message *shapes* belong here, and they cross the Core 1 ↔ Core 0 boundary
//! as plain typed values rather than dicts:
//!
//! - [`Command`] replaces `MessageType` + `CommandMessage` (Core 0 → Core 1). The
//!   integer tag is preserved by [`Command::message_type`] so it lines up 1:1 with
//!   the reference constants.
//! - [`Status`] replaces the `StatusMessage` templates (Core 1 → Core 0), as a
//!   flat `Copy` snapshot.
//!
//! Keeping with `kiln-core`'s rules, there are **no heap strings and no dicts**:
//!
//! - States and errors are the typed [`KilnState`] / [`KilnError`] enums, not
//!   strings; `comms.state_to_string` is a web-boundary concern and is not ported.
//! - The step "name" (the reference's `'ramp' | 'hold' | 'cooling'`) is the typed
//!   [`StepKind`].
//! - A profile *filename* is unavoidably text, so it rides in a fixed-capacity
//!   [`ProfileName`] (no allocation), capped at [`MAX_PROFILE_NAME`] bytes.
//! - The reference's per-field `round(...)` is presentation and is not applied
//!   here; `kiln-app` formats numbers at the web boundary.
//!
//! Three reference status fields ride along as typed `Copy` snapshots because the
//! scheduler and the tuner both live on **Core 1** (the control loop owns the
//! active profile, drains the scheduler each tick, and runs the tuner), so the
//! application layer (Core 0) can only learn them from the status message — it
//! cannot reconstruct them:
//!
//! - `profile_name` → [`Status::profile_name`] (Core 1 tracks the running
//!   profile's filename; `kiln-app` echoes it to `/api/status` and names the CSV).
//! - `scheduled_profile` → [`Status::scheduled`] ([`ScheduledSnapshot`]).
//! - the tuning `tuning` sub-dict → [`Status::tuning`] ([`TuningSnapshot`]).
//!   The presentation-only step *name* string and human error message are
//!   reconstructed in `kiln-app` from `(mode, step_index)` / `stage`.

use crate::profile::{Profile, StepKind};
use crate::state::{KilnError, KilnState};
use crate::tuner::{TuningMode, TuningStage};

/// Maximum profile-filename length carried by a [`Command`], in bytes.
pub const MAX_PROFILE_NAME: usize = 64;

/// Why building a protocol value failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    /// A profile filename exceeded [`MAX_PROFILE_NAME`] bytes.
    NameTooLong,
}

/// A fixed-capacity profile filename — the only text in the protocol, stored
/// inline so the crate stays allocation-free.
#[derive(Debug, Clone, Copy)]
pub struct ProfileName {
    bytes: [u8; MAX_PROFILE_NAME],
    len: usize,
}

impl ProfileName {
    /// Build from a string, rejecting anything longer than [`MAX_PROFILE_NAME`].
    pub fn new(s: &str) -> Result<Self, ProtocolError> {
        let src = s.as_bytes();
        if src.len() > MAX_PROFILE_NAME {
            return Err(ProtocolError::NameTooLong);
        }
        let mut bytes = [0u8; MAX_PROFILE_NAME];
        bytes[..src.len()].copy_from_slice(src);
        Ok(Self {
            bytes,
            len: src.len(),
        })
    }

    /// The filename bytes (no trailing padding).
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// The filename as `&str`. Always valid UTF-8: a `ProfileName` can only be
    /// built from a `&str`, so the stored bytes are a UTF-8 prefix.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.as_bytes()).unwrap_or("")
    }

    /// Length in bytes.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the name is empty.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl PartialEq for ProfileName {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for ProfileName {}

/// A command from the application layer (Core 0) to the control loop (Core 1).
///
/// Replaces `comms.MessageType` + `CommandMessage`; see [`Command::message_type`]
/// for the preserved integer tags.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Start running a firing profile. (`RUN_PROFILE`)
    ///
    /// Unlike the reference — where Core 1 reads `profiles/{name}` from disk —
    /// the platform-generic control loop never touches the filesystem, so
    /// `kiln-app` (Core 0, the single flash owner) parses the JSON and ships the
    /// ready-to-run [`Profile`] across the channel. `profile` is the filename,
    /// kept for status, CSV naming, and recovery.
    RunProfile {
        profile: ProfileName,
        parsed: Profile,
    },
    /// Resume a previously interrupted profile, with recovery context. The
    /// optional fields mirror `CommandMessage.resume_profile`'s defaults of
    /// `None` (let the controller recompute). `parsed` is the Core-0-parsed
    /// profile (see [`Command::RunProfile`]). (`RESUME_PROFILE`)
    ResumeProfile {
        profile: ProfileName,
        parsed: Profile,
        elapsed_seconds: f32,
        last_logged_temp: Option<f32>,
        current_temp: Option<f32>,
        step_index: Option<usize>,
    },
    /// Stop the current profile. (`STOP`)
    Stop,
    /// Emergency shutdown — stop and force the SSR off. (`SHUTDOWN`)
    Shutdown,
    /// Start PID auto-tuning. `max_temp` of `None` uses the mode's default.
    /// (`START_TUNING`)
    StartTuning {
        mode: TuningMode,
        max_temp: Option<f32>,
    },
    /// Stop auto-tuning. (`STOP_TUNING`)
    StopTuning,
    /// Liveness ping for the cross-core channel. (`PING`)
    Ping,
    /// Schedule a profile to start at `start_time` (Unix seconds). `parsed` is
    /// the Core-0-parsed profile (see [`Command::RunProfile`]), held by the
    /// control loop's scheduler until it fires. (`SCHEDULE_PROFILE`)
    ScheduleProfile {
        profile: ProfileName,
        parsed: Profile,
        start_time: u64,
    },
    /// Cancel a scheduled profile. (`CANCEL_SCHEDULED`)
    CancelScheduled,
    /// Clear an error state and return to idle. (`CLEAR_ERROR`)
    ClearError,
}

impl Command {
    /// The integer message tag, identical to `comms.MessageType` (1..=10). Kept
    /// so logs/wire traces line up with the MicroPython reference.
    #[cfg(test)]
    pub fn message_type(&self) -> u8 {
        match self {
            Command::RunProfile { .. } => 1,
            Command::ResumeProfile { .. } => 2,
            Command::Stop => 3,
            Command::Shutdown => 4,
            Command::StartTuning { .. } => 5,
            Command::StopTuning => 6,
            Command::Ping => 7,
            Command::ScheduleProfile { .. } => 8,
            Command::CancelScheduled => 9,
            Command::ClearError => 10,
        }
    }
}

/// The delayed-start scheduler's status, mirroring `scheduler.get_status()`
/// (`kiln/scheduler.py`) minus its presentation-only ISO string, which
/// `kiln-app` formats from `start_time` at the web boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScheduledSnapshot {
    /// The queued profile's filename.
    pub profile: ProfileName,
    /// Unix seconds the profile is scheduled to start.
    pub start_time: u64,
    /// Whole seconds remaining until start (`max(0, start_time − now)`).
    pub seconds_until_start: u64,
}

/// The auto-tuner's status, the typed half of `tuner.get_status()`
/// (`kiln/tuner.py:492-516`). Carries everything `/api/tuning/status` renders
/// except the step *name* string and human error message, which `kiln-app`
/// reconstructs from `(mode, step_index)` and `stage` respectively (the only
/// error path is over-max-temp), keeping this `Copy` and `kiln-core` text-free.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TuningSnapshot {
    pub stage: TuningStage,
    pub mode: TuningMode,
    pub max_temp: f32,
    /// Current step index (0-based).
    pub step_index: usize,
    pub total_steps: usize,
    /// Seconds elapsed in the current step (the reference's merged `elapsed`).
    pub step_elapsed: f32,
    /// Fixed SSR output the current step holds (%).
    pub ssr_percent: f32,
    /// The current step's temperature target, if any.
    pub target_temp: Option<f32>,
    /// The current step's timeout (seconds).
    pub timeout: f32,
    /// Whether the current step has detected a plateau.
    pub plateau_detected: bool,
    /// Peak temperature seen during the current step (°C).
    pub peak_temp: f32,
}

/// A status snapshot from the control loop (Core 1) to the application layer
/// (Core 0). Replaces the `StatusMessage` templates; a flat `Copy` struct so it
/// drops straight into an `embassy-sync` `Watch`/`Channel`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Status {
    /// Unix seconds at capture (injected; `time.time()` in the reference).
    /// Wall-clock epoch as an integer `i64` — f32 ulp at ~1.7e9 is 128 s, so it
    /// can never be f32; whole seconds keep it exact and off the soft-float path.
    pub timestamp: i64,
    pub state: KilnState,
    pub current_temp: f32,
    pub target_temp: f32,
    pub ssr_output: f32,
    /// Seconds since the run started.
    pub elapsed: f32,
    /// Typed fault reason, if the controller is in error.
    pub error: Option<KilnError>,
    pub step_index: Option<usize>,
    /// The reference's `step_name` (the step *type*), typed.
    pub step_kind: Option<StepKind>,
    pub total_steps: Option<usize>,
    /// Target ramp rate for the current step (°C/h); `0` when not applicable.
    pub desired_rate: f32,
    /// Seconds elapsed within the current step.
    pub step_elapsed: f32,
    pub is_recovering: bool,
    pub recovery_target_temp: Option<f32>,
    /// Consecutive arrival-band stall-advances without a genuine step
    /// completion in between. Non-zero means the kiln entered its current (or,
    /// on Complete, final) step(s) by giving up near their targets rather than
    /// reaching them — a Complete with `stall_advances > 0` is a degraded
    /// finish, not a true one.
    pub stall_advances: u32,
    /// Measured rate over the controller's window (°C/h).
    pub measured_rate: f32,
    /// The active (running or just-completed) profile's filename, if any.
    pub profile_name: Option<ProfileName>,
    /// The delayed-start scheduler's snapshot, if a profile is queued.
    pub scheduled: Option<ScheduledSnapshot>,
    /// The auto-tuner's snapshot, present while [`KilnState::Tuning`].
    pub tuning: Option<TuningSnapshot>,
}

impl Status {
    /// The idle snapshot — mirrors the reference `_status_template` defaults.
    pub const fn idle() -> Self {
        Self {
            timestamp: 0,
            state: KilnState::Idle,
            current_temp: 0.0,
            target_temp: 0.0,
            ssr_output: 0.0,
            elapsed: 0.0,
            error: None,
            step_index: None,
            step_kind: None,
            total_steps: None,
            desired_rate: 0.0,
            step_elapsed: 0.0,
            is_recovering: false,
            recovery_target_temp: None,
            stall_advances: 0,
            measured_rate: 0.0,
            profile_name: None,
            scheduled: None,
            tuning: None,
        }
    }
}

impl Default for Status {
    fn default() -> Self {
        Self::idle()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_profile() -> Profile {
        Profile::new(&[crate::profile::Step::hold(100.0, 10.0)]).unwrap()
    }

    #[test]
    fn message_types_match_comms_py_constants() {
        // Exact integers from kiln/comms.py MessageType (1..=10).
        let name = ProfileName::new("cone6.json").unwrap();
        assert_eq!(
            Command::RunProfile {
                profile: name,
                parsed: sample_profile(),
            }
            .message_type(),
            1
        );
        assert_eq!(
            Command::ResumeProfile {
                profile: name,
                parsed: sample_profile(),
                elapsed_seconds: 0.0,
                last_logged_temp: None,
                current_temp: None,
                step_index: None,
            }
            .message_type(),
            2
        );
        assert_eq!(Command::Stop.message_type(), 3);
        assert_eq!(Command::Shutdown.message_type(), 4);
        assert_eq!(
            Command::StartTuning {
                mode: TuningMode::Standard,
                max_temp: None
            }
            .message_type(),
            5
        );
        assert_eq!(Command::StopTuning.message_type(), 6);
        assert_eq!(Command::Ping.message_type(), 7);
        assert_eq!(
            Command::ScheduleProfile {
                profile: name,
                parsed: sample_profile(),
                start_time: 0
            }
            .message_type(),
            8
        );
        assert_eq!(Command::CancelScheduled.message_type(), 9);
        assert_eq!(Command::ClearError.message_type(), 10);
    }

    #[test]
    fn profile_name_roundtrips_and_compares_by_content() {
        let a = ProfileName::new("glaze_cone6.json").unwrap();
        assert_eq!(a.as_str(), "glaze_cone6.json");
        assert_eq!(a.len(), 16);
        assert!(!a.is_empty());
        // Equality is by content, not by the zero-padded backing array.
        let b = ProfileName::new("glaze_cone6.json").unwrap();
        assert_eq!(a, b);
        let c = ProfileName::new("bisque.json").unwrap();
        assert_ne!(a, c);
    }

    #[test]
    fn profile_name_rejects_oversize_accepts_boundary() {
        let max = "x".repeat(MAX_PROFILE_NAME);
        assert!(ProfileName::new(&max).is_ok());
        let over = "x".repeat(MAX_PROFILE_NAME + 1);
        assert_eq!(ProfileName::new(&over), Err(ProtocolError::NameTooLong));

        let empty = ProfileName::new("").unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.as_str(), "");
    }

    #[test]
    fn command_carries_typed_payloads() {
        let cmd = Command::StartTuning {
            mode: TuningMode::Safe,
            max_temp: Some(200.0),
        };
        match cmd {
            Command::StartTuning { mode, max_temp } => {
                assert_eq!(mode, TuningMode::Safe);
                assert_eq!(max_temp, Some(200.0));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn idle_status_matches_template_defaults() {
        let s = Status::idle();
        assert_eq!(s.state, KilnState::Idle);
        assert_eq!(s.current_temp, 0.0);
        assert_eq!(s.desired_rate, 0.0);
        assert!(!s.is_recovering);
        assert_eq!(s.error, None);
        assert_eq!(s.step_kind, None);
        assert_eq!(s, Status::default());
    }

    #[test]
    fn status_carries_typed_state_and_error() {
        let s = Status {
            state: KilnState::Error,
            error: Some(KilnError::Stall {
                actual_rate: 1.0,
                min_rate: 5.0,
            }),
            step_kind: Some(StepKind::Ramp),
            ..Status::idle()
        };
        assert_eq!(s.state, KilnState::Error);
        assert_eq!(s.step_kind, Some(StepKind::Ramp));
        assert!(matches!(s.error, Some(KilnError::Stall { .. })));
    }

    #[test]
    fn idle_status_has_no_profile_scheduler_or_tuning() {
        let s = Status::idle();
        assert_eq!(s.profile_name, None);
        assert_eq!(s.scheduled, None);
        assert_eq!(s.tuning, None);
    }

    #[test]
    fn status_carries_profile_scheduler_and_tuning_snapshots() {
        let name = ProfileName::new("cone6.json").unwrap();
        let s = Status {
            state: KilnState::Tuning,
            profile_name: Some(name),
            scheduled: Some(ScheduledSnapshot {
                profile: ProfileName::new("bisque.json").unwrap(),
                start_time: 1_700_000_000,
                seconds_until_start: 3599,
            }),
            tuning: Some(TuningSnapshot {
                stage: TuningStage::Running,
                mode: TuningMode::Standard,
                max_temp: 900.0,
                step_index: 2,
                total_steps: 6,
                step_elapsed: 42.5,
                ssr_percent: 50.0,
                target_temp: None,
                timeout: 1800.0,
                plateau_detected: false,
                peak_temp: 310.0,
            }),
            ..Status::idle()
        };
        assert_eq!(s.profile_name.unwrap().as_str(), "cone6.json");
        let sched = s.scheduled.unwrap();
        assert_eq!(sched.profile.as_str(), "bisque.json");
        assert_eq!(sched.seconds_until_start, 3599);
        let t = s.tuning.unwrap();
        assert_eq!(t.mode, TuningMode::Standard);
        assert_eq!(t.stage, TuningStage::Running);
        assert_eq!(t.step_index, 2);
        assert_eq!(t.peak_temp, 310.0);
        let s2 = s;
        assert_eq!(s, s2);
    }
}
