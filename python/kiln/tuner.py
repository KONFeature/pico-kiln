# kiln/tuner.py
# PID Auto-Tuning State Machine with Step-Based Sequences
#
# This module implements multi-mode tuning sequences for PID parameter estimation.
# Each mode consists of a series of steps with specific SSR outputs, temperature targets,
# hold times, and timeouts. Temperature data is streamed to CSV on Core 2 for analysis.
#
# Tuning Modes:
# - SAFE: Quick verification test (30-45 min, max 100°C)
# - STANDARD: Good characterization (1-2 hours, max 150°C)
# - THOROUGH: Comprehensive data collection (3-4 hours, max 200°C)
#
# Use the included analyze_tuning.py script to calculate PID parameters
# from the tuning CSV file.

import time

# Tuning mode constants
MODE_SAFE = 'SAFE'
MODE_STANDARD = 'STANDARD'
MODE_THOROUGH = 'THOROUGH'
MODE_HIGH_TEMP = 'HIGH_TEMP'

class TuningStage:
    """Tuning stage constants"""
    RUNNING = "running"
    COMPLETE = "complete"
    ERROR = "error"

class TuningStep:
    """
    Represents a single step in a tuning sequence

    Each step defines fixed SSR output and optional temperature/time targets.
    Steps can complete based on:
    - Reaching target temperature
    - Plateau detection (temperature stabilizes)
    - Timeout (maximum time for step)
    """

    def __init__(self, step_name, ssr_percent, target_temp=None,
                 hold_time=0, timeout=3600, plateau_detect=False):
        """
        Initialize tuning step

        Args:
            step_name: Label for logging (e.g., "heat_50pct", "cool_to_ambient")
            ssr_percent: Fixed SSR output (0-100)
            target_temp: Optional temperature target (None = time-based only)
            hold_time: Time to hold after reaching target (seconds)
            timeout: Maximum time for this step (seconds)
            plateau_detect: Enable plateau detection for this step
        """
        self.step_name = step_name
        self.ssr_percent = ssr_percent
        self.target_temp = target_temp
        self.hold_time = hold_time
        self.timeout = timeout
        self.plateau_detect = plateau_detect

        # Runtime state
        self.start_time = None
        self.target_reached_time = None
        self.peak_temp = None  # Track peak for cooling steps

        # Plateau detection state
        self.temp_history = []  # Last 5 temperature readings
        self.last_plateau_check = 0
        self.plateau_detected = False

    def start(self, current_temp):
        """Start this step"""
        self.start_time = time.time()
        self.target_reached_time = None
        self.peak_temp = current_temp
        self.temp_history = []
        self.last_plateau_check = time.time()
        self.plateau_detected = False

    def update(self, current_temp):
        """
        Update step state and check for completion

        Args:
            current_temp: Current temperature (°C)

        Returns:
            Tuple of (ssr_output_percent, step_complete)
        """
        elapsed = time.time() - self.start_time

        # Check timeout
        if elapsed >= self.timeout:
            print(f"[Tuner Step] '{self.step_name}' timeout after {elapsed:.1f}s")
            return self.ssr_percent, True

        # Track peak temperature (useful for cooling steps)
        if current_temp > self.peak_temp:
            self.peak_temp = current_temp

        # Check plateau detection (only check every 60 seconds)
        if self.plateau_detect:
            current_time = time.time()
            if current_time - self.last_plateau_check >= 60:
                self.temp_history.append(current_temp)
                self.last_plateau_check = current_time

                # Keep only last 5 readings
                if len(self.temp_history) > 5:
                    self.temp_history.pop(0)

                # Check for plateau (5 readings, max-min < 0.5°C)
                if len(self.temp_history) == 5:
                    temp_range = max(self.temp_history) - min(self.temp_history)
                    if temp_range < 0.5:
                        print(f"[Tuner Step] '{self.step_name}' plateau detected at {current_temp:.1f}°C")
                        self.plateau_detected = True
                        return self.ssr_percent, True

        # Check temperature target
        if self.target_temp is not None:
            # Heating step - target is absolute temperature
            if self.ssr_percent > 0:
                if current_temp >= self.target_temp:
                    if self.target_reached_time is None:
                        print(f"[Tuner Step] '{self.step_name}' reached {self.target_temp}°C at {elapsed:.1f}s")
                        self.target_reached_time = time.time()

                    # Check hold time
                    hold_elapsed = time.time() - self.target_reached_time
                    if hold_elapsed >= self.hold_time:
                        print(f"[Tuner Step] '{self.step_name}' hold complete after {hold_elapsed:.1f}s")
                        return self.ssr_percent, True

            # Cooling step - target is relative to peak
            else:
                cooling_target = self.peak_temp - self.target_temp
                if current_temp <= cooling_target:
                    print(f"[Tuner Step] '{self.step_name}' cooled to {current_temp:.1f}°C (from peak {self.peak_temp:.1f}°C)")
                    return self.ssr_percent, True

        # Step continues
        return self.ssr_percent, False

    def get_status(self):
        """Get current step status for logging"""
        elapsed = 0 if self.start_time is None else time.time() - self.start_time

        status = {
            'step_name': self.step_name,
            'ssr_percent': self.ssr_percent,
            'target_temp': self.target_temp,
            'elapsed': round(elapsed, 1),
            'timeout': self.timeout,
            'plateau_detected': self.plateau_detected,
            'peak_temp': round(self.peak_temp, 1) if self.peak_temp else None
        }

        return status


