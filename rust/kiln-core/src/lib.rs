//! `kiln-core` — hardware-free control logic for the pico-kiln controller.
//!
//! This crate is the safe, testable heart of the controller: PID, and (later)
//! the state machine, profiles, tuner, and rate monitor. It has **zero
//! dependencies** and is `#![no_std]` so the *exact same code* that runs on the
//! RP2350 also runs under `cargo test` on your laptop.
//!
//! The porting strategy is "prove equivalence, then build outward": every
//! module is validated against golden data captured from the original
//! MicroPython implementation before any hardware is involved. See
//! `tests/replay_pid.rs`.
#![cfg_attr(not(test), no_std)]

pub mod gain_schedule;
pub mod pid;
pub mod profile;
pub mod protocol;
pub mod rate_monitor;
pub mod recovery;
pub mod scheduler;
pub mod ssr_schedule;
pub mod state;
pub mod temp_filter;
pub mod tuner;

pub use gain_schedule::{GainSchedule, Gains};
pub use pid::{Pid, PidStats};
pub use profile::{Profile, ProfileError, Step, StepKind};
pub use protocol::{Command, ProfileName, ProtocolError, Status};
pub use rate_monitor::TempHistory;
pub use recovery::{check_recovery, LastLogEntry, RecoveryDecision, RecoveryReason};
pub use scheduler::{ScheduleError, ScheduledProfileQueue};
pub use ssr_schedule::SsrSchedule;
pub use state::{ControllerConfig, KilnController, KilnError, KilnState};
pub use temp_filter::{TempError, TempFilter};
pub use tuner::{TuningMode, TuningStage, ZieglerNicholsTuner};
