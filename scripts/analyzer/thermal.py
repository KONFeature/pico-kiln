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
        self.steady_state_gain: float = 0  # Base gain K_base (°C per % SSR at low temp)
        self.heat_loss_coefficient: float = 0  # h in: gain_scale(T) = 1 + h*(T - T_ambient)
        self.ambient_temp: float = 25.0
        self.gain_confidence: str = "LOW"  # Confidence level: HIGH, MEDIUM, LOW
        self.gain_method: str = "fallback"  # Method used: plateau, heating, fallback
        self.gain_vs_temp: List[Dict] = []  # List of {temp, gain, ssr} for gain scheduling
        self.heat_loss_method: str = "fallback"  # plateau, cooling, or fallback


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
        # For continuous gain scaling, use LOWEST temperature plateau for base gain
        # This provides the most conservative (highest gain) baseline
        best_plateau = min(plateau_phases, key=lambda p: p.temp_end)

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

    # Calculate effective gains by temperature from plateau phases
    model.gain_vs_temp = calculate_effective_gains_by_temperature(data, phases, model.ambient_temp)

    # Fit heat loss coefficient - prefer plateau data, fallback to cooling data
    if model.gain_vs_temp:
        # Prefer plateau data (more accurate)
        model.heat_loss_coefficient = fit_heat_loss_coefficient(
            model.gain_vs_temp,
            model.steady_state_gain,
            model.ambient_temp
        )
        model.heat_loss_method = "plateau"
    else:
        # Fallback: use cooling phases
        h_cooling = fit_heat_loss_from_cooling(data, phases, model.ambient_temp)
        if h_cooling > 0:
            model.heat_loss_coefficient = h_cooling
            model.heat_loss_method = "cooling"
        else:
            model.heat_loss_coefficient = 0.0001
            model.heat_loss_method = "fallback"

    return model


def calculate_effective_gains_by_temperature(data: Dict, phases: List[Phase], ambient_temp: float) -> List[Dict]:
    """
    Calculate effective gain at different temperatures from plateau phases.

    At plateau equilibrium: SSR × K_eff(T) = (T - T_ambient)
    Therefore: K_eff(T) = (T - T_ambient) / SSR

    Args:
        data: Dictionary with time, temp, ssr_output arrays
        phases: List of detected phases
        ambient_temp: Ambient temperature (°C)

    Returns:
        List of dictionaries with {temp, gain, ssr} sorted by temperature
    """
    gain_points = []

    # Find all plateau phases with sufficient SSR and duration
    time = data['time']
    plateau_phases = [p for p in phases if p.phase_type == 'plateau'
                     and p.avg_ssr > 20  # Meaningful heating
                     and (time[p.end_idx] - time[p.start_idx]) > 60]  # At least 1 minute

    for phase in plateau_phases:
        # Use average temperature of the plateau
        plateau_temp = (phase.temp_start + phase.temp_end) / 2
        temp_above_ambient = plateau_temp - ambient_temp

        # Calculate effective gain at this temperature
        if phase.avg_ssr > 0 and temp_above_ambient > 0:
            effective_gain = temp_above_ambient / phase.avg_ssr

            # Validate gain is physically reasonable
            if 0.01 <= effective_gain <= 10.0:
                gain_points.append({
                    'temp': round(plateau_temp, 1),
                    'gain': round(effective_gain, 4),
                    'ssr': round(phase.avg_ssr, 1)
                })

    # Sort by temperature
    gain_points.sort(key=lambda x: x['temp'])

    return gain_points


def fit_heat_loss_coefficient(gain_points: List[Dict], base_gain: float, ambient_temp: float) -> float:
    """
    Fit heat loss coefficient from effective gain vs temperature data.

    Physics model: K_eff(T) = K_base / (1 + h*(T - T_ambient))
    This means higher heat loss at higher temps → lower effective gain → need higher Kp

    The PID scaling formula becomes: gain_scale(T) = 1 + h*(T - T_ambient)
    Where: Kp(T) = Kp_base * gain_scale(T)

    Args:
        gain_points: List of {temp, gain, ssr} from plateau phases
        base_gain: Base steady-state gain K_base
        ambient_temp: Ambient temperature (°C)

    Returns:
        Heat loss coefficient h (typical range: 0.0001 to 0.01 for kilns)
    """
    if not gain_points or len(gain_points) < 2:
        # Not enough data - use default small value
        # This means PID scaling will be minimal (nearly constant gains)
        return 0.0001

    # Use linear regression on: 1/K_eff = (1/K_base) * (1 + h*ΔT)
    # Rearranging: (K_base/K_eff - 1) = h*ΔT
    # So: h = (K_base/K_eff - 1) / ΔT

    # Calculate h from each pair of points
    h_estimates = []
    for point in gain_points:
        delta_T = point['temp'] - ambient_temp
        if delta_T > 10:  # Only use points significantly above ambient
            K_eff = point['gain']
            if K_eff > 0 and base_gain > 0:
                # h = (K_base/K_eff - 1) / ΔT
                h = (base_gain / K_eff - 1.0) / delta_T
                # Only accept positive h (gain should decrease with temp)
                if 0 < h < 0.1:  # Reasonable range for kilns
                    h_estimates.append(h)

    if h_estimates:
        # Use median to be robust to outliers
        h_estimates.sort()
        median_idx = len(h_estimates) // 2
        h = h_estimates[median_idx]
        return round(h, 6)
    else:
        # No valid estimates - use conservative default
        return 0.0001


