# kiln/comms.py
# Inter-thread communication protocol and helpers
#
# This module defines the message structures and utilities for communication
# between the control thread (Core 1) and the web server thread (Core 2)
# using a custom ThreadSafeQueue implementation.

import gc
from micropython import const
from collections import deque

try:
    from _thread import allocate_lock
except ImportError:
    # Fallback for testing on CPython
    from threading import Lock as allocate_lock


class ThreadSafeQueue:
    """
    Thread-safe FIFO queue implementation using _thread.allocate_lock()

    This is a custom implementation since ThreadSafeQueue is not available
    in the standard MicroPython _thread module. Uses collections.deque with
    lock-based synchronization for O(1) pop operations.

    Compatible with the expected ThreadSafeQueue API (put_sync/get_sync).
    """

    def __init__(self, maxsize=10):
        """
        Initialize thread-safe queue

        Args:
            maxsize: Maximum queue size (0 = unlimited)
        """
        self.maxsize = maxsize
        # MicroPython's deque requires (iterable, maxlen) as positional arguments
        # Use maxsize if specified, otherwise use a large value for "unlimited"
        self._queue = deque((), maxsize)
        self._lock = allocate_lock()

    def put_sync(self, item):
        """
        Put item in queue (raises exception if full)

        This is non-blocking. If the queue is full, it raises an exception
        instead of blocking.

        Args:
            item: Item to add to queue

        Raises:
            Exception: If queue is full
        """
        self._lock.acquire()
        try:
            if self.maxsize > 0 and len(self._queue) >= self.maxsize:
                raise Exception("Queue full")
            self._queue.append(item)
        finally:
            self._lock.release()

    def get_sync(self):
        """
        Get item from queue (raises exception if empty)

        This is non-blocking. If the queue is empty, it raises an exception
        instead of blocking.

        Returns:
            Item from queue (FIFO order)

        Raises:
            Exception: If queue is empty
        """
        self._lock.acquire()
        try:
            if len(self._queue) == 0:
                raise Exception("Queue empty")
            return self._queue.popleft()
        finally:
            self._lock.release()

    def qsize(self):
        """Return the approximate size of the queue"""
        self._lock.acquire()
        try:
            return len(self._queue)
        finally:
            self._lock.release()

    def empty(self):
        """Return True if the queue is empty"""
        self._lock.acquire()
        try:
            return len(self._queue) == 0
        finally:
            self._lock.release()

    def full(self):
        """Return True if the queue is full"""
        self._lock.acquire()
        try:
            if self.maxsize <= 0:
                return False
            return len(self._queue) >= self.maxsize
        finally:
            self._lock.release()

    def clear(self):
        """Clear all items from the queue"""
        self._lock.acquire()
        try:
            self._queue.clear()
        finally:
            self._lock.release()


class ReadyFlag:
    """
    Thread-safe ready flag for Core 1 synchronization

    Used to signal when Core 1 has completed hardware initialization
    and is ready to receive commands. Core 2 waits for this flag before
    proceeding with operations that depend on Core 1 being operational.
    """

    def __init__(self):
        self._ready = False
        self._lock = allocate_lock()

    def set_ready(self):
        """Signal that Core 1 is ready (called from Core 1)"""
        with self._lock:
            self._ready = True

    def is_ready(self):
        """Check if Core 1 is ready (thread-safe)"""
        with self._lock:
            return self._ready

    async def wait_ready(self, timeout=5.0):
        """
        Wait for Core 1 to be ready (async, called from Core 2)

        Args:
            timeout: Maximum time to wait in seconds

        Returns:
            True if Core 1 became ready, False if timeout
        """
        import asyncio
        import time

        start = time.time()
        while not self.is_ready():
            if time.time() - start > timeout:
                return False
            await asyncio.sleep(0.1)
        return True


