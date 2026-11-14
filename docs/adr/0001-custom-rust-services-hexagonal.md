# ADR-0001: Custom Rust services with hexagonal architecture

Status: accepted

## Context
Home Assistant used too many resources on the Pi and its configuration was
cumbersome. Off-the-shelf pipelines (BLE gateway, Telegraf, TSDB, Grafana)
bring back the same many-configs problem.

## Decision
Build the system as several small Rust services, including the UI. Each
service follows a hexagonal architecture: domain and use cases at the core,
ports as traits, adapters for BLE, HTTP providers, storage, and ntfy.
Existing crates are reused for heavy lifting (storage formats, querying).

## Consequences
- We own all code and configuration; no third-party config dialects.
- More code to write than an assembled stack.
- Providers (weather, health, notification) are swappable behind ports.
