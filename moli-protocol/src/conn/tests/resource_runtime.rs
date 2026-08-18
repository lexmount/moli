use super::*;
use crate::DevToolsDocumentLifecycleWaitState;
use crate::conn::{
    BackgroundTarget, CdpInitialStoragePartition, LoadedNavigation, NavigationLoadOutcome,
    TargetIdentityState, TargetPageSlot,
};
use moli_core::runtime::storage_partition::StoragePartitionState;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;

fn stored_cookie(name: &str, value: &str) -> moli_cookie_jar::StoredCookie {
    moli_cookie_jar::StoredCookie {
        name: name.to_owned(),
        value: value.to_owned(),
        domain: "example.com".to_owned(),
        host_only: false,
        path: "/".to_owned(),
        secure: false,
        http_only: false,
        expires: None,
        same_site: moli_cookie_jar::StoredCookieSameSite::Unspecified,
        priority: None,
        partition_key: None,
        source_scheme: moli_cookie_jar::StoredCookieSourceScheme::NonSecure,
        source_port: -1,
        creation_index: 0,
        last_access_index: 0,
    }
}

async fn commit_navigation_outcome_for_test(
    conn: &mut CdpConnection,
    outcome: NavigationLoadOutcome,
) -> LoadedNavigation {
    match outcome {
        NavigationLoadOutcome::ResponseCommitReady(navigation) => {
            let navigation = *navigation;
            let configuration = conn.prepared_document_commit_configuration_for_session_owner(
                None,
                navigation.final_url(),
            );
            navigation
                .update_commit_configuration(configuration)
                .await
                .expect("test navigation commit configuration should apply");
            let permit = navigation.issue_commit_permit();
            navigation
                .commit(permit)
                .await
                .expect("test navigation should commit")
        }
        NavigationLoadOutcome::Loaded(navigation) => *navigation,
        NavigationLoadOutcome::Download(_) => {
            panic!("test navigation should not resolve to a download")
        }
        NavigationLoadOutcome::NetworkFailure(error_text) => {
            panic!("test navigation should not fail: {error_text}")
        }
    }
}

#[test]
fn current_navigation_initiator_url_uses_loaded_browser_context_url_when_available() {
    let mut conn = CdpConnection::new();
    assert!(conn.current_navigation_initiator_url().is_none());

    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_target_url("about:blank".into());
    conn.browser_context = Some(bc);
    assert!(conn.current_navigation_initiator_url().is_none());

    conn.browser_context
        .as_mut()
        .unwrap()
        .set_target_url("https://example.com/app".into());
    assert_eq!(
        conn.current_navigation_initiator_url(),
        Some(Url::parse("https://example.com/app").unwrap())
    );
}

#[test]
fn connection_initial_cookies_seed_new_browser_contexts() {
    let conn = CdpConnection::new_with_initial_cookies(vec![stored_cookie("sid", "seeded")]);

    for id in ["BID-1", "BID-2"] {
        let bc = conn.new_browser_context(id.to_owned());
        assert!(bc.is_profile_backed_storage_partition());
        assert_eq!(bc.storage_partition_kind_label(), "profile-backed");
        assert_eq!(bc.storage_partition_id(), "default");
        let cookies = bc.snapshot_cookies();
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].name, "sid");
        assert_eq!(cookies[0].value, "seeded");
    }
}

#[test]
fn initial_storage_partition_derives_store_handles_from_core_owner() {
    let storage_partition = StoragePartitionState::open(None).expect("memory partition");
    let initial_storage_partition = CdpInitialStoragePartition::from_storage_partition(
        vec![stored_cookie("sid", "seeded")],
        &storage_partition,
    );
    let conn = CdpConnection::new_with_initial_storage_partition(initial_storage_partition);

    let browser_context = conn.new_browser_context("BID-owner".to_owned());
    let shared_storage = storage_partition.shared_storage_handles();
    let expected_web_storage_store = shared_storage.web_storage_store();
    let expected_indexed_db_manager = shared_storage.indexed_db_manager();
    let expected_storage_bucket_store = shared_storage.storage_bucket_store();

    assert!(Arc::ptr_eq(
        browser_context.web_storage_store_for_test(),
        &expected_web_storage_store
    ));
    assert!(Arc::ptr_eq(
        browser_context.indexed_db_manager_for_test(),
        &expected_indexed_db_manager
    ));
    assert!(Arc::ptr_eq(
        browser_context.storage_bucket_store_for_test(),
        &expected_storage_bucket_store
    ));
    let cookies = browser_context.snapshot_cookies();
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "sid");
    assert_eq!(cookies[0].value, "seeded");
}

#[test]
fn default_browser_contexts_reuse_partition_with_distinct_target_session_storage() {
    let storage_partition = StoragePartitionState::open(None).expect("memory partition");
    let initial_storage_partition =
        CdpInitialStoragePartition::from_storage_partition(Vec::new(), &storage_partition);
    let conn = CdpConnection::new_with_initial_storage_partition(initial_storage_partition);

    let first = conn.new_browser_context("BID-first".to_owned());
    let second = conn.new_browser_context("BID-second".to_owned());
    let expected_web_storage_store = storage_partition
        .shared_storage_handles()
        .web_storage_store();

    assert!(Arc::ptr_eq(
        first.cookie_store_for_test(),
        second.cookie_store_for_test()
    ));
    assert!(Arc::ptr_eq(
        first.web_storage_store_for_test(),
        &expected_web_storage_store
    ));
    assert!(Arc::ptr_eq(
        second.web_storage_store_for_test(),
        &expected_web_storage_store
    ));
    assert!(Arc::ptr_eq(
        first.indexed_db_manager_for_test(),
        second.indexed_db_manager_for_test()
    ));
    assert!(Arc::ptr_eq(
        first.storage_bucket_store_for_test(),
        second.storage_bucket_store_for_test()
    ));
    assert!(!Arc::ptr_eq(
        first.session_storage_store_for_test(),
        second.session_storage_store_for_test()
    ));
}

#[test]
fn connection_ephemeral_browser_context_uses_isolated_storage_partition() {
    let conn = CdpConnection::new_with_initial_cookies(vec![stored_cookie("sid", "seeded")]);

    let bc = conn.new_ephemeral_browser_context("BID-ephemeral".to_owned());

    assert!(!bc.is_profile_backed_storage_partition());
    assert_eq!(bc.storage_partition_kind_label(), "ephemeral");
    assert_eq!(bc.storage_partition_id(), "BID-ephemeral");
    assert!(bc.snapshot_cookies().is_empty());
}

#[test]
fn connection_default_and_ephemeral_context_creation_use_named_partition_paths() {
    let storage_partition = StoragePartitionState::open(None).expect("memory partition");
    let initial_storage_partition =
        CdpInitialStoragePartition::from_storage_partition(Vec::new(), &storage_partition);
    let conn = CdpConnection::new_with_initial_storage_partition(initial_storage_partition);

    let profile_backed = conn.new_browser_context("BID-profile".to_owned());
    let ephemeral = conn.new_ephemeral_browser_context("BID-ephemeral".to_owned());
    let expected_web_storage_store = storage_partition
        .shared_storage_handles()
        .web_storage_store();

    assert!(profile_backed.is_profile_backed_storage_partition());
    assert_eq!(profile_backed.storage_partition_id(), "default");
    assert!(Arc::ptr_eq(
        profile_backed.web_storage_store_for_test(),
        &expected_web_storage_store
    ));

    assert!(!ephemeral.is_profile_backed_storage_partition());
    assert_eq!(ephemeral.storage_partition_id(), "BID-ephemeral");
    assert!(!Arc::ptr_eq(
        ephemeral.web_storage_store_for_test(),
        &expected_web_storage_store
    ));
}

#[test]
fn browser_context_memory_diagnostics_include_storage_partition_identity() {
    let conn = CdpConnection::new();
    let profile_backed = conn.new_browser_context("BID-profile".to_owned());
    let ephemeral = conn.new_ephemeral_browser_context("BID-ephemeral".to_owned());

    assert_eq!(
        profile_backed.storage_partition_kind_label(),
        "profile-backed"
    );
    assert_eq!(profile_backed.storage_partition_id(), "default");

    assert_eq!(ephemeral.storage_partition_kind_label(), "ephemeral");
    assert_eq!(ephemeral.storage_partition_id(), "BID-ephemeral");

    assert_eq!(
        profile_backed.moli_memory_diagnostics()["storagePartition"],
        json!({
            "kind": "profile-backed",
            "id": "default",
        })
    );
    assert_eq!(
        ephemeral.moli_memory_diagnostics()["storagePartition"],
        json!({
            "kind": "ephemeral",
            "id": "BID-ephemeral",
        })
    );
}

#[test]
fn browser_context_request_cookie_report_reads_storage_partition_cookie_handle() {
    let browser_context = BrowserContext::new("BID-cookie-report".to_owned());
    let request_url = Url::parse("https://example.com/app/index.html").unwrap();
    {
        let mut cookie_store = browser_context.cookie_store_for_test().lock();
        cookie_store.store_response_headers(
            &request_url,
            &[(
                "set-cookie".to_owned(),
                "sid=partition; Path=/app".to_owned(),
            )],
        );
    }

    let report = browser_context
        .observe_request_cookie_access_report(
            &request_url,
            moli_cookie_jar::NetworkCookieRequestContext::top_level_navigation("GET"),
        )
        .expect("partition cookie should produce a request access report");

    assert_eq!(report.included_cookies.len(), 1);
    assert_eq!(report.included_cookies[0].cookie.name, "sid");
    assert_eq!(report.included_cookies[0].cookie.value, "partition");
    assert!(report.excluded_cookies.is_empty());
}

#[test]
fn browser_context_cookie_snapshot_and_delete_use_storage_partition_cookie_handle() {
    let mut browser_context = BrowserContext::new("BID-cookie-snapshot".to_owned());
    let request_url = Url::parse("https://example.com/app/index.html").unwrap();
    {
        let mut cookie_store = browser_context.cookie_store_for_test().lock();
        cookie_store.store_response_headers(
            &request_url,
            &[(
                "set-cookie".to_owned(),
                "sid=partition; Path=/app".to_owned(),
            )],
        );
    }

    let snapshot = browser_context.snapshot_cookies();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].name, "sid");
    assert_eq!(snapshot[0].value, "partition");

    browser_context.delete_cookies(Some("sid"), Some("example.com"), Some("/app"), None);

    assert!(browser_context.snapshot_cookies().is_empty());
    assert!(
        browser_context
            .cookie_store_for_test()
            .lock()
            .cookies()
            .is_empty()
    );
}

#[test]
fn browser_context_storage_usage_reads_storage_partition_owner() {
    let browser_context = BrowserContext::new("BID-storage-usage".to_owned());
    let origin = Url::parse("https://usage.example/app")
        .unwrap()
        .origin()
        .ascii_serialization();
    let storage_key =
        moli_storage_key::MoliStorageKey::first_party_from_url(&Url::parse(&origin).unwrap(), None)
            .serialized_storage_key();
    {
        let mut store = browser_context.web_storage_store_for_test().lock();
        assert!(store.set_item(&storage_key, "local", "owner"));
    }

    let usage = browser_context
        .storage_usage_for_origin(&origin)
        .expect("storage usage should be readable");

    assert_eq!(usage.local_storage_usage, "owner".len() as u64);
    assert_eq!(usage.indexed_db_usage, 0);
    assert_eq!(usage.storage_buckets_usage, 0);
    assert_eq!(usage.total_usage, usage.local_storage_usage);
}

#[test]
fn navigation_load_inputs_own_cookie_request_and_response_reports() {
    let conn = CdpConnection::new();
    let response_url = Url::parse("https://example.com/app/index.html").unwrap();
    let load_inputs = conn.navigation_load_inputs_for_session_owner(None);

    let set_reports = load_inputs.store_response_cookie_reports(
        &response_url,
        &[(
            "set-cookie".to_owned(),
            "sid=load-input; Path=/app".to_owned(),
        )],
    );
    assert_eq!(set_reports.len(), 1);
    assert!(set_reports[0].is_accepted());

    let report = load_inputs
        .request_cookie_report_for_navigation(&response_url, "GET", false)
        .expect("load input cookie store should produce a request report");
    assert_eq!(report.included_cookies.len(), 1);
    assert_eq!(report.included_cookies[0].cookie.name, "sid");
    assert_eq!(report.included_cookies[0].cookie.value, "load-input");
    assert!(report.excluded_cookies.is_empty());
}

