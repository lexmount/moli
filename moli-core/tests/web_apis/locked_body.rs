use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn locked_bodies_reject_consumption_and_clone_without_becoming_disturbed() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = format!(
        "{}\nrunLockedBodyProbe({}).then(finish, error => finish({{error: String(error)}}));",
        include_str!("../fixtures/runtime/locked_body.js"),
        serde_json::to_string(&server.url("/compat/child-dynamic-markup-document"))?,
    );
    for target in ["window", "child", "worker"] {
        let observed = tokio::time::timeout(
            Duration::from_secs(20),
            super::pipe_disturbed::run_probe(&browser, &server, target, &source),
        )
        .await??;
        assert_eq!(observed, serde_json::json!({"errors": []}), "{target}");
    }
    server.shutdown().await;
    Ok(())
}
