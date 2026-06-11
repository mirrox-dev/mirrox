use mirrox::config::{AppConfig, BodyRewriteMode, UpstreamScheme};
use mirrox::routing::RouteTable;

fn table() -> RouteTable {
    let config = AppConfig::from_toml_str(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [rewrite]
        body = "enabled"

        [[routes]]
        incoming = "www.example.com"
        upstream = "www.bgm.tv"
        body_rewrite = "http-only"

        [[wildcard_routes]]
        incoming_suffix = ".example.com"
        upstream_suffix = ".bgm.tv"
    "#,
    )
    .unwrap();
    RouteTable::from_config(&config)
}

#[test]
fn exact_route_carries_upstream_connection_settings_and_user_agent() {
    let config = AppConfig::from_toml_str(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_scheme = "http"
        upstream_port = 8080
        user_agent = "Mirrox-Test/1.0"
    "#,
    )
    .unwrap();
    let table = RouteTable::from_config(&config);

    let route = table
        .match_host("api.example.com")
        .expect("route should match");

    assert_eq!(route.upstream_scheme, UpstreamScheme::Http);
    assert_eq!(route.upstream_port, 8080);
    assert_eq!(route.user_agent.as_deref(), Some("Mirrox-Test/1.0"));
}

#[test]
fn wildcard_route_carries_upstream_connection_settings_and_user_agent() {
    let config = AppConfig::from_toml_str(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [[wildcard_routes]]
        incoming_suffix = ".example.com"
        upstream_suffix = ".bgm.tv"
        upstream_scheme = "http"
        upstream_port = 8080
        user_agent = "Mirrox-Wildcard/1.0"
    "#,
    )
    .unwrap();
    let table = RouteTable::from_config(&config);

    let route = table
        .match_host("api.example.com")
        .expect("route should match");

    assert_eq!(route.upstream_host, "api.bgm.tv");
    assert_eq!(route.upstream_scheme, UpstreamScheme::Http);
    assert_eq!(route.upstream_port, 8080);
    assert_eq!(route.user_agent.as_deref(), Some("Mirrox-Wildcard/1.0"));
}

#[test]
fn explicit_route_wins_over_wildcard() {
    let route = table()
        .match_host("www.example.com")
        .expect("route should match");
    assert_eq!(route.incoming_host, "www.example.com");
    assert_eq!(route.upstream_host, "www.bgm.tv");
    assert_eq!(route.body_rewrite, BodyRewriteMode::HttpOnly);
}

#[test]
fn wildcard_maps_subdomain_suffix() {
    let route = table()
        .match_host("api.example.com")
        .expect("route should match");
    assert_eq!(route.incoming_host, "api.example.com");
    assert_eq!(route.upstream_host, "api.bgm.tv");
    assert_eq!(route.body_rewrite, BodyRewriteMode::Enabled);
    assert_eq!(route.upstream_scheme, UpstreamScheme::Https);
    assert_eq!(route.upstream_port, 443);
}

#[test]
fn host_port_is_ignored_for_matching() {
    let route = table()
        .match_host("api.example.com:443")
        .expect("route should match");
    assert_eq!(route.upstream_host, "api.bgm.tv");
}

#[test]
fn unknown_host_is_rejected() {
    assert!(table().match_host("evil.example.net").is_none());
}

#[test]
fn rewrite_pairs_for_wildcard_match_include_matched_route_mapping() {
    let table = table();
    let route = table
        .match_host("api.example.com")
        .expect("route should match");

    // The wildcard expansion api.bgm.tv -> api.example.com is not in the exact
    // table, so rewrite_pairs_for must add it on top of the exact pairs.
    let pairs = table.rewrite_pairs_for(&route);

    assert!(
        pairs.contains(&("api.bgm.tv".to_string(), "api.example.com".to_string())),
        "wildcard-matched route's own mapping must be present, got: {pairs:?}"
    );
    assert!(
        pairs.contains(&("www.bgm.tv".to_string(), "www.example.com".to_string())),
        "exact route mappings must still be present, got: {pairs:?}"
    );
}

#[test]
fn rewrite_pairs_for_exact_match_have_no_duplicate() {
    let table = table();
    let route = table
        .match_host("www.example.com")
        .expect("route should match");

    // An exact match is already in the exact table, so the pair list must not
    // gain a duplicate entry for it.
    let pairs = table.rewrite_pairs_for(&route);
    let www_count = pairs
        .iter()
        .filter(|(upstream, _)| upstream == "www.bgm.tv")
        .count();

    assert_eq!(
        www_count, 1,
        "exact match must not be duplicated: {pairs:?}"
    );
}

#[test]
fn matched_route_inherits_global_upstream_proxy() {
    let config = AppConfig::from_toml_str(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [upstream_proxy]
        default = "socks5://127.0.0.1:1080"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
    "#,
    )
    .unwrap();
    let table = RouteTable::from_config(&config);

    let route = table.match_host("api.example.com").unwrap();

    assert_eq!(
        route.upstream_proxy.as_deref(),
        Some("socks5://127.0.0.1:1080")
    );
}

#[test]
fn matched_route_prefers_route_upstream_proxy_override() {
    let config = AppConfig::from_toml_str(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [upstream_proxy]
        default = "socks5://127.0.0.1:1080"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
        upstream_proxy = "direct"
    "#,
    )
    .unwrap();
    let table = RouteTable::from_config(&config);

    let route = table.match_host("api.example.com").unwrap();

    assert_eq!(route.upstream_proxy.as_deref(), Some("direct"));
}
