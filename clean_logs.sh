#!/bin/bash
# clean_logs.sh
# Delete log files from Raspberry Pi Pico 2

set -e  # Exit on error

echo "======================================"
echo "Pico Kiln Log Cleanup Script"
echo "======================================"

# Check if mpremote is installed
if ! command -v mpremote &> /dev/null; then
    echo "Error: mpremote is not installed"
    echo "Install with: pip install mpremote"
    exit 1
fi

echo "Connecting to Pico..."
mpremote connect list || true

echo ""
echo "Listing log files on Pico..."

# Try to list the logs directory - if it fails, there are no logs
if ! mpremote fs ls :logs 2>/dev/null; then
    echo "No logs directory found on Pico"
    exit 0
fi

# Get list of files in logs directory
LOG_FILES=$(mpremote fs ls :logs 2>/dev/null | grep -E "\.csv$" | awk '{print $NF}' || true)

if [ -z "$LOG_FILES" ]; then
    echo "No log files found on Pico"
    exit 0
fi

# Count files
FILE_COUNT=$(echo "$LOG_FILES" | wc -l)

echo "Found $FILE_COUNT log file(s) on Pico:"
for filename in $LOG_FILES; do
    echo "  - $filename"
done

echo ""
echo "WARNING: This will permanently delete all log files from the Pico!"
read -p "Are you sure you want to continue? (y/n) " -n 1 -r
echo

if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Operation cancelled"
    exit 0
fi

echo ""
echo "Deleting log files..."

# Delete each log file
for filename in $LOG_FILES; do
    echo "  -> Deleting $filename"
    mpremote rm :logs/$filename
done

echo ""
echo "======================================"
echo "Cleanup complete!"
echo "======================================"
echo ""
echo "Deleted $FILE_COUNT log file(s)"
echo ""
echo "Note: The logs directory still exists on the Pico"
echo "New logs will be created there when you run programs"
echo ""
