# Firmware Observability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore live + persistent observability to the Rust firmware: a `log`-facade text logger feeding a RAM ring (live SSE tail over the always-on USB-NCM web server) and a rotating, boot-pruned flash diagnostic log, both gated by two `config.json` knobs.

**Architecture:** A new dependency-free `kiln-log` crate holds the pure primitives (byte ring, line formatter, rotation policy). `kiln-app` gains a `#[cfg(feature="embassy")] logging` module with the global `log::Log` impl, three bounded channels, a Core 0 drain task, a Core 0 flash-writer task, and two picoserve handlers (`/api/logs` snapshot, `/api/logs/stream` SSE). Producers on either core only `try_send` (drop-oldest) into a channel — the safety loop never blocks or touches flash. `kiln-firmware` installs the logger before the core split and spawns the two tasks on Core 0.

**Tech Stack:** Rust `no_std`, embassy 0.10 (`embassy-sync` Channel/PubSubChannel, `embassy-time`), picoserve 0.18 (SSE `EventSource`), littlefs2 via the existing `Storage` trait, `log` 0.4 facade.

**Reference spec:** `docs/superpowers/specs/2026-06-06-firmware-observability-design.md`

---

## File Structure

| File | Responsibility | New/Modified |
|---|---|---|
| `rust/kiln-log/Cargo.toml` | crate manifest, zero external deps | Create |
| `rust/kiln-log/src/lib.rs` | re-exports + consts | Create |
| `rust/kiln-log/src/ring.rs` | `Ring<N>` byte ring | Create |
| `rust/kiln-log/src/format.rs` | `format_line`, `tag_of` | Create |
| `rust/kiln-log/src/rotation.rs` | `should_rotate`, `can_append`, `boot_prune_count` | Create |
| `rust/Cargo.toml` | add `kiln-log` workspace member | Modify |
| `rust/kiln-app/Cargo.toml` | add `log` + `kiln-log` under `embassy` feature | Modify |
| `rust/kiln-app/src/config.rs` | `LogLevel` enum + two config fields | Modify |
| `rust/kiln-app/src/api.rs` | `Directory::Diag` variant | Modify |
| `rust/kiln-app/src/logging.rs` | embassy logger, channels, tasks, handlers | Create |
| `rust/kiln-app/src/lib.rs` | `pub mod logging` under `embassy` | Modify |
| `rust/kiln-app/src/server.rs` | two new routes in `build_app` | Modify |
| `rust/kiln-firmware/Cargo.toml` | enable `log` feature on net/wifi crates | Modify |
| `rust/kiln-firmware/src/platform.rs` | `Diag` path mapping + dir create | Modify |
| `rust/kiln-firmware/src/main.rs` | `init()` before split, `set_clock` + spawn tasks | Modify |
| `rust/config.example.json` | document the two knobs | Modify |

**Build/test commands used throughout:**
- Pure host tests: `cd rust && cargo test -p kiln-log` and `cargo test -p kiln-app`
- Firmware compile check (needs the ARM toolchain; see `kiln-firmware/.cargo/config.toml`):
  `cd rust/kiln-firmware && CC_thumbv8m_main_none_eabihf=/path/to/arm-none-eabi-gcc cargo build --release`

---

## Task 1: `kiln-log` crate — `Ring<N>` byte ring

**Files:**
- Create: `rust/kiln-log/Cargo.toml`
- Create: `rust/kiln-log/src/lib.rs`
- Create: `rust/kiln-log/src/ring.rs`
- Modify: `rust/Cargo.toml`

- [ ] **Step 1: Create the crate manifest**

Create `rust/kiln-log/Cargo.toml`:

```toml
[package]
name = "kiln-log"
version = "0.1.0"
edition = "2021"
description = "Pure, dependency-free logging primitives for the pico-kiln firmware: a byte ring buffer, the plain-text line formatter, and the flash rotation/retention policy. Host-tested; no embassy, no heapless, no log — so it never pulls a build script into the pure host-test build (see ../TESTING.md)."
license = "PolyForm-Noncommercial-1.0.0"
publish = false

[dependencies]
```

- [ ] **Step 2: Add `kiln-log` to the workspace members**

In `rust/Cargo.toml`, change the `members` line:

```toml
members = ["kiln-core", "kiln-hal", "kiln-control", "kiln-app", "kiln-sim"]
```

to:

```toml
members = ["kiln-core", "kiln-hal", "kiln-control", "kiln-app", "kiln-sim", "kiln-log"]
```

- [ ] **Step 3: Create the crate root with the rotation constants**

Create `rust/kiln-log/src/lib.rs`:

```rust
//! Pure logging primitives for the pico-kiln firmware — no `core`-external deps,
//! so this crate adds no build script to the pure host-test build (see
//! ../TESTING.md). The embassy-coupled glue (the `log::Log` impl, channels,
//! tasks, picoserve handlers) lives in `kiln-app::logging`.
#![cfg_attr(not(test), no_std)]

pub mod format;
pub mod ring;
pub mod rotation;

pub use format::{format_line, tag_of};
pub use ring::Ring;
pub use rotation::{boot_prune_count, can_append, should_rotate};

/// Rotate the active diag file once it reaches this size.
pub const MAX_FILE_BYTES: u32 = 64 * 1024;
/// Absolute cap across all diag files; runtime appends hard-stop here.
pub const MAX_TOTAL_BYTES: u32 = 256 * 1024;
/// Boot prune deletes oldest-first until total drops below this (¾ of the cap).
pub const PRUNE_TARGET_BYTES: u32 = 192 * 1024;
/// Boot prune also drops files older than this (7 days, in seconds).
pub const MAX_AGE_SECS: i64 = 7 * 24 * 3600;
```

- [ ] **Step 4: Write the failing `Ring` tests**

Create `rust/kiln-log/src/ring.rs`:

