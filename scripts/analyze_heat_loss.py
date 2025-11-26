#!/usr/bin/env python3
"""
Heat Loss Analysis for Kiln Runs

Analyzes heat loss characteristics of a kiln by examining temperature drop
during periods when the kiln is at full power (100% SSR) or during cooling.

This helps understand:
- How much power is being lost to the environment at different temperatures
- Insulation effectiveness
- Energy efficiency at various temperature ranges

The analysis requires:
- Kiln volume in liters (to calculate thermal mass)
- Heating element power in watts

Usage:
    python analyze_heat_loss.py <csv_file> --volume <liters> --power <watts> [options]

Example:
    python analyze_heat_loss.py logs/firing.csv --volume 50 --power 5000
    python analyze_heat_loss.py logs/firing.csv -v 50 -p 5000 --output heat_loss_report.json
"""

import sys
import csv
import json
from pathlib import Path
from datetime import datetime
import argparse
from typing import List, Dict, Tuple


def load_run_data(csv_file):
    """
    Load kiln run data from CSV file
    
    Args:
        csv_file: Path to CSV file with run data
    
    Returns:
        Dictionary with time, temp, target_temp, ssr_output, state arrays
    """
    time_data = []
    temp_data = []
    target_temp_data = []
    ssr_output_data = []
    state_data = []
    timestamps = []
    
    with open(csv_file, 'r') as f:
        reader = csv.DictReader(f)
        
        for row in reader:
            # Skip RECOVERY state entries
            if row.get('state') == 'RECOVERY':
                continue
            
            elapsed = float(row['elapsed_seconds'])
            time_data.append(elapsed)
            temp_data.append(float(row['current_temp_c']))
            target_temp_data.append(float(row['target_temp_c']))
            ssr_output_data.append(float(row['ssr_output_percent']))
            state_data.append(row['state'])
            timestamps.append(row['timestamp'])
    
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
    
    return {
        'time': time_data,
        'temp': temp_data,
        'target_temp': target_temp_data,
        'ssr_output': ssr_output_data,
        'state': state_data,
        'timestamps': timestamps
    }


def find_full_power_periods(data, min_duration=60):
    """
    Find periods where the kiln is operating at 100% SSR power.
    
    Args:
        data: Dictionary with run data
        min_duration: Minimum duration (seconds) for a period to be considered
    
    Returns:
        List of tuples (start_idx, end_idx, avg_temp) for full power periods
    """
    time = data['time']
    temp = data['temp']
    ssr = data['ssr_output']
    
    periods = []
    in_full_power = False
    period_start_idx = None
    
    for i in range(len(ssr)):
        # Check if SSR is at 100% (or very close, accounting for rounding)
        is_full_power = ssr[i] >= 99.0
        
        if is_full_power and not in_full_power:
            # Start of a full power period
            period_start_idx = i
            in_full_power = True
        elif not is_full_power and in_full_power:
            # End of a full power period
            period_end_idx = i - 1
            if period_start_idx is not None:
                duration = time[period_end_idx] - time[period_start_idx]
                
                if duration >= min_duration:
                    # Calculate average temperature during this period
                    avg_temp = sum(temp[period_start_idx:period_end_idx+1]) / (period_end_idx - period_start_idx + 1)
                    periods.append((period_start_idx, period_end_idx, avg_temp))
            
            in_full_power = False
            period_start_idx = None
    
    # Handle case where data ends while still at full power
    if in_full_power and period_start_idx is not None:
        period_end_idx = len(ssr) - 1
        duration = time[period_end_idx] - time[period_start_idx]
        
        if duration >= min_duration:
            avg_temp = sum(temp[period_start_idx:period_end_idx+1]) / (period_end_idx - period_start_idx + 1)
            periods.append((period_start_idx, period_end_idx, avg_temp))
    
    return periods


