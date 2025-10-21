# Thermal Model - Temperature-Range-Specific PID Parameters

## Overview

The thermal model feature enables **gain scheduling** - using different PID parameters at different temperature ranges. This compensates for changing kiln thermal dynamics across the wide 0-1300°C operating range, resulting in better control performance.

## Why Use a Thermal Model?

Kilns have different thermal characteristics at different temperatures:

- **Low temperatures (0-300°C)**: Fast thermal response, lower heat loss
- **Mid temperatures (300-700°C)**: Changing thermal mass effects
- **High temperatures (700-1300°C)**: High heat loss, slower response, radiation effects

Using a single PID parameter set across this entire range is suboptimal. Gain scheduling automatically adjusts PID gains based on current temperature, providing:

- **Better control** across wide temperature ranges
- **Reduced overshoot** during temperature ramps
- **Faster settling time** when reaching target temperatures
- **More stable** temperature holds at different ranges

## How It Works

### Gain Scheduling Algorithm

The controller uses simple **range-based switching**:

1. Current temperature is measured
2. PID scheduler finds the matching temperature range
3. If range changed, PID gains are updated
4. PID controller uses the new gains

Range matching: `temp_min <= current_temp < temp_max` (inclusive lower bound)

### Gain Switching Behavior

- **Instant switching** when crossing range boundaries
- **No interpolation** (keeps it simple and fast)
- **Integral term continuity** maintained during switches (prevents control discontinuities)
- **Acceptable for kilns** due to slow thermal response

## Configuration

### Step 1: Run PID Auto-Tuning

Run a tuning test that covers your full temperature range:

```bash
# Start tuning via web UI or command
# Let it run across full 0-1300°C range (or your max temp)
```

### Step 2: Analyze Tuning Data

```bash
python analyze_tuning.py logs/tuning_YYYY-MM-DD_HH-MM-SS.csv
```

This generates `tuning_results.json` with temperature-range-specific PID parameters.

### Step 3: Generate Config Snippet

```bash
python generate_thermal_model_config.py
```

Or:

```bash
python -c "from analyze_tuning import generate_config_snippet; generate_config_snippet()"
```

This prints a ready-to-paste THERMAL_MODEL configuration.

### Step 4: Update config.py

Copy the THERMAL_MODEL from step 3 and paste into `config.py`:

```python
# config.py

# Default PID parameters (fallback)
PID_KP = 25.0
PID_KI = 180.0
PID_KD = 160.0

# Temperature-range-specific PID parameters
THERMAL_MODEL = [
    {'temp_min': 0, 'temp_max': 300, 'kp': 25.0, 'ki': 180.0, 'kd': 160.0},
    {'temp_min': 300, 'temp_max': 700, 'kp': 20.0, 'ki': 150.0, 'kd': 120.0},
    {'temp_min': 700, 'temp_max': 9999, 'kp': 15.0, 'ki': 100.0, 'kd': 80.0}
]
```

### Step 5: Restart Controller

Restart the kiln controller to load the new thermal model.

## Disabling Thermal Model

To disable gain scheduling and use single PID parameters:

```python
# config.py
THERMAL_MODEL = None  # Use default PID for all temperatures
```

## Architecture Details

### Memory Usage

- Each temperature range: ~100 bytes
- Maximum 5 ranges (~500 bytes total)
- Validated at startup to prevent memory issues

### Files Modified/Created

1. **`config.py`**: Added THERMAL_MODEL configuration
2. **`kiln/pid_scheduler.py`**: New gain scheduling module (220 lines)
3. **`kiln/__init__.py`**: Export PIDGainScheduler
4. **`kiln/control_thread.py`**: Integration into control loop
5. **`kiln/comms.py`**: Added PID gains to status messages
6. **`analyze_tuning.py`**: Added config snippet generator
7. **`generate_thermal_model_config.py`**: Standalone helper script

### Control Loop Integration

The gain scheduler is called every control loop iteration (1 Hz):

```python
# In control_thread.py control_loop_iteration()

# Update PID gains based on current temperature
kp, ki, kd = self.pid_scheduler.get_gains(current_temp)
if self.pid_scheduler.gains_changed():
    self.pid.set_gains(kp, ki, kd)
    print(f"PID gains updated: Kp={kp} Ki={ki} Kd={kd} @ {current_temp}°C")

# Calculate PID output with current gains
ssr_output = self.pid.update(target_temp, current_temp)
```

### Status Reporting

Current PID gains are included in status messages:

```json
{
  "current_temp": 450.2,
  "target_temp": 500.0,
  "pid_kp": 20.0,
  "pid_ki": 150.0,
  "pid_kd": 120.0,
  "pid_stats": {
    "kp": 20.0,
    "ki": 150.0,
    "kd": 120.0,
    "p_term": 45.6,
    "i_term": 12.3,
    "d_term": -5.2
  }
}
```

Web UI can display which gains are currently active.

## Testing Recommendations

### Unit Testing (Optional)

Test the PIDGainScheduler class:

```python
# Test basic range selection
scheduler = PIDGainScheduler(
    thermal_model=[
        {'temp_min': 0, 'temp_max': 300, 'kp': 25.0, 'ki': 180.0, 'kd': 160.0},
        {'temp_min': 300, 'temp_max': 700, 'kp': 20.0, 'ki': 150.0, 'kd': 120.0},
    ],
    default_kp=25.0, default_ki=180.0, default_kd=160.0
)

# Test low temp range
kp, ki, kd = scheduler.get_gains(150)
assert kp == 25.0 and ki == 180.0 and kd == 160.0

# Test mid temp range
kp, ki, kd = scheduler.get_gains(450)
assert kp == 20.0 and ki == 150.0 and kd == 120.0

# Test boundary (inclusive lower)
kp, ki, kd = scheduler.get_gains(300)
assert kp == 20.0  # 300 is in the 300-700 range
```

