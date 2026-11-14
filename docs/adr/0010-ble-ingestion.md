# ADR-0010: BLE ingestion through BlueZ D-Bus, behind an advertisement port

Status: accepted

## Decision
- `ble-collector` reads advertisements through an advertisement-source port.
  First adapter: host BlueZ via `bluer`, with the host D-Bus socket mounted
  into the container and discovery reporting duplicate data.
- Decoders (BTHome v2 for thermometers, Xiaomi MiBeacon for HHCCJCY01) are
  independent of the source.
- The collector emits at most one measurement per sensor per metric per
  minute (last value in the window, deduplicated by BTHome packet id).

## Consequences
- No extra container privileges.
- Remote ESP32 BLE-to-MQTT listeners can be added later as a second adapter,
  for plant sensors out of the Pi's range.