def estimate_thermal_parameters(data, full_power_periods, cooling_periods, volume_liters, power_watts, ambient_temp):
    """
    Estimate thermal capacity and heat loss coefficient using both heating and cooling data.
    
    Physics equations:
    - Heating at full power: P_input = C × (dT/dt)_heat + k × (T - T_ambient)
    - Cooling (no power):    0 = C × (dT/dt)_cool + k × (T - T_ambient)
    
    From cooling: k = -C × (dT/dt)_cool / (T - T_ambient)
    
    Substitute into heating equation:
    P_input = C × (dT/dt)_heat - C × (dT/dt)_cool × (T - T_ambient) / (T - T_ambient)
    P_input = C × [(dT/dt)_heat - (dT/dt)_cool]
    
    Wait, that's wrong. Let me use a different approach:
    
    Better approach - Linear regression:
    At different temperatures, we have:
    P_input = C × (dT/dt) + k × (T - T_ambient)
    
    This is linear in C and k. We can solve using multiple data points.
    
    Args:
        data: Dictionary with run data
        full_power_periods: List of (start_idx, end_idx, avg_temp) tuples
        cooling_periods: List of (start_idx, end_idx, avg_temp) tuples
        volume_liters: Kiln volume
        power_watts: Input power
        ambient_temp: Ambient temperature
    
    Returns:
        Tuple of (thermal_capacity_J_per_C, heat_loss_coeff_W_per_C, estimation_quality)
    """
    time = data['time']
    temp = data['temp']
    
    # Collect data points: (T, dT/dt, P_input)
    # For full power: P_input = power_watts
    # For cooling: P_input = 0
    
    data_points = []
    
    # Add full power periods
    for start_idx, end_idx, avg_temp in full_power_periods:
        period_duration = time[end_idx] - time[start_idx]
        temp_change = temp[end_idx] - temp[start_idx]
        
        # Require longer periods (>2 min) and significant temp change for accurate rate measurement
        if period_duration > 120 and abs(temp_change) > 2:  # At least 2 minutes, 2°C change
            heating_rate = temp_change / period_duration  # °C/s
            temp_above_ambient = avg_temp - ambient_temp
            
            if temp_above_ambient > 50:  # Avoid low-temp periods with minimal heat loss
                data_points.append({
                    'temp_delta': temp_above_ambient,
                    'rate': heating_rate,
                    'power': power_watts,
                    'type': 'heating'
                })
    
    # Add cooling periods
    for start_idx, end_idx, avg_temp in cooling_periods:
        period_duration = time[end_idx] - time[start_idx]
        temp_change = temp[end_idx] - temp[start_idx]
        
        if period_duration > 60 and abs(temp_change) > 5:  # Significant cooling
            cooling_rate = temp_change / period_duration  # °C/s (negative)
            temp_above_ambient = avg_temp - ambient_temp
            
            if temp_above_ambient > 50:  # Need significant temp difference
                data_points.append({
                    'temp_delta': temp_above_ambient,
                    'rate': cooling_rate,
                    'power': 0,
                    'type': 'cooling'
                })
    
    if len(data_points) < 3:
        # Not enough data - use conservative default
        thermal_capacity = volume_liters * 2000  # J/°C
        heat_loss_coeff = 10.0  # W/°C (very rough estimate)
        return thermal_capacity, heat_loss_coeff, 'POOR (insufficient data)'
    
    # Solve using WEIGHTED linear regression: P = C × (dT/dt) + k × (T - T_ambient)
    # Weight data points by temperature^2 to emphasize high-temp data where heat loss dominates
    # This helps separate C from k more effectively
    
    # Build matrices for weighted least squares
    A = []  # Matrix of [rate, temp_delta]
    b = []  # Vector of power values
    weights = []  # Weight for each data point
    
    for point in data_points:
        # Weight by (temp_delta/500)^3 to strongly emphasize high temps
        # This helps constrain k more accurately since heat loss dominates at high temps
        # Using 500°C as normalization keeps weights in reasonable range
        weight = (point['temp_delta'] / 500) ** 3
        
        # Give cooling data 2x weight since it directly measures heat loss (no heating term to confuse things)
        if point['type'] == 'cooling':
            weight *= 2.0
        
        # Ensure minimum weight of 0.1 for stability
        weight = max(weight, 0.1)
        weights.append(weight)
        A.append([point['rate'], point['temp_delta']])
        b.append(point['power'])
    
    # Solve weighted least squares: [C, k] = (A^T W A)^-1 A^T W b
    # Where W is diagonal matrix of weights
    # Manual implementation for 2x2 case
    
    # Calculate A^T W A (weighted covariance)
    ATA_00 = sum(weights[i] * A[i][0] * A[i][0] for i in range(len(A)))
    ATA_01 = sum(weights[i] * A[i][0] * A[i][1] for i in range(len(A)))
    ATA_10 = ATA_01  # Symmetric
    ATA_11 = sum(weights[i] * A[i][1] * A[i][1] for i in range(len(A)))
    
    # Calculate A^T W b (weighted correlation)
    ATb_0 = sum(weights[i] * A[i][0] * b[i] for i in range(len(b)))
    ATb_1 = sum(weights[i] * A[i][1] * b[i] for i in range(len(b)))
    
    # Invert 2x2 matrix A^T W A
    det = ATA_00 * ATA_11 - ATA_01 * ATA_10
    
    if abs(det) < 1e-10:
        # Matrix is singular - use fallback
        thermal_capacity = volume_liters * 2000
        heat_loss_coeff = 10.0
        return thermal_capacity, heat_loss_coeff, 'POOR (singular matrix)'
    
    inv_00 = ATA_11 / det
    inv_01 = -ATA_01 / det
    inv_10 = -ATA_10 / det
    inv_11 = ATA_00 / det
    
    # Solve: [C, k] = inv(A^T A) × A^T b
    C = inv_00 * ATb_0 + inv_01 * ATb_1
    k = inv_10 * ATb_0 + inv_11 * ATb_1
    
    # Validate results
    if C < 10000 or C > 1000000:  # Unreasonable thermal capacity
        thermal_capacity = volume_liters * 2000
        estimation_quality = 'POOR (C out of range)'
    else:
        thermal_capacity = C
        estimation_quality = 'GOOD'
    
    if k < 0 or k > 100:  # Unreasonable heat loss coefficient
        heat_loss_coeff = 10.0
        if estimation_quality == 'GOOD':
            estimation_quality = 'FAIR (k out of range)'
    else:
        heat_loss_coeff = k
    
    # Assess quality based on number of data points and types
    heating_points = sum(1 for p in data_points if p['type'] == 'heating')
    cooling_points = sum(1 for p in data_points if p['type'] == 'cooling')
    
    if heating_points >= 3 and cooling_points >= 2:
        if estimation_quality == 'GOOD':
            estimation_quality = 'EXCELLENT'
    elif heating_points >= 2 or cooling_points >= 1:
        if estimation_quality == 'GOOD':
            estimation_quality = 'FAIR'
    
    # ADDITIONAL DIAGNOSTIC: Check if k is consistent across temperature ranges
    # Calculate k for each data point and check variation
    k_values = []
    for point in data_points:
        if point['type'] == 'heating':
            p_loss_measured = point['power'] - thermal_capacity * point['rate']
            k_individual = p_loss_measured / point['temp_delta']
            k_values.append(k_individual)
    
    if len(k_values) >= 3:
        k_std = (sum((kval - heat_loss_coeff) ** 2 for kval in k_values) / len(k_values)) ** 0.5
        k_variation_pct = (k_std / heat_loss_coeff * 100) if heat_loss_coeff > 0 else 100
        
        # If k varies by more than 20%, suggest heat loss may not be perfectly linear
        if k_variation_pct > 20:
            estimation_quality += f' (k varies {k_variation_pct:.0f}% across temps)'
    
    return thermal_capacity, heat_loss_coeff, estimation_quality


