# kiln/hardware.py
# Hardware abstraction layer for temperature sensor and SSR control

import time
from micropython import const

# Module-level constants for sensor fault detection and temperature ranges
MAX_CONSECUTIVE_FAULTS = const(20)      # Emergency shutdown after this many consecutive faults
COLD_START_FAULT_LIMIT = const(40)      # Higher tolerance during cold start (S-type noise at low mV)
COLD_START_TEMP_THRESHOLD = const(100)  # Below this, use COLD_START_FAULT_LIMIT instead
TEMP_MIN_RANGE = const(-50)             # Minimum reasonable temperature in °C
TEMP_MAX_RANGE = const(1500)            # Maximum reasonable temperature in °C
MIN_SSR_OUTPUT = 5.0                    # Floor when PID > 0% (5% × 20s cycle = 1s minimum relay pulse)

# Temperature-sensor signal conditioning defaults.
# White-noise rejection lives in the chip (AVGSEL hardware averaging + SINC/notch,
# run in continuous mode); software only median-filters to drop SSR/EMI spikes.
# Deliberately no EMA — a linear low-pass smears spikes and adds lag here.
DEFAULT_MAINS_FREQUENCY = const(60)     # Notch filter target (50 or 60 Hz mains)
DEFAULT_AVERAGING = const(8)            # MAX31856 hardware samples averaged (1,2,4,8,16)
DEFAULT_MEDIAN_WINDOW = const(3)        # Software median window for spike rejection (odd; 1=off)
VALID_AVERAGING = (1, 2, 4, 8, 16)

