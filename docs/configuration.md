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
upstream_scheme = "https"
upstream_port = 443
# Omit user_agent to preserve the client's User-Agent.
# user_agent = "Mirrox/0.1"
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
# Defaults: upstream_scheme = "https", upstream_port = 443

[[routes]]
incoming = "www.example.com"
upstream = "www.bgm.tv"
upstream_scheme = "http"
upstream_port = 8080
user_agent = "Mozilla/5.0 (compatible; Mirrox)"
body_rewrite = "http-only"
```

Exact routes map one incoming host to one upstream host. Upstream connections default to `upstream_scheme = "https"` and `upstream_port = 443`. Per-route `upstream_scheme` accepts `"http"` or `"https"`; `upstream_port` overrides the port used for the upstream TCP connection; `body_rewrite` overrides `[rewrite].body`.

Set `user_agent` on a route to replace the upstream request `User-Agent` header. Omit `user_agent` to preserve the client's original `User-Agent`.

## Wildcard routes

```toml
[[wildcard_routes]]
incoming_suffix = ".moecloud.tk"
upstream_suffix = ".bgm.tv"
upstream_scheme = "https"
upstream_port = 443
# user_agent = "Mirrox/0.1"
```

Wildcard routes map a single-label subdomain prefix to another suffix. For example, `incoming_suffix = ".moecloud.tk"` with `upstream_suffix = ".bgm.tv"` maps `api.moecloud.tk` to `api.bgm.tv`. It only matches one label before the suffix, so `v1.api.moecloud.tk` is not matched by that wildcard route. Exact routes take priority over wildcard routes.

Wildcard routes support the same upstream connection fields as exact routes: `upstream_scheme = "http"` or `"https"`, `upstream_port`, and `user_agent`. If omitted, wildcard upstreams also default to HTTPS on port `443` and preserve the client's `User-Agent`.

## Cloudflare Tunnel deployments

In `behind-proxy` mode, Mirrox is intended to sit behind another HTTPS terminator. With Cloudflare Tunnel, configure multiple public hostnames or a wildcard hostname to point at the same internal service, such as `http://mirrox:3000`. Cloudflare handles public HTTPS certificates and forwards HTTP to Mirrox inside your private network. Mirrox then reads the forwarded `Host` header, matches it against exact or wildcard routes, and connects to the configured upstream.

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
