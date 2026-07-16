use crate::error_page::{self, ErrorPage, FailureKind, Language};
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

/// Request-specific context attached to gateway errors at the proxy boundary.
#[derive(Debug, Clone)]
pub struct ErrorContext {
    pub incoming_host: String,
    pub language: String,
    pub proxy_error_status: Option<StatusCode>,
}

impl AppError {
    /// Create an upstream connection error that can be enriched at the handler boundary.
    pub fn upstream(source: impl Into<anyhow::Error>) -> Self {
        AppError::Upstream(source.into(), String::new(), String::new(), None)
    }

    pub fn enrich(self, context: &ErrorContext) -> Self {
        match self {
            AppError::Dns { host, source, .. } => AppError::Dns {
                host,
                source,
                incoming_host: context.incoming_host.clone(),
                language: context.language.clone(),
                proxy_error_status: context.proxy_error_status,
            },
            AppError::Upstream(source, _, _, _) => AppError::Upstream(
                source,
                context.incoming_host.clone(),
                context.language.clone(),
                context.proxy_error_status,
            ),
            AppError::UpstreamTimeout(_, _, _) => AppError::UpstreamTimeout(
                context.incoming_host.clone(),
                context.language.clone(),
                context.proxy_error_status,
            ),
            AppError::UpstreamError { status, domain, .. } => AppError::UpstreamError {
                status,
                domain,
                incoming_host: context.incoming_host.clone(),
                language: context.language.clone(),
                proxy_error_status: context.proxy_error_status,
            },
            other => other,
        }
    }

    /// Returns `zh` when the accepted languages contain Chinese, otherwise `en`.
    pub fn detect_language(accept_language: Option<&str>) -> &str {
        match accept_language {
            Some(header) if header.to_ascii_lowercase().contains("zh") => "zh",
            _ => "en",
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
            AppError::Dns { host, language, .. } => Some(GatewayErrorFields {
                failure_kind: FailureKind::Dns,
                language: Language::from_code(language),
                domain: host.clone(),
            }),
            AppError::Upstream(_, incoming_host, language, _) => Some(GatewayErrorFields {
                failure_kind: FailureKind::Connection,
                language: Language::from_code(language),
                domain: incoming_host.clone(),
            }),
            AppError::UpstreamTimeout(incoming_host, language, _) => Some(GatewayErrorFields {
                failure_kind: FailureKind::Timeout,
                language: Language::from_code(language),
                domain: incoming_host.clone(),
            }),
            AppError::UpstreamError {
                domain, language, ..
            } => Some(GatewayErrorFields {
                failure_kind: FailureKind::Upstream,
                language: Language::from_code(language),
                domain: domain.clone(),
            }),
            _ => None,
        }
    }
}

struct GatewayErrorFields {
    failure_kind: FailureKind,
    language: Language,
    domain: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let original_status = self.original_status();
        let response_status = self.response_status(original_status);

        if let Some(fields) = self.gateway_error_fields() {
            let html = error_page::render(ErrorPage {
                language: fields.language,
                status: original_status,
                domain: &fields.domain,
                failure_kind: fields.failure_kind,
            });
            let mut builder = Response::builder()
                .status(response_status)
                .header("content-type", "text/html; charset=utf-8")
                .header(
                    "content-security-policy",
                    "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'self'",
                )
                .header("referrer-policy", "no-referrer")
                .header("x-content-type-options", "nosniff");
            if original_status.is_server_error() {
                builder = builder
                    .header("x-mirrox-error-status", original_status.as_str())
                    .header("cache-control", "no-store");
            }

            return match builder.body(Body::from(html)) {
                Ok(response) => response,
                Err(_) => fallback_response(response_status, original_status),
            };
        }

        fallback_response(response_status, original_status)
    }
}

fn fallback_response(response_status: StatusCode, original_status: StatusCode) -> Response {
    (
        response_status,
        original_status.canonical_reason().unwrap_or("proxy error"),
    )
        .into_response()
}

pub type Result<T> = std::result::Result<T, AppError>;
