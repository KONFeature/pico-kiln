# Firmware Observability — Design Spec

**Date:** 2026-06-06
**Status:** Approved (ready for implementation plan)
**Scope:** `rust/` firmware only (RP2350 / Pico 2 W). No change to the web app beyond a new log view (its own follow-up).

## Problem

The MicroPython firmware gave live observability for free: run `main.py` over the
REPL and watch `print()` output. The Rust firmware is a flashed binary with no
stdout and **no logging at all** (zero `log`/`defmt`/`println` in-tree today). We
lost the ability to see "what is the kiln doing right now" and to diagnose a bad
firing after the fact.

## Goals

1. **REPL replacement** — live "what's happening now" tail, reachable over the
   always-on USB-NCM link (plug cable → see logs), no debug probe required.
2. **Post-mortem** — persistent, rotated diagnostic log on flash, downloadable via
   the existing web file routes.
3. **Minimal RAM/CPU** — must not impact the Core 1 real-time safety loop.
4. **Off switch** — disable everything (or just flash) from `config.json`.
5. **Bounded flash** — size + age rotation, never exhaust the littlefs partition.

## Non-goals

- No `defmt`/RTT/probe path (explicitly dropped).
- No structured/JSON log format — plain text lines only.
- No runtime level-change endpoint (level is read from config at boot; the
  internal `AtomicU8` leaves the door open but no endpoint ships now).
- Web-app UI work (log viewer / tail panel) is a separate follow-up; this spec
  delivers the firmware surface + endpoints it will consume.

## Decisions (from brainstorming)

| Axis | Decision |
|---|---|
| Persistence | Layered: RAM ring (live) **+** rotating flash file (post-mortem) |
| Facade | `log` crate (already in the dep tree at 0.4.31) |
| Format | Plain text lines: `HH:MM:SS LEVEL tag: msg` |
| Core 1 logging | Allowed, but only via a bounded, non-blocking, drop-oldest channel |
| Config | Two knobs: `log_level` (off..debug) + `log_to_flash` (bool) |
| Flash dir | New `Directory::Diag` → `diag/`, separate from CSV runs in `logs/` |
| Pruning | **Boot-only**, oldest-first, triggered at ≥¾ budget or age-expiry |

## Architecture

### Crate layout

**New crate `kiln-log`** (`no_std`, host-tested; depends only on `core` +
`heapless`). Pure, isolated, no embassy:

- `Ring` — fixed-capacity byte ring of newline-delimited records; the live-tail
  snapshot buffer. Overwrites oldest on wrap.
- `format_line(out, unix_secs, level, target, args)` — writes
  `HH:MM:SS LEVEL tag: msg\n` into a `heapless::String<LINE_CAP>` (no alloc).
- `tag_of(target) -> &str` — maps a `log` record target (module path) to a short
  tag (`ctrl`, `net`, `web`, `fs`, `ntp`, `wifi`, `lcd`, …); unknown → last path
  segment.
- `LevelFilter` — parse `"off"|"error"|"warn"|"info"|"debug"` ↔ `log::LevelFilter`.
- `RotationPolicy` — **pure** decision functions over a directory listing
  `[(name, size, mtime)]`:
  - `runtime_should_rotate(active_size) -> bool` (active ≥ `MAX_FILE_BYTES`)
  - `runtime_can_append(total) -> bool` (total < `MAX_TOTAL_BYTES`)
  - `boot_prune(files, now_secs) -> heapless::Vec<Name>` — the oldest-first set to
    delete so that, afterwards, `total < PRUNE_TARGET_BYTES` **and** no file older
    than `MAX_AGE_SECS` remains.

**`kiln-app`** (under its existing `embassy` feature) — the live, embassy-coupled
parts:

- `KilnLogger: log::Log` + `init(level)`. Installs the global logger and sets
  `log::set_max_level`.
- `LOG_CHANNEL`, `FLASH_LOG_CHANNEL`, `LOG_PUBSUB`, the drain task, the
  flash-writer task.
- SSE `EventSource` impl + the `/api/logs` and `/api/logs/stream` handlers.
- `KilnConfig` fields `log_level`, `log_to_flash` + parse.
- `Directory::Diag`.

**`kiln-firmware`** — wiring only:

- Call `kiln_app::log::init()` **before** the core split so both cores log.
- Create the channels (`StaticCell`) and spawn the drain + flash-writer tasks on
  Core 0.
- Enable the `log` cargo feature on `cyw43`, `embassy-net`, `smoltcp`,
  `embassy-rp`, `picoserve` so their internal diagnostics flow into our facade.

### Data flow

