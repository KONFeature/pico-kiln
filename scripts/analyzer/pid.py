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

    Ziegler-Nichols open-loop (reaction curve) PID: Kp = 1.2 T/(K L), Ti = 2L, Td = L/2.

    Fast response; typically ~25% overshoot on FOPDT plants with small L/T.
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

    Cohen-Coon PID (consistent PID set, r = L/T):
        Kp = (1/K)(T/L)(4/3 + r/4), Ti = L(32 + 6r)/(13 + 8r), Td = 4L/(11 + 2r)

    Designed for quarter-amplitude-decay disturbance rejection on FOPDT plants with
    sizeable dead time. For small L/T (like a kiln) it is very aggressive: Kp scales
    as T/(K L) and the loop has little phase margin.
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

    Kp = (1 / K) * (tau / theta) * (4.0 / 3.0 + ratio / 4.0)
    Ti = theta * (32 + 6 * ratio) / (13 + 8 * ratio)
    Td = theta * 4 / (11 + 2 * ratio)

    Ki = Kp / Ti if Ti > 0 else 0
    Kd = Kp * Td

    characteristics = (
        "Quarter-amplitude-decay design for disturbance rejection. Aggressive when L/T is "
        "small (high Kp, oscillatory, overshoot of 20%+ is typical); most sensible when L/T > 0.3."
    )

    return PIDParams(Kp, Ki, Kd, "Cohen-Coon", characteristics)


def calculate_amigo(model: ThermalModel) -> PIDParams:
    """
    AMIGO (Approximate M-constrained Integral Gain Optimization) tuning.

    Åström–Hägglund AMIGO PID for FOPDT:
        Kp = (1/K)(0.2 + 0.45 T/L), Ti = L(0.4L + 0.8T)/(L + 0.1T), Td = 0.5 L T/(0.3L + T)

    Robustness-oriented (maximum sensitivity Ms ~ 1.4); overshoot depends on the
    set-point weighting used and is not guaranteed to be small.
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
    Ti = L * (0.4 * L + 0.8 * T) / (L + 0.1 * T)
    Td = 0.5 * L * T / (0.3 * L + T)

    Ki = Kp / Ti if Ti > 0 else 0
    Kd = Kp * Td

    characteristics = (
        "Robust (Ms ~ 1.4) tuning: typically the most conservative of the three methods (lowest Kp for small L/T), with a "
        "well-damped response. Overshoot is not guaranteed; check on a real firing."
    )

    return PIDParams(Kp, Ki, Kd, "AMIGO", characteristics)


def calculate_all_pid_methods(model: ThermalModel) -> Dict[str, PIDParams]:
    """Calculate PID parameters using all methods."""
    return {
        'ziegler_nichols': calculate_ziegler_nichols(model),
        'cohen_coon': calculate_cohen_coon(model),
        'amigo': calculate_amigo(model)
    }
