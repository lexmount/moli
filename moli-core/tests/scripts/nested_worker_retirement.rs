use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn nested_workers_release_locks_when_their_ancestor_global_retires() -> Result<()> {
    const SOURCE: &str = include_str!("../fixtures/nested-worker-retirement.js");
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../fixtures/nested-worker-retirement.chromium.json"
    ))?;
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let expression = format!(
        r#"({SOURCE}).then(value => JSON.stringify({{
            cases: value.cases.map(row => ({{
                scriptType: row.scriptType, depth: row.depth, mode: row.mode,
                heldBefore: row.heldBefore, released: row.released,
                controlRetained: row.controlRetained
            }})),
            controlReleased: value.controlReleased,
            remainingNames: value.remainingHeld.map(lock => lock.name).sort()
        }}))"#
    );
    let result = tokio::time::timeout(
        Duration::from_secs(40),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await;
    server.shutdown().await;
    let result = result??;
    let actual: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .expect("worker retirement fixture must finish with behavior rows"),
    )?;
    assert_eq!(
        actual, oracle["expected"],
        "retiring an ancestor must release descendant locks and retain unrelated live workers"
    );
    Ok(())
}
