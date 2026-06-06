use crate::dns::SharedDnsResolver;
use crate::error::AppError;
use base64::Engine;
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::TokioIo;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::{client::TlsStream, TlsConnector};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyTarget {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyMode {
    Direct,
    HttpConnect(ProxyTarget),
    Socks5(ProxyTarget),
}

impl ProxyMode {
    pub fn parse(value: Option<&str>) -> Result<Self, AppError> {
        let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(Self::Direct);
        };
        if value.eq_ignore_ascii_case("direct") {
            return Ok(Self::Direct);
        }

        let url = url::Url::parse(value)
            .map_err(|err| AppError::Config(format!("invalid upstream proxy {value}: {err}")))?;
        let host = url
            .host_str()
            .ok_or_else(|| {
                AppError::Config(format!("invalid upstream proxy {value}: missing host"))
            })?
            .to_string();
        let port = url.port_or_known_default().ok_or_else(|| {
            AppError::Config(format!("invalid upstream proxy {value}: missing port"))
        })?;
        let username = if url.username().is_empty() {
            None
        } else {
            Some(url.username().to_string())
        };
        let password = url.password().map(ToOwned::to_owned);
        let target = ProxyTarget {
            host,
            port,
            username,
            password,
        };

        match url.scheme() {
            "http" => Ok(Self::HttpConnect(target)),
            "socks5" => Ok(Self::Socks5(target)),
            scheme => Err(AppError::Config(format!(
                "invalid upstream proxy scheme: {scheme}"
            ))),
        }
    }
}

#[derive(Clone)]
pub struct UpstreamConnector {
    dns: SharedDnsResolver,
}

impl UpstreamConnector {
    pub fn new(dns: SharedDnsResolver) -> Self {
        Self { dns }
    }

    pub async fn connect(
        &self,
        proxy: Option<&str>,
        upstream_host: &str,
        upstream_port: u16,
    ) -> Result<TcpStream, AppError> {
        match ProxyMode::parse(proxy)? {
            ProxyMode::Direct => self.connect_direct(upstream_host, upstream_port).await,
            ProxyMode::HttpConnect(proxy) => {
                connect_http_proxy(proxy, upstream_host, upstream_port).await
            }
            ProxyMode::Socks5(proxy) => {
                connect_socks5_proxy(proxy, upstream_host, upstream_port).await
            }
        }
    }

    async fn connect_direct(&self, host: &str, port: u16) -> Result<TcpStream, AppError> {
        let addresses = self.dns.resolve(host, port).await?;
        TcpStream::connect(addresses[0])
            .await
            .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))
    }
}

pub enum UpstreamStream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl AsyncRead for UpstreamStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Plain(stream) => std::pin::Pin::new(stream).poll_read(cx, buf),
            Self::Tls(stream) => std::pin::Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for UpstreamStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match &mut *self {
            Self::Plain(stream) => std::pin::Pin::new(stream).poll_write(cx, buf),
            Self::Tls(stream) => std::pin::Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Plain(stream) => std::pin::Pin::new(stream).poll_flush(cx),
            Self::Tls(stream) => std::pin::Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Plain(stream) => std::pin::Pin::new(stream).poll_shutdown(cx),
            Self::Tls(stream) => std::pin::Pin::new(stream).poll_shutdown(cx),
        }
    }
}

impl Unpin for UpstreamStream {}

impl Connection for UpstreamStream {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}

pub type UpstreamIo = TokioIo<UpstreamStream>;

pub fn boxed_body_io(stream: UpstreamStream) -> UpstreamIo {
    TokioIo::new(stream)
}

fn tls_root_store() -> Result<RootCertStore, AppError> {
    let mut root_store = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    if let Ok(path) = std::env::var("MIRROX_EXTRA_ROOT_CERT_DER") {
        let bytes = std::fs::read(path)
            .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))?;
        root_store
            .add(bytes.into())
            .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))?;
    }
    Ok(root_store)
}

pub async fn tls_stream(
    stream: TcpStream,
    upstream_host: &str,
) -> Result<UpstreamStream, AppError> {
    let root_store = tls_root_store()?;
    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let server_name = ServerName::try_from(upstream_host.to_string()).map_err(|err| {
        AppError::Upstream(
            anyhow::anyhow!("invalid TLS server name {upstream_host}: {err}"),
            String::new(),
        )
    })?;
    connector
        .connect(server_name, stream)
        .await
        .map(|stream| UpstreamStream::Tls(Box::new(stream)))
        .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))
}

pub async fn maybe_tls_stream(
    stream: TcpStream,
    upstream_host: &str,
    use_tls: bool,
) -> Result<UpstreamStream, AppError> {
    if use_tls {
        tls_stream(stream, upstream_host).await
    } else {
        Ok(UpstreamStream::Plain(stream))
    }
}

async fn connect_proxy_tcp(proxy: &ProxyTarget) -> Result<TcpStream, AppError> {
    let addr = format!("{}:{}", proxy.host, proxy.port);
    TcpStream::connect(addr)
        .await
        .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))
}

