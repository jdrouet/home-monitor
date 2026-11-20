# ADR-0027: Garmin import status and give-up notification

Status: accepted (retries added for #44)

## Context
Issue #43. The 2026-11-16 01:00 archive stop was found only in the logs.
ADR-0024 notifies only a broken login, and ADR-0023, ADR-0024 and ADR-0025
define the jobs and the chain that runs after each daily pull.

## Decision
- Five jobs are reported: daily pull, missed days, one-year health import,
  workout archive and daily archive. Each keeps its attempts in the current window
  (first try, then retries, oldest first), a `retry_at`, a `next_retry` and
  progress read from its cursor. An attempt ends done, paused (deadline), capped (the
  workout page limit; the next job still runs) or failed with one of ten
  reasons, each mapped from its source errors. The report enums have no
  unknown-variant fallback: a new outcome or reason needs every reader updated
  first. Every ended attempt carries
  counts of stored, empty and failed days (or workouts). A stop after some
  successful days keeps their count, and a done attempt with failed days is
  shown as partial, never as up to date.
- `health-collector` keeps the job records in memory and serves
  `GET /imports` on its existing `http_listen` (a version 1 report in
  `domain`). Progress and the finish estimate are computed per request from the
  cursors. The records carry only enums, timestamps, dates and counts.
- The dashboard reads it from `[dashboard] health_collector_url` and renders
  `/imports`, a fifth nav page (ADR-0026). It shows the latest attempt and how many retries ran before it in this window.
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
- Retries (#44). A chained job that stops because Garmin did not answer
  (`Unreachable`), had a server error (`GarminDown`) or answered badly
  (`BadAnswer`: other non-session 4xx, truncated or undecodable bodies) is
  retried. A rate-limit answer (427, 429) is not a bad answer: Garmin asked the
  client to slow down, and retrying at 5 and 15 minutes could lengthen the block.
  The other reasons wait for the next pull because a retry fails the same way
  and needs the owner: `LoginRejected`, `LoginFileUnreadable`,
  `ArchiveUnavailable`, `ProgressUnreadable` (a skip), `ProgressNotSaved` (local
  disk) and `PublishFailed` (not a Garmin failure; the client reconnects and the
  next window resumes from the cursor). The classification is an exhaustive
  match, so a new reason must be classified.
  - Schedule: retries start 5, 15 and 60 minutes after the failed attempt
    ends. The budget is 3 per job per window, so each chained job has its own.
    After the third failed retry the job gives up until the next pull.
  - Guard: a retry is planned only when a step started at its time still fits,
    that is at least 6 minutes (the 60 s step spacing plus the 5 min margin)
    before the next pull. The cutoff limits when steps start, not when they
    finish: no step is admitted inside the margin, a late wake from a backoff
    starts nothing and ends paused, and an admitted slow step may finish past
    the margin, after which the runner admits no further step.
  - A retry re-runs only the failed job, in a new session, through the same
    paced adapter. Then the chain goes on with the next job, which gets its
    first attempt. Earlier jobs are not re-run. A job that gives up ends the
    chain as before. The loop lives in use-cases with an injected clock.
  - A catch-up page cap (ten completed pages) applies per attempt, so a retry
    gets ten pages of its own. Resuming repeats no stored work: runners resume
    from their cursors, the archive checks for a key before every fetch, and a
    repeated health day has the same stamp (ADR-0015).
  - The 10:00 daily pull is not retried. A failed pull leaves yesterday owed to
    the gap fill at the next completed pull, the startup pull already covers
    restarts, and a retried pull would publish `session_valid` again at another
    stamp, which changes the login alert's input (ADR-0024).
  - The retry budget and wake time are in memory like the other records. A
    collector restart starts a fresh run with a fresh budget. This is accepted:
    restarts are rare and operator-driven, and the pacer still applies.
  - A health day skipped inside the missed days or the history, on a transport
    error, a 5xx or a malformed day, keeps the ADR-0023 behaviour: skipped and
    logged. It can remain missing, because the gap fill does not generally
    revisit it apart from its rule for yesterday. A follow-up is suggested.
- A give-up is a job that stops and waits for the next scheduled pull, after
  any retries:
  the daily pull ending throttled, session-failed or failed, or a chained job
  stopping, including a skip caused by an unreadable progress file (that job
  only; the chain continues). Done and paused are never give-ups. The pull
  loop fixes the scheduled pull before it starts and passes it as the deadline;
  `retry_at` is that time. It is set only when the failure ends before the
  deadline, so an overrunning pull, which the loop retries at once, sets none
  and sends nothing. A job that will retry is not a give-up: `next_retry` holds
  the instant of the planned retry, `retry_at` keeps its meaning and is never set
  together with it, and the give-up signal stays "the last attempt failed and
  `retry_at` is set". The report stays version 1 and `next_retry` is omitted
  when unset.
- The alerter sends one notification per give-up, keyed `(job, retry_at)`. It
  keeps only the newest delivered `retry_at` per job in the alert state as
  `garmin_imports` (additive, omitted when empty; no pruning). It accepts a
  give-up only when its `retry_at` is newer than that value and still in the
  future, and checks again just before sending. The message says how many retries
  failed. A daily pull refused for its login is not notified; ADR-0024 covers it. This amends ADR-0024: throttled
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
  5 min margin, rounded up. A paused job with work left is the one that resumes
  first, so it is estimated the same way, counting from the next pull. "Waits
  for" is given only when the recorded chain state proves the dependency: an
  earlier job is running, stopped, or paused with work left in the current
  window. Every other case is unknown,
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
- Redefining `retry_at` as the next try with a separate give-up marker: an old
  alerter would notify every failed try.
- Retrying throttling or the daily pull, re-running the chain from its start, a
  retry loop in the collector, and one retry budget per window.

## Rollout and rollback
Upgrades from the version before #44 can go in any order; consumers first
(`dashboard`, `alerter`, then `health-collector`) shows retries from the start.
Per key: an old alerter stays silent during a backoff and alerts without a
count; an old dashboard shows the next-pull message while a retry is pending;
an old collector's alert and a new collector's give-up in the same window share
a key, so the second is suppressed by design.

Both `health_collector_url` keys are optional. Every service parses the shared
`config.toml` with `deny_unknown_fields`, so update every image first, then set
either key if needed. Remove the keys before rolling any image back. No store
change.

## Consequences
- One failing job can hold the chain up to 80 minutes plus its runs, so later
  jobs get less of the window. Runner WARN lines still say "resuming after the
  next pull" when a retry follows.
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
- Nav width: the five tabs share the width in equal columns and stay on one
  row down to 320 px, with a smaller font below 360 px.