def analyze_heat_loss_at_full_power(data, periods, volume_liters, power_watts, thermal_capacity, heat_loss_coeff, ambient_temp):
    """
    Analyze heat loss during full power periods using estimated thermal parameters.
    
    At full power: Power_in = C × (dT/dt) + k × (T - T_ambient)
    Where:
      C = thermal capacity (J/°C)
      k = heat loss coefficient (W/°C)
      Power_loss = k × (T - T_ambient)
    
    Args:
        data: Dictionary with run data
        periods: List of (start_idx, end_idx, avg_temp) for full power periods
        volume_liters: Kiln volume in liters
        power_watts: Heating element power in watts
        thermal_capacity: Estimated thermal capacity (J/°C)
        heat_loss_coeff: Heat loss coefficient (W/°C)
        ambient_temp: Ambient temperature (°C)
    
    Returns:
        List of heat loss analysis results for each period
    """
    time = data['time']
    temp = data['temp']
    
    results = []
    
    for start_idx, end_idx, avg_temp in periods:
        period_duration = time[end_idx] - time[start_idx]
        temp_start = temp[start_idx]
        temp_end = temp[end_idx]
        temp_change = temp_end - temp_start
        
        # Calculate heating rate (°C/second and °C/hour)
        heating_rate_per_sec = temp_change / period_duration if period_duration > 0 else 0
        heating_rate_per_hour = heating_rate_per_sec * 3600
        
        # Power going to temperature increase (Watts)
        power_to_heating = thermal_capacity * heating_rate_per_sec
        
        # Power lost to environment (Watts) using heat loss coefficient
        temp_above_ambient = avg_temp - ambient_temp
        heat_loss_rate = heat_loss_coeff * temp_above_ambient
        
        # Verify power balance (should roughly equal input power)
        calculated_power = power_to_heating + heat_loss_rate
        power_balance_error = abs(calculated_power - power_watts) / power_watts * 100
        
        # Heat loss as percentage of input power
        heat_loss_percent = (heat_loss_rate / power_watts * 100) if power_watts > 0 else 0
        
        # Energy calculations for the period
        total_energy_input = power_watts * period_duration
        energy_to_temp_rise = power_to_heating * period_duration
        energy_lost = heat_loss_rate * period_duration
        
        results.append({
            'start_timestamp': data['timestamps'][start_idx],
            'end_timestamp': data['timestamps'][end_idx],
            'duration_minutes': round(period_duration / 60, 1),
            'avg_temp_c': round(avg_temp, 1),
            'temp_range_c': f"{round(temp_start, 1)}-{round(temp_end, 1)}",
            'heating_rate_c_per_hour': round(heating_rate_per_hour, 1),
            'total_energy_input_kj': round(total_energy_input / 1000, 1),
            'energy_to_heating_kj': round(energy_to_temp_rise / 1000, 1),
            'energy_lost_kj': round(energy_lost / 1000, 1),
            'heat_loss_rate_watts': round(heat_loss_rate, 0),
            'heat_loss_percent': round(heat_loss_percent, 1)
        })
    
    return results