def fit_heat_loss_from_cooling(data: Dict, phases: List[Phase], ambient_temp: float, verbose: bool = False) -> float:
    """
    Fit heat loss coefficient from cooling phases.

    Uses Newton's law of cooling: dT/dt = -k*(T - T_ambient)
    Solution: T(t) = T_ambient + (T_initial - T_ambient) * exp(-k*t)

    We linearize for fitting:
    ln(T - T_ambient) = ln(T_initial - T_ambient) - k*t

    Then convert cooling rate k to heat loss coefficient h.

    Args:
        data: Dictionary with time, temp, ssr_output arrays
        phases: List of detected phases
        ambient_temp: Ambient temperature (°C)
        verbose: If True, print diagnostic information

    Returns:
        Heat loss coefficient h (0.0001 to 0.01 for kilns), or 0 if fitting fails
    """
    time_array = data['time']
    temp_array = data['temp']

    # Find all cooling phases with sufficient temperature drop
    cooling_phases = [p for p in phases if p.phase_type == 'cooling'
                     and (p.temp_start - p.temp_end) > 5  # At least 5°C drop
                     and p.temp_start > ambient_temp + 20]  # Significantly above ambient

    if not cooling_phases:
        if verbose:
            print("\n⚠️  No suitable cooling phases found for heat loss fitting")
        return 0.0

    if verbose:
        print(f"\n📊 Fitting heat loss coefficient from {len(cooling_phases)} cooling phases:")

    h_estimates = []
    phase_num = 0

    for phase in cooling_phases:
        phase_num += 1
        # Extract phase data
        phase_time = time_array[phase.start_idx:phase.end_idx+1]
        phase_temp = temp_array[phase.start_idx:phase.end_idx+1]

        if len(phase_time) < 10:  # Need at least 10 points
            continue

        # Normalize time to start at 0
        t = [(ti - phase_time[0]) for ti in phase_time]

        # Calculate temperature above ambient
        temps_above_ambient = [T - ambient_temp for T in phase_temp]

        # Filter out points too close to ambient (avoid log issues)
        valid_indices = [i for i, T_delta in enumerate(temps_above_ambient) if T_delta > 5.0]

        if len(valid_indices) < 10:  # Need enough valid points
            continue

        # Prepare data for linear regression: ln(T - T_amb) vs t
        x_data = [t[i] for i in valid_indices]
        y_data = []
        for i in valid_indices:
            T_delta = temps_above_ambient[i]
            if T_delta > 0:
                # Use natural logarithm for linearization
                import math
                y_data.append(math.log(T_delta))
            else:
                continue

        if len(x_data) != len(y_data) or len(x_data) < 10:
            continue

        # Linear regression: y = a - k*x
        # Using least squares: k = -cov(x,y) / var(x)
        n = len(x_data)
        mean_x = sum(x_data) / n
        mean_y = sum(y_data) / n

        # Calculate covariance and variance
        cov_xy = sum((x_data[i] - mean_x) * (y_data[i] - mean_y) for i in range(n)) / n
        var_x = sum((x_data[i] - mean_x) ** 2 for i in range(n)) / n

        if var_x < 1e-10:  # Avoid division by zero
            continue

        # Slope is -k (negative because temp is decreasing)
        k = -cov_xy / var_x

        if k <= 0:  # k should be positive for cooling
            continue

        # Convert cooling rate constant k to heat loss coefficient h
        # Physics: k represents the exponential decay rate
        # h represents the linear gain scaling with temperature
        # Empirical relationship: h ≈ k / average_temp_delta
        # This accounts for the fact that cooling rate depends on temperature difference

        avg_temp = sum(phase_temp) / len(phase_temp)
        avg_temp_delta = avg_temp - ambient_temp

        if avg_temp_delta > 10:
            # Convert k to h: h represents relative change per degree
            # k is in units of 1/seconds
            # We want h such that gain_scale(T) = 1 + h*(T - T_ambient)
            # From physics: k ∝ heat_loss / thermal_mass
            # And heat_loss = h * area * (T - T_ambient)
            # So: h ≈ k / (T - T_ambient) * thermal_time_constant

            # Use a calibration factor based on typical kiln behavior
            # For a kiln with ~100s thermal time constant:
            thermal_time_constant = 100.0  # seconds (typical for small kilns)
            h = k * thermal_time_constant / avg_temp_delta

            # Validate h is in reasonable range
            if 0.0001 <= h <= 0.1:
                h_estimates.append(h)
                if verbose:
                    temp_drop = phase_temp[0] - phase_temp[-1]
                    duration = phase_time[-1] - phase_time[0]
                    print(f"  Phase {phase_num}: {phase_temp[0]:.1f}°C → {phase_temp[-1]:.1f}°C "
                          f"(Δ-{temp_drop:.1f}°C in {duration/60:.1f}min) → k={k:.6f}, h={h:.6f}")
            elif verbose:
                print(f"  Phase {phase_num}: Rejected (h={h:.6f} out of range)")
        elif verbose:
            print(f"  Phase {phase_num}: Skipped (temp too close to ambient)")

    if not h_estimates:
        if verbose:
            print("  ❌ No valid h estimates obtained from cooling phases")
        return 0.0

    # Use median for robustness against outliers
    h_estimates.sort()
    median_idx = len(h_estimates) // 2
    h = h_estimates[median_idx]

    if verbose:
        min_h = min(h_estimates)
        max_h = max(h_estimates)
        print(f"\n  Summary: {len(h_estimates)} valid estimates")
        print(f"    Range: {min_h:.6f} to {max_h:.6f}")
        print(f"    Median (selected): h = {h:.6f}")

        # Show what this means for gain scaling
        print(f"\n  Gain scaling implications:")
        for temp in [100, 300, 700, 1000]:
            gain_scale = 1.0 + h * (temp - ambient_temp)
            pct = (gain_scale - 1.0) * 100
            print(f"    At {temp:4}°C: gain scale = {gain_scale:.4f} (+{pct:.1f}%)")

    return round(h, 6)
