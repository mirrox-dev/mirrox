use crate::config::{AppConfig, BodyRewriteMode, UpstreamScheme};
use crate::dns::SharedDnsResolver;
use crate::error::AppError;
use crate::rewrite::{
    is_rewritable_content_type, registrable_suffix, rewrite_cookie_domain, rewrite_header_value,
    rewrite_text_body,
};
use crate::routing::{MatchedRoute, RouteTable};
use crate::upstream_proxy::{boxed_body_io, maybe_tls_stream, UpstreamConnector};
use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::header::{
    ACCEPT_ENCODING, CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, HOST, LOCATION,
    SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SET_COOKIE, UPGRADE, USER_AGENT,
};
use axum::http::{HeaderMap, HeaderValue, Request, Response, StatusCode, Uri};
use flate2::read::{DeflateDecoder, GzDecoder};
use futures_util::StreamExt;
use std::future::Future;
use std::io::Read;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tower::service_fn;

#[derive(Clone)]
pub struct ProxyState {
    pub config: AppConfig,
    pub routes: RouteTable,
    pub dns: SharedDnsResolver,
}

impl ProxyState {
    pub fn new(config: AppConfig, routes: RouteTable, dns: SharedDnsResolver) -> Self {
        Self {
            config,
            routes,
            dns,
        }
    }
}

pub async fn proxy_handler(
    State(state): State<Arc<ProxyState>>,
    request: Request<Body>,
) -> Result<Response<Body>, AppError> {
    let host = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::RouteNotFound("missing host".into()))?;
    let route = state
        .routes
        .match_host(host)
        .ok_or_else(|| AppError::RouteNotFound(host.to_string()))?;

    let incoming_host = host.to_string();
    let accept_language = request
        .headers()
        .get("accept-language")
        .and_then(|v| v.to_str().ok());
    let language = AppError::detect_language(accept_language).to_string();

    if is_websocket_upgrade(request.headers()) {
        return forward_websocket(state, route, request)
            .await
            .map_err(|e| e.with_incoming_host(&incoming_host).with_language(&language));
    }

    forward_http(state, route, request)
        .await
        .map_err(|e| e.with_incoming_host(&incoming_host).with_language(&language))
}

fn upstream_authority(route: &MatchedRoute) -> String {
    if route.upstream_port == route.upstream_scheme.default_port() {
        route.upstream_host.clone()
    } else {
        format!("{}:{}", route.upstream_host, route.upstream_port)
    }
}

