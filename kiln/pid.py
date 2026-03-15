# kiln/pid.py
# PID controller with conditional integration anti-windup

import time
import micropython

class PID:
    """
    PID controller with conditional integration anti-windup.

    Uses conditional integration (Åström & Hägglund) to prevent integral
    windup: the integral term is frozen when the output is saturated in
    the same direction as the error. This allows the integral to unwind
    when the error reverses, but prevents useless accumulation when the
    controller is already at its output limit.

    References:
    - Åström & Hägglund, "Advanced PID Control"
    - Siemens FB41 PID (INT_HOLD anti-windup)
    - https://github.com/Dlloydev/QuickPID (iAwCondition mode)
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

    # Performance optimization: Called every control cycle (1 Hz) with heavy floating-point math
    @micropython.native
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

        # Derivative term (compute before integral so we can use all three
        # to predict output for the conditional integration check)
        error_delta = error - self.prev_error
        d_term = self.kd * (error_delta / dt)

        # Conditional integration anti-windup (Åström & Hägglund):
        # Predict what the output would be if we accumulated normally.
        # If that output would saturate AND the error is pushing further
        # into saturation, freeze the integral. Otherwise, accumulate.
        candidate_integral = self.integral + error * dt
        i_term_candidate = self.ki * candidate_integral
        output_candidate = p_term + i_term_candidate + d_term

        saturated_high = output_candidate >= self.output_limits[1] and error > 0
        saturated_low = output_candidate <= self.output_limits[0] and error < 0

        if not (saturated_high or saturated_low):
            self.integral = candidate_integral

        # Safety clamp: catch edge cases like sensor noise spikes unfreezing
        # the integral via a large negative D-term
        if self.ki > 0:
            integral_max = self.output_limits[1] / self.ki
            integral_min = self.output_limits[0] / self.ki
            self.integral = max(min(self.integral, integral_max), integral_min)

        i_term = self.ki * self.integral

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
            'kd': self.kd,
            'integral_frozen': saturated_high or saturated_low
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
            self.stats[key] = False if key == 'integral_frozen' else 0

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
