# kiln/profile.py
# Kiln firing profile management

import json

class Profile:
    """
    Kiln firing profile with time-temperature schedule

    JSON Format:
    {
        "name": "Cone 6 Glaze",
        "temp_units": "c",
        "description": "Optional description",
        "data": [
            [0, 20],        # (seconds, temp)
            [3600, 100],    # Ramp to 100°C in 1 hour
            [7200, 100],    # Hold for 1 hour
            [14400, 1200],  # Ramp to 1200°C
            [18000, 1200],  # Hold at peak
            [21600, 20]     # Cool down
        ]
    }
    """

    def __init__(self, json_data):
        """Initialize profile from JSON data (dict or string)"""
        if isinstance(json_data, str):
            json_data = json.loads(json_data)

        self.name = json_data['name']
        self.temp_units = json_data.get('temp_units', 'c')
        self.description = json_data.get('description', '')
        self.data = json_data['data']  # [(seconds, temp), ...]

        # Validate data
        if not self.data:
            raise ValueError("Profile must have at least one data point")

        # Sort by time (just in case)
        self.data.sort(key=lambda x: x[0])

        # Calculate total duration
        self.duration = self.data[-1][0]

    def get_target_temp(self, elapsed_seconds):
        """
        Get target temperature at given elapsed time
        Uses linear interpolation between schedule points

        Args:
            elapsed_seconds: Time since profile start (seconds)

        Returns:
            Target temperature (float)
        """
        # Before start
        if elapsed_seconds <= 0:
            return self.data[0][1]

        # After end
        if elapsed_seconds >= self.duration:
            return self.data[-1][1]

        # Find surrounding points
        for i in range(len(self.data) - 1):
            t1, temp1 = self.data[i]
            t2, temp2 = self.data[i + 1]

            if t1 <= elapsed_seconds <= t2:
                # Linear interpolation
                if t2 == t1:
                    return temp1

                ratio = (elapsed_seconds - t1) / (t2 - t1)
                target = temp1 + ratio * (temp2 - temp1)
                return target

        # Shouldn't reach here, but return last temp as fallback
        return self.data[-1][1]

    def is_complete(self, elapsed_seconds):
        """Check if profile has completed"""
        return elapsed_seconds >= self.duration

    def get_progress(self, elapsed_seconds):
        """Get progress percentage (0-100)"""
        if self.duration == 0:
            return 100.0
        return min(100.0, (elapsed_seconds / self.duration) * 100)

    def to_dict(self):
        """Convert profile to dictionary for JSON serialization"""
        return {
            'name': self.name,
            'temp_units': self.temp_units,
            'description': self.description,
            'data': self.data,
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

                        # Return metadata only (not full data array)
                        profiles.append({
                            'name': data.get('name', filename),
                            'description': data.get('description', ''),
                            'temp_units': data.get('temp_units', 'c'),
                            'duration': data['data'][-1][0] if data.get('data') else 0,
                            'filename': filename
                        })
                    except Exception as e:
                        print(f"Error loading profile {filename}: {e}")
        except OSError:
            pass  # Directory doesn't exist

        return profiles

    def __str__(self):
        """String representation"""
        duration_hours = self.duration / 3600
        return f"Profile(name='{self.name}', duration={duration_hours:.1f}h, points={len(self.data)})"

    def __repr__(self):
        return self.__str__()
