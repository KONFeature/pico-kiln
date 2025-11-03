#!/usr/bin/env python3
"""
Kiln Profile Visualization Tool

Generates a temperature vs. time graph for kiln firing profiles.
Usage: python plot_profile.py <profile.json>
"""

import json
import sys
import argparse
import matplotlib.pyplot as plt


def load_profile(filepath):
    """Load and parse a kiln profile JSON file."""
    with open(filepath, 'r') as f:
        return json.load(f)


def calculate_trajectory_from_steps(profile):
    """
    Generate expected temperature trajectory from step-based profile.

    Args:
        profile: Profile dictionary with 'steps' array

    Returns:
        List of (time_seconds, temperature) tuples
    """
    if 'data' in profile:
        # Legacy format - return as-is
        return profile['data']

    if 'steps' not in profile:
        raise ValueError("Profile must contain either 'data' or 'steps' field")

    trajectory = []
    current_time = 0
    current_temp = 20  # Start at room temperature

    for step in profile['steps']:
        step_type = step.get('type', 'ramp')
        target_temp = step.get('target_temp', current_temp)

        if step_type == 'hold':
            # Hold: constant temperature for duration
            duration = step.get('duration', 0)
            trajectory.append([current_time, current_temp])
            current_time += duration
            trajectory.append([current_time, current_temp])

        elif step_type == 'ramp':
            # Ramp: linear temperature change at desired rate
            trajectory.append([current_time, current_temp])

            # Calculate ramp duration based on desired rate
            desired_rate = step.get('desired_rate', 100)  # Default 100°C/h
            temp_change = abs(target_temp - current_temp)

            if desired_rate > 0:
                duration_hours = temp_change / desired_rate
                duration_seconds = duration_hours * 3600
            else:
                # If no rate specified, use a reasonable default
                duration_seconds = temp_change * 36  # ~100°C/h

            current_time += duration_seconds
            current_temp = target_temp
            trajectory.append([current_time, current_temp])

    return trajectory


def plot_profile(profile_data, output_file=None):
    """
    Generate a temperature vs. time plot for the given profile.

    Args:
        profile_data: Dictionary containing profile data with 'data' or 'steps' array
        output_file: Optional file path to save the plot (if None, displays interactively)
    """
    # Calculate trajectory from steps or use legacy data format
    trajectory = calculate_trajectory_from_steps(profile_data)

    # Extract time and temperature data
    times = [point[0] for point in trajectory]
    temps = [point[1] for point in trajectory]

    # Convert time from seconds to hours for better readability
    times_hours = [t / 3600 for t in times]

    # Create the plot
    plt.figure(figsize=(12, 6))
    plt.plot(times_hours, temps, marker='o', linewidth=2, markersize=6)

    # Add labels and title
    plt.xlabel('Time (hours)', fontsize=12)
    temp_unit = profile_data.get('temp_units', 'c').upper()
    plt.ylabel(f'Temperature (°{temp_unit})', fontsize=12)

    profile_name = profile_data.get('name', 'Kiln Profile')
    plt.title(f'{profile_name}\n{profile_data.get("description", "")}', fontsize=14)

    # Add grid for easier reading
    plt.grid(True, alpha=0.3)

    # Format the plot
    plt.tight_layout()

    # Add annotations for steps (if using step-based format)
    if 'steps' in profile_data:
        step_time = 0
        step_temp = 20
        for i, step in enumerate(profile_data['steps']):
            step_type = step.get('type', 'ramp')
            target_temp = step.get('target_temp', step_temp)

            if step_type == 'hold':
                duration = step.get('duration', 0)
                hold_duration_min = duration / 60
                mid_time = (step_time + step_time + duration) / 2 / 3600
                plt.annotate(f'Hold {hold_duration_min:.0f}min @ {step_temp:.0f}°C',
                           xy=(mid_time, step_temp),
                           xytext=(0, 10),
                           textcoords='offset points',
                           ha='center',
                           fontsize=9,
                           bbox=dict(boxstyle='round,pad=0.3', facecolor='yellow', alpha=0.5))
                step_time += duration
            elif step_type == 'ramp':
                desired_rate = step.get('desired_rate', 100)
                temp_change = abs(target_temp - step_temp)
                duration = (temp_change / desired_rate) * 3600 if desired_rate > 0 else temp_change * 36
                mid_time = (step_time + step_time + duration) / 2 / 3600
                plt.annotate(f'{desired_rate:.0f}°C/h',
                           xy=(mid_time, (step_temp + target_temp) / 2),
                           xytext=(0, 10),
                           textcoords='offset points',
                           ha='center',
                           fontsize=9,
                           bbox=dict(boxstyle='round,pad=0.3', facecolor='lightblue', alpha=0.5))
                step_time += duration
                step_temp = target_temp
    else:
        # Legacy format: Add annotations for hold periods (where temperature stays constant)
        for i in range(len(temps) - 1):
            if temps[i] == temps[i + 1]:
                hold_duration = (times[i + 1] - times[i]) / 60  # in minutes
                mid_time = (times_hours[i] + times_hours[i + 1]) / 2
                plt.annotate(f'Hold {hold_duration:.0f}min',
                            xy=(mid_time, temps[i]),
                            xytext=(0, 10),
                            textcoords='offset points',
                            ha='center',
                            fontsize=9,
                            bbox=dict(boxstyle='round,pad=0.3', facecolor='yellow', alpha=0.5))

    # Save or display
    if output_file:
        plt.savefig(output_file, dpi=150, bbox_inches='tight')
        print(f"Plot saved to: {output_file}")
    else:
        plt.show()


def main():
    parser = argparse.ArgumentParser(
        description='Visualize kiln firing profiles from JSON files',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  python plot_profile.py profiles/bisque_cone04.json
  python plot_profile.py profiles/cone6_glaze.json -o glaze_profile.png
        """
    )

    parser.add_argument('profile', help='Path to the profile JSON file')
    parser.add_argument('-o', '--output', help='Output file path (PNG, PDF, SVG, etc.)', default=None)

    args = parser.parse_args()

    try:
        profile_data = load_profile(args.profile)
        plot_profile(profile_data, args.output)
    except FileNotFoundError:
        print(f"Error: Profile file '{args.profile}' not found", file=sys.stderr)
        sys.exit(1)
    except json.JSONDecodeError as e:
        print(f"Error: Invalid JSON in profile file: {e}", file=sys.stderr)
        sys.exit(1)
    except KeyError as e:
        print(f"Error: Missing required field in profile: {e}", file=sys.stderr)
        sys.exit(1)
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
