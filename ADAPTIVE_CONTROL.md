# Adaptive Rate Control Implementation

## Overview

Adaptive rate control allows the kiln controller to automatically adjust ramp rates when the kiln cannot maintain the desired heating rate. This prevents failures from aggressive profiles while still maintaining safety limits.

## What Changed

### 1. New Step-Based Profile Format

Profiles now use explicit steps instead of time-temperature pairs:

```json
{
  "name": "Biscuit Faience Adaptive",
  "temp_units": "c",
  "description": "Bisque firing with adaptive control",
  "steps": [
    {
      "type": "ramp",
      "target_temp": 600,
      "desired_rate": 100,
      "min_rate": 80
    },
    {
      "type": "hold",
      "target_temp": 600,
      "duration": 600
    },
    {
      "type": "ramp",
      "target_temp": 1050,
      "desired_rate": 100,
      "min_rate": 50
    },
    {
      "type": "ramp",
      "target_temp": 100
    }
  ]
}
```

**Ramp Steps:**
- `target_temp` (required): Target temperature in °C
- `desired_rate` (optional): Target rate in °C/hour
- `min_rate` (optional): Minimum acceptable rate - below this triggers failure

**Hold Steps:**
- `target_temp` (required): Temperature to maintain
- `duration` (required): How long to hold in seconds

**Backward Compatibility:** Old `data: [[time, temp], ...]` format still works! It auto-converts to steps with conservative min_rate = 70% of implied rate.

### 2. Adaptive Control Algorithm

**When does adaptation occur?**

Adaptation triggers when ALL conditions are met:
- At least 10 minutes into the step
- At least 5 minutes since last adaptation
- Temperature is >20°C behind schedule
- Actual rate < 85% of current target rate

**What happens during adaptation?**

1. Measures actual rate over last 10 minutes
2. Reduces target rate to 90% of measured rate
3. Checks if new rate >= min_rate
   - ✅ If yes: Continue with reduced rate
   - ❌ If no: **FAIL** with clear error message

**Multiple adaptations:** Can adapt multiple times per step, progressively slowing down until either succeeding or hitting min_rate limit.

### 3. Safety Features

- **Cooldown monitoring:** Detects if temperature increases during cooldown → emergency shutdown
- **Recovery-aware:** Saves adapted rate to CSV, restores it after crash/reboot
- **Memory efficient:** Only ~1KB RAM overhead for rate monitoring
- **Clear error messages:** Tells you exactly why it failed and what to do

### 4. Configuration Parameters

All in `config.py`:

```python
# === Adaptive Rate Control ===
ADAPTATION_ENABLED = True              # Enable/disable adaptive control
ADAPTATION_CHECK_INTERVAL = 60         # Check every minute
ADAPTATION_MIN_STEP_TIME = 600         # Wait 10 min before first adaptation
ADAPTATION_MIN_TIME_BETWEEN = 300      # Wait 5 min between adaptations
ADAPTATION_TEMP_ERROR_THRESHOLD = 20   # Trigger if 20°C behind
ADAPTATION_RATE_THRESHOLD = 0.85       # Trigger if actual < 85% of target
ADAPTATION_REDUCTION_FACTOR = 0.9      # Reduce to 90% of measured rate
RATE_MEASUREMENT_WINDOW = 600          # Measure rate over 10 minutes
RATE_RECORDING_INTERVAL = 10           # Record temp every 10 seconds
```

## New CSV Log Format

The `current_rate_c_per_hour` column was added:

```
timestamp,elapsed_seconds,current_temp_c,target_temp_c,ssr_output_percent,state,progress_percent,step_name,step_index,total_steps,current_rate_c_per_hour
2025-11-02 14:30:00,1800.0,350.50,365.25,98.50,RUNNING,15.2,,,2,100.0
2025-11-02 14:31:00,1860.0,352.80,366.50,99.00,RUNNING,15.5,,,2,85.5
```

This enables:
- Tracking adaptation history
- Plotting rate changes over time
- Restoring adapted rate after recovery

## Status API Enhancements

New fields in status response:

```python
{
    'current_step': 2,           # Current step number
    'total_steps': 5,            # Total steps in profile
    'step_type': 'ramp',         # 'ramp' or 'hold'
    'desired_rate': 100.0,       # Original target rate
    'current_rate': 85.5,        # Adapted rate (may differ from desired)
    'actual_rate': 87.2,         # Measured rate over last 10 min
    'min_rate': 80.0,            # Minimum acceptable rate
    'adaptation_count': 2        # How many adaptations occurred
}
```

## Files Modified

### Core Implementation
- `kiln/rate_monitor.py` (NEW) - Temperature rate monitoring
- `kiln/profile.py` - Step-based format + auto-conversion
- `kiln/state.py` - Adaptive control logic
- `kiln/control_thread.py` - Pass config to controller

### Logging & Recovery
- `server/data_logger.py` - Add current_rate column
- `server/recovery.py` - Parse and restore current_rate
- `kiln/comms.py` - Add current_rate to resume message

