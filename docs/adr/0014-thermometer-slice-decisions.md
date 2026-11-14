# ADR-0014: Thermometer slice decisions

Status: accepted

## Context
The first slice (issue #1) needed choices that earlier ADRs leave open:
crate boundaries, identifiers, the batch wire format, the D-Bus build, the
template engine, and how BLE data is trusted.

## Decision
- Crates: libraries `domain`, `bthome`, `config`, `use-cases`, `mqtt`,
  `bluez`; binaries `ble-collector`, `store`, `dashboard`.
- `source_id` is the sensor's MAC address, uppercase with colons. The
  `ATC_xxxxxx` name is only a comment in the config.
- Batches are `{"version":1,"measurements":[...]}` with timestamps in µs
  since the epoch (UTC). `store` returns the same envelope from `GET /latest`.
- libdbus is vendored through the `dbus/vendored` feature, so CI and images
  need no system D-Bus packages.
- `dashboard` renders HTML with askama, which needs only a derive macro.
- The first packet per sensor after startup is ignored: BlueZ replays cached
  data, so only a new packet id proves a fresh advertisement.
- A failed publish is requeued; a newer value for the same metric wins.

## Consequences
- One wire format for MQTT and HTTP; changing it means a new version.
- Up to one reading per sensor is lost at each collector start.
- Building the collector compiles libdbus from source.
