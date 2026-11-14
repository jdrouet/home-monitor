# home-monitor

Reads Bluetooth temperature and humidity sensors and shows the climate of each
room on a dashboard. It runs as four containers: Mosquitto, `ble-collector`,
`store` and `dashboard`. The host needs BlueZ and D-Bus, because
`ble-collector` talks to the Bluetooth adapter through them.

## Images

CI publishes `ghcr.io/jdrouet/home-monitor-ble-collector`,
`-store` and `-dashboard` for `linux/amd64` and `linux/arm64`. Tags are
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
```

Edit `config.toml` to list your thermometers. Keep `[store] data_dir = "/data"`.
Then start the services:

```sh
docker compose up -d
```

Containers use `restart: unless-stopped`, and Docker starts after BlueZ at boot,
so the services come back after a reboot. After a cold reboot, check
`docker compose ps` and that `docker compose logs ble-collector` shows the first
packets. `bluetooth` pulls in `dbus`, which `ble-collector` needs.

## Update

```sh
docker compose pull && docker compose up -d
```

Run `git pull` first if `compose.yaml` changed.

## Build locally

```sh
docker compose -f compose.yaml -f compose.build.yaml up -d --build
```
