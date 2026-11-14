# syntax=docker/dockerfile:1
FROM rust:1-alpine AS builder
RUN apk add --no-cache musl-dev linux-headers gcc make
WORKDIR /src
COPY . .
ARG BIN
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked --bin "$BIN" \
    && cp "target/release/$BIN" /app

FROM alpine:3
COPY --from=builder /app /usr/local/bin/app
ENTRYPOINT ["/usr/local/bin/app"]
