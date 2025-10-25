#!/usr/bin/env python3
"""
Tuning Phase Visualization

Detailed visualization of PID tuning runs showing phase transitions,
step responses, plateaus, and SSR duty cycle behavior.

Helps identify:
- Step response characteristics (dead time, time constant)
- Plateau detection quality
- Heating/cooling rates at different SSR levels
- Overall tuning sequence quality

Usage:
    python plot_tuning_phases.py <tuning_csv_file> [--output output.png]

Example:
    python plot_tuning_phases.py logs/tuning_2025-01-15_14-30-00.csv
    python plot_tuning_phases.py logs/tuning_2025-01-15_14-30-00.csv --output tuning_phases.png
"""

import sys
import csv
from pathlib import Path
from datetime import datetime
import argparse

try:
    import matplotlib.pyplot as plt
    from matplotlib.gridspec import GridSpec
    import matplotlib.patches as mpatches
except ImportError:
    print("\n❌ Error: matplotlib is required for plotting")
    print("Install it with: pip install matplotlib")
    sys.exit(1)


def load_tuning_data(csv_file):
    """
    Load tuning data from CSV file

    Args:
        csv_file: Path to CSV file with tuning data

    Returns:
        Dictionary with time, temp, ssr_output arrays and optional step info
    """
    time_data = []
    temp_data = []
    ssr_output_data = []
    timestamps = []
    step_names = []
    step_indices = []

    with open(csv_file, 'r') as f:
        reader = csv.DictReader(f)
        fieldnames = reader.fieldnames

        for row in reader:
            elapsed = float(row['elapsed_seconds'])
            time_data.append(elapsed)
            temp_data.append(float(row['current_temp_c']))
            ssr_output_data.append(float(row['ssr_output_percent']))
            timestamps.append(row['timestamp'])

            # Handle new optional columns for tuning runs
            if 'step_name' in fieldnames:
                step_names.append(row.get('step_name', ''))
            if 'step_index' in fieldnames:
                step_indices.append(int(row['step_index']) if row.get('step_index', '') else -1)

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

    # Convert to minutes for better readability
    time_minutes = [t / 60 for t in time_data]

    result = {
        'time': time_data,
        'time_minutes': time_minutes,
        'temp': temp_data,
        'ssr_output': ssr_output_data,
        'timestamps': timestamps
    }

    # Add step info if available
    if step_names:
        result['step_names'] = step_names
    if step_indices:
        result['step_indices'] = step_indices

    return result


def detect_phases(data, ssr_change_threshold=5):
    """
    Detect phase changes in tuning data based on SSR output changes

    Args:
        data: Dictionary with tuning data
        ssr_change_threshold: Minimum SSR change (%) to detect new phase

    Returns:
        List of phase dictionaries with start_idx, end_idx, avg_ssr, phase_type
    """
    phases = []
    ssr = data['ssr_output']
    temp = data['temp']
    time_min = data['time_minutes']

    if len(ssr) < 2:
        return phases

    current_ssr = ssr[0]
    phase_start_idx = 0

    for i in range(1, len(ssr)):
        # Detect significant SSR change
        if abs(ssr[i] - current_ssr) > ssr_change_threshold or i == len(ssr) - 1:
            end_idx = i if i < len(ssr) - 1 else len(ssr) - 1

            # Skip very short phases (< 2 minutes)
            duration = time_min[end_idx] - time_min[phase_start_idx]
            if duration < 2 and i < len(ssr) - 1:
                phase_start_idx = i
                current_ssr = ssr[i]
                continue

            # Calculate phase characteristics
            avg_ssr = sum(ssr[phase_start_idx:end_idx+1]) / (end_idx - phase_start_idx + 1)
            temp_start = temp[phase_start_idx]
            temp_end = temp[end_idx]
            temp_change = temp_end - temp_start

            # Classify phase
            if avg_ssr < 5:
                phase_type = 'cooling'
                color = 'lightblue'
            elif temp_change > 1:
                phase_type = 'heating'
                color = 'lightcoral'
            else:
                phase_type = 'plateau'
                color = 'lightyellow'

            phases.append({
                'start_idx': phase_start_idx,
                'end_idx': end_idx,
                'start_time': time_min[phase_start_idx],
                'end_time': time_min[end_idx],
                'avg_ssr': avg_ssr,
                'temp_start': temp_start,
                'temp_end': temp_end,
                'temp_change': temp_change,
                'phase_type': phase_type,
                'color': color
            })

            phase_start_idx = i
            current_ssr = ssr[i]

    return phases


def calculate_heating_rate(data, phase):
    """
    Calculate heating/cooling rate for a phase

    Args:
        data: Dictionary with tuning data
        phase: Phase dictionary

    Returns:
        Heating rate in °C/minute
    """
    duration_min = phase['end_time'] - phase['start_time']
    if duration_min <= 0:
        return 0

    temp_change = phase['temp_end'] - phase['temp_start']
    return temp_change / duration_min


