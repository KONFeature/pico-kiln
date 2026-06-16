#!/usr/bin/env bash
# watch_faults.sh — poll the kiln for captured faults AND live stack high-water.
#
# Two audit signals, two sources (PICOSERVE_RAM.md stack/RAM audit):
#
#  1. FAULTS — panic / hardfault, including the MSPLIM "[STACK OVERFLOW]" trap.
#     ERROR-level, so they persist in the rotating diag flash. A fault resets the
#     device, so the NEW boot's diag file carries the PREVIOUS boot's marker. We
#     read the newest diag file, dedupe, and addr2line the pc/lr + stack-scan bt.
#
#  2. STACK HIGH-WATER — the debug build (./scripts/deploy.sh --debug) logs
#     `stack: highwater core0 used=.../... free=...` every 30s at INFO. INFO does
#     NOT reach diag flash, so we read it from the live /api/logs RAM ring and
#     report each new per-core PEAK, plus a low-headroom warning. This is the
#     number that ranks the real per-route stack peak.
#
#  Plus: any other ERROR/WARN in the live ring (sensor faults, flash-budget,
#  wifi drops) — deduped — since those are the context an audit run wants too.
#
# Usage:
#   scripts/watch_faults.sh [HOST] [INTERVAL_SECONDS]
#   scripts/watch_faults.sh 192.168.68.70        # LAN IP, poll every 15s
#   scripts/watch_faults.sh 192.168.7.1 10       # over USB-NCM, every 10s
#
# Env:
#   LOW_FREE_PCT=15   # warn once when core0 free stack drops below this % of region

set -u
HOST="${1:-192.168.68.70}"
INTERVAL="${2:-15}"
ELF="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/kiln-firmware/target/thumbv8m.main-none-eabihf/release/kiln-firmware"
ADDR2LINE="$(command -v arm-none-eabi-addr2line || true)"
LOW_FREE_PCT="${LOW_FREE_PCT:-15}"

echo "watching http://$HOST every ${INTERVAL}s — faults (diag flash) + stack high-water (/api/logs)"
echo "  high-water needs a ./scripts/deploy.sh --debug build; low-headroom warn < ${LOW_FREE_PCT}% free (either core)"
[ -f "$ELF" ] && echo "elf: $ELF" || echo "WARN: elf not found, no addr2line decode: $ELF"
echo

seen=""          # last printed fault marker (latest|line)
peak0=0          # core0 stack high-water peak (bytes used)
peak1=0          # core1 stack high-water peak (bytes used)
warned_low=0     # one-shot low-headroom warning (core0)
warned_low1=0    # one-shot low-headroom warning (core1 — control core, more critical)
# ponytail: seen_lvl grows unbounded over a long run (newline list of deduped
# ERROR/WARN lines). Fine for a session watcher; cap it if you leave it running days.
seen_lvl=""

