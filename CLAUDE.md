# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a kiln controller designed to run on MicroPython (Raspberry Pi Pico 2). The project is inspired by [jbruce12000/kiln-controller](https://github.com/jbruce12000/kiln-controller) but reimplemented for MicroPython to simplify integration with the MAX31856 thermocouple board.

## Hardware Target

- **Platform**: Raspberry Pi Pico 2 (dual-core RP2350)
- **Runtime**: MicroPython
- **Temperature Sensor**: MAX31856 thermocouple board (SPI)
- **Output**: SSR (Solid State Relay) control

## Architecture

The system uses both cores of the Pico 2:

### Core 1: Kiln Control (Primary)
- Temperature reading from MAX31856
- PID control algorithm
- SSR toggling based on control decisions
- Bare-metal, time-critical operations

### Core 2: Web Interface & Monitoring
- Web server for monitoring and control
- Program upload capability (kiln firing schedules)
- Real-time state monitoring (current temp, SSR status, program progress)
- Non-critical UI operations

The architecture should maintain clean separation between cores with well-defined communication channels.

## Development Setup

1. **Flash MicroPython on Pico 2**: Follow instructions at https://micropython.org/download/RPI_PICO2_W/
2. **Python Environment**: Python 3 for utility scripts
3. **Deployment**: Copy files to Pico 2's mounted filesystem

## Code Structure

When implementing, maintain this separation:
- **MicroPython code**: Runs on the Pico 2, must be MicroPython-compatible
- **Utility scripts**: Standard Python 3 for testing, PID tuning, and development tools

## Development Philosophy

- Clean, maintainable code architecture
- Clear separation of concerns between control and interface
- Thread-safe communication between cores
- MicroPython compatibility constraints (no standard library features unavailable in MicroPython)
