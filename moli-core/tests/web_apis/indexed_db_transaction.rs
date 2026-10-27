use super::pipe_disturbed::run_probe;
use super::*;
#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_compound_key_paths_bound_complete_keys_without_poisoning_commits() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-compound-key-depth.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\ncompoundDepthProbe('compound-depth-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: compoundDepthChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            34,
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_keys_preserve_types_order_and_binary_snapshots() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-key-model.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nkeyModelProbe('key-model-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: keyChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            288,
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_key_ranges_protect_bounds_and_validate_receivers() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-key-range.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nkeyRangeProbe('key-range-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: rangeChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            if target == "worker" { 301 } else { 313 },
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_version_change_events_preserve_native_attributes_and_dictionary_semantics()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-version-change.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nversionChangeProbe('version-change-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: versionChangeChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            if target == "worker" { 128 } else { 384 },
            "{target}: {result}"
        );
    }
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
            if target == "worker" { 162 } else { 486 },
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
async fn indexed_db_transaction_arguments_convert_names_before_mode_and_scope_checks() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-transaction-conversion.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\ntransactionConversionProbe('transaction-conversion-{target}').then(finish, error => finish({{state:'error',error:String(error)}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            9,
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}
