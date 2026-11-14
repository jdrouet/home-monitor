# ADR-0015: Write-ahead log, flush, and series query

Status: accepted

## Context
Issue #13 needs readings kept across restarts, stored once, and queried per day
without reading all history. ADR-0003, ADR-0006 and ADR-0011 leave the details open.

## Decision
- `store` acks an MQTT batch only after it is appended and fsynced (manual acks).
  A failed append or ack ends the process; the broker redelivers after restart.
  Invalid payloads are acked and dropped.
- The log has one NDJSON segment per UTC date of the reading's timestamp. At
  startup a torn last line is truncated and each segment is fsynced before it is
  used for deduplication; any other corrupt line refuses startup. New directories
  are fsynced into their parents before the first ack.
- At startup and then every hour, every segment older than today (UTC) is flushed:
  per metric, merged with the existing day file, written to a temp file, fsynced,
  renamed, directories fsynced. The segment is deleted last, so a crash at any
  point is redone idempotently. Late readings rewrite their day file.
- Deduplication on (timestamp, source_id, metric): the first stored reading wins.
  Appends drop readings already in the log or in their day file; flushes keep the
  existing day-file row; the latest values keep the first value per timestamp.
- Before deleting segments, a flush writes a snapshot of the latest values;
  startup rebuilds `/latest` from it plus the log, without scanning history.
- Day files hold timestamp (µs, UTC), source_kind, source_id, value; metric and
  date come from the path. Compression: zstd.
- `GET /series?metric&from&to[&source_id][&bucket_secs]`: µs bounds, `to`
  exclusive; buckets are epoch-aligned averages per source. The response uses the
  batch envelope; a bucket's timestamp is its start.
- Storage sits behind a `MeasurementStore` port in `use-cases`. A new `storage`
  crate implements it and is the only crate depending on DataFusion (parquet
  feature only, no SQL). Queries and flushes exclude each other so a day is
  never read twice.
- The data directory is set by `[store] data_dir` and mounted as a Compose volume.

## Consequences
- No acked reading is lost and none is stored or served twice.
- One file per metric per day, rewritten when late readings arrive.
- Appends and flushes block the store's single-threaded runtime briefly.
