#!/usr/bin/env python3
"""
Kiln Run Comparison

Compares multiple kiln firing or tuning runs side-by-side.
Useful for evaluating different PID settings, profiles, or tuning modes.

Usage:
    python compare_runs.py <csv_file1> <csv_file2> [csv_file3 ...] [--output output.png]

Example:
    python compare_runs.py logs/run1.csv logs/run2.csv logs/run3.csv
    python compare_runs.py logs/old_pid.csv logs/new_pid.csv --output pid_comparison.png
"""

import sys
import csv
from pathlib import Path
import argparse

try:
    import matplotlib.pyplot as plt
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
        Dictionary with time, temp, target_temp, ssr_output arrays
    """
    time_data = []
    temp_data = []
    target_temp_data = []
    ssr_output_data = []
    timestamps = []
    current_rate_data = []

    with open(csv_file, 'r') as f:
        reader = csv.DictReader(f)
        fieldnames = reader.fieldnames or []

        for row in reader:
            # Skip RECOVERY state entries
            if row.get('state') == 'RECOVERY':
                continue

            elapsed = float(row['elapsed_seconds'])
            time_data.append(elapsed)
            temp_data.append(float(row['current_temp_c']))
            target_temp_data.append(float(row['target_temp_c']))
            ssr_output_data.append(float(row['ssr_output_percent']))
            timestamps.append(row['timestamp'])

            # Handle new optional columns (backward compatibility)
            if 'current_rate_c_per_hour' in fieldnames:
                current_rate_data.append(float(row.get('current_rate_c_per_hour', 0)))

    # Convert elapsed seconds to hours for better readability
    time_hours = [t / 3600 for t in time_data]

    result = {
        'time': time_data,
        'time_hours': time_hours,
        'temp': temp_data,
        'target_temp': target_temp_data,
        'ssr_output': ssr_output_data,
        'timestamps': timestamps,
        'filename': Path(csv_file).stem  # Use filename as label
    }

    # Add rate data if available
    if current_rate_data:
        result['current_rate'] = current_rate_data

    return result


def calculate_metrics(data):
    """
    Calculate performance metrics for a run

    Args:
        data: Dictionary with run data

    Returns:
        Dictionary with metrics
    """
    # Calculate overshoot
    max_temp = max(data['temp'])
    max_target = max(data['target_temp'])
    overshoot = max_temp - max_target if max_temp > max_target else 0

    # Calculate temperature tracking error (mean absolute error)
    errors = [abs(current - target) for current, target in zip(data['temp'], data['target_temp'])]
    mean_error = sum(errors) / len(errors) if errors else 0
    max_error = max(errors) if errors else 0

    # Duration
    duration = data['time_hours'][-1]

    # Average SSR usage
    avg_ssr = sum(data['ssr_output']) / len(data['ssr_output']) if data['ssr_output'] else 0

    return {
        'max_temp': max_temp,
        'overshoot': overshoot,
        'mean_error': mean_error,
        'max_error': max_error,
        'duration': duration,
        'avg_ssr': avg_ssr
    }


def compare_runs(datasets, output_file=None):
    """
    Create comparison visualization of multiple runs

    Args:
        datasets: List of data dictionaries from load_run_data()
        output_file: Optional output file path (None = show interactive plot)
    """
    # Generate colors for each run
    colors = plt.cm.tab10(range(len(datasets)))

    # Check if any dataset has rate data
    has_rate_data = any('current_rate' in d and d['current_rate'] for d in datasets)

    # Create figure with subplots - add rate row if data available
    if has_rate_data:
        fig = plt.figure(figsize=(16, 13))
        gs = GridSpec(3, 2, height_ratios=[2, 1, 1], width_ratios=[3, 1], hspace=0.3, wspace=0.3)
    else:
        fig = plt.figure(figsize=(16, 10))
        gs = GridSpec(2, 2, height_ratios=[2, 1], width_ratios=[3, 1], hspace=0.3, wspace=0.3)

    # Subplot 1: Temperature Comparison
    ax1 = fig.add_subplot(gs[0, 0])
    for i, data in enumerate(datasets):
        ax1.plot(data['time_hours'], data['temp'],
                linewidth=2, label=data['filename'], color=colors[i])

    # Overlay target from first run (assuming same profile)
    ax1.plot(datasets[0]['time_hours'], datasets[0]['target_temp'],
            'k--', linewidth=1, alpha=0.4, label='Target')

    ax1.set_xlabel('Time (hours)', fontsize=12)
    ax1.set_ylabel('Temperature (°C)', fontsize=12)
    ax1.set_title('Temperature Profile Comparison', fontsize=14, fontweight='bold')
    ax1.grid(True, alpha=0.3)
    ax1.legend(loc='best', fontsize=9)

    # Subplot 2: SSR Output Comparison
    ax2 = fig.add_subplot(gs[1, 0], sharex=ax1)
    for i, data in enumerate(datasets):
        ax2.plot(data['time_hours'], data['ssr_output'],
                linewidth=1.5, label=data['filename'], color=colors[i], alpha=0.7)

    ax2.set_ylabel('SSR Output (%)', fontsize=12)
    ax2.set_title('SSR Output Comparison', fontsize=12, fontweight='bold')
    ax2.set_ylim(-5, 105)
    ax2.grid(True, alpha=0.3)
    ax2.legend(loc='best', fontsize=9)

    # Only set xlabel on bottom left subplot
    if not has_rate_data:
        ax2.set_xlabel('Time (hours)', fontsize=12)

    # Subplot 3: Rate Comparison (if available)
    if has_rate_data:
        ax_rate = fig.add_subplot(gs[2, 0], sharex=ax1)
        for i, data in enumerate(datasets):
            if 'current_rate' in data and data['current_rate']:
                ax_rate.plot(data['time_hours'], data['current_rate'],
                           linewidth=1.5, label=data['filename'], color=colors[i], alpha=0.7)

        ax_rate.axhline(y=0, color='black', linestyle='-', alpha=0.3, linewidth=0.5)
        ax_rate.set_xlabel('Time (hours)', fontsize=12)
        ax_rate.set_ylabel('Rate (°C/h)', fontsize=12)
        ax_rate.set_title('Rate Comparison (Adaptive Control)', fontsize=12, fontweight='bold')
        ax_rate.grid(True, alpha=0.3)
        ax_rate.legend(loc='best', fontsize=9)

    # Subplot 4: Metrics Table
    ax3 = fig.add_subplot(gs[:, 1])
    ax3.axis('tight')
    ax3.axis('off')

    # Calculate metrics for all runs
    metrics_list = [calculate_metrics(data) for data in datasets]

    # Create table data
    table_data = [['Metric'] + [data['filename'][:20] for data in datasets]]  # Headers (truncate long names)
    table_data.append(['Max Temp (°C)'] + [f"{m['max_temp']:.1f}" for m in metrics_list])
    table_data.append(['Overshoot (°C)'] + [f"{m['overshoot']:.1f}" for m in metrics_list])
    table_data.append(['Mean Error (°C)'] + [f"{m['mean_error']:.2f}" for m in metrics_list])
    table_data.append(['Max Error (°C)'] + [f"{m['max_error']:.1f}" for m in metrics_list])
    table_data.append(['Duration (h)'] + [f"{m['duration']:.2f}" for m in metrics_list])
    table_data.append(['Avg SSR (%)'] + [f"{m['avg_ssr']:.1f}" for m in metrics_list])

    # Create table
    table = ax3.table(cellText=table_data, loc='center', cellLoc='center')
    table.auto_set_font_size(False)
    table.set_fontsize(9)
    table.scale(1, 2)

    # Style header row
    for i in range(len(datasets) + 1):
        cell = table[(0, i)]
        cell.set_facecolor('#4CAF50')
        cell.set_text_props(weight='bold', color='white')

    # Alternate row colors
    for i in range(1, len(table_data)):
        for j in range(len(datasets) + 1):
            cell = table[(i, j)]
            if i % 2 == 0:
                cell.set_facecolor('#f0f0f0')

    ax3.set_title('Performance Metrics', fontsize=12, fontweight='bold', pad=20)

    fig.suptitle(f'Kiln Run Comparison - {len(datasets)} Runs', fontsize=14, fontweight='bold')

    plt.tight_layout()

    if output_file:
        plt.savefig(output_file, dpi=150, bbox_inches='tight')
        print(f"✓ Comparison graph saved to: {output_file}")
    else:
        plt.show()


def main():
    """Main entry point"""
    parser = argparse.ArgumentParser(
        description='Compare multiple kiln firing or tuning runs',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  python compare_runs.py logs/run1.csv logs/run2.csv
  python compare_runs.py logs/*.csv --output comparison.png
        """
    )
    parser.add_argument('csv_files', nargs='+', help='CSV files to compare (2 or more)')
    parser.add_argument('--output', '-o', help='Output file path (default: show interactive plot)')

    args = parser.parse_args()

    # Validate number of files
    if len(args.csv_files) < 2:
        print("\n❌ Error: Please provide at least 2 CSV files to compare")
        sys.exit(1)

    if len(args.csv_files) > 10:
        print("\n⚠️  Warning: Comparing more than 10 runs may be difficult to read")

    # Check if all files exist
    for csv_file in args.csv_files:
        if not Path(csv_file).exists():
            print(f"\n❌ Error: File not found: {csv_file}")
            sys.exit(1)

    print(f"\n📂 Loading {len(args.csv_files)} runs for comparison...")

    try:
        # Load all datasets
        datasets = []
        for csv_file in args.csv_files:
            data = load_run_data(csv_file)
            datasets.append(data)
            print(f"  ✓ {Path(csv_file).name}: {len(data['time'])} points, "
                  f"{data['time_hours'][-1]:.2f}h, max {max(data['temp']):.1f}°C")

        # Create comparison plot
        print(f"\n📊 Generating comparison graph...")
        compare_runs(datasets, args.output)

        if not args.output:
            print("✓ Close the plot window to exit")

    except Exception as e:
        print(f"\n❌ Error: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)


if __name__ == "__main__":
    main()
