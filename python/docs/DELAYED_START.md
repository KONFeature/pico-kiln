# Delayed Start Feature

This document describes the delayed start feature for scheduling kiln profiles to start at a specific time.

## Architecture

The delayed start feature uses a clean, queue-based architecture with these components:

### Core Components

1. **ScheduledProfileQueue** (`kiln/scheduler.py`)
   - Thread-safe queue for managing ONE scheduled profile
   - Provides `can_consume()` / `consume()` interface for control loop
   - Supports status queries and cancellation

2. **Control Thread Integration** (`kiln/control_thread.py`)
   - Checks scheduler every control loop iteration (when IDLE)
   - Automatically starts profile when scheduled time arrives
   - Handles SCHEDULE_PROFILE and CANCEL_SCHEDULED commands

3. **Web API Endpoints** (`server/web_server.py`)
   - `POST /api/schedule` - Schedule a profile
   - `GET /api/scheduled` - Get scheduled profile status
   - `POST /api/scheduled/cancel` - Cancel scheduled profile

4. **Status Updates** (`kiln/comms.py`)
   - Scheduler status flows through existing status system
   - Includes profile name, start time, and countdown

## API Usage

### Schedule a Profile

```bash
curl -X POST http://KILN_IP/api/schedule \
  -H "Content-Type: application/json" \
  -d '{
    "profile": "biscuit_faience_adaptive",
    "start_time": 1731456000
  }'
```

Response:
```json
{
  "success": true,
  "message": "Scheduled profile: biscuit_faience_adaptive"
}
```

**Parameters:**
- `profile`: Profile name (without .json extension)
- `start_time`: Unix timestamp (must be in the future)

### Get Scheduled Profile Status

```bash
curl http://KILN_IP/api/scheduled
```

Response when scheduled:
```json
{
  "scheduled": true,
  "profile": "biscuit_faience_adaptive.json",
  "start_time": 1731456000,
  "start_time_iso": "2025-11-12 22:00:00",
  "seconds_until_start": 3600
}
```

Response when nothing scheduled:
```json
{
  "scheduled": false
}
```

### Cancel Scheduled Profile

```bash
curl -X POST http://KILN_IP/api/scheduled/cancel
```

Response:
```json
{
  "success": true,
  "message": "Cancelled scheduled profile"
}
```

## Status in /api/status

The regular status endpoint also includes scheduler information:

```json
{
  "state": "IDLE",
  "current_temp": 25.0,
  "scheduled_profile": {
    "profile_filename": "biscuit_faience_adaptive.json",
    "start_time": 1731456000,
    "start_time_iso": "2025-11-12 22:00:00",
    "seconds_until_start": 3600
  },
  ...
}
```

When nothing is scheduled, `scheduled_profile` is `null`.

## Design Decisions

### Why Queue-Based?

The queue-based approach keeps scheduling logic completely separate from execution logic:
- **Clean Separation**: Scheduler doesn't know about KilnController internals
- **Single Responsibility**: Each component has one job
- **Easy Testing**: Can test scheduler independently
- **No State Pollution**: KilnController stays focused on execution

### Why Only One Scheduled Profile?

This is a deliberate simplification that matches typical kiln usage:
- Kilns typically run one program at a time
- Simplifies UI and reduces confusion
- Makes error handling straightforward
- Can be extended later if needed

### How Does It Work?

1. User schedules a profile via web API
2. Web server sends SCHEDULE_PROFILE command to control thread
3. Control thread stores it in scheduler queue
4. Every control loop iteration (when IDLE), control thread checks `scheduler.can_consume()`
5. When start time arrives, profile is consumed and started automatically
6. Scheduler status is included in regular status updates

## Implementation Files

### New Files
- `kiln/scheduler.py` - ScheduledProfileQueue and ScheduledProfile classes

### Modified Files
- `kiln/comms.py` - Added SCHEDULE_PROFILE/CANCEL_SCHEDULED message types
- `kiln/control_thread.py` - Added scheduler instance and checking logic
- `server/web_server.py` - Added scheduling API endpoints

## Future Enhancements

Potential improvements for the future:

1. **UI Integration**: Add time picker to web interface
2. **Recovery Support**: Resume scheduled profiles after system restart
3. **Multiple Profiles**: Support queuing multiple profiles (if needed)
4. **Notifications**: Send alerts when profile is about to start
5. **Validation**: Warn if NTP hasn't synced (clock accuracy)

## Testing

To test the delayed start feature:

1. **Schedule a profile 2 minutes in future:**
   ```python
   import time
   start_time = int(time.time()) + 120  # 2 minutes from now
   ```

2. **Monitor status:**
   ```bash
   watch -n 1 'curl -s http://KILN_IP/api/scheduled | jq'
   ```

3. **Watch it start automatically:**
   - Profile will start when countdown reaches 0
   - Status will show state transition from IDLE to RUNNING

4. **Test cancellation:**
   ```bash
   curl -X POST http://KILN_IP/api/scheduled/cancel
   ```
