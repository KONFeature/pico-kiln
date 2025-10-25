#!/usr/bin/env python3
"""
Kiln Run Visualization

Generates comprehensive graphs from kiln firing or tuning CSV data.
Shows temperature curves, target temperature, SSR state, and duty cycle.

Usage:
    python plot_run.py <csv_file> [--output output.png]

Example:
    python plot_run.py logs/tuning_2025-01-15_14-30-00.csv
    python plot_run.py logs/cone6_firing_2025-01-15_14-30-00.csv --output firing_graph.png
"""

import sys
import csv
from pathlib import Path
from datetime import datetime
import argparse

try:
    import matplotlib.pyplot as plt
    import matplotlib.dates as mdates
    from matplotlib.gridspec import GridSpec
except ImportError:
    print("\n❌ Error: matplotlib is required for plotting")
    print("Install it with: pip install matplotlib")
    sys.exit(1)


def load_run_data(csv_file):
    """
    Load kiln run data from CSV file

    Args:
        csv_file: Path to CSV file with run data

    Returns:
        Dictionary with time, temp, target_temp, ssr_output, state, progress arrays
    """
    time_data = []
    temp_data = []
    target_temp_data = []
    ssr_output_data = []
    state_data = []
    progress_data = []
    timestamps = []
    step_names = []
    step_indices = []
    total_steps_data = []

    with open(csv_file, 'r') as f:
        reader = csv.DictReader(f)
        fieldnames = reader.fieldnames

        for row in reader:
            elapsed = float(row['elapsed_seconds'])
            time_data.append(elapsed)
            temp_data.append(float(row['current_temp_c']))
            target_temp_data.append(float(row['target_temp_c']))
            ssr_output_data.append(float(row['ssr_output_percent']))
            state_data.append(row['state'])
            progress_data.append(float(row['progress_percent']))
            timestamps.append(row['timestamp'])

            # Handle new optional columns (backward compatibility)
            if 'step_name' in fieldnames:
                step_names.append(row.get('step_name', ''))
            if 'step_index' in fieldnames:
                step_indices.append(int(row['step_index']) if row.get('step_index', '') else -1)
            if 'total_steps' in fieldnames:
                total_steps_data.append(int(row['total_steps']) if row.get('total_steps', '') else 0)

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

    # Convert elapsed seconds to hours for better readability
    time_hours = [t / 3600 for t in time_data]

    result = {
        'time': time_data,
        'time_hours': time_hours,
        'temp': temp_data,
        'target_temp': target_temp_data,
        'ssr_output': ssr_output_data,
        'state': state_data,
        'progress': progress_data,
        'timestamps': timestamps
    }

    # Add new columns if available
    if step_names:
        result['step_names'] = step_names
    if step_indices:
        result['step_indices'] = step_indices
    if total_steps_data:
        result['total_steps'] = total_steps_data

    return result


def detect_run_type(data):
    """
    Detect if this is a tuning run or a firing program

    Returns:
        'TUNING' or 'FIRING'
    """
    # Check if any state is TUNING
    if 'TUNING' in data['state']:
        return 'TUNING'
    return 'FIRING'


