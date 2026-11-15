# home-monitor

Reads Bluetooth temperature and humidity sensors and shows the climate of each
room on a dashboard and charts past temperature and humidity per room, and sends a phone notification when a sensor stops
reporting or a room gets too hot or too cold. It also shows the outdoor weather and today's forecast from
Open-Meteo ([ADR-0018](docs/adr/0018-weather-collection.md)). It can also pull
daily sleep, resting heart rate, steps and Body Battery from Garmin Connect
([ADR-0022](docs/adr/0022-health-collection.md)). It runs as eight
containers: Mosquitto, ntfy, `ble-collector`, `store`, `dashboard`, `alerter`,
`weather-collector` and `health-collector`. The host needs BlueZ and D-Bus, because
`ble-collector` talks to the Bluetooth adapter through them.

## Images

CI publishes `ghcr.io/jdrouet/home-monitor-ble-collector`,
`-store`, `-dashboard`, `-alerter`, `-weather-collector` and `-health-collector` for `linux/amd64` and `linux/arm64`. Tags are
`latest` (head of main) and `sha-<7-char commit>`. See
[ADR-0016](docs/adr/0016-multi-arch-images.md).

The packages must be public, or the host must run `docker login ghcr.io` with a
personal access token that has `read:packages`.

## Raspberry Pi setup (Alpine, 64-bit)

```sh
apk add docker docker-cli-compose bluez
rc-update add bluetooth default
rc-update add docker default
echo 'rc_after="bluetooth"' >> /etc/conf.d/docker
service bluetooth start
service docker start

git clone https://github.com/jdrouet/home-monitor.git
cd home-monitor
cp config.example.toml config.toml
cp .env.example .env
```

Edit `config.toml` to list your thermometers and set the `latitude` and
`longitude` of `[weather_collector]`. Keep `[store] data_dir = "/data"`.
Edit `.env` and set `NTFY_TOPIC` to a hard-to-guess name (letters, digits, `-`
or `_`, up to 64 characters): anyone who knows the topic can read your alerts.
Then start the services:

```sh
docker compose up -d
```

Containers use `restart: unless-stopped`, and Docker starts after BlueZ at boot,
so the services come back after a reboot. After a cold reboot, check
`docker compose ps` and that `docker compose logs ble-collector` shows the first
packets. `bluetooth` pulls in `dbus`, which `ble-collector` needs.

## Alerts

`alerter` sends a notification through ntfy when a sensor has not reported for
an hour, a reminder every 12 hours while it stays offline, and a message when it
reports again. Install the ntfy app on your phone, set the server to
`http://<pi>:8081` and subscribe to your `NTFY_TOPIC`. ntfy is reachable on the
local network only; do not expose port 8081 to the internet. See
[ADR-0017](docs/adr/0017-offline-alerts.md).

It also warns when a room stays outside its temperature range for 15 minutes,
repeats the warning every 12 hours, and says when the room is back in range. Set
the range per room with `min_temperature` and `max_temperature` (°C) in its
`[[thermometers]]` entry in `config.toml`; a room without them is not checked.
`alerter` keeps the alerts it has sent in `/data/alerts.json` on the
`alerter-data` volume (`[alerter] state_file` changes the path), so a restart
does not repeat them. See
[ADR-0019](docs/adr/0019-temperature-alerts-and-alert-state.md).

The dashboard's Alerts page lists the active alerts and those resolved in the
last 30 days. `alerter` keeps that history in the same state file and serves it
on its `http_listen` port; the dashboard reads it from `[dashboard] alerter_url`
(default `http://alerter:8080`). Update every image before setting
`alerter_url`: older images reject the key. See
[ADR-0021](docs/adr/0021-alert-history-and-alerts-page.md).

## Update

```sh
docker compose pull && docker compose up -d
```

Run `git pull` first if `compose.yaml` changed. If `config.example.toml` gained
new sections, copy them into your `config.toml`.

### Adding the weather collector to an existing install

Older images reject the `[weather_collector]` section and cannot decode weather
batches, so update in stages:

1. Run `docker compose pull && docker compose up -d`. `weather-collector`
   restarts until it is configured.
2. Add `[weather_collector]` from `config.example.toml` to `config.toml`, with
   your latitude and longitude.
