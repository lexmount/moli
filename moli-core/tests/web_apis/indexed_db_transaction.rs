use super::pipe_disturbed::run_probe;
use super::*;

async fn assert_multi_entry_keys(scenario: &str) -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-multientry-keys.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nmultiEntryProbe('multientry-{scenario}-{target}', '{scenario}').then(finish, error => finish({{state: 'error', error: String(error), checks: multiEntryChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{scenario}/{target}: {result}");
        assert_eq!(result["cases"], 48, "{scenario}/{target}: {result}");
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_multientry_indexes_deduplicate_equal_keys_per_record() -> Result<()> {
    assert_multi_entry_keys("duplicates").await
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_multientry_unique_conflicts_preserve_transaction_outcomes() -> Result<()> {
    assert_multi_entry_keys("conflicts").await
}

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
async fn indexed_db_queued_collections_preserve_foreign_sources_and_transaction_owners()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-queued-foreign-sources.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nforeignQueueProbe('foreign-queue-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: foreignQueueChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            if target == "worker" { 99 } else { 381 },
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
            if target == "worker" { 325 } else { 336 },
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_request_errors_cancel_bubble_and_abort_upgrades() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-request-errors.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nrequestErrorProbe('request-errors-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: requestErrorChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert!(
            result["checks"].as_array().unwrap().len() > 500,
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_queued_transactions_preserve_foreign_sources_and_request_realms() -> Result<()>
{
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-queued-transaction-sources.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nforeignQueueProbe('queued-transaction-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: foreignQueueChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            if target == "worker" { 65 } else { 245 },
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
            172,
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_upgrade_metadata_tracks_handles_across_creation_realms() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-upgrade-handles.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\nupgradeHandleProbe('upgrade-handles-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: upgradeHandleChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            if target == "worker" { 46 } else { 47 },
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

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

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_database_attributes_preserve_connection_state_across_upgrade_and_close()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-database-attributes.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\ndatabaseAttributesProbe('database-attributes-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: databaseAttributeChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            if target == "worker" { 254 } else { 830 },
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn indexed_db_transaction_attributes_preserve_native_state_receivers_and_lifetime()
-> Result<()> {
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let fixture = include_str!("fixtures/indexeddb-transaction-attributes.js");
    for target in ["window", "child", "worker"] {
        let source = format!(
            "{fixture}\ntransactionAttributesProbe('transaction-attributes-{target}').then(finish, error => finish({{state: 'error', error: String(error), checks: transactionAttributeChecks}}));"
        );
        let result = run_probe(&browser, &server, target, &source).await?;
        assert_eq!(result["state"], "pass", "{target}: {result}");
        assert_eq!(
            result["checks"].as_array().unwrap().len(),
            if target == "worker" { 199 } else { 671 },
            "{target}: {result}"
        );
    }
    server.shutdown().await;
    Ok(())
}
