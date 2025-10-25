#!/usr/bin/env python3
"""
PID Performance Analyzer for Kiln Controller

Analyzes closed-loop PID controller performance from kiln profile runs.
Detects issues like overshoot, oscillation, poor tracking, and provides
actionable recommendations for PID tuning improvements.

Features:
- 6 key performance metrics (overshoot, settling time, steady-state error,
  oscillation, tracking lag, control effort)
- Per-segment analysis (ramps vs holds)
- Performance grading system
- Actionable recommendations with specific fixes
- Beautiful terminal report + JSON output

Usage:
    python analyze_pid_performance.py <profile_csv_file>
    python analyze_pid_performance.py <profile_csv_file> --json-only
    python analyze_pid_performance.py <profile_csv_file> --verbose

Example:
    python analyze_pid_performance.py logs/cone6_glaze_2025-10-25_14-23-45.csv
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

def load_profile_data(csv_file: str) -> Dict:
    """
    Load profile run data from CSV file.

    Args:
        csv_file: Path to CSV file with profile run data

    Returns:
        Dictionary with all data arrays: time, temp, target, ssr_output,
        timestamps, and optionally step_names, step_indices if available.
    """
    time_data = []
    temp_data = []
    target_data = []
    ssr_output_data = []
    timestamps = []
    step_names = []
    step_indices = []
    has_step_data = False

    with open(csv_file, 'r') as f:
        reader = csv.DictReader(f)

        # Check if step columns exist in the CSV
        fieldnames = reader.fieldnames or []
        has_step_columns = 'step_index' in fieldnames

        if has_step_columns:
            has_step_data = True

        for row in reader:
            # Skip non-RUNNING states (we only analyze active PID control)
            if 'state' in row and row['state'] != 'RUNNING':
                continue

            time_data.append(float(row['elapsed_seconds']))
            temp_data.append(float(row['current_temp_c']))
            target_data.append(float(row['target_temp_c']))
            ssr_output_data.append(float(row['ssr_output_percent']))
            timestamps.append(row['timestamp'])

            # Load step data if available
            if has_step_columns:
                step_indices.append(int(row['step_index']) if row['step_index'] else 0)
                step_names.append(row.get('step_name', ''))

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
        'target': target_data,
        'ssr_output': ssr_output_data,
        'timestamps': timestamps,
        'has_step_data': has_step_data
    }

    # Add step data if available
    if has_step_data:
        result['step_names'] = step_names
        result['step_indices'] = step_indices

    return result


# =============================================================================
# Segment Detection
# =============================================================================

class Segment:
    """Represents a detected segment in the profile run."""
    def __init__(self, start_idx: int, end_idx: int, segment_type: str,
                 target_start: float, target_end: float,
                 step_name: Optional[str] = None, step_index: Optional[int] = None):
        self.start_idx = start_idx
        self.end_idx = end_idx
        self.segment_type = segment_type  # 'ramp', 'hold', 'cooling'
        self.target_start = target_start
        self.target_end = target_end
        self.step_name = step_name
        self.step_index = step_index

        # Performance metrics (filled in later)
        self.metrics = {}
        self.grade = "UNKNOWN"
        self.issues = []

    def __repr__(self):
        step_info = f", step={self.step_name}" if self.step_name else ""
        return f"Segment({self.segment_type}, {self.target_start:.1f}->{self.target_end:.1f}°C{step_info})"


def detect_segments(data: Dict, hold_threshold: float = 0.5) -> List[Segment]:
    """
    Detect profile segments (ramps, holds, cooling).

    Uses step_index if available for precise boundaries, otherwise detects
    from target temperature changes.

    Args:
        data: Dictionary with profile data
        hold_threshold: Temperature change threshold (°C/min) for hold detection

    Returns:
        List of Segment objects
    """
    segments = []
    time = data['time']
    target = data['target']

    if len(time) < 10:
        return segments

    # Use explicit step data if available
    if data.get('has_step_data', False):
        step_indices = data['step_indices']
        step_names = data['step_names']

        current_step_idx = step_indices[0]
        segment_start = 0

        for i in range(1, len(step_indices)):
            # Detect step transition or end of data
            if step_indices[i] != current_step_idx or i == len(step_indices) - 1:
                if i == len(step_indices) - 1:
                    segment_end = i
                else:
                    segment_end = i - 1

                # Calculate segment characteristics
                segment_duration = time[segment_end] - time[segment_start]
                if segment_duration < 10:  # Skip very short segments
                    segment_start = i
                    current_step_idx = step_indices[i]
                    continue

                target_start = target[segment_start]
                target_end = target[segment_end]
                temp_change = target_end - target_start
                step_name = step_names[segment_start]
                step_index = step_indices[segment_start]

                # Classify segment type
                rate_per_min = (temp_change / segment_duration) * 60 if segment_duration > 0 else 0

                if abs(rate_per_min) < hold_threshold:
                    segment_type = 'hold'
                elif rate_per_min > hold_threshold:
                    segment_type = 'ramp'
                else:
                    segment_type = 'cooling'

                segments.append(Segment(segment_start, segment_end, segment_type,
                                      target_start, target_end, step_name, step_index))

                segment_start = i
                current_step_idx = step_indices[i]

    else:
        # Fallback: detect from target temperature changes
        current_target = target[0]
        segment_start = 0

        for i in range(1, len(target)):
            # Detect significant target change (>2°C) or end of data
            if abs(target[i] - current_target) > 2.0 or i == len(target) - 1:
                if i == len(target) - 1:
                    segment_end = i
                else:
                    segment_end = i - 1

                # Calculate segment characteristics
                segment_duration = time[segment_end] - time[segment_start]
                if segment_duration < 30:  # Skip very short segments
                    segment_start = i
                    current_target = target[i]
                    continue

                target_start = target[segment_start]
                target_end = target[segment_end]
                temp_change = target_end - target_start

                # Classify segment type
                rate_per_min = (temp_change / segment_duration) * 60 if segment_duration > 0 else 0

                if abs(rate_per_min) < hold_threshold:
                    segment_type = 'hold'
                elif rate_per_min > hold_threshold:
                    segment_type = 'ramp'
                else:
                    segment_type = 'cooling'

                segments.append(Segment(segment_start, segment_end, segment_type,
                                      target_start, target_end))

                segment_start = i
                current_target = target[i]

    return segments


# =============================================================================
# Performance Metrics Calculation
# =============================================================================

def detect_overshoot(data: Dict, segment: Segment) -> Dict:
    """
    Detect overshoot in a segment.

    Overshoot occurs when temperature exceeds target during heating.
    Only counts if temp was previously below target.

    Returns:
        Dictionary with overshoot metrics: max_overshoot, overshoot_time,
        overshoot_count, peak_temp
    """
    temp = data['temp']
    target = data['target']
    time = data['time']

    max_overshoot = 0.0
    overshoot_time = None
    overshoot_count = 0
    peak_temp = 0.0
    overshoot_locations = []

    # Only check for overshoot during heating (not cooling)
    if segment.segment_type == 'cooling':
        return {
            'max_overshoot': 0.0,
            'overshoot_time': None,
            'overshoot_count': 0,
            'peak_temp': 0.0,
            'locations': []
        }

    was_below = False
    for i in range(segment.start_idx, segment.end_idx + 1):
        if temp[i] < target[i]:
            was_below = True
        elif temp[i] > target[i] and was_below:
            # Overshoot detected
            overshoot = temp[i] - target[i]
            if overshoot > max_overshoot:
                max_overshoot = overshoot
                overshoot_time = time[i]
                peak_temp = temp[i]

            # Count distinct overshoot events (not every point)
            if overshoot > 1.0:  # Threshold to avoid counting noise
                if not overshoot_locations or (time[i] - overshoot_locations[-1] > 60):
                    overshoot_count += 1
                    overshoot_locations.append(time[i])

    return {
        'max_overshoot': max_overshoot,
        'overshoot_time': overshoot_time,
        'overshoot_count': overshoot_count,
        'peak_temp': peak_temp,
        'locations': overshoot_locations
    }


def calculate_settling_time(data: Dict, segment: Segment, tolerance: float = 5.0,
                           settle_duration: float = 60.0) -> Optional[float]:
    """
    Calculate settling time for a segment.

    Settling time is when temperature enters and stays within ±tolerance
    of target for at least settle_duration seconds.

    Returns:
        Settling time in seconds from segment start, or None if never settles
    """
    temp = data['temp']
    target = data['target']
    time = data['time']

    # Only meaningful for holds and end of ramps
    if segment.segment_type == 'cooling':
        return None

    in_band_start = None

    for i in range(segment.start_idx, segment.end_idx + 1):
        error = abs(temp[i] - target[i])

        if error <= tolerance:
            if in_band_start is None:
                in_band_start = i

            # Check if stayed in band for settle_duration
            if time[i] - time[in_band_start] >= settle_duration:
                settling_time = time[i] - time[segment.start_idx]
                return settling_time
        else:
            # Exited band, restart
            in_band_start = None

    return None  # Never settled


def calculate_steady_state_error(data: Dict, segment: Segment) -> Dict:
    """
    Calculate steady-state error for hold segments.

    Steady-state error is the final error after system has settled.
    Calculated from the last 25% of the segment to exclude transients.

    Returns:
        Dictionary with avg_error, max_error, error_std
    """
    temp = data['temp']
    target = data['target']

    # Only meaningful for hold segments
    if segment.segment_type != 'hold':
        return {'avg_error': None, 'max_error': None, 'error_std': None}

    # Use last 25% of segment for steady-state analysis
    segment_len = segment.end_idx - segment.start_idx + 1
    steady_start = segment.start_idx + int(0.75 * segment_len)

    errors = []
    for i in range(steady_start, segment.end_idx + 1):
        error = abs(temp[i] - target[i])
        errors.append(error)

    if not errors:
        return {'avg_error': None, 'max_error': None, 'error_std': None}

    avg_error = sum(errors) / len(errors)
    max_error = max(errors)

    # Calculate standard deviation
    if len(errors) > 1:
        variance = sum((e - avg_error) ** 2 for e in errors) / len(errors)
        error_std = math.sqrt(variance)
    else:
        error_std = 0.0

    return {
        'avg_error': avg_error,
        'max_error': max_error,
        'error_std': error_std
    }


def detect_oscillation(data: Dict, segment: Segment, min_cycles: int = 3) -> Optional[Dict]:
    """
    Detect sustained oscillation using zero-crossing analysis.

    Oscillation is detected when error signal crosses zero repeatedly
    with consistent period and amplitude.

    Args:
        min_cycles: Minimum number of complete cycles to detect oscillation

    Returns:
        Dictionary with period, amplitude, cycles, or None if no oscillation
    """
    temp = data['temp']
    target = data['target']
    time = data['time']

    # Calculate error signal
    errors = []
    for i in range(segment.start_idx, segment.end_idx + 1):
        error = temp[i] - target[i]
        errors.append(error)

    if len(errors) < 20:
        return None

    # Find zero crossings
    zero_crossings = []
    for i in range(1, len(errors)):
        if errors[i-1] * errors[i] < 0:  # Sign change
            zero_crossings.append(i)

    # Need at least min_cycles * 2 crossings (2 per cycle)
    if len(zero_crossings) < min_cycles * 2:
        return None

    # Calculate periods (time between peaks = 2 zero crossings)
    periods = []
    for i in range(0, len(zero_crossings) - 2, 2):
        idx1 = segment.start_idx + zero_crossings[i]
        idx2 = segment.start_idx + zero_crossings[i + 2]
        period = time[idx2] - time[idx1]
        periods.append(period)

    if not periods:
        return None

    avg_period = sum(periods) / len(periods)

    # Calculate amplitude (peak-to-peak over oscillation region)
    # Find peaks between zero crossings
    amplitudes = []
    for i in range(len(zero_crossings) - 1):
        start = zero_crossings[i]
        end = zero_crossings[i + 1]
        segment_errors = errors[start:end]
        if segment_errors:
            peak = max(abs(e) for e in segment_errors)
            amplitudes.append(peak * 2)  # Peak-to-peak

    avg_amplitude = sum(amplitudes) / len(amplitudes) if amplitudes else 0.0

    # Only report if amplitude is significant (>0.5°C)
    if avg_amplitude < 0.5:
        return None

    return {
        'period': avg_period,
        'amplitude': avg_amplitude,
        'cycles': len(zero_crossings) // 2
    }


def calculate_tracking_lag(data: Dict, segment: Segment) -> Optional[Dict]:
    """
    Calculate tracking lag during ramp segments.

    Lag is the difference between target and actual temperature while ramping.
    Only applicable to ramp segments where target is changing.

    Returns:
        Dictionary with avg_lag, max_lag, rms_error, or None for non-ramp segments
    """
    temp = data['temp']
    target = data['target']
    time = data['time']

    # Only applicable to ramp segments
    if segment.segment_type != 'ramp':
        return None

    lags = []
    for i in range(segment.start_idx + 1, segment.end_idx + 1):
        # Check if target is actually changing
        if abs(target[i] - target[i-1]) > 0.05:
            lag = target[i] - temp[i]
            # Only count positive lag (temp below target during heating)
            if lag > 0 or segment.segment_type == 'cooling':
                lags.append(abs(lag))

    if not lags:
        return None

    avg_lag = sum(lags) / len(lags)
    max_lag = max(lags)
    rms_error = math.sqrt(sum(l**2 for l in lags) / len(lags))

    return {
        'avg_lag': avg_lag,
        'max_lag': max_lag,
        'rms_error': rms_error
    }


def calculate_control_effort(data: Dict, segment: Segment) -> Dict:
    """
    Calculate control effort statistics.

    Measures how hard the controller is working (SSR output variability).

    Returns:
        Dictionary with ssr_min, ssr_max, ssr_mean, ssr_std, saturation_time
    """
    ssr = data['ssr_output']
    time = data['time']

    segment_ssr = ssr[segment.start_idx:segment.end_idx + 1]

    if not segment_ssr:
        return {
            'ssr_min': 0, 'ssr_max': 0, 'ssr_mean': 0,
            'ssr_std': 0, 'saturation_time': 0
        }

    ssr_min = min(segment_ssr)
    ssr_max = max(segment_ssr)
    ssr_mean = sum(segment_ssr) / len(segment_ssr)

    # Calculate standard deviation
    if len(segment_ssr) > 1:
        variance = sum((s - ssr_mean) ** 2 for s in segment_ssr) / len(segment_ssr)
        ssr_std = math.sqrt(variance)
    else:
        ssr_std = 0.0

    # Calculate saturation time (SSR at 0% or 100%)
    saturation_count = sum(1 for s in segment_ssr if s <= 1.0 or s >= 99.0)
    segment_duration = time[segment.end_idx] - time[segment.start_idx]
    saturation_time = (saturation_count / len(segment_ssr)) * segment_duration if segment_ssr else 0

    return {
        'ssr_min': ssr_min,
        'ssr_max': ssr_max,
        'ssr_mean': ssr_mean,
        'ssr_std': ssr_std,
        'saturation_time': saturation_time
    }


# =============================================================================
# Grading System
# =============================================================================

def grade_overshoot(overshoot: float) -> Tuple[str, str]:
    """
    Grade overshoot performance.

    Returns: (grade, symbol)
    """
    if overshoot < 2.0:
        return "EXCELLENT", "✓"
    elif overshoot < 5.0:
        return "GOOD", "✓"
    elif overshoot < 10.0:
        return "ACCEPTABLE", "⚠️"
    else:
        return "POOR", "⚠️"


def grade_settling_time(settling_time: Optional[float]) -> Tuple[str, str]:
    """
    Grade settling time performance.

    Returns: (grade, symbol)
    """
    if settling_time is None:
        return "POOR", "⚠️"
    elif settling_time < 60:
        return "EXCELLENT", "✓"
    elif settling_time < 120:
        return "GOOD", "✓"
    elif settling_time < 300:
        return "ACCEPTABLE", "⚠️"
    else:
        return "POOR", "⚠️"


def grade_steady_state_error(error: Optional[float]) -> Tuple[str, str]:
    """
    Grade steady-state error performance.

    Returns: (grade, symbol)
    """
    if error is None:
        return "N/A", ""
    elif error < 1.0:
        return "EXCELLENT", "✓"
    elif error < 3.0:
        return "GOOD", "✓"
    elif error < 5.0:
        return "ACCEPTABLE", "⚠️"
    else:
        return "POOR", "⚠️"


def grade_oscillation(osc: Optional[Dict]) -> Tuple[str, str]:
    """
    Grade oscillation performance.

    Returns: (grade, symbol)
    """
    if osc is None:
        return "EXCELLENT", "✓"
    elif osc['amplitude'] < 2.0:
        return "ACCEPTABLE", "⚠️"
    elif osc['amplitude'] < 5.0:
        return "ACCEPTABLE", "⚠️"
    else:
        return "POOR", "⚠️"


def grade_tracking_lag(lag: Optional[Dict]) -> Tuple[str, str]:
    """
    Grade tracking lag performance.

    Returns: (grade, symbol)
    """
    if lag is None:
        return "N/A", ""

    avg_lag = lag['avg_lag']
    if avg_lag < 5.0:
        return "EXCELLENT", "✓"
    elif avg_lag < 10.0:
        return "GOOD", "✓"
    elif avg_lag < 20.0:
        return "ACCEPTABLE", "⚠️"
    else:
        return "POOR", "⚠️"


def grade_control_effort(control: Dict) -> Tuple[str, str]:
    """
    Grade control effort performance.

    Returns: (grade, symbol)
    """
    std = control['ssr_std']
    sat_pct = (control['saturation_time'] / 100) if 'saturation_time' in control else 0

    # Smooth control with low saturation is excellent
    if std < 10.0 and sat_pct < 0.1:
        return "EXCELLENT", "✓"
    elif std < 20.0 and sat_pct < 0.3:
        return "GOOD", "✓"
    elif std < 30.0:
        return "ACCEPTABLE", "⚠️"
    else:
        return "POOR", "⚠️"


def grade_segment(segment: Segment) -> Tuple[str, str]:
    """
    Grade overall segment performance.

    Combines all metrics to produce overall segment grade.

    Returns: (grade, symbol)
    """
    grades = []
    grade_values = {"EXCELLENT": 4, "GOOD": 3, "ACCEPTABLE": 2, "POOR": 1, "N/A": 0}

    metrics = segment.metrics

    # Overshoot
    if 'overshoot' in metrics:
        grade, _ = grade_overshoot(metrics['overshoot']['max_overshoot'])
        if grade != "N/A":
            grades.append(grade_values[grade])

    # Settling time
    if 'settling_time' in metrics:
        grade, _ = grade_settling_time(metrics['settling_time'])
        if grade != "N/A":
            grades.append(grade_values[grade])

    # Steady-state error
    if 'steady_state' in metrics and metrics['steady_state']['avg_error'] is not None:
        grade, _ = grade_steady_state_error(metrics['steady_state']['avg_error'])
        if grade != "N/A":
            grades.append(grade_values[grade])

    # Oscillation
    if 'oscillation' in metrics:
        grade, _ = grade_oscillation(metrics['oscillation'])
        if grade != "N/A":
            grades.append(grade_values[grade])

    # Tracking lag
    if 'tracking_lag' in metrics and metrics['tracking_lag'] is not None:
        grade, _ = grade_tracking_lag(metrics['tracking_lag'])
        if grade != "N/A":
            grades.append(grade_values[grade])

    # Control effort
    if 'control_effort' in metrics:
        grade, _ = grade_control_effort(metrics['control_effort'])
        if grade != "N/A":
            grades.append(grade_values[grade])

    if not grades:
        return "UNKNOWN", ""

    # Calculate average grade
    avg_grade = sum(grades) / len(grades)

    if avg_grade >= 3.5:
        return "EXCELLENT", "✓"
    elif avg_grade >= 2.5:
        return "GOOD", "✓"
    elif avg_grade >= 1.5:
        return "ACCEPTABLE", "⚠️"
    else:
        return "POOR", "⚠️"


def grade_overall(segments: List[Segment]) -> Tuple[str, str]:
    """
    Grade overall performance across all segments.

    Returns: (grade, symbol)
    """
    grade_values = {"EXCELLENT": 4, "GOOD": 3, "ACCEPTABLE": 2, "POOR": 1, "UNKNOWN": 0}

    grades = []
    for seg in segments:
        if seg.grade != "UNKNOWN":
            grades.append(grade_values[seg.grade])

    if not grades:
        return "UNKNOWN", ""

    avg_grade = sum(grades) / len(grades)

    if avg_grade >= 3.5:
        return "EXCELLENT", "✓"
    elif avg_grade >= 2.5:
        return "GOOD", "✓"
    elif avg_grade >= 1.5:
        return "ACCEPTABLE", "⚠️"
    else:
        return "POOR", "⚠️"


# =============================================================================
# Recommendation Engine
# =============================================================================

class Recommendation:
    """Represents a tuning recommendation."""
    def __init__(self, priority: int, issue: str, cause: str, fix: str,
                 temp_range: Optional[str] = None):
        self.priority = priority
        self.issue = issue
        self.cause = cause
        self.fix = fix
        self.temp_range = temp_range


def generate_recommendations(data: Dict, segments: List[Segment]) -> List[Recommendation]:
    """
    Generate actionable recommendations based on detected issues.

    Returns:
        List of Recommendation objects, sorted by priority
    """
    recommendations = []

    # Analyze overall patterns
    high_temp_segments = [s for s in segments if s.target_end > 1000]
    low_temp_segments = [s for s in segments if s.target_end < 300]

    # Issue 1: High temperature overshoot
    high_temp_overshoot = [s for s in high_temp_segments
                          if 'overshoot' in s.metrics
                          and s.metrics['overshoot']['max_overshoot'] > 5.0]

    if high_temp_overshoot:
        max_overshoot = max(s.metrics['overshoot']['max_overshoot'] for s in high_temp_overshoot)
        recommendations.append(Recommendation(
            priority=1,
            issue=f"High Temperature Overshoot ({max_overshoot:.1f}°C at >{high_temp_overshoot[0].target_end:.0f}°C)",
            cause="System dynamics change with temperature, gains too aggressive",
            fix="Reduce Kp by 15-20% at high temperature OR implement gain scheduling",
            temp_range="HIGH (>1000°C)"
        ))

    # Issue 2: General overshoot across all temps
    overshoot_segments = [s for s in segments
                         if 'overshoot' in s.metrics
                         and s.metrics['overshoot']['max_overshoot'] > 10.0]

    if overshoot_segments and not high_temp_overshoot:
        recommendations.append(Recommendation(
            priority=1,
            issue="Excessive Overshoot Across Temperature Range",
            cause="Proportional and/or integral gain too high",
            fix="Reduce Kp by 10-15% AND reduce Ki by 10-20%",
            temp_range="ALL"
        ))

    # Issue 3: Oscillation detection
    oscillating_segments = [s for s in segments
                           if 'oscillation' in s.metrics
                           and s.metrics['oscillation'] is not None]

    if oscillating_segments:
        osc = oscillating_segments[0].metrics['oscillation']
        period = osc['period']
        amplitude = osc['amplitude']

        if period < 30:
            # Fast oscillation - derivative gain too high
            recommendations.append(Recommendation(
                priority=2,
                issue=f"Fast Oscillation (Period {period:.0f}s, Amplitude {amplitude:.1f}°C)",
                cause="Derivative gain too high, amplifying noise",
                fix="Reduce Kd by 20-30% OR increase derivative filter time constant",
                temp_range="ALL"
            ))
        elif period > 60:
            # Slow oscillation - proportional gain too high
            temp_range = "HIGH" if oscillating_segments[0].target_end > 1000 else "ALL"
            recommendations.append(Recommendation(
                priority=2,
                issue=f"Slow Oscillation (Period {period:.0f}s, Amplitude {amplitude:.1f}°C)",
                cause="Proportional gain too high for system dynamics",
                fix="Reduce Kp by 15-20%",
                temp_range=temp_range
            ))

    # Issue 4: Slow settling time
    slow_settling = [s for s in segments
                    if 'settling_time' in s.metrics
                    and s.metrics['settling_time'] is not None
                    and s.metrics['settling_time'] > 300]

    if slow_settling:
        recommendations.append(Recommendation(
            priority=3,
            issue=f"Slow Settling Time (>{slow_settling[0].metrics['settling_time']:.0f}s)",
            cause="Gains too low or system overdamped",
            fix="Increase Kp by 10-15% OR increase Kd by 20%",
            temp_range="ALL"
        ))

    # Issue 5: Large steady-state error
    high_ss_error = [s for s in segments
                    if 'steady_state' in s.metrics
                    and s.metrics['steady_state']['avg_error'] is not None
                    and s.metrics['steady_state']['avg_error'] > 5.0]

    if high_ss_error:
        recommendations.append(Recommendation(
            priority=2,
            issue=f"Large Steady-State Error ({high_ss_error[0].metrics['steady_state']['avg_error']:.1f}°C)",
            cause="Integral gain too low",
            fix="Increase Ki by 20-30%",
            temp_range="ALL"
        ))

    # Issue 6: Tracking lag increases with temperature
    ramp_segments = [s for s in segments if s.segment_type == 'ramp'
                    and 'tracking_lag' in s.metrics
                    and s.metrics['tracking_lag'] is not None]

    if len(ramp_segments) >= 2:
        # Check if lag increases with temperature
        low_temp_ramps = [s for s in ramp_segments if s.target_end < 500]
        high_temp_ramps = [s for s in ramp_segments if s.target_end > 800]

        if low_temp_ramps and high_temp_ramps:
            low_lag = sum(s.metrics['tracking_lag']['avg_lag'] for s in low_temp_ramps) / len(low_temp_ramps)
            high_lag = sum(s.metrics['tracking_lag']['avg_lag'] for s in high_temp_ramps) / len(high_temp_ramps)

            if high_lag > low_lag * 1.5:  # 50% increase
                recommendations.append(Recommendation(
                    priority=1,
                    issue=f"Tracking Lag Increases with Temperature ({low_lag:.1f}°C -> {high_lag:.1f}°C)",
                    cause="Thermal mass increases, time constant grows with temperature",
                    fix="Implement gain scheduling with higher Ki at high temp OR reduce ramp rate above 800°C",
                    temp_range="HIGH (>800°C)"
                ))

    # Issue 7: Large tracking lag
    large_lag_segments = [s for s in ramp_segments
                         if s.metrics['tracking_lag']['avg_lag'] > 20.0]

    if large_lag_segments:
        recommendations.append(Recommendation(
            priority=3,
            issue="Excessive Tracking Lag During Ramps",
            cause="System too slow OR ramp rate too fast",
            fix="Reduce ramp rate by 25-30% OR increase Ki by 20%",
            temp_range="ALL"
        ))

    # Issue 8: Erratic control
    erratic_segments = [s for s in segments
                       if 'control_effort' in s.metrics
                       and s.metrics['control_effort']['ssr_std'] > 30.0]

    if erratic_segments:
        recommendations.append(Recommendation(
            priority=2,
            issue="Erratic Control Signal (High SSR Variability)",
            cause="Derivative gain too high, amplifying measurement noise",
            fix="Reduce Kd by 30-40% OR add low-pass filter to temperature measurement",
            temp_range="ALL"
        ))

    # Issue 9: Constant saturation
    saturated_segments = [s for s in segments
                         if 'control_effort' in s.metrics
                         and s.metrics['control_effort']['saturation_time'] > 60]

    if saturated_segments:
        recommendations.append(Recommendation(
            priority=3,
            issue="Frequent SSR Saturation",
            cause="System underpowered OR ramp rate too fast",
            fix="Reduce ramp rate by 20-30% OR increase heater capacity",
            temp_range="ALL"
        ))

    # Sort by priority
    recommendations.sort(key=lambda r: r.priority)

    return recommendations


# =============================================================================
# Analysis Pipeline
# =============================================================================

def analyze_segment_performance(data: Dict, segment: Segment) -> None:
    """
    Analyze performance metrics for a single segment.

    Updates segment.metrics dictionary with all calculated metrics.
    """
    # Calculate all metrics
    segment.metrics['overshoot'] = detect_overshoot(data, segment)
    segment.metrics['settling_time'] = calculate_settling_time(data, segment)
    segment.metrics['steady_state'] = calculate_steady_state_error(data, segment)
    segment.metrics['oscillation'] = detect_oscillation(data, segment)
    segment.metrics['tracking_lag'] = calculate_tracking_lag(data, segment)
    segment.metrics['control_effort'] = calculate_control_effort(data, segment)

    # Grade segment
    segment.grade, _ = grade_segment(segment)

    # Identify issues
    if segment.metrics['overshoot']['max_overshoot'] > 5.0:
        segment.issues.append("overshoot")
    if segment.metrics['oscillation'] is not None:
        segment.issues.append("oscillation")
    if segment.metrics['tracking_lag'] and segment.metrics['tracking_lag']['avg_lag'] > 15.0:
        segment.issues.append("tracking_lag")


def analyze_profile_performance(csv_file: str, verbose: bool = False) -> Dict:
    """
    Main analysis pipeline.

    Args:
        csv_file: Path to profile run CSV
        verbose: Print verbose debugging info

    Returns:
        Dictionary with complete analysis results
    """
    # Load data
    if verbose:
        print(f"Loading data from {csv_file}...")
    data = load_profile_data(csv_file)

    if verbose:
        print(f"Loaded {len(data['time'])} data points")

    # Detect segments
    if verbose:
        print("Detecting segments...")
    segments = detect_segments(data)

    if verbose:
        print(f"Detected {len(segments)} segments")

    # Analyze each segment
    for i, segment in enumerate(segments):
        if verbose:
            print(f"Analyzing segment {i}: {segment.segment_type}")
        analyze_segment_performance(data, segment)

    # Generate overall grade
    overall_grade, overall_symbol = grade_overall(segments)

    # Generate recommendations
    recommendations = generate_recommendations(data, segments)

    # Compile results
    results = {
        'run_info': {
            'csv_file': csv_file,
            'duration_s': data['time'][-1] - data['time'][0],
            'data_points': len(data['time']),
            'temp_min': min(data['temp']),
            'temp_max': max(data['temp']),
            'segments': len(segments)
        },
        'overall_performance': {
            'grade': overall_grade,
            'symbol': overall_symbol
        },
        'segments': segments,
        'recommendations': recommendations,
        'data': data
    }

    return results


# =============================================================================
# Output Generation
# =============================================================================

def generate_results_json(results: Dict) -> Dict:
    """
    Generate JSON-serializable results dictionary.
    """
    segments_json = []
    for seg in results['segments']:
        seg_json = {
            'segment_index': seg.step_index if seg.step_index is not None else segments_json.__len__(),
            'segment_type': seg.segment_type,
            'temp_start': round(seg.target_start, 1),
            'temp_end': round(seg.target_end, 1),
            'duration_s': round(results['data']['time'][seg.end_idx] - results['data']['time'][seg.start_idx], 1),
            'grade': seg.grade,
            'issues': seg.issues,
            'metrics': {}
        }

        # Add metrics (convert to serializable format)
        if 'overshoot' in seg.metrics:
            seg_json['metrics']['overshoot'] = {
                'max_overshoot': round(seg.metrics['overshoot']['max_overshoot'], 2),
                'overshoot_count': seg.metrics['overshoot']['overshoot_count'],
                'peak_temp': round(seg.metrics['overshoot']['peak_temp'], 1)
            }

        if 'settling_time' in seg.metrics and seg.metrics['settling_time'] is not None:
            seg_json['metrics']['settling_time'] = round(seg.metrics['settling_time'], 1)

        if 'steady_state' in seg.metrics and seg.metrics['steady_state']['avg_error'] is not None:
            seg_json['metrics']['steady_state'] = {
                'avg_error': round(seg.metrics['steady_state']['avg_error'], 2),
                'max_error': round(seg.metrics['steady_state']['max_error'], 2)
            }

        if 'oscillation' in seg.metrics and seg.metrics['oscillation'] is not None:
            seg_json['metrics']['oscillation'] = {
                'period': round(seg.metrics['oscillation']['period'], 1),
                'amplitude': round(seg.metrics['oscillation']['amplitude'], 2),
                'cycles': seg.metrics['oscillation']['cycles']
            }

        if 'tracking_lag' in seg.metrics and seg.metrics['tracking_lag'] is not None:
            seg_json['metrics']['tracking_lag'] = {
                'avg_lag': round(seg.metrics['tracking_lag']['avg_lag'], 2),
                'max_lag': round(seg.metrics['tracking_lag']['max_lag'], 2)
            }

        if 'control_effort' in seg.metrics:
            seg_json['metrics']['control_effort'] = {
                'ssr_min': round(seg.metrics['control_effort']['ssr_min'], 1),
                'ssr_max': round(seg.metrics['control_effort']['ssr_max'], 1),
                'ssr_mean': round(seg.metrics['control_effort']['ssr_mean'], 1)
            }

        segments_json.append(seg_json)

    recommendations_json = []
    for rec in results['recommendations']:
        recommendations_json.append({
            'priority': rec.priority,
            'issue': rec.issue,
            'cause': rec.cause,
            'fix': rec.fix,
            'temp_range': rec.temp_range
        })

    return {
        'run_info': results['run_info'],
        'overall_performance': {
            'grade': results['overall_performance']['grade']
        },
        'segments': segments_json,
        'recommendations': recommendations_json
    }


def print_beautiful_report(results: Dict):
    """
    Print beautifully formatted analysis report to terminal.
    """
    data = results['data']
    segments = results['segments']
    recommendations = results['recommendations']
    overall_grade = results['overall_performance']['grade']

    # Header
    print("\n" + "=" * 80)
    print(" " * 20 + "PID PERFORMANCE ANALYSIS REPORT")
    print("=" * 80)

    # Run Information
    print("\n┌─ RUN INFORMATION " + "─" * 61)
    print(f"│  CSV File:         {Path(results['run_info']['csv_file']).name}")
    print(f"│  Duration:         {results['run_info']['duration_s'] / 3600:.1f} hours ({results['run_info']['duration_s'] / 60:.0f} min)")
    print(f"│  Temperature:      {results['run_info']['temp_min']:.1f}°C → {results['run_info']['temp_max']:.1f}°C")
    print(f"│  Segments:         {results['run_info']['segments']}")
    print(f"│  Overall Grade:    {overall_grade}")
    print("└" + "─" * 79)

    # Performance Summary
    print("\n┌─ PERFORMANCE SUMMARY " + "─" * 57)

    # Calculate summary metrics
    all_overshoot = [s.metrics['overshoot']['max_overshoot'] for s in segments
                     if 'overshoot' in s.metrics and s.metrics['overshoot']['max_overshoot'] > 0]
    max_overshoot = max(all_overshoot) if all_overshoot else 0.0
    overshoot_grade, overshoot_sym = grade_overshoot(max_overshoot)

    all_settling = [s.metrics['settling_time'] for s in segments
                   if 'settling_time' in s.metrics and s.metrics['settling_time'] is not None]
    avg_settling = sum(all_settling) / len(all_settling) if all_settling else None
    settling_grade, settling_sym = grade_settling_time(avg_settling)

    all_ss_error = [s.metrics['steady_state']['avg_error'] for s in segments
                   if 'steady_state' in s.metrics and s.metrics['steady_state']['avg_error'] is not None]
    avg_ss_error = sum(all_ss_error) / len(all_ss_error) if all_ss_error else None
    ss_grade, ss_sym = grade_steady_state_error(avg_ss_error)

    has_oscillation = any(s.metrics.get('oscillation') is not None for s in segments)
    osc_grade, osc_sym = grade_oscillation(segments[0].metrics.get('oscillation') if segments else None)

    all_lag = [s.metrics['tracking_lag']['avg_lag'] for s in segments
              if 'tracking_lag' in s.metrics and s.metrics['tracking_lag'] is not None]
    avg_lag = sum(all_lag) / len(all_lag) if all_lag else None
    lag_grade, lag_sym = grade_tracking_lag({'avg_lag': avg_lag} if avg_lag else None)

    # Print summary
    if max_overshoot > 0:
        print(f"│  Overshoot:        Max {max_overshoot:.1f}°C ({overshoot_grade}) {overshoot_sym}")
    else:
        print(f"│  Overshoot:        None detected (EXCELLENT) ✓")

    if avg_settling:
        print(f"│  Settling Time:    Avg {avg_settling:.0f}s ({settling_grade}) {settling_sym}")
    else:
        print(f"│  Settling Time:    Not measured")

    if avg_ss_error:
        print(f"│  Steady-State:     Avg error {avg_ss_error:.1f}°C ({ss_grade}) {ss_sym}")
    else:
        print(f"│  Steady-State:     N/A (no hold segments)")

    if has_oscillation:
        print(f"│  Oscillation:      Detected ({osc_grade}) {osc_sym}")
    else:
        print(f"│  Oscillation:      None detected (EXCELLENT) ✓")

    if avg_lag:
        print(f"│  Tracking:         Avg lag {avg_lag:.1f}°C during ramps ({lag_grade}) {lag_sym}")
    else:
        print(f"│  Tracking:         N/A (no ramp segments)")

    print(f"│  Control Effort:   SSR range {min(data['ssr_output']):.0f}-{max(data['ssr_output']):.0f}%")

    print("└" + "─" * 79)

    # Per-Segment Analysis
    print("\n┌─ PER-SEGMENT ANALYSIS " + "─" * 56)
    print("│")

    for i, seg in enumerate(segments):
        # Segment header
        step_label = f"SEGMENT {seg.step_index}" if seg.step_index is not None else f"SEGMENT {i}"
        if seg.step_name:
            step_label += f": {seg.step_name}"
        else:
            step_label += f": {seg.segment_type.capitalize()} {seg.target_start:.0f}°C → {seg.target_end:.0f}°C"

        duration_min = (data['time'][seg.end_idx] - data['time'][seg.start_idx]) / 60

        print(f"│  {step_label}")
        print("│  " + "─" * 76)
        print(f"│    Segment Type:     {seg.segment_type.capitalize()}")
        print(f"│    Temperature:      {seg.target_start:.0f}°C → {seg.target_end:.0f}°C")
        print(f"│    Duration:         {duration_min:.1f} min")
        print("│")
        print("│    Performance Metrics:")

        # Overshoot
        if seg.metrics['overshoot']['max_overshoot'] > 0:
            ovs = seg.metrics['overshoot']['max_overshoot']
            grade, sym = grade_overshoot(ovs)
            print(f"│      Overshoot:      {ovs:.1f}°C ({grade}) {sym}")
        elif seg.segment_type != 'cooling':
            print(f"│      Overshoot:      None detected (EXCELLENT) ✓")

        # Settling time
        if seg.metrics['settling_time'] is not None:
            st = seg.metrics['settling_time']
            grade, sym = grade_settling_time(st)
            print(f"│      Settling Time:  {st:.0f}s after reaching target ({grade}) {sym}")
        elif seg.segment_type == 'hold':
            print(f"│      Settling Time:  Did not settle (POOR) ⚠️")

        # Steady-state error
        if seg.metrics['steady_state']['avg_error'] is not None:
            sse = seg.metrics['steady_state']['avg_error']
            grade, sym = grade_steady_state_error(sse)
            print(f"│      Steady-State:   ±{sse:.1f}°C ({grade}) {sym}")

        # Oscillation
        if seg.metrics['oscillation'] is not None:
            osc = seg.metrics['oscillation']
            grade, sym = grade_oscillation(osc)
            print(f"│      Oscillation:    Period {osc['period']:.0f}s, amplitude {osc['amplitude']:.1f}°C ({grade}) {sym}")
        elif seg.segment_type == 'hold':
            print(f"│      Oscillation:    None detected (EXCELLENT) ✓")

        # Tracking lag
        if seg.metrics['tracking_lag'] is not None:
            lag = seg.metrics['tracking_lag']
            grade, sym = grade_tracking_lag(lag)
            print(f"│      Tracking Lag:   {lag['avg_lag']:.1f}°C average ({grade}) {sym}")

        # Control effort
        ctrl = seg.metrics['control_effort']
        grade, sym = grade_control_effort(ctrl)
        print(f"│      Control:        SSR {ctrl['ssr_min']:.0f}-{ctrl['ssr_max']:.0f}%, avg {ctrl['ssr_mean']:.0f}% ({grade}) {sym}")

        print("│")
        print(f"│    Assessment:       {seg.grade} {results['overall_performance']['symbol']}")

        if seg.issues:
            print(f"│    Issues:           {', '.join(seg.issues)}")

        print("│")

    print("└" + "─" * 79)

    # Recommendations
    print("\n┌─ RECOMMENDATIONS " + "─" * 61)
    print("│")
    print(f"│  OVERALL ASSESSMENT: {overall_grade}")
    print("│  " + "─" * 76)

    if overall_grade == "EXCELLENT":
        print("│  The PID controller performs excellently across all segments.")
        print("│  No tuning adjustments needed.")
    elif overall_grade == "GOOD":
        print("│  The PID controller performs well with minor issues.")
        print("│  Consider the recommendations below for optimization.")
    elif overall_grade == "ACCEPTABLE":
        print("│  The PID controller has acceptable performance with some issues.")
        print("│  Follow recommendations below to improve performance.")
    else:
        print("│  The PID controller has significant performance issues.")
        print("│  Tuning adjustments strongly recommended.")

    print("│")

    if recommendations:
        print("│  SPECIFIC ISSUES:")
        print("│  " + "─" * 76)

        for i, rec in enumerate(recommendations, 1):
            print(f"│  {i}. {rec.issue}")
            print(f"│     Cause:  {rec.cause}")
            print(f"│     Fix:    {rec.fix}")
            if rec.temp_range:
                print(f"│     Range:  {rec.temp_range}")
            print("│")

        print("│  RECOMMENDED ACTIONS:")
        print("│  " + "─" * 76)

        # Prioritized action list
        priority_1 = [r for r in recommendations if r.priority == 1]
        priority_2 = [r for r in recommendations if r.priority == 2]
        priority_3 = [r for r in recommendations if r.priority == 3]

        if priority_1:
            print("│  Priority 1 (Critical):")
            for rec in priority_1:
                print(f"│    - {rec.fix}")

        if priority_2:
            print("│  Priority 2 (Important):")
            for rec in priority_2:
                print(f"│    - {rec.fix}")

        if priority_3:
            print("│  Priority 3 (Optional):")
            for rec in priority_3:
                print(f"│    - {rec.fix}")

        print("│")
        print("│  NEXT STEPS:")
        print("│  1. Apply recommended PID adjustments to config.py")
        print("│  2. Run another test profile")
        print("│  3. Re-analyze with this tool to verify improvements")
    else:
        print("│  No specific issues detected. Controller is performing well!")

    print("└" + "─" * 79)

    print("\n" + "=" * 80)
    print()


# =============================================================================
# Main Entry Point
# =============================================================================

def main():
    """Main entry point."""
    # Parse command line arguments
    if len(sys.argv) < 2:
        print("\nUsage: python analyze_pid_performance.py <profile_csv_file> [--json-only] [--verbose]")
        print("\nExample:")
        print("  python analyze_pid_performance.py logs/cone6_glaze_2025-10-25_14-23-45.csv")
        print("  python analyze_pid_performance.py logs/profile_run.csv --json-only")
        print("  python analyze_pid_performance.py logs/profile_run.csv --verbose")
        sys.exit(1)

    csv_file = sys.argv[1]
    json_only = '--json-only' in sys.argv
    verbose = '--verbose' in sys.argv

    # Check if file exists
    if not Path(csv_file).exists():
        print(f"\n❌ Error: File not found: {csv_file}")
        sys.exit(1)

    if not json_only:
        print("\n" + "=" * 80)
        print(" " * 15 + "PID PERFORMANCE ANALYZER")
        print("=" * 80)
        print(f"\n📂 Analyzing: {csv_file}")

    try:
        # Run analysis
        results = analyze_profile_performance(csv_file, verbose=verbose)

        # Generate JSON output
        json_results = generate_results_json(results)
        output_file = "pid_performance_results.json"
        with open(output_file, 'w') as f:
            json.dump(json_results, f, indent=2)

        if not json_only:
            print(f"✓ Results saved to: {output_file}")

        # Print report (unless json-only mode)
        if not json_only:
            print_beautiful_report(results)
        else:
            # In json-only mode, print the JSON to stdout
            print(json.dumps(json_results, indent=2))

    except Exception as e:
        print(f"\n❌ Error: {e}")
        if verbose:
            import traceback
            traceback.print_exc()
        sys.exit(1)


if __name__ == "__main__":
    main()
