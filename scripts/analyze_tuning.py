#!/usr/bin/env python3
"""
Advanced Kiln Tuning Data Analyzer

Analyzes temperature data from kiln tuning runs and calculates optimal PID parameters
using multiple methods including thermal modeling and phase detection.

Features:
- Multi-phase detection (heating, cooling, hold periods)
- Thermal model fitting (dead time, time constant, heat loss)
- Multiple PID calculation methods (Ziegler-Nichols, Cohen-Coon, AMIGO, Lambda)
- Temperature-range-specific PID parameters
- Comprehensive reporting and recommendations

Usage:
    python analyze_tuning.py <tuning_csv_file> [--method <name>]

Example:
    python analyze_tuning.py logs/tuning_2025-01-15_14-30-00.csv
    python analyze_tuning.py logs/tuning_2025-01-15_14-30-00.csv --method amigo
"""

import sys
import csv
import json
import math
from pathlib import Path
from datetime import datetime
from typing import List, Tuple, Dict, Optional


# =============================================================================
# Data Loading and Preprocessing
# =============================================================================

def load_tuning_data(csv_file: str) -> Dict:
    """
    Load comprehensive tuning data from CSV file.

    Args:
        csv_file: Path to CSV file with tuning data

    Returns:
        Dictionary with all data arrays: time, temp, ssr_output, timestamps,
        and optionally step_names, step_indices, total_steps if available.
        Also includes 'has_step_data' flag indicating if step columns exist.
    """
    time_data = []
    temp_data = []
    ssr_output_data = []
    timestamps = []
    step_names = []
    step_indices = []
    total_steps_data = []
    has_step_data = False

    with open(csv_file, 'r') as f:
        reader = csv.DictReader(f)

        # Check if step columns exist in the CSV
        fieldnames = reader.fieldnames or []
        has_step_columns = all(col in fieldnames for col in ['step_name', 'step_index', 'total_steps'])

        if has_step_columns:
            has_step_data = True

        for row in reader:
            time_data.append(float(row['elapsed_seconds']))
            temp_data.append(float(row['current_temp_c']))
            ssr_output_data.append(float(row['ssr_output_percent']))
            timestamps.append(row['timestamp'])

            # Load step data if available
            if has_step_columns:
                step_names.append(row['step_name'])
                step_indices.append(int(row['step_index']))
                total_steps_data.append(int(row['total_steps']))

    # Fallback: if all elapsed_seconds are 0, calculate from timestamps
    if all(t == 0.0 for t in time_data):
        print("\n⚠️  Warning: elapsed_seconds column is all zeros")
        print("Calculating elapsed time from timestamp column as fallback...")

        start_dt = datetime.strptime(timestamps[0], '%Y-%m-%d %H:%M:%S')
        time_data = []
        for ts in timestamps:
            dt = datetime.strptime(ts, '%Y-%m-%d %H:%M:%S')
            elapsed = (dt - start_dt).total_seconds()
            time_data.append(elapsed)

        print(f"✓ Rebuilt elapsed time: 0s to {time_data[-1]:.1f}s\n")

    result = {
        'time': time_data,
        'temp': temp_data,
        'ssr_output': ssr_output_data,
        'timestamps': timestamps,
        'has_step_data': has_step_data
    }

    # Add step data if available
    if has_step_data:
        result['step_names'] = step_names
        result['step_indices'] = step_indices
        result['total_steps'] = total_steps_data

    return result


# =============================================================================
# Phase Detection
# =============================================================================

class Phase:
    """Represents a detected phase in the tuning data."""
    def __init__(self, start_idx: int, end_idx: int, phase_type: str,
                 avg_ssr: float, temp_start: float, temp_end: float,
                 step_name: Optional[str] = None, step_index: Optional[int] = None):
        self.start_idx = start_idx
        self.end_idx = end_idx
        self.phase_type = phase_type  # 'heating', 'cooling', 'plateau'
        self.avg_ssr = avg_ssr
        self.temp_start = temp_start
        self.temp_end = temp_end
        self.step_name = step_name  # Name of the tuning step (e.g., "heat_60pct_to_100C")
        self.step_index = step_index  # Index of the tuning step (0, 1, 2, ...)

    def __repr__(self):
        step_info = f", step={self.step_name}" if self.step_name else ""
        return f"Phase({self.phase_type}, SSR={self.avg_ssr:.1f}%, {self.temp_start:.1f}->{self.temp_end:.1f}°C{step_info})"


