# server/data_logger.py
# Data logging for kiln firing programs
#
# This module handles CSV logging of temperature and control data during
# kiln program runs. Runs on Core 2 (web server thread) to avoid blocking
# time-critical control operations on Core 1.

import time

class DataLogger:
    """
    CSV data logger for kiln firing programs

    Records temperature, SSR state, and program progress data to CSV files
    during kiln program runs. Designed to run on Core 2 to keep file I/O
    separate from time-critical control loop on Core 1.

    Uses configurable logging interval to limit memory usage on Pico.
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

    def start_logging(self, profile_name):
        """
        Start logging data to a new CSV file

        Creates a new CSV file with timestamp and profile name.
        Format: {profile_name}_{YYYY-MM-DD_HH-MM-SS}.csv

        Args:
            profile_name: Name of the kiln profile being run
        """
        # Generate filename with timestamp
        timestamp_str = self._format_timestamp_filename(time.time())
        # Sanitize profile name for filename
        safe_profile_name = profile_name.replace(' ', '_').replace('/', '_')
        filename = f"{self.log_dir}/{safe_profile_name}_{timestamp_str}.csv"

        try:
            # Create log directory if it doesn't exist
            try:
                import os
                os.mkdir(self.log_dir)
            except OSError:
                pass  # Directory already exists

            # Open file for writing
            self.file = open(filename, 'w')
            self.is_logging = True
            self.current_profile_name = profile_name
            self.last_log_time = 0  # Reset to force first log immediately

            # Write CSV header
            self._write_header()

            print(f"[DataLogger] Started logging to {filename}")
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
