# Pico Kiln Controller - Multi-Threaded Architecture

## Overview

The Pico Kiln Controller uses a **dual-core, multi-threaded architecture** to take full advantage of the Raspberry Pi Pico 2's RP2350 dual-core processor. This design provides:

- **True parallelism**: Control operations run uninterrupted while the web server handles network requests
- **Real-time performance**: Critical control loop timing is unaffected by network operations
- **Fault isolation**: If the web server hangs or crashes, the kiln control continues safely
- **Clean separation**: Hardware and network layers are completely isolated

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                    Raspberry Pi Pico 2 (RP2350)                 │
├─────────────────────────────────┬───────────────────────────────┤
│          CORE 1                 │          CORE 2               │
│     (Control Thread)            │      (Main Thread)            │
│                                 │                               │
│  ┌──────────────────────────┐  │  ┌─────────────────────────┐  │
│  │  Temperature Reading     │  │  │   WiFi Management       │  │
│  │  (MAX31856 via SPI)      │  │  │   (Connection, Monitor) │  │
│  └──────────────────────────┘  │  └─────────────────────────┘  │
│              ↓                  │              ↓                │
│  ┌──────────────────────────┐  │  ┌─────────────────────────┐  │
│  │  PID Control Algorithm   │  │  │   Web Server            │  │
│  │  (Calculate SSR output)  │  │  │   (HTTP, asyncio)       │  │
│  └──────────────────────────┘  │  └─────────────────────────┘  │
│              ↓                  │              ↓                │
│  ┌──────────────────────────┐  │  ┌─────────────────────────┐  │
│  │  SSR Time-Proportional   │  │  │   API Endpoints         │  │
│  │  Control (10 Hz)         │  │  │   (Status, Control)     │  │
│  └──────────────────────────┘  │  └─────────────────────────┘  │
│              ↓                  │              ↑                │
│  ┌──────────────────────────┐  │              │                │
│  │  Profile Execution       │  │              │                │
│  │  (State Machine)         │  │              │                │
│  └──────────────────────────┘  │              │                │
│              ↓                  │              │                │
│  ┌──────────────────────────┐  │  ┌───────────┴─────────────┐  │
│  │  Safety Monitoring       │  │  │   Status Cache          │  │
│  │  (Temp limits, errors)   │  │  │   (Thread-safe)         │  │
│  └──────────────────────────┘  │  └─────────────────────────┘  │
│              │                  │              ↑                │
│              └──────────────────┼──────────────┘                │
│                 Status Queue    │                               │
│                                 │                               │
│              ┌──────────────────┼──────────────┐                │
│              │  Command Queue   │              │                │
│              └─────────┬────────┼──────────────┘                │
│                        ↓        │                               │
└────────────────────────┴────────┴───────────────────────────────┘
                         │
                    ┌────┴────┐
                    │   SSR   │ → Kiln Heating Elements
                    └─────────┘
```

## Core Responsibilities

### Core 1: Control Thread (`kiln/control_thread.py`)

**Purpose**: Time-critical hardware control with deterministic timing.

**Exclusive Access**:
- MAX31856 temperature sensor (SPI)
- SSR control pin (GPIO)
- PID controller state
- Kiln controller state machine

**Operations**:
1. **Temperature Reading** (every 1 second)
   - Read temperature from MAX31856 via SPI
   - Apply calibration offset
   - Fault detection and error handling

2. **State Machine Update**
   - Process active firing profile
   - Calculate target temperature
   - Track elapsed time and progress

3. **PID Control**
   - Calculate control output based on error
   - Anti-windup integral control
   - Output clamping (0-100%)

4. **SSR Control** (10 Hz updates)
   - Time-proportional control (slow PWM)
   - Precise timing for heating element control
   - Emergency shutoff capability

5. **Safety Monitoring**
   - Maximum temperature enforcement
   - Temperature tracking error detection
   - Fault recovery

6. **Command Processing**
   - Non-blocking queue checks
   - Handle run/stop/shutdown commands
   - PID gain updates

7. **Status Broadcasting**
   - Send status updates every 0.5s
   - Include temps, progress, PID stats

**Control Loop Timing**: 1 second per iteration (configurable via `TEMP_READ_INTERVAL`)

### Core 2: Main Thread (`main.py`, `web_server.py`)

**Purpose**: Network operations and user interface.

**Operations**:
1. **WiFi Management**
   - Initial connection with AP selection
   - Connection monitoring (every 5s)
   - Automatic reconnection on failure

2. **HTTP Web Server** (asyncio-based)
   - Non-blocking socket operations
   - Concurrent request handling
   - Static file serving (HTML interface)

3. **API Endpoints**
   - `GET /api/status` - Current system status
   - `GET /api/state` - Legacy status endpoint
   - `GET /api/info` - System information
   - `POST /api/run` - Start firing profile
   - `POST /api/stop` - Stop current profile
   - `POST /api/shutdown` - Emergency shutdown
   - `POST /api/profile` - Upload new profile
   - `GET /api/profile/<name>` - Get profile data

4. **Status Cache Management**
   - Consume status updates from Core 1
   - Thread-safe caching for quick API responses
   - No blocking on queue operations

## Inter-Thread Communication

Communication between cores uses **ThreadSafeQueue** - a custom thread-safe FIFO queue implementation built using `_thread.allocate_lock()`. This is necessary because MicroPython's standard `_thread` module doesn't include a built-in ThreadSafeQueue class.

### Command Queue (Core 2 → Core 1)

**Size**: 10 items (small, commands are infrequent)

**Message Types**:
```python
# Start a firing profile
{
    'type': 'run_profile',
    'profile_data': {...}  # Full profile dictionary
}

