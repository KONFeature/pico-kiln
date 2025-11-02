# kiln/hardware.py
# Hardware abstraction layer for temperature sensor and SSR control

import time

class TemperatureSensor:
    """
    MAX31856 thermocouple wrapper with fault detection

    Provides a clean interface for temperature reading with error handling
    and fault detection.
    """

    def __init__(self, spi, cs_pin, thermocouple_type=None, offset=0.0, error_log=None):
        """
        Initialize temperature sensor

        Args:
            spi: SPI bus instance (wrapped for adafruit library)
            cs_pin: Chip select pin (wrapped for adafruit library)
            thermocouple_type: Type of thermocouple (default: K-type)
            offset: Temperature offset for calibration (°C)
            error_log: Optional ErrorLog instance for cross-core error logging
        """
        self.error_log = error_log
        try:
            import adafruit_max31856
            from adafruit_max31856 import ThermocoupleType

            # Default to K-type thermocouple
            if thermocouple_type is None:
                thermocouple_type = ThermocoupleType.K

            self.sensor = adafruit_max31856.MAX31856(
                spi, cs_pin, thermocouple_type=thermocouple_type
            )
            self.offset = offset
            self.last_good_temp = None  # No fake default - require first valid read
            self.initialized = False  # Track if we've ever had a valid reading
            self.fault_count = 0
            self.max_consecutive_faults = 10  # Emergency shutdown after this many consecutive faults

            # Perform initial conversion to clear power-up faults
            # The chip needs ~160ms to complete first conversion
            print("Temperature sensor initializing...")
            time.sleep(0.2)  # Wait for first conversion to complete (MAX31856 requirement)

            # Attempt initial read (don't block startup on retries - will retry during operation)
            try:
                temp = self.sensor.temperature
                if temp is not None and -50 <= temp <= 1500:
                    self.last_good_temp = temp + self.offset
                    self.initialized = True
                    print(f"Temperature sensor ready: {self.last_good_temp:.1f}°C")
                else:
                    print("Sensor init: Invalid first reading, will retry during operation")
            except Exception as e:
                print(f"Sensor init: First read failed ({e}), will retry during operation")

        except Exception as e:
            print(f"Error initializing temperature sensor: {e}")
            raise

    def read(self):
        """
        Read temperature with fault checking

        Returns:
            Temperature in Celsius

        Raises:
            Exception if persistent sensor fault detected or sensor not initialized
        """
        try:
            # Read temperature
            temp = self.sensor.temperature

            if temp is None:
                raise Exception("Sensor returned None")

            # Check for faults
            faults = self.sensor.fault
            if any(faults.values()):
                fault_list = [k for k, v in faults.items() if v]
                raise Exception(f"Thermocouple faults: {', '.join(fault_list)}")

            # Sanity check: temperature should be in reasonable range
            if temp < -50 or temp > 1500:
                raise Exception(f"Temperature {temp}°C out of reasonable range")

            # Apply calibration offset
            temp += self.offset

            # First successful read - mark as initialized
            if not self.initialized:
                print(f"✅ Temperature sensor initialized: {temp:.1f}°C")
                self.initialized = True

            # Success - reset fault counter immediately
            if self.fault_count > 0:
                print(f"Temperature sensor recovered (after {self.fault_count} faults)")
                self.fault_count = 0

            self.last_good_temp = temp
            return temp

        except Exception as e:
            self.fault_count += 1

            # SAFETY: If never initialized, don't allow heating - fail immediately
            if not self.initialized:
                error_msg = f"Temperature sensor failed to initialize: {e}"
                self._log_error(error_msg)
                raise Exception(error_msg)

            # Log error
            self._log_error(f"Temperature read error ({self.fault_count}/{self.max_consecutive_faults}): {e}")

            # Check if we've hit the consecutive fault limit
            if self.fault_count >= self.max_consecutive_faults:
                # Emergency shutdown - sensor is genuinely failing
                error_msg = f"EMERGENCY SHUTDOWN: {self.max_consecutive_faults} consecutive sensor failures: {e}"
                self._log_error(error_msg)
                raise Exception(error_msg)
            else:
                # Transient fault - return last good value and continue
                print(f"Using last good temperature: {self.last_good_temp:.1f}°C")
                return self.last_good_temp

    def get_last_temp(self):
        """Get last successfully read temperature"""
        return self.last_good_temp

    def reset_faults(self):
        """Reset fault counter"""
        self.fault_count = 0

    def _log_error(self, message):
        """Log error to both console and error log (if available)"""
        print(message)
        if self.error_log:
            self.error_log.log_error('TemperatureSensor', message)


