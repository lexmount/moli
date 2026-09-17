use super::event_dispatch::run_probe;
use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_cursor_navigation_validates_state_in_spec_order() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-cursor-navigation.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\ncursorNavigationProbe('cursor-navigation-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: navigationChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            if target == "worker" { 2612 } else { 2621 },
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_cursors_protect_native_state_and_cache_values_in_getter_realms() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-cursor-state.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\ncursorStateProbe('cursor-state-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: cursorChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            if target == "worker" { 673 } else { 709 },
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
            if target == "worker" { 317 } else { 329 },
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_get_all_records_preserves_snapshots_options_and_realms() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-get-all-records.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nrecordProbe('records-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: recordChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            if target == "worker" { 321 } else { 330 },
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_schedules_overlapping_transactions_across_connections_and_agents() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-transaction-scheduling.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nschedulingProbe('scheduling-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: schedulingChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            162,
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
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            548,
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
            259,
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_writes_clone_before_key_paths_and_snapshot_queued_values() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-write-clone.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nwriteCloneProbe('write-clone-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: writeChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            165,
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_schema_validation_preserves_key_paths_conversion_and_exception_order()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-schema-validation.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nschemaValidationProbe('schema-validation-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: schemaChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert!(
            result["checks"].as_array().unwrap().len() >= 160,
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

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
