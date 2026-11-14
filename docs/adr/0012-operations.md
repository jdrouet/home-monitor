# ADR-0012: Configuration, secrets, and operations

Status: accepted

## Decision
- `config.toml` contains no secrets; the repository ships `config.example.toml`.
- Secrets (ntfy topic, credentials) come from environment variables in a
  git-ignored `.env`. Garmin tokens live in a Docker volume, mode 600.
- Config is read and validated at startup; invalid config refuses to start.
  Changes apply on restart; no hot reload.
- Logs go to stdout via `tracing`. Each service exposes `/healthz` for the
  Docker healthcheck. No service metrics in v1.
- Backups are out of scope; the data directory is plain files.