### Integration Testing

1. **Test with THERMAL_MODEL = None**:
   - Should use default PID values
   - No gain switching should occur

2. **Test with 2-range model**:
   - Monitor gains in console/web UI
   - Verify switching at boundary temperature
   - Check for smooth control during transition

3. **Test with 3-range model**:
   - Run full firing profile (0-1300°C)
   - Monitor gain switches at each boundary
   - Verify no control discontinuities

### Manual Testing

Run an actual firing profile and monitor:

1. **Console output**: Watch for gain update messages
2. **Web UI**: Display current Kp/Ki/Kd values
3. **Temperature control**: Check for:
   - No sudden SSR output changes at boundaries
   - No oscillations after gain switches
   - Improved temperature tracking vs single PID

## Design Decisions and Trade-offs

### 1. Range Boundaries

**Decision**: Use `temp_min <= current_temp < temp_max` (inclusive lower, exclusive upper)

**Rationale**: Standard range convention, prevents overlaps

**Trade-off**: Last range must have very high temp_max (e.g., 9999°C)

### 2. Switching Behavior

**Decision**: Instant switching when crossing boundaries (no interpolation)

**Rationale**:
- Simple and fast (O(n) lookup where n ≤ 5)
- MicroPython compatible (no floating point interpolation)
- Acceptable for slow thermal systems (kilns have minutes of time constant)

**Trade-off**: Small control discontinuity possible at boundaries, but mitigated by:
- Maintaining integral term continuity
- Slow kiln thermal response
- Reasonable gain values across ranges

### 3. Fallback Logic

**Decision**: Use default PID if temperature outside all ranges

**Rationale**: Graceful degradation, prevents controller failure

**Implementation**:
```
if THERMAL_MODEL is None:
    use default PID_KP, PID_KI, PID_KD
else:
    use gain scheduler
    if temp outside all ranges: use default PID
```

### 4. Memory Constraints

**Decision**: Maximum 5 temperature ranges

**Rationale**: Each range uses ~100 bytes, 5 ranges = ~500 bytes (acceptable on Pico 2)

**Validation**: Checked at startup, raises error if exceeded

### 5. Performance

**Decision**: O(n) linear search through ranges (n ≤ 5)

**Rationale**:
- Called once per control loop (1 Hz)
- n ≤ 5 means max 5 comparisons
- Fast enough (microseconds) vs 1-second control interval
- Simpler than binary search for small n

## Example Thermal Models

### Conservative (Wide Ranges)

```python
THERMAL_MODEL = [
    {'temp_min': 0, 'temp_max': 500, 'kp': 25.0, 'ki': 180.0, 'kd': 160.0},
    {'temp_min': 500, 'temp_max': 9999, 'kp': 15.0, 'ki': 100.0, 'kd': 80.0}
]
```

### Detailed (Narrow Ranges)

```python
THERMAL_MODEL = [
    {'temp_min': 0, 'temp_max': 200, 'kp': 30.0, 'ki': 200.0, 'kd': 180.0},
    {'temp_min': 200, 'temp_max': 500, 'kp': 25.0, 'ki': 180.0, 'kd': 160.0},
    {'temp_min': 500, 'temp_max': 800, 'kp': 20.0, 'ki': 150.0, 'kd': 120.0},
    {'temp_min': 800, 'temp_max': 1100, 'kp': 15.0, 'ki': 100.0, 'kd': 80.0},
    {'temp_min': 1100, 'temp_max': 9999, 'kp': 10.0, 'ki': 50.0, 'kd': 40.0}
]
```

## Troubleshooting

### Thermal model validation error

**Error**: `ValueError: THERMAL_MODEL limited to 5 ranges`

**Solution**: Reduce number of temperature ranges to 5 or fewer

### No gains switching during firing

**Check**:
1. Verify THERMAL_MODEL is not None in config.py
2. Check console for "PID gains updated" messages
3. Verify temperature ranges cover current temp
4. Check last range has high enough temp_max (e.g., 9999)

### Control oscillations after gain switch

**Possible causes**:
1. PID gains too aggressive for that temperature range
2. Large gain difference between ranges

**Solutions**:
1. Use more conservative gains (lower Kp, Ki, Kd)
2. Make gain transitions more gradual across ranges
3. Re-run tuning analysis with --method amigo (most conservative)

### Performance issues

**Unlikely** (gain scheduler is very fast), but if you see control loop delays:

1. Check number of ranges (should be ≤ 5)
2. Verify no exception spam in console
3. Monitor control loop timing

## Future Enhancements

Potential improvements (not currently implemented):

1. **Gain interpolation**: Smoothly interpolate between ranges instead of instant switching
2. **Adaptive tuning**: Automatically adjust gains based on observed performance
3. **Load-dependent scheduling**: Different gains based on kiln load/thermal mass
4. **Web UI configuration**: Edit THERMAL_MODEL via web interface
5. **Profile-specific models**: Different thermal models for different firing profiles

## Summary

The thermal model feature provides sophisticated gain scheduling for improved kiln control across wide temperature ranges. It's:

- **Easy to configure**: Generated automatically from tuning data
- **Memory efficient**: <500 bytes for 5 ranges
- **Fast**: Microsecond lookup overhead
- **MicroPython compatible**: No external dependencies
- **Optional**: Can be disabled by setting THERMAL_MODEL = None

For best results, run a comprehensive tuning test across your full temperature range and use the generated thermal model in production firings.