def find_cooling_periods(data, min_duration=300, min_temp_drop=10):
    """
    Find cooling periods where SSR is off and temperature is dropping.
    
    Args:
        data: Dictionary with run data
        min_duration: Minimum duration (seconds)
        min_temp_drop: Minimum temperature drop (°C)
    
    Returns:
        List of tuples (start_idx, end_idx, avg_temp)
    """
    time = data['time']
    temp = data['temp']
    ssr = data['ssr_output']
    
    periods = []
    in_cooling = False
    period_start_idx = None
    
    for i in range(len(ssr)):
        # SSR is off (or nearly off)
        is_cooling = ssr[i] < 5.0
        
        if is_cooling and not in_cooling:
            # Start of cooling period
            period_start_idx = i
            in_cooling = True
        elif not is_cooling and in_cooling:
            # End of cooling period
            period_end_idx = i - 1
            if period_start_idx is not None:
                duration = time[period_end_idx] - time[period_start_idx]
                temp_drop = temp[period_start_idx] - temp[period_end_idx]
                
                if duration >= min_duration and temp_drop >= min_temp_drop:
                    avg_temp = sum(temp[period_start_idx:period_end_idx+1]) / (period_end_idx - period_start_idx + 1)
                    periods.append((period_start_idx, period_end_idx, avg_temp))
            
            in_cooling = False
            period_start_idx = None
    
    # Handle case where data ends while still cooling
    if in_cooling and period_start_idx is not None:
        period_end_idx = len(ssr) - 1
        duration = time[period_end_idx] - time[period_start_idx]
        temp_drop = temp[period_start_idx] - temp[period_end_idx]
        
        if duration >= min_duration and temp_drop >= min_temp_drop:
            avg_temp = sum(temp[period_start_idx:period_end_idx+1]) / (period_end_idx - period_start_idx + 1)
            periods.append((period_start_idx, period_end_idx, avg_temp))
    
    return periods


