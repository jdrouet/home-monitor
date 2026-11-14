# home-monitor

Reads Bluetooth temperature and humidity sensors and shows the climate of each
room on a dashboard, and sends a phone notification when a sensor stops
reporting. It runs as six containers: Mosquitto, ntfy, `ble-collector`, `store`,
`dashboard` and `alerter`. The host needs BlueZ and D-Bus, because
`ble-collector` talks to the Bluetooth adapter through them.

## Images

CI publishes `ghcr.io/jdrouet/home-monitor-ble-collector`,
`-store`, `-dashboard` and `-alerter` for `linux/amd64` and `linux/arm64`. Tags are
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

Edit `config.toml` to list your thermometers. Keep `[store] data_dir = "/data"`.
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

## Update

```sh
docker compose pull && docker compose up -d
```

Run `git pull` first if `compose.yaml` changed. If `config.example.toml` gained
new sections, copy them into your `config.toml`.

## Build locally

```sh
docker compose -f compose.yaml -f compose.build.yaml up -d --build
```