#[test]
fn no_loaded_browser_context_navigation_inputs_reuse_initial_storage_stores() {
    let mut conn = CdpConnection::new();

    let first_inputs = conn.navigation_load_inputs_for_session_owner(None);
    let second_inputs = conn.navigation_load_inputs_for_session_owner(None);

    assert!(first_inputs.browser_context_id.is_none());
    assert!(second_inputs.browser_context_id.is_none());
    let first_storage = first_inputs.resource_storage_handles();
    let second_storage = second_inputs.resource_storage_handles();
    assert!(Arc::ptr_eq(
        &first_storage.cookie_store,
        &second_storage.cookie_store
    ));
    assert!(Arc::ptr_eq(
        &first_storage.web_storage_store,
        &second_storage.web_storage_store
    ));
    assert!(Arc::ptr_eq(
        &first_storage.session_storage_store,
        &second_storage.session_storage_store
    ));
    let first_page_storage = first_inputs.page_storage_handles();
    let second_page_storage = second_inputs.page_storage_handles();
    assert!(Arc::ptr_eq(
        first_page_storage
            .storage_bucket_store
            .as_ref()
            .expect("initial storage bucket store"),
        second_page_storage
            .storage_bucket_store
            .as_ref()
            .expect("initial storage bucket store"),
    ));

    let (loader_cookie_store, resource_runtime_id) = {
        let loader = conn
            .ensure_resource_request_client_for_navigation_load_inputs(&first_inputs)
            .expect("loader for first no-context inputs");
        (
            loader.cookie_store(),
            loader.resource_runtime_diagnostics().runtime_id,
        )
    };
    let loader = conn
        .ensure_resource_request_client_for_navigation_load_inputs(&second_inputs)
        .expect("loader for second no-context inputs");
    assert!(Arc::ptr_eq(&loader.cookie_store(), &loader_cookie_store));
    assert_eq!(
        loader.resource_runtime_diagnostics().runtime_id,
        resource_runtime_id,
        "reusing storage handles must not rebuild the browser resource runtime",
    );
}

#[test]
fn page_request_client_for_navigation_inputs_inherits_service_worker_bypass() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-1".to_owned());
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    browser_context
        .network_policy
        .set_bypass_service_worker(true);
    conn.browser_context = Some(browser_context);

    let inputs = conn.navigation_load_inputs_for_session_owner(Some("SID-1"));
    let request_client = conn
        .ensure_resource_request_client_for_navigation_load_inputs(&inputs)
        .expect("page request client for active target");

    assert!(inputs.bypass_service_worker);
    assert!(request_client.bypass_service_worker());
}

#[test]
fn connection_snapshot_cookies_collects_active_and_inactive_contexts() {
    let mut conn = CdpConnection::new();
    let active = BrowserContext::new("BID-active".to_owned());
    active.upsert_cookie_for_test(stored_cookie("active", "1"));
    let inactive = BrowserContext::new("BID-inactive".to_owned());
    inactive.upsert_cookie_for_test(stored_cookie("inactive", "1"));
    conn.browser_context = Some(active);
    conn.inactive_browser_contexts.push(inactive);

    let mut names = conn
        .snapshot_cookies()
        .into_iter()
        .map(|cookie| cookie.name)
        .collect::<Vec<_>>();
    names.sort();

    assert_eq!(names, vec!["active", "inactive"]);
}

#[test]
fn connection_profile_backed_cookie_snapshot_ignores_ephemeral_contexts() {
    let mut conn = CdpConnection::new();
    let profile_backed = conn.new_browser_context("BID-profile".to_owned());
    profile_backed.upsert_cookie_for_test(stored_cookie("profile", "1"));
    let ephemeral = conn.new_ephemeral_browser_context("BID-ephemeral".to_owned());
    ephemeral.upsert_cookie_for_test(stored_cookie("ephemeral", "1"));
    conn.browser_context = Some(ephemeral);
    conn.inactive_browser_contexts.push(profile_backed);

    let cookies = conn
        .snapshot_profile_backed_cookies()
        .expect("profile-backed snapshot");

    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "profile");
}

#[test]
fn connection_profile_backed_cookie_snapshot_is_none_without_profile_backed_context() {
    let mut conn = CdpConnection::new_with_initial_cookies(vec![stored_cookie("sid", "seeded")]);
    let ephemeral = conn.new_ephemeral_browser_context("BID-ephemeral".to_owned());
    assert!(ephemeral.snapshot_cookies().is_empty());
    conn.browser_context = Some(ephemeral);

    assert!(conn.snapshot_profile_backed_cookies().is_none());
}

#[tokio::test]
async fn build_loaded_navigation_from_buffered_response_updates_request_cookie_access_time() {
    let mut conn = CdpConnection::new();
    let requested_url = Url::parse("https://example.com/app/index.html").unwrap();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_target_url("https://example.com/origin".into());
    bc.store_response_cookie_headers_for_test(
        &requested_url,
        &[(
            "set-cookie".to_owned(),
            "sid=1; Path=/app; Secure".to_owned(),
        )],
    );
    let before = bc
        .test_last_cookie_access_index("example.com", "/app", "sid")
        .expect("cookie should exist before synthetic navigation");
    conn.browser_context = Some(bc);

    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            requested_url,
            "GET".into(),
            vec![],
            200,
            vec![],
            "<!doctype html><html><body>ok</body></html>".into(),
        )
        .await
        .expect("navigation should build");

    let after = conn
        .browser_context
        .as_ref()
        .unwrap()
        .test_last_cookie_access_index("example.com", "/app", "sid")
        .expect("cookie should still exist after synthetic navigation");
    assert!(
        after > before,
        "synthetic/request-stage navigations should touch request cookie access time"
    );
    assert_eq!(
        navigation
            .completed_body_network_events()
            .final_request_cookie_report
            .as_ref()
            .expect("navigation should capture request cookie report")
            .included_cookies[0]
            .cookie
            .name,
        "sid"
    );
}

#[tokio::test]
async fn rebuild_buffered_response_preserving_request_report_avoids_second_access_touch() {
    let mut conn = CdpConnection::new();
    let requested_url = Url::parse("https://example.com/app/index.html").unwrap();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_target_url("https://example.com/origin".into());
    bc.store_response_cookie_headers_for_test(
        &requested_url,
        &[(
            "set-cookie".to_owned(),
            "sid=1; Path=/app; Secure".to_owned(),
        )],
    );
    conn.browser_context = Some(bc);

    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            requested_url.clone(),
            "GET".into(),
            vec![],
            200,
            vec![],
            "<!doctype html><html><body>ok</body></html>".into(),
        )
        .await
        .expect("initial navigation should build");
    let after_first_touch = conn
        .browser_context
        .as_ref()
        .unwrap()
        .test_last_cookie_access_index("example.com", "/app", "sid")
        .expect("cookie should exist after initial navigation");

    let rebuilt = conn
        .build_loaded_navigation_from_buffered_response_preserving_request_cookie_report_async(
            requested_url,
            "GET".into(),
            vec![],
            204,
            vec![],
            String::new(),
            navigation
                .completed_body_network_events()
                .final_request_cookie_report
                .clone(),
        )
        .await
        .expect("response-stage rebuild should succeed");

    let after_rebuild = conn
        .browser_context
        .as_ref()
        .unwrap()
        .test_last_cookie_access_index("example.com", "/app", "sid")
        .expect("cookie should exist after response-stage rebuild");
    assert_eq!(
        after_rebuild, after_first_touch,
        "response-stage rebuilds should reuse the existing request cookie report without a second access-time touch"
    );
    assert_eq!(
        rebuilt
            .completed_body_network_events()
            .final_request_cookie_report,
        navigation
            .completed_body_network_events()
            .final_request_cookie_report
    );
}

#[tokio::test]
async fn reset_resource_runtime_clears_loaded_page_cookie_backend() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    let url = Url::parse("https://example.com/app").unwrap();

    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            url.clone(),
            "GET".into(),
            vec![],
            200,
            vec![("set-cookie".into(), "theme=dark; Path=/".into())],
            "<!doctype html><html><body>ok</body></html>".into(),
        )
        .await
        .expect("navigation should build");
    conn.browser_context
        .as_mut()
        .unwrap()
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(navigation.page);

    let page = conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_target
        .runtime_slot
        .loaded_page_mut()
        .unwrap();
    let before = page
        .evaluate_runtime_expression_async("document.cookie")
        .await
        .expect("cookie read should succeed");
    assert_eq!(before["value"], json!("theme=dark"));

    conn.reset_resource_runtime_async().await;

    let page = conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_target
        .runtime_slot
        .loaded_page_mut()
        .unwrap();
    let after = page
        .evaluate_runtime_expression_async("document.cookie")
        .await
        .expect("cookie read should still evaluate");
    assert_eq!(after["value"], json!(""));

    let snapshot = conn
        .browser_context
        .as_mut()
        .unwrap()
        .document_cookie_facade_snapshot_async()
        .await;
    assert_eq!(
        snapshot.capability_surface.backend_connection_state,
        BrowserContextCookieBackendConnectionState::Disconnected
    );
    assert_eq!(
        snapshot.freshness.cookie_get_freshness_status,
        BrowserContextCookieGetFreshnessStatus::NeedsBackendReconnect
    );
    assert_eq!(
        snapshot.freshness.cookie_set_readiness_status,
        BrowserContextCookieSetReadinessStatus::NeedsBackendReconnect
    );
    assert_eq!(
        snapshot.structured_write.readiness_status,
        BrowserContextStructuredCookieWriteReadinessStatus::ReadyUsingLoadedPageUrl
    );
    assert!(!snapshot.freshness.cookie_get_would_need_backend_access);
    assert!(snapshot.freshness.cookie_get_would_need_backend_reconnect);
    assert!(!snapshot.freshness.cookie_get_would_hit_cache);
}

#[tokio::test]
async fn same_target_navigations_reuse_local_and_session_storage() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    let first_url = Url::parse("https://storage.example/app/one").unwrap();
    let second_url = Url::parse("https://storage.example/app/two").unwrap();

    let mut first = conn
        .build_loaded_navigation_from_buffered_response_async(
            first_url,
            "GET".into(),
            vec![],
            200,
            vec![],
            "<!doctype html><html><body>one</body></html>".into(),
        )
        .await
        .expect("first synthetic navigation should build")
        .page;
    let write = first
        .evaluate_runtime_expression_async(
            "localStorage.clear(); sessionStorage.clear(); localStorage.setItem('shared', 'yes'); sessionStorage.setItem('ephemeral', 'yes'); 'ok'",
        )
        .await
        .expect("storage write should evaluate");
    assert_eq!(write["value"], json!("ok"));

    let mut second = conn
        .build_loaded_navigation_from_buffered_response_async(
            second_url,
            "GET".into(),
            vec![],
            200,
            vec![],
            "<!doctype html><html><body>two</body></html>".into(),
        )
        .await
        .expect("second synthetic navigation should build")
        .page;
    let read = second
        .evaluate_runtime_expression_async(
            "`${localStorage.getItem('shared')}|${String(sessionStorage.getItem('ephemeral'))}`",
        )
        .await
        .expect("storage read should evaluate");

    assert_eq!(read["value"], json!("yes|yes"));
}

#[tokio::test]
async fn browser_context_storage_does_not_cross_context_switches() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    conn.inactive_browser_contexts
        .push(BrowserContext::new("BID-2".into()));
    let url = Url::parse("https://context-storage.example/app").unwrap();

    let mut first = conn
        .build_loaded_navigation_from_buffered_response_async(
            url.clone(),
            "GET".into(),
            vec![],
            200,
            vec![],
            "<!doctype html><html><body>first</body></html>".into(),
        )
        .await
        .expect("first context navigation should build")
        .page;
    first
        .evaluate_runtime_expression_async(
            "localStorage.clear(); sessionStorage.clear(); localStorage.setItem('contextOnly', 'first'); sessionStorage.setItem('sessionOnly', 'first');",
        )
        .await
        .expect("first context storage write should evaluate");

    assert!(conn.activate_browser_context_by_id_async("BID-2").await);
    let mut second = conn
        .build_loaded_navigation_from_buffered_response_async(
            url,
            "GET".into(),
            vec![],
            200,
            vec![],
            "<!doctype html><html><body>second</body></html>".into(),
        )
        .await
        .expect("second context navigation should build")
        .page;
    let read = second
        .evaluate_runtime_expression_async(
            "`${String(localStorage.getItem('contextOnly'))}|${String(sessionStorage.getItem('sessionOnly'))}`",
        )
        .await
        .expect("second context storage read should evaluate");

    assert_eq!(read["value"], json!("null|null"));
}

