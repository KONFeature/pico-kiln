//! `kiln-app` — the Core 0 application layer for the pico-kiln controller, a
//! platform-generic port of `main.py` + `server/*.py`.
//!
//! Split the same way as `kiln-control`: the **exactness-critical pure logic**
//! that guarantees no behavioural drift from the MicroPython server — the status
//! JSON, the CSV schema, the crash-recovery parse, the tuning step-name table,
//! the wall-clock formatting, and the request validation — lives in `core`-only
//! modules that are host-tested. The `embassy` glue (picoserve routes,
//! `embassy-net`, the littlefs CSV logger, and the WiFi/NTP/LCD tasks) is behind
//! the `embassy` feature; `kiln-firmware` hands it a running `Stack` and flash.
#![cfg_attr(not(test), no_std)]
#![cfg_attr(feature = "embassy", feature(impl_trait_in_assoc_type))]
// The picoserve router builds a deeply-nested `Route`/`PathRouter` type, and the
// embassy task pool computes the layout of the resulting handler future; the
// default recursion limit (128) overflows while doing so.
#![recursion_limit = "512"]

pub mod api;
pub mod config;
pub mod csv;
pub mod errors;
pub mod html;
pub mod json;
pub mod profile_json;
pub mod recovery_io;
pub mod timefmt;
pub mod tuning_names;

#[cfg(feature = "embassy")]
pub mod server;