# Stop current profile
{
    'type': 'stop'
}

# Emergency shutdown (stop + force SSR off)
{
    'type': 'shutdown'
}
```

### Status Queue (Core 1 → Core 2)

**Size**: 100 items (larger, status updates are frequent)

**Message Format**:
```python
{
    'timestamp': 1234567890.123,
    'state': 'RUNNING',           # IDLE, RUNNING, COMPLETE, ERROR
    'current_temp': 750.5,         # Current measured temperature (°C)
    'target_temp': 800.0,          # Target temperature from profile (°C)
    'ssr_output': 65.2,            # PID output (0-100%)
    'ssr_is_on': True,             # Current SSR state
    'ssr_duty_cycle': 65.2,        # SSR duty cycle (0-100%)
    'elapsed': 1234.5,             # Time since profile start (seconds)
    'remaining': 5432.1,           # Time until profile end (seconds)
    'progress': 18.5,              # Profile progress (0-100%)
    'profile_name': 'Cone 6 Glaze',
    'error': None,                 # Error message if in ERROR state
    'pid_stats': {
        'p_term': 10.2,
        'i_term': 5.1,
        'd_term': 3.4,
        'error': 49.5,
        ...
    }
}
```

### Custom ThreadSafeQueue Implementation

Our custom `ThreadSafeQueue` class (`kiln/comms.py`) provides thread-safe FIFO queue operations:

**Features**:
- Lock-based synchronization using `_thread.allocate_lock()`
- Non-blocking `put_sync()` and `get_sync()` methods
- Raises exceptions when full/empty (no blocking)
- Simple list-based storage with lock protection

**API**:
```python
queue = ThreadSafeQueue(maxsize=10)

# Put item (raises Exception if full)
queue.put_sync(item)

# Get item (raises Exception if empty)
item = queue.get_sync()