async fn forward_http(
    state: Arc<ProxyState>,
    route: MatchedRoute,
    request: Request<Body>,
) -> Result<Response<Body>, AppError> {
    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    let port_suffix = if route.upstream_port == route.upstream_scheme.default_port() {
        String::new()
    } else {
        format!(":{}", route.upstream_port)
    };
    let upstream_uri: Uri = format!(
        "{}://{}{}{}",
        route.upstream_scheme.as_str(),
        route.upstream_host,
        port_suffix,
        path_and_query
    )
    .parse()
    .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new(), String::new()))?;

    let (mut parts, body) = request.into_parts();
    parts.uri = upstream_uri;
    let upstream_authority = upstream_authority(&route);
    parts.headers.insert(
        HOST,
        HeaderValue::from_str(&upstream_authority)
            .map_err(|err| AppError::Config(format!("invalid upstream host header: {err}")))?,
    );
    rewrite_request_headers(&mut parts.headers, &route);
    if let Some(user_agent) = &route.user_agent {
        let value = HeaderValue::from_str(user_agent)
            .map_err(|err| AppError::Config(format!("invalid user_agent header: {err}")))?;
        parts.headers.insert(USER_AGENT, value);
    }
    if route.body_rewrite == BodyRewriteMode::Enabled {
        // Request uncompressed content so we can safely rewrite the body.
        // Removing Accept-Encoding would mean "any encoding is acceptable" per
        // RFC 7231 §5.3.4, so we explicitly send `identity` instead.
        parts
            .headers
            .insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    }
    remove_hop_by_hop_headers(&mut parts.headers, false);

    let upstream_request = Request::from_parts(parts, body);
    let connector = UpstreamConnector::new(state.dns.clone());
    let route_for_connect = route.clone();
    let connect_timeout = Duration::from_millis(state.config.server.connect_timeout_ms);
    let service = service_fn(move |uri: Uri| {
        let connector = connector.clone();
        let route = route_for_connect.clone();
        let future: Pin<Box<dyn Future<Output = Result<_, AppError>> + Send>> =
            Box::pin(async move {
                let host = uri.host().ok_or_else(|| {
                    AppError::Upstream(anyhow::anyhow!("missing upstream host"), String::new(), String::new())
                })?;
                let port = uri.port_u16().unwrap_or(route.upstream_port);
                let stream = timeout(
                    connect_timeout,
                    connector.connect(route.upstream_proxy.as_deref(), host, port),
                )
                .await
                .map_err(|_| AppError::UpstreamTimeout(String::new(), String::new()))??;
                let stream = timeout(
                    connect_timeout,
                    maybe_tls_stream(
                        stream,
                        host,
                        matches!(route.upstream_scheme, UpstreamScheme::Https),
                    ),
                )
                .await
                .map_err(|_| AppError::UpstreamTimeout(String::new(), String::new()))??;
                Ok(boxed_body_io(stream))
            });
        future
    });
    let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
        .build::<_, Body>(service);
    let request_timeout = Duration::from_millis(state.config.server.request_timeout_ms);
    let upstream_response = timeout(request_timeout, client.request(upstream_request))
        .await
        .map_err(|_| AppError::UpstreamTimeout(String::new(), String::new()))?
        .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new(), String::new()))?;

    rewrite_response(state, route, upstream_response).await
}

async fn forward_websocket(
    state: Arc<ProxyState>,
    route: MatchedRoute,
    request: Request<Body>,
) -> Result<Response<Body>, AppError> {
    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    let port_suffix = if route.upstream_port == route.upstream_scheme.default_port() {
        String::new()
    } else {
        format!(":{}", route.upstream_port)
    };
    let upstream_url = format!(
        "{}://{}{}{}",
        match route.upstream_scheme {
            UpstreamScheme::Http => "ws",
            UpstreamScheme::Https => "wss",
        },
        route.upstream_host,
        port_suffix,
        path_and_query
    );
    let mut upstream_request = upstream_url
        .into_client_request()
        .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new(), String::new()))?;
    let connector = UpstreamConnector::new(state.dns.clone());
    let upstream_proxy = route.upstream_proxy.clone();
    let upstream_host = route.upstream_host.clone();
    let upstream_port = route.upstream_port;
    let upstream_tls = matches!(route.upstream_scheme, UpstreamScheme::Https);
    let user_agent = route.user_agent.clone();
    let client_user_agent = request.headers().get(USER_AGENT).cloned();
    let upstream_authority = upstream_authority(&route);
    upstream_request.headers_mut().insert(
        HOST,
        HeaderValue::from_str(&upstream_authority)
            .map_err(|err| AppError::Config(format!("invalid upstream host header: {err}")))?,
    );
    if let Some(user_agent) = &user_agent {
        let value = HeaderValue::from_str(user_agent)
            .map_err(|err| AppError::Config(format!("invalid user_agent header: {err}")))?;
        upstream_request.headers_mut().insert(USER_AGENT, value);
    } else if let Some(user_agent) = client_user_agent {
        upstream_request
            .headers_mut()
            .insert(USER_AGENT, user_agent);
    }

    let accept_key = request
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
        .map(|value| derive_accept_key(value.as_bytes()))
        .ok_or_else(|| {
            AppError::Upstream(anyhow::anyhow!("missing Sec-WebSocket-Key"), String::new(), String::new())
        })?;
    let upgraded = hyper::upgrade::on(request);
    tokio::spawn(async move {
        let Ok(client) = upgraded.await else {
            return;
        };
        let Ok(stream) = connector
            .connect(upstream_proxy.as_deref(), &upstream_host, upstream_port)
            .await
        else {
            return;
        };
        let Ok(stream) = maybe_tls_stream(stream, &upstream_host, upstream_tls).await else {
            return;
        };
        let Ok((upstream, _)) = tokio_tungstenite::client_async(upstream_request, stream).await
        else {
            return;
        };
        let client = tokio_tungstenite::WebSocketStream::from_raw_socket(
            hyper_util::rt::TokioIo::new(client),
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        )
        .await;
        let (client_sink, client_stream) = client.split();
        let (upstream_sink, upstream_stream) = upstream.split();
        let client_to_upstream = client_stream.forward(upstream_sink);
        let upstream_to_client = upstream_stream.forward(client_sink);
        let _ = tokio::join!(client_to_upstream, upstream_to_client);
    });

    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(CONNECTION, "upgrade")
        .header(UPGRADE, "websocket")
        .header(SEC_WEBSOCKET_ACCEPT, accept_key)
        .body(Body::empty())
        .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new(), String::new()))
}

