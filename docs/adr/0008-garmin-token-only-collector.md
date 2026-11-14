# ADR-0008: Garmin collector never logs in by itself

Status: accepted

## Context
Garmin Connect has no personal API. The unofficial SSO login is fragile and
Cloudflare-rate-limited; repeated logins have caused 48-hour lockouts.

## Decision
A `health-collector login` subcommand performs the interactive login (MFA on
the terminal) and persists tokens. The running collector only refreshes tokens
and calls data endpoints. A refresh failure raises an alert; it never retries
the login.

Data is pulled daily at 10:00 (yesterday and today, since sleep is finalized
late), with a one-time, rate-limited backfill of one year.

## Consequences
- Manual re-login when tokens expire or are revoked.
- No login retry loop can lock the account.