def analyze_cooling_heat_loss(data, periods, volume_liters, ambient_temp=25, thermal_capacity=None):
    """
    Analyze heat loss during cooling periods.
    
    During cooling with no heat input: Power_loss = (thermal_capacity) * (dT/dt)
    
    Args:
        data: Dictionary with run data
        periods: List of (start_idx, end_idx, avg_temp) for cooling periods
        volume_liters: Kiln volume in liters
        ambient_temp: Ambient temperature (°C)
        thermal_capacity: Estimated thermal capacity (J/°C), if known
    
    Returns:
        List of heat loss analysis results for each cooling period
    """
    time = data['time']
    temp = data['temp']
    
    # Estimate thermal capacity if not provided
    # Typical kilns: ~100-300 kJ/°C depending on wall thickness
    # Roughly 2000-3000 J/°C per liter of kiln volume
    if thermal_capacity is None:
        thermal_capacity = volume_liters * 2500  # J/°C
    
    results = []
    
    for start_idx, end_idx, avg_temp in periods:
        period_duration = time[end_idx] - time[start_idx]
        temp_start = temp[start_idx]
        temp_end = temp[end_idx]
        temp_drop = temp_start - temp_end
        
        # Calculate cooling rate (°C/second and °C/hour)
        cooling_rate_per_sec = temp_drop / period_duration if period_duration > 0 else 0
        cooling_rate_per_hour = cooling_rate_per_sec * 3600
        
        # Power being lost (Watts) = thermal_capacity * cooling_rate
        heat_loss_rate = thermal_capacity * cooling_rate_per_sec
        
        # Energy lost during this period
        energy_lost = heat_loss_rate * period_duration
        
        # Heat loss coefficient (W/°C) - how much power is lost per degree above ambient
        temp_above_ambient = avg_temp - ambient_temp
        heat_loss_coefficient = heat_loss_rate / temp_above_ambient if temp_above_ambient > 0 else 0
        
        results.append({
            'start_timestamp': data['timestamps'][start_idx],
            'end_timestamp': data['timestamps'][end_idx],
            'duration_minutes': round(period_duration / 60, 1),
            'avg_temp_c': round(avg_temp, 1),
            'temp_range_c': f"{round(temp_start, 1)}-{round(temp_end, 1)}",
            'temp_drop_c': round(temp_drop, 1),
            'cooling_rate_c_per_hour': round(cooling_rate_per_hour, 1),
            'energy_lost_kj': round(energy_lost / 1000, 1),
            'heat_loss_rate_watts': round(heat_loss_rate, 0),
            'temp_above_ambient_c': round(temp_above_ambient, 1),
            'heat_loss_coefficient_w_per_c': round(heat_loss_coefficient, 2)
        })
    
    return results


def analyze_heat_loss(data, volume_liters, power_watts, ambient_temp=25):
    """
    Comprehensive heat loss analysis.
    
    Args:
        data: Dictionary with run data
        volume_liters: Kiln volume in liters
        power_watts: Heating element power in watts
        ambient_temp: Ambient temperature (°C)
    
    Returns:
        Dictionary with complete analysis results
    """
    # Find full power and cooling periods
    full_power_periods = find_full_power_periods(data, min_duration=60)
    cooling_periods = find_cooling_periods(data, min_duration=300, min_temp_drop=10)
    
    # Estimate thermal capacity and heat loss coefficient using both heating and cooling data
    thermal_capacity, heat_loss_coeff, estimation_quality = estimate_thermal_parameters(
        data, full_power_periods, cooling_periods, volume_liters, power_watts, ambient_temp
    )
    
    # Analyze heat loss during full power periods
    if full_power_periods:
        full_power_analysis = analyze_heat_loss_at_full_power(
            data, full_power_periods, volume_liters, power_watts, 
            thermal_capacity, heat_loss_coeff, ambient_temp
        )
    else:
        full_power_analysis = []
    
    # Analyze heat loss during cooling periods
    if cooling_periods:
        cooling_analysis = analyze_cooling_heat_loss(
            data, cooling_periods, volume_liters, ambient_temp, thermal_capacity
        )
    else:
        cooling_analysis = []
    
    # Calculate summary statistics
    if full_power_analysis:
        avg_heat_loss_at_full_power = sum(p['heat_loss_rate_watts'] for p in full_power_analysis) / len(full_power_analysis)
        avg_heat_loss_percent = sum(p['heat_loss_percent'] for p in full_power_analysis) / len(full_power_analysis)
    else:
        avg_heat_loss_at_full_power = 0
        avg_heat_loss_percent = 0
    
    if cooling_analysis:
        avg_cooling_heat_loss = sum(p['heat_loss_rate_watts'] for p in cooling_analysis) / len(cooling_analysis)
    else:
        avg_cooling_heat_loss = 0
    
    return {
        'status': 'SUCCESS',
        'kiln_parameters': {
            'volume_liters': volume_liters,
            'power_watts': power_watts,
            'ambient_temp_c': ambient_temp,
            'estimated_thermal_capacity_kj_per_c': round(thermal_capacity / 1000, 1),
            'estimated_heat_loss_coeff_w_per_c': round(heat_loss_coeff, 2),
            'estimation_quality': estimation_quality
        },
        'summary': {
            'full_power_periods_found': len(full_power_analysis),
            'cooling_periods_found': len(cooling_analysis),
            'avg_heat_loss_at_full_power_watts': round(avg_heat_loss_at_full_power, 0),
            'avg_heat_loss_percent_of_input': round(avg_heat_loss_percent, 1),
            'avg_heat_loss_during_cooling_watts': round(avg_cooling_heat_loss, 0),
            'note': 'Heat loss increases with temperature (higher temps = more loss)'
        },
        'full_power_periods': full_power_analysis,
        'cooling_periods': cooling_analysis
    }


