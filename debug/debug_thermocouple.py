# debug/debug_thermocouple.py
# Thermocouple Debug Script - Tests MAX31856 thermocouple initialization and reading
#
# This script attempts to initialize the MAX31856 thermocouple sensor and
# perform multiple temperature readings to verify correct operation.
#
# Usage:
#   mpremote run debug/debug_thermocouple.py
#
# What it tests:
#   - SPI bus initialization
#   - MAX31856 sensor initialization
#   - Temperature reading (multiple samples)
#   - Fault detection and reporting
#   - Cold junction (reference) temperature
#   - Sensor configuration verification

import time
from machine import SPI, Pin
import sys

def log(message):
    """Print log message with timestamp"""
    timestamp = time.ticks_ms()
    print(f"[{timestamp:08d}] {message}")

def print_wiring_guide():
    """Print detailed wiring guide for MAX31856"""
    log("\n" + "=" * 60)
    log("MAX31856 Wiring Guide")
    log("=" * 60)
    log("Power connections:")
    log("  MAX31856 VIN  -> Pico 3V3(OUT)  [3.3V power]")
    log("  MAX31856 GND  -> Pico GND       [Ground]")
    log("")
    log("SPI connections (from config):")
    log("  MAX31856 SCK  -> Pico GP18      [SPI Clock]")
    log("  MAX31856 SDO  -> Pico GP16      [MISO - Data from sensor]")
    log("  MAX31856 SDI  -> Pico GP19      [MOSI - Data to sensor]")
    log("  MAX31856 CS   -> Pico GP28      [Chip Select]")
    log("")
    log("Thermocouple connections:")
    log("  T+  -> Thermocouple + (usually RED for K-type)")
    log("  T-  -> Thermocouple - (usually YELLOW for K-type)")
    log("")
    log("IMPORTANT:")
    log("  - MAX31856 MUST use 3.3V (NOT 5V!)")
    log("  - Thermocouple polarity matters!")
    log("  - Screw terminals must be tight")
    log("  - Keep thermocouple wires away from power wires")
    log("=" * 60 + "\n")

def init_spi(spi_id, sck_pin, mosi_pin, miso_pin, baudrate=500000):
    """Initialize SPI bus"""
    log(f"Initializing SPI bus {spi_id}...")
    log(f"  SCK  Pin: GP{sck_pin}")
    log(f"  MOSI Pin: GP{mosi_pin}")
    log(f"  MISO Pin: GP{miso_pin}")
    log(f"  Baudrate: {baudrate} Hz")
    
    try:
        spi = SPI(
            spi_id,
            baudrate=baudrate,
            polarity=0,
            phase=1,
            sck=Pin(sck_pin),
            mosi=Pin(mosi_pin),
            miso=Pin(miso_pin)
        )
        log("SPI bus initialized successfully")
        return spi
    except Exception as e:
        log(f"SPI initialization failed: {e}")
        sys.print_exception(e)
        return None

def init_max31856(spi, cs_pin, thermocouple_type):
    """Initialize MAX31856 thermocouple sensor"""
    log(f"\n--- Testing MAX31856 at CS pin GP{cs_pin} ---")
    
    try:
        # Import necessary modules
        log("Importing MAX31856 driver...")
        # Add lib directory to path if not already there
        if '/lib' not in sys.path:
            sys.path.append('/lib')
        
        from wrapper import DigitalInOut, SPIWrapper
        import adafruit_max31856
        from adafruit_max31856 import ThermocoupleType
        
        # Create chip select pin
        log("Creating chip select pin...")
        cs = DigitalInOut(Pin(cs_pin, Pin.OUT))
        cs.value = True  # Deselect chip initially
        log("  CS pin initialized (HIGH/deselected)")
        
        # Wrap SPI for Adafruit library compatibility
        log("Wrapping SPI bus for Adafruit library...")
        wrapped_spi = SPIWrapper(spi)
        
        # Create MAX31856 sensor object
        log(f"Creating MAX31856 sensor (thermocouple type: {thermocouple_type})...")
        sensor = adafruit_max31856.MAX31856(
            wrapped_spi,
            cs,
            thermocouple_type=thermocouple_type
        )
        
        log("MAX31856 sensor initialized successfully!")
        
        # Wait for first conversion to complete
        log("Waiting for first conversion (200ms)...")
        time.sleep_ms(200)
        
        return sensor
        
    except Exception as e:
        log(f"MAX31856 initialization failed: {e}")
        sys.print_exception(e)
        return None

