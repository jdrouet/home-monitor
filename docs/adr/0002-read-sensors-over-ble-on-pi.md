# ADR-0002: Read Xiaomi sensors directly over BLE on the Pi

Status: accepted

## Context
Thermometers are LYWSD03MMC running pvvx custom firmware (unencrypted
advertisements). Plant sensors are HHCCJCY01. A Xiaomi gateway or the Mi cloud
would add a cloud dependency.

## Decision
The Pi's built-in Bluetooth adapter reads both sensor kinds directly. No cloud,
no gateway.

## Consequences
- Sensors must be within BLE range of the Pi.
- Containerized BLE access must be solved (host BlueZ via D-Bus, or raw HCI).
