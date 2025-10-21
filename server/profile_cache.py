# server/profile_cache.py
# In-memory profile filename cache to eliminate blocking directory scans
#
# This module pre-scans the profiles directory at startup and caches just the
# filenames, preventing blocking os.listdir() calls during request handling.

import gc
import os

class ProfileCache:
    """
    Singleton cache for profile filenames

    Caches only the list of profile names (not the file contents) to avoid
    blocking os.listdir() calls during request handling.

    Core 1 handles loading actual profile data from disk when needed.
    """

    _instance = None

    def __new__(cls):
        """Singleton pattern - only one instance allowed"""
        if cls._instance is None:
            cls._instance = super(ProfileCache, cls).__new__(cls)
            cls._instance._initialized = False
        return cls._instance

    def __init__(self):
        """Initialize the profile cache (only runs once due to singleton)"""
        if self._initialized:
            return

        self._profile_names = []  # List of profile names (without .json extension)
        self._profiles_dir = None
        self._initialized = True
        print("[ProfileCache] Singleton instance created")

    def preload(self, profiles_dir):
        """
        Scan profiles directory and cache filenames

        Only caches the list of profile names, NOT the file contents.
        This is much faster and uses minimal memory.

        Args:
            profiles_dir: Path to profiles directory (e.g., 'profiles')

        Returns:
            Number of profiles found
        """
        self._profiles_dir = profiles_dir

        # Check if directory exists
        try:
            files = os.listdir(profiles_dir)
        except OSError:
            print(f"[ProfileCache] WARNING: Profiles directory '{profiles_dir}' not found")
            return 0

        # Extract profile names (just filenames, no file reading)
        self._profile_names = []
        for filename in files:
            if filename.endswith('.json'):
                profile_name = filename[:-5]  # Remove .json extension
                self._profile_names.append(profile_name)

        self._profile_names.sort()

        # Force GC after scanning
        gc.collect()
        free_mem = gc.mem_free()
        print(f"[ProfileCache] Cached {len(self._profile_names)} profile names, free memory: {free_mem}")

        return len(self._profile_names)

    def exists(self, profile_name):
        """
        Check if profile exists in cache

        Args:
            profile_name: Profile name (without .json extension)

        Returns:
            True if profile exists, False otherwise
        """
        return profile_name in self._profile_names

    def list_profiles(self):
        """
        Get list of all cached profile names

        Returns:
            Sorted list of profile names (without .json extension)
        """
        return list(self._profile_names)  # Return copy to prevent modification

    def add(self, profile_name):
        """
        Add profile name to cache

        Call this after uploading a new profile to keep cache in sync.

        Args:
            profile_name: Profile name (without .json extension)
        """
        if profile_name not in self._profile_names:
            self._profile_names.append(profile_name)
            self._profile_names.sort()
            print(f"[ProfileCache] Added '{profile_name}' to cache")

    def remove(self, profile_name):
        """
        Remove profile name from cache

        Args:
            profile_name: Profile name (without .json extension)

        Returns:
            True if profile was removed, False if not found
        """
        if profile_name in self._profile_names:
            self._profile_names.remove(profile_name)
            print(f"[ProfileCache] Removed '{profile_name}' from cache")
            return True
        return False

    def refresh(self):
        """
        Re-scan profiles directory and reload cache

        Use sparingly - blocks during directory scan!
        Only call from background tasks, never from request handlers.
        """
        if not self._profiles_dir:
            print("[ProfileCache] Cannot refresh: profiles_dir not set")
            return 0

        print("[ProfileCache] Refreshing cache from disk...")
        return self.preload(self._profiles_dir)

    def clear(self):
        """Clear all cached profile names (frees memory)"""
        count = len(self._profile_names)
        self._profile_names.clear()
        gc.collect()
        print(f"[ProfileCache] Cleared {count} cached profile names")


# Global singleton instance
_cache = ProfileCache()


def get_profile_cache():
    """
    Get the global ProfileCache singleton instance

    Returns:
        ProfileCache instance
    """
    return _cache
