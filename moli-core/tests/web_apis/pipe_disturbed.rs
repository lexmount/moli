use super::*;

async fn assert_pipe_disturbed(scenario: &str) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = format!(
        "{}\nrunPipeDisturbedProbe({}).then(finish, error => finish({{error: String(error)}}));",
        include_str!("../fixtures/runtime/pipe_disturbed.js"),
        serde_json::to_string(scenario)?,
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
async fn piping_disturbs_fetch_bodies_before_reads_and_abort_handling() -> Result<()> {
    assert_pipe_disturbed("start").await
}

#[tokio::test(flavor = "multi_thread")]
async fn rejected_piping_preserves_undisturbed_fetch_bodies() -> Result<()> {
    assert_pipe_disturbed("reject").await
}
