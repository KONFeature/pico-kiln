# kiln/control_thread.py
# Control thread implementation for Core 1
#
# This module runs the main control loop on a dedicated thread (Core 1).
# It has exclusive access to all hardware (temperature sensor, SSR, pins)
# and communicates with the web server via thread-safe queues.
#
# IMPORTANT: This thread must be started using _thread.start_new_thread()
# and must receive ThreadSafeQueue instances for communication.

import time
import micropython
from machine import Pin, SPI, WDT
from wrapper import DigitalInOut, SPIWrapper
from kiln import TemperatureSensor, SSRController, PID, KilnController, Profile
from kiln.state import KilnState
from kiln.comms import MessageType, StatusMessage, QueueHelper
from kiln.tuner import ZieglerNicholsTuner, TuningStage
from micropython import const

# Performance: const() declarations for hot path time intervals
STATUS_UPDATE_INTERVAL = const(2)  # Status updates every 2 seconds (integer for const)
SSR_UPDATE_INTERVAL = 0.1  # 100ms between SSR state updates (10 Hz)

class ControlThread:
    """
    Main control thread for kiln operations

    This class encapsulates all hardware control logic and runs on Core 1.
    All hardware access happens exclusively in this thread to avoid race conditions.
    """

    def __init__(self, command_queue, status_queue, config, error_log=None, ready_flag=None, quiet_mode=None):
        """
        Initialize control thread

        Args:
            command_queue: ThreadSafeQueue for receiving commands from Core 2
            status_queue: ThreadSafeQueue for sending status updates to Core 2
            config: Configuration object with hardware and control parameters
            error_log: ErrorLog instance for cross-core error logging (optional)
            ready_flag: ReadyFlag for signaling Core 2 when hardware is ready (optional)
            quiet_mode: QuietMode for suppressing status updates during boot (optional)
        """
        self.command_queue = command_queue
        self.status_queue = status_queue
        self.config = config
        self.error_log = error_log
        self.ready_flag = ready_flag
        self.quiet_mode = quiet_mode
        self.running = True

        # Hardware components (initialized in setup)
        self.temp_sensor = None
        self.ssr_controller = None
        self.pid = None
        self.pid_scheduler = None  # Gain scheduling
        self.controller = None
        self.ssr_pin = None
        self.wdt = None  # Watchdog timer

        # Tuning
        self.tuner = None

        # Timing
        self.last_status_update = 0
        self.status_update_interval = STATUS_UPDATE_INTERVAL

        # Fallback status storage for queue full edge cases
        self.last_status_fallback = None

    def setup_hardware(self):
        """
        Initialize all hardware components

        This must be called from the control thread context to ensure
        exclusive hardware access.
        """
        print("[Control Thread] Initializing hardware...")

        # Setup SSR control pin(s)
        # Support both single pin (int) and multiple pins (list) for backward compatibility
        if isinstance(self.config.SSR_PIN, list):
            if len(self.config.SSR_PIN) == 0:
                raise ValueError("SSR_PIN list cannot be empty - at least one SSR pin is required")
            self.ssr_pin = [Pin(pin, Pin.OUT) for pin in self.config.SSR_PIN]
            for pin in self.ssr_pin:
                pin.value(0)  # Start with all SSRs off
            print(f"[Control Thread] {len(self.ssr_pin)} SSR pins initialized on GPIO {self.config.SSR_PIN}")
        else:
            self.ssr_pin = Pin(self.config.SSR_PIN, Pin.OUT)
            self.ssr_pin.value(0)  # Start with SSR off
            print(f"[Control Thread] SSR pin initialized on GPIO {self.config.SSR_PIN}")

        # Setup SPI for MAX31856
        print(f"[Control Thread] Initializing MAX31856 on SPI{self.config.MAX31856_SPI_ID}")
        spi = SPIWrapper(
            SPI(
                self.config.MAX31856_SPI_ID,
                baudrate=1000000,
                polarity=0,  # MAX31856 requires SPI Mode 1
                phase=1,     # CPOL=0, CPHA=1
                sck=Pin(self.config.MAX31856_SCK_PIN),
                mosi=Pin(self.config.MAX31856_MOSI_PIN),
                miso=Pin(self.config.MAX31856_MISO_PIN),
            )
        )

        cs_pin = DigitalInOut(Pin(self.config.MAX31856_CS_PIN, Pin.OUT))

        # Initialize temperature sensor
        self.temp_sensor = TemperatureSensor(
            spi, cs_pin, thermocouple_type=self.config.THERMOCOUPLE_TYPE, offset=self.config.THERMOCOUPLE_OFFSET, error_log=self.error_log
        )

        # Initialize SSR controller
        stagger_delay = getattr(self.config, 'SSR_STAGGER_DELAY', 0.01)
        self.ssr_controller = SSRController(
            self.ssr_pin,
            cycle_time=self.config.SSR_CYCLE_TIME,
            stagger_delay=stagger_delay,
            error_log=self.error_log
        )

        # Get base PID gains and thermal parameters
        self.pid_kp_base = getattr(self.config, 'PID_KP_BASE', 25.0)
        self.pid_ki_base = getattr(self.config, 'PID_KI_BASE', 0.18)
        self.pid_kd_base = getattr(self.config, 'PID_KD_BASE', 160.0)
        self.thermal_h = getattr(self.config, 'THERMAL_H', 0.0)
        self.thermal_t_ambient = getattr(self.config, 'THERMAL_T_AMBIENT', 25.0)

        # Validate THERMAL_H
        if self.thermal_h < 0:
            print(f"[Control Thread] WARNING: THERMAL_H={self.thermal_h} is negative - setting to 0")
            print(f"[Control Thread] Heat loss coefficient must be non-negative")
            self.thermal_h = 0.0
        elif self.thermal_h > 0.1:
            print(f"[Control Thread] WARNING: THERMAL_H={self.thermal_h} is very large")
            print(f"[Control Thread] Typical range: 0.0001 to 0.01 - may cause control instability")
            print(f"[Control Thread] Proceeding anyway, but monitor carefully")

        # Initialize PID controller with base gains
        self.pid = PID(
            kp=self.pid_kp_base,
            ki=self.pid_ki_base,
            kd=self.pid_kd_base,
            output_limits=(0, 100)
        )

        # Track current gains for change detection
        self._current_kp = self.pid_kp_base
        self._current_ki = self.pid_ki_base
        self._current_kd = self.pid_kd_base

        # Print continuous gain scheduling status
        if self.thermal_h > 0:
            print(f"[Control Thread] Continuous gain scheduling ENABLED (h={self.thermal_h:.6f})")
            print(f"[Control Thread] Base PID: Kp={self.pid_kp_base:.3f} Ki={self.pid_ki_base:.4f} Kd={self.pid_kd_base:.3f}")
        else:
            print(f"[Control Thread] Continuous gain scheduling DISABLED (constant gains)")
            print(f"[Control Thread] PID: Kp={self.pid_kp_base:.3f} Ki={self.pid_ki_base:.4f} Kd={self.pid_kd_base:.3f}")

        # Initialize kiln controller (pass config for adaptive control parameters)
        self.controller = KilnController(self.config)

        # Initialize watchdog timer (if enabled)
        if self.config.ENABLE_WATCHDOG:
            try:
                # Initialize hardware watchdog with configured timeout
                self.wdt = WDT(timeout=self.config.WATCHDOG_TIMEOUT)
                print(f"[Control Thread] Watchdog ENABLED with {self.config.WATCHDOG_TIMEOUT}ms timeout")
                print(f"[Control Thread] WARNING: Device will auto-reset if control loop hangs!")
            except Exception as e:
                print(f"[Control Thread] WARNING: Failed to enable watchdog: {e}")
                self.wdt = None
        else:
            print("[Control Thread] Watchdog DISABLED")

        print("[Control Thread] All hardware initialized successfully")

        # Signal Core 2 that we're ready
        if self.ready_flag:
            self.ready_flag.set_ready()
            print("[Control Thread] Ready flag set - Core 2 can proceed")

    def load_profile_with_retry(self, filename, max_attempts=3):
        """
        Load profile from file with retry logic

        Implements exponential backoff retry to handle transient filesystem errors.
        This is critical for long-running operations where a single filesystem
        glitch shouldn't abort the entire program.

        Args:
            filename: Profile filename (with path, e.g., "profiles/cone6.json")
            max_attempts: Maximum number of retry attempts (default: 3)

        Returns:
            Profile object if successful

        Raises:
            Exception: If all retry attempts fail
        """
        for attempt in range(max_attempts):
            try:
                profile = Profile.load_from_file(filename)
                if attempt > 0:
                    print(f"[Control Thread] Profile loaded successfully after {attempt + 1} attempts")
                return profile
            except Exception as e:
                if attempt < max_attempts - 1:
                    backoff_time = 0.5 * (attempt + 1)  # Exponential backoff: 0.5s, 1.0s
                    print(f"[Control Thread] Profile load attempt {attempt + 1}/{max_attempts} failed: {e}")
                    print(f"[Control Thread] Retrying in {backoff_time:.1f}s...")
                    time.sleep(backoff_time)
                else:
                    # All retries exhausted
                    print(f"[Control Thread] Profile load failed after {max_attempts} attempts")
                    raise

    def handle_command(self, command):
        """
        Process command from Core 2

        Args:
            command: Command dictionary from command_queue
        """
        cmd_type = command.get('type')

        try:
            if cmd_type == MessageType.RUN_PROFILE:
                # Start running a profile
                profile_filename = command.get('profile_filename')
                if not profile_filename:
                    print("[Control Thread] Error: No profile filename in run_profile command")
                    return

                # Safety check: cannot start new profile if already running
                if self.controller.state == KilnState.RUNNING:
                    print("[Control Thread] Cannot start profile: kiln is already running")
                    print("[Control Thread] Stop current profile first")
                    return

                if self.controller.state == KilnState.TUNING:
                    print("[Control Thread] Cannot start profile: tuning is in progress")
                    print("[Control Thread] Stop tuning first")
                    return

                try:
                    profile = self.load_profile_with_retry(f"profiles/{profile_filename}")
                    self.controller.run_profile(profile)
                    print(f"[Control Thread] Started profile: {profile.name}")
                except Exception as e:
                    print(f"[Control Thread] Error loading profile '{profile_filename}': {e}")
                    self.controller.set_error(f"Failed to load profile: {e}")

            elif cmd_type == MessageType.RESUME_PROFILE:
                # Resume a previously interrupted profile
                profile_filename = command.get('profile_filename')
                elapsed_seconds = command.get('elapsed_seconds', 0)
                current_rate = command.get('current_rate')  # Adapted rate from recovery
                last_logged_temp = command.get('last_logged_temp')  # For recovery detection
                current_temp = command.get('current_temp')  # For recovery detection

                if not profile_filename:
                    print("[Control Thread] Error: No profile filename in resume_profile command")
                    return

                try:
                    profile = self.load_profile_with_retry(f"profiles/{profile_filename}")
                    self.controller.resume_profile(profile, elapsed_seconds, current_rate, last_logged_temp, current_temp)
                    print(f"[Control Thread] Resumed profile: {profile.name} at {elapsed_seconds:.1f}s")
                except Exception as e:
                    print(f"[Control Thread] Error loading profile '{profile_filename}': {e}")
                    self.controller.set_error(f"Failed to load profile: {e}")

            elif cmd_type == MessageType.STOP:
                # Stop current profile
                self.controller.stop()
                self.ssr_controller.force_off()
                print("[Control Thread] Profile stopped")

            elif cmd_type == MessageType.SHUTDOWN:
                # Emergency shutdown
                self.controller.stop()
                self.ssr_controller.force_off()
                print("[Control Thread] Emergency shutdown executed")

            elif cmd_type == MessageType.START_TUNING:
                # Start PID auto-tuning
                mode = command.get('mode', 'STANDARD')
                max_temp = command.get('max_temp')  # None = use mode default

                if self.controller.state != KilnState.IDLE:
                    print(f"[Control Thread] Cannot start tuning: kiln is in {self.controller.state} state")
                    if self.controller.state == KilnState.TUNING and self.tuner:
                        print(f"[Control Thread] Tuning already in progress: mode={self.tuner.mode}, elapsed={time.time() - self.tuner.start_time:.1f}s")
                    return

                # Extra safety: ensure no leftover tuner object
                if self.tuner is not None:
                    print(f"[Control Thread] WARNING: Cleaning up leftover tuner object before starting new tuning")
                    self.tuner = None

                print(f"[Control Thread] Starting PID auto-tuning (mode: {mode}, max_temp: {max_temp}°C)")
                self.tuner = ZieglerNicholsTuner(mode=mode, max_temp=max_temp)
                self.tuner.start()
                self.controller.state = KilnState.TUNING
                print("[Control Thread] Tuning started")

            elif cmd_type == MessageType.STOP_TUNING:
                # Stop tuning
                if self.controller.state == KilnState.TUNING:
                    print("[Control Thread] Tuning stopped by user")
                    self.controller.state = KilnState.IDLE
                    self.tuner = None
                    self.ssr_controller.force_off()

            elif cmd_type == MessageType.PING:
                # Ping message for testing
                print("[Control Thread] Received ping")

            else:
                print(f"[Control Thread] Unknown command type: {cmd_type}")

        except Exception as e:
            print(f"[Control Thread] Error handling command {cmd_type}: {e}")
            # Set error state on controller
            self.controller.set_error(f"Command error: {e}")

    def feed_watchdog(self):
        """
        Feed the watchdog timer to prevent system reset

        This should be called at the end of each successful control loop iteration.
        If not called within WATCHDOG_TIMEOUT milliseconds, the device will reset.
        """
        if self.wdt:
            self.wdt.feed()

    def send_status_update(self):
        """
        Build and send status update to Core 2

        This is called periodically to update the web server with current status
        """
        # Check quiet mode - suppress status updates during boot WiFi phase
        if self.quiet_mode and self.quiet_mode.is_quiet():
            return

        try:
            # Safety check: if state is TUNING but tuner is None, fix the inconsistency
            if self.controller.state == KilnState.TUNING and not self.tuner:
                print("[Control Thread] WARNING: State is TUNING but tuner is None - fixing state to IDLE")
                self.controller.state = KilnState.IDLE
                self.ssr_controller.force_off()

            # Choose status builder based on state
            if self.controller.state == KilnState.TUNING and self.tuner:
                status = StatusMessage.build_tuning_status(self.controller, self.tuner, self.ssr_controller)
            else:
                status = StatusMessage.build(self.controller, self.pid, self.ssr_controller)

            # Check queue size before sending (monitor Core 2 health)
            # Only warn if queue is completely full to minimize USB contention
            queue_size = self.status_queue.qsize()
            if queue_size >= 95:  # Queue is 95%+ full - very bad
                try:
                    # Minimal print to avoid USB contention between cores
                    print("[Control Thread] Queue near full!")
                except Exception:
                    # If printing fails, just continue - don't crash
                    pass

            # Try to send (non-blocking)
            if not QueueHelper.put_nowait(self.status_queue, status):
                # Queue full - clear old statuses and try again
                cleared = QueueHelper.clear(self.status_queue)
                if cleared > 0:
                    # Minimal logging to avoid USB contention
                    print("[Control Thread] CRITICAL: Queue cleared - Core 2 not consuming!")

                # Try one more time after clearing
                if not QueueHelper.put_nowait(self.status_queue, status):
                    # Still failed - store in fallback and continue
                    # This indicates Core 2 is likely compromised, but Core 1 continues
                    self.last_status_fallback = status
                    print("[Control Thread] WARNING: Status stored in fallback - queue still full")
                else:
                    # Successfully sent after clearing
                    self.last_status_fallback = None

        except Exception as e:
            print(f"[Control Thread] Error sending status: {e}")

    def tuning_loop_iteration(self):
        """
        Single iteration of the tuning loop

        This handles PID auto-tuning logic:
        1. Check for commands (allow stop)
        2. Read temperature
        3. Update tuner state
        4. Set SSR output based on tuner
        5. Check for completion/error
        6. Send status update
        """
        try:
            # 1. Check for commands (non-blocking)
            command = QueueHelper.get_nowait(self.command_queue)
            if command:
                self.handle_command(command)

            # 2. Read temperature
            current_temp = self.temp_sensor.read()
            self.controller.current_temp = current_temp

            # 3. Safety check
            if current_temp > self.config.MAX_TEMP:
                self.controller.set_error(f"Temperature {current_temp:.1f}C exceeds maximum {self.config.MAX_TEMP}C")
                self.controller.state = KilnState.ERROR
                self.tuner = None
                self.ssr_controller.force_off()
                return

            # 4. Update tuner and get SSR output
            ssr_output, continue_tuning = self.tuner.update(current_temp)

            # Store SSR output and target temp in controller for status reporting
            self.controller.ssr_output = ssr_output
            if self.tuner.current_step and self.tuner.current_step.target_temp:
                self.controller.target_temp = self.tuner.current_step.target_temp
            else:
                self.controller.target_temp = 0

            self.ssr_controller.set_output(ssr_output)

            # 5. Check if tuning is complete or errored
            if not continue_tuning:
                if self.tuner.stage == TuningStage.COMPLETE:
                    # Tuning complete - data has been streamed to CSV on Core 2
                    print("[Control Thread] Tuning complete - data saved to CSV")
                    print("[Control Thread] Use analyze_tuning.py to calculate PID parameters")
                    self.controller.state = KilnState.IDLE
                elif self.tuner.stage == TuningStage.ERROR:
                    print(f"[Control Thread] Tuning error: {self.tuner.error_message}")
                    self.controller.state = KilnState.ERROR
                    self.controller.error_message = self.tuner.error_message

                self.tuner = None
                self.ssr_controller.force_off()

            # 6. Send status update (periodically)
            current_time = time.time()
            if current_time - self.last_status_update >= self.status_update_interval:
                self.send_status_update()
                self.last_status_update = current_time

            # 7. Update SSR state multiple times during control interval
            update_count = int(self.config.TEMP_READ_INTERVAL / SSR_UPDATE_INTERVAL)  # 10 Hz updates
            for _ in range(update_count):
                self.ssr_controller.update()
                time.sleep(SSR_UPDATE_INTERVAL)

            # 8. Feed watchdog - tuning loop iteration completed successfully
            self.feed_watchdog()

        except Exception as e:
            print(f"[Control Thread] Tuning loop error: {e}")
            # Emergency shutdown on error
            if self.ssr_controller:
                self.ssr_controller.force_off()
            if self.controller:
                self.controller.set_error(str(e))
            self.tuner = None
            # NOTE: Do NOT feed watchdog on error - let it reset if we're stuck in error loop
            time.sleep(1)

    # Performance optimization: Main control loop orchestrating all operations at 1 Hz
    @micropython.native
    def control_loop_iteration(self):
        """
        Single iteration of the control loop

        This implements the core control logic:
        1. Check for commands
        2. Read temperature
        3. Update controller state
        4. Calculate PID output
        5. Set SSR output
        6. Send status update (periodically)
        """
        try:
            # Check if we're in tuning mode
            if self.controller.state == KilnState.TUNING:
                self.tuning_loop_iteration()
                return

            # 1. Check for commands (non-blocking)
            command = QueueHelper.get_nowait(self.command_queue)
            if command:
                self.handle_command(command)

            # 2. Read temperature
            current_temp = self.temp_sensor.read()

            # 3. Update controller state and get target temperature
            target_temp = self.controller.update(current_temp)

            # 3.5. Check if PID reset was requested (e.g., after rate adaptation)
            if self.controller.pid_reset_requested:
                self.pid.reset()
                self.controller.pid_reset_requested = False
                print("[Control Thread] PID reset after rate adaptation")

            # 4. Calculate PID output
            if self.controller.state == KilnState.RUNNING:
                # Continuous gain scheduling based on temperature
                # Physics: gain_scale(T) = 1 + h*(T - T_ambient)
                # This compensates for increased heat loss at higher temperatures
                # Cache thermal attributes (hot path optimization - called every control loop)
                thermal_h = self.thermal_h
                if thermal_h > 0:
                    thermal_t_ambient = self.thermal_t_ambient
                    pid_kp_base = self.pid_kp_base
                    pid_ki_base = self.pid_ki_base
                    pid_kd_base = self.pid_kd_base

                    gain_scale = 1.0 + thermal_h * (current_temp - thermal_t_ambient)
                    kp = pid_kp_base * gain_scale
                    ki = pid_ki_base * gain_scale
                    kd = pid_kd_base * gain_scale

                    # Only update gains if they changed significantly (absolute threshold)
                    # Using absolute thresholds avoids division by zero
                    if (abs(kp - self._current_kp) > 0.01 or
                        abs(ki - self._current_ki) > 0.0001 or
                        abs(kd - self._current_kd) > 0.01):
                        self.pid.set_gains(kp, ki, kd)
                        self._current_kp = kp
                        self._current_ki = ki
                        self._current_kd = kd
                        print(f"[Control Thread] PID gains updated: Kp={kp:.3f} Ki={ki:.4f} Kd={kd:.3f} @ {current_temp:.1f}°C (scale={gain_scale:.3f})")

                # PID control active
                ssr_output = self.pid.update(target_temp, current_temp)
            else:
                # Not running - turn off SSR
                ssr_output = 0
                self.pid.reset()

            self.controller.ssr_output = ssr_output
            self.ssr_controller.set_output(ssr_output)

            # 5. Safety check: force SSR off in error state
            if self.controller.state == KilnState.ERROR:
                self.ssr_controller.force_off()
                print(f"[Control Thread] ERROR STATE: {self.controller.error_message}")

            # 6. Log status (if not idle)
            if self.controller.state != KilnState.IDLE:
                elapsed = self.controller.get_elapsed_time()
                print(f"[Control Thread] [{elapsed:.0f}s] State:{self.controller.state} Temp:{current_temp:.1f}°C Target:{target_temp:.1f}°C SSR:{ssr_output:.1f}%")

            # 7. Send status update (periodically)
            current_time = time.time()
            if current_time - self.last_status_update >= self.status_update_interval:
                self.send_status_update()
                self.last_status_update = current_time

            # 8. Update SSR state multiple times during control interval
            # This provides better time-proportional control resolution
            update_count = int(self.config.TEMP_READ_INTERVAL / SSR_UPDATE_INTERVAL)  # 10 Hz updates
            for _ in range(update_count):
                self.ssr_controller.update()
                time.sleep(SSR_UPDATE_INTERVAL)

            # 9. Feed watchdog - control loop iteration completed successfully
            self.feed_watchdog()

        except Exception as e:
            print(f"[Control Thread] Control loop error: {e}")
            # Emergency shutdown on error
            if self.ssr_controller:
                self.ssr_controller.force_off()
            if self.controller:
                self.controller.set_error(str(e))
            # NOTE: Do NOT feed watchdog on error - let it reset if we're stuck in error loop
            time.sleep(1)

    def run(self):
        """
        Main control loop - runs continuously on Core 1

        This is the entry point for the control thread.
        """
        print("[Control Thread] Starting control loop...")

        # Initialize hardware
        try:
            self.setup_hardware()
        except Exception as e:
            print(f"[Control Thread] FATAL: Hardware initialization failed: {e}")
            return

        # Main loop
        print("[Control Thread] Control loop running")
        while self.running:
            self.control_loop_iteration()

        # Cleanup on exit
        print("[Control Thread] Shutting down...")
        if self.ssr_controller:
            self.ssr_controller.force_off()
        print("[Control Thread] Stopped")

    def stop(self):
        """Request control thread to stop"""
        self.running = False


def start_control_thread(command_queue, status_queue, config, error_log=None, ready_flag=None, quiet_mode=None):
    """
    Thread entry point - starts the control loop

    This function is called by _thread.start_new_thread() to start
    the control thread on Core 1.

    Args:
        command_queue: ThreadSafeQueue for receiving commands
        status_queue: ThreadSafeQueue for sending status updates
        config: Configuration object
        error_log: ErrorLog instance for cross-core error logging (optional)
        ready_flag: ReadyFlag for signaling Core 2 when hardware is ready (optional)
        quiet_mode: QuietMode for suppressing status updates during boot (optional)
    """
    control = ControlThread(command_queue, status_queue, config, error_log, ready_flag, quiet_mode)
    control.run()
