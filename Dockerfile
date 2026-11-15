# syntax=docker/dockerfile:1
FROM rust:1-alpine AS chef
RUN apk add --no-cache musl-dev linux-headers gcc make
# Alpine's gcc emits outline atomics on aarch64, which Rust's musl link lacks
# (vendored libdbus fails with undefined `__aarch64_ldadd4_sync`).
ENV CFLAGS_aarch64_unknown_linux_musl=-mno-outline-atomics
RUN cargo install cargo-chef --locked --version 0.1.78
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked --bin ble-collector --bin store --bin dashboard --bin alerter --bin weather-collector --bin health-collector

FROM alpine:3 AS ble-collector
COPY --from=builder /src/target/release/ble-collector /usr/local/bin/app
ENTRYPOINT ["/usr/local/bin/app"]

FROM alpine:3 AS store
COPY --from=builder /src/target/release/store /usr/local/bin/app
ENTRYPOINT ["/usr/local/bin/app"]

FROM alpine:3 AS dashboard
COPY --from=builder /src/target/release/dashboard /usr/local/bin/app
ENTRYPOINT ["/usr/local/bin/app"]

FROM alpine:3 AS alerter
COPY --from=builder /src/target/release/alerter /usr/local/bin/app
ENTRYPOINT ["/usr/local/bin/app"]

FROM alpine:3 AS weather-collector
COPY --from=builder /src/target/release/weather-collector /usr/local/bin/app
ENTRYPOINT ["/usr/local/bin/app"]

FROM alpine:3 AS health-collector
COPY --from=builder /src/target/release/health-collector /usr/local/bin/app
ENTRYPOINT ["/usr/local/bin/app"]
