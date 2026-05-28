use axum_test::TestServer;
use mirrox::config::AppConfig;
use mirrox::server::build_router;

fn config() -> AppConfig {
    AppConfig::from_toml_str(
        r#"
        [server]
        listen = "127.0.0.1:3000"

        [[routes]]
        incoming = "www.example.com"
        upstream = "www.bgm.tv"
    "#,
    )
    .unwrap()
}

#[tokio::test]
async fn healthz_returns_ok() {
    let app = build_router(config()).await.unwrap();
    let server = TestServer::new(app).unwrap();

    let response = server.get("/healthz").await;

    response.assert_status_ok();
    response.assert_text("ok");
}

#[tokio::test]
async fn readyz_returns_ready() {
    let app = build_router(config()).await.unwrap();
    let server = TestServer::new(app).unwrap();

    let response = server.get("/readyz").await;

    response.assert_status_ok();
    response.assert_text("ready");
}
