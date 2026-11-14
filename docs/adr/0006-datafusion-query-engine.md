# ADR-0006: Query Parquet with DataFusion

Status: accepted

## Decision
`store` queries Parquet with DataFusion (pure Rust, partition pruning) and
exposes a small typed HTTP API: series by metric, source, and time range,
with optional bucketing.

## Alternatives
DuckDB (heavy C++ build on aarch64); hand-rolled `parquet`/`arrow` reads.
