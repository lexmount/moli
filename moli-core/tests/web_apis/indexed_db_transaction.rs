use super::event_dispatch::run_probe;
use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_unique_index_builds_abort_in_order_and_deduplicate_multi_entry_keys()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-index-build.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nindexBuildProbe('index-build-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: indexBuildChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert!(
            result["checks"].as_array().unwrap().len() >= 150,
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_events_propagate_and_abort_only_active_transactions() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-event-propagation.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nidbEventProbe('event-propagation-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: idbEventChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            152,
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

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

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_requests_expose_native_state_and_factory_event_properties() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-request-state.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nrequestStateProbe('request-state-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: requestStateChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            167,
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_request_getters_preserve_cross_realm_identity() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-request-realms.js");
    let source = format!(
        "{fixture}\nrequestRealmProbe().then(finish, error => finish({{state: 'error', error: String(error)}}));"
    );
    let result = run_probe(&browser, &server, "window", &source).await?;
    assert_eq!(result["state"], "pass", "{result}");
    assert_eq!(result["checks"].as_array().unwrap().len(), 26, "{result}");
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_factory_checks_native_receivers_and_rejects_promises_in_callee_realm()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-factory-receivers.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nconst watchdog = setTimeout(() => finish({{state: 'timeout', checks: factoryReceiverChecks.slice(-12)}}), 5000); factoryReceiverProbe('factory-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: factoryReceiverChecks}})).finally(() => clearTimeout(watchdog));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            if target == "worker" { 149 } else { 447 },
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

