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
connect_timeout_ms = 10000
request_timeout_ms = 60000
# proxy_error_status = 200
```

- `listen`：当前 HTTP 服务监听的地址。
- `mode`：可选 `behind-proxy` 或 `direct`。当前实现会绑定 `listen`；`http_listen`、`https_listen`、`tls_cert`、`tls_key` 为直接绑定公网端口的部署模式预留。
- `connect_timeout_ms` 与 `request_timeout_ms`：分别限制上游连接建立和完整上游请求时间。
- `proxy_error_status`：仅覆盖 Mirrox 自定义 5xx 页面对外发送的状态。当前置 CDN 会替换 5xx 正文时可设置为 `200`；页面和 `X-Mirrox-Error-Status` 仍保留原始错误。1xx、204、205、304 和超出范围的值会被拒绝。

## DNS 设置

```toml
[dns]
mode = "doh"
servers = ["https://cloudflare-dns.com/dns-query"]
cache_min_ttl_seconds = 30
cache_max_ttl_seconds = 300
timeout_ms = 2000
```

`mode` 支持 `udp`、`tcp`、`dot` 和 `doh`。Mirrox 会按所选协议、服务器、超时和正负缓存 TTL 范围构建 Hickory resolver。URL scheme 必须与模式匹配（DoH 使用 `https://`，DoT 使用 `tls://`），DoH endpoint 使用 `/dns-query`。配置的 DNS 服务器 hostname 会通过宿主机 resolver 做引导解析。

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
upstream_scheme = "https"
upstream_port = 443
# 省略 user_agent 时会保留客户端的 User-Agent。
# user_agent = "Mirrox/0.1"
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

重写目标选择完全确定：当前路由对自身 upstream 优先；其他共享 upstream 的映射选择标准化后字典序最小的 incoming；更长的 upstream 域名优先于短后缀规则。配置 `bgm.tv -> bangumi.lol` 根域映射后，它会为 `foo.bgm.tv` 提供托底；域名标签边界保证 `fxbgm.tv`、`evil-bgm.tv` 和 `bgm.tv.cn` 不会被修改。替换器只扫描原始输入一次，替换结果不会再次触发其他规则。
`[[wildcard_routes]]` 后缀规则会贡献同样的根域托底（`bgm.tv` → `example.com`），不必把每个兄弟子域都写成精确路由。

## 脚本注入

```toml
[scripts]
dir = "./scripts"
prefix = "/mirrox-scripts"
global = ["analytics.js", "theme.js"]
```

- `dir`：本地 JS 文件目录。Mirrox 从该目录读取文件，并在 `/{prefix}/{filename}` 路径下提供服务。
- `prefix`：内置静态文件服务的 URL 路径前缀。默认值：`/mirrox-scripts`。
- `global`：注入到**所有**路由的脚本列表。每项可以是本地文件名（从 `dir` 目录提供）或远程 URL（以 `http://` 或 `https://` 开头）。

路由级脚本在全局脚本之外叠加注入：

```toml
[[routes]]
incoming = "a.example.com"
upstream = "a.site.com"
scripts = ["custom-a.js", "https://cdn.example.com/ext.js"]
```

全局脚本先注入，路由级脚本后注入。所有 `<script>` 标签插入到 HTML 响应的 `</head>` 之前。远程 URL 直接使用；本地文件名根据配置的 `prefix` 解析。

`[[routes]]` 和 `[[wildcard_routes]]` 都支持 `scripts` 字段。

## 精确路由

```toml
[[routes]]
incoming = "api.example.com"
upstream = "api.bgm.tv"
# 默认值：upstream_scheme = "https"，upstream_port = 443

[[routes]]
incoming = "www.example.com"
upstream = "www.bgm.tv"
upstream_scheme = "http"
upstream_port = 8080
user_agent = "Mozilla/5.0 (compatible; Mirrox)"
body_rewrite = "http-only"
# scripts = ["custom.js"]  # 路由级脚本（在全局脚本之外叠加）。
```

精确路由将一个入口域名映射到一个上游域名。上游连接默认使用 `upstream_scheme = "https"` 和 `upstream_port = 443`。路由级 `upstream_scheme` 可设置为 `"http"` 或 `"https"`；`upstream_port` 覆盖上游 TCP 连接使用的端口；`body_rewrite` 可以覆盖 `[rewrite].body`；`scripts` 在全局脚本之外叠加路由级脚本注入。

在路由上设置 `user_agent` 可替换发往上游请求的 `User-Agent` 头。省略 `user_agent` 时会保留客户端原始 `User-Agent`。

## 通配后缀路由

```toml
[[wildcard_routes]]
incoming_suffix = ".example.com"
upstream_suffix = ".bgm.tv"
upstream_scheme = "https"
upstream_port = 443
# user_agent = "Mirrox/0.1"
```

通配后缀路由会把单级子域名前缀映射到另一个后缀。例如 `incoming_suffix = ".example.com"` 搭配 `upstream_suffix = ".bgm.tv"` 会把 `api.example.com` 映射到 `api.bgm.tv`。它只匹配后缀前的一个标签，因此 `v1.api.example.com` 不会被这条通配路由匹配。精确路由优先级高于通配后缀路由。
同一条规则也会匹配 apex 主机（`example.com` → `bgm.tv`），并作为未列出兄弟域名的重写托底（`lain.bgm.tv` → `lain.example.com`）。

通配路由支持与精确路由相同的上游连接字段：`upstream_scheme = "http"` 或 `"https"`、`upstream_port`、`user_agent` 和 `scripts`。省略时，通配路由的上游同样默认使用 HTTPS 443，并保留客户端的 `User-Agent`。

## Cloudflare Tunnel 部署

在 `behind-proxy` 模式下，Mirrox 适合部署在另一个 HTTPS 终止层后方。使用 Cloudflare Tunnel 时，可以把多个公网 hostname 或一个通配 hostname 指向同一个内部服务，例如 `http://mirrox:3000`。Cloudflare 负责公网 HTTPS 证书和终止，并在私有网络内用 HTTP 转发给 Mirrox。Mirrox 随后读取转发过来的 `Host` 头，匹配精确或通配路由，并连接到配置的上游。

Cloudflare 会替换 Mirrox 生成的 5xx 响应时，可设置 `proxy_error_status = 200`。自包含错误页和 `X-Mirrox-Error-Status` 仍显示原始状态，4xx 不会被覆盖。若 Cloudflare 无法连接 Tunnel 或 Mirrox 进程，请求没有到达程序，Mirrox 无法渲染错误页。

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
