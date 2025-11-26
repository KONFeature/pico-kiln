#!/usr/bin/env python3
"""
Final Climb Rate Analysis for Pottery Kiln Runs

Analyzes the last 100°C of a kiln run to calculate the heating rate,
which is critical for comparing against Orton cone charts.

For pottery firing, the last 100°C climb rate determines the cone equivalence.
This script:
- Identifies the maximum temperature reached
- Finds where the last 100°C climb started
- Calculates the time taken and heating rate (°C/hour)
- Handles hold periods (uses end of hold as reference)

Usage:
    python analyze_final_climb.py <csv_file> [--output output.json]

Example:
    python analyze_final_climb.py logs/cone6_firing_2025-01-15.csv
    python analyze_final_climb.py logs/bisque_firing.csv --output report.json
"""

import sys
import csv
import json
from pathlib import Path
from datetime import datetime
import argparse


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
    
    # Always calculate time from timestamps as well (for recovery scenarios)
    start_dt = datetime.strptime(timestamps[0], '%Y-%m-%d %H:%M:%S')
    time_from_timestamps = []
    for ts in timestamps:
        dt = datetime.strptime(ts, '%Y-%m-%d %H:%M:%S')
        elapsed = (dt - start_dt).total_seconds()
        time_from_timestamps.append(elapsed)
    
    # Check if elapsed_seconds is unreliable (all zeros or suspicious values)
    elapsed_unreliable = all(t == 0.0 for t in time_data) or time_data[-1] < time_from_timestamps[-1] * 0.5
    
    if elapsed_unreliable:
        print("\n⚠️  Warning: elapsed_seconds appears unreliable (possibly from recovery)")
        print("Will use timestamp-based calculations as primary method\n")
    
    return {
        'time': time_data,
        'time_from_timestamps': time_from_timestamps,
        'elapsed_unreliable': elapsed_unreliable,
        'temp': temp_data,
        'target_temp': target_temp_data,
        'ssr_output': ssr_output_data,
        'state': state_data,
        'timestamps': timestamps
    }


def detect_hold_period(data, temp_threshold=5.0, duration_threshold=300):
    """
    Detect if there's a hold period at the end of the run.
    
    A hold period is characterized by:
    - Temperature relatively stable (within threshold)
    - Duration of at least duration_threshold seconds
    - Near the maximum temperature
    
    Args:
        data: Dictionary with run data
        temp_threshold: Maximum temperature variation (°C) during hold
        duration_threshold: Minimum duration (seconds) to consider a hold
    
    Returns:
        Tuple of (hold_start_idx, hold_end_idx) or (None, None) if no hold detected
    """
    temp = data['temp']
    time = data['time']
    
    if len(temp) < 10:
        return None, None
    
    # Find the maximum temperature
    max_temp = max(temp)
    max_temp_idx = temp.index(max_temp)
    
    # Look backwards from max temp to find when temp stabilized
    hold_start_idx = None
    for i in range(max_temp_idx, 0, -1):
        # Check if temperature is within threshold of max
        if abs(temp[i] - max_temp) > temp_threshold:
            hold_start_idx = i + 1
            break
    
    # If we went all the way back, no hold detected
    if hold_start_idx is None or hold_start_idx >= max_temp_idx:
        return None, None
    
    # Look forward from max temp to find end of hold
    hold_end_idx = max_temp_idx
    for i in range(max_temp_idx + 1, len(temp)):
        # Check if temperature starts dropping significantly
        if temp[i] < max_temp - temp_threshold:
            hold_end_idx = i - 1
            break
        hold_end_idx = i
    
    # Check if hold duration is sufficient
    hold_duration = time[hold_end_idx] - time[hold_start_idx]
    if hold_duration < duration_threshold:
        return None, None
    
    return hold_start_idx, hold_end_idx