```rust
//! A fixed-capacity byte ring of newline-delimited records — the live-tail
//! snapshot buffer. Whole records are pushed; on overflow the oldest bytes are
//! dropped. `snapshot` returns a clean, line-aligned, valid-UTF-8 view.

/// Fixed-capacity byte ring. `N` is the buffer size in bytes.
pub struct Ring<const N: usize> {
    buf: [u8; N],
    /// Index of the oldest byte.
    start: usize,
    /// Number of valid bytes currently stored (<= N).
    len: usize,
}

impl<const N: usize> Ring<N> {
    pub const fn new() -> Self {
        Self {
            buf: [0u8; N],
            start: 0,
            len: 0,
        }
    }

    /// Append `bytes`, dropping oldest bytes if it would overflow. A record
    /// longer than `N` keeps only its last `N` bytes.
    pub fn push(&mut self, bytes: &[u8]) {
        for &b in bytes {
            let pos = (self.start + self.len) % N;
            self.buf[pos] = b;
            if self.len == N {
                // Full: advance start, overwriting the oldest byte.
                self.start = (self.start + 1) % N;
            } else {
                self.len += 1;
            }
        }
    }

    /// Copy the current contents into `out`, starting just after the first
    /// newline so a partially-overwritten leading record is dropped (keeps the
    /// result line-aligned and valid UTF-8). Returns the number of bytes written.
    pub fn snapshot(&self, out: &mut [u8]) -> usize {
        // Linearize into out first.
        let mut tmp_len = 0usize;
        let cap = out.len().min(self.len);
        for i in 0..self.len {
            if tmp_len == cap {
                break;
            }
            out[tmp_len] = self.buf[(self.start + i) % N];
            tmp_len += 1;
        }
        // If the buffer has wrapped (len == N), the first line may be partial:
        // skip to just after the first newline.
        if self.len == N {
            if let Some(nl) = out[..tmp_len].iter().position(|&b| b == b'\n') {
                out.copy_within(nl + 1..tmp_len, 0);
                return tmp_len - (nl + 1);
            }
        }
        tmp_len
    }
}

impl<const N: usize> Default for Ring<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_snapshot_below_capacity() {
        let mut r: Ring<64> = Ring::new();
        r.push(b"hello\n");
        r.push(b"world\n");
        let mut out = [0u8; 64];
        let n = r.snapshot(&mut out);
        assert_eq!(&out[..n], b"hello\nworld\n");
    }

    #[test]
    fn wrap_drops_oldest_and_partial_leading_line() {
        // Capacity 16: push three 6-byte records (18 bytes) -> wraps.
        let mut r: Ring<16> = Ring::new();
        r.push(b"aaaa\n"); // 5
        r.push(b"bbbb\n"); // 5  (10 total)
        r.push(b"cccc\n"); // 5  (15 total, fits)
        r.push(b"dddd\n"); // 5  -> overflow by 4, start advances
        let mut out = [0u8; 16];
        let n = r.snapshot(&mut out);
        let s = core::str::from_utf8(&out[..n]).unwrap();
        // Leading partial record dropped; result is line-aligned and valid UTF-8.
        assert!(s.ends_with("dddd\n"));
        assert!(!s.contains("aaaa"));
        assert_eq!(s.as_bytes().iter().filter(|&&b| b == b'\n').count() as usize, s.matches('\n').count());
    }

    #[test]
    fn snapshot_truncates_to_out_len() {
        let mut r: Ring<64> = Ring::new();
        r.push(b"line-one\n");
        let mut out = [0u8; 4];
        let n = r.snapshot(&mut out);
        assert!(n <= 4);
    }
}
```

- [ ] **Step 5: Run the tests to verify they fail (modules `format`/`rotation` not yet present)**

Run: `cd rust && cargo test -p kiln-log`
Expected: FAIL to compile — `format.rs` and `rotation.rs` referenced by `lib.rs` do not exist yet.

- [ ] **Step 6: Create stub `format.rs` and `rotation.rs` so the crate compiles**

Create `rust/kiln-log/src/format.rs`:

```rust
//! Plain-text log line formatting. Filled in Task 2.
```

Create `rust/kiln-log/src/rotation.rs`:

```rust
//! Flash rotation/retention policy. Filled in Task 3.
```

Temporarily comment the unresolved re-exports in `lib.rs` so the ring tests can run:

```rust
pub use format::{format_line, tag_of};
```
→ comment out for now:
```rust
// pub use format::{format_line, tag_of};   // added in Task 2
// pub use rotation::{boot_prune_count, can_append, should_rotate}; // added in Task 3
```
(Leave `pub use ring::Ring;` active.)

- [ ] **Step 7: Run the ring tests to verify they pass**

Run: `cd rust && cargo test -p kiln-log`
Expected: PASS — 3 ring tests pass.

- [ ] **Step 8: Commit**

```bash
cd rust && git add kiln-log/Cargo.toml kiln-log/src/lib.rs kiln-log/src/ring.rs kiln-log/src/format.rs kiln-log/src/rotation.rs Cargo.toml
git commit -m "feat(kiln-log): byte ring buffer for live-tail snapshots"
```

---

## Task 2: `kiln-log` — `format_line` + `tag_of`

**Files:**
- Modify: `rust/kiln-log/src/format.rs`
- Modify: `rust/kiln-log/src/lib.rs`

- [ ] **Step 1: Write the failing formatter tests**

Replace `rust/kiln-log/src/format.rs` with:

```rust
//! Plain-text log line formatting: `HH:MM:SS LEVEL tag: message\n`, written into
//! any `core::fmt::Write` sink (the embassy side passes a `heapless::String`).
//! Time is rendered as a wall-clock-of-day derived from `secs` (Unix seconds, or
//! an uptime fallback supplied by the caller); only the time-of-day is shown.

use core::fmt::{self, Write};

/// Map a `log` record target (a module path like `kiln_app::server`) to a short
/// tag. Unknown targets fall back to the last `::`-separated segment.
pub fn tag_of(target: &str) -> &str {
    match target {
        t if t.starts_with("kiln_control") => "ctrl",
        t if t.starts_with("kiln_core") => "ctrl",
        t if t.starts_with("cyw43") => "wifi",
        t if t.starts_with("embassy_net") || t.starts_with("smoltcp") => "net",
        t if t.starts_with("embassy_usb") => "usb",
        t if t.starts_with("picoserve") => "web",
        t if t.starts_with("kiln_app::server") => "web",
        t if t.starts_with("kiln_app::logging") => "log",
        t if t.starts_with("kiln_app") => "app",
        t => t.rsplit("::").next().unwrap_or(t),
    }
}

/// Write one formatted line (including the trailing newline) into `w`.
///
/// `secs` is a time value in seconds (Unix wall-clock when known, else an uptime
/// fallback); only `secs % 86400` is shown as `HH:MM:SS`. If `w` runs out of
/// capacity mid-write it returns `Err` and the caller is responsible for
/// guaranteeing the trailing newline (the embassy side does this).
pub fn format_line<W: Write>(
    w: &mut W,
    secs: i64,
    level: &str,
    tag: &str,
    args: fmt::Arguments,
) -> fmt::Result {
    let day = secs.rem_euclid(86_400);
    let h = day / 3600;
    let m = (day % 3600) / 60;
    let s = day % 60;
    write!(w, "{:02}:{:02}:{:02} {} {}: ", h, m, s, level, tag)?;
    w.write_fmt(args)?;
    w.write_char('\n')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_timestamp_level_tag_message() {
        let mut s = String::new();
        // 01:02:03 == 3723 seconds into the day.
        format_line(&mut s, 3723, "INFO", "ctrl", format_args!("temp={}", 812)).unwrap();
        assert_eq!(s, "01:02:03 INFO ctrl: temp=812\n");
    }

    #[test]
    fn wraps_seconds_into_day() {
        let mut s = String::new();
        // One full day + 1 second -> 00:00:01.
        format_line(&mut s, 86_401, "WARN", "net", format_args!("x")).unwrap();
        assert!(s.starts_with("00:00:01 WARN net: x"));
    }

    #[test]
    fn tag_mapping() {
        assert_eq!(tag_of("kiln_control::controller"), "ctrl");
        assert_eq!(tag_of("cyw43::runner"), "wifi");
        assert_eq!(tag_of("embassy_net::tcp"), "net");
        assert_eq!(tag_of("picoserve::routing"), "web");
        assert_eq!(tag_of("some_unknown::deep::leaf"), "leaf");
    }
}
```

- [ ] **Step 2: Re-enable the `format` re-export in `lib.rs`**

In `rust/kiln-log/src/lib.rs`, uncomment:

```rust
pub use format::{format_line, tag_of};
```

- [ ] **Step 3: Run the tests to verify they fail then pass**

