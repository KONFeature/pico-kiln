from lockable import Lockable
from machine import Pin, SPI

# --- Wrapper 1: DigitalInOut for the CS Pin ---
class DigitalInOut:
    def __init__(self, pin_object):
        self.pin = pin_object
        self.pin.init(mode=Pin.OUT)

    def switch_to_output(self, value=False, **kwargs):
        pass  # The library calls this, but we can ignore it.

    @property
    def value(self):
        return self.pin.value()

    @value.setter
    def value(self, val):
        self.pin.value(1 if val else 0)


# --- Wrapper 2: The Complete SPI Compatibility Class ---
class SPIWrapper(Lockable):
    def __init__(self, spi_bus: SPI, **kwargs):
        # We pass in the already-created machine.SPI object
        self._spi = spi_bus
        # Call the Lockable parent class's __init__
        super().__init__()

    def configure(
        self, baudrate=1000000, polarity=0, phase=0, bits=8, firstbit=SPI.MSB
    ):
        # This is the missing method. It maps to MicroPython's init().
        # This allows the Adafruit library to reconfigure the bus for its device.
        self._spi.init(
            baudrate=baudrate,
            polarity=polarity,
            phase=phase,
            bits=bits,
            firstbit=firstbit,
        )

    def write(self, buf, start=0, end=None):
        self._spi.write(buf)

    def readinto(self, buf, start=0, end=None):
        self._spi.readinto(buf)

    def write_readinto(self, buffer_out, buffer_in):
        self._spi.write_readinto(buffer_out, buffer_in)