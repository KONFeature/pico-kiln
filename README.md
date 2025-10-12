# pico-kiln

A kiln controller running on MicroPython for the Raspberry Pi Pico 2.

Greatly inspired by: https://github.com/jbruce12000/kiln-controller

## Overview

This project implements a kiln temperature controller using the Raspberry Pi Pico 2's dual-core architecture:

- **Core 1**: Handles time-critical kiln control operations
  - Reads temperature from MAX31856 thermocouple board
  - Runs PID control algorithm
  - Controls SSR (Solid State Relay) based on PID output

- **Core 2**: Provides web interface and monitoring
  - Serves web interface for real-time monitoring
  - Allows uploading kiln firing programs
  - Displays current temperature, SSR status, and program state

## Hardware Requirements

- Raspberry Pi Pico 2 (RP2350)
- MAX31856 thermocouple board (SPI)
- SSR (Solid State Relay) for kiln control
- Thermocouple (K-type recommended)

## Setup

1. Flash MicroPython on the Pico 2: https://micropython.org/download/RPI_PICO2_W/
2. Copy the MicroPython files to the Pico 2's filesystem
3. Wire the MAX31856 board to the Pico 2's SPI pins
4. Connect the SSR to a GPIO pin

## Development

The project includes utility scripts for:
- Testing hardware connections
- PID tuning and optimization
- Simulating kiln behavior