Run: `cd rust && cargo test -p kiln-log`
Expected: PASS — ring + format tests pass (if a compile error appears first, it is because Step 2's re-export now resolves; fix any typo and re-run).

- [ ] **Step 4: Commit**

```bash
cd rust && git add kiln-log/src/format.rs kiln-log/src/lib.rs
git commit -m "feat(kiln-log): plain-text line formatter + target->tag mapping"
```

---

## Task 3: `kiln-log` — rotation/retention policy

**Files:**
- Modify: `rust/kiln-log/src/rotation.rs`
- Modify: `rust/kiln-log/src/lib.rs`

- [ ] **Step 1: Write the failing rotation tests**

Replace `rust/kiln-log/src/rotation.rs` with:

```rust
//! Pure flash rotation/retention decisions. The embassy flash-writer executes
//! what these return; all size/age policy lives here so it is host-testable.

use crate::{MAX_AGE_SECS, MAX_FILE_BYTES, MAX_TOTAL_BYTES, PRUNE_TARGET_BYTES};

/// One existing diag file, as seen by a directory scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagEntry {
    /// Size in bytes.
    pub size: u32,
    /// Last-modified Unix seconds (0 if unknown).
    pub mtime: i64,
}

/// True when the active file has reached the per-file rotation size.
pub fn should_rotate(active_size: u32) -> bool {
    active_size >= MAX_FILE_BYTES
}

/// True while there is still room to append under the hard total cap.
pub fn can_append(total: u32) -> bool {
    total < MAX_TOTAL_BYTES
}

/// How many of the OLDEST files (entries sorted oldest-first, i.e. ascending
/// suffix) to delete at boot so that afterwards: total < `PRUNE_TARGET_BYTES`
/// AND no remaining file is older than `MAX_AGE_SECS`.
///
/// `now` is the current Unix time; pass `0` when the clock is not yet synced to
/// skip the age check (size bounding still applies).
pub fn boot_prune_count(entries: &[DiagEntry], now: i64) -> usize {
    let mut total: u32 = entries.iter().map(|e| e.size).sum();
    let mut k = 0usize;

    // Size: drop oldest until under target.
    while total >= PRUNE_TARGET_BYTES && k < entries.len() {
        total -= entries[k].size;
        k += 1;
    }

    // Age: drop further leading (oldest) files that are expired. Only meaningful
    // with a real clock (now > 0); files are oldest-first so expiry is contiguous
    // from the front.
    if now > 0 {
        while k < entries.len() && (now - entries[k].mtime) > MAX_AGE_SECS {
            k += 1;
        }
    }

    k
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(size: u32, mtime: i64) -> DiagEntry {
        DiagEntry { size, mtime }
    }

    #[test]
    fn rotate_and_append_boundaries() {
        assert!(!should_rotate(MAX_FILE_BYTES - 1));
        assert!(should_rotate(MAX_FILE_BYTES));
        assert!(can_append(MAX_TOTAL_BYTES - 1));
        assert!(!can_append(MAX_TOTAL_BYTES));
    }

    #[test]
    fn no_prune_when_under_target_and_fresh() {
        let files = [e(50 * 1024, 1000), e(50 * 1024, 2000)]; // 100 KiB < 192 KiB
        assert_eq!(boot_prune_count(&files, 3000), 0);
    }

    #[test]
    fn size_prune_drops_oldest_until_under_target() {
        // 4 x 64 KiB = 256 KiB >= 192 KiB target. Drop oldest until < 192 KiB:
        // 256 -> 192 (drop 1, still == target, keep going) -> 128 (drop 2). k=2.
        let files = [
            e(64 * 1024, 100),
            e(64 * 1024, 200),
            e(64 * 1024, 300),
            e(64 * 1024, 400),
        ];
        assert_eq!(boot_prune_count(&files, 500), 2);
    }

    #[test]
    fn age_prune_drops_expired_even_when_small() {
        // Small total (under target) but the oldest two are > 7 days old.
        let now = 100 * 24 * 3600;
        let files = [
            e(1024, now - 9 * 24 * 3600), // expired
            e(1024, now - 8 * 24 * 3600), // expired
            e(1024, now - 1 * 24 * 3600), // fresh
        ];
        assert_eq!(boot_prune_count(&files, now), 2);
    }

    #[test]
    fn age_check_skipped_without_clock() {
        let files = [e(1024, 0), e(1024, 0)];
        assert_eq!(boot_prune_count(&files, 0), 0);
    }
}
```

- [ ] **Step 2: Re-enable the `rotation` re-export in `lib.rs`**

In `rust/kiln-log/src/lib.rs`, uncomment:

```rust
pub use rotation::{boot_prune_count, can_append, should_rotate};
```

Also export the `DiagEntry` type by adding it to that line:

```rust
pub use rotation::{boot_prune_count, can_append, should_rotate, DiagEntry};
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cd rust && cargo test -p kiln-log`
Expected: PASS — ring + format + rotation tests all pass.

- [ ] **Step 4: Commit**

```bash
cd rust && git add kiln-log/src/rotation.rs kiln-log/src/lib.rs
git commit -m "feat(kiln-log): size+age boot prune policy + rotation predicates"
```

---

## Task 4: `config.rs` — `LogLevel` enum + two config fields

**Files:**
- Modify: `rust/kiln-app/src/config.rs`
- Modify: `rust/config.example.json`

This task touches `kiln-app`'s pure layer only — no new dependencies (mirrors how
`FixedStr` keeps the layer dependency-free).

- [ ] **Step 1: Write failing config tests**

Add to the `#[cfg(test)] mod tests` block in `rust/kiln-app/src/config.rs` (find it near the bottom of the file, after the existing tests):

```rust
    #[test]
    fn log_level_defaults_to_info_and_flash_on() {
        let c = KilnConfig::default();
        assert_eq!(c.log_level, LogLevel::Info);
        assert!(c.log_to_flash);
    }

    #[test]
    fn parse_overrides_log_keys() {
        let c = parse(r#"{"LOG_LEVEL": "debug", "LOG_TO_FLASH": false}"#).unwrap();
        assert_eq!(c.log_level, LogLevel::Debug);
        assert!(!c.log_to_flash);
    }

    #[test]
    fn parse_log_level_off() {
        let c = parse(r#"{"LOG_LEVEL": "off"}"#).unwrap();
        assert_eq!(c.log_level, LogLevel::Off);
    }

    #[test]
    fn bad_log_level_is_an_error() {
        assert!(parse(r#"{"LOG_LEVEL": "verbose"}"#).is_err());
    }

    #[test]
    fn write_json_round_trips_log_keys() {
        let mut c = KilnConfig::default();
        c.log_level = LogLevel::Warn;
        c.log_to_flash = false;
        let mut s = heapless::String::<2048>::new();
        // write_json is host-testable via core::fmt::Write on a String.
        let mut std_s = String::new();
        c.write_json(&mut std_s).unwrap();
        let _ = s; // (heapless not needed here)
        let reparsed = parse(&std_s).unwrap();
        assert_eq!(reparsed.log_level, LogLevel::Warn);
        assert!(!reparsed.log_to_flash);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cd rust && cargo test -p kiln-app log_`
Expected: FAIL — `LogLevel`, `c.log_level`, `c.log_to_flash` do not exist.

- [ ] **Step 3: Add the `LogLevel` enum**

In `rust/kiln-app/src/config.rs`, add this near the other small enums (e.g. just before `pub struct KilnConfig`, around line 131):

