# kiln/comms.py
# Inter-thread communication protocol and helpers
#
# This module defines the message structures and utilities for communication
# between the control thread (Core 1) and the web server thread (Core 2)
# using a custom ThreadSafeQueue implementation.

import gc
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

class MessageType:
    """Command message types (Core 2 -> Core 1)"""
    RUN_PROFILE = 'run_profile'
    RESUME_PROFILE = 'resume_profile'
    STOP = 'stop'
    SHUTDOWN = 'shutdown'
    START_TUNING = 'start_tuning'
    STOP_TUNING = 'stop_tuning'
    PING = 'ping'  # For testing thread communication

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
    def resume_profile(profile_filename, elapsed_seconds):
        """Resume a previously interrupted firing profile

        Args:
            profile_filename: Filename of the profile to resume (e.g., "cone6_glaze.json")
            elapsed_seconds: How far through the profile execution to resume from
        """
        return {
            'type': MessageType.RESUME_PROFILE,
            'profile_filename': profile_filename,
            'elapsed_seconds': elapsed_seconds
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
    """

    @staticmethod
    def build(controller, pid, ssr_controller):
        """
        Build comprehensive status message from controller state

        Args:
            controller: KilnController instance
            pid: PID instance
            ssr_controller: SSRController instance

        Returns:
            Dictionary with complete system status
        """
        import time

        elapsed = controller.get_elapsed_time()

        status = {
            'timestamp': time.time(),
            'state': controller.state,
            'current_temp': round(controller.current_temp, 2),
            'target_temp': round(controller.target_temp, 2),
            'ssr_output': round(controller.ssr_output, 2),
            'elapsed': round(elapsed, 1),
            'profile_name': controller.active_profile.name if controller.active_profile else None,
            'error': controller.error_message,
            'remaining': 0,
            'progress': 0,
            'profile_duration': 0
        }

        # Add profile-specific info
        if controller.active_profile:
            remaining = max(0, controller.active_profile.duration - elapsed)
            status['remaining'] = round(remaining, 1)
            status['progress'] = round(controller.active_profile.get_progress(elapsed), 1)
            status['profile_duration'] = controller.active_profile.duration

            # Add step info (segment between data points)
            profile = controller.active_profile
            status['total_steps'] = len(profile.data) - 1

            # Find current segment
            current_segment = 0
            for i in range(len(profile.data) - 1):
                t1, _ = profile.data[i]
                t2, _ = profile.data[i + 1]
                if t1 <= elapsed < t2:
                    current_segment = i
                    break
                elif elapsed >= t2:
                    current_segment = i + 1

            status['step_index'] = current_segment
            status['step_name'] = ''  # Profiles don't have named steps
        else:
            # No active profile - set step fields to None
            status['step_index'] = None
            status['step_name'] = None
            status['total_steps'] = None

        # Add PID statistics
        pid_stats = pid.get_stats()
        status['pid_stats'] = pid_stats

        # Add current PID gains at top level for easy access
        status['pid_kp'] = pid_stats.get('kp', 0)
        status['pid_ki'] = pid_stats.get('ki', 0)
        status['pid_kd'] = pid_stats.get('kd', 0)

        # Add SSR state
        ssr_state = ssr_controller.get_state()
        status['ssr_duty_cycle'] = ssr_state['duty_cycle']

        return status

    @staticmethod
    def build_tuning_status(controller, tuner, ssr_controller):
        """
        Build tuning status message

        Args:
            controller: KilnController instance
            tuner: ZieglerNicholsTuner instance
            ssr_controller: SSRController instance

        Returns:
            Dictionary with tuning status
        """
        import time

        tuner_status = tuner.get_status()
        elapsed = controller.get_elapsed_time()

        status = {
            'timestamp': time.time(),
            'state': controller.state,
            'current_temp': round(controller.current_temp, 2),
            'target_temp': round(controller.target_temp, 2),
            'elapsed': round(elapsed, 1),
            'ssr_output': round(controller.ssr_output, 2),
            'progress': 0,
            'profile_name': None,
            'tuning': tuner_status,
            # Expose step fields at top level for easy logging
            'step_name': tuner_status.get('step_name'),
            'step_index': tuner_status.get('step_index'),
            'total_steps': tuner_status.get('total_steps'),
        }

        # Add SSR state (needed for web UI display)
        ssr_state = ssr_controller.get_state()
        status['ssr_duty_cycle'] = ssr_state['duty_cycle']

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


class ErrorLog:
    """
    Cross-core error logging system

    Core 1 (control) logs errors via log_error() - fast, non-blocking
    Core 2 (web/main) reads from queue and writes to disk

    Only errors are logged to avoid I/O spam. Console prints still go to stdout.
    """

    def __init__(self, max_queue_size=50):
        """
        Initialize error log

        Args:
            max_queue_size: Maximum number of queued errors before oldest are dropped
        """
        self.queue = ThreadSafeQueue(maxsize=max_queue_size)
        self.dropped_count = 0
        self.lock = allocate_lock()

    def log_error(self, source, message):
        """
        Log an error (called from Core 1)

        Non-blocking - if queue is full, drops oldest error and increments counter.

        Args:
            source: Error source (e.g., 'TemperatureSensor', 'SSRController')
            message: Error message
        """
        import time

        error_entry = {
            'timestamp': time.time(),
            'source': source,
            'message': message
        }

        # Try to add to queue
        if not QueueHelper.put_nowait(self.queue, error_entry):
            # Queue full - drop oldest and try again
            with self.lock:
                self.dropped_count += 1
            QueueHelper.get_nowait(self.queue)  # Drop oldest
            QueueHelper.put_nowait(self.queue, error_entry)

    def get_errors(self):
        """
        Get all pending errors from queue (called from Core 2)

        Returns list of error entries and current dropped count.
        Clears dropped count after reading.

        Returns:
            Tuple of (errors_list, dropped_count)
        """
        errors = []
        while not self.queue.empty():
            error = QueueHelper.get_nowait(self.queue)
            if error:
                errors.append(error)

        with self.lock:
            dropped = self.dropped_count
            self.dropped_count = 0

        return errors, dropped

    def get_stats(self):
        """
        Get error log statistics

        Returns:
            Dictionary with queue size and dropped count
        """
        with self.lock:
            return {
                'queued': self.queue.qsize(),
                'dropped': self.dropped_count
            }

    def clear(self):
        """Clear all queued errors (emergency use)"""
        self.queue.clear()
        with self.lock:
            self.dropped_count = 0