# Query operations
queue.qsize()   # Get current size
queue.empty()   # Check if empty
queue.full()    # Check if full
queue.clear()   # Clear all items
```

**Thread Safety**: All operations are protected by a lock, ensuring safe access from multiple threads. The lock is always released (using try/finally) even if an exception occurs.

### Queue Handling Strategy

**Non-blocking Operations**:
- All queue operations use `get_sync()` / `put_sync()` with exception handling
- No thread ever blocks waiting for queue space or data
- If command queue is full, web server returns HTTP 500 error
- If status queue is full, old statuses are cleared to make room

**Graceful Degradation**:
- Control thread continues if status queue is full (drops updates)
- Web server falls back to cached status if queue is empty
- System remains operational even with queue issues

## Thread Safety

### Principles

1. **No Shared State**: All communication via queues
2. **Exclusive Hardware Access**: Only Core 1 touches hardware
3. **Immutable Messages**: Messages are dictionaries (copied, not shared)
4. **Thread-Safe Cache**: Uses locks for status cache access
5. **No Blocking**: All queue operations are non-blocking

### Safety Guarantees

✅ **Safe**:
- Core 1 has exclusive SPI bus access (no race conditions)
- Core 1 has exclusive GPIO pin access (no contention)
- Status cache uses locks for thread-safe read/write
- Queue operations are atomic (provided by MicroPython)

⚠️ **Not Thread-Safe** (by design):
- Profile objects are serialized to dictionaries before queue transmission
- No object references are shared between threads
- Each thread has its own copy of all data

## Startup Sequence

1. **Main thread (Core 2) starts**
   - Initialize status LED
   - Create ThreadSafeQueue instances (command_queue, status_queue)

2. **Launch control thread (Core 1)**
   - `_thread.start_new_thread(start_control_thread, (...))`
   - Control thread initializes hardware (SPI, GPIO, sensors)
   - Control thread enters main loop

3. **Main thread continues**
   - Wait 2 seconds for Core 1 hardware initialization
   - Connect to WiFi
   - Start web server (asyncio)
   - Start WiFi monitor (asyncio)
   - Start status updater (asyncio)

4. **System Ready**
   - Core 1: Running control loop at 1 Hz
   - Core 2: Running web server, waiting for connections
   - Both cores operating independently

## Shutdown Sequence

### Normal Shutdown (KeyboardInterrupt)
1. Main thread catches `KeyboardInterrupt`
2. Optionally send shutdown command to Core 1
3. Core 1 stops profile and forces SSR off
4. Both threads terminate

### Emergency Shutdown (Error in Control Thread)
1. Control thread catches exception
2. Force SSR off immediately
3. Set controller to ERROR state
4. Continue running (status updates report error)
5. Web server remains accessible for diagnostics

### Network Failure
- Web server crashes or hangs → Control thread unaffected
- WiFi disconnects → Control thread unaffected, WiFi monitor reconnects
- Control continues safely in all network failure scenarios

## Performance Characteristics

### Control Thread (Core 1)
- **Control loop**: 1 Hz (1 second per iteration)
- **SSR updates**: 10 Hz (0.1 second updates)
- **Status broadcasts**: 2 Hz (every 0.5 seconds)
- **Command checking**: Every control loop iteration (non-blocking)
- **CPU usage**: ~10-20% (mostly sleeping during SSR updates)

### Web Server (Core 2)
- **HTTP socket polling**: 10 Hz (0.1 second)
- **Status cache updates**: 10 Hz (0.1 second)
- **WiFi monitoring**: 0.2 Hz (every 5 seconds)
- **Request handling**: Async (multiple concurrent connections)
- **CPU usage**: ~5-10% idle, 30-50% during requests

### Latency
- **Command latency**: < 1 second (next control loop iteration)
- **Status latency**: < 0.5 seconds (next status broadcast)
- **API response time**: < 10ms (cached status)
- **Control loop jitter**: < 10ms (unaffected by network)

## Error Handling

### Hardware Errors (Core 1)
- **Sensor fault**: Return last good value, increment fault counter
- **Persistent sensor fault**: Set ERROR state, force SSR off
- **Temperature too high**: Set ERROR state, force SSR off
- **Temperature tracking error**: Set ERROR state, force SSR off

### Network Errors (Core 2)
- **WiFi disconnect**: Automatic reconnection (WiFi monitor)
- **HTTP error**: Return 500, log error, continue serving
- **Queue full**: Return 500, ask client to retry

### Queue Communication Errors
- **Command queue full**: HTTP 500 error to client
- **Status queue full**: Clear old statuses, continue
- **Queue get empty**: Return cached status

## Debugging

### Log Prefixes
- `[Control Thread]` - Messages from Core 1
- `[Web Server]` - Messages from Core 2 web server
- `[Main]` - Messages from Core 2 main thread

### Monitoring
- Watch serial output for control loop status
- Access `/api/status` for detailed system state
- Check `error` field in status for error messages

### Common Issues
1. **Control loop not starting**: Check hardware initialization errors
2. **No status updates in web UI**: Check status queue, verify control thread running
3. **Commands not working**: Check command queue, verify command format
4. **SSR not responding**: Core 1 has exclusive access, check control thread status

## Files

### Core Modules
- `main.py` - Entry point, Core 2 coordination
- `kiln/control_thread.py` - Core 1 control loop
- `web_server.py` - Core 2 HTTP server
- `kiln/comms.py` - Inter-thread communication protocol

### Kiln Logic (used by Core 1)
- `kiln/state.py` - State machine and controller
- `kiln/hardware.py` - Temperature sensor and SSR control
- `kiln/pid.py` - PID controller
- `kiln/profile.py` - Firing profile management

### Support
- `config.py` - Configuration parameters
- `wrapper.py` - CircuitPython compatibility wrappers
- `lib/` - External libraries (MAX31856 driver, etc.)

## Future Enhancements

### Possible Improvements
1. **WebSocket support**: Real-time status streaming (Core 2)
2. **Data logging**: Write temp/SSR data to flash (Core 1)
3. **Auto-tuning**: PID parameter optimization (Core 1)
4. **Multiple profiles**: Queue profiles for sequential firing (Core 1)
5. **Watchdog**: Core 2 monitors Core 1 health, restart if needed

### Performance Tuning
- Adjust queue sizes based on memory availability
- Tune status update interval for desired responsiveness
- Optimize PID gains for specific kiln characteristics

## References

- [MicroPython Threading](https://docs.micropython.org/en/latest/library/_thread.html)
- [MicroPython ThreadSafeQueue Discussion](https://github.com/orgs/micropython/discussions/9875)
- [RP2350 Datasheet](https://datasheets.raspberrypi.com/rp2350/rp2350-datasheet.pdf)
