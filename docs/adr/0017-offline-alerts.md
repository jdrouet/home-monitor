# ADR-0017: Offline alert decisions

Status: accepted (alert state persistence superseded by ADR-0019)

## Context
Issue #6 needs a notification when a sensor stops reporting. ADR-0004,
ADR-0011 and ADR-0012 leave the details open, and ADR-0011's alert state file
is out of scope for this issue.

## Decision
- `alerter` subscribes to `measurements/#` at QoS 1 with a persistent session
  under client id `alerter`. It watches configured sensors only; unknown
  sensors never raise an alert.
- A sensor is offline once its newest measurement is 1 h old. The first
  notification names its room; a reminder follows 12 h after the last delivered
  one; a "back" notification follows its next measurement, only if an offline
  notification was delivered.
- Last seen is the measurement's own timestamp, never the receive time, and it
  only moves forward, so redelivered or backlog batches cannot hide or end an
  outage.
- Alert state is kept in memory only, replacing for now the JSON state file of
  ADR-0011. At startup every configured sensor counts as seen at startup time.
- A check runs every 60 s. State changes only after ntfy accepts a
  notification; a failed send is retried at the next check.
- Each notification is decided from current state just before it is sent, and a
  reminder counts from the moment the previous notification was accepted.
- Notifications go through a `Notifier` port in `use-cases`, implemented by a
  new `ntfy` crate: a POST to `<url>/<topic>` with the message as body and the
  title as the `title` query parameter.
- The ntfy URL is `[alerter] ntfy_url` in `config.toml`. The topic is a secret
  (ADR-0012): the `NTFY_TOPIC` environment variable from `.env`, validated at
  startup as 1 to 64 letters, digits, `-` or `_`. Errors never include the ntfy
  URL, so the topic stays out of logs.
- ntfy runs as a Compose service on the LAN, host port 8081, with its default
  in-memory cache. The image builder now compiles the `alerter` binary too.

## Consequences
- A restart forgets active alerts: an outage that started before it gets no
  "back" message, and a sensor already dead alerts 1 h after startup.
- A notification whose response is lost is sent again at the next check.
- Notifications arrive up to 60 s after their threshold.
- Compose needs a `.env` file before the stack starts.
