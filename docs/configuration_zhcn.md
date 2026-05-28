# 配置说明

Mirrox 优先使用 TOML 配置文件，并提供少量环境变量用于部署时覆盖。默认情况下，程序会从当前工作目录读取 `config.toml`。

## 从示例开始

```bash
cp examples/config.example.toml config.toml
cargo run --release -- --help
```

使用指定配置文件启动：

```bash
MIRROX_CONFIG=/etc/mirrox/config.toml mirrox
```

## Server 设置

```toml
[server]
listen = "127.0.0.1:3000"
mode = "behind-proxy"
http_listen = "0.0.0.0:80"
https_listen = "0.0.0.0:443"
tls_cert = "/path/to/fullchain.pem"
tls_key = "/path/to/privkey.pem"
```

- `listen`：当前 HTTP 服务监听的地址。
- `mode`：可选 `behind-proxy` 或 `direct`。当前实现会绑定 `listen`；`http_listen`、`https_listen`、`tls_cert`、`tls_key` 为直接绑定公网端口的部署模式预留。

## DNS 设置

```toml
[dns]
mode = "doh"
servers = ["https://cloudflare-dns.com/dns-query"]
cache_min_ttl_seconds = 30
cache_max_ttl_seconds = 300
timeout_ms = 2000
```

`mode` 支持的配置值包括 `udp`、`tcp`、`dot` 和 `doh`。当前代码已经有 resolver 抽象，但实际解析仍通过 Tokio 系统 DNS；如果要在生产中严格使用自定义 DoH/DoT 服务器，还需要补齐并强化这一部分实现。

## 上游代理设置

```toml
[upstream_proxy]
default = "socks5://user:pass@127.0.0.1:1080"
```

- 省略 `[upstream_proxy]` 或使用 `default = "direct"` 表示直连上游。
- `http://host:port` 和 `http://user:pass@host:port` 使用 HTTP CONNECT。
- `socks5://host:port` 和 `socks5://user:pass@host:port` 使用 SOCKS5 CONNECT。
- 路由级 `upstream_proxy` 会覆盖全局默认代理。

```toml
[[routes]]
incoming = "api.example.com"
upstream = "api.bgm.tv"
upstream_proxy = "direct"
```

## Rewrite 设置

```toml
[rewrite]
body = "enabled"
max_buffer_bytes = 2097152
```

- `body = "enabled"`：重写 HTTP 头以及可重写的文本响应体。
- `body = "http-only"`：只重写 HTTP 层数据，例如 `Host`、`Origin`、`Referer`、`Location` 和 Cookie Domain。
- `max_buffer_bytes`：响应体重写时允许缓冲的最大文本响应大小。超过该大小的响应会直接流式透传，不做正文重写。

SSE（`text/event-stream`）和非文本资源会直接流式透传，不做正文重写。

## 精确路由

```toml
[[routes]]
incoming = "api.example.com"
upstream = "api.bgm.tv"

[[routes]]
incoming = "www.example.com"
upstream = "www.bgm.tv"
body_rewrite = "http-only"
```

精确路由将一个入口域名映射到一个上游域名。每条路由的 `body_rewrite` 可以覆盖 `[rewrite].body`。

## 通配后缀路由

```toml
[[wildcard_routes]]
incoming_suffix = ".mirror.example.com"
upstream_suffix = ".bgm.tv"
```

通配后缀路由会把单级子域名前缀映射到另一个后缀。例如 `api.mirror.example.com` 会映射到 `api.bgm.tv`。精确路由优先级高于通配后缀路由。

## 环境变量覆盖

| 变量 | 含义 |
| --- | --- |
| `MIRROX_CONFIG` | TOML 配置文件路径。 |
| `MIRROX_LISTEN` | 覆盖 `[server].listen`。 |
| `MIRROX_DNS_SERVERS` | 逗号分隔的 DNS 服务器列表。 |
| `MIRROX_UPSTREAM_PROXY` | 覆盖 `[upstream_proxy].default`；使用 `direct` 或空值表示直连。 |
| `MIRROX_REWRITE_BODY` | 覆盖 `[rewrite].body`，可选 `enabled` 或 `http-only`。 |

## 安全模型

代理只接受已经配置的入口域名。未知 `Host` 的请求会返回 `421 Misdirected Request`，从而避免服务变成开放代理。
