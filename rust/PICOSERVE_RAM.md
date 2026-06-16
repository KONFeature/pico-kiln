# picoserve RAM / stack-overflow optimization

Status: **exploration + ranked plan.** Goal: stop the Core 0 stack overflow on the
web-server poll, and shrink web RAM so headroom is comfortable, not knife-edge.

## TL;DR — it's a stack-peak problem, not a total-RAM problem

512 KiB RAM is plenty. The fault is **the web-server serve poll's stack frame
(~249 KiB peak) exceeding the stack *region*** — and the stack region is whatever
RAM is left after `.bss`. They seesaw:

```
RAM = 0x20000000 .. 0x20080000   (512 KiB, memory.x:15)
stack grows DOWN from 0x20080000; its floor = top of .bss
=> bigger .bss  =>  smaller stack  =>  overflow
```

| Build | web .bss (≈84 KiB/worker) | .bss top (`_stack_end`) | stack region | serve peak | result |
|---|---|---|---|---|---|
| 3 workers (your fault log) | ~252 KiB | `0x20057B90` | **~161 KiB** | ~249 KiB | **OVERFLOW** (`sp=0x20043340`, −82 KiB into .bss) |
| 1 worker (current tree) | ~84 KiB | ~`0x2002E590` | **~322 KiB** | ~249 KiB | OK, **~73 KiB headroom** |

**The 1-worker + USB-off fix is ALREADY in the working tree**
(`WEB_TASK_POOL_TOTAL=1` server.rs:77, `USE_USB_NCM=false` main.rs:389). Your
overflow log is the *pre-fix* 3-worker state. → **First action: confirm the
current tree is flashed.** That alone clears the overflow. Everything below is
about turning ~73 KiB of knife-edge headroom into something that survives the next
route you add.

Build profile is already optimal for this (`opt-level=2`, `lto="fat"`,
`codegen-units=1` — kiln-firmware/Cargo.toml:103). `"z"` is fatal (kills stack-slot
colouring → the original overflow). No lever left in the profile.

## Reassessment after TWO structural experiments (read this first)

Both structural levers are now measured. Neither moved the stack target:

| Experiment | `.text` | `.bss` | verdict |
|---|---|---|---|
| CORS `Layer` | **+8.2 KiB** | **+7.6 KiB** | regression — reverted |
| Command-route collapse | **−8.9 KiB** | −32 B | flash win, RAM flat — kept |

The `.bss` numbers tell the whole story: **route/handler restructuring barely touches
the per-worker serve future** (dominated by picoserve's ~77 KiB `select` floor,
api.rs:17). Confirmed law:

> **The serve future is bloated by distinct response-writer monomorphisations
> `(Headers, Body)`, not by route count. Wrapping the response writer (any layer)
> multiplies that path and loses; collapsing same-body routes shrinks code but not
> static RAM.**

The **only** change that moved static RAM toward the goal was **3→1 worker (−168 KiB
`.bss`), already applied** — it fixed the overflow. Everything since is code-size
noise on the RAM axis. Recommended posture:

