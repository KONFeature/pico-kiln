# PID Auto-Tuning Guide

This document explains how to use the integrated PID auto-tuning system to optimize your kiln's temperature control.

## Overview

The PID auto-tuner uses the **Ziegler-Nichols method** to automatically calculate optimal PID parameters (Kp, Ki, Kd) for your specific kiln. This eliminates the need for manual tuning and provides a scientifically-based starting point for optimal temperature control.

## How It Works

The tuning process follows these steps:

1. **Heating Phase**: The kiln heats at maximum power (100% SSR) from room temperature to the target temperature
2. **Cooling Phase**: Heating turns off (0% SSR) and the kiln cools naturally back to the target temperature
3. **Analysis**: The temperature curve is analyzed to extract system characteristics
4. **Calculation**: PID parameters are calculated using Ziegler-Nichols formulas

The entire process typically takes **10-30 minutes** depending on your target temperature.

## Prerequisites

Before starting tuning:

- ✅ Kiln must be completely cool (room temperature)
- ✅ No firing profile should be running
- ✅ Ensure you have 30+ minutes available
- ✅ Kiln should be in the same state as during normal firing (e.g., with vent on if you normally use one)
- ✅ WiFi connection should be stable

## Step-by-Step Instructions

### 1. Access the Tuning Interface

Navigate to the tuning page via:
- Main interface → "PID Auto-Tuning" button, or
- Direct URL: `http://<your-pico-ip>/tuning`

### 2. Set Target Temperature

Choose an appropriate target temperature:

- **200°C (default)**: Good starting point, faster tuning
- **400°C**: Provides more data, better for high-temp kilns
- **Higher temps**: More accurate for your actual firing range, but takes longer

**Recommendation**: Start with 200°C for initial tuning.

### 3. Start Tuning

1. Click "Start Tuning"
2. Confirm the safety prompt
3. The system will enter TUNING state
4. Monitor progress on the web interface

### 4. Monitor Progress

The interface shows:
- **Stage**: Current phase (HEATING → COOLING → CALCULATING → COMPLETE)
- **Current Temperature**: Real-time temperature reading
- **Elapsed Time**: Time since tuning started
- **Data Points**: Number of temperature measurements collected

**Do not interrupt the process** once started. The system will automatically:
- Stop at safety limits (MAX_TEMP)
- Time out after 30 minutes if something goes wrong
- Save results when complete

### 5. Review Results

When tuning completes, the interface displays:

```
Parameter | Value      | Description
----------|------------|----------------
Kp        | 25.34      | Proportional gain
Ki        | 182.56     | Integral gain
Kd        | 157.89     | Derivative gain
```

These values are automatically saved to:
- `tuning_results.json`: Full analysis data
- `tuning_data.csv`: Temperature time-series data

### 6. Apply Results

Update your configuration file with the calculated values:

1. Open `config.py` on the Pico
2. Update the PID parameters:
   ```python
   PID_KP = 25.34
   PID_KI = 182.56
   PID_KD = 157.89
   ```
3. Save the file
4. Restart the controller

### 7. Test and Fine-Tune

1. Run a test firing profile
2. Monitor temperature tracking:
   - Good tracking: Temperature follows target within ±5-10°C
   - Overshoot: Reduce Kp
   - Oscillation: Reduce Ki and Kd
   - Slow response: Increase Kp

3. Manual adjustments (if needed):
   - **Increase Kp**: Faster response, but may cause overshoot
   - **Increase Ki**: Better long-term accuracy, but may cause oscillation
   - **Increase Kd**: Dampens oscillations, but too high causes instability

## Troubleshooting

### Tuning Times Out
**Cause**: Kiln may not be heating properly
**Solution**:
- Check SSR connections
- Verify heating elements are working
- Ensure kiln is unplugged/off before testing hardware

### Results Look Wrong (Negative Values, Very Large/Small)
**Cause**: Insufficient or poor quality data
**Solution**:
- Ensure kiln started from room temperature
- Try a higher target temperature (more data range)
- Check for external factors (drafts, door open, etc.)

