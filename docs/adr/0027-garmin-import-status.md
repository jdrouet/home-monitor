# ADR-0027: Garmin import status and give-up notification

Status: accepted

## Context
Issue #43. The 2026-11-16 01:00 archive stop was found only in the logs.
ADR-0024 notifies only a broken login, and ADR-0023, ADR-0024 and ADR-0025
define the jobs and the chain that runs after each daily pull.

## Decision
- Five jobs are reported: daily pull, missed days, one-year health import,
  workout archive and daily archive. Each keeps its latest attempt (first try,
  then retries from #44 later, oldest first), a `retry_at` and progress read
  from its cursor. An attempt ends done, paused or failed with one of ten
  reasons, each mapped from its source errors. Every ended attempt carries
  counts of stored, empty and failed days (or workouts). A stop after some
  successful days keeps their count, and a done attempt with failed days is
  shown as partial, never as up to date.
- `health-collector` keeps the job records in memory and serves
  `GET /imports` on its existing `http_listen` (a version 1 report in
  `domain`). Progress and the finish estimate are computed per request from the
  cursors. The records carry only enums, timestamps, dates and counts.
- The dashboard reads it from `[dashboard] health_collector_url` and renders
  `/imports`, a fifth nav page (ADR-0026). It shows only the latest attempt.
  An unreachable collector gives a notice and status 502; without
  `[health_collector]` the page says Garmin is not set up. The dashboard
  client reports a category only (unreachable, timeout, bad response). No
  transport or parser text reaches the HTML or the logs, and report content is
  escaped.
- The alerter polls the same endpoint in its own task every 60 s with a 5 s
  timeout, only with `[health_collector]`, from `[alerter] health_collector_url`.
  The URL is parsed at startup, so a malformed value stops the alerter
  (ADR-0012). A failed poll logs a category only and affects nothing but the
  import notification. Delivery, retry and saving follow ADR-0017 and ADR-0019.
- A give-up is a job that stops and waits for the next scheduled pull:
  the daily pull ending throttled, session-failed or failed, or a chained job
  stopping, including a skip caused by an unreadable progress file (that job
  only; the chain continues). Done and paused are never give-ups. The pull
  loop fixes the scheduled pull before it starts and passes it as the deadline;
  `retry_at` is that time. It is set only when the failure ends before the
  deadline, so an overrunning pull, which the loop retries at once, sets none
  and sends nothing.
- The alerter sends one notification per give-up, keyed `(job, retry_at)`. It
  keeps only the newest delivered `retry_at` per job in the alert state as
  `garmin_imports` (additive, omitted when empty; no pruning). It accepts a
  give-up only when its `retry_at` is newer than that value and still in the
  future, and checks again just before sending. A daily pull refused for its
  login is not notified; ADR-0024 covers it. This amends ADR-0024: throttled
  and failed daily pulls now notify. A startup pull and a restart pull that
  fail the same day share one key.
- The finish estimate uses a cost per job from how each runner paces. A health
  day (gap fill, backfill) costs 62 s (one 60 s step plus the session reopen
  every 30 min spread over its steps). An archive day costs a 60 s step plus its
  paced documents at a mean 4.5 s gap, about 254 s for 43 daily documents. The
  constants sit beside the catalog counts. The runner active at the last record
  gets an estimate "at" a time when its remaining work ends within the window
  cutoff (the next pull minus the 5 min margin). Otherwise it gets a count of
  further import days: remaining time divided by a window of 24 h minus the
  5 min margin, rounded up. "Waits for" is given
  only when the recorded chain state proves the dependency: an earlier job is
  running or stopped in the current window. Every other case is unknown,
  including a missing or unreadable cursor, workout history, a finished activity
  history with catch-up pending, and a page cap hit. Garmin gives no workout
  total, and no request is added to learn it.

## Rejected
- MQTT status messages: the subscriber decodes only measurement batches
  (ADR-0011, as in ADR-0021), a new metric breaks older `store` and `alerter`
  under a consumers-first rollout, and the dashboard would still need a second
  channel for the detail.
- A status file in the token folder: it holds secrets and is mounted only into
  `health-collector`.
- Persisting job records: progress already persists in the cursors.
- A section on the Health page: one priority and one refresh period per page,
  and five cards would push the table below the fold on a phone.
- Notifying from the collector: a second notifier and state, as ADR-0024 said.

## Rollout and rollback
Both `health_collector_url` keys are optional. Every service parses the shared
`config.toml` with `deny_unknown_fields`, so update every image first, then set
either key if needed. Remove the keys before rolling any image back. No store
change.

## Consequences
- After a collector restart a job shows "Not run yet" until the startup pull
  reaches it.
- Accepted losses: a give-up notification can be lost when the collector
  restarts within one poll (60 s) of the give-up, because records are in
  memory, and when it disappears or expires before the first successful alerter
  poll, including a polling outage longer than 60 s. A crash of ntfy or a lost
  response can repeat one notification (ADR-0017, ADR-0019).
- The estimate is rough. It ignores archive days already stored, which makes it
  too high, and the pause around the daily pull. Unknown is shown instead of a
  guess when the chain state does not support one.
- An older alerter drops `garmin_imports` at its next save and can repeat one
  notification after a rollback.
- A down collector logs one alerter warning per minute and one dashboard error
  per page request.
- The notification says "10:00" literally, so a schedule change must update it.
- Nav width: five tabs fit at 390 px after the nav minimum goes from 5.5rem to
  4.5rem; at 320 px the nav wraps to two rows, which the design system allows.
