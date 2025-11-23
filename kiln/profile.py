# kiln/profile.py
# Kiln firing profile management with adaptive rate control

import json
import gc

class Profile:
    """
    Kiln firing profile with step-based temperature schedule

    JSON Format:
    {
        "name": "Cone 6 Glaze",
        "temp_units": "c",
        "description": "Optional description",
        "steps": [
            {
                "type": "ramp",
                "target_temp": 600,
                "desired_rate": 100,  # °C/hour target
                "min_rate": 80        # Minimum acceptable (optional)
            },
            {
                "type": "hold",
                "target_temp": 600,
                "duration": 600       # seconds
            },
            {
                "type": "cooling",
                "target_temp": 100    # Optional: complete when reached, omit for natural cooling to end
            }
        ]
    }
    
    Step Types:
    - ramp: Heat/cool at controlled rate to target temperature
    - hold: Maintain target temperature for specified duration
    - cooling: Natural cooling with SSR off (target_temp optional)
    """

    def __init__(self, json_data):
        """Initialize profile from JSON data (dict or string)"""
        if isinstance(json_data, str):
            json_data = json.loads(json_data)

        self.name = json_data['name']
        self.temp_units = json_data.get('temp_units', 'c')
        self.description = json_data.get('description', '')

        # Step-based format
        if 'steps' not in json_data:
            raise ValueError("Profile must have 'steps' array")

        self.steps = json_data['steps']

        # Validate steps
        if not self.steps:
            raise ValueError("Profile must have at least one step")

        # Calculate total duration from steps
        self.duration = self._calculate_duration()

    def _calculate_duration(self):
        """
        Calculate total profile duration from steps

        Estimates duration based on desired rates. Actual duration may vary
        if adaptive control adjusts rates during execution.

        Returns:
            Estimated duration in seconds
        """
        total_seconds = 0
        current_temp = self.steps[0].get('target_temp', 20)

        for step in self.steps:
            if step['type'] == 'hold':
                total_seconds += step['duration']
            elif step['type'] == 'ramp':
                target = step['target_temp']
                dtemp = abs(target - current_temp)
                rate = step.get('desired_rate', 100)
                if rate > 0:
                    dt_hours = dtemp / rate
                    total_seconds += dt_hours * 3600
                current_temp = target
            elif step['type'] == 'cooling':
                # Natural cooling - estimate based on target if specified
                target = step.get('target_temp')
                if target is not None:
                    # Estimate natural cooling at ~100°C/hour (conservative)
                    dtemp = abs(current_temp - target)
                    dt_hours = dtemp / 100.0
                    total_seconds += dt_hours * 3600
                    current_temp = target
                else:
                    # No target - cannot estimate duration (assume 0 for now)
                    # This step likely runs until manual stop
                    pass

        return total_seconds

    def is_complete(self, elapsed_seconds):
        """
        Check if profile has completed

        For step-based profiles, completion is handled by step sequencing,
        but this provides a fallback duration check.

        Args:
            elapsed_seconds: Time since profile start

        Returns:
            True if elapsed time exceeds estimated duration
        """
        return elapsed_seconds >= self.duration

    def get_progress(self, elapsed_seconds):
        """
        Get progress percentage

        Estimates progress based on elapsed time vs total duration.
        With adaptive control, actual progress may differ.

        Args:
            elapsed_seconds: Time since profile start

        Returns:
            Progress percentage (0-100)
        """
        if self.duration == 0:
            return 100.0
        return min(100.0, (elapsed_seconds / self.duration) * 100)

    def to_dict(self):
        """Convert profile to dictionary for JSON serialization"""
        return {
            'name': self.name,
            'temp_units': self.temp_units,
            'description': self.description,
            'steps': self.steps,
            'duration': self.duration
        }

    @staticmethod
    def load_from_file(filename):
        """Load profile from JSON file"""
        with open(filename, 'r') as f:
            json_data = json.load(f)
        return Profile(json_data)

    def save_to_file(self, filename):
        """Save profile to JSON file"""
        with open(filename, 'w') as f:
            json.dump(self.to_dict(), f, indent=2)

    @staticmethod
    def list_profiles(directory='profiles'):
        """
        List all available profiles

        Returns:
            List of profile dictionaries with metadata
        """
        import os
        profiles = []

        try:
            for filename in os.listdir(directory):
                if filename.endswith('.json'):
                    try:
                        filepath = f"{directory}/{filename}"
                        with open(filepath, 'r') as f:
                            data = json.load(f)

                        # Calculate duration from steps
                        duration = 0
                        if 'steps' in data and data['steps']:
                            current_temp = data['steps'][0].get('target_temp', 20)
                            for step in data['steps']:
                                if step['type'] == 'hold':
                                    duration += step['duration']
                                elif step['type'] == 'ramp':
                                    target = step['target_temp']
                                    dtemp = abs(target - current_temp)
                                    rate = step.get('desired_rate', 100)
                                    if rate > 0:
                                        duration += (dtemp / rate) * 3600
                                    current_temp = target
                                elif step['type'] == 'cooling':
                                    target = step.get('target_temp')
                                    if target is not None:
                                        dtemp = abs(current_temp - target)
                                        duration += (dtemp / 100.0) * 3600
                                        current_temp = target

                        # Extract metadata only (not full data/steps)
                        profiles.append({
                            'name': data.get('name', filename),
                            'description': data.get('description', ''),
                            'temp_units': data.get('temp_units', 'c'),
                            'duration': duration,
                            'filename': filename
                        })

                        # MEMORY OPTIMIZED: Free memory immediately after loading each profile
                        gc.collect()

                    except Exception as e:
                        print(f"Error loading profile {filename}: {e}")
        except OSError:
            pass  # Directory doesn't exist

        return profiles

    def __str__(self):
        """String representation"""
        duration_hours = self.duration / 3600
        return f"Profile(name='{self.name}', duration={duration_hours:.1f}h, steps={len(self.steps)})"

    def __repr__(self):
        return self.__str__()