def detect_phases(data: Dict, plateau_threshold: float = 0.5) -> List[Phase]:
    """
    Detect different test phases from the data.

    If step data is available (step_indices, step_names), uses explicit step boundaries
    for perfect phase detection. Otherwise, falls back to heuristic SSR-based detection.

    Args:
        data: Dictionary with time, temp, ssr_output arrays, and optionally step data
        plateau_threshold: Temperature change threshold (°C/min) for plateau detection

    Returns:
        List of Phase objects
    """
    phases = []
    time = data['time']
    temp = data['temp']
    ssr = data['ssr_output']

    if len(time) < 10:
        return phases

    # Check if step data is available for explicit phase detection
    if data.get('has_step_data', False):
        step_indices = data['step_indices']
        step_names = data['step_names']

        # Use explicit step transitions for phase boundaries
        current_step_idx = step_indices[0]
        phase_start = 0

        for i in range(1, len(step_indices)):
            # Detect step transition or end of data
            if step_indices[i] != current_step_idx or i == len(step_indices) - 1:
                if i == len(step_indices) - 1:
                    phase_end = i
                else:
                    phase_end = i - 1

                # Calculate phase characteristics
                phase_duration = time[phase_end] - time[phase_start]
                if phase_duration < 1:  # Skip very short phases
                    phase_start = i
                    current_step_idx = step_indices[i]
                    continue

                avg_ssr = sum(ssr[phase_start:phase_end+1]) / (phase_end - phase_start + 1)
                temp_start = temp[phase_start]
                temp_end = temp[phase_end]
                temp_change = temp_end - temp_start
                step_name = step_names[phase_start]
                step_index = step_indices[phase_start]

                # Classify phase type from step name
                step_name_lower = step_name.lower()
                if 'cool' in step_name_lower:
                    phase_type = 'cooling'
                elif 'hold' in step_name_lower or 'plateau' in step_name_lower:
                    phase_type = 'plateau'
                elif 'heat' in step_name_lower:
                    phase_type = 'heating'
                else:
                    # Fallback to rate-based classification
                    rate_per_min = (temp_change / phase_duration) * 60 if phase_duration > 0 else 0
                    if abs(rate_per_min) < plateau_threshold:
                        phase_type = 'plateau'
                    elif rate_per_min > plateau_threshold:
                        phase_type = 'heating'
                    else:
                        phase_type = 'cooling'

                phases.append(Phase(phase_start, phase_end, phase_type, avg_ssr,
                                  temp_start, temp_end, step_name, step_index))

                phase_start = i
                current_step_idx = step_indices[i]

    else:
        # Fallback to heuristic SSR-based phase detection (for old CSV files)
        current_ssr = ssr[0]
        phase_start = 0

        for i in range(1, len(ssr)):
            # Detect significant SSR change (>10%)
            if abs(ssr[i] - current_ssr) > 10 or i == len(ssr) - 1:
                if i == len(ssr) - 1:
                    i = len(ssr) - 1

                # Calculate phase characteristics
                phase_duration = time[i] - time[phase_start]
                if phase_duration < 10:  # Skip very short phases
                    phase_start = i
                    current_ssr = ssr[i]
                    continue

                avg_ssr = sum(ssr[phase_start:i]) / (i - phase_start)
                temp_start = temp[phase_start]
                temp_end = temp[i-1]
                temp_change = temp_end - temp_start

                # Calculate heating/cooling rate
                rate_per_min = (temp_change / phase_duration) * 60 if phase_duration > 0 else 0

                # Classify phase type
                if abs(rate_per_min) < plateau_threshold:
                    phase_type = 'plateau'
                elif rate_per_min > plateau_threshold:
                    phase_type = 'heating'
                else:
                    phase_type = 'cooling'

                phases.append(Phase(phase_start, i-1, phase_type, avg_ssr, temp_start, temp_end))

                phase_start = i
                current_ssr = ssr[i]

    return phases


