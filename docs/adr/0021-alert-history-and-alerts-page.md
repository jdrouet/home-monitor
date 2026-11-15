# ADR-0021: Alert history and the alerts page

Status: accepted (alert kinds and state amended by ADR-0024)

## Context
Issue #11 needs a dashboard page with active alerts (what, since when) and
alerts resolved in the last 30 days (start, end). The persisted alert state
(ADR-0019) records only the last notification, and the dashboard reads only
`store` (ADR-0018).

## Decision
- `alerter` owns alert history. Each active alert records `since`, when its
  first notification was delivered; reminders keep it. When a Back or
  Resolved notification is delivered, the alert is appended to `history` as
  `{source_id, kind (offline | too_cold | too_hot), started, ended}`, ended
  being the delivery time. A direction switch ends the old alert at the
  switch and starts a new one.
- History is kept in the state file and pruned to 30 days at every save and
  every publish. The file stays `"version":1`: `since` and `history` are
  additive. A file without `since` uses the last notification time, the
  closest known; a file without `history` has none.
- `alerter` serves `GET /alerts` on its `http_listen`:
  `{"version":1,"offline":{..},"temperature":{..},"history":[..]}`, the
  alert state in an envelope owned by `domain`. The check loop publishes the
  state through a watch channel after every check pass, so the handler never
  touches the single-threaded alert state. The report is refreshed after
  every check pass, normally every 60 s, later when ntfy is slow.
- `dashboard` reads it from `[dashboard] alerter_url`, default
  `http://alerter:8080`, and renders `/alerts` server-side: active alerts by
  onset, resolved alerts newest first, room names from the config (unknown
  sources by id), times in Europe/Paris. If the alerter is unreachable the
  page says alerts are unavailable; overview and history are unaffected.
  This amends ADR-0018: the dashboard no longer reads only `store`.
- Publishing lifecycle events over MQTT into `store` was rejected: alerts do
  not fit the measurement schema (ADR-0011), and it would add a store data
  kind, an alerter publisher and a second copy of state the alerter owns.

## Consequences
- History survives restarts with the state file and is lost with it.
- Alerts active at upgrade time show their last notification as start.
- An older alerter reads the file, keeps reminder schedules, and drops
  `since` and `history` at its next save.
- Older images reject `alerter_url` if set; the default makes it unneeded.
- Removing a sensor or its range drops its alert with no history entry.
