use super::event_dispatch::run_probe;
use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_transaction_validates_arguments_and_upgrade_boundaries() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-transaction-validation.js");
    for target in ["window", "child", "worker"] {
        // This needs the production owner loop: the live upgrade pumps
        // requests while a timer checks its inactive state in another task.
        let source = format!(
            "{fixture}\ntransactionValidationProbe('transaction-validation-{target}').then(finish, error => finish({{state: 'error', error: String(error)}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(result["checks"].as_array().unwrap().len(), 55, "{target}");
    }
    server.shutdown().await;
    Ok(())
}
