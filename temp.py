from machine import Pin, SPI
from utime import sleep
from wrapper import DigitalInOut, SPIWrapper

# Define the thermocouple type
import adafruit_max31856

thermocouple_type = adafruit_max31856.ThermocoupleType.K

spi_cs_pin = Pin(28, Pin.OUT)
spi_cs = DigitalInOut(spi_cs_pin)  # Wrap it for the library

spi = SPIWrapper(
    SPI(
        0,  # Use SPI bus 0
        baudrate=1000000,
        sck=Pin(18),  # Hardware sck on GP 18
        mosi=Pin(19),  # Hardware mosi (RX) on GP 19
        miso=Pin(16),  # Hardware miso (TX) on GP 16
    )
)

# Init thermocouple
thermocouple = adafruit_max31856.MAX31856(
    spi, spi_cs, thermocouple_type=thermocouple_type
)

# Read the temp
def raw_temp(couple):
    # The underlying adafruit library does not throw exceptions
    # for thermocouple errors. Instead, they are stored in
    # dict named self.thermocouple.fault. Here we check that
    # dict for errors and raise an exception.
    # and raise Max31856_Error(message)
    temp = couple.thermocouple.temperature
    for k, v in couple.thermocouple.fault.items():
        if v:
            print("Reading error")
    return temp


print("Start to read temps...")
while True:
    try:
        temp = thermocouple.temperature
        print("Thermocouple temp:", temp)

        faults = thermocouple.fault
        if any(faults.values()):
            print(f"Thermocouple fault detected: {faults}")

        sleep(1)  # sleep 1sec
    except KeyboardInterrupt:
        break
print("Finished.")
