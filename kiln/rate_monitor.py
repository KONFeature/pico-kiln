# kiln/rate_monitor.py
# Temperature rate monitoring for adaptive control
#
# Memory-efficient circular buffer that tracks temperature history
# to calculate heating/cooling rates for adaptive profile execution.

class TempHistory:
    """
    Memory-efficient circular buffer for temperature readings

    Stores temperature samples to calculate heating/cooling rates
    over specified time windows. Uses circular buffer to minimize
    memory footprint on MicroPython.

    Memory: ~960 bytes for 60 samples (10 minutes at 10-second sampling)
    """

    def __init__(self, capacity=60):
        """
        Initialize temperature history buffer

        Args:
            capacity: Maximum number of readings to store (default: 60)
                     At 10-second sampling: 60 = 10 minutes of history
        """
        self.capacity = capacity
        self.buffer = []
        self.write_index = 0

    def add(self, timestamp, temp):
        """
        Add temperature reading to buffer

        Args:
            timestamp: Unix timestamp or elapsed seconds
            temp: Temperature in °C
        """
        if len(self.buffer) < self.capacity:
            # Still filling buffer
            self.buffer.append((timestamp, temp))
        else:
            # Buffer full - overwrite oldest
            self.buffer[self.write_index] = (timestamp, temp)
            self.write_index = (self.write_index + 1) % self.capacity

    def get_rate(self, window_seconds=600):
        """
        Calculate temperature rate over specified window

        Uses linear rate calculation from oldest to newest reading
        within the time window. If window is longer than available
        history, uses all available data.

        Args:
            window_seconds: Time window in seconds (default: 600 = 10 minutes)

        Returns:
            Temperature rate in °C/hour (positive = heating, negative = cooling)
        """
        if len(self.buffer) < 2:
            return 0.0

        # Find most recent reading
        recent = max(self.buffer, key=lambda x: x[0])
        recent_time, recent_temp = recent

        # Find reading closest to window_seconds ago
        target_time = recent_time - window_seconds

        # Get all readings up to current time
        valid_readings = [r for r in self.buffer if r[0] <= recent_time]
        if not valid_readings:
            return 0.0

        # Find closest reading to target time
        old = min(valid_readings, key=lambda x: abs(x[0] - target_time))
        old_time, old_temp = old

        # Calculate rate
        dt_hours = (recent_time - old_time) / 3600.0
        if dt_hours == 0:
            return 0.0

        dtemp = recent_temp - old_temp
        return dtemp / dt_hours

    def clear(self):
        """Clear all history (for step transitions)"""
        self.buffer = []
        self.write_index = 0

    def get_size(self):
        """Get current number of readings in buffer"""
        return len(self.buffer)

    def is_full(self):
        """Check if buffer is at capacity"""
        return len(self.buffer) >= self.capacity
