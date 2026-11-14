# ADR-0009: Build in GitHub Actions, deploy with Compose

Status: accepted

## Decision
Public GitHub repository. GitHub Actions cross-compiles static musl aarch64
binaries into minimal images on ghcr.io. The Pi runs one `compose.yaml`.
