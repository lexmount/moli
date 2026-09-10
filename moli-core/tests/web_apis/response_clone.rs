use super::*;

async fn assert_response_clone(scenario: &str) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = format!(
        "{}\nrunResponseCloneProbe({}, {}).then(finish, error => finish({{error: String(error)}}));",
        include_str!("../fixtures/runtime/response_clone.js"),
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
async fn response_clone_copies_stream_chunks_without_public_stream_operations() -> Result<()> {
    assert_response_clone("chunks").await
}

#[tokio::test(flavor = "multi_thread")]
async fn response_clone_preserves_stream_lifecycle_and_cancellation_errors() -> Result<()> {
    assert_response_clone("lifecycle").await
}

#[tokio::test(flavor = "multi_thread")]
async fn response_clone_preserves_metadata_header_guards_and_relevant_realm() -> Result<()> {
    assert_response_clone("surface").await
}
