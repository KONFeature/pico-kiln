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

    echo "  -> Removing lib/"
    mpremote rm -rf :lib 2>/dev/null || echo "     (lib/ not found, skipping)"

    echo "  -> Removing kiln/"
    mpremote rm -rf :kiln 2>/dev/null || echo "     (kiln/ not found, skipping)"

    echo "  -> Removing server/"
    mpremote rm -rf :server 2>/dev/null || echo "     (server/ not found, skipping)"

    echo "  -> Removing static/"
    mpremote rm -rf :static 2>/dev/null || echo "     (static/ not found, skipping)"

    # Also remove any .mpy files in root that might be lingering
    echo "  -> Cleaning root directory of old .mpy files"
    mpremote exec "import os; [os.remove(f) for f in os.listdir('/') if f.endswith('.mpy')]" 2>/dev/null || true

    echo "Clean complete!"
fi

echo ""
echo "Copying Python files..."

# Copy main Python files from DEPLOY_DIR
for file in "$DEPLOY_DIR"/*.py; do
    if [ -f "$file" ]; then
        filename=$(basename "$file")
        # Skip test files and example config
        if [[ "$filename" == "temp.py" ]] || [[ "$filename" == "config.example.py" ]] || [[ "$filename" == "mpy-detect.py" ]]; then
            echo "  -> Skipping $filename (test/example file)"
            continue
        fi
        echo "  -> $filename"
        mpremote cp "$file" :
    fi
done

echo ""
echo "Copying lib folder..."

# Create lib directory on Pico if it doesn't exist
mpremote mkdir :lib 2>/dev/null || true

# Copy lib folder contents if it exists (including subdirectories)
if [ -d "$DEPLOY_DIR/lib" ]; then
    # First copy files directly in lib/ (.py or .mpy)
    for file in "$DEPLOY_DIR/lib"/*; do
        if [ -f "$file" ]; then
            filename=$(basename "$file")
            echo "  -> lib/$filename"
            mpremote cp "$file" :lib/
        fi
    done

    # Then handle subdirectories (like adafruit_bus_device)
    for dir in "$DEPLOY_DIR/lib"/*/; do
        if [ -d "$dir" ]; then
            dirname=$(basename "$dir")
            echo "  -> lib/$dirname/"
            mpremote mkdir :lib/$dirname 2>/dev/null || true

            for file in "$dir"*; do
                if [ -f "$file" ]; then
                    filename=$(basename "$file")
                    echo "     -> lib/$dirname/$filename"
                    mpremote cp "$file" :lib/$dirname/
                fi
            done
        fi
    done
else
    echo "  No lib folder found (this is OK if you don't have external libraries yet)"
fi

echo ""
echo "Copying static folder..."

# Create static directory on Pico if it doesn't exist
mpremote mkdir :static 2>/dev/null || true

# Copy static folder contents if it exists
if [ -d "$DEPLOY_DIR/static" ] && [ "$(ls -A "$DEPLOY_DIR/static" 2>/dev/null)" ]; then
    for file in "$DEPLOY_DIR/static"/*; do
        if [ -f "$file" ]; then
            filename=$(basename "$file")
            echo "  -> static/$filename"
            mpremote cp "$file" :static/
        fi
    done
else
    echo "  No static files found (this is OK, using fallback HTML)"
fi

echo ""
echo "Copying kiln module..."

# Create kiln directory on Pico if it doesn't exist
mpremote mkdir :kiln 2>/dev/null || true

# Copy kiln folder contents if it exists (.py or .mpy)
if [ -d "$DEPLOY_DIR/kiln" ]; then
    for file in "$DEPLOY_DIR/kiln"/*; do
        if [ -f "$file" ] && [[ "$file" == *.py ]] || [[ "$file" == *.mpy ]]; then
            filename=$(basename "$file")
            echo "  -> kiln/$filename"
            mpremote cp "$file" :kiln/
        fi
    done
else
    echo "  Warning: kiln folder not found!"
    exit 1
fi

echo ""
echo "Copying server module..."

# Create server directory on Pico if it doesn't exist
mpremote mkdir :server 2>/dev/null || true

# Copy server folder contents if it exists (.py or .mpy)
if [ -d "$DEPLOY_DIR/server" ]; then
    for file in "$DEPLOY_DIR/server"/*; do
        if [ -f "$file" ] && [[ "$file" == *.py ]] || [[ "$file" == *.mpy ]]; then
            filename=$(basename "$file")
            echo "  -> server/$filename"
            mpremote cp "$file" :server/
        fi
    done
else
    echo "  Warning: server folder not found!"
    exit 1
fi

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
