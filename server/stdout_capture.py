# server/stdout_capture.py
# Capture print() output and make available over HTTP
#
# MicroPython doesn't have sys.stdout, so we monkey-patch print()
# to capture all output without interfering with the running program.

import builtins
from micropython import const

# Ring buffer size for captured output
STDOUT_BUFFER_SIZE = const(200)  # Keep last 200 lines


class PrintCapture:
    """
    Captures print() output into a ring buffer

    This allows remote viewing of print() output over WiFi
    without needing USB serial connection.
    """

    _instance = None

    def __init__(self):
        """Initialize print capture with ring buffer"""
        self.buffer = []
        self.write_pos = 0
        self.buffer_size = STDOUT_BUFFER_SIZE
        self.line_number = 0
        self.original_print = builtins.print
        self.unflushed_lines = []  # Lines not yet written to file

    @classmethod
    def get_instance(cls):
        """Get singleton instance"""
        if cls._instance is None:
            cls._instance = PrintCapture()
        return cls._instance

    def captured_print(self, *args, **kwargs):
        """
        Replacement for built-in print()

        Captures output while still printing to USB serial
        """
        # Build the output string like print() does
        sep = kwargs.get('sep', ' ')
        end = kwargs.get('end', '\n')

        # Convert all args to strings and join
        output = sep.join(str(arg) for arg in args) + end

        # Print to original destination (USB serial)
        self.original_print(*args, **kwargs)

        # Also buffer for network access (strip trailing newline for storage)
        line = output.rstrip('\n')
        if line:  # Don't store empty lines
            # Add to ring buffer
            if len(self.buffer) < self.buffer_size:
                self.buffer.append(line)
            else:
                # Overwrite oldest
                self.buffer[self.write_pos] = line
                self.write_pos = (self.write_pos + 1) % self.buffer_size

            self.line_number += 1

            # Add to unflushed lines for file writing
            self.unflushed_lines.append(line)

    def get_unflushed_lines(self):
        """
        Get lines that haven't been written to file yet

        Returns:
            List of unflushed lines (clears the unflushed buffer)
        """
        lines = self.unflushed_lines.copy()
        self.unflushed_lines.clear()
        return lines

    def get_recent(self, lines=100):
        """
        Get recent output lines

        Args:
            lines: Number of lines to return (max)

        Returns:
            List of recent output lines
        """
        if lines > len(self.buffer):
            lines = len(self.buffer)

        # Return most recent lines
        return self.buffer[-lines:]

    def get_all(self):
        """Get all buffered output"""
        return self.buffer.copy()

    def clear(self):
        """Clear the buffer"""
        self.buffer.clear()
        self.write_pos = 0

    def get_stats(self):
        """Get buffer statistics"""
        return {
            'total_lines': self.line_number,
            'buffered_lines': len(self.buffer),
            'buffer_size': self.buffer_size,
            'unflushed_lines': len(self.unflushed_lines)
        }


def install_print_capture():
    """
    Install print capture globally

    After calling this, all print() statements will be captured
    and available via get_print_capture()
    """
    capture = PrintCapture.get_instance()
    builtins.print = capture.captured_print
    return capture


def get_stdout_capture():
    """Get the global PrintCapture instance (alias for compatibility)"""
    return PrintCapture.get_instance()
