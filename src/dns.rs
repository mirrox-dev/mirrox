use crate::config::{DnsConfig, DnsMode};
use crate::error::{AppError, Result};
use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::Arc;

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
        Ok(Arc::new(HickoryDnsResolver::new(self.config)?))
    }
}

pub struct HickoryDnsResolver {
    _config: DnsConfig,
}

impl HickoryDnsResolver {
    pub fn new(config: DnsConfig) -> Result<Self> {
        match config.mode {
            DnsMode::Udp | DnsMode::Tcp | DnsMode::Dot | DnsMode::Doh => {
                Ok(Self { _config: config })
            }
        }
    }
}

#[async_trait]
impl DnsResolver for HickoryDnsResolver {
    async fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>> {
        let lookup = tokio::net::lookup_host((host, port))
            .await
            .map_err(|source| AppError::Dns {
                host: host.to_string(),
                source: anyhow::Error::new(source),
                incoming_host: String::new(),
                language: String::new(),
            })?;
        let addresses: Vec<_> = lookup.collect();
        if addresses.is_empty() {
            return Err(AppError::Dns {
                host: host.to_string(),
                source: anyhow::anyhow!("no addresses returned"),
                incoming_host: String::new(),
                language: String::new(),
            });
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
    fn accepts_doh_config_shape() {
        let config = DnsConfig {
            mode: DnsMode::Doh,
            servers: vec!["https://cloudflare-dns.com/dns-query".to_string()],
            cache_min_ttl_seconds: 30,
            cache_max_ttl_seconds: 300,
            timeout_ms: 2000,
        };

        assert!(ResolverFactory::new(config).is_ok());
    }
}
