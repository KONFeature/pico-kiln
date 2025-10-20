# kiln/state.py
# Kiln state machine and controller

import time

class KilnState:
    """Kiln state constants"""
    IDLE = "IDLE"           # Not running
    RUNNING = "RUNNING"     # Actively following profile
    TUNING = "TUNING"       # PID auto-tuning in progress
    COMPLETE = "COMPLETE"   # Profile finished
    ERROR = "ERROR"         # Fault condition

class KilnController:
    """
    Main kiln control state machine

    Coordinates profile execution, state transitions, and safety checks.
    Does not directly control hardware - that's handled in main loop.
    """

    def __init__(self, max_temp=1300, max_temp_error=50):
        """
        Initialize controller

        Args:
            max_temp: Maximum safe temperature (°C)
            max_temp_error: Maximum deviation from target before error (°C)
        """
        # State
        self.state = KilnState.IDLE
        self.active_profile = None
        self.start_time = None

        # Current values
        self.current_temp = 0.0
        self.target_temp = 0.0
        self.ssr_output = 0.0  # 0-100%

        # Safety limits
        self.max_temp = max_temp
        self.max_temp_error = max_temp_error

        # Error tracking
        self.error_message = None

    def run_profile(self, profile):
        """
        Start running a firing profile

        Args:
            profile: Profile instance to run
        """
        if self.state == KilnState.RUNNING:
            raise Exception("Cannot start profile: kiln is already running")

        if self.state == KilnState.TUNING:
            raise Exception("Cannot start profile: tuning is in progress")

        self.active_profile = profile
        self.state = KilnState.RUNNING
        self.start_time = time.time()
        self.error_message = None

        print(f"Starting profile: {profile.name}")

    def resume_profile(self, profile, elapsed_seconds):
        """
        Resume a previously interrupted firing profile

        Similar to run_profile(), but adjusts start_time to account for
        time that has already elapsed in the profile execution.

        Args:
            profile: Profile instance to resume
            elapsed_seconds: How far through the profile to resume from
        """
        if self.state == KilnState.RUNNING:
            raise Exception("Cannot resume profile: kiln is already running")

        if self.state == KilnState.TUNING:
            raise Exception("Cannot resume profile: tuning is in progress")

        self.active_profile = profile
        self.state = KilnState.RUNNING

        # Adjust start time to account for elapsed progress
        # This makes get_elapsed_time() return the correct value
        current_time = time.time()
        self.start_time = current_time - elapsed_seconds

        self.error_message = None

        print(f"Resuming profile: {profile.name} at {elapsed_seconds:.1f}s elapsed")

    def stop(self):
        """
        Emergency stop - immediately halt profile
        Sets target to 0 but does NOT turn off SSR (main loop handles that)
        """
        print(f"Stop requested (was in {self.state} state)")

        self.state = KilnState.IDLE
        self.active_profile = None
        self.target_temp = 0
        self.start_time = None
        self.error_message = None

    def set_error(self, message):
        """Set error state with message"""
        self.state = KilnState.ERROR
        self.error_message = message
        self.target_temp = 0
        print(f"ERROR: {message}")

    def get_elapsed_time(self):
        """
        Get elapsed time in profile

        Returns:
            Elapsed seconds since profile start
        """
        if self.start_time is None:
            return 0

        return time.time() - self.start_time

    def update(self, current_temp):
        """
        Update controller state based on current temperature

        This should be called every control loop iteration.
        Returns the target temperature for the PID controller.

        Args:
            current_temp: Current measured temperature

        Returns:
            Target temperature for PID
        """
        self.current_temp = current_temp

        # Safety check: max temperature
        if current_temp > self.max_temp:
            self.set_error(f"Temperature {current_temp:.1f}C exceeds maximum {self.max_temp}C")
            return 0

        # Handle different states
        if self.state == KilnState.RUNNING:
            return self._update_running()
        elif self.state == KilnState.TUNING:
            # Tuning is handled separately in control_thread
            # This should not be called during tuning, but return 0 for safety
            return 0
        else:
            # IDLE, COMPLETE, or ERROR - turn off heating
            return 0

    def _update_running(self):
        """Update logic for RUNNING state"""
        if not self.active_profile:
            self.set_error("No active profile")
            return 0

        elapsed = self.get_elapsed_time()

        # Check if profile is complete
        if self.active_profile.is_complete(elapsed):
            self.state = KilnState.COMPLETE
            self.target_temp = 0
            print(f"Profile '{self.active_profile.name}' completed!")
            return 0

        # Get target from profile
        target = self.active_profile.get_target_temp(elapsed)
        self.target_temp = target

        # Safety check: temperature tracking error
        error = abs(target - self.current_temp)
        if error > self.max_temp_error and target > 100:  # Only check when hot
            self.set_error(f"Temperature error {error:.1f}°C exceeds maximum {self.max_temp_error}°C")
            return 0

        return target

    def get_status(self):
        """
        Get current status dictionary for API/WebSocket

        Returns:
            Dictionary with comprehensive status information
        """
        elapsed = self.get_elapsed_time()

        status = {
            'state': self.state,
            'current_temp': round(self.current_temp, 2),
            'target_temp': round(self.target_temp, 2),
            'ssr_output': round(self.ssr_output, 2),
            'profile': self.active_profile.name if self.active_profile else None,
            'elapsed': round(elapsed, 1),
            'remaining': 0,
            'progress': 0,
            'error': self.error_message
        }

        if self.active_profile:
            remaining = max(0, self.active_profile.duration - elapsed)
            status['remaining'] = round(remaining, 1)
            status['progress'] = round(self.active_profile.get_progress(elapsed), 1)
            status['profile_duration'] = self.active_profile.duration

        return status

    def __str__(self):
        """String representation"""
        return f"KilnController(state={self.state}, temp={self.current_temp:.1f}°C, target={self.target_temp:.1f}°C)"

    def __repr__(self):
        return self.__str__()