```rust
/// Diagnostic log verbosity, mapped to `log::LevelFilter` by `kiln-app::logging`.
/// Kept as a local enum so the pure config layer pulls in no `log` dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    /// Parse the `LOG_LEVEL` string; `None` for an unknown value.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "off" => LogLevel::Off,
            "error" => LogLevel::Error,
            "warn" => LogLevel::Warn,
            "info" => LogLevel::Info,
            "debug" => LogLevel::Debug,
            _ => return None,
        })
    }

    /// The lowercase wire string (for `write_json`).
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Off => "off",
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
        }
    }
}
```

- [ ] **Step 4: Add the two struct fields**

In `rust/kiln-app/src/config.rs`, in the `struct KilnConfig` "Logging + recovery + watchdog" section (after `pub watchdog_timeout: u32,` at line 190), add:

```rust
    pub log_level: LogLevel,
    pub log_to_flash: bool,
```

- [ ] **Step 5: Add the defaults**

In the `impl Default for KilnConfig`, after `watchdog_timeout: 8000,` (line 251), add:

```rust
            log_level: LogLevel::Info,
            log_to_flash: true,
```

- [ ] **Step 6: Add `write_json` output**

In `write_json`, after the `j.int("WATCHDOG_TIMEOUT", ...)?;` line (line 372), add:

```rust
        j.string("LOG_LEVEL", self.log_level.as_str())?;
        j.boolean("LOG_TO_FLASH", self.log_to_flash)?;
```

- [ ] **Step 7: Add the `apply_key` arms**

In `apply_key`, after the `"WATCHDOG_TIMEOUT" => ...` arm (line 478), add:

```rust
        "LOG_LEVEL" => cfg.log_level = LogLevel::parse(r.parse_string()?).ok_or(ConfigError::BadValue)?,
        "LOG_TO_FLASH" => cfg.log_to_flash = r.parse_bool()?,
```

- [ ] **Step 8: Run config tests to verify they pass**

Run: `cd rust && cargo test -p kiln-app`
Expected: PASS. If a pre-existing golden test asserts the full `/api/config` JSON
string, update it to include `"LOG_LEVEL":"info","LOG_TO_FLASH":true` immediately
after the `WATCHDOG_TIMEOUT` field, then re-run.

- [ ] **Step 9: Document the knobs in the example config**

In `rust/config.example.json`, change line 40 from:

```json
  "WATCHDOG_TIMEOUT": 8000
}
```

to:

```json
  "WATCHDOG_TIMEOUT": 8000,
  "LOG_LEVEL": "info",
  "LOG_TO_FLASH": true
}
```

- [ ] **Step 10: Commit**

```bash
cd rust && git add kiln-app/src/config.rs config.example.json
git commit -m "feat(config): LOG_LEVEL + LOG_TO_FLASH knobs"
```

---

## Task 5: `Directory::Diag` variant + flash path mapping

**Files:**
- Modify: `rust/kiln-app/src/api.rs:56-76`
- Modify: `rust/kiln-firmware/src/platform.rs` (path mapping ~486-496, dir create ~1356)

- [ ] **Step 1: Write a failing parse test for the new variant**

In `rust/kiln-app/src/api.rs`, find the `#[cfg(test)]` tests for `Directory` (or add one) and add:

```rust
    #[test]
    fn parses_diag_directory() {
        assert_eq!(Directory::parse("diag"), Some(Directory::Diag));
        assert_eq!(Directory::Diag.as_str(), "diag");
    }
```

(If `api.rs` has no test module, add at the bottom:)

```rust
#[cfg(test)]
mod diag_dir_tests {
    use super::*;

    #[test]
    fn parses_diag_directory() {
        assert_eq!(Directory::parse("diag"), Some(Directory::Diag));
        assert_eq!(Directory::Diag.as_str(), "diag");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd rust && cargo test -p kiln-app diag`
Expected: FAIL — no `Directory::Diag` variant.

- [ ] **Step 3: Add the `Diag` variant + parse + as_str**

In `rust/kiln-app/src/api.rs`, update the enum (line 56-59):

```rust
pub enum Directory {
    Profiles,
    Logs,
    Diag,
}
```

In `Directory::parse` (line 64-70) add the arm:

```rust
            "logs" => Some(Directory::Logs),
            "diag" => Some(Directory::Diag),
            _ => None,
```

In `Directory::as_str` (line 72-76) add the arm:

```rust
            Directory::Logs => "logs",
            Directory::Diag => "diag",
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd rust && cargo test -p kiln-app diag`
Expected: PASS.

- [ ] **Step 5: Map `Diag` to a flash path in the firmware**

In `rust/kiln-firmware/src/platform.rs`, find the two `match dir` blocks for the
directory path (around lines 486-496) and add the `Diag` arm to each.

First block (trailing-slash prefix):

```rust
        Directory::Profiles => "profiles/",
        Directory::Logs => "logs/",
        Directory::Diag => "diag/",
```

Second block (`path!` directory handle):

```rust
        Directory::Profiles => path!("profiles"),
        Directory::Logs => path!("logs"),
        Directory::Diag => path!("diag"),
```

- [ ] **Step 6: Create the `diag/` directory at boot**

In `rust/kiln-firmware/src/platform.rs`, find the dir-create block (around line
1356) and add the diag dir:

```rust
        let _ = fs.create_dir(path!("profiles"));
        let _ = fs.create_dir(path!("logs"));
        let _ = fs.create_dir(path!("diag"));
```

- [ ] **Step 7: Verify the firmware still compiles**

Run: `cd rust/kiln-firmware && CC_thumbv8m_main_none_eabihf=/path/to/arm-none-eabi-gcc cargo build --release`
Expected: builds (the `match` arms are now exhaustive). If you do not have the ARM
toolchain handy, at minimum confirm `cargo test -p kiln-app diag` passes (Step 4);
the firmware match exhaustiveness is verified at the final build (Task 9).

- [ ] **Step 8: Commit**

```bash
cd rust && git add kiln-app/src/api.rs kiln-firmware/src/platform.rs
git commit -m "feat: add Directory::Diag (diag/) for diagnostic logs"
```

---

## Task 6: `kiln-app/src/logging.rs` — logger, channels, drain task

**Files:**
- Modify: `rust/kiln-app/Cargo.toml`
- Create: `rust/kiln-app/src/logging.rs`
- Modify: `rust/kiln-app/src/lib.rs`

This module is `#[cfg(feature = "embassy")]`; it is verified by the firmware build,
not host tests.

- [ ] **Step 1: Add `log` + `kiln-log` to the `embassy` feature**

In `rust/kiln-app/Cargo.toml`, add to `[dependencies]`:

```toml
kiln-log = { path = "../kiln-log", optional = true }
log = { version = "0.4", optional = true }
```

and add them to the `embassy` feature list:

```toml
embassy = [
    "dep:embassy-executor",
    "dep:embassy-time",
    "dep:embassy-sync",
    "dep:embassy-net",
    "dep:picoserve",
    "dep:heapless",
    "dep:kiln-log",
    "dep:log",
]
```

- [ ] **Step 2: Declare the module in `lib.rs`**

In `rust/kiln-app/src/lib.rs`, after the existing `#[cfg(feature = "embassy")] pub mod server;` line, add:

```rust
#[cfg(feature = "embassy")]
pub mod logging;
```

- [ ] **Step 3: Create the logging module (logger + channels + drain task)**

Create `rust/kiln-app/src/logging.rs`:

```rust
//! Core 0 logging glue: the global `log::Log` implementation, the bounded
//! drop-oldest channels both cores feed, the RAM ring (live-tail snapshot), the
//! SSE pub-sub, and the Core 0 drain task. The flash-writer task and the web
//! handlers live in the same module (added in the following tasks).
//!
//! Producers (either core) only `try_send` a pre-formatted line into
//! `LOG_CHANNEL` with drop-oldest on overflow — they never block, never touch
//! flash, and never take a contended lock. The single Core 0 drain task is the
//! sole writer of the ring, the pub-sub, and the flash channel.

use core::cell::Cell;
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::channel::{Channel, TrySendError};
use embassy_sync::pubsub::PubSubChannel;
use static_cell::StaticCell;

use crate::config::LogLevel;
use crate::server::Clock;

/// Bytes per formatted line. Longer messages are truncated (with a guaranteed
/// trailing newline).
pub const LINE_CAP: usize = 128;
/// Depth of the producer->drain channel.
const CHAN_CAP: usize = 32;
/// Depth of the drain->flash channel.
const FLASH_CHAN_CAP: usize = 32;
/// Pub-sub depth / max simultaneous SSE tail clients / publishers.
const PS_CAP: usize = 16;
const PS_SUBS: usize = 2;
const PS_PUBS: usize = 1;
/// RAM ring size in bytes (the `/api/logs` snapshot).
pub const RING_CAP: usize = 8 * 1024;
/// SSE keepalive period.
pub const KEEPALIVE_SECS: u64 = 15;

/// One formatted log line.
pub type LogLine = heapless::String<LINE_CAP>;

/// Producer -> drain task.
pub(crate) static LOG_CHANNEL: Channel<CriticalSectionRawMutex, LogLine, CHAN_CAP> = Channel::new();
/// Drain task -> flash-writer task.
pub(crate) static FLASH_LOG_CHANNEL: Channel<CriticalSectionRawMutex, LogLine, FLASH_CHAN_CAP> =
    Channel::new();
/// Drain task -> live SSE subscribers.
pub(crate) static LOG_PUBSUB: PubSubChannel<
    CriticalSectionRawMutex,
    LogLine,
    PS_CAP,
    PS_SUBS,
    PS_PUBS,
> = PubSubChannel::new();
/// The live-tail snapshot ring (written only by the drain task; read by the
/// snapshot handler).
pub(crate) static LOG_RING: BlockingMutex<CriticalSectionRawMutex, Cell<kiln_log::Ring<RING_CAP>>> =
    BlockingMutex::new(Cell::new(kiln_log::Ring::new()));
/// Whether flash persistence is enabled (the `LOG_TO_FLASH` knob).
pub(crate) static FLASH_ENABLED: AtomicBool = AtomicBool::new(false);
/// Late-bound wall clock (set once Core 0 builds it). Reads fall back to uptime.
static LOG_CLOCK: BlockingMutex<CriticalSectionRawMutex, Cell<Option<&'static dyn Clock>>> =
    BlockingMutex::new(Cell::new(None));

/// The global logger.
struct KilnLogger;

static LOGGER: KilnLogger = KilnLogger;

fn level_to_filter(level: LogLevel) -> log::LevelFilter {
    match level {
        LogLevel::Off => log::LevelFilter::Off,
        LogLevel::Error => log::LevelFilter::Error,
        LogLevel::Warn => log::LevelFilter::Warn,
        LogLevel::Info => log::LevelFilter::Info,
        LogLevel::Debug => log::LevelFilter::Debug,
    }
}

/// Current time in seconds for line stamping: wall-clock when a synced clock is
/// registered, else monotonic uptime.
fn now_secs() -> i64 {
    let wall = LOG_CLOCK.lock(|c| c.get()).and_then(|c| c.unix_seconds());
    wall.unwrap_or_else(|| embassy_time::Instant::now().as_secs() as i64)
}

/// Push a finished line into `LOG_CHANNEL`, dropping the OLDEST queued line if the
/// channel is full (keeps the most recent lines flowing to the live tail). Never
/// blocks — safe to call from the Core 1 control loop.
fn push_line(line: LogLine) {
    if let Err(TrySendError::Full(line)) = LOG_CHANNEL.try_send(line) {
        let _ = LOG_CHANNEL.try_receive();
        let _ = LOG_CHANNEL.try_send(line);
    }
}

impl log::Log for KilnLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let mut line = LogLine::new();
        let secs = now_secs();
        // On capacity overflow `format_line` returns Err with the newline
        // possibly dropped; guarantee a terminating newline so the ring stays
        // line-aligned.
        let res = kiln_log::format_line(
            &mut line,
            secs,
            record.level().as_str(),
            kiln_log::tag_of(record.target()),
            *record.args(),
        );
        if res.is_err() && !line.as_str().ends_with('\n') {
            // Make room for the newline if the string is full.
            if line.len() == LINE_CAP {
                line.pop();
            }
            let _ = line.push('\n');
        }
        push_line(line);
    }

    fn flush(&self) {}
}

/// Install the global logger. Call once, before the core split. The wall clock is
/// bound later via [`set_clock`]; until then lines carry an uptime timestamp.
pub fn init(level: LogLevel, flash_enabled: bool) {
    FLASH_ENABLED.store(flash_enabled, Ordering::Relaxed);
    // `set_logger` may fail only if called twice; ignore on re-init.
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(level_to_filter(level));
}

/// Bind the wall clock once Core 0 has built it (sharpens line timestamps once
/// NTP syncs). Safe to call from Core 0 after [`init`].
pub fn set_clock(clock: &'static dyn Clock) {
    LOG_CLOCK.lock(|c| c.set(Some(clock)));
}

/// Core 0 task: drain `LOG_CHANNEL` and fan each line out to the RAM ring, the
/// live SSE pub-sub, and (when enabled) the flash channel. The sole writer of the
/// ring and the only publisher of the pub-sub.
#[embassy_executor::task]
pub async fn log_drain_task() -> ! {
    static PUBLISHER: StaticCell<
        embassy_sync::pubsub::Publisher<
            'static,
            CriticalSectionRawMutex,
            LogLine,
            PS_CAP,
            PS_SUBS,
            PS_PUBS,
        >,
    > = StaticCell::new();
    let publisher = PUBLISHER.init(LOG_PUBSUB.publisher().unwrap());

    loop {
        let line = LOG_CHANNEL.receive().await;

        // Ring (snapshot). `Cell` here is only ever touched on Core 0; the
        // blocking mutex guards against the snapshot handler reading mid-write.
        LOG_RING.lock(|c| {
            let mut ring = c.replace(kiln_log::Ring::new());
            ring.push(line.as_bytes());
            c.set(ring);
        });

        // Live tail: never blocks; lagging subscribers drop oldest.
        publisher.publish_immediate(line.clone());

        // Flash: drop-oldest, never blocks the drain.
        if FLASH_ENABLED.load(Ordering::Relaxed) {
            if let Err(TrySendError::Full(line)) = FLASH_LOG_CHANNEL.try_send(line) {
                let _ = FLASH_LOG_CHANNEL.try_receive();
                let _ = FLASH_LOG_CHANNEL.try_send(line);
            }
        }
    }
}
```