3. Run `docker compose up -d --force-recreate` (or restart the services).

### Adding the health collector to an existing install

`health-collector` reads your Garmin Connect data through an unofficial API.
Garmin's terms forbid unauthorized third-party access and Garmin blocks
clients, so logins and pulls can stop working without notice. Older images
reject the `[health_collector]` section and cannot decode health batches, so
update in stages:

1. Run `git pull`, then `docker compose pull && docker compose up -d`. This
   adds the `garmin-tokens` volume. `health-collector` restarts until it is
   configured.
2. Add `[health_collector]` from `config.example.toml` to `config.toml`.
3. Stop the collector so nothing else touches the tokens, then log in once:

   ```sh
   docker compose stop health-collector
   docker compose run --rm health-collector login
   docker compose up -d --force-recreate health-collector
   ```

   The login asks for your Garmin email, password and, if enabled, the MFA
   code. Run it in a terminal, once. Never script it or repeat it after a
   failure: repeated logins can lock the account for 48 hours. If Garmin
   answers 403, 427 or 429, wait before trying again.
4. When Garmin refuses the session, or there is no token file, the
   collector stays up and `alerter` sends one ntfy notification, "Garmin login
   expired", with the three commands of step 3. The refresh token has expired
   or was revoked: repeat step 3. The alerter repeats the notification every
   12 hours while the login is broken and sends "Garmin login works again"
   after the next successful pull. Throttling and network or Garmin outages do
   not send it ([ADR-0024](docs/adr/0024-garmin-login-alert-and-gap-fill.md)).
5. After a completed pull the collector first fetches the days of the last 30
   that no daily pull fetched, so a broken login leaves no hole. It uses the
   same pace as the import below, and `garmin_gap.json` on the token volume
   records what is left. Deleting that file re-fetches the last 30 days. A
   corrupt file is logged and the gap fill is skipped until you remove it.
6. After the gap fill the collector imports the past year once, over about
   7 hours at about 2 requests per minute
   ([ADR-0023](docs/adr/0023-health-backfill.md)). Progress is in
   `garmin_backfill.json` on the token volume. Deleting that file restarts
   the import; days already stored are not duplicated. A corrupt file is
   logged and the import is skipped until you remove it.
7. After the import the collector keeps Garmin's original data in the
   `garmin-archive` volume (`/archive/garmin` in the container): every
   workout with its original file, and the documents of each completed day of
   the past year, with a random 3 to 6 s gap between requests. The history
   takes 2 to 3 days of windows, then about 72 requests a day
   ([ADR-0025](docs/adr/0025-garmin-raw-archive.md)). Progress is in
   `garmin_archive.json` (days) and `garmin_activities.json` (activities) on
   the token volume. Deleting one re-runs that import; files already stored
   are skipped. A `.denied` file marks a document Garmin refused with a 403;
   delete it to fetch the document again. A corrupt cursor file is logged and
   that import is skipped until you remove it. Without the volume the archive
   stops and logs; nothing is written to the container.

To add the archive to an existing install, run `git pull` first (it adds the
volume), then
`docker compose pull health-collector && docker compose up -d health-collector`.
Only `health-collector` changes and it publishes no new metrics. Set
`archive_dir` in `config.toml` only after the new image runs: older images
reject the key.

When you update an install that already collects health data, update the
consumers first: run `docker compose pull`, then
`docker compose up -d store alerter dashboard`, then
`docker compose up -d health-collector`. An older `store` rejects the session
status the collector now publishes. Once the alerter has saved a Garmin login
alert, an older alerter cannot read its state file and starts fresh, so
rolling it back can resend first notifications, and an older dashboard shows
"Alerts unavailable".

### Adding temperature ranges to an existing install

Older images reject the `min_temperature` and `max_temperature` keys, so update
in stages:

1. Run `git pull`, then `docker compose pull && docker compose up -d`. This
   adds the `alerter-data` volume.
2. Add the range keys from `config.example.toml` to the `[[thermometers]]`
   entries in `config.toml`.
3. Run `docker compose up -d --force-recreate` (or restart the services).

To roll back to an older image, remove the range keys first.

## Build locally

```sh
docker compose -f compose.yaml -f compose.build.yaml up -d --build
```
