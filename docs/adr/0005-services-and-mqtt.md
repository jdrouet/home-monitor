# ADR-0005: Six services in one workspace, connected by MQTT

Status: accepted

## Decision
One Cargo workspace with shared `domain` and `use-cases` crates and one
binary per service: `ble-collector`, `weather-collector`, `health-collector`,
`store`, `alerter`, `dashboard`. Each binary is a composition root that wires
adapters to use cases.

Collectors publish measurements to Mosquitto. `store` and `alerter` subscribe
independently. Messages are JSON with a versioned schema owned by `domain`.

## Consequences
- `alerter` reacts in real time and does not depend on `store`.
- One extra container (Mosquitto).