```
log::info!/warn!/…  (any crate, EITHER core)
        │  KilnLogger::log()
        │    1. atomic level check (≈ns) — return early if below threshold
        │    2. format → heapless::String<LINE_CAP> on the caller's stack
        ▼
   LOG_CHANNEL.try_send(line)        Channel<CriticalSectionRawMutex, Line, CHAN_CAP>
        │   multicore-safe; DROP-OLDEST on full → never blocks Core 1
        ▼
   Core 0 drain task: recv(line) →
        ├─► RAM Ring          (sole writer; GET /api/logs snapshot reads it)
        ├─► LOG_PUBSUB.publish (live SSE subscribers)
        └─► if log_to_flash: FLASH_LOG_CHANNEL.try_send(line)  (drop-oldest)
                                   │
                          Core 0 flash-writer task:
                                   batch lines → append diag/diag-NNNNNN.log
                                   rotate at MAX_FILE_BYTES (new file)
                                   hard-stop appends at MAX_TOTAL_BYTES (warn once)
```

Two Core 0 tasks, not one, on purpose:

- **Tail latency is independent of slow flash.** The drain task feeds the RAM ring
  + pubsub immediately; flash is downstream.
- **Flash backpressure is isolated.** `FLASH_LOG_CHANNEL` drop-oldest absorbs flash
  stalls without ever blocking the drain or the producers.

The flash-writer **batches** (flush per ~N KiB or ~few seconds, whichever first)
so the cross-core flash-write handshake — which briefly parks Core 1, the same
mechanism already used for config/CSV writes — fires rarely. No new safety risk
beyond what config/CSV logging already incur.

### Why Core 1 never stalls

- Hot path when a record is below the level threshold: one `AtomicU8` load + a
  compare, then return. `log`'s own `STATIC_MAX_LEVEL` + `max_level()` already
  short-circuit before we run.
- Hot path when logging: format into a stack `heapless::String` (no alloc) +
  `Channel::try_send` (a `CriticalSectionRawMutex` critical section, microseconds)
  with **drop-oldest** on full. Core 1 enqueues and moves on; it never touches
  flash, the network, or a blocking lock.

## RAM budget (tunable consts in `kiln-app`)

| Buffer | Size | Note |
|---|---|---|
| `LOG_CHANNEL` | 32 × `String<128>` ≈ 4 KiB | producers → drain |
| `FLASH_LOG_CHANNEL` | 32 × 128 ≈ 4 KiB | drain → flash writer |
| `LOG_PUBSUB` | 16 × 128, 2 subs ≈ 2 KiB | drain → SSE clients |
| RAM `Ring` | 8 KiB | ≈ last 80–100 lines (snapshot) |
| **Total** | **≈ 18 KiB** | of 512 KiB SRAM → negligible |

- `log_level = off` → producers short-circuit; tasks idle; channels empty.
- `log_to_flash = false` → `FLASH_LOG_CHANNEL` + flash-writer skipped; **zero flash
  wear**; live tail still works.

`LINE_CAP = 128` bytes; longer messages are truncated with a trailing `…`.

## Flash persistence + rotation

Constants (in `kiln-log`, referenced by the flash writer):

```
LINE_CAP          = 128            // bytes per record
MAX_FILE_BYTES    = 64 * 1024      // rotate active file at 64 KiB
MAX_TOTAL_BYTES   = 256 * 1024     // hard cap across all diag files
PRUNE_TRIGGER     = 192 * 1024     // ¾ of total — boot prune kicks in at/above
PRUNE_TARGET      = 192 * 1024     // boot prune deletes oldest-first until below
MAX_AGE_SECS      = 7 * 24 * 3600  // 7 days — boot prune also drops expired files
```

### Files

- Directory: **new `Directory::Diag`** → path segment `diag` (`diag/` in littlefs).
  Kept separate from CSV runs (`logs/`) so the web file list and bulk-clear act on
  each independently. Reuses all existing `Storage` plumbing (list / download /
  delete / `available_bytes`).
- Names: `diag-NNNNNN.log`, zero-padded 6-digit suffix. At boot, scan `diag/`,
  set the active suffix to `max(existing) + 1` (no persisted counter). Oldest =
  lowest suffix (monotonic, so suffix order == creation order).
- The first line of each file stamps the full ISO timestamp once the wall clock is
  known (lines before NTP sync carry the monotonic `HH:MM:SS` derived from uptime;
  acceptable — they still order correctly within a boot).

### Runtime (no deletion)

- Append batched lines to the active `diag-NNNNNN.log`.
- When `active_size ≥ MAX_FILE_BYTES` → start a new file at `suffix + 1`.
- When `total ≥ MAX_TOTAL_BYTES` → **stop appending** (do not delete), emit one
  `WARN fs: diag flash budget full, pausing flash logging until reboot`, and keep
  the RAM ring + SSE tail running. Reclaimed on next boot. This honours
  "only clear on boot" while keeping flash strictly bounded.

### Boot prune (the only deletion path)

Runs once, early in Core 0 bring-up, after the filesystem mounts and before/around
opening the active file:

1. List `diag/` → `[(name, size, mtime)]`, compute `total`.
2. If `total ≥ PRUNE_TRIGGER` **or** any `mtime` older than `MAX_AGE_SECS`:
   call `RotationPolicy::boot_prune(files, now)`; delete the returned names
   (oldest-first) until `total < PRUNE_TARGET` and no expired file remains.
