# kiln/tuner.py
# PID Auto-Tuning using Ziegler-Nichols Method
#
# This module implements PID parameter estimation using the Ziegler-Nichols
# open-loop step response method. It heats the kiln to a target temperature,
# then lets it cool, recording the temperature response curve.

import time
import json

class TuningStage:
    """Tuning stage constants"""
    HEATING = "heating"
    COOLING = "cooling"
    CALCULATING = "calculating"
    COMPLETE = "complete"
    ERROR = "error"

class ZieglerNicholsTuner:
    """
    Ziegler-Nichols PID tuner for kiln controller

    This implements the open-loop step response method:
    1. Heat kiln at maximum output to target temperature
    2. Turn off heating and let cool back to target
    3. Analyze the heating curve to extract system characteristics
    4. Calculate PID parameters using Z-N formulas

    References:
    - "Ziegler–Nichols Tuning Method" by Vishakha Vijay Patel
    - https://github.com/jbruce12000/kiln-controller/blob/master/kiln-tuner.py
    """

    def __init__(self, target_temp=200, max_time=1800, data_interval=1.0):
        """
        Initialize tuner

        Args:
            target_temp: Target temperature for tuning (°C)
            max_time: Maximum tuning time before timeout (seconds)
            data_interval: Time between data points (seconds)
        """
        self.target_temp = target_temp
        self.max_time = max_time
        self.data_interval = data_interval

        # Tuning state
        self.stage = TuningStage.HEATING
        self.start_time = None
        self.heating_complete_time = None

        # Data collection
        self.time_data = []
        self.temp_data = []

        # Calculated results
        self.results = None
        self.error_message = None

        # Tuning parameters for tangent line calculation
        self.tangent_divisor = 8.0  # Adjustable parameter for line fitting

    def start(self):
        """Start the tuning process"""
        self.start_time = time.time()
        self.stage = TuningStage.HEATING
        self.time_data = []
        self.temp_data = []
        self.results = None
        self.error_message = None
        print(f"[Tuner] Starting tuning sequence (target: {self.target_temp}°C)")

    def record_data(self, current_temp):
        """
        Record a temperature data point

        Args:
            current_temp: Current temperature reading (°C)
        """
        if self.start_time is None:
            return

        elapsed = time.time() - self.start_time
        self.time_data.append(elapsed)
        self.temp_data.append(current_temp)

    def update(self, current_temp):
        """
        Update tuning state and determine SSR output

        This should be called every control loop iteration during tuning.

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

        # Record data point
        self.record_data(current_temp)

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
                print(f"[Tuner] Heating: {current_temp:.1f}°C / {self.target_temp:.1f}°C")
                return 100, True  # Full power

        elif self.stage == TuningStage.COOLING:
            # Cool until temperature drops back to target
            if current_temp <= self.target_temp:
                print(f"[Tuner] Cooled back to target at {elapsed:.1f}s, calculating PID values")
                self.stage = TuningStage.CALCULATING
                return 0, True
            else:
                print(f"[Tuner] Cooling: {current_temp:.1f}°C / {self.target_temp:.1f}°C")
                return 0, True  # Keep SSR off

        elif self.stage == TuningStage.CALCULATING:
            # Calculate PID parameters
            try:
                self.results = self.calculate_pid_parameters()
                self.stage = TuningStage.COMPLETE
                print("[Tuner] Tuning complete!")
                print(f"[Tuner] Results: Kp={self.results['kp']:.2f}, Ki={self.results['ki']:.2f}, Kd={self.results['kd']:.2f}")
                return 0, False  # Tuning complete
            except Exception as e:
                self.stage = TuningStage.ERROR
                self.error_message = f"Calculation error: {e}"
                print(f"[Tuner] ERROR: {self.error_message}")
                return 0, False

        # ERROR or COMPLETE - should not reach here
        return 0, False

    def calculate_pid_parameters(self):
        """
        Calculate PID parameters using Ziegler-Nichols method

        Returns:
            Dictionary with PID parameters and analysis data

        Raises:
            Exception: If data is insufficient or calculation fails
        """
        if len(self.time_data) < 10:
            raise Exception("Insufficient data points for calculation")

        # Find min and max temperatures
        min_temp = min(self.temp_data)
        max_temp = max(self.temp_data)
        mid_temp = (max_temp + min_temp) / 2.0

        # Find points for tangent line using divisor method
        # This selects a linear region around the midpoint of the curve
        y_offset = (max_temp - min_temp) / self.tangent_divisor

        tangent_min_point = None
        tangent_max_point = None

        for i in range(len(self.temp_data)):
            temp = self.temp_data[i]

            if temp >= (mid_temp - y_offset) and tangent_min_point is None:
                tangent_min_point = (self.time_data[i], temp)
            elif temp >= (mid_temp + y_offset) and tangent_max_point is None:
                tangent_max_point = (self.time_data[i], temp)
                break

        if tangent_min_point is None or tangent_max_point is None:
            raise Exception("Could not find suitable points for tangent line")

        # Calculate tangent line: y = slope * x + offset
        slope = (tangent_max_point[1] - tangent_min_point[1]) / (tangent_max_point[0] - tangent_min_point[0])
        offset = tangent_min_point[1] - (slope * tangent_min_point[0])

        # Find where tangent line crosses min and max temperatures
        lower_crossing_time = (min_temp - offset) / slope
        upper_crossing_time = (max_temp - offset) / slope

        # Calculate Ziegler-Nichols parameters
        # L = dead time (delay before response starts)
        # T = time constant (time for response to complete)
        L = lower_crossing_time - self.time_data[0]
        T = upper_crossing_time - lower_crossing_time

        if L <= 0 or T <= 0:
            raise Exception(f"Invalid parameters: L={L:.2f}, T={T:.2f}")

        # Ziegler-Nichols PID tuning formulas (classic method)
        Kp = 1.2 * (T / L)
        Ti = 2.0 * L
        Td = 0.5 * L
        Ki = Kp / Ti
        Kd = Kp * Td

        # Build results dictionary
        results = {
            'kp': Kp,
            'ki': Ki,
            'kd': Kd,
            'L': L,
            'T': T,
            'min_temp': min_temp,
            'max_temp': max_temp,
            'target_temp': self.target_temp,
            'duration': time.time() - self.start_time,
            'data_points': len(self.time_data),
            'tangent_slope': slope,
            'tangent_offset': offset,
            'tangent_divisor': self.tangent_divisor,
            'timestamp': time.time()
        }

        return results

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
            'data_points': len(self.time_data),
            'error': self.error_message,
            'results': self.results
        }

        # Add current temperature data for graphing
        if len(self.temp_data) > 0:
            status['current_temp'] = round(self.temp_data[-1], 2)
            status['min_temp'] = round(min(self.temp_data), 2)
            status['max_temp'] = round(max(self.temp_data), 2)

        return status

    def get_data(self):
        """
        Get collected temperature/time data

        Returns:
            Dictionary with time and temperature arrays
        """
        return {
            'time': self.time_data,
            'temperature': self.temp_data
        }

    def save_results(self, filename="tuning_results.json"):
        """
        Save tuning results to JSON file

        Args:
            filename: Output filename
        """
        if self.results is None:
            raise Exception("No results to save")

        # Prepare data for saving
        save_data = {
            'results': self.results,
            'data': {
                'time': self.time_data,
                'temperature': self.temp_data
            }
        }

        try:
            with open(filename, 'w') as f:
                json.dump(save_data, f)
            print(f"[Tuner] Results saved to {filename}")
        except Exception as e:
            print(f"[Tuner] Error saving results: {e}")
            raise

    def save_csv(self, filename="tuning_data.csv"):
        """
        Save temperature data to CSV file

        Args:
            filename: Output filename
        """
        try:
            with open(filename, 'w') as f:
                f.write("time,temperature\n")
                for t, temp in zip(self.time_data, self.temp_data):
                    f.write(f"{t},{temp}\n")
            print(f"[Tuner] Data saved to {filename}")
        except Exception as e:
            print(f"[Tuner] Error saving CSV: {e}")
            raise

    @staticmethod
    def load_results(filename="tuning_results.json"):
        """
        Load tuning results from JSON file

        Args:
            filename: Input filename

        Returns:
            Dictionary with results and data
        """
        try:
            with open(filename, 'r') as f:
                return json.load(f)
        except Exception as e:
            print(f"[Tuner] Error loading results: {e}")
            return None
