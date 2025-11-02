# Continuous Gain Scheduling - Temperature-Dependent PID Tuning

## Overview

The continuous gain scheduling feature automatically adjusts PID parameters based on temperature to compensate for changing kiln thermal dynamics across the wide 0-1300°C operating range. This results in better control performance throughout the entire firing cycle.

## Why Use Continuous Gain Scheduling?

Kilns have temperature-dependent thermal characteristics:

- **Heat Loss Increases with Temperature**: At high temperatures, radiation and convection heat loss increase significantly
- **Effective Gain Decreases**: More SSR power is needed to maintain high temperatures → effective gain K_eff decreases
- **PID Must Compensate**: PID gains must increase to maintain consistent control performance

Using a single fixed PID parameter set across 0-1300°C is suboptimal. Continuous gain scheduling provides:

- **Consistent Control** across wide temperature ranges
- **Reduced Overshoot** during temperature ramps
- **Faster Settling Time** when reaching target temperatures
- **Stable Temperature Holds** at all temperature ranges

## How It Works

### Physics-Based Model

At plateau equilibrium, heat input equals heat loss:
```
SSR × K_eff(T) = (T - T_ambient) + heat_loss(T)
```

As temperature increases:
- Heat loss increases (radiation ∝ T⁴, convection ∝ ΔT)
- Effective gain K_eff **decreases**
- PID gains must **increase** to compensate

### Continuous Gain Scaling Formula

```python
# Heat loss model (linear approximation)
gain_scale(T) = 1 + h × (T - T_ambient)

# PID scaling
Kp(T) = Kp_base × gain_scale(T)
Ki(T) = Ki_base × gain_scale(T)
Kd(T) = Kd_base × gain_scale(T)
```

Where:
- `h` = heat loss coefficient (fitted from tuning data)
- `T_ambient` = ambient temperature (°C)
- `Kp_base`, `Ki_base`, `Kd_base` = base PID gains at low temperature

### Advantages Over Range-Based Scheduling

**Old Approach (Range-Based):**
- ❌ Discrete jumps in PID gains at range boundaries
- ❌ Requires hysteresis to prevent chattering
- ❌ More complex configuration (arrays of ranges)
- ❌ More memory usage (stores multiple PID sets)

**New Approach (Continuous):**
- ✅ Smooth, continuous gain adjustment (no jumps!)
- ✅ No hysteresis needed
- ✅ Simple configuration (just 3 scalar parameters)
- ✅ Memory efficient (critical for Pico!)
- ✅ Physically accurate model

## Configuration

### Step 1: Run PID Auto-Tuning

Run a tuning test that covers a wide temperature range:

```bash
# Start tuning via web UI
# Let it run across multiple temperature plateaus (e.g., 100°C, 400°C, 700°C)
# The tuning should include:
# - Multiple heating ramps
# - Multiple temperature plateaus at different levels
# - At least one cooling phase
```

**Important:** The tuning data must include plateau phases at different temperatures to fit the heat loss coefficient accurately.

### Step 2: Analyze Tuning Data

Run the analyzer script:

```bash
cd /path/to/pico-kiln
./scripts/analyze_tuning.py logs/tuning_YYYY-MM-DD_HH-MM-SS.csv
```

The analyzer will:
1. Fit a single thermal model (dead time L, time constant τ) from the full dataset
2. Extract effective gain from each plateau phase
3. Fit the heat loss coefficient `h`
4. Calculate base PID parameters using AMIGO tuning
5. Generate ready-to-paste config

### Step 3: Copy Configuration

The analyzer outputs a config snippet. Copy it to your `config.py`:

```python
# PID Parameters (AMIGO tuning)
PID_KP_BASE = 6.11   # Base proportional gain
PID_KI_BASE = 0.027  # Base integral gain
PID_KD_BASE = 42.5   # Base derivative gain

# Continuous Gain Scheduling
THERMAL_H = 0.002            # Heat loss coefficient
THERMAL_T_AMBIENT = 20.0     # Ambient temperature (°C)
```

### Step 4: Restart Controller

```bash
# Restart the kiln controller to load new parameters
# Monitor web UI during first firing to verify control performance
```

## How to Interpret Parameters

### Heat Loss Coefficient (THERMAL_H)

**Typical Range:** 0.0001 to 0.01

**What it means:**
- **h = 0**: No gain scaling (constant PID gains)
- **h = 0.001**: Modest scaling (~1.9x gain increase from 0°C to 1000°C)
- **h = 0.005**: Aggressive scaling (~6x gain increase from 0°C to 1000°C)