while true; do
    # ---- 1. faults (diag flash — persists across the fault's reset) --------
    latest=$(curl -s --max-time 5 "http://$HOST/api/files/diag" \
             | grep -oE 'diag-[0-9]+\.log' | sort -u | tail -1)
    if [ -n "$latest" ]; then
        body=$(curl -s --max-time 5 "http://$HOST/api/files/diag/$latest")
        # The fault summary line + the `fault bt:` backtrace line (hardfault only).
        block=$(printf '%s\n' "$body" | grep -E 'RECOVERED FROM|fault bt:')
        line=$(printf '%s\n' "$block" | grep -m1 'RECOVERED FROM')
        if [ -n "$line" ] && [ "$latest|$line" != "$seen" ]; then
            seen="$latest|$line"
            # The MSPLIM guard trip is the headline event for the stack audit.
            case "$line" in
                *"STACK OVERFLOW"*) tag="  *** STACK OVERFLOW — MSPLIM guard tripped ***" ;;
                *) tag="" ;;
            esac
            printf '[%s] FAULT %s%s\n' "$(date +%H:%M:%S)" "$latest" "$tag"
            printf '%s\n' "$block" | sed 's/^/  /'
            if [ -n "$ADDR2LINE" ] && [ -f "$ELF" ]; then
                # Decode every code-range address (10xxxxxx, flash XIP) from both
                # lines: the lr/pc, plus the stack-scan backtrace = the call chain.
                addrs=$(printf '%s\n' "$block" | grep -oE '\b10[0-9a-fA-F]{6}\b' | sort -u)
                for a in $addrs; do
                    printf '    0x%s -> %s\n' "$a" "$("$ADDR2LINE" -f -e "$ELF" "0x$a" | tr '\n' ' ')"
                done
            fi
        fi
    fi

    # ---- 2. stack high-water + ring ERROR/WARN (live RAM ring) -------------
    logs=$(curl -s --max-time 5 "http://$HOST/api/logs" || true)
    if [ -n "$logs" ]; then
        # Newest high-water line, parsed in one regex per core.
        hw=$(printf '%s\n' "$logs" | grep 'highwater core0' | tail -1)
        if [ -n "$hw" ]; then
            c0u=$(printf '%s' "$hw" | sed -nE 's#.*core0 used=([0-9]+)/([0-9]+) free=([0-9]+).*#\1#p')
            c0t=$(printf '%s' "$hw" | sed -nE 's#.*core0 used=([0-9]+)/([0-9]+) free=([0-9]+).*#\2#p')
            c0f=$(printf '%s' "$hw" | sed -nE 's#.*core0 used=([0-9]+)/([0-9]+) free=([0-9]+).*#\3#p')
            c1u=$(printf '%s' "$hw" | sed -nE 's#.*core1 used=([0-9]+)/([0-9]+) free=([0-9]+).*#\1#p')
            c1t=$(printf '%s' "$hw" | sed -nE 's#.*core1 used=([0-9]+)/([0-9]+) free=([0-9]+).*#\2#p')
            c1f=$(printf '%s' "$hw" | sed -nE 's#.*core1 used=([0-9]+)/([0-9]+) free=([0-9]+).*#\3#p')
            c0t=${c0t:-0}; c1t=${c1t:-0}

            # Report only NEW peaks (the deepest the stack has ever reached) — the
            # 30s line otherwise repeats the same number forever.
            if { [ -n "$c0u" ] && [ "$c0u" -gt "$peak0" ]; } \
               || { [ -n "$c1u" ] && [ "$c1u" -gt "$peak1" ]; }; then
                [ -n "$c0u" ] && [ "$c0u" -gt "$peak0" ] && peak0=$c0u
                [ -n "$c1u" ] && [ "$c1u" -gt "$peak1" ] && peak1=$c1u
                p0=$(( c0t > 0 ? peak0 * 100 / c0t : 0 ))
                p1=$(( c1t > 0 ? peak1 * 100 / c1t : 0 ))
                printf '[%s] STACK peak: core0 %d%% (%dK/%dK used)  core1 %d%% (%dB/%dB used)\n' \
                    "$(date +%H:%M:%S)" "$p0" "$((peak0/1024))" "$((c0t/1024))" "$p1" "$peak1" "$c1t"
            fi

            # One-shot warning when core0 headroom gets thin (knife-edge audit alarm).
            if [ -n "$c0f" ] && [ "$c0t" -gt 0 ]; then
                freepct=$(( c0f * 100 / c0t ))
                if [ "$freepct" -lt "$LOW_FREE_PCT" ] && [ "$warned_low" -eq 0 ]; then
                    warned_low=1
                    printf '[%s] !! LOW STACK HEADROOM: core0 free %d%% (%dK) < %d%% — adding routes is risky\n' \
                        "$(date +%H:%M:%S)" "$freepct" "$((c0f/1024))" "$LOW_FREE_PCT"
                fi
            fi

            # Same for core1 — the control core (sensor/PID/SSR/watchdog). It only has
            # an 8-16 KiB stack and an overflow faults the fire-control loop, so a thin
            # margin here matters MORE than core0.
            if [ -n "$c1f" ] && [ "$c1t" -gt 0 ]; then
                freepct1=$(( c1f * 100 / c1t ))
                if [ "$freepct1" -lt "$LOW_FREE_PCT" ] && [ "$warned_low1" -eq 0 ]; then
                    warned_low1=1
                    printf '[%s] !! LOW STACK HEADROOM: core1 free %d%% (%dB) < %d%% — CONTROL CORE, bump CORE1_STACK_BYTES\n' \
                        "$(date +%H:%M:%S)" "$freepct1" "$c1f" "$LOW_FREE_PCT"
                fi
            fi
        fi

        # Other ERROR/WARN lines worth auditing (deduped, timestamp-insensitive).
        # Skip fault markers — section 1 already owns those.
        while IFS= read -r l; do
            [ -z "$l" ] && continue
            case "$l" in *"RECOVERED FROM"*|*"fault bt:"*) continue ;; esac
            key=$(printf '%s' "$l" | sed -E 's/^[0-9:]+ //')   # drop HH:MM:SS so repeats dedupe
            if ! printf '%s' "$seen_lvl" | grep -qF -- "$key"; then
                seen_lvl="$seen_lvl
$key"
                printf '[%s] LOG  %s\n' "$(date +%H:%M:%S)" "$l"
            fi
        done <<EOF
$(printf '%s\n' "$logs" | grep -E ' (ERROR|WARN) ')
EOF
    fi

    sleep "$INTERVAL"
done
