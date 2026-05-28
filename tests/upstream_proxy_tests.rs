use mirrox::dns::StaticResolver;
use mirrox::upstream_proxy::{ProxyMode, ProxyTarget, UpstreamConnector};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn parses_direct_proxy_mode() {
    assert_eq!(ProxyMode::parse(None).unwrap(), ProxyMode::Direct);
    assert_eq!(ProxyMode::parse(Some("direct")).unwrap(), ProxyMode::Direct);
}

#[test]
fn parses_http_proxy_with_credentials() {
    let mode = ProxyMode::parse(Some("http://user:pass@127.0.0.1:8080")).unwrap();

    assert_eq!(
        mode,
        ProxyMode::HttpConnect(ProxyTarget {
            host: "127.0.0.1".into(),
            port: 8080,
            username: Some("user".into()),
            password: Some("pass".into()),
        })
    );
}

#[test]
fn parses_socks5_proxy_without_credentials() {
    let mode = ProxyMode::parse(Some("socks5://127.0.0.1:1080")).unwrap();

    assert_eq!(
        mode,
        ProxyMode::Socks5(ProxyTarget {
            host: "127.0.0.1".into(),
            port: 1080,
            username: None,
            password: None,
        })
    );
}

async fn spawn_http_connect_proxy() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut byte = [0_u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
        }
        let request = String::from_utf8(request).unwrap();
        assert!(request.contains("CONNECT api.bgm.tv:80 HTTP/1.1"));
        assert!(request.contains("Proxy-Authorization: Basic dXNlcjpwYXNz"));
        socket
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        socket.write_all(b"tunnel-ok").await.unwrap();
    });
    addr
}

#[tokio::test]
async fn connects_through_http_connect_proxy() {
    let proxy_addr = spawn_http_connect_proxy().await;
    let connector = UpstreamConnector::new(Arc::new(StaticResolver::new(vec![proxy_addr])));
    let mut stream = connector
        .connect(
            Some(&format!("http://user:pass@{proxy_addr}")),
            "api.bgm.tv",
            80,
        )
        .await
        .unwrap();
    let mut buffer = [0_u8; 9];

    stream.read_exact(&mut buffer).await.unwrap();

    assert_eq!(&buffer, b"tunnel-ok");
}

async fn spawn_socks5_proxy() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut greeting = [0_u8; 4];
        socket.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [0x05, 0x02, 0x00, 0x02]);
        socket.write_all(&[0x05, 0x02]).await.unwrap();

        let mut auth_prefix = [0_u8; 2];
        socket.read_exact(&mut auth_prefix).await.unwrap();
        assert_eq!(auth_prefix, [0x01, 0x04]);
        let mut username = [0_u8; 4];
        socket.read_exact(&mut username).await.unwrap();
        assert_eq!(&username, b"user");
        let mut password_len = [0_u8; 1];
        socket.read_exact(&mut password_len).await.unwrap();
        assert_eq!(password_len, [0x04]);
        let mut password = [0_u8; 4];
        socket.read_exact(&mut password).await.unwrap();
        assert_eq!(&password, b"pass");
        socket.write_all(&[0x01, 0x00]).await.unwrap();

        let mut connect_prefix = [0_u8; 5];
        socket.read_exact(&mut connect_prefix).await.unwrap();
        assert_eq!(connect_prefix, [0x05, 0x01, 0x00, 0x03, 0x0a]);
        let mut host = [0_u8; 10];
        socket.read_exact(&mut host).await.unwrap();
        assert_eq!(&host, b"api.bgm.tv");
        let mut port = [0_u8; 2];
        socket.read_exact(&mut port).await.unwrap();
        assert_eq!(u16::from_be_bytes(port), 80);
        socket
            .write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0x1f, 0x90])
            .await
            .unwrap();
        socket.write_all(b"socks-ok").await.unwrap();
    });
    addr
}

#[tokio::test]
async fn connects_through_socks5_proxy_with_auth() {
    let proxy_addr = spawn_socks5_proxy().await;
    let connector = UpstreamConnector::new(Arc::new(StaticResolver::new(vec![proxy_addr])));
    let mut stream = connector
        .connect(
            Some(&format!("socks5://user:pass@{proxy_addr}")),
            "api.bgm.tv",
            80,
        )
        .await
        .unwrap();
    let mut buffer = [0_u8; 8];

    stream.read_exact(&mut buffer).await.unwrap();

    assert_eq!(&buffer, b"socks-ok");
}
