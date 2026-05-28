# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --locked --bin mirrox

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --uid 10001 --home /nonexistent --shell /usr/sbin/nologin mirrox
COPY --from=builder /app/target/release/mirrox /usr/local/bin/mirrox

USER mirrox
EXPOSE 3000
ENV MIRROX_CONFIG=/etc/mirrox/config.toml
ENTRYPOINT ["/usr/local/bin/mirrox"]
