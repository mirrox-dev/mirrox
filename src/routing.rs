use crate::config::{normalize_host, AppConfig, BodyRewriteMode};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedRoute {
    pub incoming_host: String,
    pub upstream_host: String,
    pub body_rewrite: BodyRewriteMode,
    pub upstream_proxy: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RouteTable {
    exact: HashMap<String, MatchedRoute>,
    wildcard: Vec<WildcardRule>,
}

#[derive(Debug, Clone)]
struct WildcardRule {
    incoming_suffix: String,
    upstream_suffix: String,
    body_rewrite: BodyRewriteMode,
    upstream_proxy: Option<String>,
}

impl RouteTable {
    pub fn from_config(config: &AppConfig) -> Self {
        let exact = config
            .routes
            .iter()
            .map(|route| {
                let rewrite = route
                    .body_rewrite
                    .clone()
                    .unwrap_or_else(|| config.rewrite.body.clone());
                let matched = MatchedRoute {
                    incoming_host: route.incoming.clone(),
                    upstream_host: route.upstream.clone(),
                    body_rewrite: rewrite,
                    upstream_proxy: route
                        .upstream_proxy
                        .clone()
                        .or_else(|| config.upstream_proxy.default.clone()),
                };
                (route.incoming.clone(), matched)
            })
            .collect();

        let wildcard = config
            .wildcard_routes
            .iter()
            .map(|route| WildcardRule {
                incoming_suffix: route.incoming_suffix.clone(),
                upstream_suffix: route.upstream_suffix.clone(),
                body_rewrite: route
                    .body_rewrite
                    .clone()
                    .unwrap_or_else(|| config.rewrite.body.clone()),
                upstream_proxy: route
                    .upstream_proxy
                    .clone()
                    .or_else(|| config.upstream_proxy.default.clone()),
            })
            .collect();

        Self { exact, wildcard }
    }

    pub fn match_host(&self, host: &str) -> Option<MatchedRoute> {
        let host = normalize_request_host(host)?;
        if let Some(route) = self.exact.get(&host) {
            return Some(route.clone());
        }

        self.wildcard.iter().find_map(|rule| {
            let prefix = host.strip_suffix(&rule.incoming_suffix)?;
            if prefix.is_empty() || prefix.contains('.') {
                return None;
            }
            Some(MatchedRoute {
                incoming_host: host.clone(),
                upstream_host: format!("{}{}", prefix, rule.upstream_suffix),
                body_rewrite: rule.body_rewrite.clone(),
                upstream_proxy: rule.upstream_proxy.clone(),
            })
        })
    }
}

pub fn normalize_request_host(host: &str) -> Option<String> {
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    let without_port = host
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']').map(|(inside, _)| inside))
        .map(str::to_string)
        .unwrap_or_else(|| host.split(':').next().unwrap_or(host).to_string());
    Some(normalize_host(&without_port))
}