# =============================================================================
# Thermal Model Fitting
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


# =============================================================================
# PID Calculation Methods
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


# =============================================================================
# Test Quality Assessment
# =============================================================================

def assess_test_quality(data: Dict, phases: List[Phase], model: ThermalModel) -> str:
    """
    Assess the quality of the tuning test data.

    Returns: 'EXCELLENT', 'GOOD', or 'POOR'
    """
    score = 0
    max_score = 6

    # Check 1: Sufficient data points
    if len(data['time']) > 500:
        score += 1
    elif len(data['time']) > 200:
        score += 0.5

    # Check 2: Temperature range covered
    temp_span = max(data['temp']) - min(data['temp'])
    if temp_span > 100:
        score += 1
    elif temp_span > 50:
        score += 0.5

    # Check 3: Multiple phases detected
    if len(phases) >= 3:
        score += 1
    elif len(phases) >= 2:
        score += 0.5

    # Check 4: Heating phases present
    heating_phases = [p for p in phases if p.phase_type == 'heating']
    if len(heating_phases) >= 2:
        score += 1
    elif len(heating_phases) >= 1:
        score += 0.5

    # Check 5: Reasonable thermal parameters
    if 5 <= model.dead_time_s <= 60 and 30 <= model.time_constant_s <= 600:
        score += 1
    elif 1 <= model.dead_time_s <= 120 and 10 <= model.time_constant_s <= 1200:
        score += 0.5

    # Check 6: Test duration
    duration = data['time'][-1] - data['time'][0]
    if duration > 1800:  # 30 minutes
        score += 1
    elif duration > 900:  # 15 minutes
        score += 0.5

    # Classify based on score
    percentage = (score / max_score) * 100

    if percentage >= 80:
        return 'EXCELLENT'
    elif percentage >= 50:
        return 'GOOD'
    else:
        return 'POOR'


def analyze_tuning_steps(data: Dict, phases: List[Phase]) -> Optional[List[Dict]]:
    """
    Analyze each tuning step individually to provide per-step metrics.

    Only runs if step data is available in the CSV.

    Args:
        data: Dictionary with tuning data including step information
        phases: List of detected phases (with step information)

    Returns:
        List of step analysis dictionaries, or None if step data unavailable
    """
    if not data.get('has_step_data', False):
        return None

    time = data['time']
    temp = data['temp']
    ssr = data['ssr_output']

    # Group phases by step index
    step_analyses = []
    steps_seen = {}

    for phase in phases:
        if phase.step_index is None:
            continue

        if phase.step_index not in steps_seen:
            # Calculate metrics for this step
            step_time = time[phase.start_idx:phase.end_idx+1]
            step_temp = temp[phase.start_idx:phase.end_idx+1]
            step_ssr = ssr[phase.start_idx:phase.end_idx+1]

            duration_s = step_time[-1] - step_time[0] if len(step_time) > 0 else 0
            duration_min = duration_s / 60

            temp_start = step_temp[0] if len(step_temp) > 0 else 0
            temp_end = step_temp[-1] if len(step_temp) > 0 else 0
            temp_change = temp_end - temp_start
            temp_stability = max(step_temp) - min(step_temp) if len(step_temp) > 0 else 0

            ssr_mean = sum(step_ssr) / len(step_ssr) if len(step_ssr) > 0 else 0
            # Calculate SSR standard deviation
            if len(step_ssr) > 1:
                ssr_variance = sum((x - ssr_mean) ** 2 for x in step_ssr) / len(step_ssr)
                ssr_std = math.sqrt(ssr_variance)
            else:
                ssr_std = 0

            step_analyses.append({
                'step_index': phase.step_index,
                'step_name': phase.step_name,
                'phase_type': phase.phase_type,
                'duration_s': round(duration_s, 1),
                'duration_min': round(duration_min, 1),
                'temp_start': round(temp_start, 1),
                'temp_end': round(temp_end, 1),
                'temp_change': round(temp_change, 1),
                'temp_stability': round(temp_stability, 2),
                'ssr_mean': round(ssr_mean, 1),
                'ssr_std': round(ssr_std, 2),
                'data_points': len(step_time)
            })

            steps_seen[phase.step_index] = True

    # Sort by step index
    step_analyses.sort(key=lambda x: x['step_index'])

    return step_analyses


