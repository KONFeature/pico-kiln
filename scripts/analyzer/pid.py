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
    K = model.steady_state_gain if model.steady_state_gain > 0 else 1.0

    # Prevent division by zero
    if L < 1:
        L = 1
    if T < 1:
        T = 1

    Kp = 1.2 * T / (K * L)
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
    Ti = (0.4 * L + 0.8 * T) * (L + 0.3 * T) / (L + 0.1 * T) if L + 0.1 * T > 0 else L
    Td = 0.5 * L * T / (0.3 * L + T) if 0.3 * L + T > 0 else 0.5 * L

    Ki = Kp / Ti if Ti > 0 else 0
    Kd = Kp * Td

    characteristics = (
        "Very conservative tuning with minimal overshoot (<5%). "
        "Smooth, stable response. Excellent for preventing temperature overshoot in kilns."
    )

    return PIDParams(Kp, Ki, Kd, "AMIGO", characteristics)


def calculate_all_pid_methods(model: ThermalModel) -> Dict[str, PIDParams]:
    """Calculate PID parameters using all methods."""
    return {
        'ziegler_nichols': calculate_ziegler_nichols(model),
        'cohen_coon': calculate_cohen_coon(model),
        'amigo': calculate_amigo(model)
    }
