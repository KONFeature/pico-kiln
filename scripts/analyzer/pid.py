"""
PID Parameter Calculation

This module provides various PID tuning methods including Ziegler-Nichols,
Cohen-Coon, AMIGO, and Lambda tuning, as well as temperature-range-specific
PID parameter calculation.
"""

from typing import Dict, List
from .thermal import ThermalModel
from .data import Phase, detect_phases


# =============================================================================
# PID Parameters
# =============================================================================

class PIDParams:
    """PID parameter set with metadata."""
    def __init__(self, kp: float, ki: float, kd: float, method: str, characteristics: str):
        self.kp = kp
        self.ki = ki
        self.kd = kd
        self.method = method
        self.characteristics = characteristics

    def to_dict(self):
        return {
            'kp': round(self.kp, 3),
            'ki': round(self.ki, 4),
            'kd': round(self.kd, 3),
            'characteristics': self.characteristics
        }


# =============================================================================
# PID Calculation Methods
# =============================================================================

def calculate_ziegler_nichols(model: ThermalModel) -> PIDParams:
    """
    Ziegler-Nichols Classic PID tuning.

    Fast response, moderate overshoot (~25%), good for general purpose.
    """
    L = model.dead_time_s
    T = model.time_constant_s

    # Prevent division by zero
    if L < 1:
        L = 1
    if T < 1:
        T = 1

    Kp = 1.2 * (T / L)
    Ti = 2.0 * L
    Td = 0.5 * L
    Ki = Kp / Ti if Ti > 0 else 0
    Kd = Kp * Td

    characteristics = (
        "Fast response with moderate overshoot (~25%). "
        "Good general-purpose tuning. May oscillate if system is noisy."
    )

    return PIDParams(Kp, Ki, Kd, "Ziegler-Nichols", characteristics)


def calculate_cohen_coon(model: ThermalModel) -> PIDParams:
    """
    Cohen-Coon PID tuning.

    Better for systems with significant dead time (L/T > 0.3).
    """
    L = model.dead_time_s
    T = model.time_constant_s
    K = model.steady_state_gain if model.steady_state_gain > 0 else 1.0

    if L < 1:
        L = 1
    if T < 1:
        T = 1

    # Cohen-Coon formulas
    tau = T
    theta = L
    ratio = theta / tau

    Kp = (1 / K) * (tau / theta) * (1.0 + theta / (12 * tau))
    Ti = theta * (30 + 3 * ratio) / (9 + 20 * ratio)
    Td = theta * 4 / (11 + 2 * ratio)

    Ki = Kp / Ti if Ti > 0 else 0
    Kd = Kp * Td

    characteristics = (
        "Optimized for systems with significant dead time (L/T > 0.3). "
        "Faster response than Z-N with similar overshoot. Better disturbance rejection."
    )

    return PIDParams(Kp, Ki, Kd, "Cohen-Coon", characteristics)


def calculate_amigo(model: ThermalModel) -> PIDParams:
    """
    AMIGO (Approximate M-constrained Integral Gain Optimization) tuning.

    Very conservative, minimal overshoot (<5%), smooth response.
    """
    L = model.dead_time_s
    T = model.time_constant_s
    K = model.steady_state_gain if model.steady_state_gain > 0 else 1.0

    if L < 1:
        L = 1
    if T < 1:
        T = 1

    # AMIGO formulas
    Kp = (0.2 + 0.45 * (T / L)) / K if K > 0 else 0.45 * (T / L)
    Ti = (0.4 * L + 0.8 * T) * (L + 0.1 * T) / (L + 0.3 * T) if L + 0.3 * T > 0 else L
    Td = 0.5 * L * T / (0.3 * L + T) if 0.3 * L + T > 0 else 0.5 * L

    Ki = Kp / Ti if Ti > 0 else 0
    Kd = Kp * Td

    characteristics = (
        "Very conservative tuning with minimal overshoot (<5%). "
        "Smooth, stable response. Excellent for preventing temperature overshoot in kilns."
    )

    return PIDParams(Kp, Ki, Kd, "AMIGO", characteristics)


