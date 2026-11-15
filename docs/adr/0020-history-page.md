# ADR-0020: History page

Status: accepted

## Context
Issue #10 needs charts of past temperature and humidity per room over 24 h,
7 d, 30 d and 1 y, with a 1-year chart in under 2 s on the home network.
ADR-0006, ADR-0007 and ADR-0015 leave the query shape, bucketing, chart
delivery and data embedding open.

## Decision
- One `GET /series` per page, without `source_id`: all sources of the metric
  come back bucketed in one scan of the day files. The dashboard keeps the
  selected configured rooms and the weather source ("Outside"); unknown
  sensors are not charted. One call per room would scan the same files again.
- Buckets: 24 h → 900 s, 7 d → 1800 s, 30 d → 7200 s, 1 y → 86 400 s, at most
  about 365 points per line. Every width is a multiple of the weather's
  15-minute interval, so the outside line has a value in every bucket. The
  x axis is every bucket of the period; a bucket without readings is a gap.
  Daily buckets start at UTC midnight.
- uPlot 1.6.32 (MIT) is vendored from
  https://unpkg.com/uplot@1.6.32/dist/ (uPlot.iife.min.js, uPlot.min.css,
  licence alongside), embedded in the binary, and served under a versioned
  path with immutable caching. Upgrading changes the files and the path
  together. A CDN was rejected: the dashboard must work without internet.
- Chart data is embedded in the page by askama's `json` filter, which escapes
  `&`, `<`, `>` and `'`, so config room names cannot break out of the script.
  No JSON endpoint.
- The time axis is shown in Europe/Paris through `uPlot.tzDate`, whatever the
  browser's zone.
- The page is a plain GET form: `metric`, `period`, repeated `room`, `outside`.
  Unknown values fall back to temperature and 24 h; with no room and no
  outside selected, every room and the outside line are shown.
- The store runs on DataFusion's multi-threaded runtime (one worker per core)
  instead of a single thread. On a development machine, with the same year of
  data and the same host load, the 1-year query had a median of 306–335 ms on
  one thread and 95–129 ms with 4 workers (3 runs each).
  Limiting DataFusion to one partition was measured slower and was dropped.

## Consequences
- The 1-year query reads 365 day files; measured 106 ms (query) and 155 ms
  first, 115–216 ms warm (page, end to end) on a development machine; the Pi
  has not been measured yet. If the Pi exceeds the 2 s target, pre-aggregated
  daily files come next, under a new ADR.
- No JavaScript build step; uPlot upgrades are manual.
