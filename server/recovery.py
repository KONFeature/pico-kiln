# server/recovery.py
# Program recovery system for automatic resume after reboot
#
# This module detects if a kiln program was interrupted by a reboot/crash
# and provides recovery information to automatically resume the program.
#
# Recovery uses only the CSV logs directory - no additional state files needed.

import time
import os
import gc


class RecoveryListener:
    """
    Status receiver listener for automatic program recovery

    This listener registers with the status receiver on boot and waits for
    the first valid temperature reading. Once received, it performs the
    recovery check and sends resume command if appropriate.

    After the recovery attempt (success or failure), it automatically
    unregisters itself to avoid interfering with normal operation.
    """

    def __init__(self, command_queue, data_logger, config, wifi_manager=None):
        """
        Initialize recovery listener

        Args:
            command_queue: ThreadSafeQueue for sending commands to control thread
            data_logger: DataLogger instance to set recovery context
            config: Configuration object with recovery settings
            wifi_manager: WiFiManager instance for NTP sync callback (optional)
        """
        self.command_queue = command_queue
        self.data_logger = data_logger
        self.config = config
        self.wifi_manager = wifi_manager
        self.recovery_attempted = False
        self.ntp_retry_registered = False
        self.status_receiver = None
        self.min_valid_temp = 20.0  # Minimum temp to consider valid (avoid false readings)

    def on_status_update(self, status):
        """
        Callback for status updates - performs recovery check on first valid temp

        Args:
            status: Status dictionary from StatusMessage.build()
        """
        # Only attempt recovery once
        if self.recovery_attempted:
            return

        # Wait for a valid temperature reading
        current_temp = status.get('current_temp', 0)
        if current_temp < self.min_valid_temp:
            # Temperature reading not valid yet (likely still initializing)
            return

        # Mark as attempted to prevent multiple recovery attempts
        self.recovery_attempted = True

        print(f"[Recovery] First valid temperature reading: {current_temp:.1f}°C")
        print(f"[Recovery] Performing recovery check...")

        # Perform recovery check
        recovery_info = check_recovery(
            self.config.LOGS_DIR,
            current_temp,
            self.config.MAX_RECOVERY_DURATION,
            self.config.MAX_RECOVERY_TEMP_DELTA
        )

        if recovery_info.can_recover:
            self._attempt_recovery(recovery_info)
        else:
            print(f"[Recovery] No recovery needed: {recovery_info.recovery_reason}")

            # If recovery failed due to time issues, register retry callback for NTP sync
            if recovery_info.had_time_issue and self.wifi_manager and not self.ntp_retry_registered:
                print(f"[Recovery] Time sync issue detected - will retry after NTP sync")
                self.wifi_manager.register_ntp_sync_callback(self._retry_recovery_after_ntp)
                self.ntp_retry_registered = True
                # Don't unregister yet - keep listening for temperature updates
                return

        # Unregister this listener - we're done
        if self.status_receiver:
            self.status_receiver.unregister_listener(self.on_status_update)
            print(f"[Recovery] Recovery listener unregistered")

    def set_status_receiver(self, status_receiver):
        """
        Set reference to status receiver (needed for unregistering)

        Args:
            status_receiver: StatusReceiver instance
        """
        self.status_receiver = status_receiver

    def _retry_recovery_after_ntp(self):
        """
        Retry recovery after NTP sync completes

        This is called as a callback when NTP sync succeeds, allowing
        recovery to retry with correct timestamps.
        """
        print(f"[Recovery] NTP sync completed - retrying recovery...")

        # Get current temperature from latest status
        # We need to wait for a status update with valid temp
        # For now, just trigger a new check on next status update
        self.recovery_attempted = False
        self.ntp_retry_registered = False

        print(f"[Recovery] Recovery retry will occur on next temperature reading")

    def _attempt_recovery(self, recovery_info):
        """
        Attempt to resume the interrupted program

        Args:
            recovery_info: RecoveryInfo object with recovery details
        """
        print(f"[Recovery] RECOVERY POSSIBLE: {recovery_info.recovery_reason}")
        print(f"[Recovery] Resuming profile '{recovery_info.profile_name}'")
        print(f"[Recovery] Elapsed time: {recovery_info.elapsed_seconds:.1f}s")
        print(f"[Recovery] Last temp: {recovery_info.last_temp:.1f}°C")

        try:
            # Set recovery context for data logger
            self.data_logger.set_recovery_context(recovery_info)

            # Send resume command with filename (Core 1 will load the profile)
            from kiln.comms import CommandMessage
            profile_filename = f"{recovery_info.profile_name}.json"
            resume_cmd = CommandMessage.resume_profile(profile_filename, recovery_info.elapsed_seconds)
            self.command_queue.put_sync(resume_cmd)

            print(f"[Recovery] Resume command sent to control thread")

        except Exception as e:
            print(f"[Recovery] RECOVERY FAILED: {e}")
            print(f"[Recovery] System will continue in IDLE state")


