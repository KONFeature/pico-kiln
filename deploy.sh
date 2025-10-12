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

# Copy all main Python files
for file in *.py; do
    if [ -f "$file" ]; then
        echo "  -> $file"
        mpremote cp "$file" :
    fi
done

echo ""
echo "Copying lib folder..."

# Create lib directory on Pico if it doesn't exist
mpremote mkdir :lib 2>/dev/null || true

# Copy lib folder contents if it exists
if [ -d "lib" ]; then
    for file in lib/*; do
        if [ -f "$file" ]; then
            filename=$(basename "$file")
            echo "  -> lib/$filename"
            mpremote cp "$file" :lib/
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
echo "======================================"
echo "Deployment complete!"
echo "======================================"
echo ""
echo "To run the program:"
echo "  mpremote run main.py"
echo ""
echo "Or to make it run on boot:"
echo "  Rename main.py to main.py on the Pico (already done by this script)"
echo "  Then reset the Pico"
echo ""
echo "To view serial output:"
echo "  mpremote connect /dev/ttyACM0"
echo "  or"
echo "  mpremote"
echo ""