3. `boot_prune` is pure → fully host-tested; the firmware only executes the
   returned delete list via `Storage::remove`.

(Age checks need a wall clock. If NTP hasn't synced at boot prune time, age-expiry
is skipped that boot and only the size trigger applies; the next synced boot
catches expired files. Size bounding — the safety-critical part — never depends on
the clock.)

## Web API

- `GET /api/logs` → `text/plain`; snapshot of the RAM ring (the "what's on screen
  now" view). Bounded by ring size.
- `GET /api/logs/stream` → **SSE** (`text/event-stream`) via picoserve's
  `EventSource`: subscribes to `LOG_PUBSUB`, emits one event per new line, sends a
  keepalive every ~15 s, closes on client disconnect. This is the live-tail / REPL
  replacement; works over USB-NCM with no probe.
- Persistent diag files: served by the **existing** `/api/files/diag/...` routes
  (list / download / delete) once `Directory::Diag` parses. The web app's
  "Diagnostics" view (follow-up) consumes these.

Note: making `diag` a `Directory` variant also exposes it to the existing upload
route. Uploading into `diag/` is harmless (it is only a log dir) and the web UI
will not offer it; not worth special-casing now.

## Config

`kiln-app::config::KilnConfig` gains two fields, parsed at boot alongside the rest
(absent/invalid → defaults, consistent with the existing fail-safe parse):

```jsonc
"log_level":    "info",   // off | error | warn | info | debug — "off" disables ALL logging
"log_to_flash": true      // false → live RAM/SSE tail only, no flash writes/wear
```

Defaults: `log_level = "info"`, `log_to_flash = true`. `config.example.json` and
the `KilnConfig::default()` impl are updated to match.

## Testing

Host `cargo test` (the firmware stays device-verified as today):

- `kiln-log`:
  - `Ring`: append, wrap/overwrite-oldest, snapshot read, exact-fill, oversize line
    truncation.
  - `format_line`: level word, tag mapping, timestamp formatting, truncation with `…`.
  - `LevelFilter::parse`: every variant + invalid → error.
  - `RotationPolicy::boot_prune`: size trigger, age trigger, oldest-first order,
    stops at `PRUNE_TARGET`, no-op when under threshold, mixed size+age.
  - `runtime_should_rotate` / `runtime_can_append` boundaries.
- `kiln-app`:
  - `KilnLogger` level gating (records below threshold never enqueue).
  - drop-oldest behaviour on a full channel.
  - drain ordering into a mock ring + mock flash sink.
  - SSE `EventSource` emits buffered lines (picoserve ships a test `EventSource`
    harness) + keepalive path.

## Components & boundaries (isolation check)

| Unit | Does | Used via | Depends on |
|---|---|---|---|
| `kiln-log::Ring` | hold last-N log bytes | `push`, `snapshot` | `core` |
| `kiln-log::format_line` | render one line | fn call | `heapless` |
| `kiln-log::RotationPolicy` | decide rotate/prune | pure fns | `heapless` |
| `kiln-app::KilnLogger` | `log::Log` → channel | `log::*!` macros | `kiln-log`, `embassy-sync`, `log` |
| `kiln-app` drain task | fan-out to ring/pubsub/flash | spawned | channels |
| `kiln-app` flash-writer | batched append + rotate | spawned | `Storage`, `kiln-log` |
| `kiln-app` log handlers | snapshot + SSE | routes | `LOG_PUBSUB`, `Ring` |
| `kiln-firmware` wiring | init + spawn + features | — | all of the above |

Each unit answers "what / how-used / depends-on" cleanly; the pure logic
(`kiln-log`) is testable without hardware, and the embassy-coupled parts are thin.

## Implementation order (for the plan)

1. `kiln-log` crate + host tests (pure: ring, format, policy, level parse).
2. `KilnConfig` fields + parse + defaults + `config.example.json`.
3. `Directory::Diag` + `FlashStorage` path mapping in `platform.rs`.
4. `KilnLogger` + channels + drain task + pubsub (no flash yet) + host tests.
5. Flash-writer task: batched append, runtime rotate, hard-stop; boot prune.
6. `/api/logs` + `/api/logs/stream` (SSE) handlers + routes.
7. `kiln-firmware` wiring: `init()` before split, spawn tasks, enable lib `log`
   features.
8. Manual device verification: tail over USB-NCM, force rotation, reboot prune.

## Risks / open items

- **SSE concurrency**: budgeted for 2 simultaneous tail clients (`LOG_PUBSUB`
  subs). More clients → bump the const (RAM cost only).
- **Debug-level flood**: at `log_level = debug`, smoltcp/cyw43 are chatty; the
  drop-oldest channels + flash hard-stop bound the blast radius, but the live tail
  may drop lines under sustained flood. Acceptable; that is what `debug` is for.
- **Clock-less early boot**: size bounding is clock-independent; only age-expiry
  pruning waits for a synced boot (documented above).
