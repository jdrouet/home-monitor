# ADR-0012: Configuration, secrets, and operations

Status: accepted (state moved to bind-mounted folders for #41)

## Decision
- `config.toml` contains no secrets; the repository ships `config.example.toml`.
- Secrets (ntfy topic, credentials) come from environment variables in a
  git-ignored `.env`. Garmin tokens live in `data/garmin-tokens`, the
  directory 0700 and the file 0600, both set by `health-collector`.
- Config is read and validated at startup; invalid config refuses to start.
  Changes apply on restart; no hot reload.
- Logs go to stdout via `tracing`. Each service exposes `/healthz` for the
  Docker healthcheck. No service metrics in v1.
- All service state lives in git-ignored folders under `data/` next to
  `compose.yaml`, bind-mounted into the containers, not in named volumes:
  it is visible on the Pi and backed up by copying `data/` while the
  services are stopped. Docker creates a missing folder as root at startup;
  the containers run as root, so they can write it.