#[tokio::test]
async fn browser_context_storage_buckets_reuse_within_context_and_isolate_between_contexts() {
    let mut conn = CdpConnection::new();
    let context_a = conn.new_ephemeral_browser_context("BID-1".into());
    let context_b = conn.new_ephemeral_browser_context("BID-2".into());
    conn.browser_context = Some(context_a);
    conn.inactive_browser_contexts.push(context_b);
    let first_url = Url::parse("https://context-storage-buckets.example/app/one").unwrap();
    let second_url = Url::parse("https://context-storage-buckets.example/app/two").unwrap();

    let mut first = conn
        .build_loaded_navigation_from_buffered_response_async(
            first_url.clone(),
            "GET".into(),
            vec![],
            200,
            vec![],
            "<!doctype html><html><body>first</body></html>".into(),
        )
        .await
        .expect("first context navigation should build")
        .page;
    let write = first
        .evaluate_runtime_expression_with_await_async(
            r#"
(async () => {
  await navigator.storageBuckets.open("bucket-a");
  await navigator.storageBuckets.open("bucket-b");
  return (await navigator.storageBuckets.keys()).join("|");
})()
"#,
            true,
        )
        .await
        .expect("first context storage bucket write should evaluate");
    assert_eq!(write["value"], json!("bucket-a|bucket-b"));

    let mut same_context = conn
        .build_loaded_navigation_from_buffered_response_async(
            second_url.clone(),
            "GET".into(),
            vec![],
            200,
            vec![],
            "<!doctype html><html><body>same context</body></html>".into(),
        )
        .await
        .expect("same context navigation should build")
        .page;
    let same_context_keys = same_context
        .evaluate_runtime_expression_with_await_async(
            r#"
(async () => (await navigator.storageBuckets.keys()).join("|"))()
"#,
            true,
        )
        .await
        .expect("same context storage bucket read should evaluate");
    assert_eq!(same_context_keys["value"], json!("bucket-a|bucket-b"));

    assert!(conn.activate_browser_context_by_id_async("BID-2").await);
    let mut other_context = conn
        .build_loaded_navigation_from_buffered_response_async(
            second_url,
            "GET".into(),
            vec![],
            200,
            vec![],
            "<!doctype html><html><body>other context</body></html>".into(),
        )
        .await
        .expect("other context navigation should build")
        .page;
    let other_context_keys = other_context
        .evaluate_runtime_expression_with_await_async(
            r#"
(async () => (await navigator.storageBuckets.keys()).join("|"))()
"#,
            true,
        )
        .await
        .expect("other context storage bucket read should evaluate");
    assert_eq!(other_context_keys["value"], json!(""));
}

#[tokio::test]
async fn user_agent_override_rebinds_live_document_after_engine_runtime_invalidation() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    let url = Url::parse("https://example.com/app").unwrap();

    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            url.clone(),
            "GET".into(),
            vec![],
            200,
            vec![("set-cookie".into(), "theme=dark; Path=/".into())],
            "<!doctype html><html><body>ok</body></html>".into(),
        )
        .await
        .expect("navigation should build");
    conn.browser_context
        .as_mut()
        .unwrap()
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(navigation.page);

    // Invalidate only the NavigationEngine's cached browser runtime. The
    // committed Document keeps its exact lifecycle authority so the setting
    // update can replace that authority's transport view.
    conn.invalidate_resource_runtime();
    conn.set_user_agent_override_async("Moli/Reset").await;

    let page = conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_target
        .runtime_slot
        .loaded_page_mut()
        .unwrap();
    let payload = page
        .evaluate_runtime_expression_async("document.cookie")
        .await
        .expect("cookie read should succeed after loader rebuild");
    assert_eq!(payload["value"], json!("theme=dark"));
}

#[tokio::test]
async fn tls_and_proxy_overrides_rebind_live_document_after_engine_runtime_invalidation() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    let url = Url::parse("https://example.com/app").unwrap();

    let navigation = conn
        .build_loaded_navigation_from_buffered_response_async(
            url.clone(),
            "GET".into(),
            vec![],
            200,
            vec![("set-cookie".into(), "theme=dark; Path=/".into())],
            "<!doctype html><html><body>ok</body></html>".into(),
        )
        .await
        .expect("navigation should build");
    conn.browser_context
        .as_mut()
        .unwrap()
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(navigation.page);

    // Network settings rebuild the transport behind the live Document
    // authority; they must not retire that authority first.
    conn.invalidate_resource_runtime();
    conn.set_tls_verify_host_async(false).await;
    let page = conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_target
        .runtime_slot
        .loaded_page_mut()
        .unwrap();
    let after_tls = page
        .evaluate_runtime_expression_async("document.cookie")
        .await
        .expect("cookie read should succeed after tls rebuild");
    assert_eq!(after_tls["value"], json!("theme=dark"));

    conn.invalidate_resource_runtime();
    conn.set_http_proxy_override_async(Some("http://proxy.test:8080".into()))
        .await;
    let page = conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_target
        .runtime_slot
        .loaded_page_mut()
        .unwrap();
    let after_proxy = page
        .evaluate_runtime_expression_async("document.cookie")
        .await
        .expect("cookie read should succeed after proxy rebuild");
    assert_eq!(after_proxy["value"], json!("theme=dark"));
}

#[test]
fn build_loaded_navigation_from_buffered_response_works_inside_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut conn = CdpConnection::new();
        conn.browser_context = Some(BrowserContext::new("BID-1".into()));
        let url = Url::parse("https://example.com/app").unwrap();

        let mut navigation = conn
            .build_loaded_navigation_from_buffered_response_async(
                url.clone(),
                "GET".into(),
                vec![],
                200,
                vec![("content-type".into(), "text/html".into())],
                "<!doctype html><html><body><main id='ok'>ok</main></body></html>".into(),
            )
            .await
            .expect("navigation should build inside current-thread runtime");

        assert_eq!(navigation.final_url, url);
        assert_eq!(navigation.response_status, 200);
        assert_eq!(
            navigation
                .page
                .evaluate_runtime_expression_async("document.getElementById('ok').textContent")
                .await
                .expect("dom evaluation should succeed")["value"],
            json!("ok")
        );
    });
}

#[tokio::test]
async fn loader_uses_active_browser_context_user_agent_override() {
    let mut conn = CdpConnection::new();
    let mut first = BrowserContext::new("BID-1".into());
    first
        .network_policy
        .set_user_agent_override("Moli/Context-A".into());
    conn.browser_context = Some(first);

    let mut second = BrowserContext::new("BID-2".into());
    second
        .network_policy
        .set_user_agent_override("Moli/Context-B".into());
    conn.inactive_browser_contexts.push(second);

    assert_eq!(
        conn.ensure_resource_request_client()
            .expect("loader for first context")
            .user_agent(),
        "Moli/Context-A"
    );

    assert!(conn.activate_browser_context_by_id_async("BID-2").await);
    assert_eq!(
        conn.ensure_resource_request_client()
            .expect("loader for second context")
            .user_agent(),
        "Moli/Context-B"
    );
}

#[tokio::test]
async fn loader_uses_active_browser_context_http_proxy_override() {
    let mut conn = CdpConnection::new();
    let mut first = BrowserContext::new("BID-1".into());
    first.http_proxy_override = Some("http://proxy-a.test:8080".into());
    conn.browser_context = Some(first);

    let mut second = BrowserContext::new("BID-2".into());
    second.http_proxy_override = Some("http://proxy-b.test:8080".into());
    conn.inactive_browser_contexts.push(second);

    assert_eq!(
        conn.ensure_resource_request_client()
            .expect("loader for first context")
            .http_proxy(),
        Some("http://proxy-a.test:8080")
    );

    assert!(conn.activate_browser_context_by_id_async("BID-2").await);
    assert_eq!(
        conn.ensure_resource_request_client()
            .expect("loader for second context")
            .http_proxy(),
        Some("http://proxy-b.test:8080")
    );
}

#[tokio::test]
async fn loader_uses_active_browser_context_http_no_proxy_override() {
    let mut conn = CdpConnection::new();
    let mut first = BrowserContext::new("BID-1".into());
    first.http_no_proxy_override = Some("localhost,127.0.0.1".into());
    conn.browser_context = Some(first);

    let mut second = BrowserContext::new("BID-2".into());
    second.http_no_proxy_override = Some("::1,.example.com".into());
    conn.inactive_browser_contexts.push(second);

    assert_eq!(
        conn.ensure_resource_request_client()
            .expect("loader for first context")
            .http_no_proxy(),
        Some("localhost,127.0.0.1")
    );

    assert!(conn.activate_browser_context_by_id_async("BID-2").await);
    assert_eq!(
        conn.ensure_resource_request_client()
            .expect("loader for second context")
            .http_no_proxy(),
        Some("::1,.example.com")
    );
}

#[tokio::test]
async fn loader_uses_active_browser_context_tls_verify_host_override() {
    let mut conn = CdpConnection::new();
    let mut first = BrowserContext::new("BID-1".into());
    first.tls_verify_host_override = Some(false);
    conn.browser_context = Some(first);

    let mut second = BrowserContext::new("BID-2".into());
    second.tls_verify_host_override = Some(true);
    conn.inactive_browser_contexts.push(second);

    assert!(
        !conn
            .ensure_resource_request_client()
            .expect("loader for first context")
            .tls_verify_host()
    );

    assert!(conn.activate_browser_context_by_id_async("BID-2").await);
    assert!(
        conn.ensure_resource_request_client()
            .expect("loader for second context")
            .tls_verify_host()
    );
}

#[tokio::test]
async fn removing_an_inactive_browser_context_keeps_the_previously_active_context() {
    let mut conn = CdpConnection::new();

    let mut first = BrowserContext::new("BID-A".into());
    first
        .network_policy
        .set_user_agent_override("Moli/Context-A".into());
    conn.browser_context = Some(first);

    let mut second = BrowserContext::new("BID-B".into());
    second
        .network_policy
        .set_user_agent_override("Moli/Context-B".into());
    conn.inactive_browser_contexts.push(second);

    let mut third = BrowserContext::new("BID-C".into());
    third
        .network_policy
        .set_user_agent_override("Moli/Context-C".into());
    conn.inactive_browser_contexts.push(third);

    assert!(conn.activate_browser_context_by_id_async("BID-B").await);
    assert_eq!(conn.browser_context.as_ref().unwrap().id, "BID-B");

    let removed = conn
        .remove_browser_context_by_id_restoring_active_async("BID-B", Some("BID-A"))
        .await
        .expect("inactive context should be removable after selection");
    assert_eq!(removed.id, "BID-B");

    assert_eq!(
        conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-A"),
        "disposing an inactive context via the Target path should restore the context that was active before selection"
    );
    assert!(
        conn.inactive_browser_contexts
            .iter()
            .any(|bc| bc.id == "BID-C"),
        "the remaining third context should stay inactive"
    );
    assert_eq!(
        conn.ensure_resource_request_client()
            .expect("loader should rebuild for the restored active context")
            .user_agent(),
        "Moli/Context-A"
    );
}

#[tokio::test]
async fn manual_browser_context_restore_reselects_original_context_after_switch() {
    let mut conn = CdpConnection::new();

    let mut first = BrowserContext::new("BID-A".into());
    first
        .network_policy
        .set_user_agent_override("Moli/Context-A".into());
    conn.browser_context = Some(first);

    let mut second = BrowserContext::new("BID-B".into());
    second
        .network_policy
        .set_user_agent_override("Moli/Context-B".into());
    conn.inactive_browser_contexts.push(second);

    let previously_active_browser_context_id =
        conn.browser_context.as_ref().map(|bc| bc.id.clone());
    assert!(conn.activate_browser_context_by_id_async("BID-B").await);
    assert_eq!(
        conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-B")
    );
    if let Some(browser_context_id) = previously_active_browser_context_id.as_deref()
        && conn.has_browser_context_id(browser_context_id)
        && conn
            .browser_context
            .as_ref()
            .is_none_or(|bc| bc.id != browser_context_id)
    {
        let _ = conn
            .activate_browser_context_by_id_async(browser_context_id)
            .await;
    }

    assert_eq!(
        conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-A"),
        "scoped browser context switching should restore the original active context after the operation"
    );
    assert_eq!(
        conn.ensure_resource_request_client()
            .expect("loader should rebuild for the restored browser context")
            .user_agent(),
        "Moli/Context-A"
    );
}

