#!/bin/bash
# deploy.sh
# Flash the RP2350 (Pico 2 W) Rust firmware onto the device with picotool.
#
# Builds first (via compile.sh) unless --no-build, then loads + runs the image
# over USB. The Pico must be in BOOTSEL mode: hold the BOOTSEL button while
# plugging in USB. (The firmware exposes no USB-reset interface, so picotool
# cannot reboot a running image into BOOTSEL for you — it's a manual hold.)
#
# Only the program region is written; the littlefs partition (config / profiles
# / logs, top 1.5 MB of flash) is preserved across reflashes.
#
# Usage:
#   scripts/deploy.sh             # build + flash + run
#   scripts/deploy.sh --no-build  # flash the already-built UF2
#   scripts/deploy.sh --debug     # build/flash the debug profile

set -e  # Exit on error

BUILD=true
PROFILE="release"
COMPILE_ARGS=()
for arg in "$@"; do
    case "$arg" in
        --no-build) BUILD=false ;;
        --debug)    PROFILE="debug"; COMPILE_ARGS+=(--debug) ;;
        --release)  PROFILE="release" ;;
        -h|--help)  sed -n '2,17p' "$0"; exit 0 ;;
        *) echo "Unknown option: $arg"; echo "Usage: ./deploy.sh [--no-build] [--debug]"; exit 1 ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FW_DIR="$(cd "$SCRIPT_DIR/.." && pwd)/kiln-firmware"
TARGET="thumbv8m.main-none-eabihf"
UF2="$FW_DIR/target/$TARGET/$PROFILE/kiln-firmware.uf2"

echo "======================================"
echo "Pico Kiln (Rust) Deployment Script"
echo "======================================"

command -v picotool >/dev/null 2>&1 || { echo "Error: picotool not found (brew install picotool)"; exit 1; }

# --- build ---------------------------------------------------------------
if [ "$BUILD" = true ]; then
    "$SCRIPT_DIR/compile.sh" "${COMPILE_ARGS[@]}"
fi
[ -f "$UF2" ] || { echo "Error: UF2 not found at $UF2 (run without --no-build, or ./compile.sh first)"; exit 1; }

# --- find the device ------------------------------------------------------
echo ""
echo "Looking for a Pico in BOOTSEL mode..."
if ! picotool info >/dev/null 2>&1; then
    echo ""
    echo "No Pico detected in BOOTSEL mode."
    echo "  -> Hold the BOOTSEL button while plugging in USB, then re-run:"
    echo "     ./scripts/deploy.sh --no-build"
    exit 1
fi
picotool info | sed 's/^/  /' || true

# --- flash + run ----------------------------------------------------------
echo ""
echo "Flashing $UF2 ..."
picotool load -x "$UF2"   # -x: execute after loading

echo ""
echo "======================================"
echo "Deployed. The Pico is now running the firmware."
echo "(littlefs config/profiles/logs were left intact.)"
echo "======================================"
