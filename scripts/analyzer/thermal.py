"""
Thermal Model Fitting

Identifies a first-order-plus-dead-time (FOPDT) model of the kiln from an
open-loop step in the tuning log:

    y(t) = y0 + K * du * (1 - exp(-(t - L) / tau))   for t > L,   y0 otherwise

K is in °C per % SSR, tau and L in seconds. K is solved in closed form (linear
least squares) for each (L, tau) on a grid; the grid minimum is refined locally.

Honesty rule: a kiln has a time constant of hours, so most tuning steps end long
before steady state. When tau is longer than the observed step (or the step was
still rising when it ended) K and tau are only constrained through their ratio,
so the identification is flagged LOW confidence instead of being accepted.
"""

import math
from typing import Dict, List, Optional, Tuple
from .data import Phase

# Firmware (THERMAL_H) accepts 0..0.01; larger values are rejected.
H_MAX = 0.01

# A step must rise at least this much to carry any information about the plant.
MIN_STEP_RISE_C = 3.0
MIN_STEP_DURATION_S = 120.0
STEADY_STATE_RATE_C_PER_MIN = 0.5  # same threshold detect_phases uses for plateaus

# Fit quality thresholds (RMSE as a fraction of the observed temperature rise)
GOOD_FIT_RMSE_FRACTION = 0.03

MAX_FIT_POINTS = 300  # decimation limit for the grid search (speed)
TAU_MAX_DURATION_MULTIPLE = 50.0  # grid upper bound for tau, relative to step duration


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
        self.gain_method: str = "fallback"  # Method used: fopdt_fit or fallback
        self.gain_vs_temp: List[Dict] = []  # List of {temp, gain, ssr} for gain scheduling
        self.heat_loss_method: str = "disabled"  # plateau, cooling, or disabled
        # Identification diagnostics
        self.fit_rmse: Optional[float] = None  # °C
        self.fit_step_duration_s: float = 0.0
        self.fit_step_ssr: float = 0.0
        self.steady_state_reached: bool = False
        self.identification_warnings: List[str] = []
        # Gain scheduling (heat-loss) status
        self.gain_scheduling_enabled: bool = False
        self.heat_loss_note: str = "gain scheduling disabled — insufficient data"


# =============================================================================
# FOPDT least-squares fit
# =============================================================================

def _fopdt_basis(rel_time: List[float], dead_time: float, tau: float) -> List[float]:
    return [1.0 - math.exp(-(t - dead_time) / tau) if t > dead_time else 0.0 for t in rel_time]


def _linear_fit(basis: List[float], y: List[float]) -> Tuple[float, float, float]:
    """Fit y = a + b*basis by least squares. Returns (a, b, sse); b is forced >= 0."""
    n = len(y)
    mean_g = sum(basis) / n
    mean_y = sum(y) / n
    c_gg = sum((g - mean_g) ** 2 for g in basis)
    c_gy = sum((g - mean_g) * (v - mean_y) for g, v in zip(basis, y))
    c_yy = sum((v - mean_y) ** 2 for v in y)
    if c_gg < 1e-12 or c_gy <= 0:
        return mean_y, 0.0, c_yy
    b = c_gy / c_gg
    a = mean_y - b * mean_g
    return a, b, max(c_yy - b * c_gy, 0.0)


