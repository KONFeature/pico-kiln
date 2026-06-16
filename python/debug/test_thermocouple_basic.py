# test_thermocouple_basic.py
# Ultra-simple thermocouple test - just read faults and try one measurement
# This helps isolate whether issue is thermocouple wiring or MAX31856 chip

import time
from machine import SPI, Pin
import sys

# Add lib to path
if '/lib' not in sys.path:
    sys.path.append('/lib')

from wrapper import DigitalInOut, SPIWrapper
import adafruit_max31856
from adafruit_max31856 import ThermocoupleType

print("=" * 60)
print("Simple Thermocouple Test")
print("=" * 60)

# Configuration
SPI_ID = 0
SCK_PIN = 18
MOSI_PIN = 19
MISO_PIN = 16
CS_PIN = 28

print("\n1. Initializing SPI...")
spi = SPI(
    SPI_ID,
    baudrate=500000,
    polarity=0,
    phase=1,
    sck=Pin(SCK_PIN),
    mosi=Pin(MOSI_PIN),
    miso=Pin(MISO_PIN)
)
print("   SPI OK")

print("\n2. Setting up CS pin...")
cs = DigitalInOut(Pin(CS_PIN, Pin.OUT))
cs.value = True
print("   CS OK")

print("\n3. Wrapping SPI...")
wrapped_spi = SPIWrapper(spi)
print("   Wrapper OK")

print("\n4. Creating MAX31856 sensor...")
sensor = adafruit_max31856.MAX31856(
    wrapped_spi,
    cs,
    thermocouple_type=ThermocoupleType.K
)
print("   Sensor created OK")

print("\n5. Waiting for first conversion (200ms)...")
time.sleep_ms(200)

print("\n" + "=" * 60)
print("CRITICAL TEST: Check faults WITH and WITHOUT thermocouple")
print("=" * 60)

def check_faults_detailed():
    """Check and display faults in detail"""
    faults = sensor.fault
    
    print("\nFault Register Status:")
    print(f"  open_tc (Open Circuit):     {faults['open_tc']}")
    print(f"  voltage (Over/Under Volt):  {faults['voltage']}")
    print(f"  tc_range (TC Range):        {faults['tc_range']}")
    print(f"  tc_high (TC High):          {faults['tc_high']}")
    print(f"  tc_low (TC Low):            {faults['tc_low']}")
    print(f"  cj_range (CJ Range):        {faults['cj_range']}")
    print(f"  cj_high (CJ High):          {faults['cj_high']}")
    print(f"  cj_low (CJ Low):            {faults['cj_low']}")
    
    return faults

print("\n--- TEST 1: Current Configuration ---")
print("Check what faults are present RIGHT NOW:")
faults = check_faults_detailed()

if faults['voltage']:
    print("\n⚠️  VOLTAGE FAULT DETECTED!")
    print("\nThis fault means one of two things:")
    print("  A) Thermocouple input voltage is out of range")
    print("  B) Internal voltage reference issue")
    
if faults['open_tc']:
    print("\n⚠️  OPEN CIRCUIT DETECTED!")
    print("Thermocouple is not connected or wire is broken")

print("\n" + "=" * 60)
print("MANUAL TEST REQUIRED")
print("=" * 60)
print("\nPlease perform this test:")
print("1. DISCONNECT thermocouple from T+ and T- terminals")
print("2. Press ENTER")

input()

print("\n--- TEST 2: With Thermocouple DISCONNECTED ---")
faults = check_faults_detailed()

if faults['open_tc'] and not faults['voltage']:
    print("\n✅ GOOD! With TC disconnected:")
    print("   - open_tc fault: YES (expected)")
    print("   - voltage fault: NO (expected)")
elif faults['voltage']:
    print("\n❌ BAD! Voltage fault persists WITHOUT thermocouple!")
    print("   This indicates a problem with the MAX31856 chip itself:")
    print("   - Damaged chip")
    print("   - Bad solder joint on breakout board")
    print("   - Defective board")
    print("\n   Try a different MAX31856 board if possible")

