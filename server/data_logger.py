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

    def log_status(self, status):
        """
        Log a status update to CSV file

        Only logs if logging_interval seconds have passed since last log.
        This reduces memory usage by limiting the number of data points.

        Args:
            status: Status dictionary from StatusMessage.build()
                    Expected fields: timestamp, elapsed, current_temp,
                    target_temp, ssr_output, ssr_is_on, state, progress
        """
        if not self.is_logging or not self.file:
            return

        # Check if enough time has passed since last log
        current_time = time.time()
        if current_time - self.last_log_time < self.logging_interval:
            return  # Skip this log entry

        self.last_log_time = current_time

        try:
            # Extract fields from status message
            timestamp = status.get('timestamp', time.time())
            elapsed = status.get('elapsed', 0)
            current_temp = status.get('current_temp', 0.0)
            target_temp = status.get('target_temp', 0.0)
            ssr_output = status.get('ssr_output', 0.0)
            ssr_is_on = status.get('ssr_is_on', False)
            state = status.get('state', 'UNKNOWN')
            progress = status.get('progress', 0.0)

            # Format row
            timestamp_iso = self._format_timestamp_iso(timestamp)
            ssr_on_int = 1 if ssr_is_on else 0

            row = (
                f"{timestamp_iso},"
                f"{elapsed:.1f},"
                f"{current_temp:.2f},"
                f"{target_temp:.2f},"
                f"{ssr_output:.2f},"
                f"{ssr_on_int},"
                f"{state},"
                f"{progress:.1f}\n"
            )

            # Write to file
            self.file.write(row)
            self.file.flush()  # Ensure data is written to disk

        except Exception as e:
            print(f"[DataLogger] Error writing log entry: {e}")

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
            timestamp = current_status.get('timestamp', time.time())
            elapsed = recovery_info.elapsed_seconds
            current_temp = current_status.get('current_temp', recovery_info.last_temp)
            target_temp = current_status.get('target_temp', recovery_info.last_target_temp)
            ssr_output = current_status.get('ssr_output', 0.0)
            ssr_is_on = current_status.get('ssr_is_on', False)
            progress = current_status.get('progress', 0.0)

            # Format row with RECOVERY marker in state column
            timestamp_iso = self._format_timestamp_iso(timestamp)
            ssr_on_int = 1 if ssr_is_on else 0

            row = (
                f"{timestamp_iso},"
                f"{elapsed:.1f},"
                f"{current_temp:.2f},"
                f"{target_temp:.2f},"
                f"{ssr_output:.2f},"
                f"{ssr_on_int},"
                f"RECOVERY,"  # Special state marker
                f"{progress:.1f}\n"
            )

            # Write to file
            self.file.write(row)
            self.file.flush()  # Ensure data is written to disk

            print(f"[DataLogger] Recovery event logged at {elapsed:.1f}s")

        except Exception as e:
            print(f"[DataLogger] Error writing recovery event: {e}")

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
        - Starts logging when entering RUNNING state
        - Logs data during RUNNING state (respecting logging interval)
        - Stops logging when leaving RUNNING state

        Args:
            status: Status dictionary from StatusMessage.build()
        """
        current_state = status.get('state')
        profile_name = status.get('profile_name')

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

        # Log data during RUNNING state
        if current_state == 'RUNNING' and self.is_logging:
            self.log_status(status)

        # Stop logging when leaving RUNNING state
        if self.previous_state == 'RUNNING' and current_state != 'RUNNING':
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
            "ssr_is_on,"
            "state,"
            "progress_percent\n"
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
