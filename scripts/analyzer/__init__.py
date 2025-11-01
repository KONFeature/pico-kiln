"""
Advanced Kiln Tuning Data Analyzer

Analyzes temperature data from kiln tuning runs and calculates optimal PID parameters
using multiple methods including thermal modeling and phase detection.

This package provides modular components for:
- Data loading and preprocessing
- Thermal model fitting
- PID parameter calculation
- Report generation
"""

from .data import load_tuning_data, Phase, detect_phases
from .thermal import ThermalModel, fit_thermal_model
from .pid import (
    PIDParams,
    calculate_ziegler_nichols,
    calculate_cohen_coon,
    calculate_amigo,
    calculate_lambda,
    calculate_all_pid_methods,
    calculate_temperature_range_pids
)
from .reporting import (
    assess_test_quality,
    analyze_tuning_steps,
    generate_results_json,
    print_beautiful_report,
    generate_config_snippet
)

__all__ = [
    # Data module
    'load_tuning_data',
    'Phase',
    'detect_phases',
    # Thermal module
    'ThermalModel',
    'fit_thermal_model',
    # PID module
    'PIDParams',
    'calculate_ziegler_nichols',
    'calculate_cohen_coon',
    'calculate_amigo',
    'calculate_lambda',
    'calculate_all_pid_methods',
    'calculate_temperature_range_pids',
    # Reporting module
    'assess_test_quality',
    'analyze_tuning_steps',
    'generate_results_json',
    'print_beautiful_report',
    'generate_config_snippet',
]
