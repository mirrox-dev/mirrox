# Configuration

Mirrox is configured from a TOML file first, with a small set of environment variables for deployment-time overrides. By default the binary reads `config.toml` from the current working directory.

## Start with the example

```bash
cp examples/config.example.toml config.toml
cargo run --release -- --help
```

Run the proxy with a specific config file:

```bash
MIRROX_CONFIG=/etc/mirrox/config.toml mirrox
```

## Server settings

```toml
[server]
listen = "127.0.0.1:3000"
mode = "behind-proxy"
http_listen = "0.0.0.0:80"
https_listen = "0.0.0.0:443"
tls_cert = "/path/to/fullchain.pem"
tls_key = "/path/to/privkey.pem"
```

- `listen`: address used by the current HTTP server.
- `mode`: `behind-proxy` or `direct`. The current implementation binds `listen`; `http_listen`, `https_listen`, `tls_cert`, and `tls_key` are reserved for direct public-port deployments.

## DNS settings

```toml
[dns]
mode = "doh"
servers = ["https://cloudflare-dns.com/dns-query"]
cache_min_ttl_seconds = 30
cache_max_ttl_seconds = 300
timeout_ms = 2000
```

Supported config values for `mode` are `udp`, `tcp`, `dot`, and `doh`. The resolver abstraction is present, but this implementation currently resolves through Tokio system DNS; custom DoH/DoT server enforcement still needs hardening before relying on it in production.

## Upstream proxy settings

```toml
[upstream_proxy]
default = "socks5://user:pass@127.0.0.1:1080"
```

- Omit `[upstream_proxy]` or use `default = "direct"` for direct upstream connections.
- `http://host:port` and `http://user:pass@host:port` use HTTP CONNECT.
- `socks5://host:port` and `socks5://user:pass@host:port` use SOCKS5 CONNECT.
- Route-level `upstream_proxy` overrides the global default.

```toml
[[routes]]
incoming = "api.example.com"
upstream = "api.bgm.tv"
upstream_proxy = "direct"
```

## Rewrite settings

```toml
[rewrite]
body = "enabled"
max_buffer_bytes = 2097152
```

- `body = "enabled"`: rewrite HTTP headers and rewritable text bodies.
- `body = "http-only"`: rewrite only HTTP-layer data such as `Host`, `Origin`, `Referer`, `Location`, and cookie domains.
- `max_buffer_bytes`: maximum text response size buffered for body rewriting. Larger responses are streamed through unchanged.

SSE (`text/event-stream`) and non-text assets are streamed through without body rewriting.

## Exact routes

```toml
[[routes]]
incoming = "api.example.com"
upstream = "api.bgm.tv"

[[routes]]
incoming = "www.example.com"
upstream = "www.bgm.tv"
body_rewrite = "http-only"
```

Exact routes map one incoming host to one upstream host. Per-route `body_rewrite` overrides `[rewrite].body`.

## Wildcard routes

```toml
[[wildcard_routes]]
incoming_suffix = ".mirror.example.com"
upstream_suffix = ".bgm.tv"
```

Wildcard routes map a single-label subdomain prefix to another suffix. For example, `api.mirror.example.com` maps to `api.bgm.tv`. Exact routes take priority over wildcard routes.

## Environment overrides

| Variable | Meaning |
| --- | --- |
| `MIRROX_CONFIG` | Path to the TOML config file. |
| `MIRROX_LISTEN` | Overrides `[server].listen`. |
| `MIRROX_DNS_SERVERS` | Comma-separated DNS server list. |
| `MIRROX_UPSTREAM_PROXY` | Overrides `[upstream_proxy].default`; use `direct` or an empty value for direct mode. |
| `MIRROX_REWRITE_BODY` | Overrides `[rewrite].body` with `enabled` or `http-only`. |

## Security model

The proxy only accepts configured incoming hosts. Requests for unknown `Host` values return `421 Misdirected Request`, which prevents the service from becoming an open proxy.
