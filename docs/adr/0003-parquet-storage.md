# ADR-0003: Store measurements in Parquet files

Status: accepted

## Context
History must be kept forever, and queries should not load all data.

## Decision
- `store` appends every incoming measurement to a write-ahead log (NDJSON).
- Once a day, `store` flushes the previous day's log into one file per metric:
  `data/metric=<metric>/date=<YYYY-MM-DD>/data.parquet`, then truncates the log.
- Queries read Parquet plus the unflushed log, so today's data is visible.

## Consequences
- A crash loses nothing that reached the log.
- One file per metric per day; no compaction job.
- No database server to run.