def test_sensor_config(sensor):
    """Test and display sensor configuration"""
    log("\n--- Sensor Configuration ---")
    
    try:
        # Check averaging setting
        avg = sensor.averaging
        log(f"Averaging: {avg} sample(s)")
        
        # Check noise rejection setting
        noise_rej = sensor.noise_rejection
        log(f"Noise Rejection: {noise_rej} Hz")
        
        # Check temperature thresholds
        tc_thresholds = sensor.temperature_thresholds
        log(f"Thermocouple Thresholds: Low={tc_thresholds[0]}°C, High={tc_thresholds[1]}°C")
        
        # Check reference (cold junction) temperature thresholds
        cj_thresholds = sensor.reference_temperature_thresholds
        log(f"Cold Junction Thresholds: Low={cj_thresholds[0]}°C, High={cj_thresholds[1]}°C")
        
        return True
        
    except Exception as e:
        log(f"Failed to read sensor configuration: {e}")
        sys.print_exception(e)
        return False

def diagnose_voltage_issue(sensor):
    """Diagnose common voltage-related issues"""
    log("\n--- Voltage Issue Diagnostics ---")
    log("Common causes of voltage faults:")
    log("  1. Thermocouple not connected (open circuit)")
    log("  2. Thermocouple wires reversed")
    log("  3. Poor/loose connections at screw terminals")
    log("  4. MAX31856 power supply issues (VDD must be 3.3V)")
    log("  5. Damaged thermocouple wire")
    log("  6. Wrong thermocouple type setting")
    log("")
    log("Troubleshooting steps:")
    log("  1. Verify thermocouple is connected to + and - terminals")
    log("  2. Check polarity (usually red=+ for K-type)")
    log("  3. Tighten screw terminals firmly")
    log("  4. Measure VDD pin: should be ~3.3V")
    log("  5. Try touching thermocouple wires together (should read ~room temp)")
    log("  6. Check for physical damage to thermocouple wire")
    
    try:
        # Read fault register details
        log("\nDetailed fault analysis:")
        faults = sensor.fault
        
        if faults.get('open_tc'):
            log("  ❌ OPEN CIRCUIT: Thermocouple not connected or broken wire")
        
        if faults.get('voltage'):
            log("  ❌ VOLTAGE FAULT: Over/under voltage detected")
            log("     - Check VDD is 3.3V (not 5V!)")
            log("     - Verify thermocouple connections")
            
        if faults.get('tc_range'):
            log("  ❌ THERMOCOUPLE RANGE: Temperature out of range for this TC type")
            
        if faults.get('cj_range'):
            log("  ❌ COLD JUNCTION RANGE: Internal sensor issue")
            
        return True
        
    except Exception as e:
        log(f"Failed to diagnose voltage issue: {e}")
        return False

def check_faults(sensor):
    """Check and display any sensor faults"""
    try:
        faults = sensor.fault
        
        # Check if any faults are active
        active_faults = [name for name, active in faults.items() if active]
        
        if active_faults:
            log(f"⚠️  FAULTS DETECTED: {', '.join(active_faults)}")
            log("Fault details:")
            for name, active in faults.items():
                if active:
                    log(f"  - {name}: ACTIVE")
            return False
        else:
            log("✅ No faults detected")
            return True
            
    except Exception as e:
        log(f"Failed to read fault status: {e}")
        sys.print_exception(e)
        return False

