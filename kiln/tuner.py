# kiln/tuner.py
# PID Auto-Tuning State Machine
#
# This module implements the tuning sequence for PID parameter estimation.
# It heats the kiln to a target temperature, then lets it cool back down.
# Temperature data is streamed to CSV on Core 2 and analyzed offline.
#
# The tuning sequence:
# 1. Heat kiln at maximum output to target temperature
# 2. Turn off heating and let cool to (target - cooling_delta)°C
# 3. Complete - data is saved to CSV for offline analysis
#
# Use the included analyze_tuning.py script to calculate PID parameters
# from the tuning CSV file.

import time

class TuningStage:
    """Tuning stage constants"""
    HEATING = "heating"
    COOLING = "cooling"
    COMPLETE = "complete"
    ERROR = "error"

class ZieglerNicholsTuner:
    """
    PID tuner state machine for kiln controller

    Manages the heating/cooling sequence for tuning. Does not store
    temperature data in RAM - all data is streamed to CSV on Core 2
    for offline analysis.

    The tuner transitions through stages:
    HEATING -> COOLING -> COMPLETE

    After completion, use analyze_tuning.py to calculate PID parameters
    from the saved CSV file.
    """

    def __init__(self, target_temp=200, max_time=3600, cooling_delta=20):
        """
        Initialize tuner

        Args:
            target_temp: Target temperature for tuning (°C)
            max_time: Maximum tuning time before timeout (seconds)
            cooling_delta: How far below target to cool (°C) - default 20°C
        """
        self.target_temp = target_temp
        self.max_time = max_time
        self.cooling_delta = cooling_delta

        # Tuning state
        self.stage = TuningStage.HEATING
        self.start_time = None
        self.heating_complete_time = None

        # Error tracking
        self.error_message = None

    def start(self):
        """Start the tuning process"""
        self.start_time = time.time()
        self.stage = TuningStage.HEATING
        self.error_message = None
        print(f"[Tuner] Starting tuning sequence (target: {self.target_temp}°C)")
        print(f"[Tuner] Data will be streamed to CSV for offline analysis")

    def update(self, current_temp):
        """
        Update tuning state and determine SSR output

        This should be called every control loop iteration during tuning.
        Temperature data is automatically streamed to CSV on Core 2.

        Args:
            current_temp: Current temperature (°C)

        Returns:
            Tuple of (ssr_output_percent, continue_tuning)
            - ssr_output_percent: 0-100% SSR output
            - continue_tuning: True if tuning should continue, False if complete/error
        """
        # Check timeout
        if time.time() - self.start_time > self.max_time:
            self.stage = TuningStage.ERROR
            self.error_message = f"Tuning timeout ({self.max_time}s exceeded)"
            print(f"[Tuner] ERROR: {self.error_message}")
            return 0, False

        elapsed = time.time() - self.start_time

        # State machine
        if self.stage == TuningStage.HEATING:
            # Heat at maximum until target reached
            if current_temp >= self.target_temp:
                print(f"[Tuner] Target temperature reached at {elapsed:.1f}s, switching to cooling")
                self.stage = TuningStage.COOLING
                self.heating_complete_time = time.time()
                return 0, True  # Turn off SSR
            else:
                # Print status less frequently to avoid spam
                if int(elapsed) % 10 == 0:
                    print(f"[Tuner] Heating: {current_temp:.1f}°C / {self.target_temp:.1f}°C")
                return 100, True  # Full power

        elif self.stage == TuningStage.COOLING:
            # Cool until temperature drops significantly below target
            cooling_target = self.target_temp - self.cooling_delta
            if current_temp <= cooling_target:
                print(f"[Tuner] Cooled to {current_temp:.1f}°C (target was {cooling_target:.1f}°C) at {elapsed:.1f}s")
                print(f"[Tuner] Tuning complete! Data saved to CSV.")
                print(f"[Tuner] Use analyze_tuning.py to calculate PID parameters from the CSV file.")
                self.stage = TuningStage.COMPLETE
                return 0, False  # Tuning complete
            else:
                # Print status less frequently to avoid spam
                if int(elapsed) % 10 == 0:
                    print(f"[Tuner] Cooling: {current_temp:.1f}°C / {cooling_target:.1f}°C (target - {self.cooling_delta}°C)")
                return 0, True  # Keep SSR off

        # ERROR or COMPLETE - should not reach here
        return 0, False

    def get_status(self):
        """
        Get current tuning status

        Returns:
            Dictionary with tuning progress information
        """
        elapsed = 0 if self.start_time is None else time.time() - self.start_time

        status = {
            'stage': self.stage,
            'target_temp': self.target_temp,
            'elapsed': round(elapsed, 1),
            'error': self.error_message
        }

        return status
