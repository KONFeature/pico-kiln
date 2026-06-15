# Continuous Gain Scheduling Quick Start Guide

## What is Continuous Gain Scheduling?

Continuous gain scheduling automatically adjusts PID parameters smoothly based on temperature for better kiln control. Unlike range-based scheduling, gains change continuously (no jumps!).

## Quick Setup (4 Steps)

### 1. Run PID Tuning

Start auto-tuning via web UI. Let it run across multiple temperature plateaus (e.g., 100°C, 400°C, 700°C).

**Important:** You need plateau phases at different temperatures for accurate heat loss fitting.

### 2. Analyze Results

```bash
cd /path/to/pico-kiln
./scripts/analyze_tuning.py logs/tuning_YYYY-MM-DD_HH-MM-SS.csv
```

The analyzer will output:
- Thermal model parameters
- Base PID gains
- Heat loss coefficient
- Ready-to-paste config

### 3. Copy to Config

Copy the configuration snippet from analyzer output to `config.py`:

```python
# PID Parameters (AMIGO tuning)
PID_KP_BASE = 6.11   # Base proportional gain
PID_KI_BASE = 0.027  # Base integral gain
PID_KD_BASE = 42.5   # Base derivative gain

# Continuous Gain Scheduling
THERMAL_H = 0.002            # Heat loss coefficient
THERMAL_T_AMBIENT = 20.0     # Ambient temperature (°C)
```

### 4. Restart

Restart the kiln controller. Done!

## Example Output

During firing, console will show smooth gain updates:

```
[Control Thread] Continuous gain scheduling ENABLED (h=0.002000)
[Control Thread] Base PID: Kp=6.110 Ki=0.0270 Kd=42.500

...during firing...

[Control Thread] PID gains updated: Kp=7.03 Ki=0.0309 Kd=48.69 @ 100.0°C (scale=1.150)
[Control Thread] PID gains updated: Kp=10.69 Ki=0.0473 Kd=74.38 @ 400.0°C (scale=1.750)
[Control Thread] PID gains updated: Kp=14.36 Ki=0.0635 Kd=100.08 @ 700.0°C (scale=2.350)
```

Notice how gains increase smoothly as temperature rises!

## How It Works

Simple physics-based formula:

```python
gain_scale(T) = 1 + h × (T - T_ambient)
Kp(T) = Kp_base × gain_scale(T)
```

Where:
- `h` = heat loss coefficient (from tuning)
- Higher temps → more heat loss → higher gains needed

## To Disable

Set in `config.py`:

```python
THERMAL_H = 0.0  # Zero = no gain scaling
```

## Benefits vs Old Range-Based System

**Continuous (NEW):**
- ✅ Smooth gain changes (no jumps)
- ✅ Simpler config (3 parameters)
- ✅ Less memory (~98% reduction!)
- ✅ No hysteresis needed
- ✅ Physically accurate

**Range-Based (OLD - Removed):**
- ❌ Discrete jumps at boundaries
- ❌ Complex config (arrays)
- ❌ More memory usage
- ❌ Needed hysteresis

## Troubleshooting

**Gains not changing?**
- Check `THERMAL_H > 0` in config
- Check analyzer found plateau phases

**Control unstable at high temps?**
- Reduce `THERMAL_H` by 50%
- Or disable: `THERMAL_H = 0`

**Sluggish at high temps?**
- Increase `THERMAL_H` by 50%

## Need Help?

See `THERMAL_MODEL.md` for full documentation including:
- Detailed physics explanation
- Parameter interpretation
- Advanced troubleshooting
- Technical implementation details