async fn rewrite_response(
    state: Arc<ProxyState>,
    route: MatchedRoute,
    response: Response<hyper::body::Incoming>,
) -> Result<Response<Body>, AppError> {
    let (mut parts, body) = response.into_parts();

    // Intercept upstream error responses (4xx/5xx) and return a custom error
    // page instead of forwarding the upstream's error to the client.
    if parts.status.is_client_error() || parts.status.is_server_error() {
        // Drain the body so the upstream connection is cleanly released.
        drop(body);
        return Err(AppError::UpstreamError {
            status: parts.status,
            domain: route.upstream_host.clone(),
            // incoming_host and language are filled in by with_incoming_host() and
            // with_language() in proxy_handler
            incoming_host: String::new(),
            language: String::new(),
        });
    }

    let rewrite_pairs = state.routes.rewrite_pairs_for(&route);
    rewrite_response_headers(&mut parts.headers, &route, &rewrite_pairs);

    let content_type = parts
        .headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let should_rewrite_body =
        route.body_rewrite == BodyRewriteMode::Enabled && is_rewritable_content_type(content_type);
    let content_length = parts
        .headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());

    if !should_rewrite_body
        || content_length.is_some_and(|length| length > state.config.rewrite.max_buffer_bytes)
    {
        let body = Body::new(http_body_util::BodyExt::boxed(body));
        return Ok(Response::from_parts(parts, body));
    }

    let bytes = to_bytes(
        Body::new(http_body_util::BodyExt::boxed(body)),
        state.config.rewrite.max_buffer_bytes,
    )
    .await
    .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new(), String::new()))?;

    let encoding = parts
        .headers
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_ascii_lowercase());
    let decoded = match encoding.as_deref() {
        Some("gzip") => {
            let mut decoder = GzDecoder::new(bytes.as_ref());
            let mut decompressed = Vec::new();
            decoder
                .read_to_end(&mut decompressed)
                .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new(), String::new()))?;
            parts.headers.remove("content-encoding");
            decompressed
        }
        Some("deflate") => {
            let mut decoder = DeflateDecoder::new(bytes.as_ref());
            let mut decompressed = Vec::new();
            decoder
                .read_to_end(&mut decompressed)
                .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new(), String::new()))?;
            parts.headers.remove("content-encoding");
            decompressed
        }
        Some("br") => {
            let mut decompressed = Vec::new();
            let mut reader: &[u8] = bytes.as_ref();
            brotli::BrotliDecompress(&mut reader, &mut decompressed)
                .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new(), String::new()))?;
            parts.headers.remove("content-encoding");
            decompressed
        }
        Some(_) => {
            // Unsupported encoding — pass through unchanged.
            // Text rewriting would corrupt the compressed binary payload.
            let body = Body::from(bytes.to_vec());
            return Ok(Response::from_parts(parts, body));
        }
        None => {
            // Safety net: detect compressed data by magic bytes in case the
            // upstream ignored our Accept-Encoding: identity and returned
            // compressed content without a Content-Encoding header.
            decompress_by_magic(bytes.as_ref())
        }
    };

    let text = String::from_utf8_lossy(&decoded);
    let mut rewritten = text.to_string();
    for (from, to) in &rewrite_pairs {
        rewritten = rewrite_text_body(&rewritten, from, to);
    }
    let new_len = rewritten.len();
    parts.headers.remove(CONTENT_LENGTH);
    parts.headers.remove("transfer-encoding");
    if new_len > 0 {
        if let Ok(val) = HeaderValue::from_str(&new_len.to_string()) {
            parts.headers.insert(CONTENT_LENGTH, val);
        }
    }
    Ok(Response::from_parts(parts, Body::from(rewritten)))
}

