use crate::config::{normalize_host, AppConfig, BodyRewriteMode, UpstreamScheme};
use crate::rewrite::{RewritePlan, RewriteRule};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedRoute {
    pub incoming_host: String,
    pub upstream_host: String,
    pub body_rewrite: BodyRewriteMode,
    pub upstream_proxy: Option<String>,
    pub upstream_scheme: UpstreamScheme,
    pub upstream_port: u16,
    pub user_agent: Option<String>,
    /// Resolved scripts: global scripts followed by route-specific scripts.
    pub scripts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RouteTable {
    exact: HashMap<String, MatchedRoute>,
    ordered_exact: Vec<MatchedRoute>,
    wildcard: Vec<WildcardRule>,
    global_scripts: Vec<String>,
}

#[derive(Debug, Clone)]
struct WildcardRule {
    incoming_suffix: String,
    upstream_suffix: String,
    body_rewrite: BodyRewriteMode,
    upstream_proxy: Option<String>,
    upstream_scheme: UpstreamScheme,
    upstream_port: u16,
    user_agent: Option<String>,
    /// Route-specific scripts for this wildcard rule.
    scripts: Vec<String>,
}

impl RouteTable {
    pub fn from_config(config: &AppConfig) -> Self {
        let global_scripts = &config.scripts.global;

        let ordered_exact: Vec<_> = config
            .routes
            .iter()
            .map(|route| {
                let body_rewrite = route
                    .body_rewrite
                    .clone()
                    .unwrap_or_else(|| config.rewrite.body.clone());
                let upstream_port = route
                    .upstream_port
                    .unwrap_or_else(|| route.upstream_scheme.default_port());
                let mut scripts = global_scripts.clone();
                scripts.extend(route.scripts.iter().cloned());

                MatchedRoute {
                    incoming_host: route.incoming.clone(),
                    upstream_host: route.upstream.clone(),
                    body_rewrite,
                    upstream_proxy: route
                        .upstream_proxy
                        .clone()
                        .or_else(|| config.upstream_proxy.default.clone()),
                    upstream_scheme: route.upstream_scheme.clone(),
                    upstream_port,
                    user_agent: route.user_agent.clone(),
                    scripts,
                }
            })
            .collect();
        let exact = ordered_exact
            .iter()
            .cloned()
            .map(|route| (route.incoming_host.clone(), route))
            .collect();

        let wildcard = config
            .wildcard_routes
            .iter()
            .map(|route| {
                let upstream_port = route
                    .upstream_port
                    .unwrap_or_else(|| route.upstream_scheme.default_port());
                WildcardRule {
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
                    upstream_scheme: route.upstream_scheme.clone(),
                    upstream_port,
                    user_agent: route.user_agent.clone(),
                    scripts: route.scripts.clone(),
                }
            })
            .collect();

        Self {
            exact,
            ordered_exact,
            wildcard,
            global_scripts: global_scripts.clone(),
        }
    }

    /// Returns one deterministic `(upstream, incoming)` pair per exact
    /// upstream. Shared upstreams select the lexicographically smallest
    /// normalized incoming host.
    pub fn all_rewrite_pairs(&self) -> Vec<(String, String)> {
        sorted_pairs(self.lexical_rewrite_targets())
    }

    /// Builds the response rewrite plan for a matched route.
    ///
    /// The current route always wins for its own upstream. Ambiguous mappings
    /// for sibling upstreams use their lexicographically smallest incoming host.
    pub fn rewrite_plan_for(&self, current: &MatchedRoute) -> RewritePlan {
        let mut selected = self.lexical_rewrite_targets();
        selected.insert(current.upstream_host.clone(), current.incoming_host.clone());

        RewritePlan::new(
            selected
                .into_iter()
                .map(|(from, to)| RewriteRule::new(from, to))
                .collect(),
        )
    }

    /// Compatibility view of [`Self::rewrite_plan_for`].
    pub fn rewrite_pairs_for(&self, route: &MatchedRoute) -> Vec<(String, String)> {
        self.rewrite_plan_for(route)
            .rules()
            .iter()
            .map(|rule| (rule.from().to_string(), rule.to().to_string()))
            .collect()
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
            let mut scripts = self.global_scripts.clone();
            scripts.extend(rule.scripts.iter().cloned());
            Some(MatchedRoute {
                incoming_host: host.clone(),
                upstream_host: format!("{}{}", prefix, rule.upstream_suffix),
                body_rewrite: rule.body_rewrite.clone(),
                upstream_proxy: rule.upstream_proxy.clone(),
                upstream_scheme: rule.upstream_scheme.clone(),
                upstream_port: rule.upstream_port,
                user_agent: rule.user_agent.clone(),
                scripts,
            })
        })
    }

    fn lexical_rewrite_targets(&self) -> BTreeMap<String, String> {
        let mut selected = BTreeMap::new();
        for route in &self.ordered_exact {
            selected
                .entry(route.upstream_host.clone())
                .and_modify(|incoming: &mut String| {
                    if route.incoming_host < *incoming {
                        incoming.clone_from(&route.incoming_host);
                    }
                })
                .or_insert_with(|| route.incoming_host.clone());
        }
        selected
    }
}

fn sorted_pairs(targets: BTreeMap<String, String>) -> Vec<(String, String)> {
    let mut pairs: Vec<_> = targets.into_iter().collect();
    pairs.sort_by(|a, b| {
        b.0.len()
            .cmp(&a.0.len())
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.1.cmp(&b.1))
    });
    pairs
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