def plot_run(data, output_file=None):
    """
    Create comprehensive visualization of kiln run

    Args:
        data: Dictionary with run data from load_run_data()
        output_file: Optional output file path (None = show interactive plot)
    """
    run_type = detect_run_type(data)

    # Create figure with subplots
    fig = plt.figure(figsize=(14, 10))
    gs = GridSpec(3, 1, height_ratios=[2, 1, 1], hspace=0.3)

    # Subplot 1: Temperature vs Time
    ax1 = fig.add_subplot(gs[0])

    # Draw step transition lines if step data available
    if 'step_indices' in data and data['step_indices']:
        prev_step = -1
        for i, step_idx in enumerate(data['step_indices']):
            if step_idx != prev_step and step_idx >= 0 and i > 0:
                ax1.axvline(x=data['time_hours'][i], color='gray', linestyle='--', alpha=0.4, linewidth=1)
                prev_step = step_idx

    ax1.plot(data['time_hours'], data['temp'], 'b-', linewidth=2, label='Current Temp')
    ax1.plot(data['time_hours'], data['target_temp'], 'r--', linewidth=1.5, alpha=0.7, label='Target Temp')
    ax1.set_xlabel('Time (hours)', fontsize=12)
    ax1.set_ylabel('Temperature (°C)', fontsize=12)
    ax1.set_title(f'Kiln {run_type} - Temperature Profile', fontsize=14, fontweight='bold')
    ax1.grid(True, alpha=0.3)
    ax1.legend(loc='upper left', fontsize=10)

    # Add temperature range info
    max_temp = max(data['temp'])
    min_temp = min(data['temp'])
    duration = data['time_hours'][-1]
    ax1.text(0.98, 0.02,
             f"Duration: {duration:.2f}h\nMax Temp: {max_temp:.1f}°C\nMin Temp: {min_temp:.1f}°C",
             transform=ax1.transAxes,
             fontsize=9,
             verticalalignment='bottom',
             horizontalalignment='right',
             bbox=dict(boxstyle='round', facecolor='wheat', alpha=0.5))

    # Subplot 2: SSR Duty Cycle (%)
    ax2 = fig.add_subplot(gs[1], sharex=ax1)

    # Draw step transition lines if step data available
    if 'step_indices' in data and data['step_indices']:
        prev_step = -1
        for i, step_idx in enumerate(data['step_indices']):
            if step_idx != prev_step and step_idx >= 0 and i > 0:
                ax2.axvline(x=data['time_hours'][i], color='gray', linestyle='--', alpha=0.4, linewidth=1)
                prev_step = step_idx

    ax2.fill_between(data['time_hours'], 0, data['ssr_output'], alpha=0.3, color='orange')
    ax2.plot(data['time_hours'], data['ssr_output'], 'orange', linewidth=1, label='SSR Output (%)')
    ax2.set_ylabel('SSR Output (%)', fontsize=12)
    ax2.set_ylim(-5, 105)
    ax2.grid(True, alpha=0.3)
    ax2.legend(loc='upper right', fontsize=10)

    # Subplot 3: Progress / Step Information
    ax3 = fig.add_subplot(gs[2], sharex=ax1)

    # Show step boundaries if available
    if 'step_indices' in data and data['step_indices']:
        prev_step = -1
        step_transitions = []

        # Collect step transitions
        for i, step_idx in enumerate(data['step_indices']):
            if step_idx != prev_step and step_idx >= 0:
                step_name = ''
                if 'step_names' in data and data['step_names'] and i < len(data['step_names']):
                    step_name = data['step_names'][i]

                step_transitions.append({
                    'idx': i,
                    'time': data['time_hours'][i],
                    'name': step_name,
                    'step_idx': step_idx
                })
                prev_step = step_idx

        # Draw step regions with alternating colors
        for idx, trans in enumerate(step_transitions):
            start_time = trans['time']
            end_time = step_transitions[idx + 1]['time'] if idx + 1 < len(step_transitions) else data['time_hours'][-1]
            color = 'lightsteelblue' if idx % 2 == 0 else 'lavender'
            ax3.axvspan(start_time, end_time, alpha=0.4, color=color)

            # Add step label in the middle of the region if name available
            if trans['name']:
                mid_time = (start_time + end_time) / 2
                ax3.text(mid_time, 0.5, trans['name'],
                        horizontalalignment='center', verticalalignment='center',
                        fontsize=9, weight='bold', bbox=dict(boxstyle='round,pad=0.5',
                        facecolor='white', edgecolor='gray', alpha=0.8))

        # Draw vertical lines at transitions (except first)
        for trans in step_transitions[1:]:
            ax3.axvline(x=trans['time'], color='gray', linestyle='--', alpha=0.5, linewidth=1.5)

    # Plot progress
    ax3.plot(data['time_hours'], data['progress'], 'purple', linewidth=2, label='Progress (%)')
    ax3.set_xlabel('Time (hours)', fontsize=12)
    ax3.set_ylabel('Progress (%)', fontsize=12)
    ax3.set_ylim(-5, 105)
    ax3.grid(True, alpha=0.3)
    ax3.legend(loc='upper left', fontsize=10)

    # Add run info as title
    start_time = data['timestamps'][0]
    fig.suptitle(f'Run started: {start_time}', fontsize=10, y=0.995)

    plt.tight_layout()

    if output_file:
        plt.savefig(output_file, dpi=150, bbox_inches='tight')
        print(f"✓ Graph saved to: {output_file}")
    else:
        plt.show()


def main():
    """Main entry point"""
    parser = argparse.ArgumentParser(
        description='Visualize kiln firing or tuning run data',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  python plot_run.py logs/tuning_2025-01-15.csv
  python plot_run.py logs/cone6_firing.csv --output firing.png
        """
    )
    parser.add_argument('csv_file', help='CSV file with run data')
    parser.add_argument('--output', '-o', help='Output file path (default: show interactive plot)')

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

        run_type = detect_run_type(data)
        duration_hours = data['time_hours'][-1]
        max_temp = max(data['temp'])

        print(f"✓ Run type: {run_type}")
        print(f"✓ Duration: {duration_hours:.2f} hours")
        print(f"✓ Max temperature: {max_temp:.1f}°C")

        # Create plot
        print(f"📊 Generating graph...")
        plot_run(data, args.output)

        if not args.output:
            print("✓ Close the plot window to exit")

    except Exception as e:
        print(f"\n❌ Error: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)


if __name__ == "__main__":
    main()