1. **Stop refactoring the router for RAM.** Two experiments, zero `.bss` progress.
   Keep the collapse (it's cleaner + saves flash); don't chase more.
2. **The stack peak is the only metric that matters, and it's still UNMEASURED.**
   Section sizes (`.text`/`.bss`) are proxies that already misled us once (CORS
   "should help" → regression). Do Step 0 — measure the real poll frame — before any
   further change. This is now the single highest-value action.
3. **Make the 73 KiB headroom safe, not bigger** — MSPLIM stack guard (✅ done) +
   high-water telemetry (✅ done) + route-count budget (todo). A guarded, monitored
   margin beats KiB the refactors won't hand over.

Lever 3 (collapse *duplicated* un-overlapped scratch into one static) is the last
RAM-positive idea — but **only act on it if Step 0 proves duplication.** Everything
else is cosmetic.

## MEASURED — real high-water (this flips the priority)

Step 0 item 2 is live (`stack.rs`, `--debug`). First numbers under LAN traffic:

| Core | used | total | guard trips at | **free before HardFault** |
|---|---|---|---|---|
| **Core 0** (web) | 271 K | 331 K | total − 512 B | **~60 K (18%)** |
| **Core 1** (control) | 7188 B | 8192 B | 7680 B | **~492 B** 🔴 |

Two corrections to everything above:
- The web-core **peak is 271 K, not the estimated ~249 K** (real high-water > the
  fault-`sp` guess). Still fits 331 K with 60 K spare. Core 0 is **fine** — guarded,
  18% free. All the picoserve agonising was aimed at the comfortable core.
- **Core 1 is the actual risk.** The safety-critical control core (sensor/PID/SSR/
  watchdog, `CORE1_STACK_BYTES=8192` main.rs:59) peaked at 7188 B — **~492 B from
  tripping its own MSPLIM guard** → HardFault on the core that runs the heating
  element. It also *jumped* 4140→7188 B (+3 KB) mid-session on a deep path.

**New top priority (safety, not RAM):**
1. **Bump `CORE1_STACK_BYTES` 8192 → 16384.** One line. Cost: +8 KB `.bss`, taken
   from Core 0's 60 K headroom (→ ~52 K / 16%, still fine). Core 1 free: 492 B →
   ~8.7 KB. Non-negotiable — never run a fire-control core ~500 B from its guard.
2. **Find the Core 1 +3 KB path** (after the bump, non-urgent): `-Z emit-stack-sizes`
   on `kiln-control`, or note which control event correlates with the jump (tuner?
   a deep log/diag format?).
3. **Find Core 0's TRUE worst case.** 271 K was under observed traffic incl. a
   `ReadTimeout` — but the heaviest paths likely weren't hit: `/api/logs` (the 8 KiB
   `RING_CAP` snapshot array), file PUT/GET (`StorageBody`), `files_list` with many
   profiles. Exercise every endpoint under `--debug` and re-read high-water before
   declaring Core 0 safe. Only if that eats the margin does lever 3 (render scratch →
   shared static) become worth it — and now we can measure it.
4. **Watch script warns `<15%` on Core 0 only** — but Core 1 (12% free, tighter
   stack, more critical) got no warn. Warn on both cores; arguably a stricter Core 1
   threshold.

## Why moving buffers to statics mostly DOESN'T help (the trap)

Tempting fix: "render buffers are on the stack → move them to statics." But a
static lives in `.bss`, and `.bss` growth shrinks the stack 1:1. **Headroom
neutral.** It only wins when a buffer is currently embedded *multiple, un-overlapped
times* in the serve-poll frame (picoserve's nested router + `Response`/`Typed` move
chain can do this — see the `RENDER_CAP` comment, server.rs:475). Then collapsing N
copies → 1 shared static nets `(N−1)×size`.

Conclusion: **measure before moving anything.** Guessing trades stack for `.bss`
and changes nothing.

## Step 0 — MEASURE (do this before any change)

Right now "the router is the root cause" is a hypothesis. Confirm the actual
frame-hog instead of refactoring blind:

1. **Static frame sizes** — `RUSTFLAGS="-Z emit-stack-sizes"` (nightly) + the
   `stack-sizes` / `cargo-call-stack` tool on the ELF. Find which function owns the
   giant frame and whether it's `serve_and_shutdown`'s poll or `ApiResponse::write_to`.
2. **Runtime high-water mark** — ✅ **IMPLEMENTED** (`stack.rs`, debug-only via the
   `stack-debug` feature → `./scripts/deploy.sh --debug`). Boot paints both cores'
   free stack with `0xAAAAAAAA`; a 30 s task logs `highwater core0 used=.../...` →
   `/api/logs` + diag flash. Gives the *real* peak, not a worst-case sum. Build on
   the **release** profile (`--debug` = release + feature) so the number reflects
   the shipped opt-level=2 frame, not a dev-profile one.
3. Hit each endpoint and log the high-water per route → ranks the real offenders.

Output of Step 0 decides which of the levers below are worth it. **Don't skip it.**

## Top gainers (ranked: headroom impact × low effort × low risk)

### 1. Collapse the many tiny routes — the headline simplification
The router is **16 paths / 24 method-handlers**, and picoserve embeds *every*
handler future additively into the per-worker serve future (server.rs:1001 comment).
Two big sources of bloat:

- **6 `cors_preflight*` OPTIONS handlers** (server.rs:602-614) — three arities only
  because the path-param signature must match (`()`, `String<16>`,
  `(String<16>,String<64>)`). 12 routes carry an OPTIONS twin.
- **5 inline `enqueue(Command::X)` POST closures** (server.rs:511-552) — `/api/stop`,
  `/clear-error`, `/shutdown`, `/scheduled/cancel`, `/tuning/stop`, all identical
  but for the `Command`.

Fixes, in order of payoff:
- **Collapse the 5 command routes into one** `POST /api/cmd/{action}` that parses
  the action → `Command`. 5 routes+5 OPTIONS → 1 route+1 OPTIONS. Removes ~9
  method-handlers from the combined future. **Biggest single router shrink.**