#[tokio::test]
async fn session_scoped_process_message_restores_previously_active_context_after_dispatch() {
    let mut conn = CdpConnection::new();

    let first = BrowserContext::new("BID-A".into());
    conn.browser_context = Some(first);

    let mut second = BrowserContext::new("BID-B".into());
    second.attach_active_session("SID-B");
    conn.inactive_browser_contexts.push(second);

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":1,"method":"Network.enable","sessionId":"SID-B"}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {}, "sessionId": "SID-B"})]
    );

    assert_eq!(
        conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-A"),
        "dispatching a session-scoped command must not leave that session's browser context selected as the default active context"
    );
    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("session browser context should remain inactive after dispatch");
    assert!(
        inactive
            .active_target
            .runtime_slot
            .primary_network_events_enabled()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn session_scoped_process_message_async_restores_previously_active_context_after_dispatch() {
    let mut conn = CdpConnection::new();

    let first = BrowserContext::new("BID-A".into());
    conn.browser_context = Some(first);

    let mut second = BrowserContext::new("BID-B".into());
    second.attach_active_session("SID-B");
    conn.inactive_browser_contexts.push(second);

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":1,"method":"Network.enable","sessionId":"SID-B"}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {}, "sessionId": "SID-B"})]
    );

    assert_eq!(
        conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-A"),
        "async dispatching a session-scoped command must not leave that session's browser context selected as the default active context"
    );
    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("session browser context should remain inactive after async dispatch");
    assert!(
        inactive
            .active_target
            .runtime_slot
            .primary_network_events_enabled()
    );
}

#[tokio::test]
async fn direct_network_enable_routes_to_inactive_active_owner_without_promoting_slot() {
    let mut conn = CdpConnection::new();

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.set_active_target_id("TID-B".to_owned());
    inactive.attach_active_session("SID-B");
    conn.inactive_browser_contexts.push(inactive);

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":1,"method":"Network.enable","sessionId":"SID-B"}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {}, "sessionId": "SID-B"})]
    );
    assert!(
        conn.browser_context.is_none(),
        "direct Network.enable should not promote the inactive owner into the active slot"
    );
    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(
        inactive
            .active_target
            .runtime_slot
            .primary_network_events_enabled()
    );
}

#[tokio::test]
async fn direct_runtime_evaluate_routes_to_inactive_active_owner_without_promoting_slot() {
    let mut ctx = crate::testing::TestContext::new();

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.set_active_target_id("TID-B".to_owned());
    inactive.attach_active_session("SID-B");
    inactive
        .devtools_session_state
        .runtime_session_state
        .runtime_frontend_enabled = true;
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<!doctype html><title>runtime-direct-owner</title>",
        Some("SID-B"),
    )
    .await;

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "sessionId": "SID-B",
        "params": {"expression": "document.title", "returnByValue": true}
    }))
    .await;
    let response = std::mem::take(&mut ctx.sent);
    let response = response
        .iter()
        .find(|message| message["id"] == json!(1))
        .unwrap_or_else(|| panic!("missing Runtime.evaluate response: {response:?}"));
    assert_eq!(response["id"], json!(1));
    assert_eq!(
        response["result"]["result"]["value"],
        json!("runtime-direct-owner"),
        "{response:?}"
    );
    assert_eq!(response["sessionId"], json!("SID-B"));
    assert!(
        ctx.conn.browser_context.is_none(),
        "direct Runtime.evaluate should not promote the inactive owner into the active slot"
    );
}

#[tokio::test]
async fn direct_runtime_evaluate_document_replacement_lifecycle_uses_inactive_owner() {
    let mut ctx = crate::testing::TestContext::new();
    let mut inactive = BrowserContext::new("BID-document-replacement".into());
    inactive.set_active_target_id("TID-document-replacement".to_owned());
    inactive.attach_active_session("SID-document-replacement");
    inactive
        .devtools_session_state
        .runtime_session_state
        .runtime_frontend_enabled = true;
    inactive.devtools_session_state.dom_session_state.enabled = true;
    inactive
        .devtools_session_state
        .page_session_state
        .page_lifecycle_events = true;
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<!doctype html><body>before</body>",
        Some("SID-document-replacement"),
    )
    .await;

    ctx.process_async(json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "sessionId": "SID-document-replacement",
        "params": {
            "expression": "document.open(); document.write('<main id=\"after\">after</main>'); document.close(); 'done';",
            "returnByValue": true
        }
    }))
    .await;
    crate::testing::wait_until_message(
        &mut ctx,
        "SID-document-replacement",
        "inactive owner document replacement DCL",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("DOMContentLoaded")
        },
    )
    .await;
    let response = ctx.take_all();

    assert!(
        response.iter().any(|message| {
            message["id"] == json!(1)
                && message["sessionId"] == json!("SID-document-replacement")
                && message["result"]["result"]["value"] == json!("done")
        }),
        "Runtime.evaluate should complete under the inactive owner: {response:?}"
    );
    assert!(
        response.iter().any(|message| {
            message["sessionId"] == json!("SID-document-replacement")
                && message["method"] == json!("DOM.documentUpdated")
        }),
        "document replacement should emit DOM.documentUpdated for the inactive owner: {response:?}"
    );
    assert!(
        response.iter().any(|message| {
            message["sessionId"] == json!("SID-document-replacement")
                && message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("DOMContentLoaded")
                && message["params"]["frameId"] == json!("TID-document-replacement")
        }),
        "document replacement lifecycle should use the inactive owner frame id: {response:?}"
    );
    assert!(
        ctx.conn.browser_context.is_none(),
        "direct Runtime.evaluate document replacement should not activate the inactive owner"
    );
}

#[test]
fn devtools_document_lifecycle_wait_key_observes_interruption_and_target_loss() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-lifecycle-wait".into());
    browser_context.set_active_target_id("TID-lifecycle-wait".to_owned());
    browser_context.attach_active_session("SID-lifecycle-wait");
    browser_context
        .active_target
        .runtime_slot
        .set_page_attachment_id_for_test(901);
    conn.browser_context = Some(browser_context);

    let page_id = moli_core::PageId::new_for_testing(901);
    let frame = moli_core::page::RendererFrameToken { page_id };
    let document = moli_core::page::RendererDocumentToken::new_for_testing(page_id, 1);
    let epoch = moli_core::page::RendererLifecycleEpoch(1);
    let started = moli_core::page::RendererDocumentLifecycleEvent {
        frame,
        document,
        epoch,
        sequence: 1,
        timestamp_micros: 10,
        kind: moli_core::page::RendererDocumentLifecycleEventKind::Started {
            reason: moli_core::page::RendererLifecycleStartReason::InitialDocument,
        },
    };
    let dcl = moli_core::page::RendererDocumentLifecycleEvent {
        sequence: 2,
        timestamp_micros: 20,
        kind: moli_core::page::RendererDocumentLifecycleEventKind::Milestone(
            moli_core::page::RendererDocumentLifecycleMilestone::DomContentLoaded,
        ),
        ..started
    };
    let (_, accepted) = conn.bind_renderer_document_lifecycle_for_session_owner(
        Some("SID-lifecycle-wait"),
        moli_core::page::RendererPageCreationArtifacts {
            active_document: document,
            active_epoch: epoch,
            lifecycle_snapshot: moli_core::page::RendererDocumentLifecycleSnapshot {
                frame,
                document,
                epoch,
                started: moli_core::page::RendererLifecycleEventStamp {
                    sequence: 1,
                    timestamp_micros: 10,
                },
                dom_content_loaded: Some(moli_core::page::RendererLifecycleEventStamp {
                    sequence: 2,
                    timestamp_micros: 20,
                }),
                load: None,
                terminated: None,
            },
            initial_lifecycle_events: vec![started, dcl],
        },
        None,
        "TID-lifecycle-wait".to_owned(),
        "LID-lifecycle-wait".to_owned(),
    );
    assert_eq!(accepted, vec![started, dcl]);

    let context = crate::devtools_runtime::DevToolsCommandContext {
        protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverBidi,
        session_id: Some(crate::devtools_runtime::DevToolsSessionId::from(
            "SID-lifecycle-wait",
        )),
        target_id: Some(crate::devtools_runtime::DevToolsTargetId::from(
            "TID-lifecycle-wait",
        )),
        browser_context_id: None,
    };
    assert!(conn.devtools_context_routes_to_top_level_target(&context));
    let dcl_key = conn
        .capture_devtools_document_lifecycle_wait_key(
            &context,
            "LID-lifecycle-wait",
            moli_core::page::RendererDocumentLifecycleMilestone::DomContentLoaded,
        )
        .expect("committed root document DCL wait key");
    assert_eq!(
        dcl_key.milestone(),
        moli_core::page::RendererDocumentLifecycleMilestone::DomContentLoaded
    );
    assert_eq!(
        conn.devtools_document_lifecycle_wait_state(&context, &dcl_key),
        DevToolsDocumentLifecycleWaitState::Reached,
        "a waiter registered after DCL must observe the committed lifecycle snapshot"
    );
    assert!(conn.release_devtools_document_lifecycle_wait_key(&context, &dcl_key));

    let key = conn
        .capture_devtools_document_lifecycle_wait_key(
            &context,
            "LID-lifecycle-wait",
            moli_core::page::RendererDocumentLifecycleMilestone::Load,
        )
        .expect("committed root document wait key");
    assert_eq!(
        key.milestone(),
        moli_core::page::RendererDocumentLifecycleMilestone::Load
    );
    assert_eq!(
        conn.devtools_document_lifecycle_wait_state(&context, &key),
        DevToolsDocumentLifecycleWaitState::Pending
    );

    let terminated = moli_core::page::RendererDocumentLifecycleEvent {
        sequence: 3,
        timestamp_micros: 30,
        kind: moli_core::page::RendererDocumentLifecycleEventKind::Terminated {
            last_reached: Some(
                moli_core::page::RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
            reason: moli_core::page::RendererDocumentTerminationReason::Stopped,
        },
        ..started
    };
    let (_, accepted) = conn.ingest_renderer_document_lifecycle_events_for_session_owner(
        Some("SID-lifecycle-wait"),
        vec![terminated],
    );
    assert_eq!(accepted, vec![terminated]);
    assert_eq!(
        conn.devtools_document_lifecycle_wait_state(&context, &key),
        DevToolsDocumentLifecycleWaitState::Interrupted
    );

    conn.browser_context = None;
    assert_eq!(
        conn.devtools_document_lifecycle_wait_state(&context, &key),
        DevToolsDocumentLifecycleWaitState::Unavailable
    );

    let mut replacement_context = BrowserContext::new("BID-other".into());
    replacement_context.set_active_target_id("TID-other".to_owned());
    replacement_context.attach_active_session("SID-lifecycle-wait");
    replacement_context
        .active_target
        .runtime_slot
        .set_page_attachment_id_for_test(902);
    conn.browser_context = Some(replacement_context);
    let (_, accepted) = conn.bind_renderer_document_lifecycle_for_session_owner(
        Some("SID-lifecycle-wait"),
        moli_core::page::RendererPageCreationArtifacts {
            active_document: document,
            active_epoch: epoch,
            lifecycle_snapshot: moli_core::page::RendererDocumentLifecycleSnapshot {
                frame,
                document,
                epoch,
                started: moli_core::page::RendererLifecycleEventStamp {
                    sequence: 1,
                    timestamp_micros: 10,
                },
                dom_content_loaded: Some(moli_core::page::RendererLifecycleEventStamp {
                    sequence: 2,
                    timestamp_micros: 20,
                }),
                load: None,
                terminated: None,
            },
            initial_lifecycle_events: vec![started, dcl],
        },
        None,
        "TID-other".to_owned(),
        "LID-other".to_owned(),
    );
    assert_eq!(accepted, vec![started, dcl]);
    assert_eq!(
        conn.devtools_document_lifecycle_wait_state(&context, &key),
        DevToolsDocumentLifecycleWaitState::Unavailable,
        "a missing targetId route must not fall back to a same-named session on another target"
    );
}

