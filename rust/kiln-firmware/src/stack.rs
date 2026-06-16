//! Stack-overflow guard (MSPLIM) + debug-only stack painting / high-water.
//!
//! RP2350 is ARMv8-M (Cortex-M33), which has the **MSPLIM** stack-limit register
//! — the purpose-built hardware stack guard. When the stack pointer would cross
//! MSPLIM the core raises a UsageFault(STKOF) *before* the offending write, so a
//! runaway stack traps deterministically at the boundary instead of silently
//! smashing `.bss`/`.uninit` and crashing somewhere unrelated later (the original
//! picoserve overflow — see PICOSERVE_RAM.md). This replaces the "MPU guard" idea
//! in that doc: MSPLIM needs no no-access region carved out of the statics and has
//! no exception-stacking-into-the-guard hazard.
//!
//! UsageFault is left disabled, so STKOF **escalates to HardFault** and is caught
//! by `main.rs`'s existing `#[exception] HardFault` (which records CFSR — STKOF
//! shows as bit 20, `0x0010_0000`, decoded in `report_prior_fault`).
//!
//! The guard ([`arm_guard`] / [`arm_guard_at`]) is always compiled. The painting
//! and high-water readout are compiled only under the `stack-debug` feature
//! (`./scripts/deploy.sh --debug`), so the shipped image carries just the guard.

use cortex_m::register::msplim;

/// Bytes left between the limit and the true stack bottom so the overflow fault's
/// own exception frame (an M33 FP frame is ~104 B) stacks *above* the statics
/// rather than corrupting `.bss`/`.uninit`. 512 B comfortably covers the frame +
/// the minimal `raw_ssr_off` → `record_fault` → `sys_reset` handler path.
const GUARD_RESERVE: u32 = 512;

/// Paint fill word (debug builds): distinct, odd (so it is never a valid aligned
/// code/return address), and unlikely to occur as live data.
#[cfg(feature = "stack-debug")]
const PAINT: u32 = 0xAAAA_AAAA;

extern "C" {
    /// Lowest legal stack address (cortex-m-rt `_stack_end` == `__euninit`, the
    /// top of `.data`/`.bss`/`.uninit`). The stack grows DOWN toward this.
    static _stack_end: u8;
    /// Initial MSP (cortex-m-rt `_stack_start` == top of RAM). Stack top.
    static _stack_start: u8;
}

#[inline]
fn stack_end() -> u32 {
    core::ptr::addr_of!(_stack_end) as u32
}

#[inline]
#[cfg(feature = "stack-debug")]
fn stack_start() -> u32 {
    core::ptr::addr_of!(_stack_start) as u32
}

/// Arm the MSP stack-limit guard for the **main** (Core 0) stack. Call once, as
/// early as possible in `main`, before any deep call frame. Always compiled.
#[inline]
pub fn arm_guard() {
    arm_guard_at(stack_end());
}

/// Arm the MSP stack-limit guard for a stack whose bottom is `bottom` (used for
/// Core 1, whose stack is a `Stack<N>` static, not the `_stack_end` region). Must
/// run **on the core being guarded** — MSPLIM is banked per-core.
#[inline]
pub fn arm_guard_at(bottom: u32) {
    // SAFETY: writing MSPLIM only narrows the legal stack. `bottom + reserve` sits
    // far below the current SP (stacks start near their top), so this never
    // insta-faults; it only traps a future overflow.
    unsafe { msplim::write(bottom + GUARD_RESERVE) }
}

/// Disable the stack-limit guard (MSPLIM = 0) for the current core. Call it as the
/// FIRST thing in a fault/panic handler: the handler body (stack scan, record,
/// reset) must be free to use the bottom-of-stack reserve without the still-armed
/// limit re-tripping into a nested fault → lockup (the Memfault mitigation). The
/// overflow has already been detected by the time a handler runs.
#[inline]
pub fn disarm_guard() {
    // SAFETY: MSPLIM = 0 (its reset value) only widens the legal stack — the lowest
    // address — so it can never itself fault.
    unsafe { msplim::write(0) }
}

/// Paint the free region of the **current** (Core 0) stack at boot, so a later
/// high-water scan can read how deep it got. Paints `[_stack_end, SP-64)` —
/// everything below the current frame (which must be left intact).
#[cfg(feature = "stack-debug")]
pub fn paint_current() {
    let lo = stack_end();
    let hi = cortex_m::register::msp::read().saturating_sub(64);
    paint_range(lo, hi);
}

/// Paint `[lo, hi)` (word-aligned) with [`PAINT`]. For Core 1, called on its full
/// `Stack<N>` *before* `spawn_core1` while the whole region is still free.
#[cfg(feature = "stack-debug")]
pub fn paint_range(lo: u32, hi: u32) {
    let mut a = (lo + 3) & !3;
    let hi = hi & !3;
    while a < hi {
        // SAFETY: [lo,hi) is free stack memory (below the live frame on Core 0; not
        // yet running on Core 1). Word-aligned, in bounds.
        unsafe { core::ptr::write_volatile(a as *mut u32, PAINT) }
        a += 4;
    }
}

/// Scan a painted region `[lo, hi)` (stack grows down from `hi`) and return
/// `(used_bytes, free_bytes, total_bytes)` — `used` = how far the stack ever
/// descended below `hi`.
#[cfg(feature = "stack-debug")]
fn scan(lo: u32, hi: u32) -> (u32, u32, u32) {
    let lo = (lo + 3) & !3;
    let hi = hi & !3;
    let total = hi.saturating_sub(lo);
    let len_words = (total / 4) as usize;
    // SAFETY: [lo,hi) is the painted stack region; read-only, word-aligned, in
    // bounds. Concurrent use by the running core only makes the reading a
    // conservative snapshot, not unsound.
    let words = unsafe { core::slice::from_raw_parts(lo as *const u32, len_words) };
    match kiln_core::diag::first_dirty_word(words, PAINT) {
        // First dirty word from the bottom = lowest address the stack ever wrote =
        // the deepest SP. Everything above it has been used at some point.
        Some(idx) => {
            let mark = lo + (idx as u32) * 4;
            (hi - mark, mark - lo, total)
        }
        None => (0, total, total),
    }
}

/// Log the high-water mark of both cores' stacks. Called periodically by the
/// (feature-gated) task in `main.rs`. `core1_bottom`/`core1_bytes` describe the
/// Core 1 `Stack<N>` static.
#[cfg(feature = "stack-debug")]
pub fn report_highwater(core1_bottom: u32, core1_bytes: u32) {
    let (u0, f0, t0) = scan(stack_end(), stack_start());
    let (u1, f1, t1) = scan(core1_bottom, core1_bottom + core1_bytes);
    log::info!(
        target: "stack",
        "highwater core0 used={}/{} free={} | core1 used={}/{} free={}",
        u0, t0, f0, u1, t1, f1
    );
}