### Configuration
- `config.py` - Adaptive control parameters
- `config.example.py` - Adaptive control parameters

### Testing & Examples
- `profiles/test_adaptive.json` (NEW) - Test profile
- `profiles/biscuit_faience_adaptive.json` (NEW) - Example conversion
- `scripts/test_adaptive_control.py` (NEW) - Unit tests

## Usage Examples

### Example 1: Conservative Margins

```json
{
  "name": "Safe Bisque",
  "steps": [
    {
      "type": "ramp",
      "target_temp": 1000,
      "desired_rate": 150,
      "min_rate": 100    // 67% margin - very forgiving
    }
  ]
}
```

### Example 2: Aggressive with Safety Net

```json
{
  "name": "Fast Bisque",
  "steps": [
    {
      "type": "ramp",
      "target_temp": 1000,
      "desired_rate": 200,
      "min_rate": 150    // 75% margin - will adapt if needed
    }
  ]
}
```

### Example 3: No Adaptation (Old Behavior)

```json
{
  "name": "Strict Bisque",
  "steps": [
    {
      "type": "ramp",
      "target_temp": 1000,
      "desired_rate": 150
      // No min_rate = no adaptation, just target following
    }
  ]
}
```

## Testing

### Unit Tests (Off-Device)

```bash
python3 scripts/test_adaptive_control.py
```

Tests:
- ✅ Profile loading (new and legacy formats)
- ✅ Rate monitoring accuracy
- ✅ Step execution logic
- ✅ Adaptation trigger conditions

### On-Device Testing Recommendations

1. **Start with a test profile:** Use `test_adaptive.json` (only goes to 200°C)
2. **Monitor the logs:** Watch console output for adaptation messages
3. **Verify CSV logging:** Check that `current_rate_c_per_hour` column is populated
4. **Test recovery:** Trigger a reboot during a run, verify it resumes with correct rate
5. **Test failure mode:** Create a profile with impossible min_rate, verify clean failure

## Migration Guide

### Option 1: Auto-Convert (Recommended)

Keep your existing profiles! They auto-convert to steps with:
- `min_rate = 70%` of implied rate (conservative)
- You can tweak them later if needed

### Option 2: Manual Conversion

Convert your profiles to the new format for full control:

```python
# Old format
{
  "data": [
    [0, 20],
    [3600, 100],   // 80°C/hour
    [7200, 200],   // 100°C/hour
    [10800, 200]   // Hold for 1 hour
  ]
}

# New format (equivalent)
{
  "steps": [
    {"type": "ramp", "target_temp": 100, "desired_rate": 80, "min_rate": 60},
    {"type": "ramp", "target_temp": 200, "desired_rate": 100, "min_rate": 70},
    {"type": "hold", "target_temp": 200, "duration": 3600}
  ]
}
```

## Console Output Examples

### Successful Adaptation

```
[Step 2/4] Advanced to ramp step (target: 1050°C)
[Adaptation 1] Rate adjusted: 100.0 → 85.5°C/h (actual: 87.2°C/h, min: 80.0°C/h, error: 25.3°C)
[Adaptation 2] Rate adjusted: 85.5 → 82.1°C/h (actual: 84.5°C/h, min: 80.0°C/h, error: 18.7°C)
```

### Adaptation Failure

```
ERROR: Cannot achieve minimum rate 80.0°C/h. Actual rate: 65.3°C/h after 15 minutes. Kiln may be underpowered or needs maintenance.
```

### Cooldown Safety

```
ERROR: Temperature increasing during cooldown: 856.2°C > 850.0°C
```

## Tuning Recommendations

### If getting too many adaptations:

1. **Increase `ADAPTATION_REDUCTION_FACTOR`** (0.9 → 0.95)
   - Less aggressive reduction
   - Fewer adaptations needed

2. **Decrease `ADAPTATION_TEMP_ERROR_THRESHOLD`** (20 → 15)
   - Adapt sooner, before falling too far behind

3. **Lower profile `desired_rate`**
   - Match kiln's actual capabilities

### If hitting min_rate failures:

1. **Lower `min_rate` in profile**
   - More margin for adaptation
   - 50°C/h at high temps is often appropriate

2. **Increase `ADAPTATION_MIN_STEP_TIME`** (600 → 900)
   - Give kiln more time to reach steady state
   - Reduces false alarms from thermal lag

3. **Check kiln health**
   - Elements degrading?
   - Insulation damaged?
   - Overloaded with mass?

## Memory Impact

- **Rate monitor:** ~960 bytes (60 samples × 16 bytes)
- **Step state:** ~100 bytes
- **Total overhead:** <2 KB

Minimal impact on Pico 2's available RAM.

## What's Next?

The implementation is complete and tested! You can now:

1. Deploy to your kiln
2. Test with `test_adaptive.json` profile
3. Convert your existing profiles or let them auto-convert
4. Monitor the first few runs closely
5. Tune parameters based on your kiln's behavior

**Note:** Your existing profiles will continue to work without modification. The system will auto-convert them and apply conservative min_rate values.
