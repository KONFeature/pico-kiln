#!/usr/bin/env python3
"""
Generate THERMAL_MODEL config snippet from tuning results

This standalone script reads tuning_results.json and generates a
ready-to-paste THERMAL_MODEL configuration for config.py.

Usage:
    1. Run PID tuning analysis first:
       python analyze_tuning.py logs/tuning_YYYY-MM-DD_HH-MM-SS.csv

    2. Generate config snippet:
       python generate_thermal_model_config.py

    3. Copy the output and paste into config.py
"""

from analyze_tuning import generate_config_snippet

if __name__ == "__main__":
    generate_config_snippet()
