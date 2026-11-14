# ADR-0004: Deliver alerts through a self-hosted ntfy

Status: accepted

## Decision
Alerts (plant needs water, temperature too high or too low, sensor offline)
are sent through ntfy, behind a notification port. ntfy runs as a container on
the Pi; its URL and topic are configuration.

## Consequences
- No third-party service sees alert content.
- Notifications reach the phone only on the LAN until remote access exists.
