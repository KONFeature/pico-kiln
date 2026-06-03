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

pub mod pid;

pub use pid::{Pid, PidStats};
