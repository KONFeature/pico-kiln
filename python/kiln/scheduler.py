# kiln/scheduler.py
# Scheduled profile queue for delayed start functionality
#
# This module manages a single scheduled profile with delayed start capability.
# It provides a clean separation between scheduling logic and profile execution.

import time

try:
    from _thread import allocate_lock
except ImportError:
    # Fallback for testing on CPython
    from threading import Lock as allocate_lock


class ScheduledProfile:
    """
    Container for scheduled profile data
    
    Stores information about a profile that is scheduled to start
    at a specific time in the future.
    """
    
    def __init__(self, profile_filename, start_time):
        """
        Initialize scheduled profile
        
        Args:
            profile_filename: Filename of profile to run (e.g., "cone6_glaze.json")
            start_time: Unix timestamp when profile should start
        """
        self.profile_filename = profile_filename
        self.start_time = start_time  # Unix timestamp
        self.scheduled_at = time.time()  # When was it scheduled


class ScheduledProfileQueue:
    """
    Manages a single scheduled profile with delayed start capability
    
    This class provides thread-safe operations for scheduling, checking,
    and consuming a profile that should start at a specific time.
    
    Design: Only ONE profile can be scheduled at a time. This is a deliberate
    simplification that matches typical kiln usage patterns.
    
    Usage:
        scheduler = ScheduledProfileQueue()
        
        # Schedule a profile
        scheduler.schedule("cone6.json", time.time() + 3600)  # Start in 1 hour
        
        # In control loop (when IDLE):
        if scheduler.can_consume():
            profile_filename = scheduler.consume()
            # Start the profile...
        
        # Get status for web API
        status = scheduler.get_status()
        
        # Cancel scheduled profile
        scheduler.cancel()
    """
    
    def __init__(self):
        """Initialize empty scheduler"""
        self._lock = allocate_lock()
        self._scheduled_item = None  # None or ScheduledProfile instance
    
    def schedule(self, profile_filename, start_time):
        """
        Schedule a profile to start at specific time
        
        Args:
            profile_filename: Profile to run (e.g., "cone6_glaze.json")
            start_time: Unix timestamp when to start
        
        Raises:
            Exception: If a profile is already scheduled
            Exception: If start_time is not in the future
        """
        with self._lock:
            if self._scheduled_item is not None:
                raise Exception(f"Profile already scheduled: {self._scheduled_item.profile_filename}")
            
            # Validate start time is in future
            if start_time <= time.time():
                raise Exception("Start time must be in the future")
            
            self._scheduled_item = ScheduledProfile(profile_filename, start_time)
    
    def can_consume(self):
        """
        Check if scheduled profile is ready to start
        
        Returns:
            True if profile is scheduled and start time has arrived, False otherwise
        """
        with self._lock:
            if self._scheduled_item is None:
                return False
            
            return time.time() >= self._scheduled_item.start_time
    
    def consume(self):
        """
        Consume the scheduled profile (removes it from queue)
        
        This should be called when the control loop is ready to start
        the scheduled profile. It returns the profile filename and removes
        the item from the queue.
        
        Returns:
            profile_filename if ready to start, None otherwise
        """
        with self._lock:
            if self._scheduled_item is None:
                return None
            
            if time.time() >= self._scheduled_item.start_time:
                profile_filename = self._scheduled_item.profile_filename
                self._scheduled_item = None
                return profile_filename
            
            return None
    
    def cancel(self):
        """
        Cancel scheduled profile
        
        Returns:
            True if something was cancelled, False if queue was empty
        """
        with self._lock:
            if self._scheduled_item is None:
                return False
            
            cancelled_filename = self._scheduled_item.profile_filename
            self._scheduled_item = None
            return True
    
    def get_status(self):
        """
        Get status of scheduled profile for web API
        
        Returns:
            Dictionary with scheduled profile info, or None if nothing scheduled
            
            Dictionary format:
            {
                'profile_filename': 'cone6_glaze.json',
                'start_time': 1234567890,  # Unix timestamp
                'start_time_iso': '2025-11-12 22:00:00',  # Human-readable
                'seconds_until_start': 3600  # Time remaining
            }
        """
        with self._lock:
            if self._scheduled_item is None:
                return None
            
            seconds_until_start = max(0, self._scheduled_item.start_time - time.time())
            
            return {
                'profile_filename': self._scheduled_item.profile_filename,
                'start_time': self._scheduled_item.start_time,
                'start_time_iso': self._format_time_iso(self._scheduled_item.start_time),
                'seconds_until_start': int(seconds_until_start)
            }
    
    def _format_time_iso(self, timestamp):
        """
        Format Unix timestamp as ISO-like string
        
        Args:
            timestamp: Unix timestamp
        
        Returns:
            String in format "YYYY-MM-DD HH:MM:SS"
        """
        try:
            t = time.localtime(timestamp)
            return f"{t[0]}-{t[1]:02d}-{t[2]:02d} {t[3]:02d}:{t[4]:02d}:{t[5]:02d}"
        except:
            return f"timestamp:{int(timestamp)}"
