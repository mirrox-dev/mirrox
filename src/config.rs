use crate::error::AppError;
use anyhow::Context;
use serde::Deserialize;
use std::{fs, path::Path};

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ServerMode {
    #[default]
    BehindProxy,
    Direct,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DnsMode {
    Udp,
    Tcp,
    Dot,
    #[default]
    Doh,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BodyRewriteMode {
    #[default]
    Enabled,
    HttpOnly,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub dns: DnsConfig,
    #[serde(default)]
    pub rewrite: RewriteConfig,
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
    #[serde(default)]
    pub wildcard_routes: Vec<WildcardRouteConfig>,
    #[serde(default)]
    pub upstream_proxy: UpstreamProxyConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpstreamProxyConfig {
    pub default: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default)]
    pub mode: ServerMode,
    #[serde(default = "default_http_listen")]
    pub http_listen: String,
    #[serde(default = "default_https_listen")]
    pub https_listen: String,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            mode: ServerMode::BehindProxy,
            http_listen: default_http_listen(),
            https_listen: default_https_listen(),
            tls_cert: None,
            tls_key: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DnsConfig {
    #[serde(default)]
    pub mode: DnsMode,
    #[serde(default = "default_dns_servers")]
    pub servers: Vec<String>,
    #[serde(default = "default_dns_min_ttl")]
    pub cache_min_ttl_seconds: u64,
    #[serde(default = "default_dns_max_ttl")]
    pub cache_max_ttl_seconds: u64,
    #[serde(default = "default_dns_timeout")]
    pub timeout_ms: u64,
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            mode: DnsMode::Doh,
            servers: default_dns_servers(),
            cache_min_ttl_seconds: default_dns_min_ttl(),
            cache_max_ttl_seconds: default_dns_max_ttl(),
            timeout_ms: default_dns_timeout(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RewriteConfig {
    #[serde(default)]
    pub body: BodyRewriteMode,
    #[serde(default = "default_max_buffer_bytes")]
    pub max_buffer_bytes: usize,
}

impl Default for RewriteConfig {
    fn default() -> Self {
        Self {
            body: BodyRewriteMode::Enabled,
            max_buffer_bytes: default_max_buffer_bytes(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouteConfig {
    pub incoming: String,
    pub upstream: String,
    pub body_rewrite: Option<BodyRewriteMode>,
    pub upstream_proxy: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WildcardRouteConfig {
    pub incoming_suffix: String,
    pub upstream_suffix: String,
    pub body_rewrite: Option<BodyRewriteMode>,
    pub upstream_proxy: Option<String>,
}

impl AppConfig {
    pub fn load_from_env() -> anyhow::Result<Self> {
        Self::load_from_path_or_env(None)
    }

    pub fn load_from_path_or_env(path: Option<&Path>) -> anyhow::Result<Self> {
        let env_path = std::env::var("MIRROX_CONFIG").ok();
        let path = path
            .map(Path::to_path_buf)
            .or_else(|| env_path.map(Into::into))
            .unwrap_or_else(|| "config.toml".into());
        let text = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        Self::from_toml_str_with_env(&text).map_err(anyhow::Error::from)
    }

    pub fn from_toml_str(input: &str) -> Result<Self, AppError> {
        let mut config: Self =
            toml::from_str(input).map_err(|err| AppError::Config(err.to_string()))?;
        config.normalize();
        config.validate()?;
        Ok(config)
    }

    pub fn from_toml_str_with_env(input: &str) -> Result<Self, AppError> {
        let mut config = Self::from_toml_str(input)?;
        if let Ok(listen) = std::env::var("MIRROX_LISTEN") {
            config.server.listen = listen;
        }
        if let Ok(mode) = std::env::var("MIRROX_REWRITE_BODY") {
            config.rewrite.body = parse_body_rewrite_mode(&mode)?;
        }
        if let Ok(servers) = std::env::var("MIRROX_DNS_SERVERS") {
            config.dns.servers = servers
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect();
        }
        if let Ok(proxy) = std::env::var("MIRROX_UPSTREAM_PROXY") {
            config.upstream_proxy.default = normalize_proxy_override(&proxy);
        }
        config.validate()?;
        Ok(config)
    }

    fn normalize(&mut self) {
        for route in &mut self.routes {
            route.incoming = normalize_host(&route.incoming);
            route.upstream = normalize_host(&route.upstream);
        }
        for route in &mut self.wildcard_routes {
            route.incoming_suffix = normalize_suffix(&route.incoming_suffix);
            route.upstream_suffix = normalize_suffix(&route.upstream_suffix);
        }
    }

    fn validate(&self) -> Result<(), AppError> {
        if self.routes.is_empty() && self.wildcard_routes.is_empty() {
            return Err(AppError::Config(
                "at least one route or wildcard route is required".into(),
            ));
        }
        if self.dns.servers.is_empty() {
            return Err(AppError::Config(
                "at least one DNS server is required".into(),
            ));
        }
        if self.dns.cache_min_ttl_seconds > self.dns.cache_max_ttl_seconds {
            return Err(AppError::Config(
                "dns cache_min_ttl_seconds cannot exceed cache_max_ttl_seconds".into(),
            ));
        }
        if let Some(proxy) = &self.upstream_proxy.default {
            validate_upstream_proxy_value(proxy)?;
        }
        for route in &self.routes {
            if let Some(proxy) = &route.upstream_proxy {
                validate_upstream_proxy_value(proxy)?;
            }
        }
        for route in &self.wildcard_routes {
            if let Some(proxy) = &route.upstream_proxy {
                validate_upstream_proxy_value(proxy)?;
            }
        }
        Ok(())
    }
}

pub fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn normalize_suffix(suffix: &str) -> String {
    let normalized = normalize_host(suffix);
    if normalized.starts_with('.') {
        normalized
    } else {
        format!(".{normalized}")
    }
}

fn parse_body_rewrite_mode(value: &str) -> Result<BodyRewriteMode, AppError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "enabled" => Ok(BodyRewriteMode::Enabled),
        "http-only" => Ok(BodyRewriteMode::HttpOnly),
        other => Err(AppError::Config(format!(
            "invalid body rewrite mode: {other}"
        ))),
    }
}

fn normalize_proxy_override(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("direct") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn validate_upstream_proxy_value(value: &str) -> Result<(), AppError> {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("direct") {
        return Ok(());
    }
    let url = url::Url::parse(trimmed)
        .map_err(|err| AppError::Config(format!("invalid upstream proxy {trimmed}: {err}")))?;
    match url.scheme() {
        "http" | "socks5" => Ok(()),
        scheme => Err(AppError::Config(format!(
            "invalid upstream proxy scheme: {scheme}"
        ))),
    }
}

fn default_listen() -> String {
    "127.0.0.1:3000".into()
}
fn default_http_listen() -> String {
    "0.0.0.0:80".into()
}
fn default_https_listen() -> String {
    "0.0.0.0:443".into()
}
fn default_dns_servers() -> Vec<String> {
    vec!["https://cloudflare-dns.com/dns-query".into()]
}
fn default_dns_min_ttl() -> u64 {
    30
}
fn default_dns_max_ttl() -> u64 {
    300
}
fn default_dns_timeout() -> u64 {
    2000
}
fn default_max_buffer_bytes() -> usize {
    2 * 1024 * 1024
}
