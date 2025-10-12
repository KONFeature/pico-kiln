# kiln/pid.py
# PID controller with anti-windup

import time

class PID:
    """
    PID controller with anti-windup and comprehensive statistics

    Based on the velocity form of PID to prevent integral windup.
    Output is clamped to output_limits.

    References:
    - https://en.wikipedia.org/wiki/PID_controller
    - https://github.com/jbruce12000/kiln-controller
    """

    def __init__(self, kp, ki, kd, output_limits=(0, 100)):
        """
        Initialize PID controller

        Args:
            kp: Proportional gain
            ki: Integral gain (inverse time constant)
            kd: Derivative gain (time constant)
            output_limits: (min, max) tuple for output clamping
        """
        self.kp = kp
        self.ki = ki
        self.kd = kd
        self.output_limits = output_limits

        # Internal state
        self.prev_error = 0.0
        self.integral = 0.0
        self.prev_time = None
        self.start_time = time.time()

        # Statistics for monitoring and tuning
        self.stats = {
            'time': 0,
            'time_delta': 0,
            'setpoint': 0,
            'measured': 0,
            'error': 0,
            'error_delta': 0,
            'p_term': 0,
            'i_term': 0,
            'd_term': 0,
            'output': 0,
            'output_raw': 0  # Before clamping
        }

    def update(self, setpoint, measured_value, current_time=None):
        """
        Calculate PID output

        Args:
            setpoint: Desired value (target temperature)
            measured_value: Current value (actual temperature)
            current_time: Optional timestamp (uses time.time() if None)

        Returns:
            Control output (0-100% typically)
        """
        if current_time is None:
            current_time = time.time()

        # Calculate time delta
        if self.prev_time is None:
            dt = 1.0  # Default to 1 second on first call
        else:
            dt = current_time - self.prev_time
            if dt <= 0:
                dt = 0.001  # Prevent division by zero

        # Calculate error
        error = setpoint - measured_value

        # Proportional term
        p_term = self.kp * error

        # Integral term with anti-windup
        self.integral += error * dt

        # Clamp integral to prevent windup
        # Map output limits to integral limits based on ki
        if self.ki != 0:
            integral_max = self.output_limits[1] / abs(self.ki)
            integral_min = self.output_limits[0] / abs(self.ki)
            self.integral = max(min(self.integral, integral_max), integral_min)

        i_term = self.ki * self.integral

        # Derivative term
        error_delta = error - self.prev_error
        d_term = self.kd * (error_delta / dt)

        # Calculate raw output
        output_raw = p_term + i_term + d_term

        # Clamp output to limits
        output = max(min(output_raw, self.output_limits[1]), self.output_limits[0])

        # Save state for next iteration
        self.prev_error = error
        self.prev_time = current_time

        # Update statistics
        self.stats = {
            'time': current_time - self.start_time,
            'time_delta': dt,
            'setpoint': setpoint,
            'measured': measured_value,
            'error': error,
            'error_delta': error_delta,
            'p_term': p_term,
            'i_term': i_term,
            'd_term': d_term,
            'output': output,
            'output_raw': output_raw,
            'kp': self.kp,
            'ki': self.ki,
            'kd': self.kd
        }

        return output

    def reset(self):
        """Reset controller state (but preserve gains)"""
        self.prev_error = 0.0
        self.integral = 0.0
        self.prev_time = None
        self.start_time = time.time()

        # Reset stats
        for key in self.stats:
            self.stats[key] = 0

    def set_gains(self, kp=None, ki=None, kd=None):
        """Update PID gains on the fly"""
        if kp is not None:
            self.kp = kp
        if ki is not None:
            self.ki = ki
        if kd is not None:
            self.kd = kd

    def get_stats(self):
        """Get current statistics dictionary"""
        return self.stats.copy()

    def __str__(self):
        """String representation"""
        return f"PID(kp={self.kp}, ki={self.ki}, kd={self.kd})"

    def __repr__(self):
        return self.__str__()
