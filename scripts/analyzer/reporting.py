"""
Reporting and Output Generation

This module handles test quality assessment, JSON output generation,
and beautiful report formatting.
"""

import json
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


# =============================================================================
# Output Generation
# =============================================================================

def generate_results_json(data: Dict, phases: List[Phase], model: ThermalModel,
                         pid_methods: Dict[str, PIDParams],
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
            'heat_loss_coefficient': round(model.heat_loss_coefficient, 6),
            'ambient_temp': round(model.ambient_temp, 1),
            'gain_vs_temp': model.gain_vs_temp,
            'gain_method': model.gain_method,
            'gain_confidence': model.gain_confidence,
            'heat_loss_method': model.heat_loss_method
        },
        'pid_methods': {name: pid.to_dict() for name, pid in pid_methods.items()},
        'recommended': recommended_method,
        'test_quality': test_quality
    }


def print_beautiful_report(data: Dict, phases: List[Phase], model: ThermalModel,
                          pid_methods: Dict[str, PIDParams],
                          test_quality: str, recommended_method: str):
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
    for i, phase in enumerate(phases, 1):
        print(f"│    {i}. {phase.phase_type.upper():8} - SSR: {phase.avg_ssr:5.1f}% | {phase.temp_start:6.1f}°C → {phase.temp_end:6.1f}°C")
    print("└" + "─" * 79)

    # Thermal Model
    print("\n┌─ THERMAL MODEL PARAMETERS " + "─" * 52)
    print(f"│  Dead Time (L):        {model.dead_time_s:8.2f} seconds")
    print(f"│  Time Constant (τ):    {model.time_constant_s:8.1f} seconds ({model.time_constant_s/60:.1f} min)")
    print(f"│  L/τ Ratio:            {model.dead_time_s/model.time_constant_s if model.time_constant_s > 0 else 0:8.3f}")
    print(f"│  Base Gain (K):        {model.steady_state_gain:8.4f} °C per % SSR (from {model.gain_method})")
    print(f"│  Gain Confidence:      {model.gain_confidence}")
    print(f"│  Heat Loss Coeff (h):  {model.heat_loss_coefficient:8.6f} (from {model.heat_loss_method})")
    print(f"│  Ambient Temp:         {model.ambient_temp:8.1f}°C")
    print("└" + "─" * 79)

    # Gain Scheduling
    if model.gain_vs_temp:
        print("\n┌─ GAIN SCHEDULING (Effective Gain vs Temperature) " + "─" * 28)
        print("│  Gain varies with temperature due to heat loss. PID is scaled accordingly.")
        print("│")
        print("│  Temperature    Effective Gain    SSR Used      Gain Ratio")
        print("│  ───────────    ──────────────    ────────      ──────────")
        base_gain = model.steady_state_gain if model.steady_state_gain > 0 else 1.0
        for gp in model.gain_vs_temp:
            ratio = gp['gain'] / base_gain if base_gain > 0 else 1.0
            print(f"│     {gp['temp']:6.1f}°C          {gp['gain']:6.4f}         {gp['ssr']:5.1f}%          {ratio:5.2f}x")
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
    and prints a ready-to-paste configuration for config.py using continuous
    gain scheduling.

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

    thermal_model = results.get('thermal_model', {})
    recommended = results.get('recommended')
    pid_methods = results.get('pid_methods', {})

    # Print header
    print("\n" + "=" * 80)
    print(" " * 20 + "CONTINUOUS GAIN SCHEDULING CONFIG")
    print("=" * 80)
    print("\nCopy the following into your config.py file:\n")
    print("-" * 80)

    # Get recommended PID parameters
    if recommended and recommended in pid_methods:
        pid = pid_methods[recommended]
    else:
        # Fallback to first available PID method
        pid = list(pid_methods.values())[0] if pid_methods else {'kp': 25.0, 'ki': 0.18, 'kd': 160.0}

    # Generate config snippet
    print(f"# PID Parameters ({recommended.upper() if recommended else 'DEFAULT'} tuning)")
    print(f"# Test quality: {results.get('test_quality', 'UNKNOWN')}")
    print(f"# Gain confidence: {thermal_model.get('gain_confidence', 'UNKNOWN')}")
    print()
    print(f"PID_KP_BASE = {pid['kp']:.3f}  # Base proportional gain")
    print(f"PID_KI_BASE = {pid['ki']:.4f}  # Base integral gain")
    print(f"PID_KD_BASE = {pid['kd']:.3f}  # Base derivative gain")
    print()
    print("# Continuous Gain Scheduling (compensates for heat loss at high temps)")
    print(f"THERMAL_H = {thermal_model.get('heat_loss_coefficient', 0.0001):.6f}  # Heat loss coefficient")
    print(f"THERMAL_T_AMBIENT = {thermal_model.get('ambient_temp', 25.0):.1f}  # Ambient temperature (°C)")

    print("-" * 80)

    # Print usage instructions
    print("\n" + "=" * 80)
    print("USAGE INSTRUCTIONS:")
    print("=" * 80)
    print("1. Copy the configuration parameters above")
    print("2. Paste into config.py (replacing the existing PID and THERMAL parameters)")
    print("3. Save config.py")
    print("4. Restart the kiln controller")
    print("5. The controller will now automatically adjust PID gains based on temperature")
    print("\nHOW IT WORKS:")
    print("- At low temps: Uses base PID gains (PID_KP_BASE, etc.)")
    print(f"- As temp increases: Gains scale up using: gain_scale(T) = 1 + h*(T - T_ambient)")
    print("- Scaling is continuous and smooth (no discrete jumps)")
    print("\nBENEFITS:")
    print("- Compensates for increased heat loss at higher temperatures")
    print("- Maintains consistent control performance across 0-1300°C range")
    print("- Much simpler than range-based scheduling")
    print("- More memory efficient (a few floats vs arrays)")
    print("- Physically accurate model of kiln heat loss")
    print("\nEXAMPLE:")
    h = thermal_model.get('heat_loss_coefficient', 0.0001)
    t_amb = thermal_model.get('ambient_temp', 25.0)
    kp_base = pid['kp']
    for temp in [100, 400, 700, 1000]:
        gain_scale = 1.0 + h * (temp - t_amb)
        kp_scaled = kp_base * gain_scale
        print(f"  At {temp:4}°C: Kp = {kp_scaled:.3f} (scale = {gain_scale:.3f}x)")
    print("=" * 80)
    print()
