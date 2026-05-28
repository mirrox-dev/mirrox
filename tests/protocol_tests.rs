use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE, HOST, USER_AGENT};
use axum::http::HeaderValue;
use axum::response::{IntoResponse, Sse};
use axum::routing::any;
use axum::Router;
use futures_util::{stream, SinkExt, StreamExt};
use mirrox::config::AppConfig;
use mirrox::dns::StaticResolver;
use mirrox::proxy::ProxyState;
use mirrox::routing::RouteTable;
use mirrox::server::build_router_with_state;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

async fn spawn_upstream(app: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

async fn build_test_app(addr: SocketAddr, max_buffer_bytes: usize) -> Router {
    let config = AppConfig::from_toml_str(&format!(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [rewrite]
        max_buffer_bytes = {max_buffer_bytes}

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
    "#
    ))
    .unwrap();
    let routes = RouteTable::from_config(&config);
    let dns = Arc::new(StaticResolver::new(vec![addr]));
    build_router_with_state(Arc::new(ProxyState::new(config, routes, dns)))
        .await
        .unwrap()
}

async fn build_test_server(addr: SocketAddr, max_buffer_bytes: usize) -> axum_test::TestServer {
    axum_test::TestServer::new(build_test_app(addr, max_buffer_bytes).await).unwrap()
}

async fn build_test_app_with_proxy(addr: SocketAddr, upstream_proxy: &str) -> Router {
    let config = AppConfig::from_toml_str(&format!(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [upstream_proxy]
        default = "{upstream_proxy}"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
    "#
    ))
    .unwrap();
    let routes = RouteTable::from_config(&config);
    let dns = Arc::new(StaticResolver::new(vec![addr]));
    build_router_with_state(Arc::new(ProxyState::new(config, routes, dns)))
        .await
        .unwrap()
}

async fn build_test_app_with_route_config(addr: SocketAddr, route_config: &str) -> Router {
    let config = AppConfig::from_toml_str(&format!(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        {route_config}
    "#
    ))
    .unwrap();
    let routes = RouteTable::from_config(&config);
    let dns = Arc::new(StaticResolver::new(vec![addr]));
    build_router_with_state(Arc::new(ProxyState::new(config, routes, dns)))
        .await
        .unwrap()
}

async fn sse_upstream() -> impl IntoResponse {
    let events = stream::iter([Ok::<_, Infallible>(
        axum::response::sse::Event::default().data("https://api.bgm.tv/v0/subjects/1"),
    )]);
    Sse::new(events)
}

async fn large_text_upstream(State(body): State<String>) -> impl IntoResponse {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&body.len().to_string()).unwrap(),
    );
    (headers, body)
}

async fn ws_upstream(upgrade: WebSocketUpgrade) -> impl IntoResponse {
    upgrade.on_upgrade(echo_websocket)
}

async fn ws_seen_user_agent_upstream(
    State(seen): State<Arc<tokio::sync::Mutex<Vec<String>>>>,
    upgrade: WebSocketUpgrade,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    seen.lock().await.push(user_agent);
    upgrade.on_upgrade(echo_websocket)
}

async fn ws_seen_host_upstream(
    State(seen): State<Arc<tokio::sync::Mutex<Vec<String>>>>,
    upgrade: WebSocketUpgrade,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    seen.lock().await.push(host);
    upgrade.on_upgrade(echo_websocket)
}

async fn echo_websocket(mut socket: WebSocket) {
    while let Some(Ok(message)) = socket.recv().await {
        if socket.send(message).await.is_err() {
            break;
        }
    }
}

#[tokio::test]
async fn sse_is_not_body_rewritten() {
    let app = Router::new().route("/*path", any(sse_upstream));
    let addr = spawn_upstream(app).await;
    let server = build_test_server(addr, 1024).await;

    let response = server
        .get("/events")
        .add_header("host", "api.example.com")
        .await;

    response.assert_status_ok();
    response.assert_text_contains("https://api.bgm.tv/v0/subjects/1");
}

#[tokio::test]
async fn oversized_text_body_is_passed_through() {
    let body = "https://api.bgm.tv/".repeat(8);
    let app = Router::new()
        .route("/*path", any(large_text_upstream))
        .with_state(body.clone());
    let addr = spawn_upstream(app).await;
    let server = build_test_server(addr, 32).await;

    let response = server
        .get("/large")
        .add_header("host", "api.example.com")
        .await;

    response.assert_status_ok();
    response.assert_text(&body);
}

