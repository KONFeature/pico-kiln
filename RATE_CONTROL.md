# Rolling Rate Control and Stall Detection

## Overview

The pico-kiln controller uses a rolling rate control system to execute firing profiles. Unlike simpler controllers that wait for the kiln to reach a temperature before starting a timer, this system advances a virtual target temperature in real-time based on your desired heating rate.

The kiln follows this advancing target as closely as possible. If the kiln is underpowered or heavily loaded, the actual temperature may fall behind the target. This is expected behavior at high temperatures where heat loss increases. The controller does not automatically reduce the rate; it continues to push the kiln at 100% power until it either catches up or triggers a stall alarm.

## Profile Format

Profiles use a step-based JSON format. Each step defines a specific phase of the firing.

```json
{
  "name": "Glaze Firing",
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
      "target_temp": 1240,
      "desired_rate": 150,
      "min_rate": 60
    },
    {
      "type": "cooling",
      "target_temp": 100
    }
  ]
}
```

### Step Types

1. **ramp**: Heats or cools the kiln to a `target_temp` at a specific `desired_rate` (°C/h).
2. **hold**: Maintains the `target_temp` for a `duration` (seconds).
3. **cooling**: Turns off the SSR for natural cooling.

## Ramp Steps and Stall Detection

Ramp steps are the core of the firing process. You specify:
- `desired_rate`: The speed you want the kiln to heat or cool at (°C/h).
- `min_rate` (optional): The absolute minimum speed the kiln must maintain. If omitted, it defaults to 80% of the `desired_rate`.

### Stall Detection Logic

The controller monitors the actual heating rate using a rolling window. If the measured rate drops below the `min_rate`, the system tracks it as a potential stall.

- **Stall Check Interval**: The rate is checked every `STALL_CHECK_INTERVAL` seconds (default 60s).
- **Consecutive Fails**: If the rate is too low for `STALL_CONSECUTIVE_FAILS` checks (default 3), the firing stops with an error.
- **Grace Period**: Stall detection only starts after the kiln has been in the step for `STALL_MIN_STEP_TIME` seconds (default 10 minutes). This allows the kiln to overcome thermal lag at the start of a ramp.

Example: If you set a ramp to 1000°C at 150°C/h with a `min_rate` of 100°C/h, and the kiln can only manage 90°C/h due to old elements, the controller will shut down the firing after 3 minutes of failing the check.

## Cooling

### Controlled Cooling
To cool at a specific rate (e.g., for crystal glazes), use a `ramp` step with a `target_temp` lower than the current temperature. The PID controller will manage the SSR to slow down the descent. Stall detection works the same way using the absolute value of the rate.

### Natural Cooling
Use the `cooling` step type for natural cooling. The SSR remains off, and the controller simply monitors the temperature until it reaches the optional `target_temp`. No rate control or stall detection is performed during natural cooling.

## Configuration

Parameters in `config.py` control the sensitivity of the monitoring system:

- `STALL_CHECK_INTERVAL`: How often to evaluate the stall condition (seconds).
- `STALL_CONSECUTIVE_FAILS`: Number of failed checks before triggering an error.
- `STALL_MIN_STEP_TIME`: Initial delay before stall checking begins (seconds).
- `RATE_MEASUREMENT_WINDOW`: The time period used to calculate the rolling rate (seconds).
- `RATE_RECORDING_INTERVAL`: How often temperature samples are taken for the rate buffer (seconds).

## Implementation Details

### Rate Monitoring
The controller uses a `TempHistory` circular buffer to store recent temperature readings. This buffer is memory-efficient, using approximately 1KB of RAM to store 10 minutes of history at 10-second intervals. The rate is calculated by comparing the most recent reading with the reading closest to the start of the measurement window.

### CSV Logging
The data logger records the `measured_rate_c_per_hour` in every log entry. This allows you to analyze your kiln's performance after a run and adjust your `min_rate` values or identify degrading elements.

### PID Behavior and Overshoot
When the virtual target runs ahead of the actual temperature (common during aggressive ramps), the PID controller will saturate at 100% output. Because the integral term continues to accumulate during this saturation, you may see a brief overshoot when the kiln finally catches up to the target or transitions to a hold step. 

Future improvements will include an integral reset on step transitions to minimize this effect.
