# Mirrox

Mirrox 是一个使用 Rust 编写的可配置反向代理，用于在不把域名规则硬编码进程序的情况下发布镜像域名。它会把你控制的入口域名映射到 `config.toml` 中配置的上游域名，并重写 HTTP 头和受支持的响应正文，让链接保持在镜像域名下；未配置的 Host 会返回 `421 Misdirected Request`。

仓库：<https://github.com/mirrox-dev/mirrox>

## 功能

- 精确域名映射和通配后缀映射。
- 严格的入口域名白名单；未知 Host 返回 `421 Misdirected Request`。
- HTTP 转发，并替换上游请求的 `Host`。
- 重写请求头中的 `Origin` 和 `Referer`。
- 重写响应中的 `Location`、`Set-Cookie Domain` 和文本响应体。
- 支持按路由切换为仅 HTTP 层重写。
- SSE 和超大响应透传，避免不必要的缓冲。
- 支持 WebSocket 升级连接透传。
- 出站连接可选使用 HTTP CONNECT 或 SOCKS5 上游代理。
- 优先使用配置文件，并支持 CLI 参数和环境变量覆盖。
- GHCR 发布 `linux/amd64` 和 `linux/arm64` Docker 镜像。

## 当前状态

项目已经具备基础功能，但仍处于早期阶段。DNS 配置模型支持 `udp`、`tcp`、`dot` 和 `doh`，代码中也已经有 resolver 抽象；不过当前 resolver 实现仍通过 Tokio 系统 DNS 解析。自定义 DoH/DoT 服务器的强制使用需要后续补齐后，才建议在生产中依赖。

## 快速开始

```bash
cp examples/config.example.toml config.toml
cargo run --release -- -c config.toml
```

默认服务监听 `127.0.0.1:3000`。

使用指定配置文件路径：

```bash
mirrox -c /etc/mirrox/config.toml
mirrox --config /etc/mirrox/config.toml
```

配置路径优先级：

1. `-c, --config <PATH>`
2. `MIRROX_CONFIG`
3. `config.toml`

环境变量仍然保留，便于 Docker 部署和简单覆盖。

## Docker

拉取公开镜像：

```bash
docker pull ghcr.io/mirrox-dev/mirrox:latest
```

使用 Docker Compose 运行：

```bash
docker compose up -d
```

仓库内的 `docker-compose.yml` 使用 `ghcr.io/mirrox-dev/mirrox:latest`，并把 `./examples/config.example.toml` 挂载到 `/etc/mirrox/config.toml`。实际部署时请把挂载源替换为你自己的配置文件。

直接使用 Docker 运行：

```bash
docker run --rm \
  -p 3000:3000 \
  -e MIRROX_CONFIG=/etc/mirrox/config.toml \
  -e MIRROX_LISTEN=0.0.0.0:3000 \
  -v "$PWD/config.toml:/etc/mirrox/config.toml:ro" \
  ghcr.io/mirrox-dev/mirrox:latest
```

## 配置示例

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

完整配置说明见 [configuration_zhcn.md](configuration_zhcn.md)。

## 重写模式

默认情况下，代理会同时重写 HTTP 层数据和支持的文本响应体：

- HTTP 层：`Host`、`Origin`、`Referer`、`Location` 和 Cookie Domain。
- 响应体层：小于 `max_buffer_bytes` 的 HTML、CSS、JavaScript 和 JSON 响应。

`MIRROX_REWRITE_BODY` 默认是 `enabled`。可以在路由上设置 `body_rewrite = "http-only"`，或设置 `MIRROX_REWRITE_BODY=http-only`，以关闭响应体重写但保留 HTTP 层重写。

## 环境变量

| 变量 | 含义 |
| --- | --- |
| `MIRROX_CONFIG` | TOML 配置文件路径；未提供 `-c/--config` 时使用。 |
| `MIRROX_LISTEN` | 覆盖 `[server].listen`。 |
| `MIRROX_DNS_SERVERS` | 逗号分隔的 DNS 服务器列表。 |
| `MIRROX_UPSTREAM_PROXY` | 覆盖上游代理模式；可使用 `direct`、`http://...` 或 `socks5://...`。 |
| `MIRROX_REWRITE_BODY` | 覆盖正文重写模式；默认 `enabled`，设置为 `http-only` 可关闭正文重写。 |

## 发布

推送版本 tag 会创建 GitHub Release、上传原生二进制，并发布 Linux 多架构 GHCR 镜像：

```bash
git tag -a v0.1.0 -m "Release v0.1.0"
git push origin v0.1.0
```

镜像发布地址：

```bash
docker pull ghcr.io/mirrox-dev/mirrox:latest
docker pull ghcr.io/mirrox-dev/mirrox:v0.1.0
docker pull ghcr.io/mirrox-dev/mirrox:0.1.0
docker pull ghcr.io/mirrox-dev/mirrox:0.1
```

## 开发

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo check
```

## 许可证

MIT
