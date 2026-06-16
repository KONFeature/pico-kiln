# boot.py
# Runs before main.py on every boot
# Add delay to allow power to stabilize and hardware to settle

import time
import micropython
import gc

print("[Boot] Pico Kiln Controller - Boot sequence")

# CRITICAL: Allocate emergency exception buffer for better crash diagnostics
# This allows tracebacks during memory exhaustion or in ISRs
micropython.alloc_emergency_exception_buf(100)
print("[Boot] Emergency exception buffer allocated (100 bytes)")

# Optimize garbage collection threshold for predictive collection
# Trigger GC when heap is ~25% full to avoid emergency GC during critical operations
gc.collect()
gc.threshold(gc.mem_free() // 4 + gc.mem_alloc())
print(f"[Boot] GC threshold set (free: {gc.mem_free()} bytes)")

print("[Boot] Waiting for power to stabilize...")

# Wait for power and hardware to settle
# This is especially important when thermocouple is connected at boot
time.sleep(0.5)

print("[Boot] Power stable, proceeding to main.py...")
