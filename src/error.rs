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
    Dns { host: String, source: anyhow::Error },
    #[error("upstream request failed: {0}")]
    Upstream(anyhow::Error),
    #[error("upstream timed out")]
    UpstreamTimeout,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match self {
            AppError::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::RouteNotFound(_) => StatusCode::MISDIRECTED_REQUEST,
            AppError::Dns { .. } => StatusCode::BAD_GATEWAY,
            AppError::Upstream(_) => StatusCode::BAD_GATEWAY,
            AppError::UpstreamTimeout => StatusCode::GATEWAY_TIMEOUT,
        };
        (status, status.canonical_reason().unwrap_or("proxy error")).into_response()
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