#[tokio::test]
async fn websocket_messages_are_proxied() {
    let app = Router::new().route("/*path", any(ws_upstream));
    let addr = spawn_upstream(app).await;
    let proxy_app = build_test_app(addr, 1024).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, proxy_app).await.unwrap();
    });
    let url = format!("ws://{proxy_addr}/socket");

    let mut request = url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("host", HeaderValue::from_static("api.example.com"));
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text("ping".into()))
        .await
        .unwrap();

    let message = socket.next().await.unwrap().unwrap();

    assert_eq!(
        message,
        tokio_tungstenite::tungstenite::Message::Text("ping".into())
    );
}

async fn spawn_http_connect_proxy_for_ws(target: SocketAddr) -> (SocketAddr, Arc<AtomicBool>) {
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
async fn websocket_uses_http_custom_upstream_port() {
    let seen = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let upstream_app = Router::new()
        .route("/*path", any(ws_seen_host_upstream))
        .with_state(seen.clone());
    let upstream_addr = spawn_upstream(upstream_app).await;
    let proxy_app = build_test_app_with_route_config(
        upstream_addr,
        &format!(
            r#"
        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
        upstream_port = {}
    "#,
            upstream_addr.port()
        ),
    )
    .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, proxy_app).await.unwrap();
    });
    let url = format!("ws://{proxy_addr}/custom-port");
    let mut request = url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("host", HeaderValue::from_static("api.example.com"));
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();

    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            "custom-port".into(),
        ))
        .await
        .unwrap();
    let message = socket.next().await.unwrap().unwrap();

    assert_eq!(
        seen.lock().await[0],
        format!("api.bgm.tv:{}", upstream_addr.port())
    );
    assert_eq!(
        message,
        tokio_tungstenite::tungstenite::Message::Text("custom-port".into())
    );
}

#[tokio::test]
async fn websocket_route_user_agent_overrides_client_header() {
    let seen = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let upstream_app = Router::new()
        .route("/*path", any(ws_seen_user_agent_upstream))
        .with_state(seen.clone());
    let upstream_addr = spawn_upstream(upstream_app).await;
    let proxy_app = build_test_app_with_route_config(
        upstream_addr,
        r#"
        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
        user_agent = "Mirrox-WS/1.0"
    "#,
    )
    .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, proxy_app).await.unwrap();
    });
    let url = format!("ws://{proxy_addr}/ua");
    let mut request = url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("host", HeaderValue::from_static("api.example.com"));
    request
        .headers_mut()
        .insert(USER_AGENT, HeaderValue::from_static("Client-WS/9.9"));

    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text("ua".into()))
        .await
        .unwrap();
    let _ = socket.next().await.unwrap().unwrap();

    assert_eq!(seen.lock().await[0], "Mirrox-WS/1.0");
}

#[tokio::test]
async fn websocket_omitted_user_agent_preserves_client_header() {
    let seen = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let upstream_app = Router::new()
        .route("/*path", any(ws_seen_user_agent_upstream))
        .with_state(seen.clone());
    let upstream_addr = spawn_upstream(upstream_app).await;
    let proxy_app = build_test_app_with_route_config(
        upstream_addr,
        r#"
        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
    "#,
    )
    .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, proxy_app).await.unwrap();
    });
    let url = format!("ws://{proxy_addr}/ua-preserve");
    let mut request = url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("host", HeaderValue::from_static("api.example.com"));
    request
        .headers_mut()
        .insert(USER_AGENT, HeaderValue::from_static("Client-WS/9.9"));

    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            "ua-preserve".into(),
        ))
        .await
        .unwrap();
    let _ = socket.next().await.unwrap().unwrap();

    assert_eq!(seen.lock().await[0], "Client-WS/9.9");
}

#[tokio::test]
async fn websocket_messages_are_proxied_through_http_upstream_proxy() {
    let upstream_app = Router::new().route("/*path", any(ws_upstream));
    let upstream_addr = spawn_upstream(upstream_app).await;
    let (proxy_addr, proxy_used) = spawn_http_connect_proxy_for_ws(upstream_addr).await;
    let proxy_app = build_test_app_with_proxy(upstream_addr, &format!("http://{proxy_addr}")).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_server_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, proxy_app).await.unwrap();
    });
    let url = format!("ws://{proxy_server_addr}/socket");
    let mut request = url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("host", HeaderValue::from_static("api.example.com"));
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();

    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            "proxied".into(),
        ))
        .await
        .unwrap();
    let message = socket.next().await.unwrap().unwrap();

    assert!(proxy_used.load(Ordering::SeqCst));
    assert_eq!(
        message,
        tokio_tungstenite::tungstenite::Message::Text("proxied".into())
    );
}
