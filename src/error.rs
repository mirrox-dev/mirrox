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
        context: ErrorContext,
    },
    #[error("upstream request failed: {source}")]
    Upstream {
        source: anyhow::Error,
        context: ErrorContext,
    },
    #[error("upstream timed out")]
    UpstreamTimeout { context: ErrorContext },
    #[error("upstream returned {status}")]
    UpstreamError {
        status: StatusCode,
        domain: String,
        context: ErrorContext,
    },
}

/// Request-specific context attached to gateway errors at the proxy boundary.
#[derive(Debug, Clone)]
pub struct ErrorContext {
    pub incoming_host: String,
    pub language: String,
    pub proxy_error_status: Option<StatusCode>,
    pub retry_allowed: bool,
}

impl Default for ErrorContext {
    fn default() -> Self {
        Self {
            incoming_host: String::new(),
            language: "en".to_string(),
            proxy_error_status: None,
            retry_allowed: false,
        }
    }
}

impl AppError {
    pub fn dns(host: impl Into<String>, source: impl Into<anyhow::Error>) -> Self {
        Self::Dns {
            host: host.into(),
            source: source.into(),
            context: ErrorContext::default(),
        }
    }

    pub fn upstream(source: impl Into<anyhow::Error>) -> Self {
        Self::Upstream {
            source: source.into(),
            context: ErrorContext::default(),
        }
    }

    pub fn upstream_timeout() -> Self {
        Self::UpstreamTimeout {
            context: ErrorContext::default(),
        }
    }

    pub fn upstream_response(status: StatusCode, domain: impl Into<String>) -> Self {
        Self::UpstreamError {
            status,
            domain: domain.into(),
            context: ErrorContext::default(),
        }
    }

    pub fn enrich(mut self, context: &ErrorContext) -> Self {
        if let Some(error_context) = self.context_mut() {
            error_context.clone_from(context);
        }
        self
    }

    /// Returns `zh` when the accepted languages contain Chinese, otherwise `en`.
    pub fn detect_language(accept_language: Option<&str>) -> &str {
        match accept_language {
            Some(header) if header.to_ascii_lowercase().contains("zh") => "zh",
            _ => "en",
        }
    }

    fn context_mut(&mut self) -> Option<&mut ErrorContext> {
        match self {
            AppError::Dns { context, .. }
            | AppError::Upstream { context, .. }
            | AppError::UpstreamTimeout { context }
            | AppError::UpstreamError { context, .. } => Some(context),
            AppError::Config(_) | AppError::RouteNotFound(_) => None,
        }
    }

    fn context(&self) -> Option<&ErrorContext> {
        match self {
            AppError::Dns { context, .. }
            | AppError::Upstream { context, .. }
            | AppError::UpstreamTimeout { context }
            | AppError::UpstreamError { context, .. } => Some(context),
            AppError::Config(_) | AppError::RouteNotFound(_) => None,
        }
    }

    fn original_status(&self) -> StatusCode {
        match self {
            AppError::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::RouteNotFound(_) => StatusCode::MISDIRECTED_REQUEST,
            AppError::Dns { .. } | AppError::Upstream { .. } => StatusCode::BAD_GATEWAY,
            AppError::UpstreamTimeout { .. } => StatusCode::GATEWAY_TIMEOUT,
            AppError::UpstreamError { status, .. } => *status,
        }
    }

    /// Only custom 5xx responses may use the CDN tunneling status.
    fn response_status(&self, original: StatusCode) -> StatusCode {
        if original.is_server_error() {
            self.context()
                .and_then(|context| context.proxy_error_status)
                .unwrap_or(original)
        } else {
            original
        }
    }

    fn gateway_error_fields(&self) -> Option<GatewayErrorFields> {
        match self {
            AppError::Dns { context, .. } => Some(GatewayErrorFields {
                failure_kind: FailureKind::Dns,
                language: Language::from_code(&context.language),
                domain: context.incoming_host.clone(),
                retry_allowed: context.retry_allowed,
            }),
            AppError::Upstream { context, .. } => Some(GatewayErrorFields {
                failure_kind: FailureKind::Connection,
                language: Language::from_code(&context.language),
                domain: context.incoming_host.clone(),
                retry_allowed: context.retry_allowed,
            }),
            AppError::UpstreamTimeout { context } => Some(GatewayErrorFields {
                failure_kind: FailureKind::Timeout,
                language: Language::from_code(&context.language),
                domain: context.incoming_host.clone(),
                retry_allowed: context.retry_allowed,
            }),
            AppError::UpstreamError { context, .. } => Some(GatewayErrorFields {
                failure_kind: FailureKind::Upstream,
                language: Language::from_code(&context.language),
                domain: context.incoming_host.clone(),
                retry_allowed: context.retry_allowed,
            }),
            AppError::Config(_) | AppError::RouteNotFound(_) => None,
        }
    }
}

struct GatewayErrorFields {
    failure_kind: FailureKind,
    language: Language,
    domain: String,
    retry_allowed: bool,
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
                retry_allowed: fields.retry_allowed,
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