fn rewrite_request_headers(headers: &mut HeaderMap, route: &MatchedRoute) {
    for name in ["origin", "referer"] {
        if let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) {
            let rewritten = rewrite_header_value(value, &route.incoming_host, &route.upstream_host);
            if let Ok(value) = HeaderValue::from_str(&rewritten) {
                headers.insert(name, value);
            }
        }
    }
}

fn rewrite_response_headers(
    headers: &mut HeaderMap,
    route: &MatchedRoute,
    rewrite_pairs: &[(String, String)],
) {
    if let Some(value) = headers.get(LOCATION).and_then(|value| value.to_str().ok()) {
        // Apply every known upstream->incoming mapping, not just the matched
        // route's, so a redirect to a sibling upstream domain (e.g. an
        // api.bgm.tv response redirecting to lain.bgm.tv) is rewritten to its
        // mirror domain instead of leaking the upstream host.
        let mut rewritten = value.to_string();
        for (from, to) in rewrite_pairs {
            rewritten = rewrite_header_value(&rewritten, from, to);
        }
        if let Ok(value) = HeaderValue::from_str(&rewritten) {
            headers.insert(LOCATION, value);
        }
    }

    let upstream_suffix = registrable_suffix(&route.upstream_host).to_string();
    let incoming_suffix = registrable_suffix(&route.incoming_host).to_string();
    let cookies: Vec<_> = headers
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(|value| rewrite_cookie_domain(value, &upstream_suffix, &incoming_suffix))
        .collect();
    if !cookies.is_empty() {
        headers.remove(SET_COOKIE);
        for cookie in cookies {
            if let Ok(value) = HeaderValue::from_str(&cookie) {
                headers.append(SET_COOKIE, value);
            }
        }
    }
}

/// Attempt decompression based on magic bytes when Content-Encoding is absent.
/// Some CDNs return compressed content without a Content-Encoding header.
/// Returns the decompressed bytes, or the original bytes if no known signature
/// is found.
fn decompress_by_magic(bytes: &[u8]) -> Vec<u8> {
    // gzip magic: 1f 8b
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        let mut decoder = GzDecoder::new(bytes);
        let mut out = Vec::new();
        if decoder.read_to_end(&mut out).is_ok() {
            return out;
        }
    }
    // zlib magic: 78 followed by 01, 5e, 9c, or da
    if bytes.len() >= 2 && bytes[0] == 0x78 && matches!(bytes[1], 0x01 | 0x5e | 0x9c | 0xda) {
        let mut decoder = DeflateDecoder::new(bytes);
        let mut out = Vec::new();
        if decoder.read_to_end(&mut out).is_ok() {
            return out;
        }
    }
    // brotli: the first nibble of a brotli stream encodes the WBITS window
    // size. Valid first bytes are in the range 0x00-0x0F for the meta-block
    // header, but we avoid false positives by only trying brotli if the data
    // is NOT valid UTF-8 (compressed binary almost never is).
    if !bytes.is_empty() && std::str::from_utf8(bytes).is_err() {
        let mut reader: &[u8] = bytes;
        let mut out = Vec::new();
        if brotli::BrotliDecompress(&mut reader, &mut out).is_ok() && !out.is_empty() {
            return out;
        }
    }
    bytes.to_vec()
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
        && headers
            .get(CONNECTION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .to_ascii_lowercase()
                    .split(',')
                    .any(|part| part.trim() == "upgrade")
            })
}

fn remove_hop_by_hop_headers(headers: &mut HeaderMap, is_upgrade: bool) {
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
    ] {
        headers.remove(name);
    }
    if !is_upgrade {
        headers.remove("upgrade");
    }
}