class QuietMode:
    """
    Thread-safe quiet mode flag for boot optimization

    During WiFi connection phase, Core 1 operates in "quiet mode":
    - Temperature monitoring continues (safety)
    - Hardware control continues (SSR, PID)
    - Status updates are suppressed (reduces queue contention)

    This gives WiFi maximum CPU time during the critical connection phase.
    """

    def __init__(self):
        self._quiet = False
        self._lock = allocate_lock()

    def set_quiet(self, quiet):
        """Set quiet mode on/off (called from Core 2)"""
        with self._lock:
            self._quiet = quiet

    def is_quiet(self):
        """Check if in quiet mode (called from Core 1)"""
        with self._lock:
            return self._quiet


class MessageType:
    """Command message types (Core 2 -> Core 1) - using integer const for memory optimization"""
    RUN_PROFILE = const(1)      # Start running a profile
    RESUME_PROFILE = const(2)   # Resume a previously interrupted profile
    STOP = const(3)             # Stop current profile
    SHUTDOWN = const(4)         # Emergency shutdown
    START_TUNING = const(5)     # Start PID auto-tuning
    STOP_TUNING = const(6)      # Stop PID auto-tuning
    PING = const(7)             # For testing thread communication

def state_to_string(state_int):
    """
    Convert integer state constant to string representation for status messages

    This is needed because Core 2 (web server, LCD, data logger) expects string states,
    but KilnState uses integer constants for memory optimization on Core 1.

    Args:
        state_int: Integer state constant from KilnState

    Returns:
        String representation of state ('IDLE', 'RUNNING', 'TUNING', 'COMPLETE', 'ERROR')
    """
    # Import here to avoid circular dependency
    from kiln.state import KilnState

    if state_int == KilnState.IDLE:
        return 'IDLE'
    elif state_int == KilnState.RUNNING:
        return 'RUNNING'
    elif state_int == KilnState.TUNING:
        return 'TUNING'
    elif state_int == KilnState.COMPLETE:
        return 'COMPLETE'
    elif state_int == KilnState.ERROR:
        return 'ERROR'
    else:
        return 'UNKNOWN'

class CommandMessage:
    """
    Helper class for building command messages

    These messages are sent from Core 2 (web server) to Core 1 (control thread)
    """

    @staticmethod
    def run_profile(profile_filename):
        """Start running a firing profile

        Args:
            profile_filename: Filename of the profile to run (e.g., "cone6_glaze.json")
        """
        return {
            'type': MessageType.RUN_PROFILE,
            'profile_filename': profile_filename
        }

    @staticmethod
    def resume_profile(profile_filename, elapsed_seconds, current_rate=None, last_logged_temp=None, current_temp=None, step_index=None):
        """Resume a previously interrupted firing profile

        Args:
            profile_filename: Filename of the profile to resume (e.g., "cone6_glaze.json")
            elapsed_seconds: How far through the profile execution to resume from
            current_rate: Adapted rate to restore (from CSV log), or None for desired_rate
            last_logged_temp: Last logged temperature before crash (for recovery detection)
            current_temp: Current temperature (for recovery detection)
            step_index: Step index from CSV log (0-based), or None to calculate
        """
        return {
            'type': MessageType.RESUME_PROFILE,
            'profile_filename': profile_filename,
            'elapsed_seconds': elapsed_seconds,
            'current_rate': current_rate,
            'last_logged_temp': last_logged_temp,
            'current_temp': current_temp,
            'step_index': step_index
        }

    @staticmethod
    def stop():
        """Stop current profile"""
        return {
            'type': MessageType.STOP
        }

    @staticmethod
    def shutdown():
        """Emergency shutdown - stop and turn off SSR"""
        return {
            'type': MessageType.SHUTDOWN
        }

    @staticmethod
    def start_tuning(mode='STANDARD', max_temp=None):
        """
        Start PID auto-tuning

        Args:
            mode: Tuning mode (SAFE, STANDARD, or THOROUGH)
            max_temp: Maximum temperature (°C), uses mode default if None
        """
        return {
            'type': MessageType.START_TUNING,
            'mode': mode,
            'max_temp': max_temp
        }

    @staticmethod
    def stop_tuning():
        """Stop PID auto-tuning"""
        return {
            'type': MessageType.STOP_TUNING
        }

    @staticmethod
    def ping():
        """Ping message for testing"""
        return {
            'type': MessageType.PING
        }