class RecoveryInfo:
    """
    Container for recovery information parsed from log files

    Attributes:
        can_recover: Whether recovery is safe and possible
        profile_name: Name of the interrupted profile
        elapsed_seconds: How far through the program execution
        last_temp: Last recorded temperature
        last_target_temp: Last target temperature
        last_timestamp: Unix timestamp of last log entry
        time_since_last_log: Seconds since last log entry
        log_file: Path to the log file being recovered from
        recovery_reason: String explaining why recovery is/isn't possible
        had_time_issue: Whether recovery failed due to time sync issues
        used_mtime_fallback: Whether mtime was used instead of log timestamp
    """
    def __init__(self):
        self.can_recover = False
        self.profile_name = None
        self.elapsed_seconds = 0.0
        self.last_temp = 0.0
        self.last_target_temp = 0.0
        self.last_timestamp = 0
        self.time_since_last_log = 0
        self.log_file = None
        self.recovery_reason = "No recovery needed"
        self.had_time_issue = False
        self.used_mtime_fallback = False


def check_recovery(logs_dir, current_temp, max_recovery_duration, max_temp_delta):
    """
    Check if program recovery should be attempted

    Scans the logs directory for the most recent CSV file and determines
    if it represents an interrupted program that can be safely resumed.

    Recovery conditions:
    1. Most recent log file found
    2. Last state was RUNNING (not COMPLETE, ERROR, or IDLE)
    3. Time since last log entry < max_recovery_duration
    4. Current temperature within max_temp_delta of last logged temperature

    Args:
        logs_dir: Directory containing CSV log files
        current_temp: Current measured temperature (°C)
        max_recovery_duration: Maximum seconds since last log to allow recovery
        max_temp_delta: Maximum temperature deviation (°C) to allow recovery

    Returns:
        RecoveryInfo object with recovery details and can_recover flag
    """
    info = RecoveryInfo()

    try:
        # Find the most recent log file
        log_file = _find_most_recent_log(logs_dir)
        if not log_file:
            info.recovery_reason = "No log files found"
            return info

        info.log_file = log_file

        # Parse the last line of the log file
        last_entry = _parse_last_log_entry(log_file)
        if not last_entry:
            info.recovery_reason = "Could not parse log file"
            return info

        # Extract recovery information
        info.last_temp = last_entry['current_temp']
        info.last_target_temp = last_entry['target_temp']
        info.last_timestamp = last_entry['timestamp']
        info.elapsed_seconds = last_entry['elapsed']

        # Extract profile name from filename
        # Format: {profile_name}_{YYYY-MM-DD}_{HH-MM-SS}.csv
        filename = log_file.split('/')[-1]  # Get just the filename
        # Split on last 2 underscores (date and time components)
        parts = filename.rsplit('_', 2)  # e.g., ['Biscuit_Faience', '2025-11-02', '13-28-09.csv']
        if len(parts) >= 3:
            # Convert to lowercase to match profile filename convention
            info.profile_name = parts[0].lower()
        else:
            info.recovery_reason = "Could not extract profile name from filename"
            return info

        # Check recovery conditions

        # 1. Was state RUNNING? (not COMPLETE, ERROR, or IDLE)
        if last_entry['state'] != 'RUNNING':
            info.recovery_reason = f"Last state was {last_entry['state']}, not RUNNING"
            return info

        # 2. Is it within recovery time window?
        # Use timestamp validation with mtime fallback for robustness
        current_time = time.time()

        # Validate timestamp sanity (year >= 2020)
        # MicroPython epoch is 2000-01-01, so timestamps before 2020 indicate no NTP sync
        MIN_VALID_TIMESTAMP = 631152000  # 2020-01-01 00:00:00 UTC (20 years * 365.25 * 86400)
        timestamp_is_valid = info.last_timestamp >= MIN_VALID_TIMESTAMP
        current_time_is_valid = current_time >= MIN_VALID_TIMESTAMP

        if timestamp_is_valid and current_time_is_valid:
            # Both timestamps are valid - use log timestamp
            info.time_since_last_log = current_time - info.last_timestamp
            info.used_mtime_fallback = False
            print(f"[Recovery] Using log timestamp for time calculation")
        else:
            # One or both timestamps invalid - fallback to file mtime
            try:
                stat = os.stat(log_file)
                file_mtime = stat[8]
                info.time_since_last_log = current_time - file_mtime
                info.used_mtime_fallback = True
                print(f"[Recovery] WARNING: Invalid timestamp detected, using file mtime fallback")
                print(f"[Recovery]   Log timestamp valid: {timestamp_is_valid}, Current time valid: {current_time_is_valid}")
            except OSError:
                # Can't get mtime - mark as time issue and skip time check
                info.had_time_issue = True
                info.time_since_last_log = 0
                print(f"[Recovery] WARNING: Cannot determine time delta, skipping time check")

        # Check if time delta makes sense (not negative, not absurdly large)
        if info.time_since_last_log < 0:
            info.recovery_reason = "Clock went backwards - time sync issue"
            info.had_time_issue = True
            return info

        # If time check is unreliable, we'll rely purely on temperature validation
        if not info.had_time_issue and info.time_since_last_log > max_recovery_duration:
            info.recovery_reason = (
                f"Too much time elapsed: {info.time_since_last_log:.0f}s "
                f"(max: {max_recovery_duration}s)"
            )
            # If we used mtime fallback and failed, this might be a time issue
            if info.used_mtime_fallback:
                info.had_time_issue = True
            return info

        # 3. Is temperature within acceptable range?
        # This is the most reliable check and works regardless of time sync
        temp_deviation = abs(current_temp - info.last_temp)

        if temp_deviation > max_temp_delta:
            info.recovery_reason = (
                f"Temperature deviated too much: {temp_deviation:.1f}°C "
                f"(max: {max_temp_delta}°C)"
            )
            return info

        # All checks passed - recovery is safe!
        info.can_recover = True
        time_info = f"{info.time_since_last_log:.0f}s elapsed" if not info.had_time_issue else "time uncertain"
        info.recovery_reason = (
            f"Recovery OK: {time_info}, "
            f"temp deviation {temp_deviation:.1f}°C"
        )

        return info

    except Exception as e:
        info.recovery_reason = f"Recovery check error: {e}"
        return info


