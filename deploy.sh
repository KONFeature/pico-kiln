#!/bin/bash
# deploy.sh
# Deploy pico-kiln files to Raspberry Pi Pico 2 using mpremote

set -e  # Exit on error

# Parse arguments
CLEAN_DEPLOY=false
if [[ "$1" == "--clean" ]]; then
    CLEAN_DEPLOY=true
fi

echo "======================================"
echo "Pico Kiln Deployment Script"
echo "======================================"

# Check if mpremote is installed
if ! command -v mpremote &> /dev/null; then
    echo "Error: mpremote is not installed"
    echo "Install with: pip install mpremote"
    exit 1
fi

# Check if build directory exists (compiled bytecode)
if [ -d "build" ] && [ "$(ls -A build 2>/dev/null)" ]; then
    echo "Found compiled bytecode in build/ directory"
    echo "Deploying .mpy files for faster execution..."
    DEPLOY_DIR="build"
    USE_COMPILED=true
else
    echo "No build/ directory found - deploying source .py files"
    echo "Tip: Run ./compile.sh first for better performance"
    DEPLOY_DIR="."
    USE_COMPILED=false
fi
echo ""

# Check if config.py exists
if [ ! -f "config.py" ]; then
    echo "Warning: config.py not found!"
    echo "Please copy config.example.py to config.py and configure it first"
    echo ""
    read -p "Do you want to copy config.example.py to config.py now? (y/n) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        cp config.example.py config.py
        echo "Created config.py - please edit it with your WiFi credentials before continuing"
        exit 1
    else
        exit 1
    fi
fi

echo "Connecting to Pico..."
mpremote connect list || true

# Clean deployment if requested
if [ "$CLEAN_DEPLOY" = true ]; then
    echo ""
    echo "======================================"
    echo "Performing clean deployment..."
    echo "======================================"
    echo "Removing old directories on Pico:"
    echo "  -> Removing lib/, kiln/, server/, static/"
    echo "  -> Cleaning root directory of old .mpy files"

    # Batch all clean operations together
    mpremote rm -rf :lib + rm -rf :kiln + rm -rf :server + rm -rf :static + exec "import os; [os.remove(f) for f in os.listdir('/') if f.endswith('.mpy')]" 2>/dev/null || true

    echo "Clean complete!"
fi

echo ""
echo "Preparing file list..."

# Verify required directories exist
if [ ! -d "$DEPLOY_DIR/kiln" ]; then
    echo "  Error: kiln folder not found!"
    exit 1
fi

if [ ! -d "$DEPLOY_DIR/server" ]; then
    echo "  Error: server folder not found!"
    exit 1
fi

# Build list of root Python files to copy (excluding test files)
ROOT_FILES=()
for file in "$DEPLOY_DIR"/*.py; do
    if [ -f "$file" ]; then
        filename=$(basename "$file")
        # Skip test files and example config
        if [[ "$filename" == "temp.py" ]] || [[ "$filename" == "config.example.py" ]] || [[ "$filename" == "mpy-detect.py" ]]; then
            echo "  -> Skipping $filename (test/example file)"
            continue
        fi
        echo "  -> $filename"
        ROOT_FILES+=("$file")
    fi
done

# Check which optional directories exist
HAS_LIB=false
HAS_STATIC=false

if [ -d "$DEPLOY_DIR/lib" ]; then
    echo "  -> lib/ (recursive)"
    HAS_LIB=true
else
    echo "  -> No lib folder found (this is OK if you don't have external libraries yet)"
fi

if [ -d "$DEPLOY_DIR/static" ] && [ "$(ls -A "$DEPLOY_DIR/static" 2>/dev/null)" ]; then
    echo "  -> static/ (recursive)"
    HAS_STATIC=true
else
    echo "  -> No static files found (this is OK, using fallback HTML)"
fi

echo "  -> kiln/ (recursive)"
echo "  -> server/ (recursive)"

echo ""
echo "Deploying files in batched mode (faster)..."

# Create directories (ignore errors if they exist)
echo "Creating directories..."
if [ "$HAS_LIB" = true ]; then
    mpremote mkdir :lib 2>/dev/null || true
fi
if [ "$HAS_STATIC" = true ]; then
    mpremote mkdir :static 2>/dev/null || true
fi
mpremote mkdir :kiln 2>/dev/null || true
mpremote mkdir :server 2>/dev/null || true

# Build the batched mpremote command for copying files
CMD="mpremote"

# Copy root files
for file in "${ROOT_FILES[@]}"; do
    CMD="$CMD cp \"$file\" : +"
done

# Copy directories recursively
if [ "$HAS_LIB" = true ]; then
    CMD="$CMD cp -r \"$DEPLOY_DIR/lib/\"* :lib/ +"
fi

if [ "$HAS_STATIC" = true ]; then
    CMD="$CMD cp -r \"$DEPLOY_DIR/static/\"* :static/ +"
fi

# Always copy kiln and server
CMD="$CMD cp -r \"$DEPLOY_DIR/kiln/\"* :kiln/ +"
CMD="$CMD cp -r \"$DEPLOY_DIR/server/\"* :server/"

# Execute the batched command
echo "Copying files..."
eval $CMD

echo ""
echo "======================================"
echo "Deployment complete!"
echo "======================================"
echo ""

if [ "$USE_COMPILED" = true ]; then
    echo "Deployed compiled bytecode (.mpy files) for optimal performance!"
    echo ""
    echo "Files deployed:"
    echo "  - Root: main.py, boot.py, config.py (as .py)"
    echo "  - lib/: *.mpy (compiled bytecode)"
    echo "  - kiln/: *.mpy (compiled bytecode)"
    echo "  - server/: *.mpy (compiled bytecode)"
    echo "  - static/: HTML/CSS/JS files (if present)"
else
    echo "Deployed source code (.py files)"
    echo "Tip: Run ./compile.sh before deploying for better performance"
    echo ""
    echo "Files deployed:"
    echo "  - Root: main.py, boot.py, config.py"
    echo "  - lib/: *.py"
    echo "  - kiln/: *.py"
    echo "  - server/: *.py"
    echo "  - static/: HTML/CSS/JS files (if present)"
fi

echo ""
echo "To sync firing profiles, run:"
echo "  ./sync_profiles.sh"
echo ""
echo "To run the program:"
echo "  mpremote run main.py"
echo ""
echo "Or to make it run on boot:"
echo "  The main.py is already copied - just reset/power cycle the Pico"
echo ""
echo "To view serial output:"
echo "  mpremote connect /dev/ttyACM0"
echo "  or simply:"
echo "  mpremote"
echo ""
echo "Usage notes:"
echo "  - For clean deployment: ./deploy.sh --clean"
echo "  - This removes all old .py/.mpy files before deploying"
echo ""
