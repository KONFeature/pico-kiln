"""
Thermal Model Fitting

This module handles thermal model parameter fitting from tuning data,
including dead time, time constant, steady-state gain, and heat loss coefficients.
"""

from typing import List, Dict
from .data import Phase


# =============================================================================
# Thermal Model
# =============================================================================

class ThermalModel:
    """Thermal characteristics of the kiln system."""
    def __init__(self):
        self.dead_time_s: float = 0
        self.time_constant_s: float = 0
        self.steady_state_gain: float = 0  # °C per % SSR
        self.heat_loss_h1: float = 0  # Linear heat loss coefficient
        self.heat_loss_h2: float = 0  # Quadratic heat loss coefficient
        self.ambient_temp: float = 25.0
        self.gain_confidence: str = "LOW"  # Confidence level: HIGH, MEDIUM, LOW
        self.gain_method: str = "fallback"  # Method used: plateau, heating, fallback


def fit_thermal_model(data: Dict, phases: List[Phase]) -> ThermalModel:
    """
    Fit thermal model parameters from tuning data.

    This function prioritizes plateau phases for steady-state gain calculation,
    as they represent true equilibrium conditions where heat input equals heat loss.

    Args:
        data: Dictionary with time, temp, ssr_output arrays
        phases: List of detected phases

    Returns:
        ThermalModel object with fitted parameters
    """
    model = ThermalModel()
    time = data['time']
    temp = data['temp']

    # Estimate ambient temperature from start
    model.ambient_temp = sum(temp[:min(10, len(temp))]) / min(10, len(temp))

    # Find heating phases for parameter extraction (dead time and time constant)
    heating_phases = [p for p in phases if p.phase_type == 'heating' and p.avg_ssr > 20]

    if heating_phases:
        # Use the first significant heating phase for dead time and time constant
        phase = heating_phases[0]
        phase_time = time[phase.start_idx:phase.end_idx+1]
        phase_temp = temp[phase.start_idx:phase.end_idx+1]

        # Calculate dead time (L) - time before temperature starts rising
        initial_temp = phase_temp[0]
        temp_threshold = initial_temp + 0.5  # 0.5°C rise threshold

        dead_time_idx = 0
        for i, t in enumerate(phase_temp):
            if t >= temp_threshold:
                dead_time_idx = i
                break

        model.dead_time_s = phase_time[dead_time_idx] - phase_time[0] if dead_time_idx > 0 else 5.0

        # Calculate time constant (τ) - time to reach 63% of final value
        temp_start = phase_temp[dead_time_idx] if dead_time_idx < len(phase_temp) else phase_temp[0]
        temp_final = phase_temp[-1]
        temp_change = temp_final - temp_start
        temp_63 = temp_start + 0.63 * temp_change

        tau_idx = dead_time_idx
        for i in range(dead_time_idx, len(phase_temp)):
            if phase_temp[i] >= temp_63:
                tau_idx = i
                break

        model.time_constant_s = phase_time[tau_idx] - phase_time[dead_time_idx] if tau_idx > dead_time_idx else 60.0
    else:
        # Default values if no suitable heating phase found
        model.dead_time_s = 10.0
        model.time_constant_s = 120.0

    # Calculate steady-state gain (K) - CRITICAL FIX: Use plateau phases preferentially
    # At plateau equilibrium: Gain = (T_plateau - T_ambient) / SSR_plateau
    plateau_phases = [p for p in phases if p.phase_type == 'plateau'
                     and p.avg_ssr > 20  # Meaningful heating
                     and (time[p.end_idx] - time[p.start_idx]) > 60]  # Sufficient duration

    if plateau_phases:
        # Use plateau phase - this gives the TRUE steady-state gain at equilibrium
        # Select the plateau with highest temperature for better accuracy
        best_plateau = max(plateau_phases, key=lambda p: p.temp_end)

        plateau_temp = best_plateau.temp_end
        temp_above_ambient = plateau_temp - model.ambient_temp
        model.steady_state_gain = temp_above_ambient / best_plateau.avg_ssr

        model.gain_method = "plateau"
        model.gain_confidence = "HIGH"

        # Validate gain is physically reasonable
        if not (0.01 <= model.steady_state_gain <= 10.0):
            model.gain_confidence = "MEDIUM"

    elif heating_phases:
        # Fallback: Estimate from heating phase with heat loss correction
        # This is less accurate because it's contaminated by transient behavior
        phase = heating_phases[0]
        phase_time = time[phase.start_idx:phase.end_idx+1]
        phase_temp = temp[phase.start_idx:phase.end_idx+1]

        # Estimate heat loss during heating
        avg_temp = (phase_temp[0] + phase_temp[-1]) / 2
        temp_above_ambient = avg_temp - model.ambient_temp

        # Rough estimate: assume 10% of heat input is lost to ambient
        # (This is still imperfect but better than ignoring heat loss entirely)
        loss_fraction = 0.1 * (temp_above_ambient / 100) if temp_above_ambient > 0 else 0.1

        temp_change = phase_temp[-1] - phase_temp[0]
        corrected_temp_change = temp_change / (1 - loss_fraction) if loss_fraction < 0.9 else temp_change

        if phase.avg_ssr > 0:
            model.steady_state_gain = corrected_temp_change / phase.avg_ssr
        else:
            model.steady_state_gain = 0.5

        model.gain_method = "heating"
        model.gain_confidence = "MEDIUM"

        # Validate gain is physically reasonable
        if not (0.01 <= model.steady_state_gain <= 10.0):
            model.gain_confidence = "LOW"

    else:
        # No suitable phases - use conservative default
        model.steady_state_gain = 0.5
        model.gain_method = "fallback"
        model.gain_confidence = "LOW"

    # Fit cooling curve for heat loss parameters
    cooling_phases = [p for p in phases if p.phase_type == 'cooling']

    if cooling_phases:
        # Use Newton cooling law: dT/dt = -h1*(T - T_amb) - h2*(T - T_amb)^2
        # Simplified estimation using first cooling phase
        phase = cooling_phases[0]
        if phase.end_idx > phase.start_idx + 10:
            phase_time = time[phase.start_idx:phase.end_idx+1]
            phase_temp = temp[phase.start_idx:phase.end_idx+1]

            # Calculate average cooling rate
            temp_diff_start = phase_temp[0] - model.ambient_temp
            temp_diff_end = phase_temp[-1] - model.ambient_temp
            time_span = phase_time[-1] - phase_time[0]

            if time_span > 0 and temp_diff_start > 0:
                avg_cooling_rate = (temp_diff_end - temp_diff_start) / time_span
                avg_temp_diff = (temp_diff_start + temp_diff_end) / 2

                # Simplified: assume linear dominates at lower temps
                model.heat_loss_h1 = abs(avg_cooling_rate) / avg_temp_diff if avg_temp_diff > 0 else 0.001
                model.heat_loss_h2 = model.heat_loss_h1 / (avg_temp_diff * 1000) if avg_temp_diff > 0 else 0.000001
    else:
        # Default heat loss values
        model.heat_loss_h1 = 0.001
        model.heat_loss_h2 = 0.000001

    return model