class StatusMessage:
    """
    Helper class for building status messages

    These messages are sent from Core 1 (control thread) to Core 2 (web server)

    MEMORY OPTIMIZED: Uses pre-allocated templates to reduce dict creation overhead.
    Templates are copied (not reused) for thread safety when passing between cores.
    """

    # Pre-allocated template for status messages
    # This template is copied and updated rather than creating dict from scratch each time
    # Thread safety: Each call to build() creates a fresh copy for cross-thread passing
    # OPTIMIZED: Removed profile_duration, min_rate, progress, remaining (UI-only fields)
    # Saves ~32 bytes per message (~640 bytes with 20-item queue)
    _status_template = {
        'timestamp': 0,
        'state': 'IDLE',
        'current_temp': 0.0,
        'target_temp': 0.0,
        'ssr_output': 0.0,
        'elapsed': 0,
        'profile_name': None,
        'error': None,
        'step_index': None,
        'step_name': None,
        'total_steps': None,
        'desired_rate': 0,
        'is_recovering': False,
        'recovery_target_temp': None,
        'current_rate': 0,
        'actual_rate': 0,
        'adaptation_count': 0
    }

    # Pre-allocated template for tuning status messages
    _tuning_status_template = {
        'timestamp': 0,
        'state': 'IDLE',
        'current_temp': 0.0,
        'target_temp': 0.0,
        'elapsed': 0,
        'ssr_output': 0.0,
        'profile_name': None,
        'tuning': {},
        'step_name': None,
        'step_index': None,
        'total_steps': None
    }

    @staticmethod
    def build(controller, pid, ssr_controller):
        """
        Build comprehensive status message from controller state

        OPTIMIZED: Uses pre-allocated template and dict.copy() instead of building from scratch.
        This reduces allocation overhead while maintaining thread safety.

        Args:
            controller: KilnController instance
            pid: PID instance
            ssr_controller: SSRController instance

        Returns:
            Dictionary with complete system status
        """
        import time

        # Start with template copy (faster than building dict from scratch)
        # Copy is necessary for thread safety when passing between cores
        status = StatusMessage._status_template.copy()

        elapsed = controller.get_elapsed_time()

        # Update with current values
        status['timestamp'] = time.time()
        status['state'] = state_to_string(controller.state)
        status['current_temp'] = round(controller.current_temp, 2)
        status['target_temp'] = round(controller.target_temp, 2)
        status['ssr_output'] = round(controller.ssr_output, 2)
        status['elapsed'] = round(elapsed, 1)
        status['profile_name'] = controller.active_profile.name if controller.active_profile else None
        status['error'] = controller.error_message

        # Add profile-specific info (template already has default values)
        if controller.active_profile:
            # Add step info (from step-based profile format)
            profile = controller.active_profile
            status['total_steps'] = len(profile.steps)

            # Controller tracks current step - use it directly
            status['step_index'] = controller.current_step_index

            # Get step type (ramp/hold) for current step
            if controller.current_step_index < len(profile.steps):
                current_step = profile.steps[controller.current_step_index]
                # Safe: 'type' and 'desired_rate' are required in validated profile steps
                status['step_name'] = current_step['type']

                # Add rate information for this step
                status['desired_rate'] = current_step['desired_rate']
            else:
                status['step_name'] = ''
                # desired_rate already 0 in template
        # else: No active profile - template defaults (None/0) are already set

        # Add recovery mode information
        status['is_recovering'] = controller.recovery_target_temp is not None
        status['recovery_target_temp'] = round(controller.recovery_target_temp, 2) if controller.recovery_target_temp is not None else None

        # Add adaptive rate control information
        status['current_rate'] = round(controller.current_rate, 1)  # Adapted rate
        status['actual_rate'] = round(controller.temp_history.get_rate(controller.rate_measurement_window), 1)  # Measured rate
        status['adaptation_count'] = controller.adaptation_count  # Number of adaptations

        return status

    @staticmethod
    def build_tuning_status(controller, tuner, ssr_controller):
        """
        Build tuning status message

        OPTIMIZED: Uses pre-allocated template and dict.copy() instead of building from scratch.
        This reduces allocation overhead while maintaining thread safety.

        Args:
            controller: KilnController instance
            tuner: ZieglerNicholsTuner instance
            ssr_controller: SSRController instance

        Returns:
            Dictionary with tuning status
        """
        import time

        # Start with template copy (faster than building dict from scratch)
        # Copy is necessary for thread safety when passing between cores
        status = StatusMessage._tuning_status_template.copy()

        tuner_status = tuner.get_status()
        elapsed = controller.get_elapsed_time()

        # Update with current values
        status['timestamp'] = time.time()
        status['state'] = state_to_string(controller.state)
        status['current_temp'] = round(controller.current_temp, 2)
        status['target_temp'] = round(controller.target_temp, 2)
        status['elapsed'] = round(elapsed, 1)
        status['ssr_output'] = round(controller.ssr_output, 2)
        # profile_name already set to None in template
        status['tuning'] = tuner_status
        # Expose step fields at top level for easy logging
        # Safe: tuner_status from get_status() always includes these fields
        status['step_name'] = tuner_status['step_name']
        status['step_index'] = tuner_status['step_index']
        status['total_steps'] = tuner_status['total_steps']

        return status