#[tokio::test]
async fn direct_runtime_evaluate_same_document_navigation_updates_inactive_owner() {
    let mut ctx = crate::testing::TestContext::new();
    let initial_url = "data:text/html,<!doctype html><title>same-doc</title>".to_owned();
    let mut inactive = BrowserContext::new("BID-same-document".into());
    inactive.set_active_target_id("TID-same-document".to_owned());
    inactive.attach_active_session("SID-same-document");
    inactive
        .devtools_session_state
        .runtime_session_state
        .runtime_frontend_enabled = true;
    inactive.set_target_url(initial_url.clone());
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(&initial_url, Some("SID-same-document"))
        .await;

    ctx.process_and_wait_for_response_async(json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "sessionId": "SID-same-document",
        "params": {
            "expression": "location.hash = 'owner-fragment'; 'done';",
            "returnByValue": true
        }
    }))
    .await;
    let response = ctx.take_all();

    assert!(
        response.iter().any(|message| {
            message["id"] == json!(1)
                && message["sessionId"] == json!("SID-same-document")
                && message["result"]["result"]["value"] == json!("done")
        }),
        "Runtime.evaluate should complete under the inactive owner: {response:?}"
    );
    let navigation = response
        .iter()
        .find(|message| {
            message["sessionId"] == json!("SID-same-document")
                && message["method"] == json!("Page.navigatedWithinDocument")
        })
        .unwrap_or_else(|| {
            panic!("same-document navigation should emit for inactive owner: {response:?}")
        });
    assert_eq!(
        navigation["params"]["frameId"],
        json!("TID-same-document"),
        "same-document navigation should use inactive owner frame id"
    );
    assert!(
        navigation["params"]["url"]
            .as_str()
            .is_some_and(|url| url.ends_with("#owner-fragment")),
        "same-document navigation should carry updated fragment URL: {navigation:?}"
    );
    let inactive = ctx
        .conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-same-document")
        .expect("inactive owner should remain parked");
    assert!(
        inactive.target_url().ends_with("#owner-fragment"),
        "same-document navigation should update the inactive owner target URL"
    );
    assert!(
        ctx.conn.browser_context.is_none(),
        "direct Runtime.evaluate same-document navigation should not activate the inactive owner"
    );
}

#[tokio::test]
async fn direct_runtime_evaluate_javascript_dialog_uses_inactive_background_owner() {
    let mut ctx = crate::testing::TestContext::new();
    let page_url = "data:text/html,<!doctype html><title>dialog-owner</title>".to_owned();
    let background = BackgroundTarget::with_url(
        "TID-dialog-background".to_owned(),
        Some("SID-dialog-background".to_owned()),
        page_url.clone(),
    );

    let mut inactive = BrowserContext::new("BID-dialog-background".into());
    inactive.background_targets.push(background);
    inactive.mutate_parked_page_session_state("TID-dialog-background", |state| {
        state
            .devtools_session_state
            .runtime_session_state
            .runtime_frontend_enabled = true;
    });
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-dialog-background"))
        .await;

    ctx.process_and_wait_for_response_async(json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "sessionId": "SID-dialog-background",
        "params": {
            "expression": "alert('owner dialog'); 'done';",
            "returnByValue": true
        }
    }))
    .await;
    let response = ctx.take_all();

    assert!(
        response.iter().any(|message| {
            message["id"] == json!(1)
                && message["sessionId"] == json!("SID-dialog-background")
                && message["result"]["result"]["value"] == json!("done")
        }),
        "Runtime.evaluate should complete under the inactive background owner: {response:?}"
    );
    assert!(
        response.iter().any(|message| {
            message["sessionId"] == json!("SID-dialog-background")
                && message["method"] == json!("Page.javascriptDialogOpening")
                && message["params"]["frameId"] == json!("TID-dialog-background")
                && message["params"]["url"] == json!(page_url)
                && message["params"]["message"] == json!("owner dialog")
        }),
        "JavaScript dialog opening should use the background owner identity: {response:?}"
    );
    assert!(
        ctx.conn.browser_context.is_none(),
        "direct Runtime.evaluate dialog output should not activate the inactive owner"
    );

    ctx.process_async(json!({
        "id": 2,
        "method": "Page.handleJavaScriptDialog",
        "sessionId": "SID-dialog-background",
        "params": { "accept": true }
    }))
    .await;
    let response = ctx.take_all();

    assert!(
        response.iter().any(|message| {
            message["sessionId"] == json!("SID-dialog-background")
                && message["method"] == json!("Page.javascriptDialogClosed")
                && message["params"]["frameId"] == json!("TID-dialog-background")
                && message["params"]["result"] == json!(true)
        }),
        "Page.handleJavaScriptDialog should close the owner dialog without promotion: {response:?}"
    );
    assert!(
        response.iter().any(|message| {
            message["id"] == json!(2)
                && message["sessionId"] == json!("SID-dialog-background")
                && message["result"] == json!({})
        }),
        "Page.handleJavaScriptDialog should resolve under the owner session: {response:?}"
    );
    assert!(
        ctx.conn.browser_context.is_none(),
        "direct Page.handleJavaScriptDialog should not activate the inactive owner"
    );
    let inactive = ctx
        .conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-dialog-background")
        .expect("inactive owner should remain parked");
    assert!(
        inactive
            .parked_page_session_state("TID-dialog-background")
            .expect("background page session state should remain parked")
            .devtools_session_state
            .page_session_state
            .javascript_dialog_state
            .is_empty(),
        "handling the dialog should pop the parked session dialog queue"
    );
}

#[tokio::test]
async fn direct_runtime_evaluate_popup_creates_target_in_inactive_background_owner() {
    let mut ctx = crate::testing::TestContext::new();
    let page_url = "data:text/html,<!doctype html><title>popup-owner</title>";
    let background = BackgroundTarget::with_url(
        "TID-popup-background".to_owned(),
        Some("SID-popup-background".to_owned()),
        page_url.to_owned(),
    );

    let mut inactive = BrowserContext::new("BID-popup-background".into());
    inactive.background_targets.push(background);
    inactive.mutate_parked_page_session_state("TID-popup-background", |state| {
        state
            .devtools_session_state
            .runtime_session_state
            .runtime_frontend_enabled = true;
    });
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(page_url, Some("SID-popup-background"))
        .await;

    // The Runtime response must cross the command's exact Page cursor before
    // the already-frozen popup owner action is released after that response.
    ctx.process_and_wait_for_response_async(json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "sessionId": "SID-popup-background",
        "params": {
            "expression": "window.open('https://example.com/owner-popup', '_blank') !== null"
        }
    }))
    .await;
    let response = &ctx.sent;

    assert!(
        response.iter().any(|message| {
            message["id"] == json!(1)
                && message["sessionId"] == json!("SID-popup-background")
                && message["result"]["result"]["value"] == json!(true)
        }),
        "Runtime.evaluate should complete under the inactive background owner: {response:?}"
    );
    let created = response
        .iter()
        .find(|message| message["method"] == json!("Target.targetCreated"))
        .unwrap_or_else(|| {
            panic!("window.open should create a popup target in the owner context: {response:?}")
        });
    assert_eq!(
        created["params"]["targetInfo"]["browserContextId"],
        json!("BID-popup-background")
    );
    assert_eq!(
        created["params"]["targetInfo"]["url"],
        json!("https://example.com/owner-popup")
    );
    assert_eq!(
        created["params"]["targetInfo"]["openerId"],
        json!("TID-popup-background")
    );
    assert!(
        ctx.conn.browser_context.is_none(),
        "direct Runtime.evaluate popup output should not activate the inactive owner"
    );
    let popup_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id")
        .to_owned();
    let inactive = ctx
        .conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-popup-background")
        .expect("inactive owner should remain parked");
    assert!(
        inactive
            .background_target(&popup_target_id)
            .is_some_and(|target| target.target_url() == "https://example.com/owner-popup"),
        "popup target should be staged in the inactive owner browser context"
    );
}

#[tokio::test]
async fn direct_runtime_evaluate_self_popup_does_not_navigate_active_target_for_inactive_owner() {
    let mut ctx = crate::testing::TestContext::new();

    let mut active = BrowserContext::new("BID-active".into());
    active.set_active_target_id("TID-active".to_owned());
    active.attach_active_session("SID-active");
    active.set_target_url("https://active.example/current".to_owned());
    ctx.conn.browser_context = Some(active);

    let mut page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<!doctype html><title>self-popup</title>")
        .await
        .expect("background page should load");
    let _ = page
        .dispatch_runtime_protocol_message_async(
            &json!({"id": 9007, "method": "Runtime.enable", "params": {}}).to_string(),
        )
        .await
        .expect("Runtime.enable should create the owner inspector context");
    let mut background = BackgroundTarget::with_url(
        "TID-self-popup-background".to_owned(),
        Some("SID-self-popup-background".to_owned()),
        page.final_url().as_str().to_owned(),
    );
    background.replace_loaded_page(Some(page));

    let mut inactive = BrowserContext::new("BID-self-popup-background".into());
    inactive.background_targets.push(background);
    inactive.mutate_parked_page_session_state("TID-self-popup-background", |state| {
        state
            .devtools_session_state
            .runtime_session_state
            .runtime_frontend_enabled = true;
    });
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "sessionId": "SID-self-popup-background",
        "params": {
            "expression": "window.open('https://example.com/should-not-hit-active', '_self') !== null"
        }
    }))
    .await;
    let response = std::mem::take(&mut ctx.sent);

    assert!(
        response.iter().any(|message| {
            message["id"] == json!(1)
                && message["sessionId"] == json!("SID-self-popup-background")
                && message["result"]["result"]["value"] == json!(true)
        }),
        "Runtime.evaluate should return the inactive owner's existing WindowProxy: {response:?}"
    );
    assert!(
        response
            .iter()
            .all(|message| message["method"] != json!("Target.targetCreated")),
        "_self window.open should not create a popup target: {response:?}"
    );
    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.target_url()),
        Some("https://active.example/current"),
        "inactive owner _self popup must not navigate the currently active target"
    );
}

#[tokio::test]
async fn direct_runtime_evaluate_file_chooser_uses_inactive_background_owner() {
    let mut ctx = crate::testing::TestContext::new();
    let page_url = "data:text/html,<!doctype html><input id='picker' type='file' multiple>";
    let background = BackgroundTarget::with_url(
        "TID-file-background".to_owned(),
        Some("SID-file-background".to_owned()),
        page_url.to_owned(),
    );

    let mut inactive = BrowserContext::new("BID-file-background".into());
    inactive.background_targets.push(background);
    inactive.mutate_parked_page_session_state("TID-file-background", |state| {
        state
            .devtools_session_state
            .runtime_session_state
            .runtime_frontend_enabled = true;
        state
            .devtools_session_state
            .page_session_state
            .page_file_chooser_opened_event_enabled = true;
    });
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(page_url, Some("SID-file-background"))
        .await;

    ctx.process_and_wait_for_response_async(json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "sessionId": "SID-file-background",
        "params": {
            "expression": "document.getElementById('picker').click(); 'done';",
            "returnByValue": true
        }
    }))
    .await;
    let response = ctx.take_all();

    assert!(
        response.iter().any(|message| {
            message["id"] == json!(1)
                && message["sessionId"] == json!("SID-file-background")
                && message["result"]["result"]["value"] == json!("done")
        }),
        "Runtime.evaluate should complete under the inactive background owner: {response:?}"
    );
    let file_chooser = response
        .iter()
        .find(|message| {
            message["sessionId"] == json!("SID-file-background")
                && message["method"] == json!("Page.fileChooserOpened")
                && message["params"]["frameId"] == json!("TID-file-background")
                && message["params"]["mode"] == json!("selectMultiple")
        })
        .unwrap_or_else(|| {
            panic!(
                "file chooser event should use the inactive background owner identity: {response:?}"
            )
        });
    let backend_node_id = file_chooser["params"]["backendNodeId"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .unwrap_or_else(|| {
            panic!("file chooser event should include u32 backendNodeId: {file_chooser:?}")
        });
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "file chooser backendNodeId should use renderer registry namespace: {file_chooser:?}"
    );
    assert!(
        ctx.conn.browser_context.is_none(),
        "direct Runtime.evaluate file chooser output should not activate the inactive owner"
    );
}