def format_console_output(results):
    """
    Format results for console output.
    
    Args:
        results: Analysis results dictionary
    """
    print("\n" + "="*70)
    print("HEAT LOSS ANALYSIS")
    print("="*70)
    
    # Kiln parameters
    params = results['kiln_parameters']
    print(f"\n⚙️  KILN PARAMETERS")
    print(f"   Volume: {params['volume_liters']} liters")
    print(f"   Heating Power: {params['power_watts']} watts")
    print(f"   Ambient Temperature: {params['ambient_temp_c']}°C")
    print(f"\n   Estimated Thermal Parameters:")
    print(f"   • Thermal Capacity: {params['estimated_thermal_capacity_kj_per_c']} kJ/°C")
    print(f"     (Includes kiln walls, shelves, furniture, and air)")
    print(f"   • Heat Loss Coefficient: {params['estimated_heat_loss_coeff_w_per_c']} W/°C")
    print(f"     (Power lost per degree above ambient)")
    print(f"   • Estimation Quality: {params['estimation_quality']}")
    
    # Summary
    summary = results['summary']
    print(f"\n📊 SUMMARY")
    print(f"   Full Power Periods Found: {summary['full_power_periods_found']}")
    print(f"   Cooling Periods Found: {summary['cooling_periods_found']}")
    
    if summary['full_power_periods_found'] > 0:
        print(f"\n   Average Heat Loss at Full Power:")
        print(f"      {summary['avg_heat_loss_at_full_power_watts']} watts ({summary['avg_heat_loss_percent_of_input']}% of input power)")
    
    if summary['cooling_periods_found'] > 0:
        print(f"\n   Average Heat Loss During Cooling:")
        print(f"      {summary['avg_heat_loss_during_cooling_watts']} watts")
    
    # Full power periods detail
    if results['full_power_periods']:
        print(f"\n🔥 FULL POWER PERIODS (100% SSR)")
        print(f"   Note: 'Balance' shows calculated power (heating + loss) vs input power")
        print(f"   {'-'*90}")
        print(f"   {'Period':<10} {'Temp':<12} {'Rate':<15} {'Duration':<12} {'Heat Loss':<20} {'Balance':<15}")
        print(f"   {'-'*90}")
        
        for i, period in enumerate(results['full_power_periods'], 1):
            temp_info = f"{period['avg_temp_c']}°C"
            rate_info = f"{period['heating_rate_c_per_hour']}°C/h"
            duration_info = f"{period['duration_minutes']} min"
            loss_info = f"{period['heat_loss_rate_watts']}W ({period['heat_loss_percent']}%)"
            
            # Calculate power balance check
            C = results['kiln_parameters']['estimated_thermal_capacity_kj_per_c'] * 1000  # Convert to J/°C
            k = results['kiln_parameters']['estimated_heat_loss_coeff_w_per_c']
            power_in = results['kiln_parameters']['power_watts']
            rate_per_sec = period['heating_rate_c_per_hour'] / 3600
            temp_delta = period['avg_temp_c'] - results['kiln_parameters']['ambient_temp_c']
            
            calculated_power = C * rate_per_sec + k * temp_delta
            error_pct = abs(calculated_power - power_in) / power_in * 100
            balance_info = f"{calculated_power:.0f}W ({error_pct:.1f}%)"
            
            print(f"   {i:<10} {temp_info:<12} {rate_info:<15} {duration_info:<12} {loss_info:<20} {balance_info:<15}")
    
    # Cooling periods detail
    if results['cooling_periods']:
        print(f"\n❄️  COOLING PERIODS (SSR Off)")
        print(f"   {'-'*68}")
        print(f"   {'Period':<20} {'Temp':<15} {'Duration':<12} {'Heat Loss':<20}")
        print(f"   {'-'*68}")
        
        for i, period in enumerate(results['cooling_periods'], 1):
            temp_info = f"{period['avg_temp_c']}°C"
            duration_info = f"{period['duration_minutes']} min"
            loss_info = f"{period['heat_loss_rate_watts']}W"
            print(f"   Period {i:<14} {temp_info:<15} {duration_info:<12} {loss_info:<20}")
    
    # Insights
    print(f"\n💡 INSIGHTS")
    
    print(f"   Heat loss increases with temperature (proportional to ΔT from ambient)")
    print(f"   At higher temperatures, more of your input power goes to compensating for")
    print(f"   heat loss rather than raising temperature.")
    
    if results['full_power_periods']:
        max_loss_period = max(results['full_power_periods'], key=lambda p: p['heat_loss_rate_watts'])
        min_loss_period = min(results['full_power_periods'], key=lambda p: p['heat_loss_rate_watts'])
        
        print(f"\n   Heat Loss Range at Full Power:")
        print(f"      Low temp ({min_loss_period['avg_temp_c']}°C): {min_loss_period['heat_loss_rate_watts']}W ({min_loss_period['heat_loss_percent']}%)")
        print(f"      High temp ({max_loss_period['avg_temp_c']}°C): {max_loss_period['heat_loss_rate_watts']}W ({max_loss_period['heat_loss_percent']}%)")
        
        if max_loss_period['heat_loss_percent'] > 70:
            print(f"\n   ⚠️  At high temps, over 70% of your power is going to heat loss!")
            print(f"      Consider improving insulation to reduce firing time and energy costs.")
    
    if results['cooling_periods']:
        print(f"\n   Cooling data confirms heat loss increases with temperature")
        print(f"   Better insulation would reduce losses and improve energy efficiency")
    
    print("\n" + "="*70 + "\n")