def plot_tuning_phases(data, phases, output_file=None):
    """
    Create detailed visualization of tuning phases

    Args:
        data: Dictionary with tuning data
        phases: List of phase dictionaries
        output_file: Optional output file path (None = show interactive plot)
    """
    fig = plt.figure(figsize=(16, 10))
    gs = GridSpec(3, 1, height_ratios=[2, 1, 1], hspace=0.3)

    # Subplot 1: Temperature with phase backgrounds
    ax1 = fig.add_subplot(gs[0])

    # Draw phase backgrounds
    for phase in phases:
        ax1.axvspan(phase['start_time'], phase['end_time'],
                   alpha=0.3, color=phase['color'])

    # Draw step transition lines if step data available
    if 'step_indices' in data and data['step_indices']:
        prev_step = -1
        for i, step_idx in enumerate(data['step_indices']):
            if step_idx != prev_step and step_idx >= 0 and i > 0:
                time_min = data['time_minutes'][i]
                ax1.axvline(x=time_min, color='gray', linestyle='--', alpha=0.5, linewidth=1.5)
                prev_step = step_idx

    # Plot temperature
    ax1.plot(data['time_minutes'], data['temp'], 'b-', linewidth=2, label='Temperature')

    # Annotate each phase
    for i, phase in enumerate(phases):
        mid_time = (phase['start_time'] + phase['end_time']) / 2
        mid_temp = (phase['temp_start'] + phase['temp_end']) / 2
        rate = calculate_heating_rate(data, phase)

        # Phase label
        label = f"{phase['phase_type'].upper()}\n{phase['avg_ssr']:.0f}% SSR"
        if abs(rate) > 0.1:
            label += f"\n{rate:+.1f}°C/min"

        ax1.text(mid_time, mid_temp, label,
                horizontalalignment='center',
                verticalalignment='center',
                fontsize=8,
                bbox=dict(boxstyle='round', facecolor='white', alpha=0.8, edgecolor='gray'))

    ax1.set_ylabel('Temperature (°C)', fontsize=12)
    ax1.set_title('Tuning Phases - Temperature Response', fontsize=14, fontweight='bold')
    ax1.grid(True, alpha=0.3)
    ax1.legend(loc='upper left', fontsize=10)

    # Subplot 2: SSR Output
    ax2 = fig.add_subplot(gs[1], sharex=ax1)

    # Draw phase backgrounds
    for phase in phases:
        ax2.axvspan(phase['start_time'], phase['end_time'],
                   alpha=0.3, color=phase['color'])

    # Draw step transition lines if step data available
    if 'step_indices' in data and data['step_indices']:
        prev_step = -1
        for i, step_idx in enumerate(data['step_indices']):
            if step_idx != prev_step and step_idx >= 0 and i > 0:
                time_min = data['time_minutes'][i]
                ax2.axvline(x=time_min, color='gray', linestyle='--', alpha=0.5, linewidth=1.5)
                prev_step = step_idx

    ax2.fill_between(data['time_minutes'], 0, data['ssr_output'],
                     alpha=0.5, color='orange')
    ax2.plot(data['time_minutes'], data['ssr_output'],
            'orange', linewidth=1.5, label='SSR Output (%)')
    ax2.set_ylabel('SSR Output (%)', fontsize=12)
    ax2.set_ylim(-5, 105)
    ax2.grid(True, alpha=0.3)
    ax2.legend(loc='upper right', fontsize=10)

    # Subplot 3: Step Information (if available) or Phase Timeline
    ax3 = fig.add_subplot(gs[2], sharex=ax1)

    # Draw phase backgrounds
    for phase in phases:
        ax3.axvspan(phase['start_time'], phase['end_time'],
                   alpha=0.3, color=phase['color'])

    # If we have step names from tuning program, show them
    if 'step_names' in data and data['step_names'] and 'step_indices' in data:
        prev_step = -1
        step_transitions = []

        # Collect step transitions
        for i, step_idx in enumerate(data['step_indices']):
            if step_idx != prev_step and step_idx >= 0:
                if i < len(data['step_names']) and data['step_names'][i]:
                    step_transitions.append({
                        'idx': i,
                        'time': data['time_minutes'][i],
                        'name': data['step_names'][i],
                        'step_idx': step_idx
                    })
                prev_step = step_idx

        # Draw step regions with alternating colors
        for idx, trans in enumerate(step_transitions):
            start_time = trans['time']
            end_time = step_transitions[idx + 1]['time'] if idx + 1 < len(step_transitions) else data['time_minutes'][-1]
            color = 'lightsteelblue' if idx % 2 == 0 else 'lavender'
            ax3.axvspan(start_time, end_time, alpha=0.4, color=color)

            # Add step label in the middle of the region
            mid_time = (start_time + end_time) / 2
            ax3.text(mid_time, 0.5, trans['name'],
                    horizontalalignment='center', verticalalignment='center',
                    fontsize=9, weight='bold', bbox=dict(boxstyle='round,pad=0.5',
                    facecolor='white', edgecolor='gray', alpha=0.8))

        # Draw vertical lines at transitions
        for trans in step_transitions[1:]:  # Skip first transition
            ax3.axvline(x=trans['time'], color='gray', linestyle='--', alpha=0.5, linewidth=1.5)

        ax3.set_ylabel('Tuning Steps', fontsize=12)
        ax3.set_ylim(0, 1)
        ax3.set_yticks([])
    else:
        # Fallback: show phase type labels
        for i, phase in enumerate(phases):
            mid_time = (phase['start_time'] + phase['end_time']) / 2
            ax3.text(mid_time, 0.5, phase['phase_type'].upper(),
                    horizontalalignment='center', verticalalignment='center',
                    fontsize=9, weight='bold')
        ax3.set_ylabel('Phase Type', fontsize=12)
        ax3.set_ylim(0, 1)
        ax3.set_yticks([])

    ax3.set_xlabel('Time (minutes)', fontsize=12)
    ax3.grid(True, alpha=0.3, axis='x')

    # Add legend for phase types
    heating_patch = mpatches.Patch(color='lightcoral', alpha=0.3, label='Heating')
    cooling_patch = mpatches.Patch(color='lightblue', alpha=0.3, label='Cooling')
    plateau_patch = mpatches.Patch(color='lightyellow', alpha=0.3, label='Plateau')
    ax1.legend(handles=[heating_patch, cooling_patch, plateau_patch],
              loc='upper right', fontsize=9, title='Phase Types')

    # Add summary info
    duration = data['time_minutes'][-1]
    max_temp = max(data['temp'])
    min_temp = min(data['temp'])
    start_time = data['timestamps'][0]

    fig.suptitle(
        f'Tuning Run - {len(phases)} Phases | Duration: {duration:.1f}min | '
        f'Temp Range: {min_temp:.1f}°C - {max_temp:.1f}°C | Started: {start_time}',
        fontsize=11, y=0.995
    )

    plt.tight_layout()

    if output_file:
        plt.savefig(output_file, dpi=150, bbox_inches='tight')
        print(f"✓ Tuning phase graph saved to: {output_file}")
    else:
        plt.show()


