use crate::config::{DnsConfig, DnsMode};
use crate::error::{AppError, Result};
use async_trait::async_trait;
use hickory_resolver::config::{
    NameServerConfig, NameServerConfigGroup, Protocol, ResolverConfig, ResolverOpts,
};
use hickory_resolver::TokioAsyncResolver;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

#[async_trait]
pub trait DnsResolver: Send + Sync + 'static {
    async fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>>;
}

pub type SharedDnsResolver = Arc<dyn DnsResolver>;

#[derive(Debug, Clone)]
pub struct StaticResolver {
    addresses: Vec<SocketAddr>,
}

impl StaticResolver {
    pub fn new(addresses: Vec<SocketAddr>) -> Self {
        Self { addresses }
    }
}

#[async_trait]
impl DnsResolver for StaticResolver {
    async fn resolve(&self, _host: &str, _port: u16) -> Result<Vec<SocketAddr>> {
        Ok(self.addresses.clone())
    }
}

pub struct ResolverFactory {
    config: DnsConfig,
}

impl ResolverFactory {
    pub fn new(config: DnsConfig) -> Result<Self> {
        if config.servers.is_empty() {
            return Err(AppError::Config(
                "at least one DNS server is required".into(),
            ));
        }
        Ok(Self { config })
    }

    pub fn build(self) -> Result<SharedDnsResolver> {
        let resolver = build_hickory_resolver(&self.config)?;
        Ok(Arc::new(HickoryDnsResolver { resolver }))
    }
}

pub struct HickoryDnsResolver {
    resolver: TokioAsyncResolver,
}

#[derive(Debug)]
struct ParsedServer {
    socket_addr: SocketAddr,
    protocol: Protocol,
    tls_dns_name: Option<String>,
}

fn resolve_host_port(host: &str, port: u16) -> Result<SocketAddr> {
    if let Ok(ip) = host.parse() {
        return Ok(SocketAddr::new(ip, port));
    }
    (host, port)
        .to_socket_addrs()
        .map_err(|err| AppError::Config(format!("failed to resolve DNS server {host}: {err}")))?
        .next()
        .ok_or_else(|| AppError::Config(format!("no addresses for DNS server {host}")))
}

fn parse_server(value: &str, mode: &DnsMode) -> Result<ParsedServer> {
    if let Ok(url) = url::Url::parse(value) {
        let expected_scheme = match mode {
            DnsMode::Doh => "https",
            DnsMode::Dot => "tls",
            DnsMode::Udp => "udp",
            DnsMode::Tcp => "tcp",
        };
        if url.scheme() != expected_scheme {
            return Err(AppError::Config(format!(
                "DNS mode {mode:?} requires {expected_scheme}:// servers, got {value}"
            )));
        }
        if matches!(mode, DnsMode::Doh) && url.path() != "/dns-query" {
            return Err(AppError::Config(format!(
                "DNS-over-HTTPS server must use the /dns-query endpoint: {value}"
            )));
        }
        let host = url
            .host_str()
            .ok_or_else(|| AppError::Config(format!("missing host in DNS server URL: {value}")))?;
        let port = url.port().unwrap_or(match mode {
            DnsMode::Dot => 853,
            DnsMode::Doh => 443,
            DnsMode::Udp | DnsMode::Tcp => 53,
        });
        let protocol = protocol_for_mode(mode);
        return Ok(ParsedServer {
            socket_addr: resolve_host_port(host, port)?,
            protocol,
            tls_dns_name: encrypted_dns_name(mode, host),
        });
    }

    let socket_addr: SocketAddr = value
        .parse()
        .map_err(|err| AppError::Config(format!("invalid DNS server {value}: {err}")))?;
    Ok(ParsedServer {
        socket_addr,
        protocol: protocol_for_mode(mode),
        tls_dns_name: encrypted_dns_name(mode, &socket_addr.ip().to_string()),
    })
}

fn protocol_for_mode(mode: &DnsMode) -> Protocol {
    match mode {
        DnsMode::Udp => Protocol::Udp,
        DnsMode::Tcp => Protocol::Tcp,
        DnsMode::Dot => Protocol::Tls,
        DnsMode::Doh => Protocol::Https,
    }
}

fn encrypted_dns_name(mode: &DnsMode, host: &str) -> Option<String> {
    matches!(mode, DnsMode::Doh | DnsMode::Dot).then(|| host.to_string())
}

