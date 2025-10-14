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

    Example: 100% duty = ON for full cycle
             50% duty  = ON for half cycle, OFF for half cycle
             0% duty   = OFF for full cycle
    """

    def __init__(self, pin, cycle_time=2.0):
        """
        Initialize SSR controller

        Args:
            pin: GPIO pin connected to SSR (machine.Pin instance)
            cycle_time: Time-proportional cycle period in seconds (default: 2.0)
        """
        self.pin = pin
        self.cycle_time = cycle_time
        self.duty_cycle = 0.0  # 0-100%
        self.cycle_start = time.time()
        self.is_on = False

        # Ensure SSR starts OFF
        self.pin.value(0)
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
        """
        current_time = time.time()
        elapsed = current_time - self.cycle_start

        # Check if we need to start a new cycle
        if elapsed >= self.cycle_time:
            self.cycle_start = current_time
            elapsed = 0

        # Calculate when SSR should be ON
        on_time = (self.duty_cycle / 100.0) * self.cycle_time

        # Update SSR state
        if elapsed < on_time:
            if not self.is_on:
                self.pin.value(1)
                self.is_on = True
        else:
            if self.is_on:
                self.pin.value(0)
                self.is_on = False

    def force_off(self):
        """Force SSR off immediately (emergency stop)"""
        self.duty_cycle = 0
        self.pin.value(0)
        self.is_on = False
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
        """Destructor - ensure SSR is turned off"""
        try:
            self.pin.value(0)
        except:
            pass