def analyze_final_climb(data, climb_degrees=100):
    """
    Analyze the final climb of the kiln run.
    Calculates rates using both elapsed_seconds and timestamps to handle recovery scenarios.
    
    Args:
        data: Dictionary with run data
        climb_degrees: Number of degrees to analyze (default: 100)
    
    Returns:
        Dictionary with analysis results
    """
    temp = data['temp']
    time_elapsed = data['time']
    time_from_ts = data['time_from_timestamps']
    timestamps = data['timestamps']
    elapsed_unreliable = data['elapsed_unreliable']
    
    # Use timestamp-based time if elapsed is unreliable, otherwise use elapsed
    time = time_from_ts if elapsed_unreliable else time_elapsed
    time_source = 'timestamps' if elapsed_unreliable else 'elapsed_seconds'
    
    # Detect hold period (using appropriate time source)
    # Temporarily set data['time'] to the correct source for hold detection
    original_time = data['time']
    data['time'] = time
    hold_start_idx, hold_end_idx = detect_hold_period(data)
    data['time'] = original_time
    
    # Determine reference point (end of hold if exists, otherwise max temp)
    if hold_end_idx is not None:
        reference_idx = hold_end_idx
        reference_temp = temp[reference_idx]
        reference_time_elapsed = time_elapsed[reference_idx]
        reference_time_ts = time_from_ts[reference_idx]
        reference_timestamp = timestamps[reference_idx]
        has_hold = True
        hold_duration_elapsed = time_elapsed[hold_end_idx] - time_elapsed[hold_start_idx]
        hold_duration_ts = time_from_ts[hold_end_idx] - time_from_ts[hold_start_idx]
    else:
        # No hold detected, use maximum temperature point
        reference_temp = max(temp)
        reference_idx = temp.index(reference_temp)
        reference_time_elapsed = time_elapsed[reference_idx]
        reference_time_ts = time_from_ts[reference_idx]
        reference_timestamp = timestamps[reference_idx]
        has_hold = False
        hold_duration_elapsed = 0
        hold_duration_ts = 0
    
    # Calculate target temperature for start of final climb
    climb_start_temp = reference_temp - climb_degrees
    
    # Find where the climb started (closest point to target temp going backwards)
    climb_start_idx = None
    for i in range(reference_idx, -1, -1):
        if temp[i] <= climb_start_temp:
            climb_start_idx = i
            break
    
    if climb_start_idx is None:
        # Kiln never reached the starting point of the climb
        actual_climb = reference_temp - temp[0]
        return {
            'status': 'INCOMPLETE',
            'message': f'Run did not reach {climb_degrees}°C below max temp',
            'max_temp': round(reference_temp, 1),
            'actual_climb': round(actual_climb, 1),
            'requested_climb': climb_degrees
        }
    
    # Calculate climb characteristics using BOTH time sources
    climb_start_temp_actual = temp[climb_start_idx]
    climb_start_time_elapsed = time_elapsed[climb_start_idx]
    climb_start_time_ts = time_from_ts[climb_start_idx]
    climb_start_timestamp = timestamps[climb_start_idx]
    
    actual_climb = reference_temp - climb_start_temp_actual
    
    # Calculate using elapsed_seconds
    climb_duration_elapsed = reference_time_elapsed - climb_start_time_elapsed
    climb_duration_hours_elapsed = climb_duration_elapsed / 3600.0
    climb_duration_minutes_elapsed = climb_duration_elapsed / 60.0
    heating_rate_elapsed = (actual_climb / climb_duration_hours_elapsed) if climb_duration_elapsed > 0 else 0
    
    # Calculate using timestamps
    climb_duration_ts = reference_time_ts - climb_start_time_ts
    climb_duration_hours_ts = climb_duration_ts / 3600.0
    climb_duration_minutes_ts = climb_duration_ts / 60.0
    heating_rate_ts = (actual_climb / climb_duration_hours_ts) if climb_duration_ts > 0 else 0
    
    # Choose primary values based on reliability
    if elapsed_unreliable:
        primary_duration_hours = climb_duration_hours_ts
        primary_duration_minutes = climb_duration_minutes_ts
        primary_heating_rate = heating_rate_ts
        primary_hold_duration = hold_duration_ts
        primary_total_duration = reference_time_ts
    else:
        primary_duration_hours = climb_duration_hours_elapsed
        primary_duration_minutes = climb_duration_minutes_elapsed
        primary_heating_rate = heating_rate_elapsed
        primary_hold_duration = hold_duration_elapsed
        primary_total_duration = reference_time_elapsed
    
    # Prepare results with BOTH calculations
    results = {
        'status': 'SUCCESS',
        'time_source': {
            'primary': time_source,
            'elapsed_unreliable': elapsed_unreliable,
            'note': 'Primary values use ' + time_source + (' (elapsed_seconds was unreliable)' if elapsed_unreliable else '')
        },
        'run_info': {
            'start_timestamp': timestamps[0],
            'end_timestamp': reference_timestamp,
            'total_duration_hours': round(primary_total_duration / 3600.0, 2),
            'max_temp_c': round(reference_temp, 1)
        },
        'hold_period': {
            'detected': has_hold,
            'duration_minutes': round(primary_hold_duration / 60.0, 1) if has_hold else 0,
            'start_timestamp': timestamps[hold_start_idx] if has_hold else None,
            'end_timestamp': timestamps[hold_end_idx] if has_hold else None
        },
        'final_climb': {
            'requested_climb_c': climb_degrees,
            'actual_climb_c': round(actual_climb, 1),
            'start_temp_c': round(climb_start_temp_actual, 1),
            'end_temp_c': round(reference_temp, 1),
            'start_timestamp': climb_start_timestamp,
            'end_timestamp': reference_timestamp,
            'duration_hours': round(primary_duration_hours, 2),
            'duration_minutes': round(primary_duration_minutes, 1),
            'heating_rate_c_per_hour': round(primary_heating_rate, 1)
        },
        'final_climb_elapsed_seconds': {
            'duration_hours': round(climb_duration_hours_elapsed, 2),
            'duration_minutes': round(climb_duration_minutes_elapsed, 1),
            'heating_rate_c_per_hour': round(heating_rate_elapsed, 1)
        },
        'final_climb_from_timestamps': {
            'duration_hours': round(climb_duration_hours_ts, 2),
            'duration_minutes': round(climb_duration_minutes_ts, 1),
            'heating_rate_c_per_hour': round(heating_rate_ts, 1)
        },
        'orton_reference': {
            'note': 'Compare heating rate against Orton cone charts',
            'heating_rate_c_per_hour': round(primary_heating_rate, 1),
            'typical_ranges': {
                'slow': '0-50 C/hour',
                'medium': '50-150 C/hour', 
                'fast': '150-300 C/hour'
            }
        }
    }
    
    return results


