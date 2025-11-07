# server/status_receiver.py
# Singleton status receiver for consuming status updates from control thread
#
# This module implements an observer pattern where the StatusReceiver consumes
# status updates from Core 1 and notifies registered listeners (data logger,
# web server cache, etc.). This provides clean separation of concerns.

import asyncio
from kiln.comms import QueueHelper, StatusCache
from micropython import const

# Performance: const() declaration for hot path interval
STATUS_CHECK_INTERVAL = 0.1  # Check status queue at 10 Hz

try:
    from _thread import allocate_lock
except ImportError:
    # Fallback for testing on CPython
    from threading import Lock as allocate_lock


class StatusReceiver:
    """
    Singleton status receiver that consumes status updates from control thread

    Implements observer pattern:
    - Consumes status messages from status_queue (Core 1 -> Core 2)
    - Updates shared status cache for web server
    - Notifies registered listeners (callbacks) when status updates arrive

    This allows multiple independent consumers (data logger, web UI, etc.)
    without coupling them together.
    """

    _instance = None
    _lock = allocate_lock()

    def __new__(cls):
        """Singleton pattern - only one instance allowed (thread-safe)"""
        cls._lock.acquire()
        try:
            if cls._instance is None:
                cls._instance = super(StatusReceiver, cls).__new__(cls)
                cls._instance._initialized = False
            return cls._instance
        finally:
            cls._lock.release()

    def __init__(self):
        """Initialize the status receiver (only runs once due to singleton)"""
        if self._initialized:
            return

        self.status_queue = None
        self.status_cache = StatusCache()
        self.listeners = []  # List of callback functions
        self._initialized = True
        print("[StatusReceiver] Singleton instance created")

    def initialize(self, status_queue):
        """
        Initialize with communication queues

        Args:
            status_queue: ThreadSafeQueue for receiving status from Core 1
        """
        self.status_queue = status_queue
        print("[StatusReceiver] Initialized with status queue")

    def register_listener(self, callback):
        """
        Register a listener callback to be notified of status updates

        The callback will be called with the status dictionary as argument:
            callback(status: dict) -> None

        Args:
            callback: Function to call when status updates arrive
        """
        if callback not in self.listeners:
            self.listeners.append(callback)
            print(f"[StatusReceiver] Registered listener: {callback.__name__}")

    def unregister_listener(self, callback):
        """
        Unregister a listener callback

        Args:
            callback: Function to remove from listeners
        """
        if callback in self.listeners:
            self.listeners.remove(callback)
            print(f"[StatusReceiver] Unregistered listener: {callback.__name__}")

    def get_status(self):
        """
        Get current cached status

        Returns:
            Dictionary with current system status
        """
        return self.status_cache.get()

    def get_cached_status(self):
        """
        Alias for get_status() - get current cached status

        Returns:
            Dictionary with current system status
        """
        return self.get_status()

    def get_status_field(self, field, default=None):
        """
        Get specific field from cached status

        Args:
            field: Field name to retrieve
            default: Default value if field doesn't exist

        Returns:
            Field value or default
        """
        return self.status_cache.get_field(field, default)

    def get_status_fields(self, *fields):
        """
        Get multiple specific fields from cached status

        More memory-efficient than get_status() when you only need a few fields.

        Args:
            *fields: Field names to retrieve

        Returns:
            Dictionary with only requested fields

        Example:
            receiver.get_status_fields('current_temp', 'target_temp', 'ssr_output')
        """
        return self.status_cache.get_fields(*fields)

    async def run(self):
        """
        Main async task that consumes status updates

        This should be started as a background task on Core 2.
        Continuously reads from status_queue and:
        1. Updates the status cache
        2. Notifies all registered listeners
        """
        if not self.status_queue:
            print("[StatusReceiver] ERROR: Not initialized with status queue!")
            return

        print("[StatusReceiver] Status receiver running...")

        while True:
            # Non-blocking check for status updates
            status = QueueHelper.get_nowait(self.status_queue)

            if status:
                # Update cached status
                self.status_cache.update(status)

                # Notify all listeners
                for listener in list(self.listeners):
                    try:
                        listener(status)
                    except Exception as e:
                        print(f"[StatusReceiver] Error in listener {listener.__name__}: {e}")

            await asyncio.sleep(STATUS_CHECK_INTERVAL)  # Check 10 times per second


# Global singleton instance
_receiver = StatusReceiver()


def get_status_receiver():
    """
    Get the global StatusReceiver singleton instance

    Returns:
        StatusReceiver instance
    """
    return _receiver
