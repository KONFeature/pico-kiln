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

def calculate_temperature_range_pids(model: ThermalModel, data: Dict) -> List[Dict]:
    """
    Calculate PID parameters for different temperature ranges using continuous gain scaling.

    This implementation uses a SINGLE thermal model (L, τ, K_base) fitted from full data,
    then adjusts PID gains based on temperature-dependent heat loss using a simple formula.

    Physics:
    - At higher temperatures, heat loss increases (radiation, convection)
    - This means you need more SSR power to maintain temperature
    - Therefore, the effective gain K_eff (°C per % SSR) DECREASES at high temps
    - We model this as: K_eff(T) = K_base / (1 + h*(T - T_ambient))
    - PID scaling: gain_scale(T) = 1 + h*(T - T_ambient)
    - Therefore: Kp(T) = Kp_base * gain_scale(T)

    Args:
        model: ThermalModel with heat_loss_coefficient
        data: Dictionary with tuning data (used to determine temperature span)

    Returns:
        List of dictionaries with range-specific PIDs in config format:
        {'temp_min': X, 'temp_max': Y, 'kp': Z, 'ki': W, 'kd': V}
        Note: These are for display - runtime should use continuous formula!
    """
    temp = data['temp']
    max_temp = max(temp)
    temp_span = max_temp - min(temp)

    # Only create ranges if we have significant temperature span
    if temp_span < 100:
        return []

    # If no heat loss coefficient, return uniform PID (no temperature dependence)
    if model.heat_loss_coefficient <= 0:
        return []

    # Define standard temperature ranges for pottery kilns
    # These are for display/documentation - runtime should use continuous formula
    ranges = []
    if max_temp > 700:
        ranges = [
            {'name': 'LOW', 'temp_min': 0, 'temp_max': 300},
            {'name': 'MID', 'temp_min': 300, 'temp_max': 700},
            {'name': 'HIGH', 'temp_min': 700, 'temp_max': 1300}
        ]
    elif max_temp > 300:
        ranges = [
            {'name': 'LOW', 'temp_min': 0, 'temp_max': 300},
            {'name': 'MID', 'temp_min': 300, 'temp_max': max(700, max_temp + 50)}
        ]
    else:
        return []

    # Calculate base PID using AMIGO (conservative, minimal overshoot)
    base_pid = calculate_amigo(model)

    range_pids = []
    for temp_range in ranges:
        # Calculate average temperature for this range
        avg_temp = (temp_range['temp_min'] + temp_range['temp_max']) / 2

        # Calculate gain scaling factor at this temperature
        # gain_scale = 1 + h*(T - T_ambient)
        delta_T = avg_temp - model.ambient_temp
        gain_scale = 1.0 + model.heat_loss_coefficient * delta_T

        # Scale PID parameters
        # When K_eff decreases (high temp), Kp must increase to compensate
        kp = base_pid.kp * gain_scale
        ki = base_pid.ki * gain_scale
        kd = base_pid.kd * gain_scale

        range_pids.append({
            'temp_min': temp_range['temp_min'],
            'temp_max': temp_range['temp_max'],
            'kp': round(kp, 3),
            'ki': round(ki, 4),
            'kd': round(kd, 3)
        })

    return range_pids