def format_console_output(results):
    """
    Format results for console output.
    
    Args:
        results: Analysis results dictionary
    """
    if results['status'] == 'INCOMPLETE':
        print(f"\n⚠️  {results['message']}")
        print(f"   Max temp reached: {results['max_temp']}°C")
        print(f"   Actual climb from start: {results['actual_climb']}°C")
        print(f"   Requested climb: {results['requested_climb']}°C")
        return
    
    print("\n" + "="*70)
    print("FINAL CLIMB RATE ANALYSIS FOR POTTERY FIRING")
    print("="*70)
    
    # Time source info
    time_src = results['time_source']
    if time_src['elapsed_unreliable']:
        print(f"\n⚠️  TIME SOURCE: {time_src['primary'].upper()}")
        print(f"   {time_src['note']}")
    
    # Run info
    run_info = results['run_info']
    print(f"\n📊 RUN INFORMATION")
    print(f"   Start: {run_info['start_timestamp']}")
    print(f"   End:   {run_info['end_timestamp']}")
    print(f"   Total Duration: {run_info['total_duration_hours']} hours")
    print(f"   Max Temperature: {run_info['max_temp_c']}°C")
    
    # Hold period
    hold = results['hold_period']
    if hold['detected']:
        print(f"\n⏱️  HOLD PERIOD DETECTED")
        print(f"   Duration: {hold['duration_minutes']} minutes")
        print(f"   Start: {hold['start_timestamp']}")
        print(f"   End:   {hold['end_timestamp']}")
    else:
        print(f"\n⏱️  NO HOLD PERIOD DETECTED")
    
    # Final climb analysis
    climb = results['final_climb']
    print(f"\n🔥 FINAL CLIMB ANALYSIS (Last {climb['requested_climb_c']}°C)")
    print(f"   Temperature Range: {climb['start_temp_c']}°C → {climb['end_temp_c']}°C")
    print(f"   Actual Climb: {climb['actual_climb_c']}°C")
    print(f"   Start Time: {climb['start_timestamp']}")
    print(f"   End Time:   {climb['end_timestamp']}")
    print(f"   Duration: {climb['duration_hours']} hours ({climb['duration_minutes']} minutes)")
    
    print(f"\n🎯 HEATING RATE (PRIMARY - from {time_src['primary']})")
    print(f"   {climb['heating_rate_c_per_hour']} °C/hour")
    
    # Show both calculations if different
    climb_elapsed = results['final_climb_elapsed_seconds']
    climb_ts = results['final_climb_from_timestamps']
    if abs(climb_elapsed['heating_rate_c_per_hour'] - climb_ts['heating_rate_c_per_hour']) > 1.0:
        print(f"\n📊 COMPARISON (both calculation methods)")
        print(f"   From elapsed_seconds:  {climb_elapsed['heating_rate_c_per_hour']} °C/hour ({climb_elapsed['duration_hours']} hours)")
        print(f"   From timestamps:       {climb_ts['heating_rate_c_per_hour']} °C/hour ({climb_ts['duration_hours']} hours)")
        if time_src['elapsed_unreliable']:
            print(f"   💡 Using timestamp-based calculation (more reliable for recovery runs)")
    
    # Orton reference
    orton = results['orton_reference']
    print(f"\n📖 ORTON CONE CHART REFERENCE")
    print(f"   Your heating rate: {orton['heating_rate_c_per_hour']} °C/hour")
    print(f"   Typical ranges:")
    print(f"      Slow:   {orton['typical_ranges']['slow']}")
    print(f"      Medium: {orton['typical_ranges']['medium']}")
    print(f"      Fast:   {orton['typical_ranges']['fast']}")
    print(f"\n   💡 Use this rate to select the correct Orton cone chart column")
    
    print("\n" + "="*70 + "\n")


def main():
    """Main entry point"""
    parser = argparse.ArgumentParser(
        description='Analyze final climb rate for pottery kiln runs',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  python analyze_final_climb.py logs/cone6_firing.csv
  python analyze_final_climb.py logs/bisque.csv --climb 120
  python analyze_final_climb.py logs/glaze.csv --output report.json
        """
    )
    parser.add_argument('csv_file', help='CSV file with kiln run data')
    parser.add_argument('--climb', '-c', type=float, default=100,
                       help='Number of degrees to analyze (default: 100)')
    parser.add_argument('--output', '-o', help='Output JSON file path')
    
    args = parser.parse_args()
    
    # Check if file exists
    if not Path(args.csv_file).exists():
        print(f"\n❌ Error: File not found: {args.csv_file}")
        sys.exit(1)
    
    print(f"\n📂 Loading data from: {args.csv_file}")
    
    try:
        # Load data
        data = load_run_data(args.csv_file)
        print(f"✓ Loaded {len(data['time']):,} data points")
        
        # Analyze final climb
        print(f"🔍 Analyzing final {args.climb}°C climb...")
        results = analyze_final_climb(data, climb_degrees=args.climb)
        
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
