use axum::body::Body;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("route not found for host: {0}")]
    RouteNotFound(String),
    #[error("dns resolution failed for {host}: {source}")]
    Dns {
        host: String,
        source: anyhow::Error,
        incoming_host: String,
        language: String,
        proxy_error_status: Option<StatusCode>,
    },
    #[error("upstream request failed: {0}")]
    Upstream(anyhow::Error, String, String, Option<StatusCode>),
    #[error("upstream timed out")]
    UpstreamTimeout(String, String, Option<StatusCode>),
    #[error("upstream returned {status}")]
    UpstreamError {
        status: StatusCode,
        domain: String,
        incoming_host: String,
        language: String,
        proxy_error_status: Option<StatusCode>,
    },
}

/// Shared context attached to every gateway error so error-page rendering
/// doesn't need to pass it through every call site.
#[derive(Debug, Clone)]
pub struct ErrorContext {
    pub incoming_host: String,
    pub language: String,
    pub proxy_error_status: Option<StatusCode>,
}

impl AppError {
    /// Create an upstream connection error (unadorned — enrich later).
    pub fn upstream(source: impl Into<anyhow::Error>) -> Self {
        AppError::Upstream(source.into(), String::new(), String::new(), None)
    }

    pub fn enrich(self, ctx: &ErrorContext) -> Self {
        match self {
            AppError::Dns { host, source, .. } => AppError::Dns {
                host,
                source,
                incoming_host: ctx.incoming_host.clone(),
                language: ctx.language.clone(),
                proxy_error_status: ctx.proxy_error_status,
            },
            AppError::Upstream(err, _, _, _) => AppError::Upstream(
                err,
                ctx.incoming_host.clone(),
                ctx.language.clone(),
                ctx.proxy_error_status,
            ),
            AppError::UpstreamTimeout(_, _, _) => AppError::UpstreamTimeout(
                ctx.incoming_host.clone(),
                ctx.language.clone(),
                ctx.proxy_error_status,
            ),
            AppError::UpstreamError { status, domain, .. } => AppError::UpstreamError {
                status,
                domain,
                incoming_host: ctx.incoming_host.clone(),
                language: ctx.language.clone(),
                proxy_error_status: ctx.proxy_error_status,
            },
            other => other,
        }
    }

    fn original_status(&self) -> StatusCode {
        match self {
            AppError::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::RouteNotFound(_) => StatusCode::MISDIRECTED_REQUEST,
            AppError::Dns { .. } => StatusCode::BAD_GATEWAY,
            AppError::Upstream(..) => StatusCode::BAD_GATEWAY,
            AppError::UpstreamTimeout(..) => StatusCode::GATEWAY_TIMEOUT,
            AppError::UpstreamError { status, .. } => *status,
        }
    }

    /// Only custom 5xx responses may use the CDN tunneling status.
    fn response_status(&self, original: StatusCode) -> StatusCode {
        if original.is_server_error() {
            self.proxy_error_status().unwrap_or(original)
        } else {
            original
        }
    }

    fn proxy_error_status(&self) -> Option<StatusCode> {
        match self {
            AppError::Dns {
                proxy_error_status, ..
            } => *proxy_error_status,
            AppError::Upstream(_, _, _, status) => *status,
            AppError::UpstreamTimeout(_, _, status) => *status,
            AppError::UpstreamError {
                proxy_error_status, ..
            } => *proxy_error_status,
            _ => None,
        }
    }

    fn gateway_error_fields(&self) -> Option<GatewayErrorFields> {
        match self {
            AppError::Dns {
                incoming_host,
                language,
                ..
            } => Some(GatewayErrorFields {
                kind: GatewayErrorKind::Dns,
                incoming_host: incoming_host.clone(),
                language: language.clone(),
                upstream_domain: String::new(),
            }),
            AppError::Upstream(_, incoming_host, language, _) => Some(GatewayErrorFields {
                kind: GatewayErrorKind::Connection,
                incoming_host: incoming_host.clone(),
                language: language.clone(),
                upstream_domain: String::new(),
            }),
            AppError::UpstreamTimeout(incoming_host, language, _) => Some(GatewayErrorFields {
                kind: GatewayErrorKind::Timeout,
                incoming_host: incoming_host.clone(),
                language: language.clone(),
                upstream_domain: String::new(),
            }),
            AppError::UpstreamError {
                status,
                domain,
                incoming_host,
                language,
                ..
            } => Some(GatewayErrorFields {
                kind: GatewayErrorKind::UpstreamStatus(*status),
                incoming_host: incoming_host.clone(),
                language: language.clone(),
                upstream_domain: domain.clone(),
            }),
            _ => None,
        }
    }