class ZieglerNicholsTuner:
    """
    Multi-mode PID tuner with step-based sequences

    Supports three tuning modes with different time/temperature profiles:
    - SAFE: Quick safety verification (30-45 min, max 100°C)
    - STANDARD: Good PID characterization (1-2 hours, max 150°C)
    - THOROUGH: Comprehensive data (3-4 hours, max 200°C)

    Temperature data is streamed to CSV on Core 2 for offline analysis.
    Use analyze_tuning.py to calculate PID parameters from the CSV file.
    """

    def __init__(self, mode=MODE_STANDARD, max_temp=None):
        """
        Initialize tuner with specified mode

        Args:
            mode: Tuning mode ('SAFE', 'STANDARD', or 'THOROUGH')
            max_temp: Maximum temperature for safety (°C), None = use mode default
        """
        if mode not in [MODE_SAFE, MODE_STANDARD, MODE_THOROUGH, MODE_HIGH_TEMP]:
            raise ValueError(f"Invalid mode: {mode}. Must be 'SAFE', 'STANDARD', 'THOROUGH', or 'HIGH_TEMP'")

        self.mode = mode
        
        # Set default max_temp based on mode if not provided
        if max_temp is None:
            if mode == MODE_SAFE:
                max_temp = 200
            elif mode == MODE_STANDARD:
                max_temp = 900
            elif mode == MODE_THOROUGH:
                max_temp = 900
            elif mode == MODE_HIGH_TEMP:
                max_temp = 900
        
        self.max_temp = max_temp

        # Build step sequence based on mode
        self.steps = self._build_step_sequence()

        # Tuning state
        self.stage = TuningStage.RUNNING
        self.start_time = None
        self.current_step_index = 0
        self.current_step = None

        # Error tracking
        self.error_message = None

    def _build_step_sequence(self):
        """
        Build step sequence based on tuning mode

        Returns:
            List of TuningStep objects
        """
        if self.mode == MODE_SAFE:
            # SAFE mode: Quick verification (30-45 min)
            return [
                TuningStep(
                    step_name="heat_60pct_to_100C",
                    ssr_percent=60,
                    target_temp=min(100, self.max_temp),
                    hold_time=0,
                    timeout=2400  # 20 min timeout
                ),
                TuningStep(
                    step_name="hold_30pct_5min",
                    ssr_percent=30,
                    target_temp=None,
                    hold_time=0,
                    timeout=300  # 5 min
                ),
                TuningStep(
                    step_name="cool_to_50C",
                    ssr_percent=0,
                    target_temp=50,  # Cool 50°C below peak
                    hold_time=0,
                    timeout=1800  # 30 min timeout
                )
            ]

        elif self.mode == MODE_STANDARD:
            # STANDARD mode: Good characterization (1-2 hours)
            return [
                TuningStep(
                    step_name="heat_25pct_plateau",
                    ssr_percent=25,
                    target_temp=None,
                    hold_time=0,
                    timeout=1800,  # 30 min timeout
                    plateau_detect=True
                ),
                TuningStep(
                    step_name="cool_10min",
                    ssr_percent=0,
                    target_temp=None,
                    hold_time=0,
                    timeout=1200
                ),
                TuningStep(
                    step_name="heat_50pct_plateau",
                    ssr_percent=50,
                    target_temp=None,
                    hold_time=0,
                    timeout=1800,
                    plateau_detect=True
                ),
                TuningStep(
                    step_name="cool_10min",
                    ssr_percent=0,
                    target_temp=None,
                    hold_time=0,
                    timeout=1200
                ),
                TuningStep(
                    step_name="heat_75pct_plateau",
                    ssr_percent=75,
                    target_temp=None,
                    hold_time=0,
                    timeout=1800,
                    plateau_detect=True
                ),
                TuningStep(
                    step_name="cool_to_ambient",
                    ssr_percent=0,
                    target_temp=None,
                    hold_time=0,
                    timeout=3600  # 60 min timeout for full cooldown
                )
            ]

        elif self.mode == MODE_THOROUGH:
            # THOROUGH mode: Maximum data (3-4 hours)
            steps = []

            # For each power level: heat, hold, cool
            for power in [20, 40, 60, 80]:
                steps.extend([
                    TuningStep(
                        step_name=f"heat_{power}pct_plateau",
                        ssr_percent=power,
                        target_temp=None,
                        hold_time=0,
                        timeout=2700,  # 45 min timeout
                        plateau_detect=True
                    ),
                    TuningStep(
                        step_name=f"hold_{power}pct_5min",
                        ssr_percent=power,
                        target_temp=None,
                        hold_time=0,
                        timeout=300  # 5 min
                    ),
                    TuningStep(
                        step_name=f"cool_30C",
                        ssr_percent=0,
                        target_temp=50,  # Cool 30°C below peak
                        hold_time=0,
                        timeout=1200
                    )
                ])

            # Final cooldown
            steps.append(
                TuningStep(
                    step_name="final_cooldown",
                    ssr_percent=0,
                    target_temp=None,
                    hold_time=0,
                    timeout=3600
                )
            )

            return steps

        elif self.mode == MODE_HIGH_TEMP:
            # HIGH_TEMP mode: Fast heatup for high thermal mass kilns (3-4 hours)
            # Skip slow 0-200°C "insulation charging" phase
            # Characterize dynamics at 200-500°C range
            return [
                # Step 1: Blast through low-temp phase at full power
                TuningStep(
                    step_name="fast_heat_to_200C",
                    ssr_percent=100,
                    target_temp=200,
                    hold_time=0,
                    timeout=3600  # 60 min timeout for high thermal mass
                ),

                # Step 2: Cool slightly to reset
                TuningStep(
                    step_name="cool_10min",
                    ssr_percent=0,
                    target_temp=None,
                    hold_time=0,
                    timeout=600  # 10 min
                ),

                # Step 3: Characterize at 60% SSR (should reach ~300-350°C)
                TuningStep(
                    step_name="heat_60pct_plateau",
                    ssr_percent=60,
                    target_temp=None,
                    hold_time=0,
                    timeout=1800,  # 30 min timeout
                    plateau_detect=True
                ),

                # Step 4: Cool reset
                TuningStep(
                    step_name="cool_10min",
                    ssr_percent=0,
                    target_temp=None,
                    hold_time=0,
                    timeout=600  # 10 min
                ),

                # Step 5: Characterize at 80% SSR (should reach ~400-450°C)
                TuningStep(
                    step_name="heat_80pct_plateau",
                    ssr_percent=80,
                    target_temp=None,
                    hold_time=0,
                    timeout=1800,  # 30 min timeout
                    plateau_detect=True
                ),

                # Step 6: Cool reset
                TuningStep(
                    step_name="cool_10min",
                    ssr_percent=0,
                    target_temp=None,
                    hold_time=0,
                    timeout=600  # 10 min
                ),

                # Step 7: Full power push to max temp
                TuningStep(
                    step_name="heat_100pct_to_max",
                    ssr_percent=100,
                    target_temp=min(600, self.max_temp),
                    hold_time=300,  # Hold 5 min at max
                    timeout=1800,  # 30 min timeout
                ),

                # Step 8: Final cooldown
                TuningStep(
                    step_name="final_cooldown",
                    ssr_percent=0,
                    target_temp=None,
                    hold_time=0,
                    timeout=3600  # 60 min
                )
            ]

        return []

    def start(self):
        """Start the tuning process"""
        self.start_time = time.time()
        self.stage = TuningStage.RUNNING
        self.current_step_index = 0
        self.error_message = None

        # Start first step
        self.current_step = self.steps[0]

        print(f"[Tuner] Starting {self.mode} tuning mode")
        print(f"[Tuner] Max temp: {self.max_temp}°C")
        print(f"[Tuner] Total steps: {len(self.steps)}")
        print(f"[Tuner] Data will be streamed to CSV for offline analysis")
        print(f"[Tuner] Step 1/{len(self.steps)}: {self.current_step.step_name}")

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
        # Safety check: max temperature
        if current_temp > self.max_temp:
            self.stage = TuningStage.ERROR
            self.error_message = f"Temperature {current_temp:.1f}°C exceeds maximum {self.max_temp}°C"
            print(f"[Tuner] ERROR: {self.error_message}")
            return 0, False

        # Start current step if not started
        if self.current_step.start_time is None:
            self.current_step.start(current_temp)

        # Update current step
        ssr_output, step_complete = self.current_step.update(current_temp)

        # Check if step completed
        if step_complete:
            # Move to next step
            self.current_step_index += 1

            # Check if all steps complete
            if self.current_step_index >= len(self.steps):
                elapsed_total = time.time() - self.start_time
                print(f"[Tuner] All steps complete after {elapsed_total:.1f}s!")
                print(f"[Tuner] Tuning complete! Data saved to CSV.")
                print(f"[Tuner] Use analyze_tuning.py to calculate PID parameters from the CSV file.")
                self.stage = TuningStage.COMPLETE
                return 0, False

            # Start next step
            self.current_step = self.steps[self.current_step_index]
            print(f"[Tuner] Step {self.current_step_index + 1}/{len(self.steps)}: {self.current_step.step_name}")
            self.current_step.start(current_temp)

            # Return next step's SSR output
            return self.current_step.ssr_percent, True

        # Continue current step
        return ssr_output, True

    def get_status(self):
        """
        Get current tuning status

        Returns:
            Dictionary with tuning progress information
        """
        elapsed_total = 0 if self.start_time is None else time.time() - self.start_time

        status = {
            'stage': self.stage,
            'mode': self.mode,
            'max_temp': self.max_temp,
            'elapsed': round(elapsed_total, 1),
            'step_index': self.current_step_index if self.current_step else 0,
            'total_steps': len(self.steps),
            'error': self.error_message
        }

        # Add current step status
        if self.current_step:
            step_status = self.current_step.get_status()
            status.update(step_status)

        return status
