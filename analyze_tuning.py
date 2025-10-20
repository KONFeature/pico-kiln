#!/usr/bin/env python3
"""
Analyze Tuning Data - Ziegler-Nichols PID Calculator

This script analyzes temperature data from a kiln tuning run and calculates
optimal PID parameters using the Ziegler-Nichols open-loop method.

Usage:
    python analyze_tuning.py logs/tuning_2025-01-15_14-30-00.csv

The script will:
1. Load the tuning CSV data
2. Analyze the heating curve to extract system characteristics
3. Calculate PID parameters (Kp, Ki, Kd) using Z-N formulas
4. Display the results and save to tuning_results.json

References:
- "Ziegler–Nichols Tuning Method" by Vishakha Vijay Patel
- https://github.com/jbruce12000/kiln-controller/blob/master/kiln-tuner.py
"""

import sys
import csv
import json
from pathlib import Path


def load_tuning_data(csv_file):
    """
    Load tuning data from CSV file

    Args:
        csv_file: Path to CSV file with tuning data

    Returns:
        Tuple of (time_data, temp_data) lists
    """
    time_data = []
    temp_data = []

    with open(csv_file, 'r') as f:
        reader = csv.DictReader(f)
        for row in reader:
            time_data.append(float(row['elapsed_seconds']))
            temp_data.append(float(row['current_temp_c']))

    return time_data, temp_data


def calculate_pid_parameters(time_data, temp_data, tangent_divisor=8.0):
    """
    Calculate PID parameters using Ziegler-Nichols method

    Args:
        time_data: List of time values (seconds)
        temp_data: List of temperature values (°C)
        tangent_divisor: Divisor for tangent line calculation (default: 8.0)

    Returns:
        Dictionary with PID parameters and analysis data

    Raises:
        Exception: If data is insufficient or calculation fails
    """
    if len(time_data) < 10:
        raise Exception("Insufficient data points for calculation (need at least 10)")

    # Find min and max temperatures
    min_temp = min(temp_data)
    max_temp = max(temp_data)
    mid_temp = (max_temp + min_temp) / 2.0

    print(f"\nData Analysis:")
    print(f"  Data points: {len(time_data)}")
    print(f"  Min temp: {min_temp:.1f}°C")
    print(f"  Max temp: {max_temp:.1f}°C")
    print(f"  Mid temp: {mid_temp:.1f}°C")

    # Find points for tangent line using divisor method
    # This selects a linear region around the midpoint of the curve
    y_offset = (max_temp - min_temp) / tangent_divisor

    tangent_min_point = None
    tangent_max_point = None

    for i in range(len(temp_data)):
        temp = temp_data[i]

        if temp >= (mid_temp - y_offset) and tangent_min_point is None:
            tangent_min_point = (time_data[i], temp)
        elif temp >= (mid_temp + y_offset) and tangent_max_point is None:
            tangent_max_point = (time_data[i], temp)
            break

    if tangent_min_point is None or tangent_max_point is None:
        raise Exception("Could not find suitable points for tangent line")

    print(f"\nTangent Line Analysis:")
    print(f"  Tangent min point: t={tangent_min_point[0]:.1f}s, T={tangent_min_point[1]:.1f}°C")
    print(f"  Tangent max point: t={tangent_max_point[0]:.1f}s, T={tangent_max_point[1]:.1f}°C")

    # Calculate tangent line: y = slope * x + offset
    slope = (tangent_max_point[1] - tangent_min_point[1]) / (tangent_max_point[0] - tangent_min_point[0])
    offset = tangent_min_point[1] - (slope * tangent_min_point[0])

    print(f"  Slope: {slope:.4f}°C/s")
    print(f"  Offset: {offset:.2f}°C")

    # Find where tangent line crosses min and max temperatures
    lower_crossing_time = (min_temp - offset) / slope
    upper_crossing_time = (max_temp - offset) / slope

    # Calculate Ziegler-Nichols parameters
    # L = dead time (delay before response starts)
    # T = time constant (time for response to complete)
    L = lower_crossing_time - time_data[0]
    T = upper_crossing_time - lower_crossing_time

    if L <= 0 or T <= 0:
        raise Exception(f"Invalid parameters: L={L:.2f}, T={T:.2f}")

    print(f"\nSystem Characteristics:")
    print(f"  Dead time (L): {L:.1f}s")
    print(f"  Time constant (T): {T:.1f}s")
    print(f"  L/T ratio: {L/T:.3f}")

    # Ziegler-Nichols PID tuning formulas (classic method)
    Kp = 1.2 * (T / L)
    Ti = 2.0 * L
    Td = 0.5 * L
    Ki = Kp / Ti
    Kd = Kp * Td

    # Build results dictionary
    results = {
        'kp': round(Kp, 2),
        'ki': round(Ki, 2),
        'kd': round(Kd, 2),
        'L': round(L, 2),
        'T': round(T, 2),
        'min_temp': min_temp,
        'max_temp': max_temp,
        'duration': time_data[-1] - time_data[0],
        'data_points': len(time_data),
        'tangent_slope': slope,
        'tangent_offset': offset,
        'tangent_divisor': tangent_divisor
    }

    return results


def save_results(results, output_file="tuning_results.json"):
    """
    Save tuning results to JSON file

    Args:
        results: Dictionary with PID parameters
        output_file: Output filename
    """
    with open(output_file, 'w') as f:
        json.dump(results, f, indent=2)
    print(f"\nResults saved to: {output_file}")


def main():
    """Main entry point"""
    print("=" * 60)
    print("Kiln Tuning Data Analyzer - Ziegler-Nichols Method")
    print("=" * 60)

    # Check command line arguments
    if len(sys.argv) < 2:
        print("\nUsage: python analyze_tuning.py <tuning_csv_file>")
        print("\nExample: python analyze_tuning.py logs/tuning_2025-01-15_14-30-00.csv")
        sys.exit(1)

    csv_file = sys.argv[1]

    # Check if file exists
    if not Path(csv_file).exists():
        print(f"\nError: File not found: {csv_file}")
        sys.exit(1)

    print(f"\nLoading data from: {csv_file}")

    try:
        # Load data
        time_data, temp_data = load_tuning_data(csv_file)

        # Calculate PID parameters
        results = calculate_pid_parameters(time_data, temp_data)

        # Display results
        print("\n" + "=" * 60)
        print("CALCULATED PID PARAMETERS (Ziegler-Nichols)")
        print("=" * 60)
        print(f"  Kp (Proportional):  {results['kp']:.2f}")
        print(f"  Ki (Integral):      {results['ki']:.2f}")
        print(f"  Kd (Derivative):    {results['kd']:.2f}")
        print("=" * 60)

        print("\nNext steps:")
        print("1. Update config.py with these values:")
        print(f"   PID_KP = {results['kp']:.1f}")
        print(f"   PID_KI = {results['ki']:.1f}")
        print(f"   PID_KD = {results['kd']:.1f}")
        print("2. Restart the kiln controller")
        print("3. Test with a firing profile")

        # Save results
        save_results(results)

    except Exception as e:
        print(f"\nError: {e}")
        sys.exit(1)


if __name__ == "__main__":
    main()
