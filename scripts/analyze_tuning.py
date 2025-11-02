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
import json
from pathlib import Path

# Import analyzer modules
from analyzer import (
    load_tuning_data,
    detect_phases,
    fit_thermal_model,
    calculate_all_pid_methods,
    calculate_temperature_range_pids,
    assess_test_quality,
    analyze_tuning_steps,
    generate_results_json,
    print_beautiful_report,
    generate_config_snippet
)


# =============================================================================
# Main Entry Point
# =============================================================================

def select_recommended_method(model, test_quality: str) -> str:
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

        # Calculate temperature-range-specific PIDs using gain scheduling
        print("📊 Analyzing temperature-range-specific parameters...")
        range_pids = calculate_temperature_range_pids(model, data)
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


if __name__ == "__main__":
    main()