def print_phase_summary(phases, data):
    """
    Print detailed summary of detected phases

    Args:
        phases: List of phase dictionaries
        data: Dictionary with tuning data
    """
    print("\n" + "=" * 80)
    print("TUNING PHASE SUMMARY")
    print("=" * 80)

    for i, phase in enumerate(phases, 1):
        duration = phase['end_time'] - phase['start_time']
        rate = calculate_heating_rate(data, phase)

        print(f"\nPhase {i}: {phase['phase_type'].upper()}")
        print(f"  Time:        {phase['start_time']:.1f} - {phase['end_time']:.1f} min ({duration:.1f} min)")
        print(f"  SSR Output:  {phase['avg_ssr']:.1f}%")
        print(f"  Temperature: {phase['temp_start']:.1f}°C → {phase['temp_end']:.1f}°C (Δ{phase['temp_change']:+.1f}°C)")
        print(f"  Rate:        {rate:+.2f}°C/min")

    print("\n" + "=" * 80)


def main():
    """Main entry point"""
    parser = argparse.ArgumentParser(
        description='Visualize tuning phases with detailed step response analysis',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  python plot_tuning_phases.py logs/tuning_2025-01-15.csv
  python plot_tuning_phases.py logs/tuning_2025-01-15.csv --output phases.png
        """
    )
    parser.add_argument('csv_file', help='CSV file with tuning data')
    parser.add_argument('--output', '-o', help='Output file path (default: show interactive plot)')

    args = parser.parse_args()

    # Check if file exists
    if not Path(args.csv_file).exists():
        print(f"\n❌ Error: File not found: {args.csv_file}")
        sys.exit(1)

    print(f"\n📂 Loading tuning data from: {args.csv_file}")

    try:
        # Load data
        data = load_tuning_data(args.csv_file)
        print(f"✓ Loaded {len(data['time']):,} data points")

        duration_min = data['time_minutes'][-1]
        max_temp = max(data['temp'])
        min_temp = min(data['temp'])

        print(f"✓ Duration: {duration_min:.1f} minutes ({duration_min/60:.2f} hours)")
        print(f"✓ Temperature range: {min_temp:.1f}°C - {max_temp:.1f}°C")

        # Detect phases
        print(f"\n🔍 Detecting tuning phases...")
        phases = detect_phases(data)
        print(f"✓ Detected {len(phases)} phases")

        # Print phase summary
        print_phase_summary(phases, data)

        # Create plot
        print(f"\n📊 Generating phase visualization...")
        plot_tuning_phases(data, phases, args.output)

        if not args.output:
            print("\n✓ Close the plot window to exit")

    except Exception as e:
        print(f"\n❌ Error: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)


if __name__ == "__main__":
    main()