def test_temperature_reading(sensor, num_readings=5):
    """Test multiple temperature readings"""
    log(f"\n--- Testing Temperature Readings ({num_readings} samples) ---")
    
    if not sensor:
        log("No sensor to test (initialization failed)")
        return False
    
    success_count = 0
    temperatures = []
    
    for i in range(num_readings):
        try:
            log(f"\nReading {i+1}/{num_readings}:")
            
            # Check faults BEFORE attempting to read (prevents hanging)
            log("  Pre-read fault check...")
            fault_free = check_faults(sensor)
            if not fault_free:
                log("  ⚠️  Faults detected before reading - skipping this sample")
                continue
            
            # Use non-blocking approach: initiate measurement, then wait
            log("  Initiating one-shot measurement...")
            sensor.initiate_one_shot_measurement()
            
            # Wait for measurement with timeout
            log("  Waiting for measurement to complete (~160ms)...")
            timeout_ms = 500  # 500ms timeout (measurement should take ~160ms)
            start_time = time.ticks_ms()
            
            while sensor.oneshot_pending:
                if time.ticks_diff(time.ticks_ms(), start_time) > timeout_ms:
                    log(f"  ⚠️  Timeout waiting for measurement!")
                    raise Exception("Measurement timeout")
                time.sleep_ms(10)
            
            log("  Measurement complete, unpacking temperature...")
            
            # Read the temperature value (should be instant now)
            temp = sensor.unpack_temperature()
            log(f"  Thermocouple: {temp:.2f}°C")
            temperatures.append(temp)
            
            # Read cold junction (reference) temperature
            log("  Reading cold junction temperature...")
            cj_temp = sensor.unpack_reference_temperature()
            log(f"  Cold Junction: {cj_temp:.2f}°C")
            
            # Check for faults after reading
            log("  Post-read fault check...")
            fault_free = check_faults(sensor)
            
            # Sanity check temperature range
            if temp < -50 or temp > 1500:
                log(f"  ⚠️  WARNING: Temperature {temp}°C outside reasonable range!")
            else:
                log(f"  ✅ Temperature in reasonable range")
            
            if fault_free:
                success_count += 1
            
            # Wait before next reading
            if i < num_readings - 1:
                time.sleep(1)
                
        except Exception as e:
            log(f"  ❌ Reading {i+1} failed: {e}")
            sys.print_exception(e)
            # Try to check faults after error
            try:
                log("  Checking faults after error...")
                check_faults(sensor)
            except:
                pass
    
    # Summary
    log(f"\n--- Temperature Reading Summary ---")
    log(f"Successful readings: {success_count}/{num_readings}")
    
    if temperatures:
        avg_temp = sum(temperatures) / len(temperatures)
        min_temp = min(temperatures)
        max_temp = max(temperatures)
        temp_range = max_temp - min_temp
        
        log(f"Average temperature: {avg_temp:.2f}°C")
        log(f"Min temperature: {min_temp:.2f}°C")
        log(f"Max temperature: {max_temp:.2f}°C")
        log(f"Temperature range: {temp_range:.2f}°C")
        
        if temp_range > 5:
            log(f"⚠️  WARNING: Large temperature variation ({temp_range:.2f}°C)")
            log("    This could indicate:")
            log("    - Unstable sensor readings")
            log("    - Rapid temperature changes")
            log("    - Electrical noise")
        else:
            log(f"✅ Temperature readings stable (variation: {temp_range:.2f}°C)")
    
    return success_count == num_readings

def test_averaging(sensor):
    """Test different averaging settings"""
    log("\n--- Testing Averaging Settings ---")
    
    if not sensor:
        log("No sensor to test (initialization failed)")
        return
    
    avg_settings = [1, 2, 4, 8, 16]
    
    for avg in avg_settings:
        try:
            log(f"\nTesting averaging = {avg}:")
            sensor.averaging = avg
            
            # Verify setting was applied
            actual_avg = sensor.averaging
            if actual_avg == avg:
                log(f"  ✅ Averaging set to {actual_avg}")
            else:
                log(f"  ⚠️  Expected {avg}, got {actual_avg}")
                continue
            
            # Take a reading
            log(f"  Reading temperature...")
            temp = sensor.temperature
            log(f"  Temperature: {temp:.2f}°C")
            
            time.sleep(0.5)
            
        except Exception as e:
            log(f"  ❌ Averaging test failed: {e}")
            sys.print_exception(e)
    
    # Reset to default
    try:
        sensor.averaging = 1
        log("\nReset averaging to 1 (default)")
    except:
        pass

