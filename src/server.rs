use crate::config::AppConfig;
use crate::dns::ResolverFactory;
use crate::error::AppError;
use crate::proxy::{proxy_handler, ProxyState};
use crate::routing::RouteTable;
use crate::scripts::{static_file_handler, ScriptServerState};
use axum::routing::get;
use axum::Router;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tower_http::request_id::{MakeRequestUuid, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

pub async fn build_router(config: AppConfig) -> Result<Router, AppError> {
    let routes = RouteTable::from_config(&config);
    let dns = ResolverFactory::new(config.dns.clone())?.build()?;
    let state = Arc::new(ProxyState::new(config, routes, dns));
    build_router_with_state(state).await
}

pub async fn build_router_with_state(state: Arc<ProxyState>) -> Result<Router, AppError> {
    let script_prefix = state.config.scripts.prefix.trim_end_matches('/');
    let script_route = format!("{}/:filename", script_prefix);

    let mut router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ready" }));

    if !state.config.scripts.global.is_empty()
        || state.config.routes.iter().any(|r| !r.scripts.is_empty())
        || state
            .config
            .wildcard_routes
            .iter()
            .any(|r| !r.scripts.is_empty())
    {
        let script_state = Arc::new(ScriptServerState::new(&state.config.scripts.dir));
        router = router.route(
            &script_route,
            get(static_file_handler).with_state(script_state),
        );
    }

    Ok(router
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(tower_http::trace::DefaultMakeSpan::new().include_headers(true)),
        )
        .layer(SetRequestIdLayer::new(
            http::HeaderName::from_static("x-request-id"),
            MakeRequestUuid,
        ))
        .fallback(proxy_handler)
        .with_state(state))
}

pub async fn run(config: AppConfig) -> anyhow::Result<()> {
    let listen: SocketAddr = config.server.listen.parse()?;
    let app = build_router(config).await?;
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(%listen, "proxy listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("proxy shut down gracefully");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("received SIGINT, starting graceful shutdown");
        },
        _ = terminate => {
            tracing::info!("received SIGTERM, starting graceful shutdown");
        },
    }

    // Give in-flight requests time to drain.
    tokio::time::sleep(Duration::from_secs(5)).await;
}
