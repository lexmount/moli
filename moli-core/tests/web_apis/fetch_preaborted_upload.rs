use super::*;

async fn assert_preaborted_upload(scenario: &str) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = format!(
        "{}\nrunPreabortedUploadProbe({}, {}).then(finish, error => finish({{error: String(error)}}));",
        include_str!("../fixtures/runtime/fetch_preaborted_upload.js"),
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
async fn preaborted_fetch_cancels_readable_uploads_synchronously_with_original_reason() -> Result<()>
{
    assert_preaborted_upload("cancel").await
}

#[tokio::test(flavor = "multi_thread")]
async fn preaborted_fetch_cancels_only_the_selected_body_and_allows_reentry() -> Result<()> {
    assert_preaborted_upload("selection").await
}

#[tokio::test(flavor = "multi_thread")]
async fn preaborted_fetch_validates_stream_requests_before_cancellation() -> Result<()> {
    assert_preaborted_upload("validation").await
}

#[tokio::test(flavor = "multi_thread")]
async fn preaborted_fetch_uses_internal_stream_cancellation_and_promise_reactions() -> Result<()> {
    assert_preaborted_upload("intrinsics").await
}
