"""
Data Loading and Phase Detection

This module handles loading tuning data from CSV files and detecting test phases.
"""

import csv
from datetime import datetime
from typing import List, Dict, Optional


# =============================================================================
# Data Structures
# =============================================================================

class Phase:
    """Represents a detected phase in the tuning data."""
    def __init__(self, start_idx: int, end_idx: int, phase_type: str,
                 avg_ssr: float, temp_start: float, temp_end: float,
                 step_name: Optional[str] = None, step_index: Optional[int] = None):
        self.start_idx = start_idx
        self.end_idx = end_idx
        self.phase_type = phase_type  # 'heating', 'cooling', 'plateau'
        self.avg_ssr = avg_ssr
        self.temp_start = temp_start
        self.temp_end = temp_end
        self.step_name = step_name  # Name of the tuning step (e.g., "heat_60pct_to_100C")
        self.step_index = step_index  # Index of the tuning step (0, 1, 2, ...)

    def __repr__(self):
        step_info = f", step={self.step_name}" if self.step_name else ""
        return f"Phase({self.phase_type}, SSR={self.avg_ssr:.1f}%, {self.temp_start:.1f}->{self.temp_end:.1f}°C{step_info})"


# =============================================================================
# Data Loading
# =============================================================================

def load_tuning_data(csv_file: str) -> Dict:
    """
    Load comprehensive tuning data from CSV file.

    Args:
        csv_file: Path to CSV file with tuning data

    Returns:
        Dictionary with all data arrays: time, temp, ssr_output, timestamps,
        and optionally step_names, step_indices, total_steps if available.
        Also includes 'has_step_data' flag indicating if step columns exist.
    """
    time_data = []
    temp_data = []
    ssr_output_data = []
    timestamps = []
    step_names = []
    step_indices = []
    total_steps_data = []
    has_step_data = False

    with open(csv_file, 'r') as f:
        reader = csv.DictReader(f)

        # Check if step columns exist in the CSV
        fieldnames = reader.fieldnames or []
        has_step_columns = all(col in fieldnames for col in ['step_name', 'step_index', 'total_steps'])

        if has_step_columns:
            has_step_data = True

        for row in reader:
            # Skip RECOVERY state entries
            if row.get('state') == 'RECOVERY':
                continue

            # Note: elapsed_seconds in tuning CSV is per-step, not overall
            # We'll calculate overall elapsed time from timestamps below
            temp_data.append(float(row['current_temp_c']))
            ssr_output_data.append(float(row['ssr_output_percent']))
            timestamps.append(row['timestamp'])

            # Load step data if available
            if has_step_columns:
                step_names.append(row['step_name'])
                step_indices.append(int(row['step_index']))
                total_steps_data.append(int(row['total_steps']))

    # Always calculate overall elapsed time from timestamps
    # (elapsed_seconds in tuning CSV is per-step, not overall)
    start_dt = datetime.strptime(timestamps[0], '%Y-%m-%d %H:%M:%S')
    time_data = []
    for ts in timestamps:
        dt = datetime.strptime(ts, '%Y-%m-%d %H:%M:%S')
        elapsed = (dt - start_dt).total_seconds()
        time_data.append(elapsed)

    result = {
        'time': time_data,
        'temp': temp_data,
        'ssr_output': ssr_output_data,
        'timestamps': timestamps,
        'has_step_data': has_step_data
    }

    # Add step data if available
    if has_step_data:
        result['step_names'] = step_names
        result['step_indices'] = step_indices
        result['total_steps'] = total_steps_data

    return result


# =============================================================================
# Phase Detection
# =============================================================================

def detect_phases(data: Dict, plateau_threshold: float = 0.5, ssr_change_threshold: float = 10.0) -> List[Phase]:
    """
    Detect different test phases from the data using physics-based measurement analysis.

    This function uses a unified algorithm based purely on measurements (SSR output and
    temperature changes), making it robust and independent of naming conventions or metadata.

    Phase boundaries are detected when SSR output changes significantly. Phase types are
    classified based on actual temperature behavior and SSR level.

    Args:
        data: Dictionary with time, temp, ssr_output arrays (step data optional, for reference only)
        plateau_threshold: Temperature change threshold (°C/min) for plateau detection (default: 0.5)
        ssr_change_threshold: SSR change (%) that triggers a new phase boundary (default: 10.0)

    Returns:
        List of Phase objects with measurement-based classification

    Phase Classification Logic:
        - COOLING: SSR < 5% (natural cooling, no heat input)
        - HEATING: SSR >= 5% AND temp_rate > plateau_threshold
        - PLATEAU: SSR >= 5% AND |temp_rate| <= plateau_threshold (steady-state hold)
    """
    phases = []
    time = data['time']
    temp = data['temp']
    ssr = data['ssr_output']

    if len(time) < 10:
        return phases

    # Optional step data for reference/debugging (not used for classification)
    step_names = data.get('step_names', [])
    step_indices = data.get('step_indices', [])
    has_step_data = data.get('has_step_data', False)

    # Unified phase detection algorithm based on SSR changes
    current_ssr = ssr[0]
    phase_start = 0

    for i in range(1, len(ssr) + 1):
        # Detect phase boundary: significant SSR change or end of data
        is_ssr_change = i < len(ssr) and abs(ssr[i] - current_ssr) > ssr_change_threshold
        is_end = i == len(ssr)

        if is_ssr_change or is_end:
            # Define phase end index
            phase_end = i - 1 if not is_end else i - 1

            # Calculate phase characteristics
            phase_duration = time[phase_end] - time[phase_start]

            # Skip very short phases (less than 1 second)
            if phase_duration < 1:
                if not is_end:
                    phase_start = i
                    current_ssr = ssr[i]
                continue

            # Calculate measurements for this phase
            avg_ssr = sum(ssr[phase_start:phase_end+1]) / (phase_end - phase_start + 1)
            temp_start = temp[phase_start]
            temp_end = temp[phase_end]
            temp_change = temp_end - temp_start

            # Calculate temperature rate (°C/min)
            rate_per_min = (temp_change / phase_duration) * 60 if phase_duration > 0 else 0

            # Classify phase type based purely on measurements (physics, not names!)
            if avg_ssr < 5.0:
                # No heat input → natural cooling
                phase_type = 'cooling'
            elif rate_per_min > plateau_threshold:
                # Heating with SSR on and temp rising
                phase_type = 'heating'
            elif abs(rate_per_min) <= plateau_threshold:
                # SSR on but temp stable → plateau/hold
                phase_type = 'plateau'
            else:
                # SSR on but temp falling (rare, could be cooling from higher setpoint)
                phase_type = 'cooling'

            # Attach step metadata if available (for reference/debugging only)
            step_name = step_names[phase_start] if has_step_data and phase_start < len(step_names) else None
            step_index = step_indices[phase_start] if has_step_data and phase_start < len(step_indices) else None

            phases.append(Phase(phase_start, phase_end, phase_type, avg_ssr,
                              temp_start, temp_end, step_name, step_index))

            # Move to next phase
            if not is_end:
                phase_start = i
                current_ssr = ssr[i]

    return phases
