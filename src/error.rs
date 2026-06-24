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
    },
    #[error("upstream request failed: {0}")]
    Upstream(anyhow::Error, String),
    #[error("upstream timed out")]
    UpstreamTimeout(String),
    #[error("upstream returned {status}")]
    UpstreamError {
        status: StatusCode,
        domain: String,
        incoming_host: String,
    },
}

impl AppError {
    /// Attach the incoming host to gateway errors so the error page can display it.
    pub fn with_incoming_host(self, incoming_host: &str) -> Self {
        match self {
            AppError::Dns { host, source, .. } => AppError::Dns {
                host,
                source,
                incoming_host: incoming_host.to_string(),
            },
            AppError::Upstream(err, _) => AppError::Upstream(err, incoming_host.to_string()),
            AppError::UpstreamTimeout(_) => AppError::UpstreamTimeout(incoming_host.to_string()),
            AppError::UpstreamError {
                status, domain, ..
            } => AppError::UpstreamError {
                status,
                domain,
                incoming_host: incoming_host.to_string(),
            },
            other => other,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::RouteNotFound(_) => StatusCode::MISDIRECTED_REQUEST,
            AppError::Dns { .. } => StatusCode::BAD_GATEWAY,
            AppError::Upstream(..) => StatusCode::BAD_GATEWAY,
            AppError::UpstreamTimeout(..) => StatusCode::GATEWAY_TIMEOUT,
            AppError::UpstreamError { status, .. } => *status,
        };

        // For gateway errors (502/504) and upstream errors (4xx/5xx), return a
        // custom HTML error page instead of forwarding the upstream response.
        if matches!(
            self,
            AppError::Dns { .. }
                | AppError::Upstream(..)
                | AppError::UpstreamTimeout(..)
                | AppError::UpstreamError { .. }
        ) {
            let (error_type, badge_label, badge_class, error_desc, domain) = match &self {
                AppError::Dns { incoming_host, .. } => (
                    "DNS 解析失败",
                    "代理错误",
                    "proxy",
                    "无法解析上游域名",
                    incoming_host.as_str(),
                ),
                AppError::Upstream(_, incoming_host) => (
                    "连接失败",
                    "代理错误",
                    "proxy",
                    "无法连接到上游服务器",
                    incoming_host.as_str(),
                ),
                AppError::UpstreamTimeout(incoming_host) => (
                    "连接超时",
                    "代理错误",
                    "proxy",
                    "上游服务器未在预期时间内响应",
                    incoming_host.as_str(),
                ),
                AppError::UpstreamError {
                    status,
                    domain,
                    incoming_host: _,
                } => {
                    let reason = match *status {
                        StatusCode::BAD_REQUEST => "请求格式有误",
                        StatusCode::FORBIDDEN => "访问被拒绝",
                        StatusCode::NOT_FOUND => "请求的资源在上游不存在",
                        StatusCode::INTERNAL_SERVER_ERROR => "上游服务器内部错误",
                        StatusCode::BAD_GATEWAY => "上游网关错误",
                        StatusCode::SERVICE_UNAVAILABLE => "上游服务暂不可用",
                        StatusCode::GATEWAY_TIMEOUT => "上游响应超时",
                        _ => "上游返回了错误响应",
                    };
                    (
                        "上游错误",
                        "上游错误",
                        "upstream",
                        reason,
                        domain.as_str(),
                    )
                }
                _ => unreachable!(),
            };

            let html = ERROR_PAGE_TEMPLATE
                .replace("{status_code}", status.as_str())
                .replace(
                    "{status_reason}",
                    status.canonical_reason().unwrap_or("Error"),
                )
                .replace("{error_type}", error_type)
                .replace("{error_desc}", error_desc)
                .replace("{domain}", domain)
                .replace("{badge_label}", badge_label)
                .replace("{badge_class}", badge_class);

            return Response::builder()
                .status(status)
                .header("content-type", "text/html; charset=utf-8")
                .body(Body::from(html))
                .unwrap();
        }

        (status, status.canonical_reason().unwrap_or("proxy error")).into_response()
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

const ERROR_PAGE_TEMPLATE: &str = r#"<!DOCTYPE html>
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