- **Handle CORS in one `Layer` — TRIED + MEASURED + REJECTED (net regression).**
  picoserve 0.18 *does* support it (`Router::layer()` routing.rs:1332 + `Layer`
  trait routing/layer.rs:44 — can short-circuit OPTIONS and wrap responses). The
  code came out cleaner (one CORS site, 15 fewer route clauses). But the RAM went
  the **wrong way** on both axes (release, deps ELF):

  | Section | Before | After | Δ |
  |---|---|---|---|
  | `.text` | 441,536 | 449,696 | **+8,160** |
  | `.rodata` | 304,016 | 304,124 | +108 |
  | `.bss` | 185,040 | 192,624 | **+7,584** |

  **Why:** the 15 `.options(...)` shared just 3 tiny `cors_preflight*` fns
  (returning a static enum arm) — deleting them saved almost nothing. But the
  `CorsWriter` wrapping *every* response re-monomorphises picoserve's
  `write_response<H, B>` socket-write path over each `(HeadersChain<…>, Body)` the
  handlers emit (`ChunkedResponse`, `StorageBody`, `Typed<…>`, …), and the
  `Layer`/`Next` dispatch adds its own wrapper. Net: bigger per-worker serve future
  (`.bss`) **and** more code (`.text`). `.bss` up ⇒ stack region down (seesaw), and
  a fatter serve future ⇒ deeper poll ⇒ peak likely up too.

  **Lesson (the important takeaway):** the serve-future bloat is NOT driven by route
  *count* — it's driven by the number of distinct **response-writer
  monomorphisations** `(Headers, Body)`. Anything that *wraps* the response writer
  (a CORS layer, a logging layer) multiplies that path and loses. Per-response
  `.with_headers(CORS)` is actually the cheaper form — it monomorphises once per
  handler, no extra wrapper. **Keep CORS as-is. Do not layer it.**
- closure→named-fn: cosmetic, ~0 RAM. Skip unless it helps readability.

What this leaves of lever 1: the **command-route collapse** (`/api/cmd/{action}`) —
**TRIED + MEASURED + KEPT, but it's a flash win, not the RAM win.** 5 command routes
→ 1; action→Command+message map in api.rs (pure, host-tested); 8-line handler.

  | Section | Baseline | After | Δ |
  |---|---|---|---|
  | `.text` | 441,536 | 432,664 | **−8,872** |
  | `.rodata` | 304,016 | 303,932 | −84 |
  | `.bss` | 185,040 | 185,008 | **−32** |

  Flash −8.7 KiB (5 monomorphised dispatch+handler chains → 1). **`.bss` flat (−32 B)
  — this is the decisive number:** it confirms api.rs:17, the per-worker serve future
  is dominated by picoserve's ~77 KiB `select` floor, **not** handler count. The serve
  future barely changed size, so the poll frame almost certainly didn't either. The
  router is ~10 method-handlers shallower (`Route<A,Route<B,…>>` dispatch depth),
  which *could* trim the poll frame, but `.bss`-flat says don't expect it. Opposite of
  the CORS layer (which grew both) — so collapse is the *right-direction* refactor, but
  for **code size**, not the stack peak. Keep it for cleanliness + flash; don't count
  it toward the stack goal.

