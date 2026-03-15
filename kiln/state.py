# kiln/state.py
# Kiln state machine and controller with rolling rate control

import time
from micropython import const
from kiln.rate_monitor import TempHistory

# Module-level constants for temperature thresholds
TEMP_LOSS_THRESHOLD = const(5)  # Temperature loss tolerance in °C for recovery detection

class KilnState:
    """Kiln state constants - using integer const for memory optimization"""
    IDLE = const(0)        # Not running
    RUNNING = const(1)     # Actively following profile
    TUNING = const(2)      # PID auto-tuning in progress
    COMPLETE = const(3)    # Profile finished
    ERROR = const(4)       # Fault condition

class KilnController:
    """
    Main kiln control state machine with rolling rate control

    Coordinates profile execution, step sequencing, rate monitoring,
    and stall detection. Performs safety checks and
    state transitions.

    Does not directly control hardware - that's handled in main loop.
    """

    def __init__(self, config):
        """
        Initialize controller

        Args:
            config: Configuration object with safety limits and stall detection parameters
        """
        self.state = KilnState.IDLE
        self.active_profile = None
        self.start_time = None
        self.elapsed_offset = 0.0
        self.last_update_time = None

        self.current_temp = 0.0
        self.target_temp = 0.0
        self.ssr_output = 0.0

        self.max_temp = config.MAX_TEMP
        self.max_temp_error = config.MAX_TEMP_ERROR

        # Rate measurement config
        self.rate_measurement_window = getattr(config, 'RATE_MEASUREMENT_WINDOW', 600)
        self.rate_recording_interval = getattr(config, 'RATE_RECORDING_INTERVAL', 10)

        # Stall detection config
        self.stall_check_interval = getattr(config, 'STALL_CHECK_INTERVAL', 60)
        self.stall_consecutive_fails = getattr(config, 'STALL_CONSECUTIVE_FAILS', 3)
        self.stall_min_step_time = getattr(config, 'STALL_MIN_STEP_TIME', 600)

        # Step execution state
        self.current_step_index = 0
        self.step_start_time = 0
        self.step_start_temp = 0.0

        # Rate monitoring
        self.temp_history = TempHistory(capacity=60)
        self.last_temp_recording = 0
        self.last_stall_check = 0
        self.stall_fail_count = 0

        self.error_message = None

        # Recovery mode state
        self.recovery_target_temp = None
        self.recovery_start_time = None

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
        self.elapsed_offset = 0.0
        self.last_update_time = None
        self.error_message = None

        self.current_step_index = 0
        self.step_start_time = 0
        self.step_start_temp = self.current_temp

        self.temp_history.clear()
        self.last_temp_recording = 0
        self.last_stall_check = 0
        self.stall_fail_count = 0

        print(f"Starting profile: {profile.name} ({len(profile.steps)} steps)")

    def resume_profile(self, profile, elapsed_seconds, last_logged_temp=None, current_temp=None, step_index=None):
        """
        Resume a previously interrupted firing profile

        Similar to run_profile(), but adjusts start_time and step state
        to account for time that has already elapsed.

        If current_temp is significantly lower than last_logged_temp, enters
        recovery mode to stabilize at last_logged_temp before resuming profile.

        Args:
            profile: Profile instance to resume
            elapsed_seconds: How far through the profile to resume from
            last_logged_temp: Last logged temperature before crash (for recovery detection)
            current_temp: Current temperature (for recovery detection)
            step_index: Step index from CSV log (0-based), or None to calculate
        """
        if self.state == KilnState.RUNNING:
            raise Exception("Cannot resume profile: kiln is already running")

        if self.state == KilnState.TUNING:
            raise Exception("Cannot resume profile: tuning is in progress")

        self.active_profile = profile
        self.state = KilnState.RUNNING

        # Store elapsed seconds directly (NTP-safe)
        self.start_time = time.time()
        self.elapsed_offset = elapsed_seconds
        self.last_update_time = None  # Will be set on first get_elapsed_time()

        self.error_message = None

        # Calculate timing information from elapsed time
        # This gives us accurate time_in_step and step_start_temp
        calc_step_index, time_in_step, step_start_temp = self._find_step_for_elapsed(elapsed_seconds)
        
        # Use step_index from CSV if available (more reliable), otherwise use calculated
        if step_index is not None:
            # CSV knows the actual step that was running
            # Use it instead of calculated (handles recovery timing changes)
            print(f"[Recovery] Using step index from CSV: {step_index} (calculated: {calc_step_index})")
            self.current_step_index = step_index
        else:
            # No CSV step_index - use calculated value
            self.current_step_index = calc_step_index
        self.step_start_time = elapsed_seconds - time_in_step
        
        # For ramp steps, calculate step_start_temp by working backwards from last_logged_temp
        current_step = profile.steps[self.current_step_index]
        if current_step['type'] == 'ramp' and last_logged_temp is not None and time_in_step > 0:
            rate = current_step.get('desired_rate', 100)
            hours_in_step = time_in_step / 3600.0
            temp_change = rate * hours_in_step
            
            target = current_step['target_temp']
            if target > last_logged_temp:
                self.step_start_temp = last_logged_temp - temp_change
            else:
                self.step_start_temp = last_logged_temp + temp_change
            
            print(f"[Recovery] Calculated step_start_temp: {self.step_start_temp:.1f}°C (working backwards from {last_logged_temp:.1f}°C)")
        else:
            self.step_start_temp = step_start_temp

        # Reset rate monitoring and stall detection
        self.temp_history.clear()
        self.last_temp_recording = elapsed_seconds
        self.last_stall_check = elapsed_seconds
        self.stall_fail_count = 0

        # Check for temperature loss and enter recovery mode if needed
        # BUT: Don't recover during cooling steps (temp drop is expected)
        if last_logged_temp is not None and current_temp is not None:
            # Determine if current step is a cooling operation
            is_cooling = (current_step['type'] == 'cooling' or 
                          (current_step['type'] == 'ramp' and 
                           current_step['target_temp'] < self.step_start_temp))
            
            temp_loss = last_logged_temp - current_temp
            if temp_loss > TEMP_LOSS_THRESHOLD and not is_cooling:
                # Enter recovery mode - hold at last logged temp until caught up
                self.recovery_target_temp = last_logged_temp
                self.recovery_start_time = time.time()
                print(f"Resuming profile: {profile.name} at step {self.current_step_index + 1}/{len(profile.steps)}, {elapsed_seconds:.1f}s elapsed")
                print(f"[Recovery] Temperature loss detected: {temp_loss:.1f}°C")
                print(f"[Recovery] Current temp: {current_temp:.1f}°C, need to reach: {last_logged_temp:.1f}°C")
                print(f"[Recovery] Profile progression paused until temperature recovered")
                return
            elif temp_loss > TEMP_LOSS_THRESHOLD and is_cooling:
                # Temperature dropped during cooling - this is expected, not a problem
                print(f"Resuming profile: {profile.name} at step {self.current_step_index + 1}/{len(profile.steps)}, {elapsed_seconds:.1f}s elapsed")
                print(f"[Recovery] Temperature drop during cooling: {temp_loss:.1f}°C (expected)")
                print(f"[Recovery] Continuing cooling from current temp: {current_temp:.1f}°C")
                return

        print(f"Resuming profile: {profile.name} at step {self.current_step_index + 1}/{len(profile.steps)}, {elapsed_seconds:.1f}s elapsed")

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
        # Track theoretical temperature progression through profile
        # Start from room temperature (typical kiln start point)
        profile_temp = 20

        for i, step in enumerate(self.active_profile.steps):
            # Estimate step duration based on theoretical progression
            if step['type'] == 'hold':
                step_duration = step['duration']
            elif step['type'] == 'ramp':
                target = step['target_temp']
                dtemp = abs(target - profile_temp)
                # Use desired_rate if specified, otherwise use default 100°C/h
                rate = step.get('desired_rate', 100)
                step_duration = (dtemp / rate) * 3600 if rate > 0 else 0
            elif step['type'] == 'cooling':
                # Natural cooling step
                target = step.get('target_temp')
                if target is not None:
                    dtemp = abs(profile_temp - target)
                    step_duration = (dtemp / 100.0) * 3600  # Estimate 100°C/h natural cooling
                else:
                    step_duration = 0  # Unknown duration
            else:
                step_duration = 0

            if cumulative_time + step_duration >= elapsed_seconds:
                # We're in this step - profile_temp is where this step started
                time_in_step = elapsed_seconds - cumulative_time
                return i, time_in_step, profile_temp

            # Move to next step
            cumulative_time += step_duration
            if step['type'] == 'ramp':
                profile_temp = step['target_temp']
            elif step['type'] == 'cooling':
                target = step.get('target_temp')
                if target is not None:
                    profile_temp = target

        # Past all steps - return last step
        return len(self.active_profile.steps) - 1, 0, profile_temp

    def stop(self):
        """
        Emergency stop - immediately halt profile
        Sets target to 0 but does NOT turn off SSR (main loop handles that)
        """
        print(f"Stop requested (was in {self.state} state)")
        self._reset_to_idle()

    def _reset_to_idle(self):
        """
        Full reset of all runtime state back to IDLE.
        
        Used by stop(), clear_error(), and auto-transition from COMPLETE.
        Does NOT print any messages — callers handle their own logging.
        """
        self.state = KilnState.IDLE
        self.active_profile = None
        self.target_temp = 0
        self.start_time = None
        self.elapsed_offset = 0.0
        self.last_update_time = None
        self.error_message = None

        self.current_step_index = 0
        self.step_start_time = 0
        self.step_start_temp = 0.0

        self.recovery_target_temp = None
        self.recovery_start_time = None

        self.temp_history.clear()
        self.last_temp_recording = 0
        self.last_stall_check = 0
        self.stall_fail_count = 0

    def set_error(self, message):
        """Set error state with message"""
        self.state = KilnState.ERROR
        self.error_message = message
        self.target_temp = 0
        print(f"ERROR: {message}")

    def clear_error(self):
        """
        Clear error state and return to idle
        
        This resets the controller to a safe idle state, clearing any error
        messages and resetting internal state. The SSR must be turned off
        by the calling code (control thread).
        """
        if self.state != KilnState.ERROR:
            print(f"[KilnController] Cannot clear error: not in error state (current state: {self.state})")
            return False
        
        print("[KilnController] Clearing error state and returning to idle")
        self._reset_to_idle()
        return True

    def get_elapsed_time(self):
        """
        Get elapsed time in profile
        
        Uses monotonic time deltas to avoid NTP jump issues.
        For recovery, starts from elapsed_offset instead of 0.

        Returns:
            Elapsed seconds since profile start (or resumed offset)
        """
        if self.start_time is None:
            return 0
        
        current_time = time.time()
        
        # First call after start/resume
        if self.last_update_time is None:
            self.last_update_time = current_time
            return self.elapsed_offset
        
        # Calculate delta since last update (immune to NTP jumps)
        delta = current_time - self.last_update_time
        
        # Sanity check: if delta is negative or huge, NTP jumped
        if delta < 0 or delta > 60:  # Max 60s between updates is reasonable
            print(f"[KilnController] Time jump detected: {delta:.1f}s - ignoring")
            delta = 1.0  # Assume 1 second passed
        
        self.last_update_time = current_time
        self.elapsed_offset += delta
        
        return self.elapsed_offset

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

        # Check for stall condition (every check interval for ramp steps with min_rate)
        min_rate = current_step.get('min_rate')
        if (current_step['type'] == 'ramp' and
            min_rate and
            elapsed - self.last_stall_check >= self.stall_check_interval):

            self.last_stall_check = elapsed
            time_in_step = elapsed - self.step_start_time

            if time_in_step >= self.stall_min_step_time:
                actual_rate = self.temp_history.get_rate(self.rate_measurement_window)
                if actual_rate < min_rate:
                    self.stall_fail_count += 1
                    print(f"[Stall check] Rate {actual_rate:.1f}°C/h < min {min_rate:.1f}°C/h "
                          f"({self.stall_fail_count}/{self.stall_consecutive_fails})")
                    if self.stall_fail_count >= self.stall_consecutive_fails:
                        self.set_error(
                            f"Stall detected: {actual_rate:.1f}°C/h below minimum "
                            f"{min_rate:.1f}°C/h for {self.stall_consecutive_fails} "
                            f"consecutive checks. Kiln may be underpowered or needs maintenance."
                        )
                        return 0
                else:
                    self.stall_fail_count = 0

        # Get target temperature (always uses desired_rate from step)
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

        elif step['type'] == 'cooling':
            # Natural cooling step
            target = step.get('target_temp')
            if target is not None:
                # Complete when cooled to target temperature
                return self.current_temp <= target
            else:
                # No target specified - never completes (must be last step or manually stopped)
                return False

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

        next_step = self.active_profile.steps[self.current_step_index]
        self.temp_history.clear()
        self.last_stall_check = elapsed
        self.stall_fail_count = 0

        step_type = next_step['type']
        step_num = self.current_step_index + 1
        total = len(self.active_profile.steps)
        
        # Format target temp display (handle cooling steps with optional target)
        if step_type == 'cooling':
            target_temp = next_step.get('target_temp')
            if target_temp is not None:
                target_str = f"{target_temp}°C"
            else:
                target_str = "natural cooling"
        else:
            target_str = f"{next_step['target_temp']}°C"
        
        print(f"[Step {step_num}/{total}] Advanced to {step_type} step (target: {target_str})")

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

            temp_change = step.get('desired_rate', 100) * hours_in_step

            if target > self.step_start_temp:
                # Heating ramp
                calculated = self.step_start_temp + temp_change
                return min(calculated, target)  # Clamp to step target
            else:
                # Cooling ramp
                calculated = self.step_start_temp - temp_change
                return max(calculated, target)  # Clamp to step target

        elif step['type'] == 'cooling':
            # Natural cooling: target = 0 ensures SSR stays off (PID output = 0)
            return 0

        return self.current_temp  # Fallback

    def get_status(self):
        """
        Get current status dictionary for API/WebSocket

        Returns:
            Dictionary with comprehensive status information including
            step and stall detection details
        """
        elapsed = self.get_elapsed_time()

        status = {
            'state': self.state,
            'current_temp': round(self.current_temp, 2),
            'target_temp': round(self.target_temp, 2),
            'ssr_output': round(self.ssr_output, 2),
            'profile': self.active_profile.name if self.active_profile else None,
            'elapsed': round(elapsed, 1),
            'error': self.error_message,

            # Step information
            'current_step': self.current_step_index + 1 if self.active_profile else 0,
            'total_steps': len(self.active_profile.steps) if self.active_profile else 0,
            'step_type': None,

            # Rate information
            'desired_rate': 0,
            'measured_rate': round(self.temp_history.get_rate(self.rate_measurement_window), 1),

            # Recovery mode information
            'is_recovering': self.recovery_target_temp is not None,
            'recovery_target_temp': round(self.recovery_target_temp, 2) if self.recovery_target_temp is not None else None
        }

        if self.active_profile and self.active_profile.steps:
            # Current step details
            if 0 <= self.current_step_index < len(self.active_profile.steps):
                try:
                    current_step = self.active_profile.steps[self.current_step_index]
                    status['step_type'] = current_step.get('type')
                    # Use desired_rate if specified, otherwise 0 (for cooldown/unspecified)
                    status['desired_rate'] = current_step.get('desired_rate', 0)
                except (IndexError, KeyError, TypeError) as e:
                    # Gracefully handle any step access errors
                    print(f"[KilnController] Warning: Error accessing step {self.current_step_index}: {e}")
                    pass

        return status

    def __str__(self):
        """String representation"""
        return f"KilnController(state={self.state}, temp={self.current_temp:.1f}°C, target={self.target_temp:.1f}°C)"

    def __repr__(self):
        return self.__str__()
