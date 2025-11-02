# kiln/state.py
# Kiln state machine and controller with adaptive rate control

import time
from kiln.rate_monitor import TempHistory

class KilnState:
    """Kiln state constants"""
    IDLE = "IDLE"           # Not running
    RUNNING = "RUNNING"     # Actively following profile
    TUNING = "TUNING"       # PID auto-tuning in progress
    COMPLETE = "COMPLETE"   # Profile finished
    ERROR = "ERROR"         # Fault condition

class KilnController:
    """
    Main kiln control state machine with adaptive rate control

    Coordinates profile execution, step sequencing, rate monitoring,
    and adaptive control adjustments. Performs safety checks and
    state transitions.

    Does not directly control hardware - that's handled in main loop.
    """

    def __init__(self, config):
        """
        Initialize controller

        Args:
            config: Configuration object with safety limits and adaptation parameters
        """
        # State
        self.state = KilnState.IDLE
        self.active_profile = None
        self.start_time = None

        # Current values
        self.current_temp = 0.0
        self.target_temp = 0.0
        self.ssr_output = 0.0  # 0-100%

        # Safety limits from config
        self.max_temp = config.MAX_TEMP
        self.max_temp_error = config.MAX_TEMP_ERROR

        # Adaptive control configuration
        self.adaptation_enabled = getattr(config, 'ADAPTATION_ENABLED', True)
        self.adaptation_check_interval = getattr(config, 'ADAPTATION_CHECK_INTERVAL', 60)
        self.adaptation_min_step_time = getattr(config, 'ADAPTATION_MIN_STEP_TIME', 600)  # 10 min
        self.adaptation_min_time_between = getattr(config, 'ADAPTATION_MIN_TIME_BETWEEN', 300)  # 5 min
        self.adaptation_temp_error_threshold = getattr(config, 'ADAPTATION_TEMP_ERROR_THRESHOLD', 20)
        self.adaptation_rate_threshold = getattr(config, 'ADAPTATION_RATE_THRESHOLD', 0.85)
        self.adaptation_reduction_factor = getattr(config, 'ADAPTATION_REDUCTION_FACTOR', 0.9)
        self.rate_measurement_window = getattr(config, 'RATE_MEASUREMENT_WINDOW', 600)  # 10 min
        self.rate_recording_interval = getattr(config, 'RATE_RECORDING_INTERVAL', 10)

        # Step execution state
        self.current_step_index = 0
        self.step_start_time = 0
        self.step_start_temp = 0.0
        self.current_rate = 0.0  # Adapted rate (starts at desired_rate)

        # Rate monitoring
        self.temp_history = TempHistory(capacity=60)  # 10 min history at 10-sec intervals
        self.last_adaptation_check = 0
        self.last_temp_recording = 0
        self.last_adaptation_time = 0
        self.adaptation_count = 0

        # Error tracking
        self.error_message = None

        # Recovery mode state
        self.recovery_target_temp = None  # If set, we're in recovery mode
        self.recovery_start_time = None   # When recovery started (for time adjustment)

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

        # Initialize step execution
        self.current_step_index = 0
        self.step_start_time = 0
        self.step_start_temp = self.current_temp

        # Initialize rate from first step
        first_step = profile.steps[0]
        self.current_rate = first_step.get('desired_rate', 0)

        # Reset adaptation tracking
        self.temp_history.clear()
        self.last_adaptation_check = 0
        self.last_temp_recording = 0
        self.last_adaptation_time = 0
        self.adaptation_count = 0

        print(f"Starting profile: {profile.name} ({len(profile.steps)} steps)")

    def resume_profile(self, profile, elapsed_seconds, current_rate=None, last_logged_temp=None, current_temp=None):
        """
        Resume a previously interrupted firing profile

        Similar to run_profile(), but adjusts start_time and step state
        to account for time that has already elapsed.

        If current_temp is significantly lower than last_logged_temp, enters
        recovery mode to stabilize at last_logged_temp before resuming profile.

        Args:
            profile: Profile instance to resume
            elapsed_seconds: How far through the profile to resume from
            current_rate: Adapted rate to restore (from CSV log), or None for desired_rate
            last_logged_temp: Last logged temperature before crash (for recovery detection)
            current_temp: Current temperature (for recovery detection)
        """
        if self.state == KilnState.RUNNING:
            raise Exception("Cannot resume profile: kiln is already running")

        if self.state == KilnState.TUNING:
            raise Exception("Cannot resume profile: tuning is in progress")

        self.active_profile = profile
        self.state = KilnState.RUNNING

        # Adjust start time to account for elapsed progress
        current_time = time.time()
        self.start_time = current_time - elapsed_seconds

        self.error_message = None

        # Determine which step we're in based on elapsed time
        step_index, time_in_step, step_start_temp = self._find_step_for_elapsed(elapsed_seconds)

        self.current_step_index = step_index
        self.step_start_time = elapsed_seconds - time_in_step
        self.step_start_temp = step_start_temp

        # Restore or initialize rate
        current_step = profile.steps[step_index]
        if current_rate is not None and current_rate > 0:
            # Restore adapted rate from CSV log
            self.current_rate = current_rate
            print(f"Resuming with adapted rate: {current_rate:.1f}°C/h")
        else:
            # Use desired rate from step
            self.current_rate = current_step.get('desired_rate', 0)

        # Reset adaptation tracking
        self.temp_history.clear()
        self.last_adaptation_check = elapsed_seconds
        self.last_temp_recording = elapsed_seconds
        self.last_adaptation_time = elapsed_seconds
        self.adaptation_count = 0

        # Check for temperature loss and enter recovery mode if needed
        TEMP_LOSS_THRESHOLD = 5.0  # °C tolerance
        if last_logged_temp is not None and current_temp is not None:
            temp_loss = last_logged_temp - current_temp
            if temp_loss > TEMP_LOSS_THRESHOLD:
                # Enter recovery mode - hold at last logged temp until caught up
                self.recovery_target_temp = last_logged_temp
                self.recovery_start_time = time.time()
                print(f"Resuming profile: {profile.name} at step {step_index + 1}/{len(profile.steps)}, {elapsed_seconds:.1f}s elapsed")
                print(f"[Recovery] Temperature loss detected: {temp_loss:.1f}°C")
                print(f"[Recovery] Current temp: {current_temp:.1f}°C, need to reach: {last_logged_temp:.1f}°C")
                print(f"[Recovery] Profile progression paused until temperature recovered")
                return

        print(f"Resuming profile: {profile.name} at step {step_index + 1}/{len(profile.steps)}, {elapsed_seconds:.1f}s elapsed")

    def _find_step_for_elapsed(self, elapsed_seconds):
        """
        Find which step we should be in for given elapsed time

        Used for profile recovery to restore step state.

        Args:
            elapsed_seconds: Time elapsed since profile start

        Returns:
            Tuple of (step_index, time_in_current_step, step_start_temp)
        """
        if not self.active_profile or not self.active_profile.steps:
            return 0, 0, self.current_temp

        cumulative_time = 0
        current_temp = self.current_temp

        for i, step in enumerate(self.active_profile.steps):
            # Estimate step duration
            if step['type'] == 'hold':
                step_duration = step['duration']
            elif step['type'] == 'ramp':
                target = step['target_temp']
                dtemp = abs(target - current_temp)
                rate = step.get('desired_rate', 100)
                step_duration = (dtemp / rate) * 3600 if rate > 0 else 0
            else:
                step_duration = 0

            if cumulative_time + step_duration >= elapsed_seconds:
                # We're in this step
                time_in_step = elapsed_seconds - cumulative_time
                return i, time_in_step, current_temp

            # Move to next step
            cumulative_time += step_duration
            if step['type'] == 'ramp':
                current_temp = step['target_temp']

        # Past all steps - return last step
        return len(self.active_profile.steps) - 1, 0, current_temp

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

        # Reset step state
        self.current_step_index = 0
        self.step_start_time = 0
        self.current_rate = 0

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
        """Update logic for RUNNING state with adaptive control"""
        if not self.active_profile:
            self.set_error("No active profile")
            return 0

        elapsed = self.get_elapsed_time()

        # Record temperature for rate calculation (every 10 seconds)
        if elapsed - self.last_temp_recording >= self.rate_recording_interval:
            self._record_temp_for_rate(elapsed)

        # Check step completion and advance
        if self._is_step_complete(elapsed):
            if self.current_step_index >= len(self.active_profile.steps) - 1:
                # Last step complete
                self.state = KilnState.COMPLETE
                self.target_temp = 0
                print(f"Profile '{self.active_profile.name}' completed!")
                return 0
            else:
                self._advance_to_next_step(elapsed)

        # Get current step
        current_step = self.active_profile.steps[self.current_step_index]

        # Check if we're in recovery mode
        if self.recovery_target_temp is not None:
            # In recovery mode - hold at recovery target until current temp catches up
            target = self.recovery_target_temp

            # Check if recovery is complete (within 1°C of target)
            if self.current_temp >= self.recovery_target_temp - 1.0:
                recovery_duration = time.time() - self.recovery_start_time
                print(f"[Recovery] Temperature recovered! Took {recovery_duration/60:.1f} minutes")
                print(f"[Recovery] Adjusting profile clock to exclude recovery time")

                # Adjust start_time to exclude recovery duration from profile progression
                self.start_time += recovery_duration

                # Exit recovery mode
                self.recovery_target_temp = None
                self.recovery_start_time = None

                print(f"[Recovery] Resuming normal profile execution")
                # Continue to normal profile execution below
            else:
                # Still recovering - return recovery target
                self.target_temp = target
                return target

        # Check for adaptation (every minute for ramp steps)
        if (current_step['type'] == 'ramp' and
            self.adaptation_enabled and
            elapsed - self.last_adaptation_check >= self.adaptation_check_interval):

            self.last_adaptation_check = elapsed
            self._check_and_adapt_rate(elapsed, current_step)

            # If adaptation failed, state is now ERROR
            if self.state == KilnState.ERROR:
                return 0

        # Get target temperature (using possibly adapted rate)
        target = self._get_step_target_temp(elapsed, current_step)
        self.target_temp = target

        return target

    def _is_step_complete(self, elapsed):
        """
        Check if current step has completed

        Args:
            elapsed: Elapsed seconds since profile start

        Returns:
            True if step is complete
        """
        if not self.active_profile or self.current_step_index >= len(self.active_profile.steps):
            return False

        step = self.active_profile.steps[self.current_step_index]
        time_in_step = elapsed - self.step_start_time

        if step['type'] == 'hold':
            # Hold complete after duration
            return time_in_step >= step['duration']

        elif step['type'] == 'ramp':
            target = step['target_temp']

            # Heating ramp: complete when temp >= target
            if target > self.step_start_temp:
                return self.current_temp >= target

            # Cooling ramp: complete when temp <= target
            else:
                return self.current_temp <= target

        return False

    def _advance_to_next_step(self, elapsed):
        """
        Advance to next step in profile

        Args:
            elapsed: Elapsed seconds since profile start
        """
        self.current_step_index += 1
        self.step_start_time = elapsed
        self.step_start_temp = self.current_temp

        # Reset for new step
        next_step = self.active_profile.steps[self.current_step_index]
        self.current_rate = next_step.get('desired_rate', 0)
        self.temp_history.clear()
        self.last_adaptation_check = elapsed
        self.last_adaptation_time = elapsed

        step_type = next_step['type']
        step_num = self.current_step_index + 1
        total = len(self.active_profile.steps)
        print(f"[Step {step_num}/{total}] Advanced to {step_type} step (target: {next_step['target_temp']}°C)")

    def _record_temp_for_rate(self, elapsed):
        """
        Record current temperature for rate monitoring

        Args:
            elapsed: Elapsed seconds since profile start
        """
        self.temp_history.add(elapsed, self.current_temp)
        self.last_temp_recording = elapsed

    def _get_step_target_temp(self, elapsed, step):
        """
        Calculate target temperature for current step

        Args:
            elapsed: Elapsed seconds since profile start
            step: Current step dictionary

        Returns:
            Target temperature in °C
        """
        if step['type'] == 'hold':
            # Hold: target is constant
            return step['target_temp']

        elif step['type'] == 'ramp':
            time_in_step = elapsed - self.step_start_time
            hours_in_step = time_in_step / 3600.0
            target = step['target_temp']

            # Calculate using CURRENT (possibly adapted) rate
            temp_change = self.current_rate * hours_in_step

            if target > self.step_start_temp:
                # Heating ramp
                calculated = self.step_start_temp + temp_change
                return min(calculated, target)  # Clamp to step target
            else:
                # Cooling ramp
                calculated = self.step_start_temp - temp_change
                return max(calculated, target)  # Clamp to step target

        return self.current_temp  # Fallback

    def _check_and_adapt_rate(self, elapsed, step):
        """
        Check if rate adaptation is needed and perform it

        Args:
            elapsed: Elapsed seconds since profile start
            step: Current step dictionary
        """
        time_in_step = elapsed - self.step_start_time

        # Don't adapt if we don't have enough history
        if time_in_step < self.adaptation_min_step_time:
            return

        # Don't adapt too frequently
        if elapsed - self.last_adaptation_time < self.adaptation_min_time_between:
            return

        # Don't adapt if no min_rate specified
        min_rate = step.get('min_rate')
        if not min_rate:
            return

        # Check for temperature going UP during cooldown
        target = step['target_temp']
        if target < self.step_start_temp:  # This is a cooldown
            if self.current_temp > self.step_start_temp + 10:  # 10°C margin
                self.set_error(f"Temperature increasing during cooldown: {self.current_temp:.1f}°C > {self.step_start_temp:.1f}°C")
                return

        # Measure actual rate over measurement window
        actual_rate = self.temp_history.get_rate(window_seconds=self.rate_measurement_window)

        # Calculate current target temp
        target_temp = self._get_step_target_temp(elapsed, step)
        temp_error = target_temp - self.current_temp

        # Check if adaptation is needed
        needs_adaptation = (
            temp_error > self.adaptation_temp_error_threshold and  # Behind schedule
            actual_rate < self.current_rate * self.adaptation_rate_threshold  # Rate below target
        )

        if not needs_adaptation:
            return

        # Calculate proposed new rate (conservative: reduce to % of measured rate)
        proposed_rate = actual_rate * self.adaptation_reduction_factor

        # Check against minimum
        if proposed_rate >= min_rate:
            # Accept adaptation
            old_rate = self.current_rate
            self.current_rate = proposed_rate
            self.last_adaptation_time = elapsed
            self.adaptation_count += 1

            print(f"[Adaptation {self.adaptation_count}] Rate adjusted: {old_rate:.1f} → {proposed_rate:.1f}°C/h "
                  f"(actual: {actual_rate:.1f}°C/h, min: {min_rate:.1f}°C/h, error: {temp_error:.1f}°C)")

            # Force immediate temp recording so adapted rate gets logged ASAP
            self._record_temp_for_rate(elapsed)
        else:
            # Cannot achieve minimum rate - fail
            self.set_error(
                f"Cannot achieve minimum rate {min_rate:.1f}°C/h. "
                f"Actual rate: {actual_rate:.1f}°C/h after {time_in_step/60:.0f} minutes. "
                f"Kiln may be underpowered or needs maintenance."
            )

    def get_status(self):
        """
        Get current status dictionary for API/WebSocket

        Returns:
            Dictionary with comprehensive status information including
            step and adaptation details
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
            'error': self.error_message,

            # Step information
            'current_step': self.current_step_index + 1 if self.active_profile else 0,
            'total_steps': len(self.active_profile.steps) if self.active_profile else 0,
            'step_type': None,

            # Rate information
            'desired_rate': 0,
            'current_rate': round(self.current_rate, 1),
            'actual_rate': round(self.temp_history.get_rate(self.rate_measurement_window), 1),
            'min_rate': 0,
            'adaptation_count': self.adaptation_count,

            # Recovery mode information
            'is_recovering': self.recovery_target_temp is not None,
            'recovery_target_temp': round(self.recovery_target_temp, 2) if self.recovery_target_temp is not None else None
        }

        if self.active_profile:
            # Duration and progress
            remaining = max(0, self.active_profile.duration - elapsed)
            status['remaining'] = round(remaining, 1)
            status['progress'] = round(self.active_profile.get_progress(elapsed), 1)
            status['profile_duration'] = self.active_profile.duration

            # Current step details
            if self.current_step_index < len(self.active_profile.steps):
                current_step = self.active_profile.steps[self.current_step_index]
                status['step_type'] = current_step['type']
                status['desired_rate'] = current_step.get('desired_rate', 0)
                status['min_rate'] = current_step.get('min_rate', 0)

        return status

    def __str__(self):
        """String representation"""
        return f"KilnController(state={self.state}, temp={self.current_temp:.1f}°C, target={self.target_temp:.1f}°C)"

    def __repr__(self):
        return self.__str__()
