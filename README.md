# Mirrox

Mirrox is a configurable Rust reverse proxy for publishing mirror domains without hardcoding domain rules into the binary. It maps incoming hosts you control to upstream hosts defined in `config.toml`, rewrites HTTP headers and supported response content so links stay on the mirror domain, and rejects unknown hosts with `421 Misdirected Request`.

Repository: <https://github.com/mirrox-dev/mirrox>

## What it does

Mirrox is designed for self-hosted domain mirroring and controlled reverse-proxy deployments where each public domain maps to a known upstream domain. The configuration model supports exact host mappings and wildcard suffix mappings, so one proxy instance can serve several mirrored hosts while keeping routing explicit.

Typical use cases include:

- exposing a service through your own domain while preserving upstream host routing;
- mirroring multiple upstream subdomains with wildcard suffix rules;
- keeping request and response URLs consistent with the public mirror domain;
- routing outbound traffic directly or through an HTTP CONNECT / SOCKS5 proxy.

## Features

- Exact and wildcard host mapping.
- Strict configured-host allowlist; unknown hosts return `421 Misdirected Request`.
- HTTP forwarding with upstream `Host` replacement.
- Request header rewriting for `Origin` and `Referer`.
- Response rewriting for `Location`, `Set-Cookie Domain`, and supported text bodies.
- Per-route switch for HTTP-layer-only rewriting.
- SSE and oversized response passthrough to avoid unnecessary buffering.
- WebSocket passthrough for upgraded connections.
- Optional HTTP CONNECT or SOCKS5 upstream proxy for outbound connections.
- Config-file-first setup with CLI and environment variable overrides.
- Docker images published to GHCR for `linux/amd64` and `linux/arm64`.

## Current status

Mirrox is functional but still early. The DNS configuration model supports `udp`, `tcp`, `dot`, and `doh`, and a resolver abstraction is in place; however, the current resolver implementation still uses Tokio system DNS internally. Do not rely on custom DoH/DoT server enforcement until that resolver wiring is completed.

## Quick start

```bash
cp examples/config.example.toml config.toml
cargo run --release -- -c config.toml
```

The default server listens on `127.0.0.1:3000`.

Use a specific config path:

```bash
mirrox -c /etc/mirrox/config.toml
mirrox --config /etc/mirrox/config.toml
```

Config path priority is:

1. `-c, --config <PATH>`
2. `MIRROX_CONFIG`
3. `config.toml`

Environment variables remain supported for container deployments and simple overrides.

## Docker

Pull the public image:

```bash
docker pull ghcr.io/mirrox-dev/mirrox:latest
```

Run with Docker Compose:

```bash
docker compose up -d
```

The included `docker-compose.yml` uses `ghcr.io/mirrox-dev/mirrox:latest` and mounts `./examples/config.example.toml` to `/etc/mirrox/config.toml`. For a real deployment, replace that mount source with your own config file.

Run directly with Docker:

```bash
docker run --rm \
  -p 3000:3000 \
  -e MIRROX_CONFIG=/etc/mirrox/config.toml \
  -e MIRROX_LISTEN=0.0.0.0:3000 \
  -v "$PWD/config.toml:/etc/mirrox/config.toml:ro" \
  ghcr.io/mirrox-dev/mirrox:latest
```

## Example config

```toml
[server]
listen = "127.0.0.1:3000"
mode = "behind-proxy"

[dns]
mode = "doh"
servers = ["https://cloudflare-dns.com/dns-query"]

[upstream_proxy]
default = "direct"

[rewrite]
body = "enabled"
max_buffer_bytes = 2097152

[[routes]]
incoming = "api.example.com"
upstream = "api.bgm.tv"

[[routes]]
incoming = "www.example.com"
upstream = "www.bgm.tv"
body_rewrite = "http-only"
upstream_proxy = "http://user:pass@127.0.0.1:8080"

[[wildcard_routes]]
incoming_suffix = ".mirror.example.com"
upstream_suffix = ".bgm.tv"
```

See [docs/configuration.md](docs/configuration.md) for the full configuration reference. The Simplified Chinese README is available at [docs/README_zhcn.md](docs/README_zhcn.md).

## Rewrite modes

By default, Mirrox rewrites both HTTP-layer values and supported text bodies:

- HTTP layer: `Host`, `Origin`, `Referer`, `Location`, and cookie domains.
- Body layer: HTML, CSS, JavaScript, and JSON responses under `max_buffer_bytes`.

`MIRROX_REWRITE_BODY` defaults to `enabled`. Set `body_rewrite = "http-only"` on a route, or set `MIRROX_REWRITE_BODY=http-only`, to disable body rewriting while keeping HTTP-layer rewriting.

## Environment variables

| Variable | Meaning |
| --- | --- |
| `MIRROX_CONFIG` | Path to the TOML config file. Used when `-c/--config` is not provided. |
| `MIRROX_LISTEN` | Overrides `[server].listen`. |
| `MIRROX_DNS_SERVERS` | Comma-separated DNS server list. |
| `MIRROX_UPSTREAM_PROXY` | Overrides upstream proxy mode; use `direct`, `http://...`, or `socks5://...`. |
| `MIRROX_REWRITE_BODY` | Overrides body rewrite mode. Defaults to `enabled`; set `http-only` to disable body rewriting. |

## Releases

Pushing a version tag creates a GitHub Release, uploads native binaries, and publishes a Linux multi-architecture GHCR image:

```bash
git tag -a v0.1.0 -m "Release v0.1.0"
git push origin v0.1.0
```

Images are published to:

```bash
docker pull ghcr.io/mirrox-dev/mirrox:latest
docker pull ghcr.io/mirrox-dev/mirrox:v0.1.0
docker pull ghcr.io/mirrox-dev/mirrox:0.1.0
docker pull ghcr.io/mirrox-dev/mirrox:0.1
```

## Development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo check
```

## License

MIT