class TemperatureSensor:
    """
    MAX31856 thermocouple wrapper with fault detection

    Provides a clean interface for temperature reading with error handling
    and fault detection.
    """

    def __init__(self, spi, cs_pin, thermocouple_type=None, offset=0.0,
                 mains_frequency=DEFAULT_MAINS_FREQUENCY,
                 averaging=DEFAULT_AVERAGING,
                 median_window=DEFAULT_MEDIAN_WINDOW):
        """
        Initialize temperature sensor

        Configures the MAX31856's internal filtering, then runs it in continuous
        (auto) conversion mode so the chip free-runs its SINC + notch + averaging
        filter. Reads become non-blocking register fetches of the latest result.

        Args:
            spi: SPI bus instance (wrapped for adafruit library)
            cs_pin: Chip select pin (wrapped for adafruit library)
            thermocouple_type: Type of thermocouple (default: K-type)
            offset: Temperature offset for calibration (°C)
            mains_frequency: AC line frequency for the notch filter (50 or 60 Hz)
            averaging: MAX31856 hardware samples averaged per result (1,2,4,8,16)
            median_window: Software median window for spike rejection (odd; 1=off)
        """
        try:
            import adafruit_max31856
            from adafruit_max31856 import ThermocoupleType

            # Default to K-type thermocouple
            if thermocouple_type is None:
                thermocouple_type = ThermocoupleType.K

            if mains_frequency not in (50, 60):
                print(f"Sensor init: invalid mains_frequency={mains_frequency}, using {DEFAULT_MAINS_FREQUENCY}")
                mains_frequency = DEFAULT_MAINS_FREQUENCY
            if averaging not in VALID_AVERAGING:
                print(f"Sensor init: invalid averaging={averaging}, using {DEFAULT_AVERAGING}")
                averaging = DEFAULT_AVERAGING
            if median_window < 1:
                median_window = 1

            self.sensor = adafruit_max31856.MAX31856(
                spi, cs_pin, thermocouple_type=thermocouple_type
            )
            self.offset = offset
            self.last_good_temp = None  # No fake default - require first valid read
            self.initialized = False  # Track if we've ever had a valid reading
            self.fault_count = 0
            self.max_recorded_temp = 0.0  # Highest temp seen since boot (for cold-start detection)
            self.median_window = median_window
            self._samples = []

            # Configure chip filtering BEFORE auto-conversion. Datasheet: the notch
            # frequency and AVGSEL may only change in "Normally Off" mode (we are).
            self.sensor.noise_rejection = mains_frequency
            self.sensor.averaging = averaging
            self.sensor.start_autoconverting()

            print(f"Temperature sensor initializing (avg={averaging}, notch={mains_frequency}Hz)...")

            # In auto-conversion mode the temperature registers read exactly 0 until
            # the first conversion completes. Poll for the first real sample (adapts
            # to any averaging/notch); the timeout bounds boot if the sensor is stuck.
            try:
                start = time.ticks_ms()
                raw = self.sensor.unpack_temperature()
                while raw == 0.0 and time.ticks_diff(time.ticks_ms(), start) < 1500:
                    time.sleep_ms(20)
                    raw = self.sensor.unpack_temperature()

                temp = raw + self.offset
                if raw != 0.0 and TEMP_MIN_RANGE <= temp <= TEMP_MAX_RANGE:
                    self._samples.append(temp)
                    self.last_good_temp = temp
                    self.max_recorded_temp = temp
                    self.initialized = True
                    print(f"Temperature sensor ready: {temp:.1f}°C")
                else:
                    print("Sensor init: no valid first reading yet, will retry during operation")
            except Exception as e:
                print(f"Sensor init: First read failed ({e}), will retry during operation")

        except Exception as e:
            print(f"Error initializing temperature sensor: {e}")
            raise

    # Performance optimization: Called every control cycle (1 Hz) on SPI read path
    @micropython.native
    def read(self):
        """
        Read temperature with median spike-rejection and fault checking

        Returns:
            Temperature in Celsius (median-filtered)

        Raises:
            Exception if persistent sensor fault detected or sensor not initialized
        """
        try:
            faults = self.sensor.fault
            if any(faults.values()):
                fault_list = [k for k, v in faults.items() if v]
                raise Exception(f"Thermocouple faults: {', '.join(fault_list)}")

            # Latest continuous-conversion result: non-blocking register fetch, no
            # one-shot trigger or ~160ms wait. Already SINC/notch filtered + averaged.
            temp = self.sensor.unpack_temperature()

            if temp < TEMP_MIN_RANGE or temp > TEMP_MAX_RANGE:
                raise Exception(f"Temperature {temp}°C out of reasonable range")

            temp += self.offset

            samples = self._samples
            samples.append(temp)
            if len(samples) > self.median_window:
                samples.pop(0)
            filtered = self._median(samples)

            if filtered > self.max_recorded_temp:
                self.max_recorded_temp = filtered

            if not self.initialized:
                print(f"Temperature sensor initialized: {filtered:.1f}°C")
                self.initialized = True

            if self.fault_count > 0:
                print(f"Temperature sensor recovered (after {self.fault_count} faults)")
                self.fault_count = 0

            self.last_good_temp = filtered
            return filtered

        except Exception as e:
            self.fault_count += 1

            # After a sustained dropout, discard stale samples so the median
            # re-seeds from fresh readings instead of blending pre-fault values.
            if self.fault_count >= self.median_window:
                self._samples = []

            if not self.initialized:
                error_msg = f"Temperature sensor failed to initialize: {e}"
                print(error_msg)
                raise Exception(error_msg)

            # Use higher fault tolerance during cold start (S-type thermocouple has
            # very low output voltage below 100°C, making it prone to noise/faults)
            if self.max_recorded_temp < COLD_START_TEMP_THRESHOLD:
                fault_limit = COLD_START_FAULT_LIMIT
            else:
                fault_limit = MAX_CONSECUTIVE_FAULTS

            print(f"Temperature read error ({self.fault_count}/{fault_limit}): {e}")

            if self.fault_count >= fault_limit:
                error_msg = f"EMERGENCY SHUTDOWN: {self.fault_count} consecutive sensor failures: {e}"
                print(error_msg)
                raise Exception(error_msg)
            else:
                print(f"Using last good temperature: {self.last_good_temp:.1f}°C")
                return self.last_good_temp

    @staticmethod
    def _median(values):
        ordered = sorted(values)
        mid = len(ordered) // 2
        if len(ordered) % 2:
            return ordered[mid]
        return 0.5 * (ordered[mid - 1] + ordered[mid])

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

        self.cycle_time_ms = int(cycle_time * 1000)  # Store as milliseconds
        self.stagger_delay = stagger_delay
        self.duty_cycle = 0.0  # 0-100% (requested duty - may change mid-cycle)
        self.duty_cycle_locked = 0.0  # Duty locked at cycle start (initialize to 0 to avoid race)
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
            percent: Output percentage (0-100). Values > 0 are floored to MIN_SSR_OUTPUT
                     to ensure minimum relay on-time.
        """
        if percent > 0:
            self.duty_cycle = max(MIN_SSR_OUTPUT, min(100.0, percent))
        else:
            self.duty_cycle = 0.0

    # Performance optimization: CRITICAL - Called 10 times per second for time-proportional control
    @micropython.native
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

    def __del__(self):
        """Destructor - ensure all SSRs are turned off"""
        try:
            for pin in self.pins:
                pin.value(0)
        except Exception as e:
            print(f"Error in SSR destructor: {e}")
