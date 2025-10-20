#!/bin/bash
# deploy.sh
# Deploy pico-kiln files to Raspberry Pi Pico 2 using mpremote

set -e  # Exit on error

echo "======================================"
echo "Pico Kiln Deployment Script"
echo "======================================"

# Check if mpremote is installed
if ! command -v mpremote &> /dev/null; then
    echo "Error: mpremote is not installed"
    echo "Install with: pip install mpremote"
    exit 1
fi

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

echo ""
echo "Copying Python files..."

# Copy main Python files (excluding test files)
for file in *.py; do
    if [ -f "$file" ]; then
        # Skip test files and example config
        if [[ "$file" == "temp.py" ]] || [[ "$file" == "config.example.py" ]]; then
            echo "  -> Skipping $file (test/example file)"
            continue
        fi
        echo "  -> $file"
        mpremote cp "$file" :
    fi
done

echo ""
echo "Copying lib folder..."

# Create lib directory on Pico if it doesn't exist
mpremote mkdir :lib 2>/dev/null || true

# Copy lib folder contents if it exists (including subdirectories)
if [ -d "lib" ]; then
    # First copy files directly in lib/
    for file in lib/*; do
        if [ -f "$file" ]; then
            filename=$(basename "$file")
            echo "  -> lib/$filename"
            mpremote cp "$file" :lib/
        fi
    done

    # Then handle subdirectories (like adafruit_bus_device)
    for dir in lib/*/; do
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
if [ -d "static" ] && [ "$(ls -A static)" ]; then
    for file in static/*; do
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

# Copy kiln folder contents if it exists
if [ -d "kiln" ]; then
    for file in kiln/*.py; do
        if [ -f "$file" ]; then
            filename=$(basename "$file")
            echo "  -> kiln/$filename"
            mpremote cp "$file" :kiln/
        fi
    done
else
    echo "  Warning: kiln folder not found!"
    exit 1
fi

# Create server directory on Pico if it doesn't exist
mpremote mkdir :server 2>/dev/null || true

# Copy server folder contents if it exists
if [ -d "server" ]; then
    for file in server/*.py; do
        if [ -f "$file" ]; then
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
echo "Copying profiles..."

# Create profiles directory on Pico if it doesn't exist
mpremote mkdir :profiles 2>/dev/null || true

# Copy profile JSON files if they exist
if [ -d "profiles" ] && [ "$(ls -A profiles/*.json 2>/dev/null)" ]; then
    for file in profiles/*.json; do
        if [ -f "$file" ]; then
            filename=$(basename "$file")
            echo "  -> profiles/$filename"
            mpremote cp "$file" :profiles/
        fi
    done
else
    echo "  No profile files found (add some .json profiles to the profiles/ directory)"
fi

echo ""
echo "======================================"
echo "Deployment complete!"
echo "======================================"
echo ""
echo "Files deployed:"
echo "  - Root: main.py, web_server.py, config.py"
echo "  - lib/: wrapper.py, busio.py, adafruit_max31856.py, etc."
echo "  - kiln/: __init__.py, profile.py, pid.py, state.py, hardware.py"
echo "  - profiles/: *.json firing profiles"
echo "  - static/: HTML/CSS/JS files (if present)"
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
