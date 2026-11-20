# Home Monitor — Context

Lightweight replacement for Home Assistant on a Raspberry Pi 4B. Collects
indoor climate, plant, weather, and personal health data; stores history;
shows a dashboard; sends alerts.

## Glossary

- **Sensor**: a physical device that emits measurements. Identified by its BLE
  MAC address. Kinds: *thermometer* (LYWSD03MMC, pvvx firmware) and *plant
  sensor* (HHCCJCY01 "Flower Care").
- **Source**: anything that produces measurements: the BLE scanner, the
  weather provider, the Garmin account.
- **Measurement**: one value of one metric at one instant, from one source
  (for example, temperature 21.4 °C in the bedroom at 10:02).
- **Metric**: what a measurement quantifies: temperature, humidity, soil
  moisture, soil conductivity, light, battery, resting heart rate, and so on.
- **Weather provider**: an external service for outdoor conditions. Abstracted
  behind a port; Open-Meteo is the first adapter.
- **Health provider**: an external service for personal health data. Garmin
  Connect (unofficial API) is the first adapter.
- **Garmin archive**: Garmin's original documents, kept as received in a
  directory of files: per completed day of the past year and per workout
  (including its original file). Empty files mean Garmin has no data; a
  `.denied` file means Garmin refused the document (ADR-0025).
- **Alert**: a notification sent when a rule's condition holds (plant needs
  water, temperature too high or too low, sensor offline). Delivered through
  ntfy. Lifecycle: fires on onset, reminds every 12 h while active, sends a
  resolved message on clear.
- **Rule**: a threshold with hysteresis and a hold duration (15 min for
  temperature, 1 h for soil moisture), set per room or per plant.
- **Offline**: a sensor with no measurement for 1 h.
- **Room**: a named place a thermometer is assigned to.
- **Plant**: a named plant a plant sensor is assigned to, with its own rules.

## Configuration

One hand-edited `config.toml`, shared by all services: sensor MAC to room or
plant, rules, provider settings. Unknown sensors are logged and stored under
their MAC.

Timestamps are stored in UTC and displayed in `Europe/Paris`. The weather
location (latitude, longitude) is set in `config.toml`.

## Dashboard (v1)

- **Overview**: current value per room and plant, outdoor weather, stale markers.
- **History**: chart per metric, ranges 24 h / 7 d / 30 d / 1 y.
- **Health**: Garmin daily metrics.
- **Alerts**: active alerts and recent history.
- **Imports**: what each Garmin job did, its progress and its failures.

## Testing

- Use cases tested with in-memory adapters.
- Decoders tested against captured advertisement bytes.
- One `store` integration test: log, flush, query through DataFusion.
- No real BLE or Garmin in CI.

## Environment

- Raspberry Pi 4B, 4 GB RAM, Alpine Linux (aarch64), SSD-booted, reachable as
  `ssh rpi4`.
- All workloads run in Docker (Compose). BlueZ and D-Bus run on the host,
  adapter `hci0`.
- LAN only for now; remote access is out of scope.
- Thermometers advertise BTHome v2, unencrypted (service UUID `0xFCD2`):
  battery, temperature, humidity.
- 3 thermometers seen: `A4:C1:38:1C:02:76`, `A4:C1:38:E1:6F:B2`,
  `A4:C1:38:45:51:3E`. 5 plant sensors exist but were not seen by the Pi.

## Delivery order

1. Thermometer slice: workspace, `ble-collector` (BTHome), Mosquitto, `store`,
   minimal overview page.
2. `weather-collector` (Open-Meteo).
3. `alerter`: temperature and offline rules, self-hosted ntfy.
4. Plant sensors (MiBeacon) and plant rules.
5. History, Health, and Alerts pages.
6. `health-collector` (Garmin).

## Open items

- Plant sensors not received by the Pi: check batteries and placement, then
  rescan. If still out of range, add an ESP32 advertisement source (ADR-0010).
