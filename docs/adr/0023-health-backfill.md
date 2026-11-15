# ADR-0023: Garmin health backfill

Status: accepted (runner shared with the gap fill by ADR-0024; window checks within a step amended by ADR-0025)

## Context
Issue #15. ADR-0008 promised a one-time, rate-limited backfill of one year of
Garmin data. ADR-0022 deferred later syncs and failed days to it.

## Decision
- Range: the anchor D is the Paris date of the first completed pull that finds
  no cursor. The backfill covers D-365 to D-2 inclusive (364 days). The range
  is saved before the first fetch.
- Stamps: each backfilled day is stamped at its last Paris microsecond, as in
  ADR-0022.
- One sequential loop in the pull task. After each pull that ends completed,
  the backfill runs in the same task until 5 minutes before the next
  scheduled pull. A pull that Garmin throttled does not start the backfill.
  There are no concurrent token refreshes.
- Pacing: every Garmin step (a session open or a day) waits 60 s first. A step
  starts only with at least 5 minutes left, and the window is checked again
  after the wait, so a late wake starts nothing. One step is bounded by the
  HTTP timeouts (about 70 s worst case) and a token-lock wait, so the margin
  absorbs it unless the owner runs `login` concurrently (the README says to
  stop the collector first).
- Sessions: a session is reused for at most 30 minutes, below the 3600 s
  token lifetime; then a new one is opened before the next day.
- Totals: about 7 h, about 754 requests, about 2 requests per minute.
- Stops without advancing the cursor on: a failed session open, a session or
  authentication error, throttling (427 or 429) on data or refresh, a publish
  failure, a cursor save failure. The import resumes after the next completed
  pull: at most one attempt per scheduled pull, and it never logs in. Any
  other day error is logged and the day is skipped.
- Cursor: `garmin_backfill.json` next to the token file, as
  `{"version":1,"next":"YYYY-MM-DD","last":"YYYY-MM-DD"}`, written atomically
  as in ADR-0019. The import is done when `next > last` and then never
  contacts Garmin again. A missing file means no cursor. A corrupt or
  unknown-version file is logged, the import is skipped and the file is never
  overwritten. Deleting the file re-anchors the import. No config change.
- Ports: `BackfillStore` in `use-cases` (implemented by `BackfillFile` in
  `state-file`), `HealthProvider::is_throttled`. The use case stays
  timezone-free.
- Delivery is effectively once. The `Publisher` port accepts a batch once it
  is queued in the MQTT client, as for every collector. A crash after the
  cursor passed a queued day can lose that day. A crash before the cursor save
  repeats a day, which is harmless: same stamp, first stored wins (ADR-0015).
- Rejected: a concurrent task, exponential backoff, a `backfill_file` config
  key, a range ending yesterday.

## Consequences
- Re-fetching a stored day changes nothing, so recent days are not refreshed.
  Days after the anchor whose 10:00 pull failed, or that changed later, stay
  as they are; repair is a follow-up.
- A transient error on a backfilled day leaves it empty.
- Throttling or a lost session can stretch the import over several days.