# =============================================================================
# Output Generation
# =============================================================================

def generate_results_json(data: Dict, phases: List[Phase], model: ThermalModel,
                         pid_methods: Dict[str, PIDParams], range_pids: List[Dict],
                         test_quality: str, recommended_method: str) -> Dict:
    """Generate comprehensive results dictionary for JSON output."""

    return {
        'test_info': {
            'duration_s': round(data['time'][-1] - data['time'][0], 1),
            'data_points': len(data['time']),
            'temp_min': round(min(data['temp']), 1),
            'temp_max': round(max(data['temp']), 1),
            'phases_detected': len(phases)
        },
        'thermal_model': {
            'dead_time_s': round(model.dead_time_s, 2),
            'time_constant_s': round(model.time_constant_s, 1),
            'steady_state_gain': round(model.steady_state_gain, 4),
            'heat_loss_h1': round(model.heat_loss_h1, 6),
            'heat_loss_h2': round(model.heat_loss_h2, 9),
            'ambient_temp': round(model.ambient_temp, 1)
        },
        'pid_methods': {name: pid.to_dict() for name, pid in pid_methods.items()},
        'temperature_ranges': range_pids if range_pids else None,
        'recommended': recommended_method,
        'test_quality': test_quality
    }


def print_beautiful_report(data: Dict, phases: List[Phase], model: ThermalModel,
                          pid_methods: Dict[str, PIDParams], range_pids: List[Dict],
                          test_quality: str, recommended_method: str,
                          step_analyses: Optional[List[Dict]] = None):
    """Print a beautifully formatted analysis report."""

    # Header
    print("\n" + "=" * 80)
    print(" " * 25 + "KILN TUNING ANALYSIS REPORT")
    print("=" * 80)

    # Test Information
    print("\n┌─ TEST INFORMATION " + "─" * 60)
    print(f"│  Data Points:      {len(data['time']):,}")
    print(f"│  Duration:         {(data['time'][-1] - data['time'][0]) / 60:.1f} minutes")
    print(f"│  Temperature:      {min(data['temp']):.1f}°C → {max(data['temp']):.1f}°C (Δ{max(data['temp']) - min(data['temp']):.1f}°C)")
    print(f"│  Test Quality:     {test_quality}")
    print(f"│  Phases Detected:  {len(phases)}")
    for i, phase in enumerate(phases[:5], 1):  # Show first 5 phases
        print(f"│    {i}. {phase.phase_type.upper():8} - SSR: {phase.avg_ssr:5.1f}% | {phase.temp_start:6.1f}°C → {phase.temp_end:6.1f}°C")
    if len(phases) > 5:
        print(f"│    ... and {len(phases) - 5} more phases")
    print("└" + "─" * 79)

    # Thermal Model
    print("\n┌─ THERMAL MODEL PARAMETERS " + "─" * 52)
    print(f"│  Dead Time (L):        {model.dead_time_s:8.2f} seconds")
    print(f"│  Time Constant (τ):    {model.time_constant_s:8.1f} seconds ({model.time_constant_s/60:.1f} min)")
    print(f"│  L/τ Ratio:            {model.dead_time_s/model.time_constant_s if model.time_constant_s > 0 else 0:8.3f}")
    print(f"│  Steady-State Gain:    {model.steady_state_gain:8.4f} °C per % SSR (from {model.gain_method})")
    print(f"│  Gain Confidence:      {model.gain_confidence}")
    print(f"│  Heat Loss (linear):   {model.heat_loss_h1:8.6f}")
    print(f"│  Heat Loss (quad):     {model.heat_loss_h2:8.9f}")
    print(f"│  Ambient Temp:         {model.ambient_temp:8.1f}°C")
    print("└" + "─" * 79)

    # PID Methods
    print("\n┌─ PID CALCULATION METHODS " + "─" * 53)
    for name, pid in pid_methods.items():
        is_recommended = (name == recommended_method)
        marker = " ⭐ RECOMMENDED" if is_recommended else ""
        print(f"│")
        print(f"│  {pid.method.upper()}{marker}")
        print(f"│  ────────────────────────────────────────────────────────────────────────────")
        print(f"│    Kp: {pid.kp:8.3f}  |  Ki: {pid.ki:8.4f}  |  Kd: {pid.kd:8.3f}")
        print(f"│    {pid.characteristics}")
    print("└" + "─" * 79)

    # Temperature Range PIDs
    if range_pids:
        print("\n┌─ TEMPERATURE-RANGE-SPECIFIC PID " + "─" * 46)
        print("│  (Use these for better control across wide temperature ranges)")
        print("│")
        for rp in range_pids:
            print(f"│  {rp['name']:4} ({rp['range']:9}°C) - Kp:{rp['kp']:7.3f} Ki:{rp['ki']:7.4f} Kd:{rp['kd']:7.3f}  [{rp['samples']:4} samples]")
        print("└" + "─" * 79)

    # Per-Step Analysis
    if step_analyses:
        print("\n┌─ PER-STEP ANALYSIS " + "─" * 59)
        print("│  Detailed breakdown of each tuning step")
        print("│")
        for step in step_analyses:
            print(f"│  STEP {step['step_index']}: {step['step_name']}")
            print("│  " + "─" * 76)
            print(f"│    Duration:     {step['duration_min']:.1f} min ({step['duration_s']:.0f}s)")
            temp_sign = '+' if step['temp_change'] >= 0 else ''
            print(f"│    Temperature:  {step['temp_start']:.1f}°C → {step['temp_end']:.1f}°C (Δ{temp_sign}{step['temp_change']:.1f}°C)")
            print(f"│    Stability:    ±{step['temp_stability']:.2f}°C")
            print(f"│    SSR Output:   {step['ssr_mean']:.1f}% (±{step['ssr_std']:.2f}%)")
            print(f"│    Data Points:  {step['data_points']}")
            print("│")
        print("└" + "─" * 79)

    # Recommendations
    print("\n┌─ RECOMMENDATIONS " + "─" * 61)
    print("│")

    recommended_pid = pid_methods[recommended_method]
    print(f"│  RECOMMENDED METHOD: {recommended_pid.method.upper()}")
    print(f"│  ────────────────────────────────────────────────────────────────────────────")
    print(f"│    Kp = {recommended_pid.kp:.3f}")
    print(f"│    Ki = {recommended_pid.ki:.4f}")
    print(f"│    Kd = {recommended_pid.kd:.3f}")
    print("│")

    if test_quality == 'EXCELLENT':
        print("│  ✓ Test quality is EXCELLENT. High confidence in these parameters.")
    elif test_quality == 'GOOD':
        print("│  ✓ Test quality is GOOD. These parameters should work well.")
        print("│    For even better tuning, consider a longer test with more temp range.")
    else:
        print("│  ⚠ Test quality is POOR. Parameters may need manual adjustment.")
        print("│    Consider running a longer test with wider temperature range.")

    print("│")
    print("│  NEXT STEPS:")
    print("│  1. Update your config.py with the recommended values above")
    print("│  2. Restart the kiln controller")
    print("│  3. Test with a real firing profile and monitor for overshoot")
    print("│  4. Fine-tune if needed: reduce Kp/Ki for less overshoot, increase for faster response")
    print("└" + "─" * 79)

    print("\n" + "=" * 80)
    print()


