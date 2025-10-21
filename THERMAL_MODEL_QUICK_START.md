# Thermal Model Quick Start Guide

## What is the Thermal Model?

The thermal model enables **gain scheduling** - using different PID parameters at different temperatures for better kiln control.

## Quick Setup (5 Steps)

### 1. Run PID Tuning

Start auto-tuning via web UI. Let it run across your full temperature range (e.g., 0-1300°C).

### 2. Analyze Results

```bash
python analyze_tuning.py logs/tuning_YYYY-MM-DD_HH-MM-SS.csv
```

### 3. Generate Config

```bash
python generate_thermal_model_config.py
```

### 4. Copy to Config

Copy the `THERMAL_MODEL = [...]` output and paste into `config.py`.

### 5. Restart

Restart the kiln controller. Done!

## Example Output

After step 3, you'll see:

```python
THERMAL_MODEL = [
    {'temp_min': 0, 'temp_max': 300, 'kp': 25.0, 'ki': 180.0, 'kd': 160.0},  # LOW
    {'temp_min': 300, 'temp_max': 700, 'kp': 20.0, 'ki': 150.0, 'kd': 120.0},  # MID
    {'temp_min': 700, 'temp_max': 9999, 'kp': 15.0, 'ki': 100.0, 'kd': 80.0}  # HIGH
]
```

## What You'll See

During firing, console will show gain updates:

```
[Control Thread] PID gains updated: Kp=20.000 Ki=150.0000 Kd=120.000 @ 300.1°C
[Control Thread] PID gains updated: Kp=15.000 Ki=100.0000 Kd=80.000 @ 700.2°C
```

## To Disable

Set in `config.py`:

```python
THERMAL_MODEL = None
```

## Benefits

- Better temperature control
- Less overshoot
- Faster settling
- Compensates for changing kiln dynamics

## Need Help?

See `THERMAL_MODEL.md` for full documentation.