> **Note on `Cell<Ring>`:** `Ring<N>` is `Copy`-free and large, so `Cell::replace`
> moves it out and back rather than copying. This keeps the ring behind the
> blocking mutex without `RefCell` borrow-panics. The `replace`/`set` pair runs
> entirely on Core 0 with no `.await` between, so it is non-reentrant.

- [ ] **Step 4: Verify it compiles (firmware build pulls the `embassy` feature)**

The `embassy`-gated module is not built by `cargo test -p kiln-app` (default
features). Compile-check it by building `kiln-app` with the feature:

Run: `cd rust && cargo build -p kiln-app --features embassy`
Expected: FAILS to fully link on host (embassy needs the embedded target) BUT
type-checks the module. If your host cannot build embassy at all, defer
verification to the firmware build in Task 9 and ensure the code matches this
plan exactly. (Prefer the firmware build: `cd rust/kiln-firmware && cargo build --release`.)

- [ ] **Step 5: Commit**

```bash
cd rust && git add kiln-app/Cargo.toml kiln-app/src/lib.rs kiln-app/src/logging.rs
git commit -m "feat(kiln-app): global log facade, drop-oldest channels, drain task"
```

---

## Task 7: Flash-writer task (batching, rotation, hard-stop, boot prune)

**Files:**
- Modify: `rust/kiln-app/src/logging.rs`

- [ ] **Step 1: Append the flash-writer task to `logging.rs`**

Add to the bottom of `rust/kiln-app/src/logging.rs`:

```rust
use crate::api::Directory;
use crate::server::Storage;
use crate::timefmt::write_iso;

/// Batch buffer for flash appends.
const FLASH_BUF_CAP: usize = 512;
/// Flush the batch at least this often (seconds) so lines reach flash promptly.
const FLASH_FLUSH_SECS: u64 = 2;
/// Max diag files we scan at boot (suffix bookkeeping). Far above the retention
/// budget's worst case (256 KiB / 64 KiB = 4 active files; extras only from many
/// short boots between prunes).
const MAX_SCAN_FILES: usize = 64;

/// Build the `diag-NNNNNN.log` name for a suffix.
fn diag_name(suffix: u32) -> heapless::String<24> {
    let mut s = heapless::String::new();
    let _ = core::fmt::Write::write_fmt(&mut s, format_args!("diag-{:06}.log", suffix));
    s
}

/// Parse the numeric suffix from a `diag-NNNNNN.log` name.
fn parse_suffix(name: &str) -> Option<u32> {
    let stem = name.strip_prefix("diag-")?.strip_suffix(".log")?;
    stem.parse().ok()
}

/// Core 0 task: persist diag lines to `diag/diag-NNNNNN.log`, rotating by size and
/// hard-stopping at the total cap. Runs a boot prune first. Idle (and writes
/// nothing) while `LOG_TO_FLASH` is false — the drain task simply never forwards.
#[embassy_executor::task]
pub async fn diag_flash_task(
    storage: &'static dyn Storage,
    clock: &'static dyn Clock,
) -> ! {
    use embassy_time::{with_timeout, Duration, Instant};

    // --- Boot prune + active-file selection ---------------------------------
    let now0 = clock.unix_seconds().unwrap_or(0);
    let mut suffixes: heapless::Vec<u32, MAX_SCAN_FILES> = heapless::Vec::new();
    let mut entries: heapless::Vec<(u32, kiln_log::DiagEntry), MAX_SCAN_FILES> =
        heapless::Vec::new();
    storage.for_each(Directory::Diag, &mut |name, size, modified| {
        if let Some(suf) = parse_suffix(name) {
            let _ = suffixes.push(suf);
            let _ = entries.push((suf, kiln_log::DiagEntry { size: size as u32, mtime: modified as i64 }));
        }
    });
    // Sort oldest-first (ascending suffix) for the prune policy.
    entries.sort_unstable_by_key(|(suf, _)| *suf);
    let sorted: heapless::Vec<kiln_log::DiagEntry, MAX_SCAN_FILES> =
        entries.iter().map(|(_, e)| *e).collect();
    let drop_k = kiln_log::boot_prune_count(&sorted, now0);
    for &(suf, _) in entries.iter().take(drop_k) {
        let _ = storage.remove(Directory::Diag, &diag_name(suf));
    }

    // Active suffix = one past the highest surviving suffix.
    let max_suffix = entries.iter().skip(drop_k).map(|(s, _)| *s).max();
    let mut suffix = max_suffix.map(|m| m + 1).unwrap_or(0);
    // Recompute surviving total for the running hard-stop accounting.
    let mut total: u32 = entries.iter().skip(drop_k).map(|(_, e)| e.size).sum();

    let mut active_size: u32 = 0;
    let mut stopped = false;
    let mut warned = false;

    // Open the first active file with a header line.
    open_new_diag_file(storage, clock, suffix, &mut total, &mut active_size, &mut stopped);

    // --- Batch loop ----------------------------------------------------------
    let mut buf = heapless::String::<FLASH_BUF_CAP>::new();
    let mut last_flush = Instant::now();

    loop {
        // Wait for a line, but wake to flush on a timeout so batches don't sit.
        match with_timeout(Duration::from_secs(FLASH_FLUSH_SECS), FLASH_LOG_CHANNEL.receive()).await {
            Ok(line) => {
                if buf.len() + line.len() > buf.capacity() {
                    flush_batch(storage, &diag_name(suffix), &mut buf, &mut active_size, &mut total, stopped);
                }
                let _ = buf.push_str(&line);
            }
            Err(_) => {} // timeout: fall through to flush
        }

        let due = Instant::now().duration_since(last_flush).as_secs() >= FLASH_FLUSH_SECS;
        if !buf.is_empty() && due {
            flush_batch(storage, &diag_name(suffix), &mut buf, &mut active_size, &mut total, stopped);
            last_flush = Instant::now();
        }

        // Hard-stop at the total cap (no mid-run deletion; reclaimed on reboot).
        if !stopped && !kiln_log::can_append(total) {
            stopped = true;
            if !warned {
                warned = true;
                log::warn!(target: "kiln_app::logging", "diag flash budget full, pausing flash logging until reboot");
            }
        }

        // Rotate to a fresh file once the active one is large enough.
        if !stopped && kiln_log::should_rotate(active_size) {
            suffix += 1;
            open_new_diag_file(storage, clock, suffix, &mut total, &mut active_size, &mut stopped);
        }
    }
}

/// Append `buf` to the active file (unless stopped), updating size accounting, and
/// clear `buf`.
fn flush_batch(
    storage: &'static dyn Storage,
    name: &str,
    buf: &mut heapless::String<FLASH_BUF_CAP>,
    active_size: &mut u32,
    total: &mut u32,
    stopped: bool,
) {
    if buf.is_empty() {
        return;
    }
    if !stopped {
        if storage
            .append(Directory::Diag, name, buf.as_bytes(), false)
            .is_ok()
        {
            *active_size += buf.len() as u32;
            *total += buf.len() as u32;
        }
    }
    buf.clear();
}

/// Create a fresh `diag-NNNNNN.log` with a one-line ISO/uptime header (truncating
/// any pre-existing file at that name). Resets `active_size`.
fn open_new_diag_file(
    storage: &'static dyn Storage,
    clock: &'static dyn Clock,
    suffix: u32,
    total: &mut u32,
    active_size: &mut u32,
    stopped: &mut bool,
) {
    if *stopped {
        return;
    }
    let name = diag_name(suffix);
    let mut header = heapless::String::<64>::new();
    let _ = header.push_str("# diag ");
    match clock.unix_seconds() {
        Some(secs) => {
            let _ = write_iso(&mut header, secs);
        }
        None => {
            let _ = core::fmt::Write::write_fmt(
                &mut header,
                format_args!("uptime+{}s", embassy_time::Instant::now().as_secs()),
            );
        }
    }
    let _ = header.push('\n');
    // `create = true` truncates, so the header defines a fresh file.
    if storage
        .append(Directory::Diag, &name, header.as_bytes(), true)
        .is_ok()
    {
        *active_size = header.len() as u32;
        *total += header.len() as u32;
    }
}
```

