"""
Reporting and Output Generation

This module handles test quality assessment, per-step analysis,
JSON output generation, and beautiful report formatting.
"""

import json
import math
from pathlib import Path
from typing import Dict, List, Optional
from .data import Phase
from .thermal import ThermalModel
from .pid import PIDParams


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


def generate_config_snippet():
    """
    Generate config.py snippet from tuning_results.json

    This helper function reads tuning_results.json (generated by main() above)
    and prints a ready-to-paste THERMAL_MODEL configuration for config.py.

    Usage:
        python analyze_tuning.py <csv_file>  # Generates tuning_results.json
        python -c "from analyze_tuning import generate_config_snippet; generate_config_snippet()"
    """
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
