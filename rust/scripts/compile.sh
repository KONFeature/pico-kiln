#!/bin/bash
# compile.sh
# Build the RP2350 (Pico 2 W) Rust firmware and pack it into a flashable UF2.
#
# The Rust port is a single bare-metal image (unlike the MicroPython build's
# .mpy files), so "compile" = cargo build -> ELF -> UF2. Flash it with
# ./deploy.sh (picotool) or by copying the .uf2 onto the BOOTSEL drive.
#
# Usage:
#   scripts/compile.sh            # release build (optimised; default)
#   scripts/compile.sh --debug    # debug build (faster compile, bigger image)

set -e  # Exit on error

PROFILE="release"
for arg in "$@"; do
    case "$arg" in
        --release) PROFILE="release" ;;
        --debug)   PROFILE="debug" ;;
        -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
        *)
            echo "Unknown option: $arg"
            echo "Usage: ./compile.sh [--debug]"
            exit 1
            ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FW_DIR="$(cd "$SCRIPT_DIR/.." && pwd)/kiln-firmware"
TARGET="thumbv8m.main-none-eabihf"
ELF="$FW_DIR/target/$TARGET/$PROFILE/kiln-firmware"
UF2="$FW_DIR/target/$TARGET/$PROFILE/kiln-firmware.uf2"

echo "======================================"
echo "Pico Kiln (Rust) Firmware Compiler"
echo "======================================"
echo "Profile: $PROFILE"

# --- prerequisites --------------------------------------------------------
command -v cargo    >/dev/null 2>&1 || { echo "Error: cargo not found";    exit 1; }
command -v picotool >/dev/null 2>&1 || { echo "Error: picotool not found (brew install picotool)"; exit 1; }

# arm-none-eabi-gcc WITH newlib (littlefs2-sys compiles bundled C). The Homebrew
# arm-none-eabi-gcc formula ships no libc; the ArmGNU toolchain does. Override
# with ARM_GCC=/path/to/arm-none-eabi-gcc if it is not auto-found.
if [ -z "${ARM_GCC:-}" ]; then
    for c in /Applications/ArmGNUToolchain/*/arm-none-eabi/bin/arm-none-eabi-gcc; do
        [ -x "$c" ] && { ARM_GCC="$c"; break; }
    done
    : "${ARM_GCC:=$(command -v arm-none-eabi-gcc || true)}"
fi
[ -n "${ARM_GCC:-}" ] || { echo "Error: arm-none-eabi-gcc (newlib) not found; set ARM_GCC=..."; exit 1; }
export CC_thumbv8m_main_none_eabihf="$ARM_GCC"

# --- build (nightly: kiln-app needs impl-trait-in-assoc-type / TAIT) -------
echo ""
echo "Building firmware ($PROFILE)..."
if [ "$PROFILE" = "release" ]; then
    ( cd "$FW_DIR" && cargo +nightly build --release )
else
    ( cd "$FW_DIR" && cargo +nightly build )
fi
[ -f "$ELF" ] || { echo "Error: ELF not found at $ELF"; exit 1; }

# --- pack UF2 -------------------------------------------------------------
# picotool reads the RP2350 family from the ELF's IMAGE_DEF block (emitted by
# embassy-rp via memory.x). If it ever can't, add: --family rp2350-arm-s
echo ""
echo "Packing UF2 (picotool uf2 convert)..."
picotool uf2 convert -t elf "$ELF" "$UF2"

echo ""
echo "======================================"
echo "Done."
echo "  ELF: $ELF"
echo "  UF2: $UF2  ($(du -h "$UF2" | cut -f1))"
echo ""
echo "Flash it:  ./scripts/deploy.sh    (or drop the .uf2 on the BOOTSEL drive)"
echo "======================================"
