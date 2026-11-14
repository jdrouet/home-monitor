# ADR-0018: Outdoor weather collection

Status: accepted

## Context
Issue #4 needs outdoor temperature, humidity and today's forecast on the
overview. ADR-0005, ADR-0011 and ADR-0015 leave the details open: how a
forecast fits the measurement schema, which timestamps to use, and how to
reach an HTTPS provider from musl images.

## Decision
- `weather-collector` polls Open-Meteo every 15 min behind a
  `WeatherProvider` port in `use-cases`, implemented by a new `open-meteo`
  crate. A failed fetch or publish is logged and skipped; the next poll
  replaces it.
- Each poll becomes one batch under source kind `weather`, source id
  `open-meteo`, topic `measurements/weather/open-meteo`: `temperature`,
  `humidity`, and new metrics `temperature_min`, `temperature_max` (°C,
  today's forecast) and `weather_code` (WMO code).
- All five values carry Open-Meteo's `current.time`, the start of its
  15-minute interval, requested as Unix time. Today's forecast is a forecast
  as issued at that instant, not a daily value, so it does not use ADR-0011's
  start-of-day timestamp: with first-wins deduplication that would freeze the
  first forecast of the day. Repeated polls in one interval are deduplicated.
- "Today" is the Europe/Paris day (`timezone=Europe/Paris`, `forecast_days=1`).
- The dashboard still reads only `store`'s `/latest`. Weather is unavailable
  once its newest measurement is 1 h old, the sensors' offline rule; the
  forecast is shown only while the weather is available; the forecast is shown
  only if it was issued during the current Europe/Paris day.
- A retained MQTT topic for the forecast was rejected: the dashboard would
  need an MQTT client and would no longer depend only on `store`, and the
  forecast history would be lost.
- `weather-collector` reads `[weather_collector]` (`http_listen`, `latitude`,
  `longitude`). The section is optional so existing configs stay valid for the
  other services; the collector refuses to start without it. Latitude must be
  in [-90, 90] and longitude in [-180, 180].
- HTTPS uses reqwest's `rustls` feature (aws-lc-rs) with the system CA store,
  which `alpine:3` provides. A ring provider through `rustls-no-provider` was
  rejected: Cargo merges features across the workspace, and every other
  reqwest client would then panic without an installed provider. Plain HTTP
  was rejected because the request contains the home's coordinates.

## Consequences
- One reading per 15-minute interval is kept, and today's forecast keeps its
  history.
- The overview hides a forecast issued before the current Paris day, so after
  midnight it reappears with the next successful poll.
- Every binary using reqwest links aws-lc; builds and binaries grow.
- Older services reject the new config section and cannot decode weather
  batches, so rollout is staged: update every image first, then add
  `[weather_collector]` and restart. Rolling `store` back after weather data was
  stored needs that data removed or a compatible build.