def fit_fopdt_step(rel_time: List[float], temp: List[float], step_ssr: float) -> Dict:
    """
    Least-squares FOPDT fit of a fixed-SSR step response.

    Args:
        rel_time: Seconds since the step started
        temp: Temperature samples (°C)
        step_ssr: SSR level held during the step (%)

    Returns:
        Dict with gain (°C/% SSR), tau_s, dead_time_s, y0, rmse (°C, on all samples),
        tau_at_bound (True if tau hit the search limit, i.e. unidentifiable).
    """
    duration = rel_time[-1] - rel_time[0]
    stride = max(1, len(temp) // MAX_FIT_POINTS)
    t_s = rel_time[::stride]
    y_s = temp[::stride]

    tau_min = max(5.0, duration / 500.0)
    tau_max = TAU_MAX_DURATION_MULTIPLE * duration
    n_tau = 90
    ratio = (tau_max / tau_min) ** (1.0 / (n_tau - 1))
    taus = [tau_min * ratio ** i for i in range(n_tau)]

    l_max = min(0.25 * duration, 600.0)
    n_l = 25
    l_step = l_max / (n_l - 1)
    dead_times = [i * l_step for i in range(n_l)]

    best = (float('inf'), 0.0, taus[0])
    for dead_time in dead_times:
        for tau in taus:
            _, _, sse = _linear_fit(_fopdt_basis(t_s, dead_time, tau), y_s)
            if sse < best[0]:
                best = (sse, dead_time, tau)

    # Local refinement around the coarse optimum
    _, l0, tau0 = best
    for dead_time in [max(0.0, l0 + (i - 10) * l_step / 10.0) for i in range(21)]:
        for k in range(41):
            tau = tau0 * ratio ** ((k - 20) / 10.0)
            if tau < tau_min:
                continue
            _, _, sse = _linear_fit(_fopdt_basis(t_s, dead_time, tau), y_s)
            if sse < best[0]:
                best = (sse, dead_time, tau)

    _, dead_time, tau = best
    y0, b, sse = _linear_fit(_fopdt_basis(rel_time, dead_time, tau), temp)
    return {
        'gain': b / step_ssr if step_ssr > 0 else 0.0,
        'tau_s': tau,
        'dead_time_s': dead_time,
        'y0': y0,
        'rmse': math.sqrt(sse / len(temp)),
        'tau_at_bound': tau >= 0.9 * tau_max,
    }


def select_identification_step(data: Dict, phases: List[Phase]) -> Optional[Phase]:
    """
    Pick the best fixed-SSR step for open-loop identification.

    Candidates are heating/plateau phases with meaningful SSR, rise and duration. Steps that
    start near the coldest temperature of the log (plant at rest) are preferred, then the
    longest one (more of the response is observed).
    """
    time = data['time']
    temp = data['temp']
    coldest = min(temp)
    candidates = []
    for p in phases:
        if p.phase_type not in ('heating', 'plateau') or p.avg_ssr <= 20:
            continue
        duration = time[p.end_idx] - time[p.start_idx]
        rise = max(temp[p.start_idx:p.end_idx + 1]) - temp[p.start_idx]
        if duration < MIN_STEP_DURATION_S or rise < MIN_STEP_RISE_C:
            continue
        starts_at_rest = temp[p.start_idx] <= coldest + 10.0
        candidates.append((starts_at_rest, duration, p))
    if not candidates:
        return None
    return max(candidates, key=lambda c: (c[0], c[1]))[2]


def estimate_ambient(temp: List[float]) -> float:
    """Ambient estimate: coldest 5-sample moving mean (never above the first-10 mean)."""
    first = sum(temp[:10]) / len(temp[:10])
    if len(temp) < 5:
        return first
    coldest = min(sum(temp[i:i + 5]) / 5.0 for i in range(len(temp) - 4))
    return min(first, coldest)


def fit_thermal_model(data: Dict, phases: List[Phase]) -> ThermalModel:
    """
    Fit thermal model parameters (K, tau, L) with a least-squares FOPDT fit and
    the heat-loss coefficient h (only if the data supports it).

    Args:
        data: Dictionary with time, temp, ssr_output arrays
        phases: List of detected phases

    Returns:
        ThermalModel object with fitted parameters and confidence diagnostics
    """
    model = ThermalModel()
    time = data['time']
    temp = data['temp']

    model.ambient_temp = estimate_ambient(temp)

    step = select_identification_step(data, phases)
    if step is None:
        model.dead_time_s = 10.0
        model.time_constant_s = 120.0
        model.steady_state_gain = 0.5
        model.gain_method = "fallback"
        model.gain_confidence = "LOW"
        model.identification_warnings.append(
            "No usable heating step found: K, tau, L are DEFAULTS, not identified from data")
    else:
        idx = slice(step.start_idx, step.end_idx + 1)
        rel_time = [t - time[step.start_idx] for t in time[idx]]
        fit = fit_fopdt_step(rel_time, temp[idx], step.avg_ssr)

        model.dead_time_s = fit['dead_time_s']
        model.time_constant_s = fit['tau_s']
        model.steady_state_gain = fit['gain']
        model.fit_rmse = fit['rmse']
        model.fit_step_duration_s = rel_time[-1]
        model.fit_step_ssr = step.avg_ssr
        model.gain_method = "fopdt_fit"

        rise = max(temp[idx]) - temp[step.start_idx]
        tail_rate = step.tail_rate_per_min
        timed_out = tail_rate > STEADY_STATE_RATE_C_PER_MIN
        tau_exceeds_step = fit['tau_s'] > model.fit_step_duration_s
        model.steady_state_reached = (not timed_out) and (not tau_exceeds_step)

        confidence = "HIGH"
        if fit['rmse'] > GOOD_FIT_RMSE_FRACTION * rise:
            confidence = "MEDIUM"
            model.identification_warnings.append(
                f"Poor fit: RMSE {fit['rmse']:.2f}°C is {100 * fit['rmse'] / rise:.1f}% of the rise")
        if model.fit_step_duration_s < 3.0 * fit['tau_s']:
            confidence = "MEDIUM"
        if timed_out or tau_exceeds_step or fit['tau_at_bound']:
            confidence = "LOW"
            model.identification_warnings.append(
                f"Not at steady state: fitted tau={fit['tau_s']:.0f}s vs step duration "
                f"{model.fit_step_duration_s:.0f}s (still rising {tail_rate:.2f}°C/min at the end). "
                "K and tau are extrapolated and only their ratio K/tau is well constrained; "
                "run a longer fixed-SSR step (>= 3 tau) for a trustworthy gain.")
        if not (0.01 <= model.steady_state_gain <= 100.0):
            confidence = "LOW"
            model.identification_warnings.append(
                f"Gain K={model.steady_state_gain:.4g} °C/% is physically implausible")
        model.gain_confidence = confidence

    model.gain_vs_temp = calculate_effective_gains_by_temperature(data, phases, model.ambient_temp)
    _apply_heat_loss(model, data, phases)
    return model


def _apply_heat_loss(model: ThermalModel, data: Dict, phases: List[Phase]) -> None:
    """Set h from plateau gains (preferred) or cooling; otherwise disable gain scheduling."""
    h = None
    method = "disabled"
    # h is relative to K_base, so it is meaningless when K_base is an extrapolation.
    if model.gain_confidence == "LOW":
        model.identification_warnings.append(
            "Heat-loss fit skipped: base gain is LOW confidence")
    elif model.gain_vs_temp and model.steady_state_gain > 0:
        h = fit_heat_loss_coefficient(model.gain_vs_temp, model.steady_state_gain, model.ambient_temp)
        method = "plateau"
    if h is None and model.gain_confidence != "LOW":
        h = fit_heat_loss_from_cooling(data, phases, model.ambient_temp, model.time_constant_s)
        method = "cooling"

    if h is None:
        model.heat_loss_coefficient = 0.0
        model.heat_loss_method = "disabled"
        model.gain_scheduling_enabled = False
        model.heat_loss_note = "gain scheduling disabled — insufficient data"
        return

    if h > H_MAX:
        model.identification_warnings.append(
            f"Fitted h={h:.6f} exceeds the firmware limit {H_MAX}; clamped to {H_MAX}")
        h = H_MAX
    model.heat_loss_coefficient = round(h, 6)
    model.heat_loss_method = method
    model.gain_scheduling_enabled = h > 0
    model.heat_loss_note = f"h fitted from {method} data"


# =============================================================================
# Heat loss / gain scheduling
# =============================================================================

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

    time = data['time']
    plateau_phases = [p for p in phases if p.phase_type == 'plateau'
                     and p.avg_ssr > 20  # Meaningful heating
                     and (time[p.end_idx] - time[p.start_idx]) > 60]  # At least 1 minute

    for phase in plateau_phases:
        # Plateau classification uses the tail slope, so the equilibrium is the END temperature
        plateau_temp = phase.temp_end
        temp_above_ambient = plateau_temp - ambient_temp

        if phase.avg_ssr > 0 and temp_above_ambient > 0:
            effective_gain = temp_above_ambient / phase.avg_ssr

            if 0.01 <= effective_gain <= 10.0:
                gain_points.append({
                    'temp': round(plateau_temp, 1),
                    'gain': round(effective_gain, 4),
                    'ssr': round(phase.avg_ssr, 1)
                })

    gain_points.sort(key=lambda x: x['temp'])
    return gain_points


def fit_heat_loss_coefficient(gain_points: List[Dict], base_gain: float, ambient_temp: float) -> Optional[float]:
    """
    Fit heat loss coefficient from effective gain vs temperature data.

    Physics model: K_eff(T) = K_base / (1 + h*(T - T_ambient))
    The PID scaling formula is gain_scale(T) = 1 + h*(T - T_ambient).

    Needs at least 2 plateau points; the h estimate per point is
    h = (K_base/K_eff - 1) / ΔT, and the median is returned.

    Args:
        gain_points: List of {temp, gain, ssr} from plateau phases
        base_gain: Base steady-state gain K_base
        ambient_temp: Ambient temperature (°C)

    Returns:
        h (unclamped, may exceed H_MAX; the caller clamps and warns), or None when
        there is not enough data. No value is fabricated.
    """
    if not gain_points or len(gain_points) < 2:
        return None

    h_estimates = []
    for point in gain_points:
        delta_T = point['temp'] - ambient_temp
        if delta_T > 10:
            K_eff = point['gain']
            if K_eff > 0 and base_gain > 0:
                h = (base_gain / K_eff - 1.0) / delta_T
                if h > 0:  # gain should decrease with temperature
                    h_estimates.append(h)

    if not h_estimates:
        return None
    h_estimates.sort()
    return h_estimates[len(h_estimates) // 2]


def fit_heat_loss_from_cooling(data: Dict, phases: List[Phase], ambient_temp: float,
                               time_constant_s: Optional[float] = None) -> Optional[float]:
    """
    Derive h from cooling phases.

    Model consistent with K_eff(T) = K_base/(1 + h*ΔT): the loss law is
    dΔT/dt = -(ΔT + h*ΔT²)/tau0, so the observed exponential cooling rate k at
    mean excess temperature ΔT satisfies k = (1 + h*ΔT)/tau0, i.e.

        h = (k * tau0 - 1) / ΔT

    where tau0 is the fitted low-temperature time constant (no magic constant).
    Because it depends on tau0 it is only meaningful when tau0 was identified.

    Args:
        data: Dictionary with time, temp, ssr_output arrays
        phases: List of detected phases
        ambient_temp: Ambient temperature (°C)
        time_constant_s: Identified time constant tau0 (s)

    Returns:
        h (unclamped), or None when there is no usable cooling data / tau0.
    """
    if not time_constant_s or time_constant_s <= 0:
        return None

    time_array = data['time']
    temp_array = data['temp']

    cooling_phases = [p for p in phases if p.phase_type == 'cooling'
                     and (p.temp_start - p.temp_end) > 5
                     and p.temp_start > ambient_temp + 20]

    h_estimates = []
    for phase in cooling_phases:
        phase_time = time_array[phase.start_idx:phase.end_idx + 1]
        phase_temp = temp_array[phase.start_idx:phase.end_idx + 1]

        valid = [(t - phase_time[0], T - ambient_temp) for t, T in zip(phase_time, phase_temp)
                 if T - ambient_temp > 5.0]
        if len(valid) < 10:
            continue

        # Linearise: ln(ΔT) = a - k*t
        x_data = [v[0] for v in valid]
        y_data = [math.log(v[1]) for v in valid]
        n = len(x_data)
        mean_x = sum(x_data) / n
        mean_y = sum(y_data) / n
        var_x = sum((x - mean_x) ** 2 for x in x_data) / n
        if var_x < 1e-10:
            continue
        k = -sum((x - mean_x) * (y - mean_y) for x, y in zip(x_data, y_data)) / n / var_x
        if k <= 0:
            continue

        mean_delta = sum(v[1] for v in valid) / n
        if mean_delta <= 10:
            continue
        h = (k * time_constant_s - 1.0) / mean_delta
        if h > 0:
            h_estimates.append(h)

    if not h_estimates:
        return None
    h_estimates.sort()
    return h_estimates[len(h_estimates) // 2]