fn resolver_options(config: &DnsConfig) -> ResolverOpts {
    let mut options = ResolverOpts::default();
    options.timeout = Duration::from_millis(config.timeout_ms);
    options.positive_min_ttl = Some(Duration::from_secs(config.cache_min_ttl_seconds));
    options.positive_max_ttl = Some(Duration::from_secs(config.cache_max_ttl_seconds));
    options.negative_min_ttl = Some(Duration::from_secs(config.cache_min_ttl_seconds));
    options.negative_max_ttl = Some(Duration::from_secs(config.cache_max_ttl_seconds));
    options
}

fn build_hickory_resolver(config: &DnsConfig) -> Result<TokioAsyncResolver> {
    let mut group = NameServerConfigGroup::new();
    for server in &config.servers {
        let parsed = parse_server(server, &config.mode)?;
        let mut ns_config = NameServerConfig::new(parsed.socket_addr, parsed.protocol);
        if let Some(tls_name) = parsed.tls_dns_name {
            ns_config.tls_dns_name = Some(tls_name);
        }
        group.push(ns_config);
    }

    let resolver_config = ResolverConfig::from_parts(None, vec![], group);
    let resolver = TokioAsyncResolver::tokio(resolver_config, resolver_options(config));
    Ok(resolver)
}

#[async_trait]
impl DnsResolver for HickoryDnsResolver {
    async fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>> {
        let lookups = self
            .resolver
            .lookup_ip(host)
            .await
            .map_err(|source| AppError::dns(host, anyhow::Error::new(source)))?;
        let addresses: Vec<_> = lookups.iter().map(|ip| SocketAddr::new(ip, port)).collect();
        if addresses.is_empty() {
            return Err(AppError::dns(
                host,
                anyhow::anyhow!("no addresses returned for {host}"),
            ));
        }
        Ok(addresses)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DnsConfig, DnsMode};

    #[tokio::test]
    async fn mock_resolver_returns_configured_addresses() {
        let resolver = StaticResolver::new(vec!["127.0.0.1:443".parse().unwrap()]);
        let addresses = resolver.resolve("www.bgm.tv", 443).await.unwrap();
        assert_eq!(addresses[0].to_string(), "127.0.0.1:443");
    }

    #[test]
    fn accepts_doh_config_with_url() {
        let config = DnsConfig {
            mode: DnsMode::Doh,
            servers: vec!["https://cloudflare-dns.com/dns-query".to_string()],
            cache_min_ttl_seconds: 30,
            cache_max_ttl_seconds: 300,
            timeout_ms: 2000,
        };

        assert!(ResolverFactory::new(config).is_ok());
    }

    #[test]
    fn accepts_udp_config_with_ip() {
        let config = DnsConfig {
            mode: DnsMode::Udp,
            servers: vec!["8.8.8.8:53".to_string()],
            ..Default::default()
        };
        let factory = ResolverFactory::new(config).unwrap();
        assert!(factory.build().is_ok());
    }

    #[test]
    fn parses_doh_server_with_expected_protocol_and_tls_name() {
        let parsed = parse_server("https://127.0.0.1/dns-query", &DnsMode::Doh).unwrap();

        assert_eq!(parsed.socket_addr, "127.0.0.1:443".parse().unwrap());
        assert_eq!(parsed.protocol, Protocol::Https);
        assert_eq!(parsed.tls_dns_name.as_deref(), Some("127.0.0.1"));
    }

    #[test]
    fn rejects_dns_url_scheme_that_does_not_match_mode() {
        let error = parse_server("tls://127.0.0.1", &DnsMode::Doh).unwrap_err();
        assert!(error.to_string().contains("requires https://"));
    }

    #[test]
    fn configured_timeout_and_ttls_populate_resolver_options() {
        let config = DnsConfig {
            timeout_ms: 1750,
            cache_min_ttl_seconds: 45,
            cache_max_ttl_seconds: 420,
            ..Default::default()
        };

        let options = resolver_options(&config);
        assert_eq!(options.timeout, Duration::from_millis(1750));
        assert_eq!(options.positive_min_ttl, Some(Duration::from_secs(45)));
        assert_eq!(options.positive_max_ttl, Some(Duration::from_secs(420)));
        assert_eq!(options.negative_min_ttl, Some(Duration::from_secs(45)));
        assert_eq!(options.negative_max_ttl, Some(Duration::from_secs(420)));
    }
}