**Too small?** PID won't compensate enough at high temps → may be sluggish at high temperatures
**Too large?** Over-compensation → may cause instability at high temperatures

### Example: Gain Scaling at Different Temperatures

With `THERMAL_H = 0.002` and `THERMAL_T_AMBIENT = 25°C`:

| Temperature | ΔT from Ambient | Gain Scale | Kp (if Kp_base=6.11) |
|-------------|-----------------|------------|----------------------|
| 100°C       | 75°C            | 1.15x      | 7.03                 |
| 400°C       | 375°C           | 1.75x      | 10.69                |
| 700°C       | 675°C           | 2.35x      | 14.36                |
| 1000°C      | 975°C           | 2.95x      | 18.02                |

Notice: **Smooth, continuous scaling** from 6.11 to 18.02 (3x increase over 1000°C range)

## Disabling Gain Scheduling

To use constant PID gains (no temperature compensation):

```python
# Set THERMAL_H to zero
THERMAL_H = 0.0

# The base gains will be used at all temperatures
PID_KP_BASE = 25.0
PID_KI_BASE = 0.18
PID_KD_BASE = 160.0
```

## Monitoring During Operation

The controller prints gain updates to the console:

```
[Control Thread] Continuous gain scheduling ENABLED (h=0.002000)
[Control Thread] Base PID: Kp=6.110 Ki=0.0270 Kd=42.500

...during firing...

[Control Thread] PID gains updated: Kp=7.03 Ki=0.0309 Kd=48.69 @ 100.0°C (scale=1.150)
[Control Thread] PID gains updated: Kp=10.69 Ki=0.0473 Kd=74.38 @ 400.0°C (scale=1.750)
[Control Thread] PID gains updated: Kp=14.36 Ki=0.0635 Kd=100.08 @ 700.0°C (scale=2.350)
```

Watch for:
- ✅ Gains increasing smoothly as temperature rises
- ❌ Excessive oscillations (h may be too large)
- ❌ Sluggish response at high temps (h may be too small)

## Troubleshooting

### "Heat loss coefficient is very small (h=0.000100)"

**Cause:** Insufficient temperature range in tuning data, or heat loss variation is minimal

**Solution:**
- Run tuning across wider temperature range
- Or just use constant gains (`THERMAL_H = 0`)

### "THERMAL_H is very large"

**Cause:** User error or tuning data has errors

**Solution:**
- Check tuning data quality
- Re-run tuning with better plateaus
- Expected range: 0.0001 to 0.01

### Control oscillates at high temperatures

**Cause:** `THERMAL_H` is too large, over-compensating

**Solution:**
- Reduce `THERMAL_H` by 50%
- Re-test
- Or disable gain scheduling (`THERMAL_H = 0`)

### Sluggish response at high temperatures

**Cause:** `THERMAL_H` is too small, under-compensating

**Solution:**
- Increase `THERMAL_H` by 50%
- Re-test

## Technical Details

### Implementation

**Analyzer** (`scripts/analyzer/`):
- Fits single thermal model (L, τ) from full dataset
- Extracts effective gain from each plateau: `K_eff = (T - T_ambient) / SSR`
- Fits heat loss coefficient using linear regression
- Uses median for robustness to outliers

**Runtime** (`kiln/control_thread.py`):
- Calculates `gain_scale = 1 + h × (T - T_ambient)` in control loop
- Updates PID gains when change exceeds threshold (0.01 for Kp/Kd, 0.0001 for Ki)
- Simple, fast computation (~3 floating-point operations)

### Memory Usage

- Old range-based: ~500 bytes (multiple PID sets + hysteresis logic)
- New continuous: ~12 bytes (3 floats: h, T_ambient, and current gain_scale)

**Savings:** ~98% reduction in memory usage!

### Computational Cost

Per PID update (1 Hz):
```python
gain_scale = 1.0 + self.thermal_h * (current_temp - self.thermal_t_ambient)  # 2 float ops
kp = self.pid_kp_base * gain_scale  # 1 float multiply
# Total: 3 float operations (negligible on RP2350)
```

## References

- Process control textbook: "Gain scheduling for wide-range temperature control"
- Similar approach used in industrial furnace controllers
- Physics: Stefan-Boltzmann law (radiation heat loss ∝ T⁴), linearized for control

## Support

For questions or issues:
- Check tuning data quality (analyzer provides warnings)
- Start conservative (smaller h values)
- Monitor first firing closely
- Report issues: https://github.com/anthropics/claude-code/issues
