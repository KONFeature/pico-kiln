# boot.py
# Runs before main.py on every boot
# Add delay to allow power to stabilize and hardware to settle

import time

print("[Boot] Pico Kiln Controller - Boot sequence")
print("[Boot] Waiting for power to stabilize...")

# Wait for power and hardware to settle
# This is especially important when thermocouple is connected at boot
time.sleep(0.5)

print("[Boot] Power stable, proceeding to main.py...")