class QueueHelper:
    """
    Helper class for safe queue operations

    Wraps ThreadSafeQueue operations with error handling and
    provides blocking/non-blocking variants
    """

    @staticmethod
    def put_nowait(queue, item):
        """
        Put item in queue (non-blocking)

        Returns:
            True if successful, False if queue full
        """
        try:
            queue.put_sync(item)
            return True
        except:
            return False

    @staticmethod
    def get_nowait(queue):
        """
        Get item from queue (non-blocking)

        Returns:
            Item if available, None if queue empty
        """
        try:
            return queue.get_sync()
        except:
            return None

    @staticmethod
    def clear(queue):
        """
        Clear all items from queue

        Returns:
            Number of items cleared
        """
        count = 0
        while True:
            try:
                queue.get_sync()
                count += 1
            except:
                break
        return count

class StatusCache:
    """
    Thread-safe cache for latest status message

    Used by web server (Core 2) to quickly serve status requests
    without blocking on queue operations

    MEMORY OPTIMIZED: Added get_fields() method to fetch multiple fields without
    full dictionary copy, and added periodic GC trigger to clean up old copies.
    """

    def __init__(self):
        self.lock = allocate_lock()
        self._status = {
            'timestamp': 0,
            'state': 'IDLE',
            'current_temp': 0.0,
            'target_temp': 0.0,
            'ssr_output': 0.0,
            'elapsed': 0,
            'profile_name': None,
            'error': None
        }
        self._copy_count = 0  # Track copies for periodic GC

    def update(self, status):
        """Update cached status (thread-safe)"""
        with self.lock:
            self._status = status

    def get(self):
        """
        Get cached status (thread-safe)

        MEMORY NOTE: Creates a copy for thread safety. For frequently accessed
        fields, consider using get_field() or get_fields() to avoid copying.
        """
        with self.lock:
            copy = self._status.copy()

            # Trigger GC every 10 copies to clean up old dictionaries
            self._copy_count += 1
            if self._copy_count >= 10:
                self._copy_count = 0
                gc.collect()

            return copy

    def get_field(self, field, default=None):
        """
        Get specific field from cached status without copying entire dict

        Args:
            field: Field name to retrieve
            default: Default value if field doesn't exist

        Returns:
            Field value or default
        """
        with self.lock:
            return self._status.get(field, default)

    def get_fields(self, *fields):
        """
        Get multiple specific fields from cached status without full copy

        More memory-efficient than get() when you only need a few fields.

        Args:
            *fields: Field names to retrieve

        Returns:
            Dictionary with only requested fields

        Example:
            cache.get_fields('current_temp', 'target_temp', 'ssr_output')
        """
        with self.lock:
            return {field: self._status.get(field) for field in fields}
