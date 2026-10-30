use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_store_names_follow_upgrade_commit_abort_and_utf16_identity() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("../fixtures/indexeddb-store-rename.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nstoreRenameProbe('store-rename-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: storeRenameChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert!(
            result["checks"].as_array().unwrap().len() > 150,
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}