    /// Returns "zh" if the Accept-Language header contains Chinese, "en" otherwise.
    pub fn detect_language(accept_language: Option<&str>) -> &str {
        match accept_language {
            Some(header) if header.to_ascii_lowercase().contains("zh") => "zh",
            _ => "en",
        }
    }
}

enum GatewayErrorKind {
    Dns,
    Connection,
    Timeout,
    UpstreamStatus(StatusCode),
}

struct GatewayErrorFields {
    kind: GatewayErrorKind,
    incoming_host: String,
    language: String,
    upstream_domain: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let original_status = self.original_status();
        let response_status = self.response_status(original_status);

        if let Some(fields) = self.gateway_error_fields() {
            let is_zh = fields.language == "zh";
            let (error_type, badge_label, badge_class, error_desc, domain) =
                error_page_locale(&fields, is_zh);
            let template = if is_zh {
                ERROR_PAGE_TEMPLATE_ZH
            } else {
                ERROR_PAGE_TEMPLATE_EN
            };
            let html = template
                .replace("{status_code}", original_status.as_str())
                .replace(
                    "{status_reason}",
                    original_status.canonical_reason().unwrap_or("Error"),
                )
                .replace("{error_type}", error_type)
                .replace("{error_desc}", error_desc)
                .replace("{domain}", domain)
                .replace("{badge_label}", badge_label)
                .replace("{badge_class}", badge_class);

            let mut builder = Response::builder()
                .status(response_status)
                .header("content-type", "text/html; charset=utf-8");
            if original_status.is_server_error() {
                builder = builder
                    .header("x-mirrox-error-status", original_status.as_str())
                    .header("cache-control", "no-store")
                    .header("x-content-type-options", "nosniff");
            }
            return match builder.body(Body::from(html)) {
                Ok(response) => response,
                Err(_) => (
                    response_status,
                    original_status.canonical_reason().unwrap_or("proxy error"),
                )
                    .into_response(),
            };
        }

        (
            response_status,
            original_status.canonical_reason().unwrap_or("proxy error"),
        )
            .into_response()
    }
}

fn error_page_locale(fields: &GatewayErrorFields, is_zh: bool) -> (&str, &str, &str, &str, &str) {
    match &fields.kind {
        GatewayErrorKind::Dns => {
            if is_zh {
                (
                    "DNS 解析失败",
                    "代理错误",
                    "proxy",
                    "无法解析上游域名",
                    fields.incoming_host.as_str(),
                )
            } else {
                (
                    "DNS resolution failed",
                    "Proxy Error",
                    "proxy",
                    "Unable to resolve upstream domain",
                    fields.incoming_host.as_str(),
                )
            }
        }
        GatewayErrorKind::Connection => {
            if is_zh {
                (
                    "连接失败",
                    "代理错误",
                    "proxy",
                    "无法连接到上游服务器",
                    fields.incoming_host.as_str(),
                )
            } else {
                (
                    "Connection failed",
                    "Proxy Error",
                    "proxy",
                    "Unable to connect to upstream server",
                    fields.incoming_host.as_str(),
                )
            }
        }
        GatewayErrorKind::Timeout => {
            if is_zh {
                (
                    "连接超时",
                    "代理错误",
                    "proxy",
                    "上游服务器未在预期时间内响应",
                    fields.incoming_host.as_str(),
                )
            } else {
                (
                    "Connection timed out",
                    "Proxy Error",
                    "proxy",
                    "Upstream server did not respond in time",
                    fields.incoming_host.as_str(),
                )
            }
        }
        GatewayErrorKind::UpstreamStatus(status) => {
            if is_zh {
                let reason = upstream_error_reason_zh(*status);
                (
                    "上游错误",
                    "上游错误",
                    "upstream",
                    reason,
                    fields.upstream_domain.as_str(),
                )
            } else {
                let reason = upstream_error_reason_en(*status);
                (
                    "Upstream Error",
                    "Upstream Error",
                    "upstream",
                    reason,
                    fields.upstream_domain.as_str(),
                )
            }
        }
    }
}

