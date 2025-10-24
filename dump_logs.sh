#!/bin/bash
# dump_logs.sh
# Download log files from Raspberry Pi Pico 2 to local machine

set -e  # Exit on error

echo "======================================"
echo "Pico Kiln Log Dump Script"
echo "======================================"

# Check if mpremote is installed
if ! command -v mpremote &> /dev/null; then
    echo "Error: mpremote is not installed"
    echo "Install with: pip install mpremote"
    exit 1
fi

# Create local logs directory if it doesn't exist
LOCAL_LOG_DIR="scripts/logs"
mkdir -p "$LOCAL_LOG_DIR"

echo "Connecting to Pico..."
mpremote connect list || true

echo ""
echo "Listing log files on Pico..."

# Get list of log files from Pico
# Try to list the logs directory - if it fails, there are no logs
if ! mpremote fs ls :logs 2>/dev/null; then
    echo "No logs directory found on Pico (or it's empty)"
    echo "This is normal if you haven't run any programs yet"
    exit 0
fi

echo ""
echo "Downloading log files to $LOCAL_LOG_DIR/..."

# Get list of files in logs directory
# Parse the output of ls and extract just the filenames
LOG_FILES=$(mpremote fs ls :logs 2>/dev/null | grep -E "\.csv$" | awk '{print $NF}' || true)

if [ -z "$LOG_FILES" ]; then
    echo "No log files found on Pico"
    exit 0
fi

# Download each log file
for filename in $LOG_FILES; do
    echo "  -> $filename"
    mpremote cp :logs/$filename "$LOCAL_LOG_DIR/$filename"
done

echo ""
echo "======================================"
echo "Download complete!"
echo "======================================"
echo ""
echo "Log files saved to: $LOCAL_LOG_DIR/"
echo ""
echo "Number of files downloaded: $(echo "$LOG_FILES" | wc -l)"
echo ""
