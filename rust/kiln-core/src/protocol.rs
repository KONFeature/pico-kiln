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
//! - Two presentation-only fields from the Python template are intentionally
//!   dropped: `profile_name` (the human profile name — `profile.rs` deliberately
//!   has none; the app already knows which profile it started) and
//!   `scheduled_profile` (the scheduler's status dict — an app-side concern). The
//!   reference's per-field `round(...)` is presentation too and is not applied.

use crate::profile::StepKind;
use crate::state::{KilnError, KilnState};
use crate::tuner::TuningMode;

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
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the name is empty.
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
    /// Start running a firing profile by filename. (`RUN_PROFILE`)
    RunProfile { profile: ProfileName },
    /// Resume a previously interrupted profile, with recovery context. The
    /// optional fields mirror `CommandMessage.resume_profile`'s defaults of
    /// `None` (let the controller recompute). (`RESUME_PROFILE`)
    ResumeProfile {
        profile: ProfileName,
        elapsed_seconds: f64,
        last_logged_temp: Option<f64>,
        current_temp: Option<f64>,
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
        max_temp: Option<f64>,
    },
    /// Stop auto-tuning. (`STOP_TUNING`)
    StopTuning,
    /// Liveness ping for the cross-core channel. (`PING`)
    Ping,
    /// Schedule a profile to start at `start_time` (Unix seconds). (`SCHEDULE_PROFILE`)
    ScheduleProfile {
        profile: ProfileName,
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

/// A status snapshot from the control loop (Core 1) to the application layer
/// (Core 0). Replaces the `StatusMessage` templates; a flat `Copy` struct so it
/// drops straight into an `embassy-sync` `Watch`/`Channel`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Status {
    /// Unix seconds at capture (injected; `time.time()` in the reference).
    pub timestamp: f64,
    pub state: KilnState,
    pub current_temp: f64,
    pub target_temp: f64,
    pub ssr_output: f64,
    /// Seconds since the run started.
    pub elapsed: f64,
    /// Typed fault reason, if the controller is in error.
    pub error: Option<KilnError>,
    pub step_index: Option<usize>,
    /// The reference's `step_name` (the step *type*), typed.
    pub step_kind: Option<StepKind>,
    pub total_steps: Option<usize>,
    /// Target ramp rate for the current step (°C/h); `0` when not applicable.
    pub desired_rate: f64,
    /// Seconds elapsed within the current step.
    pub step_elapsed: f64,
    pub is_recovering: bool,
    pub recovery_target_temp: Option<f64>,
    /// Measured rate over the controller's window (°C/h).
    pub measured_rate: f64,
}

impl Status {
    /// The idle snapshot — mirrors the reference `_status_template` defaults.
    pub const fn idle() -> Self {
        Self {
            timestamp: 0.0,
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
            measured_rate: 0.0,
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

    #[test]
    fn message_types_match_comms_py_constants() {
        // Exact integers from kiln/comms.py MessageType (1..=10).
        let name = ProfileName::new("cone6.json").unwrap();
        assert_eq!(Command::RunProfile { profile: name }.message_type(), 1);
        assert_eq!(
            Command::ResumeProfile {
                profile: name,
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
}
