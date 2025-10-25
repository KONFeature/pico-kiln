# kiln/hardware.py
# Hardware abstraction layer for temperature sensor and SSR control

import time

class TemperatureSensor:
    """
    MAX31856 thermocouple wrapper with fault detection

    Provides a clean interface for temperature reading with error handling
    and fault detection.
    """

    def __init__(self, spi, cs_pin, thermocouple_type=None, offset=0.0):
        """
        Initialize temperature sensor

        Args:
            spi: SPI bus instance (wrapped for adafruit library)
            cs_pin: Chip select pin (wrapped for adafruit library)
            thermocouple_type: Type of thermocouple (default: K-type)
            offset: Temperature offset for calibration (°C)
        """
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
            self.last_good_temp = 20.0  # Reasonable default
            self.fault_count = 0
            self.max_fault_count = 3  # Allow some transient faults

            # Perform initial conversion to clear power-up faults
            # The chip needs ~160ms to complete first conversion
            print("Temperature sensor initialized, waiting for first conversion...")
            time.sleep(0.2)  # Wait for first conversion to complete

            # Read and discard first temperature to clear any power-up faults
            # Try a few times in case of transient power-up issues
            for attempt in range(3):
                try:
                    _ = self.sensor.temperature
                    print("Temperature sensor ready")
                    break
                except Exception as e:
                    if attempt < 2:
                        print(f"Temperature read attempt {attempt+1} failed: {e}, retrying...")
                        time.sleep(0.5)
                    else:
                        print(f"Temperature read failed after 3 attempts (may work later): {e}")

        except Exception as e:
            print(f"Error initializing temperature sensor: {e}")
            raise

    def read(self):
        """
        Read temperature with fault checking

        Returns:
            Temperature in Celsius

        Raises:
            Exception if persistent sensor fault detected
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

            # Apply calibration offset
            temp += self.offset

            # Sanity check: temperature should be in reasonable range
            if temp < -50 or temp > 1500:
                raise Exception(f"Temperature {temp}°C out of reasonable range")

            # Success - reset fault counter and save value
            self.fault_count = 0
            self.last_good_temp = temp
            return temp

        except Exception as e:
            self.fault_count += 1
            print(f"Temperature read error ({self.fault_count}/{self.max_fault_count}): {e}")

            if self.fault_count >= self.max_fault_count:
                # Persistent fault - raise error
                raise Exception(f"Persistent sensor fault: {e}")
            else:
                # Transient fault - return last good value
                print(f"Using last good temperature: {self.last_good_temp}°C")
                return self.last_good_temp

    def get_last_temp(self):
        """Get last successfully read temperature"""
        return self.last_good_temp

    def reset_faults(self):
        """Reset fault counter"""
        self.fault_count = 0


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

    def __init__(self, pin, cycle_time=2.0, stagger_delay=0.01):
        """
        Initialize SSR controller

        Args:
            pin: GPIO pin(s) connected to SSR (machine.Pin instance or list of instances)
            cycle_time: Time-proportional cycle period in seconds (default: 2.0)
            stagger_delay: Delay between SSR state changes in seconds (default: 0.01)
                          Only applies when pin is a list (multiple SSRs)
        """
        # Convert single pin to list for uniform handling
        if isinstance(pin, list):
            self.pins = pin
        else:
            self.pins = [pin]

        self.cycle_time = cycle_time
        self.stagger_delay = stagger_delay
        self.duty_cycle = 0.0  # 0-100%
        self.cycle_start = time.time()
        self.is_on = False  # True if ANY pin is on

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

        For multiple SSRs, state changes are staggered with delays to
        prevent large inrush current draw.
        """
        current_time = time.time()
        elapsed = current_time - self.cycle_start

        # Check if we need to start a new cycle
        if elapsed >= self.cycle_time:
            self.cycle_start = current_time
            elapsed = 0

        # Calculate when SSR should be ON
        on_time = (self.duty_cycle / 100.0) * self.cycle_time

        # Determine desired state (all pins should have same state in parallel config)
        desired_state = elapsed < on_time

        # Check if we need to change state
        if desired_state != self.is_on:
            # State transition needed - apply staggered switching
            try:
                for i, pin in enumerate(self.pins):
                    # Only change pin if it's not already in the desired state
                    # Read current pin state directly from hardware
                    current_state = bool(pin.value())
                    if current_state != desired_state:
                        pin.value(1 if desired_state else 0)

                        # Apply stagger delay between pins (except for last pin)
                        # Only delay if we actually changed a pin and have more pins to process
                        if i < len(self.pins) - 1 and len(self.pins) > 1 and self.stagger_delay > 0:
                            time.sleep(self.stagger_delay)

                self.is_on = desired_state
            except Exception as e:
                # Rollback: force all pins off on any error during state change
                print(f"Error during SSR state change: {e}")
                print("Emergency rollback: forcing all SSRs off")
                try:
                    for pin in self.pins:
                        pin.value(0)
                    self.is_on = False
                    self.duty_cycle = 0
                except:
                    pass  # Best effort cleanup
                raise  # Re-raise the original exception

    def force_off(self):
        """
        Force all SSRs off immediately (emergency stop)

        NOTE: No stagger delay is applied for safety reasons.
        All SSRs are turned off as quickly as possible.
        """
        self.duty_cycle = 0

        # Turn off all pins immediately (no stagger for safety)
        for pin in self.pins:
            pin.value(0)

        self.is_on = False

        if len(self.pins) > 1:
            print(f"All {len(self.pins)} SSRs forced OFF")
        else:
            print("SSR forced OFF")

    def get_state(self):
        """
        Get current SSR state

        Returns:
            Dictionary with duty_cycle and is_on status
        """
        return {
            'duty_cycle': self.duty_cycle,
            'is_on': self.is_on
        }

    def __del__(self):
        """Destructor - ensure all SSRs are turned off"""
        try:
            for pin in self.pins:
                pin.value(0)
        except:
            pass
