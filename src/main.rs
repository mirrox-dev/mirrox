use anyhow::Context;
use mirrox::cli::Cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let config = mirrox::config::AppConfig::load_from_path_or_env(cli.config.as_deref())
        .context("failed to load configuration")?;
    mirrox::server::run(config).await
}