#[tokio::test]
async fn direct_runtime_evaluate_routes_to_inactive_auxiliary_owner_without_promoting_slot() {
    let mut ctx = crate::testing::TestContext::new();

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.set_active_target_id("TID-B".to_owned());
    inactive.attach_active_session("SID-primary");
    assert!(inactive.assign_auxiliary_session_to_target("TID-B", "SID-aux".to_owned()));
    inactive
        .devtools_session_state
        .runtime_session_state
        .runtime_frontend_enabled = true;
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<!doctype html><title>runtime-direct-aux</title>",
        Some("SID-aux"),
    )
    .await;

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "sessionId": "SID-aux",
        "params": {"expression": "document.title", "returnByValue": true}
    }))
    .await;
    let response = std::mem::take(&mut ctx.sent);
    let response = response
        .iter()
        .find(|message| message["id"] == json!(1))
        .unwrap_or_else(|| panic!("missing Runtime.evaluate response: {response:?}"));
    assert_eq!(response["id"], json!(1));
    assert_eq!(
        response["result"]["result"]["value"],
        json!("runtime-direct-aux"),
        "{response:?}"
    );
    assert_eq!(response["sessionId"], json!("SID-aux"));
    assert!(
        ctx.conn.browser_context.is_none(),
        "direct auxiliary Runtime.evaluate should not promote the inactive owner"
    );
}

#[tokio::test]
async fn direct_network_enable_disable_routes_to_inactive_auxiliary_owner_without_promoting_slot() {
    let mut conn = CdpConnection::new();

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.set_active_target_id("TID-B".to_owned());
    inactive.attach_active_session("SID-primary");
    assert!(inactive.assign_auxiliary_session_to_target("TID-B", "SID-aux".to_owned()));
    conn.inactive_browser_contexts.push(inactive);

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":1,"method":"Network.enable","sessionId":"SID-aux"}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {}, "sessionId": "SID-aux"})]
    );
    assert!(
        conn.browser_context.is_none(),
        "direct auxiliary Network.enable should not promote the inactive owner"
    );
    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(
        !inactive
            .active_target
            .runtime_slot
            .primary_network_events_enabled(),
        "auxiliary Network.enable must not enable the target's primary listener"
    );
    assert!(
        inactive
            .active_target
            .runtime_slot
            .has_auxiliary_network_events_for_session("SID-aux")
    );

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":2,"method":"Network.disable","sessionId":"SID-aux"}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 2, "result": {}, "sessionId": "SID-aux"})]
    );
    assert!(
        conn.browser_context.is_none(),
        "direct auxiliary Network.disable should not promote the inactive owner"
    );
    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(
        !inactive
            .active_target
            .runtime_slot
            .has_auxiliary_network_events_for_session("SID-aux")
    );
}

#[tokio::test]
async fn direct_page_preload_routes_to_inactive_active_owner_without_promoting_slot() {
    let mut conn = CdpConnection::new();

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.set_active_target_id("TID-B".to_owned());
    inactive.attach_active_session("SID-B");
    conn.inactive_browser_contexts.push(inactive);

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":1,"method":"Page.addScriptToEvaluateOnNewDocument","sessionId":"SID-B","params":{"source":"globalThis.__inactivePreload = 'ready';"}}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {"identifier": "1"}, "sessionId": "SID-B"})]
    );
    assert!(
        conn.browser_context.is_none(),
        "direct Page.addScriptToEvaluateOnNewDocument should not activate the inactive owner"
    );
    assert!(
        conn.target_owner_state_for_session(Some("SID-B"))
            .expect("inactive owner state should be readable")
            .document_start_scripts
            .iter()
            .any(|(identifier, script)| identifier == "1"
                && script.source == "globalThis.__inactivePreload = 'ready';"),
        "preload script should be staged on the inactive target owner"
    );

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":2,"method":"Page.removeScriptToEvaluateOnNewDocument","sessionId":"SID-B","params":{"identifier":"1"}}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 2, "result": {}, "sessionId": "SID-B"})]
    );
    assert!(
        conn.browser_context.is_none(),
        "direct Page.removeScriptToEvaluateOnNewDocument should not activate the inactive owner"
    );
    assert!(
        conn.target_owner_state_for_session(Some("SID-B"))
            .expect("inactive owner state should be readable")
            .document_start_scripts
            .is_empty(),
        "removeScriptToEvaluateOnNewDocument should mutate the inactive target owner"
    );
}

#[tokio::test]
async fn direct_page_preload_routes_to_inactive_background_owner_without_promoting_slot() {
    let mut conn = CdpConnection::new();

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.background_targets.push(BackgroundTarget::new(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    ));
    conn.inactive_browser_contexts.push(inactive);

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":1,"method":"Page.addScriptToEvaluateOnNewDocument","sessionId":"SID-background","params":{"source":"globalThis.__backgroundPreload = 'ready';"}}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {"identifier": "1"}, "sessionId": "SID-background"})]
    );
    assert!(
        conn.browser_context.is_none(),
        "direct Page.addScriptToEvaluateOnNewDocument should not activate the inactive background owner"
    );
    assert!(
        conn.target_owner_state_for_session(Some("SID-background"))
            .expect("background owner state should be readable")
            .document_start_scripts
            .iter()
            .any(|(identifier, script)| identifier == "1"
                && script.source == "globalThis.__backgroundPreload = 'ready';"),
        "preload script should be staged on the inactive background owner"
    );

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":2,"method":"Page.removeScriptToEvaluateOnNewDocument","sessionId":"SID-background","params":{"identifier":"1"}}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 2, "result": {}, "sessionId": "SID-background"})]
    );
    assert!(
        conn.browser_context.is_none(),
        "direct Page.removeScriptToEvaluateOnNewDocument should not activate the inactive background owner"
    );
    assert!(
        conn.target_owner_state_for_session(Some("SID-background"))
            .expect("background owner state should be readable")
            .document_start_scripts
            .is_empty(),
        "removeScriptToEvaluateOnNewDocument should mutate the inactive background owner"
    );
}

#[tokio::test]
async fn direct_network_enable_routes_to_inactive_background_owner_without_promoting_slot() {
    let mut conn = CdpConnection::new();

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.background_targets.push(BackgroundTarget::new(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    ));
    conn.inactive_browser_contexts.push(inactive);

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":1,"method":"Network.enable","sessionId":"SID-background"}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {}, "sessionId": "SID-background"})]
    );
    assert!(
        conn.browser_context.is_none(),
        "direct background Network.enable should not promote the inactive owner"
    );
    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(
        inactive
            .parked_page_session_state("TID-background")
            .expect("background session state should be staged")
            .network_enabled
    );
}

#[tokio::test]
async fn direct_auxiliary_network_enable_for_background_target_does_not_enable_primary_listener() {
    let mut conn = CdpConnection::new();

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.background_targets.push(BackgroundTarget::new(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    ));
    assert!(
        inactive
            .assign_auxiliary_session_to_target("TID-background", "SID-aux-background".to_owned())
    );
    conn.inactive_browser_contexts.push(inactive);

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":1,"method":"Network.enable","sessionId":"SID-aux-background"}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {}, "sessionId": "SID-aux-background"})]
    );
    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(
        inactive
            .parked_page_session_state("TID-background")
            .is_none_or(|state| !state.network_enabled),
        "auxiliary background Network.enable must not enable the target's primary listener"
    );
}

#[tokio::test]
async fn direct_network_enable_routes_to_active_background_owner_without_promoting_target() {
    let mut conn = CdpConnection::new();

    let mut active = BrowserContext::new("BID-A".into());
    active.set_active_target_id("TID-active".to_owned());
    active.attach_active_session("SID-active");
    active.background_targets.push(BackgroundTarget::new(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    ));
    conn.browser_context = Some(active);

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":1,"method":"Network.enable","sessionId":"SID-background"}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {}, "sessionId": "SID-background"})]
    );
    let active = conn
        .browser_context
        .as_ref()
        .expect("active context remains");
    assert_eq!(
        active.active_target_id(),
        Some("TID-active"),
        "direct Network.enable should not promote the background target"
    );
    assert!(
        active.background_target("TID-background").is_some(),
        "background target should remain parked after direct session execution"
    );
    assert!(
        active
            .parked_page_session_state("TID-background")
            .expect("background session state should be staged")
            .network_enabled
    );
}

#[tokio::test]
async fn direct_network_enable_for_loaded_background_owner_starts_at_network_tail() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buf = [0_u8; 1024];
                loop {
                    let Ok(read) = stream.read(&mut buf).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    request.extend_from_slice(&buf[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request);
                if request.starts_with("GET /before.js ") {
                    let body = b"globalThis.__before_background_network_enable = true;";
                    let response = format!(
                        concat!(
                            "HTTP/1.1 200 OK\r\n",
                            "Content-Type: application/javascript\r\n",
                            "Content-Length: {}\r\n",
                            "\r\n"
                        ),
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.write_all(body).await;
                } else {
                    let body = br#"<!doctype html><script src="/before.js"></script>"#;
                    let response = format!(
                        concat!(
                            "HTTP/1.1 200 OK\r\n",
                            "Content-Type: text/html\r\n",
                            "Content-Length: {}\r\n",
                            "\r\n"
                        ),
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.write_all(body).await;
                }
            });
        }
    });

    let mut ctx = crate::testing::TestContext::new();
    let page_url = format!("http://{addr}/page");
    let background = BackgroundTarget::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        page_url.clone(),
    );

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.background_targets.push(background);
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-background"))
        .await;
    assert!(
        ctx.sent.iter().all(|message| !message["method"]
            .as_str()
            .is_some_and(|method| method.starts_with("Network."))),
        "pre-enable Network records must be retained as target state, not emitted: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_and_wait_for_response_async(json!({
        "id": 1,
        "method": "Network.enable",
        "sessionId": "SID-background"
    }))
    .await;
    let response = ctx.take_all();
    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {}, "sessionId": "SID-background"})]
    );
    let inactive = ctx
        .conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    let artifacts = inactive
        .parked_network_artifacts("TID-background")
        .expect("background network artifacts should be staged");
    assert_eq!(
        artifacts.emitted_subresource_record_count_for_session(None),
        1,
        "background Network.enable should not replay pre-enable subresource records"
    );
    assert_eq!(
        artifacts.emitted_websocket_event_count_for_session(None),
        0,
        "background Network.enable should initialize websocket cursor at the loaded target tail"
    );

    server.await.unwrap();
}

#[tokio::test]
async fn direct_background_command_does_not_emit_active_observable_output_under_background_session()
{
    let mut conn = CdpConnection::new();
    let active_page = conn
        .load_page_via_runtime_async(
            "data:text/html,<!doctype html><script>console.warn('active warning')</script>",
        )
        .await
        .expect("active page should load");

    let mut active = BrowserContext::new("BID-A".into());
    active.set_active_target_id("TID-active".to_owned());
    active.attach_active_session("SID-active");
    active
        .devtools_session_state
        .console_output_session_state
        .console_enabled = true;
    active.replace_loaded_page(Some(active_page));
    active.background_targets.push(BackgroundTarget::new(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    ));
    conn.browser_context = Some(active);

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":1,"method":"Network.enable","sessionId":"SID-background"}"#,
        )
        .await;

    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {}, "sessionId": "SID-background"})],
        "direct background commands must not drain active-target console output under the background session"
    );
}

#[tokio::test]
async fn direct_console_routes_to_inactive_active_owner_without_promoting_slot() {
    let mut ctx = crate::testing::TestContext::new();
    let page_url = "data:text/html,<!doctype html><script>console.warn('boot warning')</script>";
    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.set_active_target_id("TID-B".to_owned());
    inactive.attach_active_session("SID-B");
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(page_url, Some("SID-B"))
        .await;
    ctx.wait_for_scheduler_message("inactive console fixture load", |message| {
        message["method"] == json!("Page.loadEventFired") && message["sessionId"] == json!("SID-B")
    })
    .await;
    ctx.sent.clear();

    ctx.process_and_wait_for_response_async(json!({
        "id": 1,
        "method": "Console.enable",
        "sessionId": "SID-B"
    }))
    .await;
    let response = ctx.take_all();
    assert!(
        response.iter().any(|message| {
            message["method"] == json!("Console.messageAdded")
                && message["sessionId"] == json!("SID-B")
                && message["params"]["message"]["text"] == json!("boot warning")
        }),
        "Console.enable should replay V8 buffered console output: {response:?}"
    );
    assert!(
        response
            .iter()
            .any(|message| message == &json!({"id": 1, "result": {}, "sessionId": "SID-B"})),
        "Console.enable should still return success: {response:?}"
    );
    assert!(
        ctx.conn.browser_context.is_none(),
        "direct Console.enable should not promote the inactive owner into the active slot"
    );
    let inactive = ctx
        .conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(
        inactive
            .devtools_session_state
            .console_output_session_state
            .console_enabled
    );
    assert_eq!(
        inactive
            .active_target
            .owner_state
            .console_output_state
            .console_domain_cursor(),
        (0, 0),
        "V8 Inspector owns buffered Console API replay; the protocol observable cursor must not claim the same message"
    );

    ctx.process_and_wait_for_response_async(json!({
        "id": 2,
        "method": "Console.clearMessages",
        "sessionId": "SID-B"
    }))
    .await;
    let response = ctx.take_all();
    assert_eq!(
        response,
        vec![json!({"id": 2, "result": {}, "sessionId": "SID-B"})]
    );
    assert!(
        ctx.conn.browser_context.is_none(),
        "direct Console.clearMessages should not promote the inactive owner"
    );

    ctx.process_and_wait_for_response_async(json!({
        "id": 3,
        "method": "Console.disable",
        "sessionId": "SID-B"
    }))
    .await;
    let response = ctx.take_all();
    assert_eq!(
        response,
        vec![json!({"id": 3, "result": {}, "sessionId": "SID-B"})]
    );
    assert!(
        ctx.conn.browser_context.is_none(),
        "direct Console.disable should not promote the inactive owner"
    );
    let inactive = ctx
        .conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(
        !inactive
            .devtools_session_state
            .console_output_session_state
            .console_enabled
    );
    assert_eq!(
        inactive
            .active_target
            .owner_state
            .console_output_state
            .console_domain_cursor(),
        (0, 0),
        "clearing or disabling V8-owned Console output must not manufacture a protocol-queue cursor"
    );
}