fn upstream_error_reason_zh(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "请求格式有误",
        StatusCode::FORBIDDEN => "访问被拒绝",
        StatusCode::NOT_FOUND => "请求的资源在上游不存在",
        StatusCode::INTERNAL_SERVER_ERROR => "上游服务器内部错误",
        StatusCode::BAD_GATEWAY => "上游网关错误",
        StatusCode::SERVICE_UNAVAILABLE => "上游服务暂不可用",
        StatusCode::GATEWAY_TIMEOUT => "上游响应超时",
        _ => "上游返回了错误响应",
    }
}

fn upstream_error_reason_en(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "The request was malformed",
        StatusCode::FORBIDDEN => "Access denied by upstream",
        StatusCode::NOT_FOUND => "The requested resource was not found on the upstream server",
        StatusCode::INTERNAL_SERVER_ERROR => "The upstream server encountered an internal error",
        StatusCode::BAD_GATEWAY => "The upstream gateway returned an invalid response",
        StatusCode::SERVICE_UNAVAILABLE => "The upstream service is temporarily unavailable",
        StatusCode::GATEWAY_TIMEOUT => "The upstream server did not respond in time",
        _ => "The upstream server returned an error response",
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

const ERROR_PAGE_TEMPLATE_ZH: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{status_code} {status_reason} - mirrox</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "SF Pro Display", "Helvetica Neue", sans-serif;
            background: #F7F6F3;
            color: #111111;
            min-height: 100dvh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 32px 20px;
            -webkit-font-smoothing: antialiased;
        }
        .card {
            background: #FFFFFF;
            border: 1px solid #EAEAEA;
            border-radius: 12px;
            padding: 48px 40px;
            max-width: 480px;
            width: 100%;
        }
        .status-code {
            font-family: "SF Mono", "JetBrains Mono", "Fira Code", "Cascadia Code", monospace;
            font-size: 64px;
            font-weight: 700;
            letter-spacing: -0.04em;
            line-height: 1;
            color: #111111;
        }
        .status-reason {
            font-size: 18px;
            font-weight: 500;
            color: #111111;
            margin-top: 4px;
        }
        .divider {
            border: none;
            border-top: 1px solid #EAEAEA;
            margin: 24px 0;
        }
        .badge {
            display: inline-block;
            font-size: 11px;
            font-weight: 600;
            letter-spacing: 0.05em;
            text-transform: uppercase;
            padding: 4px 12px;
            border-radius: 9999px;
        }
        .badge.proxy {
            background: #FDEBEC;
            color: #9F2F2D;
        }
        .badge.upstream {
            background: #E1F3FE;
            color: #1F6C9F;
        }
        .domain {
            font-family: "SF Mono", "JetBrains Mono", "Fira Code", "Cascadia Code", monospace;
            font-size: 14px;
            color: #787774;
            margin-top: 12px;
            word-break: break-all;
        }
        .description {
            font-size: 15px;
            line-height: 1.6;
            color: #787774;
            margin-top: 16px;
        }
        .btn {
            display: inline-block;
            margin-top: 24px;
            padding: 10px 24px;
            font-size: 14px;
            font-weight: 500;
            color: #FFFFFF;
            background: #111111;
            border: none;
            border-radius: 6px;
            cursor: pointer;
            text-decoration: none;
            transition: background 0.15s ease;
            -webkit-appearance: none;
        }
        .btn:hover {
            background: #333333;
        }
        .btn:active {
            transform: scale(0.98);
        }
        .footer {
            margin-top: 32px;
            padding-top: 20px;
            border-top: 1px solid #EAEAEA;
            font-size: 12px;
            color: #787774;
            font-family: "SF Mono", "JetBrains Mono", "Fira Code", "Cascadia Code", monospace;
        }
        @media (max-width: 480px) {
            .card { padding: 32px 24px; }
            .status-code { font-size: 48px; }
        }
        @media (prefers-color-scheme: dark) {
            body { background: #1A1A1A; color: #E8E8E8; }
            .card {
                background: #242424;
                border-color: #333333;
            }
            .status-code { color: #E8E8E8; }
            .status-reason { color: #E8E8E8; }
            .divider { border-color: #333333; }
            .badge.proxy { background: #3D2222; color: #F0A0A0; }
            .badge.upstream { background: #1C2D3A; color: #8BC8F0; }
            .domain { color: #9A9A9A; }
            .description { color: #9A9A9A; }
            .btn { color: #111111; background: #E8E8E8; }
            .btn:hover { background: #D0D0D0; }
            .footer { border-color: #333333; color: #9A9A9A; }
        }
    </style>
</head>
<body>
    <div class="card">
        <div class="status-code">{status_code}</div>
        <div class="status-reason">{status_reason}</div>
        <hr class="divider">
        <span class="badge {badge_class}">{badge_label}</span>
        <div class="domain">{domain}</div>
        <div class="description">{error_type}。{error_desc}。</div>
        <button class="btn" onclick="location.reload()">重试</button>
        <div class="footer">mirrox</div>
    </div>
</body>
</html>"#;

const ERROR_PAGE_TEMPLATE_EN: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{status_code} {status_reason} - mirrox</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "SF Pro Display", "Helvetica Neue", sans-serif;
            background: #F7F6F3;
            color: #111111;
            min-height: 100dvh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 32px 20px;
            -webkit-font-smoothing: antialiased;
        }
        .card {
            background: #FFFFFF;
            border: 1px solid #EAEAEA;
            border-radius: 12px;
            padding: 48px 40px;
            max-width: 480px;
            width: 100%;
        }
        .status-code {
            font-family: "SF Mono", "JetBrains Mono", "Fira Code", "Cascadia Code", monospace;
            font-size: 64px;
            font-weight: 700;
            letter-spacing: -0.04em;
            line-height: 1;
            color: #111111;
        }
        .status-reason {
            font-size: 18px;
            font-weight: 500;
            color: #111111;
            margin-top: 4px;
        }
        .divider {
            border: none;
            border-top: 1px solid #EAEAEA;
            margin: 24px 0;
        }
        .badge {
            display: inline-block;
            font-size: 11px;
            font-weight: 600;
            letter-spacing: 0.05em;
            text-transform: uppercase;
            padding: 4px 12px;
            border-radius: 9999px;
        }
        .badge.proxy {
            background: #FDEBEC;
            color: #9F2F2D;
        }
        .badge.upstream {
            background: #E1F3FE;
            color: #1F6C9F;
        }
        .domain {
            font-family: "SF Mono", "JetBrains Mono", "Fira Code", "Cascadia Code", monospace;
            font-size: 14px;
            color: #787774;
            margin-top: 12px;
            word-break: break-all;
        }
        .description {
            font-size: 15px;
            line-height: 1.6;
            color: #787774;
            margin-top: 16px;
        }
        .btn {
            display: inline-block;
            margin-top: 24px;
            padding: 10px 24px;
            font-size: 14px;
            font-weight: 500;
            color: #FFFFFF;
            background: #111111;
            border: none;
            border-radius: 6px;
            cursor: pointer;
            text-decoration: none;
            transition: background 0.15s ease;
            -webkit-appearance: none;
        }
        .btn:hover {
            background: #333333;
        }
        .btn:active {
            transform: scale(0.98);
        }
        .footer {
            margin-top: 32px;
            padding-top: 20px;
            border-top: 1px solid #EAEAEA;
            font-size: 12px;
            color: #787774;
            font-family: "SF Mono", "JetBrains Mono", "Fira Code", "Cascadia Code", monospace;
        }
        @media (max-width: 480px) {
            .card { padding: 32px 24px; }
            .status-code { font-size: 48px; }
        }
        @media (prefers-color-scheme: dark) {
            body { background: #1A1A1A; color: #E8E8E8; }
            .card {
                background: #242424;
                border-color: #333333;
            }
            .status-code { color: #E8E8E8; }
            .status-reason { color: #E8E8E8; }
            .divider { border-color: #333333; }
            .badge.proxy { background: #3D2222; color: #F0A0A0; }
            .badge.upstream { background: #1C2D3A; color: #8BC8F0; }
            .domain { color: #9A9A9A; }
            .description { color: #9A9A9A; }
            .btn { color: #111111; background: #E8E8E8; }
            .btn:hover { background: #D0D0D0; }
            .footer { border-color: #333333; color: #9A9A9A; }
        }
    </style>
</head>
<body>
    <div class="card">
        <div class="status-code">{status_code}</div>
        <div class="status-reason">{status_reason}</div>
        <hr class="divider">
        <span class="badge {badge_class}">{badge_label}</span>
        <div class="domain">{domain}</div>
        <div class="description">{error_type}: {error_desc}.</div>
        <button class="btn" onclick="location.reload()">Retry</button>
        <div class="footer">mirrox</div>
    </div>
</body>
</html>"#;
