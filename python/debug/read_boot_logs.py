#!/usr/bin/env python3
"""
Read boot debug logs from Pico via mpremote

Usage:
    python3 read_boot_logs.py [--watch]

Options:
    --watch     Continuously watch logs (updates every 2 seconds)
"""

import subprocess
import sys
import time

LOG_FILES = [
    '/boot_debug.log',    # From debug_boot.py
    '/boot_stages.log',   # From main_safe.py
    '/boot_error.log',    # From main_safe.py
    '/errors.log'         # Runtime errors
]

def read_file_from_pico(filepath):
    """Read a file from Pico using mpremote"""
    try:
        result = subprocess.run(
            ['mpremote', 'fs', 'cat', filepath],
            capture_output=True,
            text=True,
            timeout=5
        )
        if result.returncode == 0:
            return result.stdout
        else:
            return None
    except subprocess.TimeoutExpired:
        return "[TIMEOUT]"
    except Exception as e:
        return f"[ERROR: {e}]"

def read_all_logs():
    """Read all log files from Pico"""
    logs = {}
    for logfile in LOG_FILES:
        content = read_file_from_pico(logfile)
        logs[logfile] = content
    return logs

def print_logs(logs):
    """Print logs in a formatted way"""
    print("\n" + "=" * 80)
    print("PICO BOOT LOGS")
    print("=" * 80)

    for logfile, content in logs.items():
        print(f"\n{'=' * 80}")
        print(f"FILE: {logfile}")
        print('=' * 80)

        if content is None:
            print("[File does not exist or is empty]")
        elif content.strip() == "":
            print("[File exists but is empty]")
        else:
            print(content)

    print("\n" + "=" * 80)

def watch_logs(interval=2):
    """Continuously watch logs"""
    print(f"Watching logs (updates every {interval}s, Ctrl+C to stop)...")

    try:
        while True:
            # Clear screen (works on Unix/Linux/Mac)
            print("\033[2J\033[H", end="")

            logs = read_all_logs()
            print_logs(logs)

            print(f"\n[Refreshing in {interval}s... Press Ctrl+C to stop]")
            time.sleep(interval)

    except KeyboardInterrupt:
        print("\n\nStopped watching.")

def main():
    """Main entry point"""
    if len(sys.argv) > 1 and sys.argv[1] == '--watch':
        watch_logs()
    else:
        logs = read_all_logs()
        print_logs(logs)

if __name__ == '__main__':
    main()