def _find_most_recent_log(logs_dir):
    """
    Find the most recent CSV log file in the logs directory

    Uses file modification time (mtime) to find the most recent file.
    This is robust to clock sync issues - even if timestamps are wrong,
    the relative ordering of file mtimes is preserved within a boot session.

    Filters out tuning logs (files starting with "tuning_") since they
    are not profile runs and should not be candidates for recovery.

    Args:
        logs_dir: Directory to scan for log files

    Returns:
        Full path to most recent log file, or None if no logs found
    """
    try:
        # List all files in logs directory
        files = os.listdir(logs_dir)

        # Filter for CSV files only, excluding tuning logs
        csv_files = [f for f in files
                     if f.endswith('.csv') and not f.startswith('tuning_')]

        if not csv_files:
            return None

        # Sort by file modification time (mtime) instead of filename
        # This is robust to clock sync issues
        csv_files_with_mtime = []
        for f in csv_files:
            try:
                file_path = f"{logs_dir}/{f}"
                stat = os.stat(file_path)
                mtime = stat[8]  # ST_MTIME (modification time)
                csv_files_with_mtime.append((f, mtime))
            except OSError:
                # Skip files we can't stat
                continue

        if not csv_files_with_mtime:
            return None

        # Sort by mtime, most recent first
        csv_files_with_mtime.sort(key=lambda x: x[1], reverse=True)

        # Return full path to most recent file
        return f"{logs_dir}/{csv_files_with_mtime[0][0]}"

    except OSError:
        # Directory doesn't exist or can't be read
        return None