# =============================================================================
# Main Entry Point
# =============================================================================

def select_recommended_method(model: ThermalModel, test_quality: str) -> str:
    """Select the best PID method based on system characteristics."""

    # Calculate L/T ratio
    ratio = model.dead_time_s / model.time_constant_s if model.time_constant_s > 0 else 0

    # Decision logic
    if test_quality == 'POOR':
        # Conservative choice for poor data
        return 'amigo'
    elif ratio > 0.3:
        # Significant dead time - Cohen-Coon is better
        return 'cohen_coon'
    else:
        # For kilns, minimal overshoot is usually desired
        # AMIGO is generally the best choice
        return 'amigo'


def main():
    """Main entry point."""
    print("\n" + "=" * 80)
    print(" " * 20 + "ADVANCED KILN TUNING ANALYZER")
    print("=" * 80)

    # Parse command line arguments
    if len(sys.argv) < 2:
        print("\nUsage: python analyze_tuning.py <tuning_csv_file> [--method <name>]")
        print("\nExample:")
        print("  python analyze_tuning.py logs/tuning_2025-01-15_14-30-00.csv")
        print("  python analyze_tuning.py logs/tuning_2025-01-15_14-30-00.csv --method amigo")
        print("\nAvailable methods: ziegler_nichols, cohen_coon, amigo, lambda")
        sys.exit(1)

    csv_file = sys.argv[1]
    filter_method = None

    # Check for --method flag
    if len(sys.argv) > 2 and sys.argv[2] == '--method':
        if len(sys.argv) > 3:
            filter_method = sys.argv[3].lower()
            valid_methods = ['ziegler_nichols', 'cohen_coon', 'amigo', 'lambda']
            if filter_method not in valid_methods:
                print(f"\n❌ Error: Unknown method '{filter_method}'")
                print(f"Valid methods: {', '.join(valid_methods)}")
                sys.exit(1)

    # Check if file exists
    if not Path(csv_file).exists():
        print(f"\n❌ Error: File not found: {csv_file}")
        sys.exit(1)

    print(f"\n📂 Loading data from: {csv_file}")

    try:
        # Load data
        data = load_tuning_data(csv_file)
        print(f"✓ Loaded {len(data['time']):,} data points")

        # Detect phases
        print("🔍 Detecting test phases...")
        phases = detect_phases(data)
        print(f"✓ Detected {len(phases)} phases")

        # Fit thermal model
        print("🔬 Fitting thermal model...")
        model = fit_thermal_model(data, phases)
        print(f"✓ Model fitted (L={model.dead_time_s:.1f}s, τ={model.time_constant_s:.1f}s)")

        # Calculate PID parameters
        print("🧮 Calculating PID parameters using multiple methods...")
        pid_methods = calculate_all_pid_methods(model)
        print(f"✓ Calculated {len(pid_methods)} PID parameter sets")

        # Calculate temperature-range-specific PIDs
        print("📊 Analyzing temperature-range-specific parameters...")
        range_pids = calculate_temperature_range_pids(data, phases)
        if range_pids:
            print(f"✓ Generated {len(range_pids)} temperature-range-specific PID sets")
        else:
            print("  (Temperature range too small for range-specific PIDs)")

        # Assess test quality
        test_quality = assess_test_quality(data, phases, model)
        print(f"✓ Test quality: {test_quality}")

        # Analyze per-step metrics (if step data available)
        step_analyses = None
        if data.get('has_step_data', False):
            print("📋 Analyzing per-step metrics...")
            step_analyses = analyze_tuning_steps(data, phases)
            if step_analyses:
                print(f"✓ Generated analysis for {len(step_analyses)} steps")
        else:
            print("  (No step data available - using heuristic phase detection)")

        # Select recommended method
        recommended_method = select_recommended_method(model, test_quality)

        # If filter specified, show only that method
        if filter_method:
            print(f"\n📌 Showing only: {filter_method.upper()}")
            filtered_methods = {filter_method: pid_methods[filter_method]}
            pid_methods = filtered_methods
            recommended_method = filter_method

        # Generate results
        results = generate_results_json(data, phases, model, pid_methods,
                                       range_pids, test_quality, recommended_method)

        # Save JSON
        output_file = "tuning_results.json"
        with open(output_file, 'w') as f:
            json.dump(results, f, indent=2)
        print(f"✓ Results saved to: {output_file}")

        # Print beautiful report
        print_beautiful_report(data, phases, model, pid_methods, range_pids,
                              test_quality, recommended_method, step_analyses)

        # Print hint about config snippet generator
        if range_pids:
            print("\n" + "=" * 80)
            print("📋 THERMAL MODEL CONFIG SNIPPET")
            print("=" * 80)
            print("To generate a ready-to-paste config snippet, run:")
            print("  python -c \"from analyze_tuning import generate_config_snippet; generate_config_snippet()\"")
            print("=" * 80)
            print()

    except Exception as e:
        print(f"\n❌ Error: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)


def generate_config_snippet():
    """
    Generate config.py snippet from tuning_results.json

    This helper function reads tuning_results.json (generated by main() above)
    and prints a ready-to-paste THERMAL_MODEL configuration for config.py.

    Usage:
        python analyze_tuning.py <csv_file>  # Generates tuning_results.json
        python -c "from analyze_tuning import generate_config_snippet; generate_config_snippet()"
    """
    import json
    from pathlib import Path

    results_file = "tuning_results.json"

    if not Path(results_file).exists():
        print(f"\n❌ Error: {results_file} not found")
        print("Run analyze_tuning.py first to generate tuning results:")
        print("  python analyze_tuning.py logs/tuning_YYYY-MM-DD_HH-MM-SS.csv")
        return

    # Load results
    with open(results_file, 'r') as f:
        results = json.load(f)

    temp_ranges = results.get('temperature_ranges')

    if not temp_ranges:
        print("\n⚠️  No temperature-range-specific PID parameters found in results")
        print("This may be because the tuning test didn't cover a wide enough temperature range.")
        print("\nYou can still use the recommended single PID values:")

        recommended = results.get('recommended')
        if recommended and recommended in results.get('pid_methods', {}):
            pid = results['pid_methods'][recommended]
            print(f"\nPID_KP = {pid['kp']:.3f}")
            print(f"PID_KI = {pid['ki']:.4f}")
            print(f"PID_KD = {pid['kd']:.3f}")
        return

    # Print header
    print("\n" + "=" * 80)
    print(" " * 20 + "THERMAL MODEL CONFIG SNIPPET")
    print("=" * 80)
    print("\nCopy the following into your config.py file:\n")
    print("-" * 80)

    # Generate THERMAL_MODEL snippet
    print("# Temperature-range-specific PID parameters")
    print(f"# Generated from: {results_file}")
    print(f"# Test quality: {results.get('test_quality', 'UNKNOWN')}")
    print("THERMAL_MODEL = [")

    for range_data in temp_ranges:
        # Extract range bounds
        range_str = range_data['range']
        min_temp, max_temp = range_str.split('-')

        print(f"    {{'temp_min': {min_temp}, 'temp_max': {max_temp}, "
              f"'kp': {range_data['kp']}, 'ki': {range_data['ki']}, 'kd': {range_data['kd']}}},  "
              f"# {range_data['name']} range")

    print("]")
    print("-" * 80)

    # Print usage instructions
    print("\n" + "=" * 80)
    print("USAGE INSTRUCTIONS:")
    print("=" * 80)
    print("1. Copy the THERMAL_MODEL definition above")
    print("2. Paste into config.py (replacing the existing THERMAL_MODEL = None)")
    print("3. Save config.py")
    print("4. Restart the kiln controller")
    print("5. The controller will now use different PID gains at different temperatures")
    print("\nBENEFITS:")
    print("- Better control across wide temperature ranges")
    print("- Compensates for changing kiln thermal dynamics")
    print("- Reduces overshoot and improves settling time")
    print("\nNOTE:")
    print("- Gains switch instantly when crossing range boundaries")
    print("- Controller maintains integral term continuity during switches")
    print("- Monitor web UI to see active gains during firing")
    print("=" * 80)
    print()


if __name__ == "__main__":
    main()
