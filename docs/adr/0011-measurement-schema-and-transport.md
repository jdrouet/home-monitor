# ADR-0011: Narrow measurement schema, QoS 1 MQTT transport

Status: accepted (daily timestamps amended by ADR-0022)

## Decision
- Schema: `timestamp (UTC, µs) | source_kind | source_id | metric | value (f64)`.
  The unit is implied by the metric enum. Daily values (Garmin) use the start
  of the day as timestamp.
- Topic `measurements/<source_kind>/<source_id>`, JSON batch payload, QoS 1.
- `store` and `alerter` use persistent sessions; Mosquitto persistence on.
- `store` deduplicates on `(timestamp, source_id, metric)`.
- `alerter` persists active alerts and reminder timers to a JSON state file.

## Consequences
- Adding a metric needs no schema change.
- At-least-once delivery, made idempotent by deduplication.