#[tokio::test]
async fn direct_console_routes_to_inactive_background_owner_without_promoting_slot() {
    let mut conn = CdpConnection::new();

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.background_targets.push(BackgroundTarget::new(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    ));
    conn.inactive_browser_contexts.push(inactive);

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":1,"method":"Console.enable","sessionId":"SID-background"}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {}, "sessionId": "SID-background"})]
    );
    assert!(
        conn.browser_context.is_none(),
        "direct background Console.enable should not promote the inactive owner"
    );
    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(
        inactive
            .parked_page_session_state("TID-background")
            .is_some_and(|state| state
                .devtools_session_state
                .console_output_session_state
                .console_enabled),
        "background target should stage Console.enable"
    );

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":2,"method":"Console.clearMessages","sessionId":"SID-background"}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 2, "result": {}, "sessionId": "SID-background"})]
    );
    assert!(
        conn.browser_context.is_none(),
        "direct background Console.clearMessages should not promote the inactive owner"
    );
    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(
        inactive
            .parked_page_session_state("TID-background")
            .is_some_and(|state| state
                .devtools_session_state
                .console_output_session_state
                .console_enabled),
        "clearMessages should not disable the staged background Console state"
    );

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":3,"method":"Console.disable","sessionId":"SID-background"}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 3, "result": {}, "sessionId": "SID-background"})]
    );
    assert!(
        conn.browser_context.is_none(),
        "direct background Console.disable should not promote the inactive owner"
    );
    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(
        inactive
            .parked_page_session_state("TID-background")
            .is_none_or(|state| !state
                .devtools_session_state
                .console_output_session_state
                .console_enabled),
        "background target should stage or collapse Console.disable"
    );
}

#[tokio::test]
async fn direct_console_routes_to_loaded_background_owner_and_advances_parked_cursor() {
    let mut ctx = crate::testing::TestContext::new();
    let page_url =
        "data:text/html,<!doctype html><script>console.warn('background warning')</script>";
    let background = BackgroundTarget::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        page_url.to_owned(),
    );

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.background_targets.push(background);
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(page_url, Some("SID-background"))
        .await;
    ctx.wait_for_scheduler_message("background console fixture load", |message| {
        message["method"] == json!("Page.loadEventFired")
            && message["sessionId"] == json!("SID-background")
    })
    .await;
    ctx.sent.clear();

    ctx.process_and_wait_for_response_async(json!({
        "id": 1,
        "method": "Console.enable",
        "sessionId": "SID-background"
    }))
    .await;
    let response = ctx.take_all();
    assert!(
        response.iter().any(|message| {
            message["method"] == json!("Console.messageAdded")
                && message["sessionId"] == json!("SID-background")
                && message["params"]["message"]["text"] == json!("background warning")
        }),
        "background Console.enable should replay V8 buffered console output: {response:?}"
    );
    assert!(
        response.iter().any(
            |message| message == &json!({"id": 1, "result": {}, "sessionId": "SID-background"})
        ),
        "background Console.enable should still return success: {response:?}"
    );
    let inactive = ctx
        .conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert_eq!(
        inactive
            .parked_target_owner_state_or_default("TID-background")
            .console_output_state
            .console_domain_cursor(),
        (0, 0),
        "V8 Inspector owns buffered Console API replay for a loaded background target"
    );

    ctx.process_and_wait_for_response_async(json!({
        "id": 2,
        "method": "Console.clearMessages",
        "sessionId": "SID-background"
    }))
    .await;
    let response = ctx.take_all();
    assert_eq!(
        response,
        vec![json!({"id": 2, "result": {}, "sessionId": "SID-background"})]
    );
    let inactive = ctx
        .conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert_eq!(
        inactive
            .parked_target_owner_state_or_default("TID-background")
            .console_output_state
            .console_domain_cursor(),
        (0, 0),
        "clearing V8-owned Console output must not advance the separate protocol observable cursor"
    );
}

#[tokio::test]
async fn direct_log_enable_routes_to_inactive_active_owner_without_promoting_slot_or_replaying_console_api()
 {
    let mut conn = CdpConnection::new();

    let page = conn
        .load_page_via_runtime_async(
            "data:text/html,<!doctype html><script>console.warn('boot warning')</script>",
        )
        .await
        .expect("test page should load");
    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.set_target_url("data:text/html,log-direct-test".to_owned());
    inactive.set_active_target_id("TID-B".to_owned());
    inactive.attach_active_session("SID-B");
    inactive.replace_loaded_page(Some(page));
    conn.inactive_browser_contexts.push(inactive);

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":1,"method":"Log.enable","sessionId":"SID-B"}"#,
        )
        .await;
    assert_eq!(
        response.first(),
        Some(&json!({"id": 1, "result": {}, "sessionId": "SID-B"})),
        "Log.enable should return the command result before replay events"
    );
    assert!(
        !response
            .iter()
            .any(|message| message["method"] == json!("Log.entryAdded")),
        "direct Log.enable should not replay buffered console API output: {response:?}"
    );
    assert!(
        conn.browser_context.is_none(),
        "direct Log.enable should not promote the inactive owner into the active slot"
    );
    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(
        inactive
            .devtools_session_state
            .page_session_state
            .log_enabled
    );
    assert_eq!(
        inactive
            .devtools_session_state
            .console_output_session_state
            .log_lifecycle_entries,
        0,
        "console API output should not advance the inactive session's Log cursor"
    );
}

#[tokio::test]
async fn direct_log_enable_routes_to_inactive_background_owner_without_promoting_slot() {
    let mut conn = CdpConnection::new();

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.background_targets.push(BackgroundTarget::new(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    ));
    conn.inactive_browser_contexts.push(inactive);

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":1,"method":"Log.enable","sessionId":"SID-background"}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {}, "sessionId": "SID-background"})]
    );
    assert!(
        conn.browser_context.is_none(),
        "direct background Log.enable should not promote the inactive owner"
    );
    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(
        inactive
            .parked_page_session_state("TID-background")
            .is_some_and(|state| state.devtools_session_state.page_session_state.log_enabled),
        "background target should stage Log.enable"
    );
}

#[tokio::test]
async fn direct_log_enable_routes_to_loaded_background_owner_without_replaying_console_api() {
    let mut conn = CdpConnection::new();
    let page = conn
        .load_page_via_runtime_async(
            "data:text/html,<!doctype html><script>console.warn('background log')</script>",
        )
        .await
        .expect("background page should load");

    let mut background = BackgroundTarget::new(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    );
    background.set_target_url(page.final_url().as_str().to_owned());
    background.replace_loaded_page(Some(page));

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.background_targets.push(background);
    conn.inactive_browser_contexts.push(inactive);

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":1,"method":"Log.enable","sessionId":"SID-background"}"#,
        )
        .await;
    assert_eq!(
        response.first(),
        Some(&json!({"id": 1, "result": {}, "sessionId": "SID-background"})),
        "Log.enable should return the command result before replay events"
    );
    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {}, "sessionId": "SID-background"})],
        "background Log.enable should not replay loaded parked target console API output"
    );

    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert_eq!(
        inactive
            .parked_page_session_state("TID-background")
            .expect("parked session state")
            .devtools_session_state
            .console_output_session_state
            .log_lifecycle_entries,
        0,
        "console API output should not advance the parked session's Log cursor"
    );

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":2,"method":"Log.enable","sessionId":"SID-background"}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 2, "result": {}, "sessionId": "SID-background"})],
        "a second background Log.enable should still not replay console API output"
    );
}

#[tokio::test]
async fn direct_log_disable_routes_to_inactive_active_owner_without_promoting_slot() {
    let mut ctx = crate::testing::TestContext::new();
    let page_url = "data:text/html,<!doctype html><script>console.warn('boot warning')</script>";
    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.set_active_target_id("TID-B".to_owned());
    inactive.attach_active_session("SID-B");
    inactive
        .devtools_session_state
        .page_session_state
        .log_enabled = true;
    ctx.conn.inactive_browser_contexts.push(inactive);
    ctx.install_navigation_fixture_for_session_owner(page_url, Some("SID-B"))
        .await;
    ctx.sent.clear();

    ctx.process_and_wait_for_response_async(json!({
        "id": 1,
        "method": "Log.disable",
        "sessionId": "SID-B"
    }))
    .await;
    let response = ctx.take_all();
    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {}, "sessionId": "SID-B"})]
    );
    assert!(
        ctx.conn.browser_context.is_none(),
        "direct Log.disable should not promote the inactive owner into the active slot"
    );
    let inactive = ctx
        .conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(
        !inactive
            .devtools_session_state
            .page_session_state
            .log_enabled
    );
    assert_eq!(
        inactive
            .devtools_session_state
            .console_output_session_state
            .log_lifecycle_entries,
        0,
        "Log.disable should preserve storage for replay on the next enable"
    );
}

#[tokio::test]
async fn direct_log_disable_routes_to_inactive_background_owner_without_promoting_slot() {
    let mut conn = CdpConnection::new();

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.background_targets.push(BackgroundTarget::new(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    ));
    inactive.mutate_parked_page_session_state("TID-background", |state| {
        state.devtools_session_state.page_session_state.log_enabled = true;
    });
    conn.inactive_browser_contexts.push(inactive);

    let response = conn
        .process_message_messages_only_for_test(
            r#"{"id":1,"method":"Log.disable","sessionId":"SID-background"}"#,
        )
        .await;
    assert_eq!(
        response,
        vec![json!({"id": 1, "result": {}, "sessionId": "SID-background"})]
    );
    assert!(
        conn.browser_context.is_none(),
        "direct background Log.disable should not promote the inactive owner"
    );
    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(
        inactive
            .parked_page_session_state("TID-background")
            .is_none_or(|state| !state.devtools_session_state.page_session_state.log_enabled),
        "background target should stage or collapse Log.disable"
    );
}

#[tokio::test]
async fn direct_network_policy_routes_to_inactive_active_owner_without_promoting_slot() {
    let mut conn = CdpConnection::new();

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.set_active_target_id("TID-B".to_owned());
    inactive.attach_active_session("SID-B");
    conn.inactive_browser_contexts.push(inactive);

    for raw in [
        r#"{"id":1,"method":"Network.setCacheDisabled","sessionId":"SID-B","params":{"cacheDisabled":true}}"#,
        r#"{"id":2,"method":"Network.setBypassServiceWorker","sessionId":"SID-B","params":{"bypass":true}}"#,
        r#"{"id":3,"method":"Network.setBlockedURLs","sessionId":"SID-B","params":{"urls":["*://blocked.test/*"]}}"#,
        r#"{"id":4,"method":"Network.setExtraHTTPHeaders","sessionId":"SID-B","params":{"headers":{"X-Test":"direct"}}}"#,
        r#"{"id":5,"method":"Network.setUserAgentOverride","sessionId":"SID-B","params":{"userAgent":"Moli/Direct-UA"}}"#,
        r#"{"id":6,"method":"Network.emulateNetworkConditions","sessionId":"SID-B","params":{"offline":true,"latency":25,"downloadThroughput":1024,"uploadThroughput":256,"connectionType":"cellular3g"}}"#,
    ] {
        let response = conn.process_message_messages_only_for_test(raw).await;
        let request_id = serde_json::from_str::<serde_json::Value>(raw)
            .expect("test request")
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .expect("request id");
        assert_eq!(
            response,
            vec![json!({"id": request_id, "result": {}, "sessionId": "SID-B"})]
        );
        assert!(
            conn.browser_context.is_none(),
            "direct Network policy commands should not promote the inactive owner"
        );
    }

    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(inactive.network_policy.cache_disabled());
    assert!(inactive.network_policy.bypass_service_worker());
    assert_eq!(
        inactive.network_policy.blocked_url_patterns(),
        vec!["*://blocked.test/*".to_owned()]
    );
    assert_eq!(
        inactive.network_policy.extra_headers(),
        vec![("X-Test".to_owned(), "direct".to_owned())]
    );
    assert_eq!(
        inactive.network_policy.user_agent_override(),
        Some("Moli/Direct-UA")
    );
    assert!(inactive.network_policy.network_offline());
    assert_eq!(inactive.network_policy.emulated_network_latency(), 25.0);
    assert_eq!(
        inactive.network_policy.emulated_download_throughput(),
        1024.0
    );
    assert_eq!(inactive.network_policy.emulated_upload_throughput(), 256.0);
    assert_eq!(
        inactive.network_policy.emulated_connection_type(),
        Some("cellular3g")
    );
}

