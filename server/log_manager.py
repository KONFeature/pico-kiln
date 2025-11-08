# server/log_manager.py
# Log file rotation and cleanup to prevent disk space exhaustion
#
# Strategy:
# - Monitors log file size periodically (not on every write)
# - Rotates when file exceeds MAX_LOG_SIZE_BYTES
# - Rotation: current.log -> current.old.log (overwrites previous backup)
# - Old files cleaned up at boot
#
# Performance:
# - Size checks every N flushes (~100-200 seconds intervals)
# - Only 1 syscall per check (os.stat)
# - No overhead during normal writes
# - Boot cleanup is one-time cost

import os
from micropython import const

# Maximum log file size before rotation (100 KB)
MAX_LOG_SIZE_KB = const(100)
MAX_LOG_SIZE_BYTES = MAX_LOG_SIZE_KB * 1024

# Maximum number of .old backup files to keep
MAX_OLD_BACKUPS = const(5)


class LogRotator:
    """
    Manages log file rotation with minimal performance impact

    Checks file size periodically (not every write) and rotates when
    the file exceeds max size. Designed for MicroPython on resource-
    constrained devices.

    Usage:
        rotator = LogRotator('/errors.log')

        # After each flush:
        if rotator.should_rotate():
            rotator.rotate()
    """

    def __init__(self, log_path, max_size_bytes=MAX_LOG_SIZE_BYTES, check_every_n_flushes=20):
        """
        Initialize log rotator

        Args:
            log_path: Path to log file to manage
            max_size_bytes: Maximum file size before rotation (default: 100KB)
            check_every_n_flushes: Check file size every N flushes (default: 20)
                                  Higher = less overhead, but delayed rotation
        """
        self.log_path = log_path
        self.max_size_bytes = max_size_bytes
        self.check_every_n_flushes = check_every_n_flushes
        self.flush_count = 0

    def should_rotate(self):
        """
        Check if rotation is needed

        Call this after each flush. Only performs actual size check
        every N flushes to minimize syscall overhead.

        Returns:
            True if file should be rotated, False otherwise
        """
        self.flush_count += 1

        # Only check file size every N flushes (minimize syscalls)
        if self.flush_count % self.check_every_n_flushes != 0:
            return False

        try:
            stat = os.stat(self.log_path)
            size = stat[6]  # st_size field
            return size >= self.max_size_bytes
        except OSError:
            # File doesn't exist yet
            return False

    def rotate(self):
        """
        Rotate the log file with numbered backups

        Strategy:
        - current.log -> current.log.old.1
        - Keep up to MAX_OLD_BACKUPS (5) numbered backups
        - Delete oldest when limit exceeded

        This prevents disk overflow during very long runs (24+ hours)
        where multiple rotations may occur in a single session.

        Returns:
            True if rotation succeeded, False otherwise
        """
        try:
            # Get current size for logging message
            try:
                stat = os.stat(self.log_path)
                size = stat[6]
                size_kb = size // 1024
            except OSError:
                size = 0
                size_kb = 0

            # Find existing numbered backups and count them
            existing_backups = []
            for i in range(1, MAX_OLD_BACKUPS + 10):  # Check a bit beyond limit
                backup_path = f"{self.log_path}.old.{i}"
                try:
                    os.stat(backup_path)
                    existing_backups.append(i)
                except OSError:
                    pass  # Doesn't exist

            # Delete excess backups (keep only MAX_OLD_BACKUPS - 1, make room for new one)
            if len(existing_backups) >= MAX_OLD_BACKUPS:
                # Sort to get oldest first
                existing_backups.sort()
                # Delete oldest backups beyond limit
                to_delete = existing_backups[:len(existing_backups) - MAX_OLD_BACKUPS + 1]
                for backup_num in to_delete:
                    try:
                        os.remove(f"{self.log_path}.old.{backup_num}")
                    except OSError:
                        pass

            # Find next available backup number
            next_num = 1
            if existing_backups:
                next_num = max(existing_backups) + 1

            # Rename current to numbered backup
            backup_path = f"{self.log_path}.old.{next_num}"
            try:
                os.rename(self.log_path, backup_path)
                print(f"[LogRotator] Rotated {self.log_path} ({size_kb} KB -> .old.{next_num})")
            except OSError:
                # File doesn't exist, nothing to rotate
                pass

            # Reset flush counter
            self.flush_count = 0

            return True

        except Exception as e:
            print(f"[LogRotator] Rotation failed for {self.log_path}: {e}")
            return False


def cleanup_old_logs():
    """
    Clean up old and oversized log files at boot

    Strategy:
    1. Find all numbered .old.N backups for each log file
    2. Keep only MAX_OLD_BACKUPS (5) most recent, delete older ones
    3. Archive any .log files > 100KB to .old.1 (crash recovery case)

    This ensures clean state at boot and prevents disk space accumulation,
    even after very long runs with multiple rotations.
    Runs once at startup with minimal performance impact (~20-30ms).
    """
    try:
        # Log files to manage
        log_files = ['/errors.log', '/stdout.log']

        cleanup_count = 0
        archived_count = 0

        for log_file in log_files:
            # Find all numbered backups for this log file
            existing_backups = []
            for i in range(1, MAX_OLD_BACKUPS + 20):  # Check well beyond limit
                backup_path = f"{log_file}.old.{i}"
                try:
                    os.stat(backup_path)
                    existing_backups.append((i, backup_path))
                except OSError:
                    pass  # Doesn't exist

            # Delete excess backups (keep only MAX_OLD_BACKUPS most recent)
            if len(existing_backups) > MAX_OLD_BACKUPS:
                # Sort by number (oldest first)
                existing_backups.sort(key=lambda x: x[0])
                # Delete oldest beyond limit
                to_delete = existing_backups[:len(existing_backups) - MAX_OLD_BACKUPS]
                for backup_num, backup_path in to_delete:
                    try:
                        os.remove(backup_path)
                        cleanup_count += 1
                    except OSError:
                        pass

            # Check if main log is oversized (crash recovery case)
            try:
                stat = os.stat(log_file)
                size = stat[6]
                if size > MAX_LOG_SIZE_BYTES:
                    # Find next available backup number after cleanup
                    # Recalculate remaining backups after deletion
                    remaining_nums = []
                    for i in range(1, MAX_OLD_BACKUPS + 20):
                        backup_path = f"{log_file}.old.{i}"
                        try:
                            os.stat(backup_path)
                            remaining_nums.append(i)
                        except OSError:
                            pass

                    next_num = max(remaining_nums) + 1 if remaining_nums else 1

                    # Archive to numbered backup
                    backup_path = f"{log_file}.old.{next_num}"
                    os.rename(log_file, backup_path)
                    size_kb = size // 1024
                    print(f"[Cleanup] Archived oversized {log_file} ({size_kb} KB -> .old.{next_num})")
                    archived_count += 1
            except OSError:
                pass  # Doesn't exist, ok

        if cleanup_count > 0 or archived_count > 0:
            print(f"[Cleanup] Log cleanup complete ({cleanup_count} old deleted, {archived_count} archived)")

    except Exception as e:
        print(f"[Cleanup] Error during log cleanup: {e}")
        # Don't fail boot on cleanup errors
