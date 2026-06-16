# pico-kiln — Python → Rust Migration Parity Audit

**Date:** 2026-06-03
**Branch:** `feat/rust-kiln-core`
**Scope:** 1:1 feature / business-logic parity of the Rust port (`rust/`) against the MicroPython source of truth (`kiln/`, `server/`, `lib/`, `config*.py`, `main.py`, `boot.py`, `static/`).
**Method:** 9 parallel feature-block audits, each reading both sides and cross-checking constants, branches, comparison operators, and edge cases. Evidence is cited as `file:line`.

---

## 1. Executive summary

The **brain is excellent; the device/app glue is where the parity gaps live.**

`kiln-core` — every control decision (PID, tuner, gain schedule, profile math, scheduler, rate monitor, state machine, temp filter, SSR schedule, recovery decision, protocol data model) — is a near-exact, golden-replay-tested port. Operators, constants, and subtle quirks were preserved with unusual discipline. If the audit stopped at `kiln-core` + `kiln-hal`, this would be a ~98% faithful migration.

The divergences cluster in the **two outer halves (`kiln-app`, `kiln-firmware`)**: the web/REST contract, the boot/handshake sequence, recovery *logging* (vs. recovery *decision*), config-default provenance, and the unported LCD. Several are real breaks that ship today; a few are intentional improvements that simply aren't 1:1; and a large surface is **device-only `unimplemented!()` stubs** (WiFi join, flash append, LCD) that cannot be verified from source yet.

### Parity scorecard

| Block | Verdict | Headline risk |
|---|---|---|
| PID + Tuner + Gain schedule | 🟢 ~98% | None in logic; `PID_KI_BASE` fallback split (0.14 vs 0.18) |
| Profile + Scheduler + Rate monitor | 🟢 Strong | New `MAX_STEPS=32` cap (Python unbounded) |
| State management | 🟢 Strong | `get_elapsed_time` idempotency change; `step_index` 0- vs 1-based |
| Control loop + SSR + Temp filter | 🟢 Strong | `PID_KI_BASE` default 0.14 vs Python fallback 0.18; `quiet_mode` gone |
| Comms / Protocol | 🟡 Mixed | `ReadyFlag` + `QuietMode` **not ported**; status queue → lossy `Watch` |
| Web server + REST + HTML | 🔴 Not 1:1 | `{profiles_list}` template unsubstituted; `DELETE …/all` missing; CORS preflight broken |
| Data logging + Recovery + Profile cache | 🟡 Mixed | Recovery-resume CSV convention unwired; `LOGGING_INTERVAL` hardcoded |
| Hardware / HAL + LCD | 🟡 Mixed | Sensor/SSR exact; **LCD entirely unported**; init may omit `start_autoconverting()` |
| Config + WiFi + Boot/Firmware | 🟡 Mixed | Device `config.py` overrides may silently revert to template defaults; no WiFi-retry monitor; no Core-1-ready gate |

### Verdict against the explicitly-requested requirements

| Requirement (from the brief) | Status |
|---|---|
| "same recovery logic" | ⚠️ **Decision** logic = faithful & replay-tested. **Recovery logging** (append to existing file, skip header, emit `RECOVERY` event row) = **unwired / dead code**. Boot-time Core-1-ready recovery gate = missing. |
| "same webserver logic" | ⚠️ Core routes + status JSON faithful, but missing bulk-delete route, broken CORS preflight, dropped success-envelope fields, no 408/413 paths. |
| "same two static html serving with templates" | 🔴 `tuning.html` served fine (no tokens). `index.html`'s single template var `{profiles_list}` is **never substituted** — the on-device page ships the raw placeholder and renders no profile buttons. |
| "same firing logics, profile tracking, pid params tracking" | 🟢 Faithful and golden-tested in `kiln-core`. The only firing-relevant risk is config-default provenance (below). |

---

## 1b. Resolution log (branch `feat/rust-kiln-core`)

All actionable Ugly items + two requested Bad items were implemented. Pure logic
is host-tested (170 host tests green, clippy clean); firmware/embassy wiring is
review-only (see the picoserve note below).

