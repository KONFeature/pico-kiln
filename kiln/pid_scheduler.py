# kiln/pid_scheduler.py
# PID gain scheduling for temperature-dependent control
#
# This module implements gain scheduling to use different PID parameters
# at different temperature ranges. This compensates for changing kiln
# thermal dynamics across the wide 0-1300°C operating range.

class PIDGainScheduler:
    """
    Selects PID gains based on current temperature and thermal model.

    Uses gain scheduling: different PID parameters for different temperature ranges.
    This improves control performance across wide temperature ranges where system
    dynamics change significantly.

    Example:
        # Create scheduler with thermal model from config
        scheduler = PIDGainScheduler(
            thermal_model=[
                {'temp_min': 0, 'temp_max': 300, 'kp': 25.0, 'ki': 180.0, 'kd': 160.0},
                {'temp_min': 300, 'temp_max': 700, 'kp': 20.0, 'ki': 150.0, 'kd': 120.0},
                {'temp_min': 700, 'temp_max': 9999, 'kp': 15.0, 'ki': 100.0, 'kd': 80.0}
            ],
            default_kp=25.0,
            default_ki=180.0,
            default_kd=160.0
        )

        # In control loop
        kp, ki, kd = scheduler.get_gains(current_temp)
        if scheduler.gains_changed():
            pid.set_gains(kp, ki, kd)
    """

    def __init__(self, thermal_model=None, default_kp=25.0, default_ki=180.0, default_kd=160.0):
        """
        Initialize gain scheduler.

        Args:
            thermal_model: List of dicts with temp ranges and PID params, or None
                          Each dict must have: temp_min, temp_max, kp, ki, kd
            default_kp: Fallback proportional gain
            default_ki: Fallback integral gain
            default_kd: Fallback derivative gain
        """
        self.thermal_model = thermal_model
        self.default_kp = default_kp
        self.default_ki = default_ki
        self.default_kd = default_kd

        # Track current gains to detect changes
        self._current_kp = default_kp
        self._current_ki = default_ki
        self._current_kd = default_kd
        self._gains_just_changed = False

        # Validate thermal model
        if thermal_model is not None:
            self._validate_thermal_model()
            print(f"[PIDGainScheduler] Initialized with {len(thermal_model)} temperature ranges")
            for i, range_spec in enumerate(thermal_model):
                print(f"  Range {i+1}: {range_spec['temp_min']:.0f}-{range_spec['temp_max']:.0f}°C "
                      f"-> Kp={range_spec['kp']:.3f} Ki={range_spec['ki']:.4f} Kd={range_spec['kd']:.3f}")
        else:
            print(f"[PIDGainScheduler] Using default gains for all temperatures "
                  f"(Kp={default_kp:.3f} Ki={default_ki:.4f} Kd={default_kd:.3f})")

    def _validate_thermal_model(self):
        """
        Validate thermal model structure and constraints.

        Raises:
            ValueError: If thermal model is invalid
        """
        if not isinstance(self.thermal_model, list):
            raise ValueError("THERMAL_MODEL must be a list of dictionaries")

        if len(self.thermal_model) == 0:
            raise ValueError("THERMAL_MODEL cannot be empty (use None instead)")

        if len(self.thermal_model) > 3:
            raise ValueError("THERMAL_MODEL limited to 3 ranges (sufficient for pottery kilns)")

        for i, range_spec in enumerate(self.thermal_model):
            # Check required fields
            required_fields = ['temp_min', 'temp_max', 'kp', 'ki', 'kd']
            for field in required_fields:
                if field not in range_spec:
                    raise ValueError(f"Range {i} missing required field: {field}")

            # Validate temperature range
            if range_spec['temp_min'] >= range_spec['temp_max']:
                raise ValueError(f"Range {i}: temp_min must be < temp_max")

            # Validate gains are positive
            if range_spec['kp'] < 0 or range_spec['ki'] < 0 or range_spec['kd'] < 0:
                raise ValueError(f"Range {i}: PID gains must be non-negative")

    def get_gains(self, current_temp):
        """
        Get PID gains for current temperature.

        This method uses simple range switching without interpolation.
        When temperature crosses a range boundary, gains switch instantly.

        Range matching uses: temp_min <= current_temp < temp_max (inclusive lower bound)

        Args:
            current_temp: Current temperature (°C)

        Returns:
            Tuple of (kp, ki, kd)
        """
        # If no thermal model, always return defaults
        if self.thermal_model is None:
            self._update_current_gains(self.default_kp, self.default_ki, self.default_kd)
            return (self.default_kp, self.default_ki, self.default_kd)

        # Search for matching temperature range
        for range_spec in self.thermal_model:
            if range_spec['temp_min'] <= current_temp < range_spec['temp_max']:
                # Found matching range
                kp = range_spec['kp']
                ki = range_spec['ki']
                kd = range_spec['kd']
                self._update_current_gains(kp, ki, kd)
                return (kp, ki, kd)

        # No matching range - use defaults as fallback
        # This shouldn't happen if thermal model is properly configured,
        # but we handle it gracefully
        self._update_current_gains(self.default_kp, self.default_ki, self.default_kd)
        return (self.default_kp, self.default_ki, self.default_kd)

    def _update_current_gains(self, kp, ki, kd):
        """
        Update current gains and set change flag if they differ.

        Args:
            kp, ki, kd: New gains
        """
        # Check if gains changed (with small epsilon for float comparison)
        epsilon = 1e-6
        changed = (abs(kp - self._current_kp) > epsilon or
                  abs(ki - self._current_ki) > epsilon or
                  abs(kd - self._current_kd) > epsilon)

        if changed:
            self._gains_just_changed = True
            self._current_kp = kp
            self._current_ki = ki
            self._current_kd = kd
        else:
            self._gains_just_changed = False

    def gains_changed(self):
        """
        Check if gains changed on last get_gains() call.

        This allows the control loop to only update the PID controller
        when gains actually change, avoiding unnecessary work.

        Returns:
            True if gains changed on last get_gains() call
        """
        return self._gains_just_changed

    def __str__(self):
        """String representation"""
        if self.thermal_model is None:
            return f"PIDGainScheduler(default: Kp={self.default_kp}, Ki={self.default_ki}, Kd={self.default_kd})"
        else:
            return f"PIDGainScheduler({len(self.thermal_model)} ranges, current: Kp={self._current_kp}, Ki={self._current_ki}, Kd={self._current_kd})"

    def __repr__(self):
        return self.__str__()
