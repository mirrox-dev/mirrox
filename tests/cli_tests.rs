use mirrox::cli::Cli;
use mirrox::config::{AppConfig, BodyRewriteMode};
use std::io::Write;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn config_text(listen: &str) -> String {
    format!(
        r#"
        [server]
        listen = "{listen}"

        [[routes]]
        incoming = "www.example.com"
        upstream = "www.bgm.tv"
    "#
    )
}

fn write_config(listen: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(config_text(listen).as_bytes()).unwrap();
    file
}

#[test]
fn parses_short_config_argument() {
    let cli = Cli::parse_from(["mirrox", "-c", "/etc/mirrox/config.toml"]);

    assert_eq!(
        cli.config.as_deref(),
        Some(std::path::Path::new("/etc/mirrox/config.toml"))
    );
}

#[test]
fn parses_long_config_argument() {
    let cli = Cli::parse_from(["mirrox", "--config", "/etc/mirrox/config.toml"]);

    assert_eq!(
        cli.config.as_deref(),
        Some(std::path::Path::new("/etc/mirrox/config.toml"))
    );
}

#[test]
fn explicit_config_path_overrides_mirrox_config_environment() {
    let _guard = ENV_LOCK.lock().unwrap();
    let env_config = write_config("127.0.0.1:3001");
    let cli_config = write_config("127.0.0.1:3002");
    std::env::set_var("MIRROX_CONFIG", env_config.path());

    let config = AppConfig::load_from_path_or_env(Some(cli_config.path())).unwrap();

    std::env::remove_var("MIRROX_CONFIG");
    assert_eq!(config.server.listen, "127.0.0.1:3002");
}

#[test]
fn environment_overrides_still_apply_with_explicit_config_path() {
    let _guard = ENV_LOCK.lock().unwrap();
    let cli_config = write_config("127.0.0.1:3002");
    std::env::set_var("MIRROX_REWRITE_BODY", "http-only");

    let config = AppConfig::load_from_path_or_env(Some(cli_config.path())).unwrap();

    std::env::remove_var("MIRROX_REWRITE_BODY");
    assert_eq!(config.rewrite.body, BodyRewriteMode::HttpOnly);
}
