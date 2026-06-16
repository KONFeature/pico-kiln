#!/usr/bin/env bash
#
# ram.sh — measure the RP2350 firmware's static RAM usage (.data + .bss).
#
# RAM on this target is whatever the linker places in the RAM region (memory.x:
# 512 KiB at 0x2000_0000). `.text`/`.rodata` are flash, not RAM. This script
# builds kiln-firmware, then reports:
#   - section totals + static RAM as a % of the 512 KiB budget
#   - the largest static symbols (which structs/pools/stacks eat the RAM)
#   - (with --types) every type/async-future size via -Zprint-type-sizes
#
# Usage:
#   scripts/ram.sh                 # build + section totals + top symbols
#   scripts/ram.sh --no-build      # measure the existing ELF, skip the build
#   scripts/ram.sh --types         # also dump the 25 largest types
#   scripts/ram.sh --types PATTERN # ...and the field/variant tree of PATTERN
#
# Env overrides:
#   ARM_GCC=/path/to/arm-none-eabi-gcc   # newlib gcc (littlefs2-sys needs a libc)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FW_DIR="$(cd "$SCRIPT_DIR/.." && pwd)/kiln-firmware"
TARGET="thumbv8m.main-none-eabihf"
SRAM=524288 # 512 KiB — the RAM region in memory.x

# --- args ---
DO_BUILD=1
DO_TYPES=0
TYPE_PATTERN=""
for arg in "$@"; do
    case "$arg" in
        --no-build) DO_BUILD=0 ;;
        --types) DO_TYPES=1 ;;
        --help | -h) sed -n '3,21p' "$0"; exit 0 ;;
        *) [[ "$DO_TYPES" == 1 && -z "$TYPE_PATTERN" ]] && TYPE_PATTERN="$arg" || { echo "unknown arg: $arg" >&2; exit 2; } ;;
    esac
done

# --- locate the arm toolchain ---------------------------------------------
# gcc (for building) — newlib build, NOT the libc-less Homebrew formula.
if [[ -z "${ARM_GCC:-}" ]]; then
    for c in /Applications/ArmGNUToolchain/*/arm-none-eabi/bin/arm-none-eabi-gcc; do
        [[ -x "$c" ]] && { ARM_GCC="$c"; break; }
    done
    : "${ARM_GCC:=$(command -v arm-none-eabi-gcc || true)}"
fi
# size/nm (for reading the ELF) — any arm binutils or the llvm equivalents work.
BINDIR="$(dirname "${ARM_GCC:-/nonexistent}")"
SIZE="$BINDIR/arm-none-eabi-size"; [[ -x "$SIZE" ]] || SIZE="$(command -v arm-none-eabi-size || command -v llvm-size || command -v rust-size || true)"
NM="$BINDIR/arm-none-eabi-nm";     [[ -x "$NM" ]]   || NM="$(command -v arm-none-eabi-nm   || command -v llvm-nm   || command -v rust-nm   || true)"
[[ -x "$SIZE" && -x "$NM" ]] || { echo "error: need arm-none-eabi-size/nm (or llvm-size/nm) on PATH" >&2; exit 1; }

build() { # build [extra RUSTFLAGS] [toolchain, e.g. +nightly; default: rustup default]
    [[ -n "${ARM_GCC:-}" ]] && export "CC_thumbv8m_main_none_eabihf=$ARM_GCC"
    local tc="${2:-}"
    # CRITICAL: never export an empty RUSTFLAGS. An empty-but-set RUSTFLAGS
    # *overrides* the `[target.*] rustflags` in .cargo/config.toml, dropping the
    # `-Tlink.x` linker script — which links a ~20-byte stub (.data+.bss = 0 B).
    # Only set RUSTFLAGS when we actually have extra flags to add.
    if [[ -n "${1:-}" ]]; then
        ( cd "$FW_DIR" && RUSTFLAGS="$1" cargo ${tc:+$tc} build --release )
    else
        ( cd "$FW_DIR" && cargo ${tc:+$tc} build --release )
    fi
}

# --- build + locate ELF ----------------------------------------------------
# Build with the default toolchain (keeps the config.toml link flags → real
# binary). `--types` additionally does a nightly `-Zprint-type-sizes` build, but
# that dump comes from the compiler, not the ELF, so it runs after this one.
[[ "$DO_BUILD" == 1 ]] && build "" >&2
# Measure the final linked binary, not the newest deps/ artifact (deps/ also
# holds stale nightly stubs, and `ls -t` can pick one).
ELF="$FW_DIR/target/$TARGET/release/kiln-firmware"
[[ -f "$ELF" ]] || ELF="$(ls -t "$FW_DIR/target/$TARGET/release/deps/kiln_firmware-"* 2>/dev/null | grep -v '\.d$' | head -1 || true)"
[[ -n "$ELF" && -f "$ELF" ]] || { echo "error: no firmware ELF found — run without --no-build first" >&2; exit 1; }

# --- 1. section totals -----------------------------------------------------
echo "ELF: ${ELF#"$FW_DIR"/}"
"$SIZE" "$ELF"
read -r _text data bss _ < <("$SIZE" "$ELF" | awk 'NR==2{print $1, $2, $3, $4}')
static=$((data + bss))
awk -v s="$static" -v t="$SRAM" 'BEGIN{printf "static RAM (.data+.bss) = %d B  =  %.1f%% of %d B (512 KiB)\n", s, 100*s/t, t}'
[[ "$static" -eq 0 ]] && echo "warning: static RAM is 0 B — this ELF looks like a nightly stub; measure a stable build" >&2

# --- 2. largest static symbols --------------------------------------------
echo
echo "top static symbols (.bss/.data, by size):"
# nm --size-sort already orders by size ascending; tail = the largest. The size
# column is hex, converted with bash base-16 arithmetic (portable; macOS awk has
# no strtonum).
"$NM" --print-size --size-sort "$ELF" 2>/dev/null \
    | grep -E ' [bBdD] ' \
    | tail -15 \
    | while read -r _addr sz _type name; do
        printf "%9d  %s\n" "$((16#$sz))" "$name"
    done

# --- 3. (optional) per-type / per-future sizes -----------------------------
if [[ "$DO_TYPES" == 1 ]]; then
    OUT="$(mktemp -t ram-types.XXXXXX)"
    echo
    echo "building with -Zprint-type-sizes (full rebuild)…" >&2
    build "-Zprint-type-sizes" "+nightly" 2>/dev/null | grep 'print-type-size' > "$OUT" || true
    echo
    echo "25 largest types:"
    sed -E 's/print-type-size type: `(.*)`: ([0-9]+) bytes.*/\2 \1/' "$OUT" \
        | grep -E '^[0-9]' \
        | grep -ivE 'MaybeUninit|MaybeDangling|ManuallyDrop|UnsafeCell|UninitCell' \
        | sort -rn | head -25 | cut -c1-130
    if [[ -n "$TYPE_PATTERN" ]]; then
        echo
        echo "field/variant tree of types matching: $TYPE_PATTERN"
        awk -v p="$TYPE_PATTERN" '
            /print-type-size type:/ { show = index($0, p) > 0 }
            show { print }
        ' "$OUT" | sed -E 's/print-type-size +//' | cut -c1-130 | head -60
    fi
    echo
    echo "(type-size dump kept at $OUT)"
fi