| Item | Status | Notes |
|---|---|---|
| U1 `{profiles_list}` template | ✅ done | New host-tested `kiln-app::html` (`render_profiles_list`/`profile_display_name`/`split_profiles_placeholder`); `page_index` streams prefix+list+suffix via `ApiResponse::Index`. Renders **stems** (matches `/api/run` re-appending `.json`). |
| U2 recovery-resume CSV | ✅ done | `attempt_recovery` returns `RecoveryLog{filename, elapsed_seconds}`; `csv_logger_task` appends to the existing file (no header) + writes the one-shot RECOVERY row (`csv::write_recovery_event_row`, already host-tested). |
| U3 bulk delete `…/logs/all` | ✅ done | `file_delete` special-cases `file=="all"` → `file_delete_all` (idle+logs-only guard, `remove_all`, `{success,deleted_count,deleted_files}`). |
| U4 CORS preflight | ✅ done | `.options(cors_preflight)` on every `/api/*` route (picoserve 0.18 `MethodRouter::options`); 200 + CORS via the `Text` arm. |
| U5 ReadyFlag + QuietMode | ✅ done | `READY: Signal` (Core 1 signals after `build_kiln_io`; Core 0 waits 20 s then `sys_reset` on timeout) + `QUIET: AtomicBool` gating Core 1's status publish during WiFi bring-up. |
| U6 config.json | ✅ done | `rust/config.example.json` (committed) + `rust/config.json` (gitignored, generated from `config.py` incl. WiFi). **Real device values captured from source** — note `PID_KP_BASE=6.11`, `PID_KI_BASE=0.0037`, `PID_KD_BASE=42.534`, `THERMAL_H=0.0`, `THERMAL_T_AMBIENT=10`, `THERMOCOUPLE_TYPE=S`, `MAINS_FREQUENCY=50`, `MAX_RECOVERY_TEMP_DELTA=60` (these differ from §4-U6's earlier estimate, which was a transcription error). Host test asserts the overrides apply + the example file is valid. |
| U7 `LOGGING_INTERVAL` | ✅ done | `csv_logger_task` takes `&KilnConfig`; interval = `config.logging_interval` (TUNING still 2 s). |
| U8 `PID_KI_BASE` → 0.14 | ✅ done | Python getattr fallback + `gain_schedule.rs` tests aligned to 0.14. |
| U9 LCD | ⏸ deferred (as requested) | `lcd_task` no longer spawned; `init_display`/import removed; `TODO(LCD)` left. Driver port still future work. |
| U10 `start_autoconverting()` | ✅ done | `build_kiln_io` DEVICE recipe corrected to call it after `set_noise_filter` (else sensor reads constant 0 °C). |
| Bad-A power-settle | ✅ done | 500 ms `Timer` at the top of `control_task`, before `build_kiln_io` (`boot.py:26`). |
| Bad-B staggered SSR-off | ✅ done | `MultiSsr` gains `falling_edge_ms`; `apply(false)` staggers turn-off pin-by-pin; `force_off`/`all_off` stay immediate. Host-tested (N=2). |
| WiFi retry monitor | ✅ wired | `wifi_monitor_task` spawned on Core 0 + join-retry recipe in `init_network` (device body still stub). |
| Intentional Bad items | kept | Status `Watch`, hourly NTP, pure `elapsed()` — kept per decision. `error`/`error_message`: now emitting **both** keys (React reads `error_message`; static pages + Python read `error`). `step_index`: already correct (0-based; React + `index.html:68` agree) — no change. |

### ⚠️ Pre-existing blocker discovered: the embassy layer doesn't compile against picoserve 0.18

While verifying, I cross-compiled `kiln-app --features embassy` for `thumbv8m`. The
**committed** `server.rs` already fails with **27 errors** against the pinned
`picoserve 0.18` — it was written against an older picoserve API and has never
compiled (consistent with its "written, device-only" status). Categories:
`listen_and_serve_with_state` renamed/removed, the `Body::write_response_body`
signature now takes 2 type params, `Content`/`with_headers` bounds, route
handlers not satisfying `RequestHandler`/`State: FromStr`, `HeaderValue::parse`,
`RequestBodyReader::read`, `Vec::as_str`. My parity changes follow the existing
patterns faithfully (e.g. `IndexBody` mirrors `StorageBody`), so they inherit the
same drift but add no new error *category*. **Making the firmware build is a
separate picoserve-0.18 migration**, out of scope for this parity pass — but it
means the whole Core-0 web/logging/firmware stack is currently non-building
regardless of parity. Recommend a dedicated follow-up.

---

## 2. The Good (faithful, often test-backed)

- **`kiln-core` is a disciplined port.** PID arithmetic is expression-for-expression identical, including the non-obvious ordering (D before I, conditional anti-windup that predicts saturation from P+I only, then a hard `[out_min/ki, out_max/ki]` clamp) and bumpless `set_gains` (`integral *= old_ki/ki`). `pid.rs` vs `pid.py`.
- **Golden-replay tests** pin core modules to real-MicroPython traces at 1e-6…1e-9 tolerance: `replay_pid/tuner/profile/rate/state/temp_filter/ssr_schedule/recovery`. This is the strongest evidence in the repo and it passes.
- **Tuner step sequences** match exactly across all four modes (SAFE/STANDARD/THOROUGH/HIGH_TEMP) — every SSR %, target temp (incl. `min(100,max)`/`min(600,max)` clamps), hold time, timeout, plateau flag, step counts 3/6/13/8.
- **Gain schedule** `g(T)=1+h·(T−T_ambient)`, `h<0`→disable, and the change-threshold gate (`0.01 / 0.0001 / 0.01`, OR-combined, compared vs last-emitted gains) are exact.
- **State machine** is branch-for-branch faithful: all 5 states + numeric values, every transition guard, the `_update_running` operation order, the recovery-hold band (`>= target − 1.0`), stall detection (ramp-only, `min_rate = desired_rate*0.8`), and even the `desired_rate` status default of `0` (not 100).
- **Profile / rate math** preserves the leading-ramp-contributes-zero quirk, rate in °C/**hour**, scheduler boundary operators (`<=` reject past, `>=` consume), and ring-buffer tie-breaking (first-wins) exactly.
- **MAX31856 driver** is register-for-register: MASK=0x00, CR0 OCFAULT0, CR1 type/averaging masks, notch bit, autoconvert bit, SPI framing (`|0x80` write / `&0x7F` read). Decode math is provably equal (`*2^-7` 19-bit == `/4096` 12-bit-shift), tested for +/− temps. All 8 fault bits decoded with matching names.
- **SSR driver** is active-high, inits OFF, and adds a `Drop`-guard + RAM panic-path `raw_ssr_off()` — **stronger** de-energize-on-failure than Python's `__del__`.
- **Protocol data model** preserves all 10 command tags (1..=10, test-locked) and every status field with matching types/units; tricky reference behaviors reproduced at the JSON boundary (`step_name` null/`""`/kind three-way, tuning `elapsed`=step-elapsed merge, `peak_temp==0→null`).
- **CSV format** is byte-exact (header, 10 columns, filename sanitization, `-1`/`RECOVERY` markers, falsy-0 `total_steps`). **Recovery decision** faithful with strict-`>` delta and resume params echoed pre-check.
- **Config coverage**: every `config.py`/`config.example.py` UPPER_SNAKE knob exists in `KilnConfig` with matching `example.py` default & type; read once at boot before the core split; **graceful fallback to defaults on malformed JSON** (better than Python's import-crash). `parse_over` PATCH-merge is correct and tested.

---

## 3. The Bad (divergences — mostly intentional, but not 1:1)

1. **`get_elapsed_time` idempotency change.** Python's `get_status` calls the *mutating* `get_elapsed_time()` (`state.py:602`), so polling status advances the clock. Rust's `elapsed()` is a pure read (`state.rs:172-177`). Identical under normal cadence; diverges sub-second if the web layer polls between control ticks. Documented.
2. **`step_index` 0- vs 1-based + rename.** Python emits `current_step = index + 1` (`state.py:614`); Rust JSON emits the raw 0-based index under key `step_index` (`json.rs:100`). Off-by-one **and** a key rename — confirm the React client was updated in lockstep.
3. **Status transport: 20-deep FIFO queue → latest-only `Watch`** (`server.rs:44`). Python buffered up to 20 status messages; Rust keeps only the newest, multicast to consumers. Defensible (terminal consumer is latest-wins; CSV logger subscribes via `changed()`), but any consumer depending on intermediate-status backlog would now miss rows.
4. **NTP cadence flipped.** Python syncs **once** at boot (3 tries) and never again after success; Rust `ntp_task` re-syncs **hourly forever** (`platform.rs:470-475`). Low control impact (loop is monotonic-timed) but affects CSV/recovery timestamps on long firings.
5. **`STATUS_UPDATE_INTERVAL` / `SSR_UPDATE_INTERVAL` promoted** from hardcoded Python `const()` to runtime config knobs. Defaults match (5 s / 0.1 s) so behavior is identical, but the runtime surface expanded.
6. **API success-envelope fields dropped.** Python returns `{success, message}` (and `filename`/`size` on upload, `deleted_count` on bulk delete); Rust returns bare `{"success":true}`. The web app marks `message` optional, but `ProfileEditor.tsx` reads `uploadMutation.data.filename` → renders `undefined`.
7. **`file_get` omits `Content-Length` + `Content-Disposition: attachment`** headers Python sends — browser downloads lose filename/progress.
8. **Oversized JSON body: 413 → silent truncate.** Python returns 413 "Body too large"; Rust clips the body to 4096 (`server.rs:734`), which then likely fails as a 400.
9. **Multi-SSR turn-OFF not staggered.** Python staggers off pin-by-pin (`hardware.py:317-324`); Rust `all_off` de-energizes simultaneously. Safer, not 1:1. (Turn-ON stagger is non-blocking vs Python's blocking `sleep` — also a deliberate improvement.)
10. **`WEB_SERVER_HOST` / `WEB_SERVER_PORT` carried but unused** — Rust binds hardcoded; changing them in `config.json` is a no-op. Python actually binds them (`web_server.py:868`).
11. **`boot.py` dropped behaviors**: the 0.5 s power-settle delay (rationale was cold thermocouple-at-boot) has no port. GC tuning is correctly N/A for Rust.
12. **`ProfileCache` and `Profile.to_dict()` not ported** — architecturally obviated (FS owned by Core 0, profiles stored verbatim, direct existence check). Functionally equivalent, but missing relative to source.
13. **`MAX_STEPS = 32` cap** in `profile.rs` (Python unbounded) and `TooManySteps` rejection. Benign for real profiles (≤5 steps), divergent on pathological input.
14. **File-handle error recovery differs**: Python reopens-on-error and can permanently stop logging after a double failure; Rust opens per-row so transient errors self-heal silently. Comparable resilience, different mechanism.

---

## 4. The Ugly (real parity breaks / risks that ship today)

Ordered by severity.

### U1 — `{profiles_list}` index template is never substituted 🔴
`index.html` is served as compiled-in raw bytes (`platform.rs:285-292`, `server.rs:567`). Python prerenders the single placeholder `{profiles_list}` at boot (`main.py:318-321` → `<ul><li>… Start button …</li></ul>`). Rust has **no substitution step anywhere**. The on-device index page shows the literal text `{profiles_list}` and renders **no profile Start buttons** — index.html's JS has no client-side fallback. This directly fails the "two static html with templates" requirement.
**Fix:** add a boot-time or request-time substitution for `{profiles_list}` (enumerate `profiles/` → list items), or change index.html to fetch the list via JS.

### U2 — Recovery-resume CSV logging convention is unwired 🔴
The user explicitly asked for "same recovery logic." The recovery **decision** matches, but the **logging** does not. Python on resume: opens the *existing* log file in append mode, skips the header, and writes a one-shot `RECOVERY` event row (`data_logger.py:61-93,201-264,320-325`). Rust's `csv_logger_task` unconditionally opens with `create=true` (truncate + header) on every IDLE→RUNNING edge (`server.rs:177`), so a resume starts a **new** timestamped file, orphaning the interrupted run's data and splitting firing history across files with a gap. `csv.rs:write_recovery_event_row` is fully implemented **but has zero production callers** — dead code kept alive only by its unit test. The logger task currently has no channel to the recovery context to know (a) the resume target filename and (b) to append-without-header + emit the event row.
**Fix:** plumb resume context into `csv_logger_task`; on a recovery resume, append to the existing file and call `write_recovery_event_row`.

### U3 — `DELETE /api/files/<dir>/all` bulk-log-delete route is missing 🔴
No `/all` route is registered (`server.rs:271-275`); the path falls through to single-file delete of a file literally named `"all"`, which always fails → `{"success":false,"error":"Failed to delete file"}`. The logs-only 403 guard and the `{deleted_count, deleted_files}` response are absent. `bulk_delete_allowed`/`remove_all` are dead code. **The shipped Tauri web app's "Delete all logs" feature is broken** (`client.ts:301` → `hooks.ts:477`).

### U4 — CORS OPTIONS preflight is unhandled 🔴
Python returns `200 + CORS` for any OPTIONS (`web_server.py:780-782`). Rust/picoserve returns **405 with no `Access-Control-Allow-*` headers**, and no global OPTIONS/fallback is mounted (`main.rs:229-236`). CORS headers are only attached by handler responses, which never run for the 405. A browser-context cross-origin POST/PUT/DELETE-with-JSON from the web app is blocked at preflight. (Tauri's native webview may bypass CORS; a browser `bun run dev` session will not.)

### U5 — `ReadyFlag` Core-1-ready boot gate is missing 🔴 (safety-relevant)
Python blocks Core 2 bring-up on `ready_flag.wait_ready(timeout=20)` and treats a timeout as a **fatal** "System unsafe to operate" error (`main.py:235-242`). Rust `core0_main` brings up network/web/recovery **without any Core-1-ready handshake** (`main.rs:191-247`); a dead Core 1 produces no boot-time error, and web handlers / `attempt_recovery` can enqueue commands before Core 1's sensor/SSR are initialized. The `≥20 °C` recovery gate partially compensates but the explicit "Core 1 dead → stop" guard is gone. `protocol.rs:3` claims these primitives are "reimplemented with embassy primitives in the firmware crates," but **grep finds no such reimplementation** — the claim is currently aspirational.

### U6 — Config defaults vs the *actual* device `config.py` overrides 🔴 (silent firing-behavior change)
`KilnConfig::default()` tracks `config.example.py` (correct baseline). But the real `config.py` overrides several values that, if **not** carried into `config.json`, silently revert to template defaults:
- `MAINS_FREQUENCY` = **50** (config.py:63) vs **60** default → wrong notch/mains rejection on the sensor.
- `THERMOCOUPLE_TYPE` = **S** (config.py) vs **K** default → wrong thermocouple linearization.
- `MAX_RECOVERY_TEMP_DELTA` = **60** (config.py) vs **30** default → different recovery tolerance.
- Tuned PID gains (e.g. 1.690 / 0.0038 / 33.987 for the 8 kWh kiln) vs the example 25 / 0.14 / 160.

**Action:** confirm the migration emits a `config.json` capturing the real `config.py` values. This is the single highest-risk item for firing correctness and lives outside the ported source.

### U7 — `LOGGING_INTERVAL` is hardcoded to 30 s 🔴
`csv_logger_task` uses a literal `30` (`server.rs:187`) and is never handed the config. The `KilnConfig.logging_interval` field is parsed and stored (`config.rs:199`) but ignored. A user who sets `LOGGING_INTERVAL=10` still gets 30 s.
**Fix:** pass `&KilnConfig` (or just the interval) into `csv_logger_task`.

### U8 — `PID_KI_BASE` default provenance split 🔴 (functional)
Three different sources disagree: `config.example.py:90` = **0.14**, the Python in-code fallback `getattr(config,'PID_KI_BASE',0.18)` (`control_thread.py:134`) = **0.18**, Rust `params.rs:41` default = **0.14**, and `gain_schedule.rs` tests use **0.18**. For any deployed `config.py` that omits the key, Python runs **Ki=0.18** while Rust runs **Ki=0.14** — a ~28% integral-gain difference affecting steady-state control.
**Fix:** pick the canonical Ki and align Python's getattr fallback, Rust's default, and the tests.

### U9 — LCD layer entirely unported 🟡→🔴 (marked optional, but stubbed)
No HD44780/PCF8574 driver exists in `rust/`. `LcdDisplay::show`/`init_display` are comment-only DEVICE stubs (`platform.rs:312-362`). The whole `lcd_manager.py` presentation layer is gone: the exact 2-line content (`"{temp}C {state}"`, `"Tgt:{target}C {ssr}%"`), the 5 s cadence (Rust would render on every ~1 Hz status change), the 300 s periodic hardware reset, the consecutive-error backoff/auto-disable, the I2C device scan, and the init timeout. `lcd_enabled` config is parsed but the task is spawned unconditionally (`main.rs:243`). CLAUDE.md marks LCD "optional" and `Cargo.toml` marks it `[next]`/`[later]`, so the absence is *planned* — but it is unported beyond a config struct + empty task.

### U10 — Firmware sensor init may omit `start_autoconverting()` 🔴 (device-only, latent)
`build_kiln_io`'s DEVICE comment (`platform.rs:150-153`) lists `init()` + `set_averaging()` + `set_noise_filter()` but **not** `start_autoconverting()`. Python's init requires it (`hardware.py:84`); without it the MAX31856 stays in one-shot/off mode and the LTCB registers read 0 forever → `read_temperature()` always returns 0.0 °C and the control loop never sees real temperature. The driver method exists and is correct; the wiring is missing from the documented sequence. Code is `unimplemented!()`, so this is a spec-level gap to fix when the device body is written.

### Note — `STOP_TUNING`: Rust is *more correct* than Python ⚖️
A `STOP_TUNING` during tuning makes Python fall through to `self.tuner.update()` on a now-`None` tuner → `AttributeError` → caught → `set_error` + `sleep(1)` + **no watchdog feed**, landing in **ERROR**. Rust detects `tuner.is_none()` first, feeds the watchdog, and stops cleanly to **IDLE** (`controller.rs:226-232`). Strict 1:1 would reproduce the Python crash; Rust deliberately doesn't. **Recommend documenting Rust as the corrected reference rather than "fixing" to match the bug.**

---

## 5. Device-stub caveat (unverifiable from source)

`kiln-firmware` is excluded from the host workspace and contains multiple `unimplemented!()` DEVICE bodies. The following could **not** be verified and need confirmation once implemented:
- WiFi join / DHCP / static-IP application / best-AP scan (`platform.rs:339`).
- Flash `FlashStorage::append` / littlefs `sync` cadence.
- LCD I2C writes.
- `start_autoconverting()` placement (U10).
- NTP attempt/backoff inside `ntp_task`.

Until these land, end-to-end device behavior (and several parity claims that depend on them) is asserted by structure only, not exercised.

---

## 6. Cross-cutting open questions

1. **Does the deployed device ship a `config.json` mirroring `config.py`'s overrides** (S, 50 Hz, 60 °C delta, tuned PID gains)? If not, the Rust build silently runs example defaults. (U6 — highest risk.)
2. **Is the bundled on-device `index.html` still in scope**, or is the Tauri web app the only supported front-end? If the latter, U1's severity drops — but the placeholder is still user-visible on the device.
3. **Are `ReadyFlag` / `QuietMode` intentionally dropped**, or meant to live in firmware boot (they don't)? If dropped, update the `protocol.rs:3` doc claim and decide whether the "Core 1 dead → fatal" safety stop is acceptable to lose. (U5.)
4. **Canonical `PID_KI_BASE`** — 0.14 or 0.18? (U8.)
5. **NTP**: keep hourly resync, or match Python's one-shot-after-success? Confirm a mid-firing NTP jump can't perturb timestamp-derived recovery elapsed-seconds.
6. **CORS / OPTIONS**: handled by picoserve config, or unnecessary because only the CORS-exempt Tauri webview is supported? (U4.)
7. **CSV timestamps are UTC, not localtime** (`time.localtime` in Python). Confirm `scripts/plot_run.py` / `analyze_*` don't assume localtime.
8. **Status `Watch` (latest-only)**: confirm no Core 0 consumer depends on the 20-deep backlog Python provided.

---

## 7. Recommended fix priority

| # | Item | Why now |
|---|---|---|
| 1 | **U6** confirm/emit `config.json` from real `config.py` | Silent wrong mains freq / TC type / PID gains → wrong firings |
| 2 | **U2** wire recovery-resume CSV (append + event row) | Explicit "same recovery logic" requirement; data-continuity loss |
| 3 | **U5** Core-1-ready boot gate (or document the drop) | Safety: web/recovery start without confirming Core 1 |
| 4 | **U10** ensure `start_autoconverting()` in firmware init | Sensor reads constant 0 °C if omitted |
| 5 | **U1** substitute `{profiles_list}` | On-device UI broken; named requirement |
| 6 | **U3 / U4** restore bulk-delete route + CORS preflight | Shipped web-app features broken |
| 7 | **U7** honor `LOGGING_INTERVAL`; **U8** reconcile `PID_KI_BASE` | Trivial fixes, real behavior drift |
| 8 | **U9** gate `lcd_task` on `lcd_enabled` now; port LCD when scheduled | Avoid spawning a no-op; finish optional feature later |

---

*Generated from 9 parallel feature-block audits. Line citations reflect the state of `feat/rust-kiln-core` at audit time.*
