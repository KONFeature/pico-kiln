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


def plot_profile(profile_data, output_file=None):
    """
    Generate a temperature vs. time plot for the given profile.

    Args:
        profile_data: Dictionary containing profile data with 'data' array
        output_file: Optional file path to save the plot (if None, displays interactively)
    """
    # Extract time and temperature data
    times = [point[0] for point in profile_data['data']]
    temps = [point[1] for point in profile_data['data']]

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

    # Add annotations for hold periods (where temperature stays constant)
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