### Temperature Doesn't Reach Target
**Cause**: Target too high or heating insufficient
**Solution**:
- Lower target temperature
- Verify all heating elements are working
- Check MAX_TEMP safety limit in config.py

### Tuning Stops with Error
**Cause**: Safety limit exceeded or sensor fault
**Solution**:
- Check error message on tuning page
- Verify thermocouple connections
- Ensure MAX_TEMP is appropriate for your kiln

## Technical Details

### Ziegler-Nichols Method

The system uses the **open-loop step response** variant:

1. Record temperature response to step input (0% → 100% → 0%)
2. Fit a tangent line to the inflection point of the heating curve
3. Calculate system parameters:
   - **L** (lag time): Dead time before response
   - **T** (time constant): Time for response to complete
4. Apply Z-N formulas:
   - Kp = 1.2 × (T / L)
   - Ki = Kp / (2L)
   - Kd = Kp × (0.5L)

### Safety Features

- **Maximum temperature check**: Stops if temp > MAX_TEMP
- **Timeout**: Automatic stop after 30 minutes
- **Emergency stop**: Can be stopped at any time via web interface
- **Automatic SSR shutoff**: On error, timeout, or completion
- **State isolation**: Cannot start profiles during tuning

### File Outputs

After successful tuning:

**tuning_results.json**:
```json
{
  "results": {
    "kp": 25.34,
    "ki": 182.56,
    "kd": 157.89,
    "L": 45.2,
    "T": 952.1,
    "min_temp": 23.4,
    "max_temp": 205.8,
    "target_temp": 200.0,
    "duration": 1243.5,
    "data_points": 1243
  },
  "data": {
    "time": [0, 1, 2, ...],
    "temperature": [23.4, 23.5, 23.6, ...]
  }
}
```

**tuning_data.csv**:
```csv
time,temperature
0.0,23.4
1.0,23.5
2.0,23.6
...
```

## API Reference

For programmatic access:

### Start Tuning
```
POST /api/tuning/start
Content-Type: application/json

{
  "target_temp": 200
}
```

### Stop Tuning
```
POST /api/tuning/stop
```

### Get Status
```
GET /api/tuning/status

Response:
{
  "state": "TUNING",
  "current_temp": 156.3,
  "tuning": {
    "stage": "heating",
    "target_temp": 200,
    "elapsed": 543.2,
    "data_points": 543
  }
}
```

## Architecture Notes

### Multi-Core Integration

The tuning system is fully integrated with the dual-core architecture:

- **Core 1 (Control Thread)**:
  - Executes tuning logic
  - Records temperature data
  - Calculates PID parameters
  - Controls SSR during tuning

- **Core 2 (Web Server)**:
  - Serves tuning interface
  - Handles API requests
  - Displays real-time status
  - Manages result application

Communication occurs via thread-safe queues, ensuring no race conditions or data corruption.

### State Machine

The tuning process introduces a new state `TUNING` to the kiln controller:

```
IDLE → TUNING → IDLE/ERROR/COMPLETE
      ↓
   HEATING → COOLING → CALCULATING → COMPLETE
```

Tuning is mutually exclusive with profile execution - you cannot run a firing profile during tuning or vice versa.

## Best Practices

1. **Initial Setup**: Always run tuning before first use
2. **Periodic Re-tuning**: Re-tune if you:
   - Change heating elements
   - Modify kiln insulation
   - Notice degraded temperature control
3. **Target Temperature**: Use a temperature in your typical firing range
4. **Test Thoroughly**: Always test with a simple profile before production firings
5. **Document Results**: Keep a log of tuning results for reference

## References

- "Ziegler–Nichols Tuning Method" by Vishakha Vijay Patel
- Original inspiration: [jbruce12000/kiln-controller](https://github.com/jbruce12000/kiln-controller)

## Support

If you encounter issues:
1. Check this guide's troubleshooting section
2. Review `tuning_results.json` for diagnostic data
3. Examine console logs on the Pico
4. Open an issue with:
   - Tuning results file
   - Temperature data CSV
   - Description of the problem
   - Kiln specifications
