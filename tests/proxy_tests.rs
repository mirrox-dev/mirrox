use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum::routing::any;
use axum::{Json, Router};
use mirrox::config::AppConfig;
use mirrox::dns::StaticResolver;
use mirrox::proxy::ProxyState;
use mirrox::routing::RouteTable;
use mirrox::server::build_router_with_state;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn upstream(
    State(state): State<Arc<tokio::sync::Mutex<Vec<String>>>>,
    request: axum::extract::Request,
) -> Json<serde_json::Value> {
    let host = request
        .headers()
        .get("host")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let path = request.uri().path_and_query().unwrap().as_str().to_string();
    let user_agent = request
        .headers()
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    state
        .lock()
        .await
        .push(format!("{host}{path}|ua={user_agent}"));
    Json(json!({ "host": host, "path": path, "user_agent": user_agent }))
}

async fn spawn_upstream() -> (SocketAddr, Arc<tokio::sync::Mutex<Vec<String>>>) {
    let seen = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/*path", any(upstream))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, seen)
}

#[tokio::test]
async fn forwards_request_to_matched_upstream() {
    let (addr, seen) = spawn_upstream().await;
    let config = AppConfig::from_toml_str(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
    "#,
    )
    .unwrap();
    let routes = RouteTable::from_config(&config);
    let dns = Arc::new(StaticResolver::new(vec![addr]));
    let app = build_router_with_state(Arc::new(ProxyState::new(config, routes, dns)))
        .await
        .unwrap();

    let server = axum_test::TestServer::new(app).unwrap();
    let response = server
        .get("/v0/subjects/1?responseGroup=small")
        .add_header("host", "api.example.com")
        .await;

    response.assert_status_ok();
    response.assert_json(&json!({
        "host": "api.example.com",
        "path": "/v0/subjects/1?responseGroup=small",
        "user_agent": ""
    }));
    assert_eq!(
        seen.lock().await[0],
        "api.bgm.tv/v0/subjects/1?responseGroup=small|ua="
    );
}

#[tokio::test]
async fn rejects_unknown_host() {
    let (addr, _seen) = spawn_upstream().await;
    let config = AppConfig::from_toml_str(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
    "#,
    )
    .unwrap();
    let routes = RouteTable::from_config(&config);
    let dns = Arc::new(StaticResolver::new(vec![addr]));
    let app = build_router_with_state(Arc::new(ProxyState::new(config, routes, dns)))
        .await
        .unwrap();

    let server = axum_test::TestServer::new(app).unwrap();
    let response = server
        .get("/v0/subjects/1")
        .add_header("host", "evil.example.net")
        .await;

    response.assert_status(axum::http::StatusCode::MISDIRECTED_REQUEST);
}

async fn rewrite_upstream() -> impl IntoResponse {
    (
        [
            ("location", "https://api.bgm.tv/v0/subjects/2"),
            ("set-cookie", "session=abc; Domain=.bgm.tv; Path=/"),
        ],
        Html("<a href=\"https://api.bgm.tv/v0/subjects/2\">subject</a>"),
    )
}

#[tokio::test]
async fn rewrites_response_headers_and_text_body() {
    let app = Router::new().route("/*path", any(rewrite_upstream));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let config = AppConfig::from_toml_str(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
    "#,
    )
    .unwrap();
    let routes = RouteTable::from_config(&config);
    let dns = Arc::new(StaticResolver::new(vec![addr]));
    let app = build_router_with_state(Arc::new(ProxyState::new(config, routes, dns)))
        .await
        .unwrap();

    let server = axum_test::TestServer::new(app).unwrap();
    let response = server
        .get("/page")
        .add_header("host", "api.example.com")
        .await;

    response.assert_status_ok();
    assert_eq!(
        response.headers()["location"],
        "https://api.example.com/v0/subjects/2"
    );
    assert_eq!(
        response.headers()["set-cookie"],
        "session=abc; Domain=.example.com; Path=/"
    );
    response.assert_text_contains("https://api.example.com/v0/subjects/2");
}

#[tokio::test]
async fn http_only_route_does_not_rewrite_body() {
    let app = Router::new().route("/*path", any(rewrite_upstream));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let config = AppConfig::from_toml_str(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
        body_rewrite = "http-only"
    "#,
    )
    .unwrap();
    let routes = RouteTable::from_config(&config);
    let dns = Arc::new(StaticResolver::new(vec![addr]));
    let app = build_router_with_state(Arc::new(ProxyState::new(config, routes, dns)))
        .await
        .unwrap();

    let server = axum_test::TestServer::new(app).unwrap();
    let response = server
        .get("/page")
        .add_header("host", "api.example.com")
        .await;

    response.assert_status_ok();
    assert_eq!(
        response.headers()["location"],
        "https://api.example.com/v0/subjects/2"
    );
    response.assert_text_contains("https://api.bgm.tv/v0/subjects/2");
}

async fn spawn_http_connect_forward_proxy(target: SocketAddr) -> (SocketAddr, Arc<AtomicBool>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let used = Arc::new(AtomicBool::new(false));
    let used_for_task = used.clone();
    tokio::spawn(async move {
        let (mut client, _) = listener.accept().await.unwrap();
        used_for_task.store(true, Ordering::SeqCst);
        let mut request = Vec::new();
        let mut byte = [0_u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            client.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
        }
        let request = String::from_utf8(request).unwrap();
        assert!(request.contains("CONNECT api.bgm.tv:80 HTTP/1.1"));
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        let mut upstream = tokio::net::TcpStream::connect(target).await.unwrap();
        tokio::io::copy_bidirectional(&mut client, &mut upstream)
            .await
            .unwrap();
    });
    (addr, used)
}

#[tokio::test]
async fn forwards_request_through_http_upstream_proxy() {
    let (upstream_addr, seen) = spawn_upstream().await;
    let (proxy_addr, proxy_used) = spawn_http_connect_forward_proxy(upstream_addr).await;
    let config = AppConfig::from_toml_str(&format!(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [upstream_proxy]
        default = "http://{proxy_addr}"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
    "#
    ))
    .unwrap();
    let routes = RouteTable::from_config(&config);
    let dns = Arc::new(StaticResolver::new(vec![upstream_addr]));
    let app = build_router_with_state(Arc::new(ProxyState::new(config, routes, dns)))
        .await
        .unwrap();
    let server = axum_test::TestServer::new(app).unwrap();

    let response = server
        .get("/v0/proxy")
        .add_header("host", "api.example.com")
        .await;

    response.assert_status_ok();
    assert!(proxy_used.load(Ordering::SeqCst));
    assert_eq!(seen.lock().await[0], "api.bgm.tv/v0/proxy|ua=");
}

#[tokio::test]
async fn route_direct_override_bypasses_global_upstream_proxy() {
    let (upstream_addr, seen) = spawn_upstream().await;
    let bad_proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bad_proxy_addr = bad_proxy.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut socket, _)) = bad_proxy.accept().await {
            let _ = socket.write_all(b"HTTP/1.1 500 nope\r\n\r\n").await;
        }
    });
    let config = AppConfig::from_toml_str(&format!(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [upstream_proxy]
        default = "http://{bad_proxy_addr}"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
        upstream_proxy = "direct"
    "#
    ))
    .unwrap();
    let routes = RouteTable::from_config(&config);
    let dns = Arc::new(StaticResolver::new(vec![upstream_addr]));
    let app = build_router_with_state(Arc::new(ProxyState::new(config, routes, dns)))
        .await
        .unwrap();
    let server = axum_test::TestServer::new(app).unwrap();

    let response = server
        .get("/direct")
        .add_header("host", "api.example.com")
        .await;

    response.assert_status_ok();
    assert_eq!(seen.lock().await[0], "api.bgm.tv/direct|ua=");
}

async fn spawn_socks5_forward_proxy(target: SocketAddr) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut client, _) = listener.accept().await.unwrap();
        let mut greeting = [0_u8; 3];
        client.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [0x05, 0x01, 0x00]);
        client.write_all(&[0x05, 0x00]).await.unwrap();
        let mut prefix = [0_u8; 5];
        client.read_exact(&mut prefix).await.unwrap();
        assert_eq!(prefix, [0x05, 0x01, 0x00, 0x03, 0x0a]);
        let mut host = [0_u8; 10];
        client.read_exact(&mut host).await.unwrap();
        assert_eq!(&host, b"api.bgm.tv");
        let mut port = [0_u8; 2];
        client.read_exact(&mut port).await.unwrap();
        assert_eq!(u16::from_be_bytes(port), 80);
        client
            .write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0x1f, 0x90])
            .await
            .unwrap();
        let mut upstream = tokio::net::TcpStream::connect(target).await.unwrap();
        tokio::io::copy_bidirectional(&mut client, &mut upstream)
            .await
            .unwrap();
    });
    addr
}

#[tokio::test]
async fn forwards_request_through_socks5_upstream_proxy() {
    let (upstream_addr, seen) = spawn_upstream().await;
    let proxy_addr = spawn_socks5_forward_proxy(upstream_addr).await;
    let config = AppConfig::from_toml_str(&format!(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [upstream_proxy]
        default = "socks5://{proxy_addr}"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
    "#
    ))
    .unwrap();
    let routes = RouteTable::from_config(&config);
    let dns = Arc::new(StaticResolver::new(vec![upstream_addr]));
    let app = build_router_with_state(Arc::new(ProxyState::new(config, routes, dns)))
        .await
        .unwrap();
    let server = axum_test::TestServer::new(app).unwrap();

    let response = server
        .get("/socks")
        .add_header("host", "api.example.com")
        .await;

    response.assert_status_ok();
    assert_eq!(seen.lock().await[0], "api.bgm.tv/socks|ua=");
}

#[tokio::test]
async fn route_can_forward_to_http_custom_port() {
    let (addr, seen) = spawn_upstream().await;
    let config = AppConfig::from_toml_str(&format!(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
        upstream_port = {}
    "#,
        addr.port()
    ))
    .unwrap();
    let routes = RouteTable::from_config(&config);
    let dns = Arc::new(StaticResolver::new(vec![addr]));
    let app = build_router_with_state(Arc::new(ProxyState::new(config, routes, dns)))
        .await
        .unwrap();

    let server = axum_test::TestServer::new(app).unwrap();
    let response = server
        .get("/custom-port")
        .add_header("host", "api.example.com")
        .await;

    response.assert_status_ok();
    assert_eq!(seen.lock().await[0], "api.bgm.tv/custom-port|ua=");
}

#[tokio::test]
async fn route_user_agent_overrides_client_header() {
    let (addr, seen) = spawn_upstream().await;
    let config = AppConfig::from_toml_str(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
        user_agent = "Mirrox-UA/1.0"
    "#,
    )
    .unwrap();
    let routes = RouteTable::from_config(&config);
    let dns = Arc::new(StaticResolver::new(vec![addr]));
    let app = build_router_with_state(Arc::new(ProxyState::new(config, routes, dns)))
        .await
        .unwrap();

    let server = axum_test::TestServer::new(app).unwrap();
    let response = server
        .get("/ua")
        .add_header("host", "api.example.com")
        .add_header("user-agent", "Client-UA/9.9")
        .await;

    response.assert_status_ok();
    response.assert_json(&json!({
        "host": "api.example.com",
        "path": "/ua",
        "user_agent": "Mirrox-UA/1.0"
    }));
    assert_eq!(seen.lock().await[0], "api.bgm.tv/ua|ua=Mirrox-UA/1.0");
}

#[tokio::test]
async fn wildcard_user_agent_overrides_client_header() {
    let (addr, seen) = spawn_upstream().await;
    let config = AppConfig::from_toml_str(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [[wildcard_routes]]
        incoming_suffix = ".example.com"
        upstream_suffix = ".bgm.tv"
        upstream_scheme = "http"
        user_agent = "Wildcard-UA/2.0"
    "#,
    )
    .unwrap();
    let routes = RouteTable::from_config(&config);
    let dns = Arc::new(StaticResolver::new(vec![addr]));
    let app = build_router_with_state(Arc::new(ProxyState::new(config, routes, dns)))
        .await
        .unwrap();

    let server = axum_test::TestServer::new(app).unwrap();
    let response = server
        .get("/wildcard-ua")
        .add_header("host", "api.example.com")
        .add_header("user-agent", "Client-UA/9.9")
        .await;

    response.assert_status_ok();
    assert_eq!(
        seen.lock().await[0],
        "api.bgm.tv/wildcard-ua|ua=Wildcard-UA/2.0"
    );
}

#[tokio::test]
async fn omitted_user_agent_preserves_client_header() {
    let (addr, seen) = spawn_upstream().await;
    let config = AppConfig::from_toml_str(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
    "#,
    )
    .unwrap();
    let routes = RouteTable::from_config(&config);
    let dns = Arc::new(StaticResolver::new(vec![addr]));
    let app = build_router_with_state(Arc::new(ProxyState::new(config, routes, dns)))
        .await
        .unwrap();

    let server = axum_test::TestServer::new(app).unwrap();
    let response = server
        .get("/preserve-ua")
        .add_header("host", "api.example.com")
        .add_header("user-agent", "Client-UA/9.9")
        .await;

    response.assert_status_ok();
    assert_eq!(
        seen.lock().await[0],
        "api.bgm.tv/preserve-ua|ua=Client-UA/9.9"
    );
}