def calculate_lambda(model: ThermalModel, lambda_factor: float = 1.5) -> PIDParams:
    """
    Lambda Tuning (IMC-based).

    User specifies desired closed-loop time constant as multiple of system time constant.

    Args:
        lambda_factor: Closed-loop time constant = lambda_factor * system_time_constant
                      Lower values = faster response, higher = more conservative
    """
    L = model.dead_time_s
    T = model.time_constant_s
    K = model.steady_state_gain if model.steady_state_gain > 0 else 1.0

    if L < 1:
        L = 1
    if T < 1:
        T = 1

    # Lambda tuning formulas
    lambda_cl = lambda_factor * T  # Closed-loop time constant

    Kp = T / (K * (lambda_cl + L)) if K > 0 and (lambda_cl + L) > 0 else 1.0
    Ti = T
    Td = 0

    Ki = Kp / Ti if Ti > 0 else 0
    Kd = 0  # Lambda tuning typically uses PI control

    characteristics = (
        f"Lambda tuning with λ={lambda_factor}x system time constant. "
        "Predictable response based on desired closed-loop speed. "
        "No derivative action (PI control)."
    )

    return PIDParams(Kp, Ki, Kd, "Lambda", characteristics)


def calculate_all_pid_methods(model: ThermalModel) -> Dict[str, PIDParams]:
    """Calculate PID parameters using all methods."""
    return {
        'ziegler_nichols': calculate_ziegler_nichols(model),
        'cohen_coon': calculate_cohen_coon(model),
        'amigo': calculate_amigo(model),
        'lambda': calculate_lambda(model, lambda_factor=1.5)
    }


# =============================================================================
# Temperature-Range-Specific PID
# =============================================================================

def calculate_temperature_range_pids(data: Dict, phases: List[Phase],
                                    min_range_size: float = 50) -> List[Dict]:
    """
    Calculate PID parameters for different temperature ranges.

    Args:
        data: Dictionary with tuning data
        phases: List of detected phases
        min_range_size: Minimum temperature range size (°C)

    Returns:
        List of dictionaries with range-specific PIDs
    """
    # Import fit_thermal_model here to avoid circular import
    from .thermal import fit_thermal_model

    temp = data['temp']
    min_temp = min(temp)
    max_temp = max(temp)
    temp_span = max_temp - min_temp

    # Only create ranges if we have significant temperature span
    if temp_span < 100:
        return []

    # Define temperature ranges
    ranges = []
    if max_temp > 700:
        ranges = [
            {'name': 'LOW', 'min': 0, 'max': 300},
            {'name': 'MID', 'min': 300, 'max': 700},
            {'name': 'HIGH', 'min': 700, 'max': 1300}
        ]
    elif max_temp > 300:
        ranges = [
            {'name': 'LOW', 'min': 0, 'max': 300},
            {'name': 'MID', 'min': 300, 'max': max(700, max_temp + 50)}
        ]
    else:
        return []

    range_pids = []

    for temp_range in ranges:
        # Filter data for this temperature range
        range_indices = [i for i, t in enumerate(temp)
                        if temp_range['min'] <= t <= temp_range['max']]

        if len(range_indices) < 10:
            continue

        # Create filtered dataset for this range
        range_data = {
            'time': [data['time'][i] for i in range_indices],
            'temp': [data['temp'][i] for i in range_indices],
            'ssr_output': [data['ssr_output'][i] for i in range_indices],
            'timestamps': [data['timestamps'][i] for i in range_indices]
        }

        # Detect phases in this range
        range_phases = detect_phases(range_data)

        # Fit thermal model for this range
        range_model = fit_thermal_model(range_data, range_phases)

        # Calculate AMIGO parameters (conservative choice for range-specific)
        pid = calculate_amigo(range_model)

        range_pids.append({
            'range': f"{temp_range['min']}-{temp_range['max']}",
            'name': temp_range['name'],
            'kp': round(pid.kp, 3),
            'ki': round(pid.ki, 4),
            'kd': round(pid.kd, 3),
            'samples': len(range_indices)
        })

    return range_pids
