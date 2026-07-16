use crate::config::AppConfig;
use crate::dns::ResolverFactory;
use crate::error::AppError;
use crate::proxy::{proxy_handler, ProxyState};
use crate::routing::RouteTable;
use crate::scripts::{static_file_handler, ScriptServerState};
use axum::routing::get;
use axum::Router;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use tower::ServiceBuilder;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
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

    let request_id_header = http::HeaderName::from_static("x-request-id");
    let middleware = ServiceBuilder::new()
        .layer(SetRequestIdLayer::new(
            request_id_header.clone(),
            MakeRequestUuid,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(PropagateRequestIdLayer::new(request_id_header));

    Ok(router
        .layer(middleware)
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
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "failed to listen for SIGINT");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!(%error, "failed to listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    wait_for_shutdown(ctrl_c, terminate).await;
}

async fn wait_for_shutdown<C, T>(ctrl_c: C, terminate: T)
where
    C: Future<Output = ()>,
    T: Future<Output = ()>,
{
    let signal = tokio::select! {
        _ = ctrl_c => "SIGINT",
        _ = terminate => "SIGTERM",
    };
    tracing::info!(signal, "starting graceful shutdown");
}

#[cfg(test)]
mod tests {
    use super::wait_for_shutdown;
    use std::future::{pending, ready};
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn shutdown_trigger_completes_without_an_artificial_delay() {
        timeout(
            Duration::from_millis(50),
            wait_for_shutdown(ready(()), pending()),
        )
        .await
        .expect("shutdown should begin as soon as a signal future resolves");
    }
}
