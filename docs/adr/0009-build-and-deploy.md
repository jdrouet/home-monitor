# ADR-0009: Build in GitHub Actions, deploy with Compose

Status: accepted (build method refined by ADR-0016)

## Decision
Public GitHub repository. GitHub Actions cross-compiles static musl aarch64
binaries into minimal images on ghcr.io. The Pi runs one `compose.yaml`.
