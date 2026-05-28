use crate::config::AppConfig;
use crate::dns::ResolverFactory;
use crate::error::AppError;
use crate::proxy::{proxy_handler, ProxyState};
use crate::routing::RouteTable;
use axum::routing::get;
use axum::Router;
use std::net::SocketAddr;
use std::sync::Arc;

pub async fn build_router(config: AppConfig) -> Result<Router, AppError> {
    let routes = RouteTable::from_config(&config);
    let dns = ResolverFactory::new(config.dns.clone())?.build()?;
    let state = Arc::new(ProxyState::new(config, routes, dns));
    build_router_with_state(state).await
}

pub async fn build_router_with_state(state: Arc<ProxyState>) -> Result<Router, AppError> {
    Ok(Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ready" }))
        .fallback(proxy_handler)
        .with_state(state))
}

pub async fn run(config: AppConfig) -> anyhow::Result<()> {
    let listen: SocketAddr = config.server.listen.parse()?;
    let app = build_router(config).await?;
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(%listen, "proxy listening");
    axum::serve(listener, app).await?;
    Ok(())
}