def _parse_last_log_entry(log_file):
    """
    Parse the last line of a CSV log file

    MEMORY OPTIMIZED: Reads file line-by-line, keeping only the last non-empty line
    in memory instead of loading the entire file. This reduces memory usage from
    ~120KB (for a 10-hour log) to <1KB.

    CSV format (new):
    timestamp,elapsed_seconds,current_temp_c,target_temp_c,
    ssr_output_percent,state,progress_percent,step_name,step_index,total_steps

    Args:
        log_file: Path to CSV log file

    Returns:
        Dictionary with parsed values, or None if parsing failed
    """
    try:
        # Read file line-by-line, keeping only the last non-empty line
        # This uses minimal memory compared to readlines() which loads entire file
        last_line = None
        line_count = 0

        with open(log_file, 'r') as f:
            for line in f:
                line_count += 1
                stripped = line.strip()
                if stripped:
                    last_line = stripped

        # Need at least header + one data row
        if line_count < 2:
            return None

        if not last_line:
            return None

        # Parse CSV values
        values = last_line.split(',')

        # Need at least 7 core columns (step columns are optional)
        if len(values) < 7:
            return None

        # Parse timestamp (ISO format: YYYY-MM-DD HH:MM:SS)
        timestamp_str = values[0]
        timestamp_unix = _parse_iso_timestamp(timestamp_str)

        result = {
            'timestamp': timestamp_unix,
            'elapsed': float(values[1]),
            'current_temp': float(values[2]),
            'target_temp': float(values[3]),
            'ssr_output': float(values[4]),
            'state': values[5],
            'progress': float(values[6])
        }

        # Force garbage collection after parsing
        gc.collect()

        return result

    except Exception as e:
        print(f"[Recovery] Error parsing log entry: {e}")
        return None


def _parse_iso_timestamp(timestamp_str):
    """
    Parse ISO timestamp string to unix timestamp

    Format: YYYY-MM-DD HH:MM:SS

    Args:
        timestamp_str: ISO formatted timestamp string

    Returns:
        Unix timestamp (seconds since epoch)
    """
    # Split date and time parts
    parts = timestamp_str.split(' ')
    if len(parts) != 2:
        return 0

    date_part = parts[0]
    time_part = parts[1]

    # Parse date: YYYY-MM-DD
    date_values = date_part.split('-')
    if len(date_values) != 3:
        return 0

    year = int(date_values[0])
    month = int(date_values[1])
    day = int(date_values[2])

    # Parse time: HH:MM:SS
    time_values = time_part.split(':')
    if len(time_values) != 3:
        return 0

    hour = int(time_values[0])
    minute = int(time_values[1])
    second = int(time_values[2])

    # Convert to unix timestamp
    # MicroPython's time.mktime expects tuple:
    # (year, month, day, hour, minute, second, weekday, yearday)
    # weekday and yearday can be 0 for mktime
    time_tuple = (year, month, day, hour, minute, second, 0, 0)

    return time.mktime(time_tuple)
