# server/data_logger.py
# Data logging for kiln firing programs
#
# This module handles CSV logging of temperature and control data during
# kiln program runs. Runs on Core 2 (web server thread) to avoid blocking
# time-critical control operations on Core 1.
#
# Works as a listener for StatusReceiver - registers a callback to receive
# status updates independently from web server.

import time

class DataLogger:
    """
    CSV data logger for kiln firing programs

    Records temperature, SSR state, and program progress data to CSV files
    during kiln program runs. Designed to run on Core 2 to keep file I/O
    separate from time-critical control loop on Core 1.

    Uses configurable logging interval to limit memory usage on Pico.

    Register with StatusReceiver to automatically receive status updates:
        receiver = get_status_receiver()
        receiver.register_listener(data_logger.on_status_update)
    """

    def __init__(self, log_dir="logs", logging_interval=30):
        """
        Initialize data logger

        Args:
            log_dir: Directory to store log files (default: "logs")
            logging_interval: Seconds between log entries (default: 30)
        """
        self.log_dir = log_dir
        self.logging_interval = logging_interval
        self.file = None
        self.is_logging = False
        self.current_profile_name = None
        self.current_filename = None
        self.last_log_time = 0
        self.previous_state = None

        # Recovery context
        self.recovery_log_file = None
        self.recovery_info = None

    def start_logging(self, profile_name, recovery_log_file=None):
        """
        Start logging data to a new CSV file, or resume to existing file

        Creates a new CSV file with timestamp and profile name, unless
        recovery_log_file is specified (for program recovery).
        Format: {profile_name}_{YYYY-MM-DD_HH-MM-SS}.csv

        Args:
            profile_name: Name of the kiln profile being run
            recovery_log_file: Optional path to existing log file to append to (for recovery)
        """
        if recovery_log_file:
            # Resume logging to existing file (program recovery)
            filename = recovery_log_file
            mode = 'a'  # Append mode
        else:
            # Generate filename with timestamp for new run
            timestamp_str = self._format_timestamp_filename(time.time())
            # Sanitize profile name for filename
            safe_profile_name = profile_name.replace(' ', '_').replace('/', '_')
            filename = f"{self.log_dir}/{safe_profile_name}_{timestamp_str}.csv"
            mode = 'w'  # Write mode

        try:
            # Create log directory if it doesn't exist
            try:
                import os
                os.mkdir(self.log_dir)
            except OSError:
                pass  # Directory already exists

            # Open file for writing or appending
            self.file = open(filename, mode)
            self.is_logging = True
            self.current_profile_name = profile_name
            self.current_filename = filename
            self.last_log_time = 0  # Reset to force first log immediately

            # Write CSV header only for new files
            if mode == 'w':
                self._write_header()
                print(f"[DataLogger] Started logging to {filename}")
            else:
                print(f"[DataLogger] Resumed logging to {filename}")

            print(f"[DataLogger] Logging interval: {self.logging_interval}s")

        except Exception as e:
            print(f"[DataLogger] Error starting log file: {e}")
            self.is_logging = False
            self.file = None
            self.current_filename = None

    def log_status(self, status):
        """
        Log a status update to CSV file

        Only logs if logging_interval seconds have passed since last log.
        This reduces memory usage by limiting the number of data points.

        For TUNING state, uses a shorter interval (2s) to capture detailed data
        needed for PID analysis.

        Args:
            status: Status dictionary from StatusMessage.build()
                    Expected fields: timestamp, elapsed, current_temp,
                    target_temp, ssr_output, state
                    Step fields (optional): step_name, step_index, total_steps
        """
        if not self.is_logging or not self.file:
            return

        # Check if enough time has passed since last log
        current_time = time.time()
        current_state = status['state']  # Safe: guaranteed by StatusMessage template

        # Use shorter interval for TUNING to capture detailed response curve
        interval = 2.0 if current_state == 'TUNING' else self.logging_interval

        if current_time - self.last_log_time < interval:
            return  # Skip this log entry

        self.last_log_time = current_time

        try:
            # Extract fields from status message
            # Safe: All fields guaranteed by StatusMessage template
            timestamp = status['timestamp']
            elapsed = status['elapsed']
            current_temp = status['current_temp']
            target_temp = status['target_temp']
            ssr_output = status['ssr_output']
            state = status['state']

            # Check if we're in recovery mode
            # Safe: Field guaranteed by StatusMessage template
            is_recovering = status['is_recovering']

            if is_recovering:
                # In recovery mode - use special markers
                step_name = 'RECOVERY'
                step_index = -1
                total_steps = status['total_steps'] or ''  # Convert None to empty string
                current_rate = 0.0  # No rate during recovery
            else:
                # Normal logging - extract step info (populated for both tuning and profile runs)
                # Safe: Fields guaranteed by StatusMessage template, but can be None
                step_name = status['step_name'] or ''  # Convert None to empty string
                step_index = str(status['step_index']) if status['step_index'] is not None else ''
                total_steps = status['total_steps'] or ''
                # Extract rate info (for adaptive control)
                current_rate = status['current_rate']

            # Format row - use simple string concatenation for MicroPython compatibility
            timestamp_iso = self._format_timestamp_iso(timestamp)

            # Build CSV line (MicroPython-compatible approach)
            # Note: Removed progress_percent column (no longer in StatusMessage)
            row = (
                f"{timestamp_iso},"
                f"{elapsed:.1f},"
                f"{current_temp:.2f},"
                f"{target_temp:.2f},"
                f"{ssr_output:.2f},"
                f"{state},"
                f"{step_name if step_name else ''},"
                f"{step_index if step_index is not None and step_index != '' else ''},"
                f"{total_steps if total_steps is not None and total_steps != '' else ''},"
                f"{current_rate:.1f}\n"
            )

            # Write to file
            self.file.write(row)
            self.file.flush()  # Ensure data is written to disk

        except Exception as e:
            print(f"[DataLogger] Error writing log entry: {e}")
            # Try to recover the file handle
            if not self._recover_file_handle():
                print(f"[DataLogger] Failed to recover - logging stopped")
                return
            # Try writing again after recovery
            try:
                self.file.write(row)
                self.file.flush()
                print(f"[DataLogger] Successfully wrote after recovery")
            except Exception as e2:
                print(f"[DataLogger] Write failed after recovery: {e2}")
                self.is_logging = False
                self.file = None
                self.current_filename = None

    def log_recovery_event(self, recovery_info, current_status):
        """
        Log a recovery event to CSV file

        Writes a special log entry marking when program recovery occurred.
        Should be called immediately after recovery is initiated.

        Args:
            recovery_info: RecoveryInfo object with recovery details
            current_status: Current status dictionary with system state
        """
        if not self.is_logging or not self.file:
            return

        try:
            # Extract fields from status message
            # Safe: All fields guaranteed by StatusMessage template
            timestamp = current_status['timestamp']
            elapsed = recovery_info.elapsed_seconds
            current_temp = current_status['current_temp']
            target_temp = current_status['target_temp']
            ssr_output = current_status['ssr_output']

            # Extract rate info (for adaptive control)
            current_rate = current_status['current_rate']

            # Format row with RECOVERY marker in state column
            timestamp_iso = self._format_timestamp_iso(timestamp)

            # Build CSV line (MicroPython-compatible approach)
            # Note: Removed progress_percent column (no longer in StatusMessage)
            row = (
                f"{timestamp_iso},"
                f"{elapsed:.1f},"
                f"{current_temp:.2f},"
                f"{target_temp:.2f},"
                f"{ssr_output:.2f},"
                f"RECOVERY,"  # Special state marker
                f",,,"  # Empty step fields for recovery events
                f"{current_rate:.1f}\n"
            )

            # Write to file
            self.file.write(row)
            self.file.flush()  # Ensure data is written to disk

            print(f"[DataLogger] Recovery event logged at {elapsed:.1f}s")

        except Exception as e:
            print(f"[DataLogger] Error writing recovery event: {e}")
            # Try to recover the file handle
            if not self._recover_file_handle():
                print(f"[DataLogger] Failed to recover - logging stopped")
                return
            # Try writing again after recovery
            try:
                self.file.write(row)
                self.file.flush()
                print(f"[DataLogger] Successfully wrote recovery event after file recovery")
            except Exception as e2:
                print(f"[DataLogger] Write failed after recovery: {e2}")
                self.is_logging = False
                self.file = None
                self.current_filename = None

    def stop_logging(self):
        """
        Stop logging and close the CSV file
        """
        if not self.is_logging:
            return

        try:
            if self.file:
                self.file.close()
                print(f"[DataLogger] Stopped logging for {self.current_profile_name}")

            self.file = None
            self.is_logging = False
            self.current_profile_name = None
            self.current_filename = None

        except Exception as e:
            print(f"[DataLogger] Error closing log file: {e}")

    def set_recovery_context(self, recovery_info):
        """
        Set recovery context for resuming logging to existing file

        Should be called before the program resumes to tell the logger
        to append to the existing log file instead of creating a new one.

        Args:
            recovery_info: RecoveryInfo object with recovery details
        """
        self.recovery_log_file = recovery_info.log_file
        self.recovery_info = recovery_info
        print(f"[DataLogger] Recovery context set: will resume to {recovery_info.log_file}")

    def on_status_update(self, status):
        """
        Callback for StatusReceiver - called when status updates arrive

        Handles state transitions and logging:
        - Starts logging when entering RUNNING or TUNING state
        - Logs data during RUNNING or TUNING state (respecting logging interval)
        - Stops logging when leaving RUNNING or TUNING state

        Args:
            status: Status dictionary from StatusMessage.build()
        """
        # Safe: Fields guaranteed by StatusMessage template
        current_state = status['state']
        profile_name = status['profile_name']

        # Start logging when entering RUNNING state
        if current_state == 'RUNNING' and self.previous_state != 'RUNNING':
            if profile_name:
                # Check if we're in recovery mode
                if self.recovery_log_file:
                    # Resume logging to existing file
                    self.start_logging(profile_name, self.recovery_log_file)
                    # Write recovery event
                    if self.recovery_info:
                        self.log_recovery_event(self.recovery_info, status)
                    # Clear recovery context
                    self.recovery_log_file = None
                    self.recovery_info = None
                else:
                    # Normal start - create new log file
                    self.start_logging(profile_name)

        # Start logging when entering TUNING state
        if current_state == 'TUNING' and self.previous_state != 'TUNING':
            # Use "tuning" as profile name for tuning sessions
            tuning_name = "tuning"
            self.start_logging(tuning_name)

        # Log data during RUNNING or TUNING state
        if current_state in ['RUNNING', 'TUNING'] and self.is_logging:
            self.log_status(status)

        # Stop logging when leaving RUNNING or TUNING state
        if self.previous_state in ['RUNNING', 'TUNING'] and current_state not in ['RUNNING', 'TUNING']:
            self.stop_logging()

        self.previous_state = current_state

    def _write_header(self):
        """Write CSV header row"""
        header = (
            "timestamp,"
            "elapsed_seconds,"
            "current_temp_c,"
            "target_temp_c,"
            "ssr_output_percent,"
            "state,"
            "step_name,"
            "step_index,"
            "total_steps,"
            "current_rate_c_per_hour\n"
        )
        self.file.write(header)
        self.file.flush()

    def _format_timestamp_iso(self, unix_timestamp):
        """
        Format unix timestamp as ISO-like string for CSV

        Format: YYYY-MM-DD HH:MM:SS

        Args:
            unix_timestamp: Unix timestamp (seconds since epoch)

        Returns:
            ISO-formatted timestamp string
        """
        # MicroPython's time.localtime() returns tuple:
        # (year, month, day, hour, minute, second, weekday, yearday)
        t = time.localtime(unix_timestamp)
        return f"{t[0]:04d}-{t[1]:02d}-{t[2]:02d} {t[3]:02d}:{t[4]:02d}:{t[5]:02d}"

    def _format_timestamp_filename(self, unix_timestamp):
        """
        Format unix timestamp for use in filename

        Format: YYYY-MM-DD_HH-MM-SS

        Args:
            unix_timestamp: Unix timestamp (seconds since epoch)

        Returns:
            Filename-safe timestamp string
        """
        t = time.localtime(unix_timestamp)
        return f"{t[0]:04d}-{t[1]:02d}-{t[2]:02d}_{t[3]:02d}-{t[4]:02d}-{t[5]:02d}"

    def _recover_file_handle(self):
        """
        Attempt to recover from a file handle error by reopening the file

        This method is called when a write operation fails. It attempts to:
        1. Close the existing file handle (if possible)
        2. Reopen the file in append mode
        3. Restore logging state

        Returns:
            True if recovery succeeded, False otherwise
        """
        if not self.current_filename:
            print(f"[DataLogger] Cannot recover - no filename stored")
            self.is_logging = False
            self.file = None
            return False

        # Try to close the existing file handle
        try:
            if self.file:
                self.file.close()
        except Exception as e:
            print(f"[DataLogger] Error closing file during recovery: {e}")
            # Continue anyway - we'll try to reopen

        # Try to reopen the file in append mode
        try:
            self.file = open(self.current_filename, 'a')
            print(f"[DataLogger] Successfully recovered file handle for {self.current_filename}")
            return True
        except Exception as e:
            print(f"[DataLogger] Failed to reopen file during recovery: {e}")
            self.is_logging = False
            self.file = None
            self.current_filename = None
            return False