print("\n3. Now CONNECT thermocouple to T+ and T- terminals")
print("   (Make sure screws are TIGHT)")
print("4. Press ENTER")

input()

print("\n--- TEST 3: With Thermocouple CONNECTED ---")
faults = check_faults_detailed()

if not faults['open_tc'] and not faults['voltage']:
    print("\n✅ EXCELLENT! Faults cleared with thermocouple connected")
    print("   Trying to read temperature...")
    
    try:
        # Try a simple read
        sensor.initiate_one_shot_measurement()
        
        # Wait for measurement
        timeout = 50  # 500ms
        waited = 0
        while sensor.oneshot_pending and waited < timeout:
            time.sleep_ms(10)
            waited += 1
        
        if waited >= timeout:
            print("   ⚠️  Timeout waiting for measurement")
        else:
            temp = sensor.unpack_temperature()
            cj_temp = sensor.unpack_reference_temperature()
            
            print(f"\n✅ SUCCESS!")
            print(f"   Thermocouple Temperature: {temp:.2f}°C")
            print(f"   Cold Junction Temperature: {cj_temp:.2f}°C")
            
            # Sanity check
            if 15 <= temp <= 35:
                print(f"\n✅ Temperature looks reasonable for room temp")
            elif temp < -50 or temp > 1500:
                print(f"\n⚠️  Temperature seems out of range")
                print("   Possible issues:")
                print("   - Wrong thermocouple type (check if K-type)")
                print("   - Thermocouple polarity reversed")
            else:
                print(f"\n   Temperature: {temp:.2f}°C")
                
    except Exception as e:
        print(f"\n❌ Error reading temperature: {e}")
        sys.print_exception(e)
        
elif faults['voltage'] and not faults['open_tc']:
    print("\n❌ VOLTAGE FAULT with thermocouple connected")
    print("\nThis usually means:")
    print("  1. Thermocouple input voltage is too high/low")
    print("  2. Wrong type of sensor connected (not a thermocouple)")
    print("  3. Thermocouple is damaged/shorted")
    print("  4. Electrical noise on thermocouple wires")
    
    print("\nTROUBLESHOOTING:")
    print("  - Verify you're using a real thermocouple (not RTD/thermistor)")
    print("  - Check thermocouple isn't damaged")
    print("  - Keep thermocouple wires away from power cables")
    print("  - Try a different thermocouple if available")
    
elif faults['open_tc'] and faults['voltage']:
    print("\n❌ BOTH open_tc AND voltage faults!")
    print("   Likely causes:")
    print("   - Loose connection in screw terminals")
    print("   - Intermittent connection")
    print("   - Damaged thermocouple wire")
    
    print("\nTROUBLESHOOTING:")
    print("   - Remove wires, clean ends, re-insert")
    print("   - Tighten screws VERY firmly")
    print("   - Check for broken wire near connectors")
    
elif faults['open_tc']:
    print("\n❌ Still showing open_tc fault")
    print("   The sensor doesn't detect the thermocouple")
    print("   - Check polarity (RED=+, YELLOW=-)")
    print("   - Verify screws are TIGHT")
    print("   - Check for broken wire")

print("\n" + "=" * 60)
print("Additional Diagnostic Info:")
print("=" * 60)

try:
    print(f"Noise rejection: {sensor.noise_rejection} Hz")
    print(f"Averaging: {sensor.averaging} samples")
    
    # Try to read cold junction even with faults
    try:
        cj_temp = sensor.unpack_reference_temperature()
        print(f"Cold junction temp: {cj_temp:.2f}°C")
        
        if 15 <= cj_temp <= 35:
            print("  ✅ Cold junction reading looks normal (chip is working)")
        else:
            print("  ⚠️  Cold junction temp seems off")
            
    except Exception as e:
        print(f"Could not read cold junction: {e}")
        
except Exception as e:
    print(f"Error reading config: {e}")

print("\n" + "=" * 60)
print("Test Complete")
print("=" * 60)