async fn connect_http_proxy(
    proxy: ProxyTarget,
    upstream_host: &str,
    upstream_port: u16,
) -> Result<TcpStream, AppError> {
    let mut stream = connect_proxy_tcp(&proxy).await?;
    let authority = format!("{upstream_host}:{upstream_port}");
    let mut request = format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Connection: Keep-Alive\r\n"
    );
    if let Some(username) = &proxy.username {
        let password = proxy.password.as_deref().unwrap_or("");
        let credentials =
            base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
        request.push_str(&format!("Proxy-Authorization: Basic {credentials}\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))?;

    let mut response = Vec::new();
    let mut byte = [0_u8; 1];
    while !response.ends_with(b"\r\n\r\n") {
        let read = stream
            .read(&mut byte)
            .await
            .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))?;
        if read == 0 {
            return Err(AppError::Upstream(
                anyhow::anyhow!("http proxy closed during CONNECT"),
                String::new(),
            ));
        }
        response.push(byte[0]);
        if response.len() > 8192 {
            return Err(AppError::Upstream(
                anyhow::anyhow!("http proxy CONNECT response too large"),
                String::new(),
            ));
        }
    }
    let response = String::from_utf8_lossy(&response);
    if response.starts_with("HTTP/1.1 2") || response.starts_with("HTTP/1.0 2") {
        Ok(stream)
    } else {
        Err(AppError::Upstream(
            anyhow::anyhow!("http proxy CONNECT failed: {response}"),
            String::new(),
        ))
    }
}

async fn connect_socks5_proxy(
    proxy: ProxyTarget,
    upstream_host: &str,
    upstream_port: u16,
) -> Result<TcpStream, AppError> {
    let mut stream = connect_proxy_tcp(&proxy).await?;
    let wants_auth = proxy.username.is_some();
    let greeting: &[u8] = if wants_auth {
        &[0x05, 0x02, 0x00, 0x02]
    } else {
        &[0x05, 0x01, 0x00]
    };
    stream
        .write_all(greeting)
        .await
        .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))?;
    let mut method = [0_u8; 2];
    stream
        .read_exact(&mut method)
        .await
        .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))?;
    if method[0] != 0x05 {
        return Err(AppError::Upstream(
            anyhow::anyhow!("invalid SOCKS5 proxy response"),
            String::new(),
        ));
    }
    match method[1] {
        0x00 => {}
        0x02 => authenticate_socks5(&mut stream, &proxy).await?,
        0xff => {
            return Err(AppError::Upstream(
                anyhow::anyhow!("SOCKS5 proxy rejected auth methods"),
                String::new(),
            ))
        }
        other => {
            return Err(AppError::Upstream(
                anyhow::anyhow!("unsupported SOCKS5 auth method {other}"),
                String::new(),
            ))
        }
    }

    let host = upstream_host.as_bytes();
    if host.len() > u8::MAX as usize {
        return Err(AppError::Upstream(
            anyhow::anyhow!("SOCKS5 upstream host too long"),
            String::new(),
        ));
    }
    let mut request = Vec::with_capacity(7 + host.len());
    request.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, host.len() as u8]);
    request.extend_from_slice(host);
    request.extend_from_slice(&upstream_port.to_be_bytes());
    stream
        .write_all(&request)
        .await
        .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))?;

    let mut header = [0_u8; 4];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))?;
    if header[0] != 0x05 || header[1] != 0x00 {
        return Err(AppError::Upstream(
            anyhow::anyhow!("SOCKS5 CONNECT failed"),
            String::new(),
        ));
    }
    read_socks5_bound_address(&mut stream, header[3]).await?;
    Ok(stream)
}

async fn authenticate_socks5(stream: &mut TcpStream, proxy: &ProxyTarget) -> Result<(), AppError> {
    let username = proxy.username.as_deref().unwrap_or("").as_bytes();
    let password = proxy.password.as_deref().unwrap_or("").as_bytes();
    if username.len() > u8::MAX as usize || password.len() > u8::MAX as usize {
        return Err(AppError::Upstream(
            anyhow::anyhow!("SOCKS5 credentials too long"),
            String::new(),
        ));
    }
    let mut request = Vec::with_capacity(3 + username.len() + password.len());
    request.push(0x01);
    request.push(username.len() as u8);
    request.extend_from_slice(username);
    request.push(password.len() as u8);
    request.extend_from_slice(password);
    stream
        .write_all(&request)
        .await
        .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))?;
    let mut response = [0_u8; 2];
    stream
        .read_exact(&mut response)
        .await
        .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))?;
    if response == [0x01, 0x00] {
        Ok(())
    } else {
        Err(AppError::Upstream(
            anyhow::anyhow!("SOCKS5 authentication failed"),
            String::new(),
        ))
    }
}

async fn read_socks5_bound_address(stream: &mut TcpStream, atyp: u8) -> Result<(), AppError> {
    match atyp {
        0x01 => {
            let mut ignored = [0_u8; 6];
            stream
                .read_exact(&mut ignored)
                .await
                .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))?;
        }
        0x03 => {
            let mut len = [0_u8; 1];
            stream
                .read_exact(&mut len)
                .await
                .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))?;
            let mut ignored = vec![0_u8; len[0] as usize + 2];
            stream
                .read_exact(&mut ignored)
                .await
                .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))?;
        }
        0x04 => {
            let mut ignored = [0_u8; 18];
            stream
                .read_exact(&mut ignored)
                .await
                .map_err(|err| AppError::Upstream(anyhow::Error::new(err), String::new()))?;
        }
        other => {
            return Err(AppError::Upstream(
                anyhow::anyhow!("unsupported SOCKS5 address type {other}"),
                String::new(),
            ))
        }
    }
    Ok(())
}
