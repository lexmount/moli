use super::*;

async fn assert_request_stream_body(scenario: &str) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = format!(
        "{}\nrunRequestStreamBodyProbe({}, {}).then(finish, error => finish({{error: String(error)}}));",
        include_str!("../fixtures/runtime/request_stream_body.js"),
        serde_json::to_string(scenario)?,
        serde_json::to_string(&server.url("/net/echo"))?,
    );
    for target in ["window", "child", "worker"] {
        let observed = tokio::time::timeout(
            Duration::from_secs(20),
            super::event_dispatch::run_probe(&browser, &server, target, &source),
        )
        .await??;
        assert_eq!(
            observed,
            serde_json::json!({"errors": []}),
            "{scenario}/{target}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn request_stream_inputs_preserve_identity_bytes_and_consumption_state() -> Result<()> {
    assert_request_stream_body("input").await
}

#[tokio::test(flavor = "multi_thread")]
async fn request_stream_clones_tee_bytes_and_forward_cancellation_and_errors() -> Result<()> {
    assert_request_stream_body("clone").await
}

#[tokio::test(flavor = "multi_thread")]
async fn request_stream_construction_proxies_inherited_bodies_and_respects_overrides() -> Result<()>
{
    assert_request_stream_body("inherit").await
}