#[tokio::test]
async fn direct_network_policy_invalid_params_return_owner_plan_error_without_promoting_slot() {
    let mut conn = CdpConnection::new();

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.set_active_target_id("TID-B".to_owned());
    inactive.attach_active_session("SID-B");
    conn.inactive_browser_contexts.push(inactive);

    for raw in [
        r#"{"id":1,"method":"Network.setCacheDisabled","sessionId":"SID-B","params":{}}"#,
        r#"{"id":2,"method":"Network.setExtraHTTPHeaders","sessionId":"SID-B","params":{"headers":[]}}"#,
    ] {
        let response = conn.process_message_messages_only_for_test(raw).await;
        let request_id = serde_json::from_str::<serde_json::Value>(raw)
            .expect("test request")
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .expect("request id");
        assert_eq!(
            response,
            vec![json!({
                "id": request_id,
                "error": {"code": -32602, "message": "InvalidParams"},
                "sessionId": "SID-B"
            })]
        );
        assert!(
            conn.browser_context.is_none(),
            "invalid direct Network policy commands should not promote the inactive owner"
        );
    }

    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    assert!(!inactive.network_policy.cache_disabled());
    assert!(
        inactive.network_policy.extra_headers().is_empty(),
        "invalid direct output-plan commands must not mutate owner policy"
    );
}

#[tokio::test]
async fn direct_network_policy_routes_to_inactive_background_owner_without_promoting_slot() {
    let mut conn = CdpConnection::new();

    let mut inactive = BrowserContext::new("BID-B".into());
    inactive.background_targets.push(BackgroundTarget::new(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    ));
    conn.inactive_browser_contexts.push(inactive);

    for raw in [
        r#"{"id":1,"method":"Network.setCacheDisabled","sessionId":"SID-background","params":{"cacheDisabled":true}}"#,
        r#"{"id":2,"method":"Network.setBypassServiceWorker","sessionId":"SID-background","params":{"bypass":true}}"#,
        r#"{"id":3,"method":"Network.setBlockedURLs","sessionId":"SID-background","params":{"urls":["*://blocked-background.test/*"]}}"#,
        r#"{"id":4,"method":"Network.setExtraHTTPHeaders","sessionId":"SID-background","params":{"headers":{"X-Background":"direct"}}}"#,
        r#"{"id":5,"method":"Network.setUserAgentOverride","sessionId":"SID-background","params":{"userAgent":"Moli/Background-UA"}}"#,
        r#"{"id":6,"method":"Network.emulateNetworkConditions","sessionId":"SID-background","params":{"offline":true,"latency":50,"downloadThroughput":2048,"uploadThroughput":512,"connectionType":"wifi"}}"#,
    ] {
        let response = conn.process_message_messages_only_for_test(raw).await;
        let request_id = serde_json::from_str::<serde_json::Value>(raw)
            .expect("test request")
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .expect("request id");
        assert_eq!(
            response,
            vec![json!({"id": request_id, "result": {}, "sessionId": "SID-background"})]
        );
        assert!(
            conn.browser_context.is_none(),
            "direct background Network policy commands should not promote the inactive owner"
        );
    }

    let inactive = conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-B")
        .expect("inactive owner must remain parked");
    let staged = inactive
        .parked_page_session_state("TID-background")
        .expect("background session state should be staged");
    assert!(staged.network_policy.cache_disabled());
    assert!(staged.network_policy.bypass_service_worker());
    assert_eq!(
        staged.network_policy.blocked_url_patterns(),
        vec!["*://blocked-background.test/*".to_owned()]
    );
    assert_eq!(
        staged.network_policy.extra_headers(),
        vec![("X-Background".to_owned(), "direct".to_owned())]
    );
    assert_eq!(
        staged.network_policy.user_agent_override(),
        Some("Moli/Background-UA")
    );
    assert!(staged.network_policy.network_offline());
    assert_eq!(staged.network_policy.emulated_network_latency(), 50.0);
    assert_eq!(staged.network_policy.emulated_download_throughput(), 2048.0);
    assert_eq!(staged.network_policy.emulated_upload_throughput(), 512.0);
    assert_eq!(
        staged.network_policy.emulated_connection_type(),
        Some("wifi")
    );
}

#[tokio::test]
async fn streaming_navigation_collect_transition_preserves_redirect_cookie_and_body_metadata() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut request = Vec::new();
            let mut buf = [0_u8; 1024];
            loop {
                let Ok(read) = stream.read(&mut buf).await else {
                    return;
                };
                if read == 0 {
                    return;
                }
                request.extend_from_slice(&buf[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&request);
            if request.starts_with("GET /start ") {
                let response = concat!(
                    "HTTP/1.1 302 Found\r\n",
                    "Location: /final\r\n",
                    "Set-Cookie: hop=redirect; Path=/\r\n",
                    "Content-Length: 0\r\n",
                    "\r\n"
                );
                let _ = stream.write_all(response.as_bytes()).await;
            } else {
                let body = b"<!doctype html><main id=\"from-stream\">streamed</main>";
                let response = format!(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/html\r\n",
                        "Set-Cookie: final=yes; Path=/\r\n",
                        "Content-Length: {}\r\n",
                        "\r\n"
                    ),
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.write_all(body).await;
            }
        }
    });

    let start_url = format!("http://{addr}/start");
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    let outcome = conn
        .load_navigation_request_via_runtime_async("GET", &start_url, None, Vec::new())
        .await
        .expect("streaming navigation should load");
    let mut navigation = commit_navigation_outcome_for_test(&mut conn, outcome).await;

    assert_eq!(
        navigation.final_url.as_str(),
        format!("http://{addr}/final")
    );
    assert_eq!(navigation.response_status, 200);
    assert!(navigation.response_body().contains("from-stream"));
    let network_events = navigation.completed_body_network_events();
    assert_eq!(network_events.redirect_chain.len(), 1);
    assert_eq!(network_events.redirect_chain[0].status, 302);
    assert_eq!(
        network_events.redirect_chain[0].to_url.as_str(),
        format!("http://{addr}/final")
    );
    assert!(
        network_events
            .response_cookie_reports
            .iter()
            .any(|report| report.is_accepted())
    );
    let cookie_names = conn
        .browser_context
        .as_ref()
        .unwrap()
        .snapshot_cookies()
        .into_iter()
        .map(|cookie| cookie.name)
        .collect::<Vec<_>>();
    assert!(cookie_names.iter().any(|name| name == "final"));
    assert_eq!(
        navigation
            .page
            .evaluate_runtime_expression_async("document.getElementById('from-stream').textContent")
            .await
            .expect("loaded page should be evaluable")["value"],
        json!("streamed")
    );

    server.await.unwrap();
}

#[tokio::test]
async fn data_image_navigation_loads_from_synthetic_response_without_curl() {
    let data_url = "data:image/png;base64,AP9h";
    let request_headers = vec![("accept".to_owned(), "image/png".to_owned())];
    let mut conn = CdpConnection::new();

    let outcome = conn
        .load_navigation_request_via_runtime_async("GET", data_url, None, request_headers.clone())
        .await
        .expect("data:image navigation should load without a network fetch");
    let navigation = commit_navigation_outcome_for_test(&mut conn, outcome).await;

    assert_eq!(navigation.requested_url.as_str(), data_url);
    assert_eq!(navigation.final_url.as_str(), data_url);
    assert_eq!(navigation.request_method, "GET");
    assert_eq!(navigation.request_headers, request_headers);
    assert_eq!(navigation.response_status, 200);
    assert_eq!(
        navigation.response_headers,
        vec![("Content-Type".to_owned(), "image/png".to_owned())]
    );
    assert!(navigation.pending_download.is_none());

    let network_events = navigation.completed_body_network_events();
    assert_eq!(network_events.request_method, "GET");
    assert_eq!(network_events.request_headers, request_headers);
    assert!(network_events.final_request_cookie_report.is_none());
    assert_eq!(network_events.response_status, 200);
    assert_eq!(
        network_events.response_headers,
        vec![("Content-Type".to_owned(), "image/png".to_owned())]
    );
    assert!(network_events.response_cookie_reports.is_empty());
    assert!(network_events.redirect_chain.is_empty());
}

#[tokio::test]
async fn streaming_navigation_feeds_parser_before_body_eof() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let script_requested = Arc::new(AtomicBool::new(false));
    let release_tail = Arc::new(Notify::new());
    let server_script_requested = Arc::clone(&script_requested);
    let server_release_tail = Arc::clone(&release_tail);
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let script_requested = Arc::clone(&server_script_requested);
            let release_tail = Arc::clone(&server_release_tail);
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buf = [0_u8; 1024];
                loop {
                    let Ok(read) = stream.read(&mut buf).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    request.extend_from_slice(&buf[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request);
                if request.starts_with("GET /gate.js ") {
                    script_requested.store(true, Ordering::SeqCst);
                    release_tail.notify_waiters();
                    let body = b"document.documentElement.setAttribute('data-script','seen');";
                    let response = format!(
                        concat!(
                            "HTTP/1.1 200 OK\r\n",
                            "Content-Type: application/javascript\r\n",
                            "Content-Length: {}\r\n",
                            "\r\n"
                        ),
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.write_all(body).await;
                    return;
                }

                let response = concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: text/html; charset=utf-8\r\n",
                    "Transfer-Encoding: chunked\r\n",
                    "\r\n"
                );
                let first = b"<!doctype html><script src=\"/gate.js\"></script><main id=\"tail\">";
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream
                    .write_all(format!("{:x}\r\n", first.len()).as_bytes())
                    .await;
                let _ = stream.write_all(first).await;
                let _ = stream.write_all(b"\r\n").await;
                if tokio::time::timeout(std::time::Duration::from_secs(2), release_tail.notified())
                    .await
                    .is_err()
                {
                    return;
                }
                let tail = b"done</main>";
                let _ = stream
                    .write_all(format!("{:x}\r\n", tail.len()).as_bytes())
                    .await;
                let _ = stream.write_all(tail).await;
                let _ = stream.write_all(b"\r\n0\r\n\r\n").await;
            });
        }
    });

    let page_url = format!("http://{addr}/page");
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    let mut navigation = tokio::time::timeout(std::time::Duration::from_secs(4), async {
        let outcome = conn
            .load_navigation_request_via_runtime_async("GET", &page_url, None, Vec::new())
            .await
            .expect("streaming navigation should prepare");
        commit_navigation_outcome_for_test(&mut conn, outcome).await
    })
    .await
    .expect("streaming navigation should not wait for EOF before parser resource fetch");

    assert!(
        script_requested.load(Ordering::SeqCst),
        "parser should request the external script before the main body EOF"
    );
    assert!(navigation.response_body().contains("id=\"tail\""));
    assert_eq!(
        navigation
            .page
            .evaluate_runtime_expression_async(
                "document.documentElement.getAttribute('data-script')"
            )
            .await
            .expect("loaded page should be evaluable")["value"],
        json!("seen")
    );
    assert_eq!(
        navigation
            .page
            .evaluate_runtime_expression_async("document.getElementById('tail').textContent")
            .await
            .expect("loaded page should be evaluable")["value"],
        json!("done")
    );
    server.await.expect("server should finish");
}
