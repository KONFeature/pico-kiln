#!/usr/bin/env python3
"""
Tuning Phase Visualization (Enhanced with Physics-Based Detection)

Detailed visualization of PID tuning runs showing phase transitions,
step responses, plateaus, and SSR duty cycle behavior.

Now uses the analyzer module for superior physics-based phase detection:
- HEATING: SSR ≥ 5% AND temperature rising (>0.5°C/min)
- COOLING: SSR < 5% (natural cooling)
- PLATEAU: SSR ≥ 5% AND temperature stable (±0.5°C/min)

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
from pathlib import Path
import argparse

# Import analyzer modules for data loading and phase detection
from analyzer import load_tuning_data, detect_phases, Phase

try:
    import matplotlib.pyplot as plt
    from matplotlib.gridspec import GridSpec
    import matplotlib.patches as mpatches
except ImportError:
    print("\n❌ Error: matplotlib is required for plotting")
    print("Install it with: pip install matplotlib")
    sys.exit(1)


def calculate_heating_rate(data, phase):
    """
    Calculate heating/cooling rate for a phase

    Args:
        data: Dictionary with tuning data
        phase: Phase object from analyzer

    Returns:
        Heating rate in °C/minute
    """
    duration_s = data['time'][phase.end_idx] - data['time'][phase.start_idx]
    duration_min = duration_s / 60

    if duration_min <= 0:
        return 0

    temp_change = phase.temp_end - phase.temp_start
    return temp_change / duration_min


def plot_tuning_phases(data, phases, output_file=None):
    """
    Create detailed visualization of tuning phases with physics-based detection

    Args:
        data: Dictionary with tuning data from analyzer.load_tuning_data()
        phases: List of Phase objects from analyzer.detect_phases()
        output_file: Optional output file path (None = show interactive plot)
    """
    # Convert time to minutes for better readability
    time_minutes = [t / 60 for t in data['time']]

    # Phase color mapping (consistent with physics-based detection)
    phase_colors = {
        'heating': 'lightcoral',
        'cooling': 'lightblue',
        'plateau': 'lightyellow'
    }

    fig = plt.figure(figsize=(16, 10))
    gs = GridSpec(3, 1, height_ratios=[2, 1, 1], hspace=0.3)

    # Subplot 1: Temperature with phase backgrounds
    ax1 = fig.add_subplot(gs[0])

    # Draw phase backgrounds with physics-based type coloring
    for phase in phases:
        start_time = time_minutes[phase.start_idx]
        end_time = time_minutes[phase.end_idx]
        color = phase_colors.get(phase.phase_type, 'lightgray')

        ax1.axvspan(start_time, end_time, alpha=0.3, color=color)

    # Draw step transition lines if step data available
    if data.get('has_step_data', False):
        step_indices = data['step_indices']
        prev_step = -1
        for i, step_idx in enumerate(step_indices):
            if step_idx != prev_step and step_idx >= 0 and i > 0:
                ax1.axvline(x=time_minutes[i], color='gray', linestyle='--', alpha=0.5, linewidth=1.5)
                prev_step = step_idx

    # Plot temperature
    ax1.plot(time_minutes, data['temp'], 'b-', linewidth=2, label='Temperature')

    # Annotate each phase with enhanced info
    for phase in phases:
        start_time = time_minutes[phase.start_idx]
        end_time = time_minutes[phase.end_idx]
        mid_time = (start_time + end_time) / 2
        mid_temp = (phase.temp_start + phase.temp_end) / 2
        rate = calculate_heating_rate(data, phase)

        # Create enhanced phase label with type, SSR, and rate
        label = f"{phase.phase_type.upper()}\n{phase.avg_ssr:.0f}% SSR"
        if abs(rate) > 0.1:
            label += f"\n{rate:+.1f}°C/min"

        ax1.text(mid_time, mid_temp, label,
                horizontalalignment='center',
                verticalalignment='center',
                fontsize=8, weight='bold',
                bbox=dict(boxstyle='round', facecolor='white', alpha=0.9, edgecolor='gray'))

    ax1.set_ylabel('Temperature (°C)', fontsize=12)
    ax1.set_title('Tuning Phases - Physics-Based Detection', fontsize=14, fontweight='bold')
    ax1.grid(True, alpha=0.3)

    # Subplot 2: SSR Output
    ax2 = fig.add_subplot(gs[1], sharex=ax1)

    # Draw phase backgrounds
    for phase in phases:
        start_time = time_minutes[phase.start_idx]
        end_time = time_minutes[phase.end_idx]
        color = phase_colors.get(phase.phase_type, 'lightgray')
        ax2.axvspan(start_time, end_time, alpha=0.3, color=color)

    # Draw step transition lines if step data available
    if data.get('has_step_data', False):
        step_indices = data['step_indices']
        prev_step = -1
        for i, step_idx in enumerate(step_indices):
            if step_idx != prev_step and step_idx >= 0 and i > 0:
                ax2.axvline(x=time_minutes[i], color='gray', linestyle='--', alpha=0.5, linewidth=1.5)
                prev_step = step_idx

    ax2.fill_between(time_minutes, 0, data['ssr_output'],
                     alpha=0.5, color='orange')
    ax2.plot(time_minutes, data['ssr_output'],
            'orange', linewidth=1.5, label='SSR Output (%)')
    ax2.set_ylabel('SSR Output (%)', fontsize=12)
    ax2.set_ylim(-5, 105)
    ax2.grid(True, alpha=0.3)
    ax2.legend(loc='upper right', fontsize=10)

    # Subplot 3: Step Information (if available) or Phase Timeline
    ax3 = fig.add_subplot(gs[2], sharex=ax1)

    # Draw phase backgrounds
    for phase in phases:
        start_time = time_minutes[phase.start_idx]
        end_time = time_minutes[phase.end_idx]
        color = phase_colors.get(phase.phase_type, 'lightgray')
        ax3.axvspan(start_time, end_time, alpha=0.3, color=color)

    # If we have step names from tuning program, show them
    if data.get('has_step_data', False) and data.get('step_names'):
        step_indices = data['step_indices']
        step_names = data['step_names']
        prev_step = -1
        step_transitions = []

        # Collect step transitions
        for i, step_idx in enumerate(step_indices):
            if step_idx != prev_step and step_idx >= 0:
                if i < len(step_names) and step_names[i]:
                    step_transitions.append({
                        'idx': i,
                        'time': time_minutes[i],
                        'name': step_names[i],
                        'step_idx': step_idx
                    })
                prev_step = step_idx

        # Draw step regions with alternating colors
        for idx, trans in enumerate(step_transitions):
            start_time = trans['time']
            end_time = step_transitions[idx + 1]['time'] if idx + 1 < len(step_transitions) else time_minutes[-1]
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
        for phase in phases:
            start_time = time_minutes[phase.start_idx]
            end_time = time_minutes[phase.end_idx]
            mid_time = (start_time + end_time) / 2

            # Add detected type vs step name comparison if available
            label = phase.phase_type.upper()
            if phase.step_name:
                label += f"\n({phase.step_name})"

            ax3.text(mid_time, 0.5, label,
                    horizontalalignment='center', verticalalignment='center',
                    fontsize=9, weight='bold')
        ax3.set_ylabel('Phase Type', fontsize=12)
        ax3.set_ylim(0, 1)
        ax3.set_yticks([])

    ax3.set_xlabel('Time (minutes)', fontsize=12)
    ax3.grid(True, alpha=0.3, axis='x')

    # Add legend for phase types
    heating_patch = mpatches.Patch(color='lightcoral', alpha=0.3, label='Heating (SSR on, temp rising)')
    cooling_patch = mpatches.Patch(color='lightblue', alpha=0.3, label='Cooling (SSR off)')
    plateau_patch = mpatches.Patch(color='lightyellow', alpha=0.3, label='Plateau (SSR on, temp stable)')
    ax1.legend(handles=[heating_patch, cooling_patch, plateau_patch],
              loc='upper right', fontsize=9, title='Phase Types (Physics-Based)')

    # Add summary info
    duration = time_minutes[-1]
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
        print(f"✓ Enhanced tuning phase graph saved to: {output_file}")
    else:
        plt.show()


def print_phase_summary(phases, data):
    """
    Print detailed summary of detected phases with physics-based classification

    Args:
        phases: List of Phase objects from analyzer
        data: Dictionary with tuning data
    """
    print("\n" + "=" * 80)
    print("TUNING PHASE SUMMARY (Physics-Based Detection)")
    print("=" * 80)

    for i, phase in enumerate(phases, 1):
        start_time = data['time'][phase.start_idx] / 60
        end_time = data['time'][phase.end_idx] / 60
        duration = end_time - start_time
        rate = calculate_heating_rate(data, phase)

        print(f"\nPhase {i}: {phase.phase_type.upper()}")
        print(f"  Time:        {start_time:.1f} - {end_time:.1f} min ({duration:.1f} min)")
        print(f"  SSR Output:  {phase.avg_ssr:.1f}%")
        print(f"  Temperature: {phase.temp_start:.1f}°C → {phase.temp_end:.1f}°C (Δ{phase.temp_end - phase.temp_start:+.1f}°C)")
        print(f"  Rate:        {rate:+.2f}°C/min")

        # Show step name if available for comparison
        if phase.step_name:
            print(f"  Step Name:   {phase.step_name} (index {phase.step_index})")

    print("\n" + "=" * 80)
    print("\nPhase Classification Logic:")
    print("  COOLING: SSR < 5% (natural cooling, no heat input)")
    print("  HEATING: SSR ≥ 5% AND temp rising > 0.5°C/min")
    print("  PLATEAU: SSR ≥ 5% AND temp stable ±0.5°C/min")
    print("=" * 80)


def main():
    """Main entry point"""
    parser = argparse.ArgumentParser(
        description='Visualize tuning phases with physics-based detection',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  python plot_tuning_phases.py logs/tuning_2025-01-15.csv
  python plot_tuning_phases.py logs/tuning_2025-01-15.csv --output phases.png

Phase Detection:
  Uses physics-based measurement analysis from the analyzer module.
  Phases are classified based on actual SSR output and temperature behavior,
  not on step names or metadata, ensuring robust and accurate detection.
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
        # Load data using analyzer module
        data = load_tuning_data(args.csv_file)
        print(f"✓ Loaded {len(data['time']):,} data points")

        duration_min = data['time'][-1] / 60
        max_temp = max(data['temp'])
        min_temp = min(data['temp'])

        print(f"✓ Duration: {duration_min:.1f} minutes ({duration_min/60:.2f} hours)")
        print(f"✓ Temperature range: {min_temp:.1f}°C - {max_temp:.1f}°C")

        # Detect phases using physics-based algorithm from analyzer
        print(f"\n🔍 Detecting tuning phases (physics-based algorithm)...")
        phases = detect_phases(data)
        print(f"✓ Detected {len(phases)} phases")

        # Print phase summary
        print_phase_summary(phases, data)

        # Create enhanced plot
        print(f"\n📊 Generating enhanced phase visualization...")
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
