[简体中文](docs/README_zhcn.md)

# Mirrox

Mirrox is a high-performance Rust reverse proxy for publishing controlled mirror domains. It maps incoming hosts you own to upstream hosts declared in a TOML configuration file, rewrites HTTP-layer values and supported response bodies so links stay on the mirror domain, and rejects unknown hosts instead of acting as an open proxy.

Mirrox is designed for self-hosted mirror gateways, private domain fronting, and deployments that need explicit host-to-upstream routing without recompiling the proxy when domain rules change.

## Highlights

- **Config-file-first routing**: define exact host mappings and wildcard suffix mappings in `config.toml`.
- **Strict host allowlist**: requests for unconfigured `Host` values return `421 Misdirected Request`.
- **HTTP rewrite support**: rewrites upstream `Host`, request `Origin` / `Referer`, response `Location`, and cookie domains.
- **Optional body rewriting**: rewrites supported HTML, CSS, JavaScript, and JSON bodies below the configured buffer limit.
- **Streaming-aware behavior**: passes through SSE, oversized responses, and non-text assets without unnecessary buffering.
- **WebSocket support**: proxies upgraded WebSocket connections.
- **Outbound proxy support**: connect to upstreams directly or through HTTP CONNECT / SOCKS5 proxies.
- **Deployment friendly**: supports CLI config selection, environment overrides, Docker, and GitHub Release binaries.

## Current status

Mirrox is usable but still early. The DNS configuration model accepts `udp`, `tcp`, `dot`, and `doh`, and the codebase includes a resolver abstraction. The current resolver implementation still uses Tokio system DNS internally, so do not rely on custom DoH/DoT server enforcement until that wiring is completed.

## Quick start

Create a config file from the example and run the proxy:

```bash
cp examples/config.example.toml config.toml
cargo run --release -- -c config.toml
```

The default server listens on `127.0.0.1:3000`.

Use an explicit config path:

```bash
mirrox -c /etc/mirrox/config.toml
mirrox --config /etc/mirrox/config.toml
```

Config path priority:

1. `-c, --config <PATH>`
2. `MIRROX_CONFIG`
3. `config.toml`

## Docker

After a release image is published, pull it from GHCR:

```bash
docker pull ghcr.io/mirrox-dev/mirrox:latest
```

Run with Docker Compose:

```bash
docker compose up -d
```

The included `docker-compose.yml` uses `ghcr.io/mirrox-dev/mirrox:latest` and mounts `./examples/config.example.toml` to `/etc/mirrox/config.toml`. For real deployments, replace that mount source with your own config file.

Run directly with Docker:

```bash
docker run --rm \
  -p 3000:3000 \
  -e MIRROX_CONFIG=/etc/mirrox/config.toml \
  -e MIRROX_LISTEN=0.0.0.0:3000 \
  -v "$PWD/config.toml:/etc/mirrox/config.toml:ro" \
  ghcr.io/mirrox-dev/mirrox:latest
```

## Configuration example

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
# Upstreams default to HTTPS on port 443. Override when needed:
# upstream_scheme = "http"
# upstream_port = 80
# user_agent = "Mirrox/0.1"

[[routes]]
incoming = "www.example.com"
upstream = "www.bgm.tv"
body_rewrite = "http-only"
upstream_proxy = "http://user:pass@127.0.0.1:8080"
user_agent = "Mozilla/5.0 (compatible; Mirrox)"

[[wildcard_routes]]
incoming_suffix = ".example.com"
upstream_suffix = ".bgm.tv"
upstream_scheme = "https"
upstream_port = 443
```

See [docs/configuration.md](docs/configuration.md) for the full configuration reference.

## Upstream connection settings

Routes connect to upstreams with HTTPS on port `443` by default. Both exact `[[routes]]` and `[[wildcard_routes]]` can set `upstream_scheme = "http"` or `"https"`, `upstream_port`, and `user_agent`.

If `user_agent` is omitted, Mirrox preserves the client's `User-Agent` header on the upstream request. If configured, Mirrox replaces the upstream request `User-Agent` with that value.

Wildcard routes use a single-label multi-domain model: `incoming_suffix = ".example.com"` with `upstream_suffix = ".bgm.tv"` maps `api.example.com` to `api.bgm.tv`, but does not match deeper names such as `v1.api.example.com`.

## Cloudflare Tunnel

When Mirrox runs behind Cloudflare Tunnel, Cloudflare can route multiple public hostnames or a wildcard hostname to the same internal service, for example `http://mirrox:3000`. Cloudflare terminates public HTTPS, while Mirrox receives HTTP behind the proxy and chooses the upstream route from the incoming `Host` header.

## Rewrite model

Mirrox has two rewrite layers:

- **HTTP layer**: request `Host`, `Origin`, `Referer`; response `Location`; cookie `Domain` attributes.
- **Body layer**: supported text responses such as HTML, CSS, JavaScript, and JSON under `max_buffer_bytes`.

Body rewriting defaults to `enabled`. Set `body_rewrite = "http-only"` on a route, or set `MIRROX_REWRITE_BODY=http-only`, to disable body rewriting while keeping HTTP-layer rewriting.

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

The release workflow also accepts unprefixed semver tags such as `0.1.0`.

Published images use these tags:

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

Mirrox is licensed under the [MIT License](LICENSE).
