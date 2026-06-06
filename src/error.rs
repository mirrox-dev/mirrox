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
            AppError::UpstreamTimeout(_) => {
                AppError::UpstreamTimeout(incoming_host.to_string())
            }
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
        };

        // For gateway errors (502/504), return a modern HTML error page
        if matches!(
            self,
            AppError::Dns { .. } | AppError::Upstream(..) | AppError::UpstreamTimeout(..)
        ) {
            let (error_type, incoming_host) = match &self {
                AppError::Dns { incoming_host, .. } => ("DNS 解析失败", incoming_host.as_str()),
                AppError::Upstream(_, incoming_host) => ("连接失败", incoming_host.as_str()),
                AppError::UpstreamTimeout(incoming_host) => ("连接超时", incoming_host.as_str()),
                _ => unreachable!(),
            };

            let html = ERROR_PAGE_TEMPLATE
                .replace("{status_code}", status.as_str())
                .replace("{status_reason}", status.canonical_reason().unwrap_or("Error"))
                .replace("{error_type}", error_type)
                .replace("{domain}", incoming_host);

            return Response::builder()
                .status(status)
                .header("content-type", "text/html; charset=utf-8")
                .body(axum::body::Body::from(html))
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
    <title>{status_code} {status_reason}</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 20px;
        }
        .card {
            background: white;
            border-radius: 16px;
            box-shadow: 0 20px 60px rgba(0, 0, 0, 0.3);
            padding: 48px 40px;
            max-width: 520px;
            width: 100%;
            text-align: center;
        }
        .icon {
            width: 80px;
            height: 80px;
            background: #fff3cd;
            border-radius: 50%;
            display: flex;
            align-items: center;
            justify-content: center;
            margin: 0 auto 24px;
        }
        .icon svg {
            width: 40px;
            height: 40px;
            color: #856404;
        }
        h1 {
            font-size: 24px;
            font-weight: 700;
            color: #1a1a2e;
            margin-bottom: 8px;
        }
        .domain {
            font-size: 18px;
            color: #667eea;
            font-weight: 600;
            margin-bottom: 24px;
            word-break: break-all;
        }
        .error-badge {
            display: inline-block;
            background: #fee2e2;
            color: #dc2626;
            padding: 6px 16px;
            border-radius: 20px;
            font-size: 14px;
            font-weight: 500;
            margin-bottom: 24px;
        }
        .message {
            font-size: 15px;
            color: #555;
            line-height: 1.6;
            margin-bottom: 32px;
        }
        .footer {
            font-size: 13px;
            color: #999;
            border-top: 1px solid #eee;
            padding-top: 20px;
        }
        @media (max-width: 480px) {
            .card { padding: 32px 24px; }
            h1 { font-size: 20px; }
            .domain { font-size: 16px; }
        }
    </style>
</head>
<body>
    <div class="card">
        <div class="icon">
            <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z" />
            </svg>
        </div>
        <h1>无法访问此网站</h1>
        <div class="domain">{domain}</div>
        <div class="error-badge">⚠ {error_type}</div>
        <div class="message">
            当前网站暂时无法响应您的请求。这可能是服务器维护、网络波动或配置问题导致的，请稍后再试。
        </div>
        <div class="footer">
            如果问题持续存在，请联系该网站的管理员获取帮助。
        </div>
    </div>
</body>
</html>"#;
