# ADR-0022: Garmin health collection

Status: accepted (backfill consequence amended by ADR-0023; session outcomes and repair of missed days amended by ADR-0024)

## Context
Issue #12 needs daily sleep, resting heart rate, steps and Body Battery from
Garmin Connect, which has no personal API. ADR-0008 fixes the login model.
garth's SSO + OAuth1 flow stopped accepting new logins in March 2026, and no
Rust crate implements the current flow soundly. ADR-0011 stamps daily values
at the start of the day; with first-stored-wins deduplication (ADR-0015) a
day's first, partial value would be kept and its final value dropped.

## Decision
- A `garmin-connect` crate implements the flow python-garminconnect 0.3.20
  uses: load the mobile sign-in page, `POST sso.garmin.com/mobile/api/login`,
  `POST /mobile/api/mfa/verifyCode` when asked, then exchange the service
  ticket at `diauth.garmin.com/di-oauth2-service/oauth/token` for a DI access
  and refresh token. It sits behind a `HealthProvider` port in `use-cases`.
- `health-collector login` prompts for the email, the password (not echoed)
  and the MFA code, makes one attempt, stops on 403, 427 or 429, checks the
  token against the API and saves it. It never retries.
- Tokens live in `[health_collector] token_file`, default
  `/tokens/garmin_tokens.json` on the `garmin-tokens` volume, as
  `di_token`, `di_refresh_token`, `di_client_id`: python-garminconnect's
  format, so tokens obtained elsewhere can be copied in. The file is written
  0600 to a temp file, fsynced and renamed; its directory is 0700. Tokens,
  passwords, URLs and response bodies never reach logs or errors.
- A pull or login holds an exclusive lock on the token file's `.lock` sibling
  for its whole read-refresh-write sequence.
- Each pull reads the file, refreshes the access token, writes the file back,
  reads the display name, then fetches the daily user summary and daily sleep
  per day, sequentially, 3 s apart (6 requests). A failed session skips the
  pull and is reported as such for alerting (#16); a session or authentication
  error during data fetches also counts as a failed session; any other failed
  day is skipped.
- Pulls run at startup and daily at 10:00 Europe/Paris. Today is always
  pulled; yesterday only from 10:00 on.
- Source kind `health`, source id `garmin`: `sleep_duration` (s),
  `sleep_score`, `resting_heart_rate` (bpm), `steps`, `body_battery_max` and
  `body_battery_min` (0–100). Missing values are not published.
- Timestamps, amending ADR-0011: a completed Europe/Paris day is stamped at
  its last microsecond, today at the pull's start. Readers take, per Paris
  day and metric, the latest reading. Repeated pulls of a completed day are
  deduplicated. Rejected: start of day (freezes the first partial value),
  a last-wins store (changes ADR-0015 for every source), fetch time for
  every value (lands on the day after the one it describes).
- The dashboard Health page shows the last 30 Paris days as a table, one
  `/series` call per metric without buckets.
- `paris-clock` holds the Europe/Paris helpers shared by `dashboard` and
  `health-collector`; `use-cases` stays timezone-free.
- `[health_collector]` (`http_listen`, `token_file`) is optional; the
  collector refuses to start without it.

## Consequences
- Garmin's terms forbid unauthorized third-party access and Garmin blocks
  clients; logins or pulls can stop working without notice.
- The login can be blocked by Cloudflare client fingerprinting; tokens can
  then be produced elsewhere and copied into the volume.
- Yesterday keeps the value Garmin had at 10:00. The backfill (ADR-0023)
  covers only days before the first run, so later syncs and days whose 10:00
  pull failed are not repaired.
- A re-login is needed when the refresh token expires or is revoked; its
  lifetime is not documented.
- Older images reject the section and cannot decode health batches: update
  every image, then add `[health_collector]` and log in.
