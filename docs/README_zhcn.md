# Mirrox

Mirrox 是一个使用 Rust 编写的高性能可配置反向代理，用于发布受控镜像域名。它会把你控制的入口域名映射到 TOML 配置文件中声明的上游域名，并重写 HTTP 层数据和受支持的响应正文，让链接保持在镜像域名下；未配置的 Host 会被拒绝，避免服务变成开放代理。

Mirrox 适合自托管镜像网关、私有域名前置，以及需要显式 host-to-upstream 路由且不希望每次修改域名规则都重新编译程序的部署场景。

仓库：<https://github.com/mirrox-dev/mirrox>

## 特性

- **配置文件优先路由**：在 `config.toml` 中定义精确域名映射和通配后缀映射。
- **严格 Host 白名单**：未配置 `Host` 的请求返回 `421 Misdirected Request`。
- **HTTP 重写支持**：重写上游 `Host`、请求 `Origin` / `Referer`、响应 `Location` 和 Cookie Domain。
- **可选响应体重写**：重写低于缓冲限制的 HTML、CSS、JavaScript 和 JSON 响应体。
- **面向流式传输的行为**：SSE、超大响应和非文本资源会透传，避免不必要的缓冲。
- **WebSocket 支持**：代理升级后的 WebSocket 连接。
- **出站代理支持**：上游连接可直连，也可通过 HTTP CONNECT / SOCKS5 代理。
- **便于部署**：支持 CLI 配置选择、环境变量覆盖、Docker 和 GitHub Release 二进制文件。

## 当前状态

Mirrox 已经可用，但仍处于早期阶段。DNS 配置模型接受 `udp`、`tcp`、`dot` 和 `doh`，代码中也包含 resolver 抽象。当前 resolver 实现仍通过 Tokio 系统 DNS 解析，因此在相关逻辑完成前，不建议依赖自定义 DoH/DoT 服务器的强制使用。

## 快速开始

从示例创建配置文件并运行代理：

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

## Docker

发布镜像生成后，可以从 GHCR 拉取：

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
# 上游默认使用 HTTPS 和 443 端口；需要时可覆盖：
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

完整配置说明见 [configuration_zhcn.md](configuration_zhcn.md)。

## 上游连接设置

路由默认使用 HTTPS 连接上游的 `443` 端口。精确 `[[routes]]` 和 `[[wildcard_routes]]` 都可以设置 `upstream_scheme = "http"` 或 `"https"`、`upstream_port` 和 `user_agent`。

省略 `user_agent` 时，Mirrox 会保留客户端请求中的 `User-Agent` 并转发给上游；配置该值时，Mirrox 会用配置值覆盖发往上游请求的 `User-Agent`。

通配路由使用单级标签的多域名模型：`incoming_suffix = ".example.com"` 搭配 `upstream_suffix = ".bgm.tv"` 会把 `api.example.com` 映射到 `api.bgm.tv`，但不会匹配 `v1.api.example.com` 这类更深层级域名。

## Cloudflare Tunnel

Mirrox 部署在 Cloudflare Tunnel 后面时，Cloudflare 可以把多个公网 hostname 或一个通配 hostname 路由到同一个内部服务，例如 `http://mirrox:3000`。Cloudflare 负责公网 HTTPS 终止，Mirrox 在代理后方接收 HTTP，并根据传入的 `Host` 选择路由。

## 重写模型

Mirrox 有两层重写：

- **HTTP 层**：请求 `Host`、`Origin`、`Referer`；响应 `Location`；Cookie `Domain` 属性。
- **响应体层**：低于 `max_buffer_bytes` 的 HTML、CSS、JavaScript 和 JSON 等文本响应。

响应体重写默认启用。可以在路由上设置 `body_rewrite = "http-only"`，或设置 `MIRROX_REWRITE_BODY=http-only`，以关闭响应体重写但保留 HTTP 层重写。

## 环境变量

| 变量 | 含义 |
| --- | --- |
| `MIRROX_CONFIG` | TOML 配置文件路径；未提供 `-c/--config` 时使用。 |
| `MIRROX_LISTEN` | 覆盖 `[server].listen`。 |
| `MIRROX_DNS_SERVERS` | 逗号分隔的 DNS 服务器列表。 |
| `MIRROX_UPSTREAM_PROXY` | 覆盖上游代理模式；可使用 `direct`、`http://...` 或 `socks5://...`。 |
| `MIRROX_REWRITE_BODY` | 覆盖响应体重写模式；默认 `enabled`，设置为 `http-only` 可关闭响应体重写。 |

## 发布

推送版本 tag 会创建 GitHub Release、上传原生二进制，并发布 Linux 多架构 GHCR 镜像：

```bash
git tag -a v0.1.0 -m "Release v0.1.0"
git push origin v0.1.0
```

发布 workflow 也接受 `0.1.0` 这样的无 `v` 前缀 semver tag。

镜像发布后使用以下 tag：

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

Mirrox 使用 [MIT License](../LICENSE) 授权。
