# ADR-0024: Garmin login alert and gap fill

Status: accepted (import chain extended by ADR-0025; throttled and failed pulls notified by ADR-0027)

## Context
Issue #16. ADR-0008 says a refresh failure raises an alert; ADR-0022 reports a
failed session but nothing consumes it, and maps every session-open error,
throttling and outages included, to a failed session. Days missed while the
login was broken are never fetched: the backfill (ADR-0023) covers only days
before its anchor.

## Decision
- Pull outcomes: on session open, throttling (427, 429) is `Throttled`. A
  session error is `SessionFailed`: a session rejected by Garmin, a refresh
  refused with 400, 401 or 403, a missing token file, or an invalid token
  file. Anything else is a new `Failed`, which a login would not fix: network
  errors, 5xx, 408, malformed answers, and other token file I/O errors
  (permission, disk, lock). `Completed` carries the days that were fetched and
  published (or were empty).
- After every pull, `health-collector` publishes `session_valid` (source
  `health/garmin`, stamped at the pull's start): 1 if completed, 0 if the
  session failed. Throttled and failed pulls publish nothing. `store` keeps it;
  pages that list metrics ignore it.
- `alerter` owns the alert (ADR-0005). It keeps the newest 0 and the newest 1
  by their own timestamps; the login is broken while the newest 0 is newer
  than the newest 1, so order and backlog do not matter. Onset: first
  notification at once, no hold. Reminder: 12 h after the last delivered
  notification. Back: a newer 1. Delivery, retries and saves follow ADR-0017
  and ADR-0019. Notifications carry the documented login commands.
- State: `garmin_login: {failed_at, ok_at, since, notified_at}` in the alert
  state holds the whole watch, so a restart keeps both watermarks and a Back
  that failed to send stays due. It is additive; file and report stay version
  1. A moved watermark marks the state unsaved, so the next check persists it.
  Back appends history kind `garmin_login` (ADR-0021); the Alerts page names
  it "Garmin".
- Gap fill: `garmin_gap.json` next to the token file, the backfill cursor
  format (ADR-0023). After each completed pull on Paris date T, the days whose
  final value no daily pull fetched are owed, up to T-2 and no earlier than
  T-31 (30 days); `last` is the newest day fetched or owed. Only days that
  were actually fetched count: if the pull requested yesterday and it failed,
  yesterday stays owed. With no file, the last 30 days are owed. The cursor is
  saved before the first fetch, so a crash never loses owed days.
- The gap fill runs in the pull task with the backfill's runner, pacing,
  window and stop rules, before the backfill. It never logs in. The runner
  reports why it ended (done, out of time, skipped, stopped); the yearly
  backfill runs after the gap fill unless the gap fill stopped on an error,
  because the same error would stop it too.
- Rollout: consumers first. Run `docker compose pull`, then
  `docker compose up -d store alerter dashboard`, then
  `docker compose up -d health-collector`.
- Rejected: alerting inside `health-collector` (a second notifier and state),
  a hold before onset, a gap with no cap, a separate "last pull" file.

## Consequences
- One failed daily pull sends one notification; throttling and outages send
  none.
- After a re-login, at most the last 30 days are repaired. A recovery after
  10:00 fetches yesterday twice, which is harmless.
- A fresh install fetches its last 30 days twice (gap, then backfill).
- A corrupt gap file is logged and skips the gap fill until deleted.
- Older `store` images reject `session_valid` batches, so update consumers
  before the collector.
- Once the alert state has a `garmin_login` entry, an older alerter cannot
  read the state file and starts fresh (ADR-0019). Rolling the alerter back
  can resend first notifications. An older dashboard shows "Alerts
  unavailable".
