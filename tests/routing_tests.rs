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
    assert_eq!(
        route.user_agent.as_deref(),
        Some("Mirrox-Wildcard/1.0")
    );
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
