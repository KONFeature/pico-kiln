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
from machine import Pin, SPI, WDT
from wrapper import DigitalInOut, SPIWrapper
from kiln import TemperatureSensor, SSRController, PID, KilnController, Profile
from kiln.state import KilnState
from kiln.comms import MessageType, StatusMessage, QueueHelper
from kiln.tuner import ZieglerNicholsTuner, TuningStage

class ControlThread:
    """
    Main control thread for kiln operations

    This class encapsulates all hardware control logic and runs on Core 1.
    All hardware access happens exclusively in this thread to avoid race conditions.
    """

    def __init__(self, command_queue, status_queue, config):
        """
        Initialize control thread

        Args:
            command_queue: ThreadSafeQueue for receiving commands from Core 2
            status_queue: ThreadSafeQueue for sending status updates to Core 2
            config: Configuration object with hardware and control parameters
        """
        self.command_queue = command_queue
        self.status_queue = status_queue
        self.config = config
        self.running = True

        # Hardware components (initialized in setup)
        self.temp_sensor = None
        self.ssr_controller = None
        self.pid = None
        self.controller = None
        self.ssr_pin = None
        self.wdt = None  # Watchdog timer

        # Tuning
        self.tuner = None

        # Timing
        self.last_status_update = 0
        self.status_update_interval = 0.5  # Send status updates every 0.5s

    def setup_hardware(self):
        """
        Initialize all hardware components

        This must be called from the control thread context to ensure
        exclusive hardware access.
        """
        print("[Control Thread] Initializing hardware...")

        # Setup SSR control pin
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
            spi, cs_pin, offset=self.config.THERMOCOUPLE_OFFSET
        )

        # Initialize SSR controller
        self.ssr_controller = SSRController(
            self.ssr_pin, cycle_time=self.config.SSR_CYCLE_TIME
        )

        # Initialize PID controller
        self.pid = PID(
            kp=self.config.PID_KP,
            ki=self.config.PID_KI,
            kd=self.config.PID_KD,
            output_limits=(0, 100)
        )

        # Initialize kiln controller
        self.controller = KilnController(
            max_temp=self.config.MAX_TEMP,
            max_temp_error=self.config.MAX_TEMP_ERROR
        )

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

                try:
                    profile = Profile.load_from_file(f"profiles/{profile_filename}")
                    self.controller.run_profile(profile)
                    print(f"[Control Thread] Started profile: {profile.name}")
                except Exception as e:
                    print(f"[Control Thread] Error loading profile '{profile_filename}': {e}")
                    self.controller.set_error(f"Failed to load profile: {e}")

            elif cmd_type == MessageType.RESUME_PROFILE:
                # Resume a previously interrupted profile
                profile_filename = command.get('profile_filename')
                elapsed_seconds = command.get('elapsed_seconds', 0)

                if not profile_filename:
                    print("[Control Thread] Error: No profile filename in resume_profile command")
                    return

                try:
                    profile = Profile.load_from_file(f"profiles/{profile_filename}")
                    self.controller.resume_profile(profile, elapsed_seconds)
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

            elif cmd_type == MessageType.SET_PID_GAINS:
                # Update PID gains
                kp = command.get('kp')
                ki = command.get('ki')
                kd = command.get('kd')
                self.pid.set_gains(kp=kp, ki=ki, kd=kd)
                print(f"[Control Thread] PID gains updated: kp={kp}, ki={ki}, kd={kd}")

            elif cmd_type == MessageType.START_TUNING:
                # Start PID auto-tuning
                target_temp = command.get('target_temp', 200)
                if self.controller.state != KilnState.IDLE:
                    print(f"[Control Thread] Cannot start tuning: kiln is in {self.controller.state} state")
                    return

                print(f"[Control Thread] Starting PID auto-tuning (target: {target_temp}°C)")
                self.tuner = ZieglerNicholsTuner(target_temp=target_temp)
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
        try:
            # Choose status builder based on state
            if self.controller.state == KilnState.TUNING and self.tuner:
                status = StatusMessage.build_tuning_status(self.controller, self.tuner)
            else:
                status = StatusMessage.build(self.controller, self.pid, self.ssr_controller)

            # Try to send (non-blocking)
            if not QueueHelper.put_nowait(self.status_queue, status):
                # Queue full - clear old statuses and try again
                cleared = QueueHelper.clear(self.status_queue)
                if cleared > 0:
                    print(f"[Control Thread] Cleared {cleared} old status messages")
                QueueHelper.put_nowait(self.status_queue, status)

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
            self.ssr_controller.set_output(ssr_output)

            # 5. Check if tuning is complete or errored
            if not continue_tuning:
                if self.tuner.stage == TuningStage.COMPLETE:
                    # Save results
                    try:
                        self.tuner.save_results("tuning_results.json")
                        self.tuner.save_csv("tuning_data.csv")
                        print("[Control Thread] Tuning complete - results saved")
                    except Exception as e:
                        print(f"[Control Thread] Error saving tuning results: {e}")

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
            update_count = int(self.config.TEMP_READ_INTERVAL / 0.1)  # 10 Hz updates
            for _ in range(update_count):
                self.ssr_controller.update()
                time.sleep(0.1)

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

            # 4. Calculate PID output
            if self.controller.state == KilnState.RUNNING:
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
            update_count = int(self.config.TEMP_READ_INTERVAL / 0.1)  # 10 Hz updates
            for _ in range(update_count):
                self.ssr_controller.update()
                time.sleep(0.1)

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


def start_control_thread(command_queue, status_queue, config):
    """
    Thread entry point - starts the control loop

    This function is called by _thread.start_new_thread() to start
    the control thread on Core 1.

    Args:
        command_queue: ThreadSafeQueue for receiving commands
        status_queue: ThreadSafeQueue for sending status updates
        config: Configuration object
    """
    control = ControlThread(command_queue, status_queue, config)
    control.run()
