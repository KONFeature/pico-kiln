# Advanced PID Auto-Tuning Guide

This guide explains how to use the **multi-mode PID auto-tuning system** with thermal modeling to optimize your kiln's temperature control across the full operating range (0-1300°C).

## Table of Contents

1. [Overview](#overview)
2. [Tuning Modes](#tuning-modes)
3. [Quick Start Workflow](#quick-start-workflow)
4. [Step-by-Step Instructions](#step-by-step-instructions)
5. [Analyzing Tuning Data](#analyzing-tuning-data)
6. [Applying Results](#applying-results)
7. [Technical Details](#technical-details)
8. [Troubleshooting](#troubleshooting)
9. [Best Practices](#best-practices)

---

## Overview

The pico-kiln controller features a **comprehensive thermal characterization system** that goes far beyond simple PID tuning. Instead of a single set of PID parameters, the system can:

✅ Run different test sequences for different needs (safety verification, standard tuning, comprehensive characterization)
✅ Analyze thermal behavior across multiple power levels
✅ Calculate PID parameters using **4 different methods** (Ziegler-Nichols, Cohen-Coon, AMIGO, Lambda)
✅ Generate **temperature-range-specific PID parameters** for improved control across wide temperature ranges
✅ Provide test quality assessment and recommendations

### Two-Phase Process

1. **Tuning Run** (on Pico): Collect temperature data using automated test sequences
2. **Analysis** (on laptop): Analyze data offline, calculate PID parameters, generate config

This separation allows sophisticated analysis without memory constraints on the Pico 2.

---

## Tuning Modes

The system offers three tuning modes, each optimized for different scenarios:

### SAFE Mode (30-45 minutes)

**Purpose:** Quick safety verification for new or untested kilns

**Sequence:**
- Heat at 30% SSR power for 10 minutes
- Monitor for proper heating behavior
- Cool back to ambient

**Max Temperature:** 100°C (default)

**Best For:**
- First-time kiln operation
- Verifying hardware connections
- Quick functionality test
- Checking for thermal issues

**Data Quality:** Basic (suitable for initial PID estimates)

---

### STANDARD Mode (1-2 hours) ⭐ **Recommended**

**Purpose:** Balanced characterization providing good PID data without excessive time investment

**Sequence:**
1. Heat at 25% SSR → plateau or 10 min timeout
2. Heat at 50% SSR → plateau or 10 min timeout
3. Heat at 75% SSR → plateau or 10 min timeout
4. Cool to ambient

**Max Temperature:** 150°C (default)

**Best For:**
- Normal PID tuning for most kilns
- Good balance of time vs. data quality
- Recommended for initial tuning after SAFE mode
- Sufficient for single PID parameter set

**Data Quality:** Good to Excellent

**Plateau Detection:** Enabled (auto-advances when temperature stabilizes)

---

### THOROUGH Mode (3-4 hours)

**Purpose:** Comprehensive thermal modeling for maximum control accuracy

**Sequence:**
1. Heat at 20% SSR → plateau or 10 min timeout → hold 5 min
2. Heat at 40% SSR → plateau or 10 min timeout → hold 5 min
3. Heat at 60% SSR → plateau or 10 min timeout → hold 5 min
4. Heat at 80% SSR → plateau or 10 min timeout → hold 5 min
5. Cool to ambient

**Max Temperature:** 200°C (default)

**Best For:**
- Professional kiln operations
- Wide temperature range firings (0-1300°C)
- Generating temperature-range-specific PID parameters
- Maximum control accuracy
- Research and development

**Data Quality:** Excellent

**Plateau Detection:** Enabled with extended hold times

---

## Quick Start Workflow

### First-Time Setup

```
1. SAFE Mode Test (30-45 min)
   ↓
2. Verify basic operation
   ↓
3. STANDARD Mode Tuning (1-2 hours)
   ↓
4. Analyze data (laptop)
   ↓
5. Apply PID parameters
   ↓
6. Test with firing profile
   ↓
7. Optional: THOROUGH mode for thermal modeling
```

### Recommended Path

**For Most Users:**
1. Run **SAFE** mode first (safety check)
2. Run **STANDARD** mode for PID tuning
3. Use default single PID parameter set

**For Advanced Users:**
1. Run **SAFE** mode first (safety check)
2. Run **THOROUGH** mode for comprehensive data
3. Use temperature-range-specific PID parameters (gain scheduling)

---

## Step-by-Step Instructions

### Prerequisites

Before starting any tuning:

- ✅ Kiln must be completely cool (room temperature)
- ✅ No firing profile should be running
- ✅ Ensure sufficient time available (30min - 4 hours depending on mode)
- ✅ Kiln should be in normal operating configuration (vents, doors, etc.)
- ✅ WiFi connection should be stable
- ✅ Clear workspace with fire safety equipment nearby

### Step 1: Access Tuning Interface

Navigate to the tuning page:
- **Web Interface:** Main menu → "PID Auto-Tuning"
- **Direct URL:** `http://<pico-ip>/tuning.html`

### Step 2: Select Tuning Mode

Choose the appropriate mode by clicking on the mode card:

**First-time users:** Start with **SAFE** mode
**Normal tuning:** Use **STANDARD** mode
**Advanced tuning:** Use **THOROUGH** mode

Each card displays:
- Duration estimate
- Temperature range
- Power levels
- Recommended use case

### Step 3: Configure Parameters (Optional)

**Max Temperature Override:**
- Leave blank to use mode default (100/150/200°C)
- Enter custom value (50-500°C) for specific testing
- Example: Enter 120 for SAFE mode to test slightly higher

### Step 4: Start Tuning

1. Click **"Start Tuning"** button
2. Confirm the safety prompt
3. System enters TUNING state
4. Web interface updates with real-time progress

### Step 5: Monitor Progress

The interface displays:

**Status Grid:**
- Current Temperature
- Max Temperature (safety limit)
- SSR Output (%)
- Elapsed Time

**Step Indicator:**
- Visual progress through test sequence
- Current step highlighted in blue
- Completed steps shown in green
- Pending steps shown in gray

**Plateau Detection:**
- Status: "Monitoring" or "Detecting"
- Countdown when plateau detected
- Auto-advances to next step when stabilized

**Important:** Do not interrupt the tuning process. The system will:
- Automatically progress through steps
- Stop at safety limits (MAX_TEMP)
- Save data when complete
- Handle errors gracefully

### Step 6: Wait for Completion

**Duration estimates:**
- SAFE: 30-45 minutes
- STANDARD: 1-2 hours
- THOROUGH: 3-4 hours

Actual duration depends on:
- Your kiln's heating rate
- Plateau detection (may finish early)
- Ambient temperature
- Kiln insulation

**You can monitor progress remotely** - the web interface updates automatically every 2 seconds.

### Step 7: Download Tuning Data

When tuning completes:
1. Data automatically saved to: `logs/tuning_YYYY-MM-DD_HH-MM-SS.csv`
2. Copy CSV file to your laptop for analysis

**CSV Format:**
```csv
timestamp,elapsed_seconds,current_temp_c,target_temp_c,ssr_output_percent,ssr_is_on,state,progress_percent
2025-10-21 11:32:41,0.0,29.34,0.00,0.00,0,TUNING,0.0
2025-10-21 11:32:43,2.0,29.28,0.00,25.00,1,TUNING,5.0
...
```

---

## Analyzing Tuning Data

### Run the Analyzer

On your laptop, analyze the tuning data:

```bash
cd pico-kiln/

# Basic analysis (all methods)
python analyze_tuning.py logs/tuning_2025-10-21_11-32-41.csv

# Show only specific method
python analyze_tuning.py logs/tuning_2025-10-21_11-32-41.csv --method amigo
```

### Output: Terminal Report

The analyzer generates a beautiful formatted report:

```
================================================================================
                     KILN TUNING ANALYSIS REPORT
================================================================================

┌─ TEST INFORMATION ────────────────────────────────────────────────────────
│  Data Points:      1,011
│  Duration:         36.5 minutes
│  Temperature:      29.1°C → 100.3°C (Δ71.2°C)
│  Test Quality:     GOOD
│  Phases Detected:  3
│    1. HEATING  - SSR:  25.0% |  29.1°C →  55.9°C
│    2. HEATING  - SSR:  50.0% |  55.9°C →  85.3°C
│    3. COOLING  - SSR:   0.0% |  85.3°C →  50.2°C
└───────────────────────────────────────────────────────────────────────────

┌─ THERMAL MODEL PARAMETERS ────────────────────────────────────────────────
│  Dead Time (L):         10.50 seconds
│  Time Constant (τ):    120.3 seconds (2.0 min)
│  L/τ Ratio:              0.087
│  Steady-State Gain:      0.6543 °C per % SSR
│  Heat Loss (linear):     0.001234
│  Heat Loss (quad):       0.000001234
│  Ambient Temp:          29.1°C
└───────────────────────────────────────────────────────────────────────────

┌─ PID CALCULATION METHODS ─────────────────────────────────────────────────
│
│  ZIEGLER-NICHOLS
│  ────────────────────────────────────────────────────────────────────────
│    Kp:   13.750  |  Ki:    0.6548  |  Kd:   72.188
│    Fast response with moderate overshoot (~25%). Good general-purpose
│    tuning. May oscillate if system is noisy.
│
│  COHEN-COON
│  ────────────────────────────────────────────────────────────────────────
│    Kp:   15.234  |  Ki:    0.7123  |  Kd:   68.432
│    Optimized for systems with significant dead time (L/T > 0.3). Faster
│    response than Z-N with similar overshoot. Better disturbance rejection.
│
│  AMIGO ⭐ RECOMMENDED
│  ────────────────────────────────────────────────────────────────────────
│    Kp:   11.234  |  Ki:    0.5234  |  Kd:   54.678
│    Very conservative tuning with minimal overshoot (<5%). Smooth, stable
│    response. Excellent for preventing temperature overshoot in kilns.
│
│  LAMBDA
│  ────────────────────────────────────────────────────────────────────────
│    Kp:    9.876  |  Ki:    0.4567  |  Kd:    0.000
│    Lambda tuning with λ=1.5x system time constant. Predictable response
│    based on desired closed-loop speed. No derivative action (PI control).
└───────────────────────────────────────────────────────────────────────────

┌─ TEMPERATURE-RANGE-SPECIFIC PID ──────────────────────────────────────────
│  (Use these for better control across wide temperature ranges)
│
│  LOW  (0-300°C)     - Kp: 12.345 Ki:  0.5678 Kd: 60.123  [ 234 samples]
│  MID  (300-700°C)   - Kp: 10.234 Ki:  0.4567 Kd: 50.234  [ 189 samples]
│  HIGH (700-1300°C)  - Kp:  8.123 Ki:  0.3456 Kd: 40.123  [ 156 samples]
└───────────────────────────────────────────────────────────────────────────

┌─ RECOMMENDATIONS ─────────────────────────────────────────────────────────
│
│  RECOMMENDED METHOD: AMIGO
│  ────────────────────────────────────────────────────────────────────────
│    Kp = 11.234
│    Ki = 0.523
│    Kd = 54.678
│
│  ✓ Test quality is GOOD. These parameters should work well.
│    For even better tuning, consider a longer test with more temp range.
│
│  NEXT STEPS:
│  1. Update your config.py with the recommended values above
│  2. Restart the kiln controller
│  3. Test with a real firing profile and monitor for overshoot
│  4. Fine-tune if needed: reduce Kp/Ki for less overshoot, increase for
│     faster response
└───────────────────────────────────────────────────────────────────────────

================================================================================
```

### Output: JSON File

Results also saved to `tuning_results.json`:

```json
{
  "test_info": {
    "duration_s": 2193.0,
    "data_points": 1011,
    "temp_min": 29.1,
    "temp_max": 100.3,
    "phases_detected": 3
  },
  "thermal_model": {
    "dead_time_s": 10.5,
    "time_constant_s": 120.3,
    "steady_state_gain": 0.6543,
    "heat_loss_h1": 0.001234,
    "heat_loss_h2": 0.000001234,
    "ambient_temp": 29.1
  },
  "pid_methods": {
    "ziegler_nichols": {
      "kp": 13.75,
      "ki": 0.6548,
      "kd": 72.188,
      "characteristics": "Fast response with moderate overshoot..."
    },
    "amigo": {
      "kp": 11.234,
      "ki": 0.523,
      "kd": 54.678,
      "characteristics": "Very conservative tuning..."
    }
  },
  "temperature_ranges": [
    {"range": "0-300", "name": "LOW", "kp": 12.345, "ki": 0.5678, "kd": 60.123},
    {"range": "300-700", "name": "MID", "kp": 10.234, "ki": 0.4567, "kd": 50.234}
  ],
  "recommended": "amigo",
  "test_quality": "GOOD"
}
```

---

## Applying Results

### Option 1: Single PID Parameter Set (Recommended for most users)

Update `config.py` with recommended values:

```python
# PID Parameters
PID_KP = 11.234
PID_KI = 0.523
PID_KD = 54.678
```

**When to use:**
- First-time setup
- Simple firing profiles
- Temperature range < 500°C
- Prefer simplicity

### Option 2: Temperature-Range-Specific PID (Advanced)

Generate config snippet:

```bash
python generate_thermal_model_config.py
```

Output:
```python
THERMAL_MODEL = [
    {'temp_min': 0, 'temp_max': 300, 'kp': 12.345, 'ki': 0.5678, 'kd': 60.123},
    {'temp_min': 300, 'temp_max': 700, 'kp': 10.234, 'ki': 0.4567, 'kd': 50.234},
    {'temp_min': 700, 'temp_max': 9999, 'kp': 8.123, 'ki': 0.3456, 'kd': 40.123}
]
```

Copy and paste into `config.py`:

```python
# PID Parameters (fallback if THERMAL_MODEL is None)
PID_KP = 11.234
PID_KI = 0.523
PID_KD = 54.678

# Temperature-Range-Specific PID (Gain Scheduling)
THERMAL_MODEL = [
    {'temp_min': 0, 'temp_max': 300, 'kp': 12.345, 'ki': 0.5678, 'kd': 60.123},
    {'temp_min': 300, 'temp_max': 700, 'kp': 10.234, 'ki': 0.4567, 'kd': 50.234},
    {'temp_min': 700, 'temp_max': 9999, 'kp': 8.123, 'ki': 0.3456, 'kd': 40.123}
]
```

**When to use:**
- Wide temperature ranges (>500°C)
- Professional kiln operations
- Maximum control accuracy
- Complex firing profiles

**See [THERMAL_MODEL.md](THERMAL_MODEL.md) for complete details on gain scheduling**

### Restart Controller

After updating `config.py`:
1. Save the file
2. Reset the Pico 2 (CTRL+D in REPL, or power cycle)
3. Check console for initialization messages:
   ```
   [PIDGainScheduler] Initialized with 3 temperature ranges
     Range 1: 0-300°C -> Kp=12.345 Ki=0.5678 Kd=60.123
     Range 2: 300-700°C -> Kp=10.234 Ki=0.4567 Kd=50.234
     Range 3: 700-9999°C -> Kp=8.123 Ki=0.3456 Kd=40.123
   ```

---

## Technical Details

### Tuning Methods Explained

#### 1. Ziegler-Nichols (Classic)

**Best for:** General-purpose control, faster response
**Characteristics:** ~25% overshoot, may oscillate
**Formula:**
- Kp = 1.2 × (T / L)
- Ki = Kp / (2L)
- Kd = Kp × (0.5L)

**When to use:** Systems that can tolerate some overshoot

#### 2. Cohen-Coon

**Best for:** Systems with high dead time (L/T > 0.3)
**Characteristics:** Faster than Z-N, better disturbance rejection
**Formula:** More complex, optimized for L/T ratio

**When to use:** Slow-responding systems with significant lag

#### 3. AMIGO (Recommended) ⭐

**Best for:** Kilns and systems requiring minimal overshoot
**Characteristics:** <5% overshoot, very stable, smooth response
**Formula:** Conservative tuning optimized for M-constraint

**When to use:** Pottery kilns, glass kilns, any application where overshoot can damage work

#### 4. Lambda Tuning

**Best for:** Predictable, tunable response
**Characteristics:** No derivative action (PI only), adjustable speed
**Formula:** Based on desired closed-loop time constant

**When to use:** When you want specific response time characteristics

### Plateau Detection Algorithm

The system detects when temperature has stabilized:

1. **Sample Rate:** Every 60 seconds
2. **Window Size:** Last 5 readings (5 minutes)
3. **Threshold:** Temperature range < 0.5°C
4. **Action:** Advance to next step

**Benefits:**
- Avoids wasting time waiting for fixed duration
- Captures true steady-state behavior
- Faster tuning when kiln responds quickly

### Test Quality Assessment

The analyzer scores data quality based on:

1. **Data Points:** >500 = excellent, >200 = good
2. **Temperature Range:** >100°C = excellent, >50°C = good
3. **Multiple Phases:** ≥3 phases = excellent, ≥2 = good
4. **Heating Phases:** ≥2 heating cycles = excellent
5. **Thermal Parameters:** Reasonable L and τ values
6. **Duration:** >30 min = excellent, >15 min = good

**Scoring:**
- **EXCELLENT:** ≥80% score
- **GOOD:** ≥50% score
- **POOR:** <50% score

Poor quality data → conservative PID recommendations

---

## Troubleshooting

### Tuning Doesn't Start

**Symptoms:** "Start Tuning" button does nothing, or error message

**Possible Causes:**
- Another program is running
- Previous tuning session not properly stopped
- Controller in ERROR state

**Solutions:**
1. Stop any running programs first
2. Refresh web page
3. Check console logs for errors
4. Restart controller if needed

### Temperature Doesn't Rise

**Symptoms:** SSR shows ON but temperature flat

**Possible Causes:**
- SSR not connected or faulty
- Heating elements not working
- SSR wired backwards (AC side swapped)
- SSR undersized for kiln power

**Solutions:**
1. Check SSR connections and indicator LED
2. Verify heating elements with multimeter (power OFF!)
3. Check SSR relay is clicking/switching
4. Verify SSR rated for your kiln's power draw

### Temperature Rises Too Slowly

**Symptoms:** Test times out before reaching target

**Possible Causes:**
- Normal for large/well-insulated kilns
- Heating elements degraded
- Low voltage supply

**Solutions:**
1. Increase timeout in tuner.py (not recommended)
2. Lower max_temp target
3. Use SAFE mode with lower targets
4. Check heating element resistance

### Plateau Detection Not Working

**Symptoms:** Steps don't advance automatically, wait full duration

**Possible Causes:**
- Temperature oscillating (±2°C threshold not met)
- Very slow heating (never stabilizes)
- High ambient temperature variations

**Solutions:**
1. Check for drafts or temperature fluctuations
2. Ensure kiln is in stable environment
3. This is normal behavior - system will continue after timeout

### Analysis Fails with Error

**Symptoms:** `analyze_tuning.py` crashes or shows "POOR" quality

**Possible Causes:**
- Insufficient data (test too short)
- CSV file corrupted
- Very linear heating (no S-curve)
- Missing elapsed_seconds in CSV

**Solutions:**
1. Check CSV file is complete
2. Run longer tuning test (STANDARD or THOROUGH mode)
3. Script has fallback for missing elapsed_seconds
4. If quality is POOR, parameters may still work - test carefully

### Results Look Wrong (Extreme Values)

**Symptoms:** Kp > 100 or < 0.1, negative values, NaN

**Possible Causes:**
- Data quality issues
- System didn't reach steady state
- Incorrect CSV format

**Solutions:**
1. Review terminal report for warnings
2. Check "Test Quality" score
3. Re-run tuning with longer duration
4. Use STANDARD or THOROUGH mode instead of SAFE
5. Manually use default PID values as starting point

### Temperature-Range PIDs Not Generated

**Symptoms:** Analyzer doesn't show range-specific parameters

**Possible Causes:**
- Temperature span < 100°C (by design)
- Only ran SAFE or short STANDARD mode

**Solutions:**
1. This is normal for SAFE mode (100°C range)
2. Run THOROUGH mode to reach 200°C+
3. Use single PID parameter set instead
4. Temperature ranges require significant data span

---

## Best Practices

### Initial Setup

1. **Start conservative:** Always run SAFE mode first
2. **Test incrementally:** SAFE → STANDARD → THOROUGH
3. **Verify hardware:** Use SAFE mode to catch wiring issues early
4. **Document results:** Keep log of all tuning runs

### Choosing Tuning Mode

**Use SAFE when:**
- First-time kiln operation
- After hardware changes
- Quick safety verification needed
- Time constrained (<1 hour)

**Use STANDARD when:**
- Normal PID tuning needed
- Good balance of time vs. quality
- Temperature range < 500°C in actual firings
- Single PID parameter set sufficient

**Use THOROUGH when:**
- Professional/production kilns
- Wide temperature range firings (>500°C)
- Maximum control accuracy needed
- Have 3-4 hours available

### Re-Tuning

Re-run tuning if you:
- Change heating elements
- Modify kiln insulation
- Notice degraded temperature control
- See excessive overshoot (>15°C)
- Experience persistent oscillations

### Testing New PID Parameters

After applying new parameters:

1. **Start with simple test:**
   - Ramp to 200°C at 50°C/hour
   - Hold 30 minutes
   - Monitor for overshoot and oscillations

2. **Check behavior:**
   - ✅ Good: Temp tracks within ±5-10°C
   - ⚠️ Overshoot: Reduce Kp by 20%
   - ⚠️ Oscillations: Reduce Ki and Kd by 20%
   - ⚠️ Slow response: Increase Kp by 20%

3. **Gradual adjustments:**
   - Make one change at a time
   - Test after each change
   - Document results

### Safety

- ⚠️ Never leave tuning unattended (first run)
- ⚠️ Have fire extinguisher nearby
- ⚠️ Ensure proper ventilation
- ⚠️ Start with low max_temp targets
- ⚠️ Monitor console for errors
- ⚠️ Test in safe environment first

---

## Advanced Topics

### Custom Tuning Sequences

For advanced users, tuning sequences can be customized in `kiln/tuner.py`:

```python
# Example: Add custom mode
MODE_CUSTOM = 'custom'

# In __init__:
elif mode == MODE_CUSTOM:
    self.steps = [
        TuningStep(ssr_percent=30, duration=600, name="heat_30pct"),
        TuningStep(ssr_percent=60, duration=600, name="heat_60pct"),
        TuningStep(ssr_percent=0, duration=1200, name="cool_down"),
    ]
```

### Manual PID Adjustments

If automatic tuning doesn't provide ideal results:

**Proportional (Kp):**
- Controls primary response strength
- Too high → overshoot and oscillation
- Too low → slow response, large steady-state error
- Adjust in 10-20% increments

**Integral (Ki):**
- Eliminates steady-state error
- Too high → oscillations, instability
- Too low → slow elimination of error
- Adjust in 10-20% increments

**Derivative (Kd):**
- Dampens oscillations, predicts future error
- Too high → noise amplification, instability
- Too low → more overshoot
- Adjust in 10-20% increments

---

## References

- Ziegler-Nichols Tuning Method (1942)
- Cohen-Coon Tuning Method (1953)
- AMIGO Method: Åström & Hägglund (2004)
- Lambda Tuning (IMC-based): Rivera et al. (1986)
- Original inspiration: [jbruce12000/kiln-controller](https://github.com/jbruce12000/kiln-controller)

---

## Support

For issues or questions:

1. **Check documentation:**
   - This guide (TUNING.md)
   - [THERMAL_MODEL.md](THERMAL_MODEL.md)
   - [README.md](README.md)

2. **Review data:**
   - Check `tuning_results.json`
   - Examine CSV data for anomalies
   - Look at console logs

3. **Open an issue with:**
   - Tuning mode used
   - CSV data file
   - `tuning_results.json`
   - Console logs
   - Description of problem
   - Kiln specifications

---

**Happy Firing!** 🔥🏺
