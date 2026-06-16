# Boot Debugging Guide

## Problem Summary
Your Pico boots perfectly when running via `mpremote run main.py` (REPL mode) but fails silently when standalone (powered without USB console). The LED doesn't even blink.

## Why This Happens

**REPL mode (`mpremote run main.py`):**
- Skips `boot.py`
- USB console is attached
- All print() and exceptions are visible
- Different timing and memory state

**Standalone mode (power on):**
- Runs `boot.py` then `main.py`
- No console attached
- Exceptions are completely silent
- LED only initializes at line 216 of main.py (inside WiFiManager)
- If anything crashes before that, you get ZERO feedback

## Root Cause Analysis

Looking at your main.py:
1. Lines 11-22: Many imports (any could fail)
2. Line 216: WiFiManager creates LED (first visual indicator)
3. Line 358-369: Exception handling only catches at main() level

**If an import fails or exception occurs before line 216, you get:**
- No LED blink
- No error log
- No indication of what failed
- Silent death

## Debugging Strategy

I've created two tools to diagnose the issue:

### Tool 1: debug_boot.py (Minimal Diagnostic)
Tests each import stage with LED feedback

### Tool 2: main_safe.py (Full Safe Mode)
Wraps your entire boot sequence with logging and LED indicators

---

## Step 1: Use debug_boot.py (Quick Diagnosis)

This will tell you exactly WHERE the boot fails.

### Deploy:
```bash
# Backup current main.py
mpremote fs cp :main.py :main_backup.py

# Deploy debug boot
mpremote fs cp debug_boot.py :main.py

# Power cycle the Pico (unplug USB, plug back in)
```

### LED Patterns:

| Blinks | Stage | Meaning |
|--------|-------|---------|
| 1 | Boot started | LED working, power OK |
| 2 | Basic imports | asyncio, _thread, network imported |
| 3 | Config loaded | config.py loaded successfully |
| 4 | Project imports | All server/kiln modules imported |
| 5 | Success | All stages passed! |
| Fast (10Hz) | ERROR | Something failed - check log |
| Slow (0.5Hz) | Success | All successful |

### Read the log:
```bash
# Read debug log
python3 read_boot_logs.py

# Or manually:
mpremote fs cat /boot_debug.log
```

### What to look for:
- **Stops at 1 blink:** Power/LED issue (unlikely)
- **Stops at 2 blinks:** Core import failed (asyncio/network)
- **Stops at 3 blinks:** config.py has errors
- **Stops at 4 blinks:** Project module import failed (likely culprit!)
- **Fast blinking:** Check /boot_debug.log for exception details

---

## Step 2: Use main_safe.py (Production Safe Mode)

Once you identify the issue, use this for ongoing protection.

### Deploy:
```bash
# Restore original main.py
mpremote fs cp :main_backup.py :main_original.py

# Deploy safe version
mpremote fs cp main_safe.py :main.py

# Power cycle
```

### LED Patterns:

| Blinks | Stage | Meaning |
|--------|-------|---------|
| 1 | LED init | Early boot started |
| 2 | Core imports | asyncio, config imported |
| 3 | Server imports | WiFi, web_server imported |
| 4 | Kiln imports | control_thread imported |
| 5 | Entering main() | main() function started |
| 6 | Infrastructure | Queues/threads created |
| 7 | Core 1 started | Control thread running |
| 8 | Core 1 ready | Hardware initialized |
| 9 | Boot complete | System ready! |
| Solid ON | Running | System operational |
| Fast blink | Fatal error | Check logs |

### Read logs:
```bash
# Watch logs in real-time
python3 read_boot_logs.py --watch

# Or read once:
python3 read_boot_logs.py

# Or manually:
mpremote fs cat /boot_stages.log
mpremote fs cat /boot_error.log
```

---

## Step 3: Common Issues & Fixes

### Issue: Stops at import stage
**Cause:** Missing file or syntax error in module
**Fix:** Check which module failed in log, verify file exists:
```bash
mpremote fs ls
mpremote fs ls server/
mpremote fs ls kiln/
```

### Issue: Stops before Core 1 ready (Stage 8)
**Cause:** Hardware initialization timeout (MAX31856, SSR, etc.)
**Fix:** Check hardware connections, verify pins in config.py

### Issue: Stops during WiFi connection
**Cause:** WiFi timeout or configuration error
**Fix:**
- Check WIFI_SSID and WIFI_PASSWORD in config.py
- Increase timeout in main.py line 217
- Check router visibility

### Issue: Memory error (MemoryError exception)
**Cause:** Not enough RAM on cold boot
**Fix:**
- Reduce imports (use lazy loading)
- Decrease queue sizes (line 178-184)
- Reduce cache sizes

### Issue: Import order/circular dependency
**Cause:** Module A imports B, B imports A
**Fix:** Refactor imports to break circular dependencies

---

## Step 4: Advanced Debugging

### Check available memory:
```bash
mpremote exec "import gc; gc.collect(); print(f'Free: {gc.mem_free()} bytes')"
```

### Test specific module:
```bash
mpremote exec "import server.wifi_manager; print('OK')"
```

### Check all files deployed:
```bash
mpremote fs ls
mpremote fs ls server/
mpremote fs ls kiln/
mpremote fs ls static/
```

### Verify config.py:
```bash
mpremote fs cat config.py
```

---

## Step 5: Restore Normal Operation

Once you've fixed the issue:

```bash
# Option 1: Keep safe mode (recommended)
# main_safe.py provides ongoing protection

# Option 2: Restore original
mpremote fs rm :main.py
mpremote fs cp :main_original.py :main.py
```

---

## Quick Reference Commands

```bash
# Read all logs
python3 read_boot_logs.py

# Watch logs live
python3 read_boot_logs.py --watch

# Read specific log
mpremote fs cat /boot_debug.log
mpremote fs cat /boot_stages.log
mpremote fs cat /boot_error.log

# Check Pico is connected
mpremote version

# Emergency: Restore backup
mpremote fs cp :main_backup.py :main.py

# Clean logs
mpremote fs rm /boot_debug.log
mpremote fs rm /boot_stages.log
mpremote fs rm /boot_error.log
```

---

## Understanding the Difference

### mpremote run main.py:
```
[USB Console] → [REPL] → [Run main.py] → [See all output]
                 ↑
                 Exceptions visible here
```

### Standalone boot:
```
[Power On] → [boot.py] → [main.py] → [?]
                                      ↑
                                      Silent if crashes
```

The key insight: **You need logging and LED feedback BEFORE anything can fail**.

---

## Prevention

To avoid this in the future:

1. **Always test standalone** after deploying changes
2. **Keep main_safe.py** as your main.py for built-in protection
3. **Add LED indicators** at critical stages in your code
4. **Use try-except** around all imports
5. **Log to files** not just print() statements
6. **Test memory usage** with `gc.mem_free()` regularly

---

## Need More Help?

If the logs show something unclear:

1. Copy the full /boot_error.log content
2. Note which LED pattern you saw
3. Share the error log for detailed analysis

The logs will contain:
- Exact line number of failure
- Full exception traceback
- Module that failed to import
- Timestamp of each boot stage