> **`write_iso` check:** `kiln_app::timefmt::write_iso(w, unix_seconds)` is the
> existing CSV-row timestamp formatter. Confirm its signature in
> `kiln-app/src/timefmt.rs`; if it takes different argument order/types, adjust the
> two call sites above to match (it is the same function `csv::write_row` uses).

- [ ] **Step 2: Verify it compiles via the firmware build**

Run: `cd rust/kiln-firmware && CC_thumbv8m_main_none_eabihf=/path/to/arm-none-eabi-gcc cargo build --release`
Expected: still fails to *link* only if other tasks are incomplete, but this module
must type-check. Resolve any signature mismatch with `write_iso`, `for_each`, or
`heapless::Vec::sort_unstable_by_key` (heapless `Vec` derefs to a slice, so
`sort_unstable_by_key` is available).

- [ ] **Step 3: Commit**

```bash
cd rust && git add kiln-app/src/logging.rs
git commit -m "feat(kiln-app): diag flash-writer task (batch, rotate, hard-stop, boot prune)"
```

---

## Task 8: Web handlers + routes (`/api/logs`, `/api/logs/stream`)

**Files:**
- Modify: `rust/kiln-app/src/logging.rs` (handlers + SSE source)
- Modify: `rust/kiln-app/src/server.rs` (routes in `build_app`)

- [ ] **Step 1: Append the handlers + SSE `EventSource` to `logging.rs`**

Add to the bottom of `rust/kiln-app/src/logging.rs`:

```rust
use picoserve::response::sse::{EventSource, EventStream, EventWriter};
use picoserve::response::{IntoResponse, Response};
use picoserve::io::Write as _;

/// `GET /api/logs` — plain-text snapshot of the RAM ring (the "what's on screen
/// now" view).
pub async fn logs_snapshot() -> impl IntoResponse {
    let mut buf = [0u8; RING_CAP];
    let n = LOG_RING.lock(|c| {
        let ring = c.replace(kiln_log::Ring::new());
        let n = ring.snapshot(&mut buf);
        c.set(ring);
        n
    });
    // The ring keeps records line-aligned and valid UTF-8; fall back to empty on
    // the rare boundary error.
    let body: heapless::String<RING_CAP> = core::str::from_utf8(&buf[..n])
        .ok()
        .and_then(|s| heapless::String::try_from(s).ok())
        .unwrap_or_default();
    Response::new(picoserve::response::StatusCode::OK, body)
}

/// The SSE event stream: one `log` event per new line, keepalive every
/// `KEEPALIVE_SECS`. picoserve drives this until the client disconnects.
struct LogEvents;

impl EventSource for LogEvents {
    async fn write_events<W: picoserve::io::Write>(
        self,
        mut writer: EventWriter<'_, W>,
    ) -> Result<(), W::Error> {
        use embassy_sync::pubsub::WaitResult;
        use embassy_time::{with_timeout, Duration};

        // Refuse extra clients beyond PS_SUBS (returns Err) — bounded fan-out.
        let mut sub = match LOG_PUBSUB.subscriber() {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };
        loop {
            match with_timeout(Duration::from_secs(KEEPALIVE_SECS), sub.next_message()).await {
                Ok(WaitResult::Message(line)) => {
                    // Strip the trailing newline (SSE framing adds its own).
                    let data = line.as_str().trim_end_matches('\n');
                    writer.write_event("log", data).await?;
                }
                Ok(WaitResult::Lagged(_)) => { /* missed some lines; keep going */ }
                Err(_) => writer.write_keepalive().await?,
            }
        }
    }
}

/// `GET /api/logs/stream` — live tail over Server-Sent Events.
pub async fn logs_stream() -> impl IntoResponse {
    EventStream(LogEvents)
}
```

> **`heapless::String::try_from(&str)`:** available in heapless 0.8. If the exact
> constructor differs, build the body with a `push_str` loop instead:
> `let mut body = heapless::String::<RING_CAP>::new(); let _ = body.push_str(core::str::from_utf8(&buf[..n]).unwrap_or(""));`

- [ ] **Step 2: Import `picoserve::io::Write` correctly**

The `EventWriter` generic bound uses picoserve's own `Write`. Ensure the
`use picoserve::io::Write as _;` at the top of the added block does not collide
with `core::fmt::Write` already used in the file — they are imported under
different names (`_` alias and fully-qualified `core::fmt::Write::write_fmt`), so
there is no clash. If the compiler reports ambiguity, remove the `as _` import and
write the bound as `W: picoserve::io::Write` (already fully qualified in the
signature).

- [ ] **Step 3: Add the two routes to `build_app`**

In `rust/kiln-app/src/server.rs`, in the `web` module's `build_app` (around line
544, right after the `/api/reboot` route), add:

```rust
                .route("/api/reboot", post(reboot).options(cors_preflight))
                .route(
                    "/api/logs",
                    get(crate::logging::logs_snapshot).options(cors_preflight),
                )
                .route(
                    "/api/logs/stream",
                    get(crate::logging::logs_stream).options(cors_preflight),
                )
```

> **Recursion limit:** `lib.rs` already sets `#![recursion_limit = "512"]` for the
> router type. Two more routes are well within it; no change needed.

- [ ] **Step 4: Verify the firmware compiles**

Run: `cd rust/kiln-firmware && CC_thumbv8m_main_none_eabihf=/path/to/arm-none-eabi-gcc cargo build --release`
Expected: type-checks the handlers + routes. Fix any picoserve `Handler`/`Content`
trait mismatch (a bare `async fn() -> impl IntoResponse` is a valid picoserve
handler; `Response::new(StatusCode, heapless::String)` and `EventStream` both
implement `IntoResponse`).

- [ ] **Step 5: Commit**

```bash
cd rust && git add kiln-app/src/logging.rs kiln-app/src/server.rs
git commit -m "feat(kiln-app): /api/logs snapshot + /api/logs/stream SSE live tail"
```

---

## Task 9: Firmware wiring — install logger, spawn tasks, enable lib logs

**Files:**
- Modify: `rust/kiln-firmware/Cargo.toml`
- Modify: `rust/kiln-firmware/src/main.rs`

- [ ] **Step 1: Enable the `log` feature on the networking/wifi crates**

In `rust/kiln-firmware/Cargo.toml`, update these dependency lines to add the `log`
feature so their internal diagnostics flow into our facade:

```toml
embassy-net = { version = "0.9", features = ["tcp", "udp", "dns", "dhcpv4", "medium-ethernet", "proto-ipv4", "log"] }
cyw43 = { version = "0.7", features = ["firmware-logs", "log"] }
embassy-rp = { version = "0.10", features = ["rp235xa", "time-driver", "critical-section-impl", "log"] }
picoserve = { version = "0.18", features = ["embassy", "log"] }
```

(Leave the other dependency lines unchanged.)

- [ ] **Step 2: Install the logger before the core split**

In `rust/kiln-firmware/src/main.rs`, in `fn main()`, right after
`let config = platform::load_config(storage);` (around line 78), add:

```rust
    // Install the global logger before the split so BOTH cores can log from the
    // outset. The wall clock is bound later (core0_main) once it exists.
    kiln_app::logging::init(config.log_level.into_app_log_level(), config.log_to_flash);
```

> **Conversion:** `config.log_level` is `kiln_app::config::LogLevel`; `init` takes
> the same type. So pass it directly — drop the `.into_app_log_level()` and write:
>
> ```rust
>     kiln_app::logging::init(config.log_level, config.log_to_flash);
> ```

- [ ] **Step 3: Bind the clock + spawn the two tasks in `core0_main`**

In `rust/kiln-firmware/src/main.rs`, in `async fn core0_main`, just after
`let clock: &'static NtpClock = platform::init_clock();` (around line 332), add:

```rust
    // Bind the wall clock to the logger and start the Core 0 logging tasks.
    kiln_app::logging::set_clock(clock);
    spawner.spawn(kiln_app::logging::log_drain_task().unwrap());
    spawner.spawn(kiln_app::logging::diag_flash_task(storage, clock).unwrap());
```

> **`storage` in scope:** `core0_main` already has the `&'static dyn Storage` it
> uses for `AppState` / recovery. Confirm the local binding name (it is the value
> passed as `AppState.storage`); if it is `flash` or similar, use that name. The
> `&'static FlashStorage` coerces to `&'static dyn Storage` at the call.

> **Spawner availability:** `core0_main` receives a `Spawner` (it spawns the web,
> wifi, ntp, lcd, csv tasks). Use the same `spawner` binding those spawns use.

- [ ] **Step 4: Full firmware build**

Run: `cd rust/kiln-firmware && CC_thumbv8m_main_none_eabihf=/path/to/arm-none-eabi-gcc cargo build --release`
Expected: PASS — the image links. Note the reported flash size; it should grow
modestly (the `log` features add formatting strings). If the linker reports the
2560 KiB FLASH region overflow, see the Risks section.

- [ ] **Step 5: Run the full host test suite (no regressions)**

Run: `cd rust && cargo test`
Expected: PASS — all workspace host tests (kiln-core/app/control/hal/sim/log).

- [ ] **Step 6: Commit**

```bash
cd rust && git add kiln-firmware/Cargo.toml kiln-firmware/src/main.rs
git commit -m "feat(firmware): install log facade, spawn drain+flash tasks, enable lib logs"
```

---

## Task 10: Device verification (manual)

**No code changes.** Flash the image and verify on hardware.

- [ ] **Step 1: Flash + live tail over USB-NCM**

Flash the release UF2, plug USB, and tail the SSE stream over the USB-NCM IP:

```bash
curl -N http://<device-ip>/api/logs/stream
```

Expected: a stream of `event: log` / `data: ...` records appearing as the kiln
runs (wifi/dhcp/web lines at boot; control/tuner lines during a firing), plus a
`:` keepalive line every ~15 s when idle.

- [ ] **Step 2: Snapshot**

```bash
curl http://<device-ip>/api/logs
```

Expected: the last ~8 KiB of plain-text log lines, line-aligned.

- [ ] **Step 3: Flash persistence + rotation**

Set `LOG_LEVEL` to `debug` in `config.json` (more volume), run for a while, then:

```bash
curl http://<device-ip>/api/files/diag        # lists diag-NNNNNN.log files
curl http://<device-ip>/api/files/diag/diag-000000.log   # downloads one
```

Expected: one or more `diag-NNNNNN.log` files, each ≤ 64 KiB, first line a
`# diag <ISO>` header.

- [ ] **Step 4: Boot prune**

Fill several files (or pre-place >192 KiB of diag files), reboot, and re-list.
Expected: oldest files removed so total < 192 KiB.

- [ ] **Step 5: Off switch**

Set `LOG_LEVEL: "off"`, reboot. Expected: `/api/logs` empty, no new diag files,
SSE stream only sends keepalives. Then set `LOG_TO_FLASH: false` (level back to
`info`): live tail works, no new diag files written.

- [ ] **Step 6: Final commit (if any config tweaks were made during verification)**

```bash
cd rust && git add -A && git commit -m "chore: device-verify observability"
```

---

## Self-Review

**Spec coverage:**
- REPL replacement / live tail → Task 8 (`/api/logs/stream` SSE) + Task 10 Step 1. ✓
- Post-mortem persistent flash → Task 7 (flash-writer) + Task 10 Step 3. ✓
- Minimal RAM/CPU, no Core 1 stall → Task 6 (`push_line` drop-oldest, `try_send`, no flash on producers). ✓
- Off switch (two knobs) → Task 4 (`LogLevel`, `log_to_flash`) + Task 9 (`init`) + Task 10 Step 5. ✓
- Bounded flash, size+age rotation, boot-only prune, runtime hard-stop → Task 3 (policy) + Task 7 (executor). ✓
- New `diag/` dir → Task 5. ✓
- New `kiln-log` crate → Tasks 1–3. ✓
- Plain-text `log` facade, lib logs included → Task 6 (`KilnLogger`) + Task 9 Step 1 (lib `log` features). ✓
- RAM budget ~18 KiB → consts in Task 6 (CHAN_CAP 32, FLASH_CHAN_CAP 32, PS 16, RING 8 KiB). ✓
- No defmt path → not included (dropped per refinement). ✓

**Type consistency:** `LogLevel` (config) ↔ `level_to_filter` (logging); `LogLine = heapless::String<128>` used by all three channels + drain + flash; `kiln_log::Ring<RING_CAP>` shared by drain + snapshot; `kiln_log::DiagEntry` produced in Task 7 scan, consumed by `boot_prune_count` from Task 3; `Directory::Diag` from Task 5 used in Task 7 + platform mapping; `init`/`set_clock`/`log_drain_task`/`diag_flash_task` defined in Tasks 6–7, called in Task 9.

**Known follow-ups (out of scope, noted for later):**
- Web app UI "Diagnostics" view + tail panel that consume `/api/logs*` and
  `/api/files/diag` (separate front-end task).
- The `write_iso` / `for_each` / `storage`-binding-name confirmations flagged
  inline are mechanical and resolved at first compile.

## Risks
- **Flash size growth from lib `log` features.** If the image overflows the
  2560 KiB region, drop the heaviest contributor first (`embassy-net`'s `log`
  feature is the chattiest) — our own + cyw43/picoserve logs still cover the
  important paths.
- **`heapless::String::try_from`** constructor name — fall back to the `push_str`
  loop shown inline if it does not resolve.
- **picoserve handler trait bounds** — bare `async fn` handlers returning
  `impl IntoResponse` are valid; if `Response::new` typing fights you, return the
  `heapless::String` body directly (it implements `Content`/`IntoResponse`).
