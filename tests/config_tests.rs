use mirrox::config::{AppConfig, BodyRewriteMode, DnsMode, ServerMode};
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn parses_minimal_config_with_defaults() {
    let toml = r#"
        [server]
        listen = "127.0.0.1:3000"

        [[routes]]
        incoming = "www.example.com"
        upstream = "www.bgm.tv"
    "#;

    let config = AppConfig::from_toml_str(toml).expect("config should parse");

    assert_eq!(config.server.listen, "127.0.0.1:3000");
    assert_eq!(config.server.mode, ServerMode::BehindProxy);
    assert_eq!(config.dns.mode, DnsMode::Doh);
    assert_eq!(config.rewrite.body, BodyRewriteMode::Enabled);
    assert_eq!(config.routes[0].incoming, "www.example.com");
    assert_eq!(config.routes[0].upstream, "www.bgm.tv");
}

#[test]
fn rejects_empty_route_list() {
    let toml = r#"
        [server]
        listen = "127.0.0.1:3000"
    "#;

    let err = AppConfig::from_toml_str(toml).expect_err("empty routes should fail");
    assert!(err.to_string().contains("at least one route"));
}

#[test]
fn parses_global_and_route_upstream_proxy() {
    let config = AppConfig::from_toml_str(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [upstream_proxy]
        default = "socks5://user:pass@127.0.0.1:1080"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"

        [[routes]]
        incoming = "www.example.com"
        upstream = "www.bgm.tv"
        upstream_proxy = "direct"

        [[wildcard_routes]]
        incoming_suffix = ".mirror.example.com"
        upstream_suffix = ".bgm.tv"
        upstream_proxy = "http://proxy-user:proxy-pass@127.0.0.1:8080"
    "#,
    )
    .unwrap();

    assert_eq!(
        config.upstream_proxy.default.as_deref(),
        Some("socks5://user:pass@127.0.0.1:1080")
    );
    assert_eq!(config.routes[0].upstream_proxy, None);
    assert_eq!(config.routes[1].upstream_proxy.as_deref(), Some("direct"));
    assert_eq!(
        config.wildcard_routes[0].upstream_proxy.as_deref(),
        Some("http://proxy-user:proxy-pass@127.0.0.1:8080")
    );
}

#[test]
fn environment_overrides_global_upstream_proxy() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("MIRROX_UPSTREAM_PROXY", "http://127.0.0.1:8080");
    let config = AppConfig::from_toml_str_with_env(
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
    std::env::remove_var("MIRROX_UPSTREAM_PROXY");

    assert_eq!(
        config.upstream_proxy.default.as_deref(),
        Some("http://127.0.0.1:8080")
    );
}

#[test]
fn direct_environment_override_clears_global_upstream_proxy() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("MIRROX_UPSTREAM_PROXY", "direct");
    let config = AppConfig::from_toml_str_with_env(
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
    std::env::remove_var("MIRROX_UPSTREAM_PROXY");

    assert_eq!(config.upstream_proxy.default, None);
}

#[test]
fn rejects_invalid_upstream_proxy_scheme() {
    let error = AppConfig::from_toml_str(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [upstream_proxy]
        default = "ftp://127.0.0.1:21"

        [[routes]]
        incoming = "api.example.com"
        upstream = "api.bgm.tv"
    "#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("invalid upstream proxy"));
}