def main():
    """Main debug routine"""
    
    print("=" * 60)
    log("MAX31856 Thermocouple Debug Script Starting")
    print("=" * 60)
    
    # Show wiring guide first
    print_wiring_guide()
    
    try:
        # Configuration from config.py
        SPI_ID = 0
        SCK_PIN = 18
        MOSI_PIN = 19
        MISO_PIN = 16
        CS_PIN = 28
        BAUDRATE = 500000
        
        # Import thermocouple type
        from adafruit_max31856 import ThermocoupleType
        THERMOCOUPLE_TYPE = ThermocoupleType.K
        
        log(f"Configuration:")
        log(f"  SPI Bus: {SPI_ID}")
        log(f"  SCK Pin: GP{SCK_PIN}")
        log(f"  MOSI Pin: GP{MOSI_PIN}")
        log(f"  MISO Pin: GP{MISO_PIN}")
        log(f"  CS Pin: GP{CS_PIN}")
        log(f"  Baudrate: {BAUDRATE} Hz")
        log(f"  Thermocouple Type: K")
        
        # Initialize SPI bus
        spi = init_spi(SPI_ID, SCK_PIN, MOSI_PIN, MISO_PIN, BAUDRATE)
        if not spi:
            log("\nERROR: Failed to initialize SPI bus!")
            log("Check wiring:")
            log(f"  - SCK  -> GP{SCK_PIN}")
            log(f"  - MOSI -> GP{MOSI_PIN}")
            log(f"  - MISO -> GP{MISO_PIN}")
            log(f"  - CS   -> GP{CS_PIN}")
            log("  - VCC  -> 3.3V")
            log("  - GND  -> GND")
            return
        
        # Initialize MAX31856 sensor
        sensor = init_max31856(spi, CS_PIN, THERMOCOUPLE_TYPE)
        if not sensor:
            log("\nERROR: Failed to initialize MAX31856 sensor!")
            log("Possible issues:")
            log("  - Check SPI wiring")
            log("  - Verify CS pin connection")
            log("  - Ensure MAX31856 is powered (3.3V)")
            log("  - Check for solder bridges on breakout board")
            log("  - Verify thermocouple is connected to +/- terminals")
            return
        
        # Display sensor configuration
        config_ok = test_sensor_config(sensor)
        
        # Initial fault check
        log("\n--- Initial Fault Check ---")
        initial_faults = check_faults(sensor)
        
        # If voltage or open circuit faults detected, show diagnostics
        if not initial_faults:
            diagnose_voltage_issue(sensor)
            log("\n⚠️  Faults detected at startup!")
            log("Fix the issues above before proceeding with temperature readings.")
            log("The script will continue anyway to gather more diagnostic data...\n")
        
        # Test temperature readings
        readings_ok = test_temperature_reading(sensor, num_readings=5)
        
        # If readings failed, show diagnostics again
        if not readings_ok:
            diagnose_voltage_issue(sensor)
        
        # Test averaging settings (only if basic readings worked)
        if readings_ok:
            test_averaging(sensor)
        else:
            log("\n--- Skipping Averaging Tests (basic readings failed) ---")
        
        # Final summary
        print("\n" + "=" * 60)
        if readings_ok and config_ok:
            log("✅ All tests PASSED! Thermocouple is working correctly.")
        else:
            log("⚠️  Some tests FAILED - check output for details")
        print("=" * 60)
        
    except Exception as e:
        log(f"\nFATAL ERROR: {e}")
        sys.print_exception(e)
    
    print("\nDebug script finished")

if __name__ == "__main__":
    main()