def main():
    """Main entry point"""
    parser = argparse.ArgumentParser(
        description='Analyze heat loss characteristics of kiln runs',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  python analyze_heat_loss.py logs/firing.csv --volume 50 --power 5000
  python analyze_heat_loss.py logs/firing.csv -v 50 -p 5000 --ambient 20
  python analyze_heat_loss.py logs/firing.csv -v 50 -p 5000 -o report.json
        """
    )
    parser.add_argument('csv_file', help='CSV file with kiln run data')
    parser.add_argument('--volume', '-v', type=float, required=True,
                       help='Kiln volume in liters')
    parser.add_argument('--power', '-p', type=float, required=True,
                       help='Heating element power in watts')
    parser.add_argument('--ambient', '-a', type=float, default=25,
                       help='Ambient temperature in °C (default: 25)')
    parser.add_argument('--output', '-o', help='Output JSON file path')
    
    args = parser.parse_args()
    
    # Validate inputs
    if args.volume <= 0:
        print(f"\n❌ Error: Volume must be positive")
        sys.exit(1)
    
    if args.power <= 0:
        print(f"\n❌ Error: Power must be positive")
        sys.exit(1)
    
    # Check if file exists
    if not Path(args.csv_file).exists():
        print(f"\n❌ Error: File not found: {args.csv_file}")
        sys.exit(1)
    
    print(f"\n📂 Loading data from: {args.csv_file}")
    
    try:
        # Load data
        data = load_run_data(args.csv_file)
        print(f"✓ Loaded {len(data['time']):,} data points")
        
        # Analyze heat loss
        print(f"🔍 Analyzing heat loss...")
        results = analyze_heat_loss(data, args.volume, args.power, args.ambient)
        
        # Display results
        format_console_output(results)
        
        # Save to JSON if requested
        if args.output:
            with open(args.output, 'w') as f:
                json.dump(results, f, indent=2)
            print(f"✓ Results saved to: {args.output}\n")
    
    except Exception as e:
        print(f"\n❌ Error: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)


if __name__ == "__main__":
    main()
