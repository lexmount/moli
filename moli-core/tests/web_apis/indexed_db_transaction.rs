use super::pipe_disturbed::run_probe;
use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_queries_snapshot_arguments_and_delete_key_ranges() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-query-snapshots.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nquerySnapshotProbe('query-snapshots-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: queryChecks}}));"
        );
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            run_probe(&browser, &server, target, &source),
        )
        .await??;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            308,
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}