### 2. ~~Shrink the big `.bss` static buffers~~ — REJECTED
The 8 KiB `CSV_BUF_CAP` / `DIAG_BUF_CAP` (logging.rs:93/482) are **load-bearing**:
they batch CSV/diag writes so the SSR doesn't have to pause control to flush to
flash as often. Shrinking them = more frequent flushes = more SSR pauses. **Keep at
8 KiB.** Not a lever. (The `RING_CAP=8192` log snapshot is a separate per-request
array — leave it; it's request-local, not a steady `.bss` cost.)

### 3. Collapse duplicated render scratch into ONE shared static (1-connection safe)
`MAX_CONCURRENT_CONNECTIONS = 1` (api.rs:31) → at most one request renders at a
time, so per-request scratch CAN be one shared static with zero contention. Only
worth it for buffers Step 0 shows are **duplicated** in the frame. Candidates:
- `String<RENDER_CAP=2048>` × 6 variants in `write_to` (server.rs:1154-1218) —
  comment already says these are "multiplied across the worker". Prime suspect.
- `[0u8; 2048]` profile read in `load_profile`, held across `.await` (server.rs:825).
- `Vec<String<64>,32>` (2 KiB) Index, `Vec<u8,2048>` FileList arena.

If `write_to` currently carries several 2 KiB buffers un-overlapped, one shared
`static RENDER: Mutex<String<2048>>` collapses them → several KiB off the peak.
RENDER_CAP itself is justified at 2048 (config can hit 1.5 KiB, server.rs:478) —
don't shrink it, share it.

### 4. Stack-limit guard (MSPLIM) — convert silent corruption into a clean fault
✅ **IMPLEMENTED** (`stack.rs`, **always-on**, both cores). The overflow smashed
`.bss` (branch-to-null from a clobbered return addr) — the "random fault / looks
like a leak" symptom. RP2350 is ARMv8-M (Cortex-M33), which has **MSPLIM**, the
purpose-built stack-limit register — used here **instead of the MPU**: SP crossing
the limit raises a UsageFault(STKOF) *before* the bad write, so no no-access region
has to be carved out of the statics and there is no exception-stacking-into-the-guard
hazard the MPU approach has. UsageFault is left disabled → STKOF **escalates to
HardFault**, caught by the existing `#[exception] HardFault` (no new handler);
`report_prior_fault` decodes CFSR bit 20 → `[STACK OVERFLOW]`. The limit is set at
`_stack_end + 512` (512 B reserve so the fault's own FP exception frame stacks above
`.bss`). Overflow now traps **immediately and deterministically** at the boundary
instead of corrupting `.bss` and crashing somewhere unrelated later. Doesn't add
headroom; kills the whole class of hard-to-diagnose faults. **This is the main
"prevent further leaks" answer** — the "leak" is stack creep silently eating `.bss`.

## "Reduce RAM by half" — honest target

Halving total RAM is the wrong target (it's a seesaw, and the 84 KiB/worker floor is
picoserve's own serve machinery, not handler code — api.rs:17). The right target:
**peak ≤ ~60% of stack region** (comfortable margin for new routes). Realistic path:

- Lever 1 (route collapse + CORS layer) shrinks the 84 KiB serve future — ~9 command
  handlers + ~12 OPTIONS handlers leave the combined type (confirm via Step 0
  before/after). This is now the dominant lever (buffer-shrink is rejected).
- Lever 3 shaves several KiB off the peak (duplicated scratch only).
- Stack region stays ~322 KiB (no `.bss` change — buffers kept). Headroom grows by
  *lowering the peak*: ~249 KiB → low-200s → headroom ~73 → ~100-120 KiB. The win is
  peak-side, not stack-side. Measured, not promised.

## Prevent regressions ("no further leaks")

- **MSPLIM stack guard** (lever 4) — ✅ deterministic trap on overflow (always-on).
- **Runtime high-water logging** — ✅ implemented (debug build, 30 s task → /api/logs).
  Turns silent creep into a visible number. Next: alert if it crosses a budget.
- **Route budget** — a host-side test/`const_assert` on method-handler count so a new
  route can't silently push the combined future past the headroom.
- Keep `opt-level=2` pinned; re-measure on any picoserve bump or route change
  (already noted in kiln-firmware/Cargo.toml:90-100).
- USB-NCM stays off until the peak has real margin; re-enabling adds a 2nd ~84 KiB
  stack+worker (main.rs:383-389).

## Open questions (resolve in Step 0)

- [ ] What function actually owns the ~249 KiB frame — picoserve serve poll, or
      `write_to`? (emit-stack-sizes)
- [ ] Which scratch buffers are duplicated un-overlapped in the frame vs counted
      once? (decides lever 3 scope)
- [x] Does picoserve 0.18 support a global OPTIONS fallback / layer? → **YES**,
      `Router::layer()` + `Layer` trait (can short-circuit OPTIONS and inject CORS
      headers globally). CORS handlers + `.options()` routes + per-response
      `.with_headers(CORS)` can all be deleted. See lever 1.
- [~] Real high-water per route under LAN load — which endpoint is worst? (Now
      measurable: flash `./scripts/deploy.sh --debug`, hit each endpoint, read the
      `target: "stack"` high-water lines. Still needs a device run to fill in.)
- [ ] Can the 8 KiB log/CSV/diag buffers drop to 2-4 KiB without hurting flush
      behaviour?

## Sources / refs
- kiln-app/src/server.rs (router, write_to, pool) · api.rs (worker-floor comments)
- kiln-app/src/logging.rs (8 KiB buffers, ring snapshot)
- kiln-firmware/src/main.rs:383-389 (USB/worker gating) · Cargo.toml:90-105 (profile)
- kiln-firmware/memory.x (RAM/stack layout)
