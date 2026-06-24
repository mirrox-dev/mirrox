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

    // Register the static script file handler if scripts are configured.
    if !state.config.scripts.global.is_empty()
        || state
            .config
            .routes
            .iter()
            .any(|r| !r.scripts.is_empty())
        || state
            .config
            .wildcard_routes
            .iter()
            .any(|r| !r.scripts.is_empty())
    {
        let script_state = Arc::new(ScriptServerState::new(&state.config.scripts.dir));
        router = router.route(&script_route, get(static_file_handler).with_state(script_state));
    }

    Ok(router.fallback(proxy_handler).with_state(state))
}

pub async fn run(config: AppConfig) -> anyhow::Result<()> {
    let listen: SocketAddr = config.server.listen.parse()?;
    let app = build_router(config).await?;
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(%listen, "proxy listening");
    axum::serve(listener, app).await?;
    Ok(())
}
