# ADR-0016: Native multi-arch image builds published to ghcr.io

Status: accepted

## Context
Issue #2 needs every merge to main to publish images for the Pi (linux/arm64)
and a PC (linux/amd64). Emulating arm64 with QEMU is too slow for a cold musl
build (about 11 minutes natively), and the vendored libdbus and zstd-sys builds
are proven only on `rust:1-alpine`. ADR-0009 assumed cross-compilation.

## Decision
- Each architecture builds on a native GitHub runner (`ubuntu-24.04`,
  `ubuntu-24.04-arm`) from the same Dockerfile. No QEMU, no cross toolchain;
  this replaces the cross-compilation in ADR-0009.
- One Dockerfile: a cargo-chef dependency layer, one builder stage compiling
  all three binaries in one `cargo build`, and one `alpine:3` stage per service
  selected with `--target`.
- Layers are cached in the GitHub Actions cache, one scope per architecture.
- Images are `ghcr.io/jdrouet/home-monitor-<service>`, tagged `latest` and
  `sha-<7-char commit>`, both multi-arch. Each architecture pushes
  `sha-<commit>-<arch>`; a merge job joins them into `sha-<commit>`, and
  `latest` moves only to the current head of main, one run at a time.
  Publishing runs on push to main and on manual dispatch only.
- Pull requests build both architectures without pushing.
- `compose.yaml` runs the published images; `compose.build.yaml` adds local builds.

## Consequences
- The Pi never builds; updating is `docker compose pull && docker compose up -d`.
- On the Pi, Docker starts after BlueZ (OpenRC ordering) and `ble-collector`
  mounts `/run/dbus`, so services come back after a cold boot.
- Architecture-suffixed tags accumulate in the registry.
- A change to `Cargo.lock` or the Rust base image means a cold build per architecture.
