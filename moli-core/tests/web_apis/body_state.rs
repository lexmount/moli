use super::*;

async fn assert_body_consumption_state(scenario: &str) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = format!(
        "{}\nrunBodyConsumptionStateProbe({}, {}).then(finish, error => finish({{error: String(error)}}));",
        include_str!("../fixtures/runtime/body_consumption_state.js"),
        serde_json::to_string(scenario)?,
        serde_json::to_string(&server.url("/compat/child-dynamic-markup-document"))?,
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
async fn null_bodies_remain_unused_and_allow_repeated_consumption_and_cloning() -> Result<()> {
    assert_body_consumption_state("null").await
}

#[tokio::test(flavor = "multi_thread")]
async fn locked_bodies_reject_consumption_and_clone_without_becoming_disturbed() -> Result<()> {
    assert_body_consumption_state("locked").await
}

#[tokio::test(flavor = "multi_thread")]
async fn body_consumption_locks_and_disturbs_both_buffered_and_streaming_bodies() -> Result<()> {
    assert_body_consumption_state("consumed").await
}

#[tokio::test(flavor = "multi_thread")]
async fn disturbed_body_streams_remain_unusable_after_readers_release_their_locks() -> Result<()> {
    assert_body_consumption_state("disturbed").await
}

#[tokio::test(flavor = "multi_thread")]
async fn body_read_and_conversion_errors_preserve_consumption_state() -> Result<()> {
    assert_body_consumption_state("errors").await
}

#[tokio::test(flavor = "multi_thread")]
async fn body_state_checks_and_stream_reads_ignore_public_member_overrides() -> Result<()> {
    assert_body_consumption_state("poison").await
}