class SSRController:
    """
    Time-proportional Solid State Relay controller

    Implements time-proportional control (slow PWM) for SSR control.
    This is appropriate for SSRs which should not be switched at high frequency.

    Supports multiple SSRs with staggered switching to prevent inrush current.

    Example: 100% duty = ON for full cycle
             50% duty  = ON for half cycle, OFF for half cycle
             0% duty   = OFF for full cycle
    """

    def __init__(self, pin, cycle_time=2.0, stagger_delay=0.01, error_log=None):
        """
        Initialize SSR controller

        Args:
            pin: GPIO pin(s) connected to SSR (machine.Pin instance or list of instances)
            cycle_time: Time-proportional cycle period in seconds (default: 2.0)
            stagger_delay: Delay between SSR state changes in seconds (default: 0.01)
                          Only applies when pin is a list (multiple SSRs)
            error_log: Optional ErrorLog instance for cross-core error logging
        """
        self.error_log = error_log

        # Convert single pin to list for uniform handling
        if isinstance(pin, list):
            self.pins = pin
        else:
            self.pins = [pin]

        self.cycle_time_ms = int(cycle_time * 1000)  # Store as milliseconds
        self.stagger_delay = stagger_delay
        self.duty_cycle = 0.0  # 0-100% (requested duty - may change mid-cycle)
        self.duty_cycle_locked = None  # Duty locked at cycle start (None = not locked yet)
        self.cycle_start = time.ticks_ms()  # Use ticks for efficiency

        # Track individual pin states
        self.pin_states = [False] * len(self.pins)

        # Ensure all SSRs start OFF
        for pin in self.pins:
            pin.value(0)

        if len(self.pins) > 1:
            print(f"SSR controller initialized with {len(self.pins)} SSRs (cycle time: {cycle_time}s, stagger: {stagger_delay}s)")
        else:
            print(f"SSR controller initialized (cycle time: {cycle_time}s)")

    def set_output(self, percent):
        """
        Set SSR output percentage

        Args:
            percent: Output percentage (0-100)
        """
        self.duty_cycle = max(0.0, min(100.0, percent))

    def update(self):
        """
        Update SSR state based on time-proportional control

        This should be called frequently (e.g., every 0.1s) to maintain
        accurate timing.

        The SSR is turned ON for duty_cycle% of the cycle_time,
        then OFF for the remainder.

        The duty cycle is LOCKED at the start of each cycle to prevent
        mid-cycle changes that cause relay flickering. New duty cycle
        values take effect on the next cycle.

        For multiple SSRs, state changes are staggered with delays to
        prevent large inrush current draw.
        """
        current_time = time.ticks_ms()
        elapsed = time.ticks_diff(current_time, self.cycle_start)

        # Lock duty on first call (initialize immediately so first cycle works)
        if self.duty_cycle_locked is None:
            self.duty_cycle_locked = self.duty_cycle

        # Check if we need to start a new cycle
        if elapsed >= self.cycle_time_ms:
            # Lock duty cycle for the new cycle (prevents mid-cycle changes)
            self.duty_cycle_locked = self.duty_cycle
            self.cycle_start = time.ticks_add(self.cycle_start, self.cycle_time_ms)
            elapsed = time.ticks_diff(current_time, self.cycle_start)

        # Calculate when SSR should be ON using LOCKED duty cycle
        on_time_ms = int((self.duty_cycle_locked / 100.0) * self.cycle_time_ms)

        # Determine desired state
        should_be_on = elapsed < on_time_ms
        current_state = any(self.pin_states)

        # Update SSR state based on simple time-proportional logic
        if should_be_on and not current_state:
            # Turn ON with staggered switching for multiple SSRs
            for i, pin in enumerate(self.pins):
                pin.value(1)
                self.pin_states[i] = True
                # Apply stagger delay between pins (except last)
                if i < len(self.pins) - 1 and self.stagger_delay > 0:
                    time.sleep(self.stagger_delay)

        elif not should_be_on and current_state:
            # Turn OFF with staggered switching for multiple SSRs
            for i, pin in enumerate(self.pins):
                pin.value(0)
                self.pin_states[i] = False
                # Apply stagger delay between pins (except last)
                if i < len(self.pins) - 1 and self.stagger_delay > 0:
                    time.sleep(self.stagger_delay)

    def force_off(self):
        """
        Force all SSRs off immediately (emergency stop)

        NOTE: No stagger delay is applied for safety reasons.
        All SSRs are turned off as quickly as possible.
        """
        self.duty_cycle = 0
        self.duty_cycle_locked = 0  # Reset locked duty too

        # Turn off all pins immediately (no stagger for safety)
        for i, pin in enumerate(self.pins):
            pin.value(0)
            self.pin_states[i] = False

        if len(self.pins) > 1:
            print(f"All {len(self.pins)} SSRs forced OFF")
        else:
            print("SSR forced OFF")

    def get_state(self):
        """
        Get current SSR state

        Returns:
            Dictionary with duty_cycle, duty_cycle_locked, is_on status, and individual pin states
        """
        return {
            'duty_cycle': self.duty_cycle,
            'duty_cycle_locked': self.duty_cycle_locked,  # What's actually being applied
            'is_on': any(self.pin_states),
            'pin_states': self.pin_states.copy() if len(self.pins) > 1 else None
        }

    def _log_error(self, message):
        """Log error to both console and error log (if available)"""
        print(message)
        if self.error_log:
            self.error_log.log_error('SSRController', message)

    def __del__(self):
        """Destructor - ensure all SSRs are turned off"""
        try:
            for pin in self.pins:
                pin.value(0)
        except Exception as e:
            self._log_error(f"Error in SSR destructor: {e}")
