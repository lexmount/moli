use super::cookie_manager_surface;
use super::state::TargetPageAbsenceReason;
use super::{
    BackgroundProtocolEvent, BrowserContext, BrowserContextCookieBackendConnectionState,
    BrowserContextCookieGetFreshnessStatus, BrowserContextCookieSetReadinessStatus,
    BrowserContextDefaultCookieWriteUrlSource, BrowserContextDocumentCookieCacheLookupResult,
    BrowserContextFirstCookieRequest, BrowserContextReservedSiteDataOwnerState,
    BrowserContextSiteDataManagerOwnerState, BrowserContextStructuredCookieCommandVerdict,
    BrowserContextStructuredCookieWriteBackendStatus,
    BrowserContextStructuredCookieWriteReadinessStatus, CdpConnection, CdpSessionRoute,
    CommandDispatchContext, CommandResponseFlushContext, NavigationBackgroundEvent,
    NavigationDispatchState, NavigationResultProjection, ServiceWorkerTargetState,
    SharedWorkerTargetState, build_event,
};
use crate::devtools_runtime::{
    AutomationEvent, DevToolsFrameId, DevToolsLoaderId, DevToolsTargetFilterEntry,
    DevToolsTargetId, NavigationFrameEvent, NavigationFrameEventKind,
};
use crate::domains::network::{
    FailedNavigationDocumentPolicy, FailedNavigationResponseMode,
    MaterializedFailedDocumentProgress, MaterializedNavigationLoadOutcome,
    empty_main_document_progress_gate_for_test,
};
use crate::domains::page::MaterializedNavigationCompletion;
use crate::testing::TestContext;
use moli_cookie_jar::{
    BrowserCookieFacadeContextOverrides, BrowserCookieFacadeOverrides, CookieSiteDataClearScope,
    CookieSiteDataOperation, CookieSiteDataOperationPreviewReport, CookieSiteDataScope,
    CookieSiteDataSummary, CookieStorageClearTarget, StoredCookie, StoredCookieSameSite,
    StoredCookieSetRejectionReason, StoredCookieSourceScheme,
};
use moli_core::page::RendererServiceWorkerVersionStatus;
use moli_core::{
    LayoutPolicy, OptionalResourceFetchMask,
    runtime::{NavigationEngine, NavigationRuntimeConfig},
};
use moli_fetch::FetchConfig;
use moli_shared_worker::SharedWorkerInstanceId;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use url::Url;

mod cookie_surfaces;
mod message;
mod resource_runtime;
mod site_data;

#[test]
fn published_default_target_defers_navigation_runtime_until_materialization() {
    let mut conn = CdpConnection::new_with_deferred_navigation_runtime(
        crate::CdpInitialStoragePartition::memory(),
        NavigationRuntimeConfig::default(),
    );

    assert!(!conn.standalone_navigation_engine.is_materialized());
    let (renderer_publication_sender, _renderer_publication_receiver) =
        moli_core::renderer_output_transport_channel();
    conn.set_renderer_publication_sender(renderer_publication_sender);
    conn.publish_default_browser_target();

    assert!(!conn.standalone_navigation_engine.is_materialized());
    let page = conn
        .devtools_target_info(conn.default_target_id())
        .expect("published default page target");
    let tab = conn
        .devtools_target_info(conn.default_tab_target_id())
        .expect("published default tab target");
    assert_eq!(page.url, "about:blank");
    assert_eq!(tab.url, "about:blank");

    conn.install_default_browser_target();

    assert!(
        !conn.standalone_navigation_engine.is_materialized(),
        "materializing the default Page must not allocate the fallback engine"
    );
    assert!(
        conn.browser_context
            .as_ref()
            .and_then(|context| context.page_navigation_engine(conn.default_target_id()))
            .is_some(),
        "materializing the default target must install its Page-owned engine"
    );
    assert!(
        conn.devtools_target_info(conn.default_target_id())
            .is_some()
    );
    assert_eq!(
        conn.browser_context
            .as_ref()
            .and_then(BrowserContext::active_target_id),
        Some(conn.default_target_id())
    );
}

fn deferred_default_connection() -> CdpConnection {
    let mut conn = CdpConnection::new_with_deferred_navigation_runtime(
        crate::CdpInitialStoragePartition::memory(),
        NavigationRuntimeConfig::default(),
    );
    conn.publish_default_browser_target();
    conn
}

#[tokio::test]
async fn activating_the_only_default_placeholder_does_not_start_the_runtime() {
    let mut conn = deferred_default_connection();
    let target_id = conn.default_target_id();
    let raw = json!({
        "id": 1,
        "method": "Target.activateTarget",
        "params": { "targetId": target_id },
    })
    .to_string();

    let messages = conn.process_message_messages_only_for_test(&raw).await;

    assert_eq!(messages, vec![json!({ "id": 1, "result": {} })]);
    assert!(!conn.standalone_navigation_engine.is_materialized());
    assert!(conn.devtools_target_info(target_id).is_some());
}

#[tokio::test]
async fn closing_the_default_placeholder_does_not_start_the_runtime() {
    let mut conn = deferred_default_connection();
    let raw = json!({
        "id": 2,
        "method": "Target.closeTarget",
        "params": { "targetId": conn.default_tab_target_id() },
    })
    .to_string();

    let messages = conn.process_message_messages_only_for_test(&raw).await;

    assert_eq!(
        messages.first(),
        Some(&json!({ "id": 2, "result": { "success": true } }))
    );
    assert!(!conn.standalone_navigation_engine.is_materialized());
    assert!(
        conn.devtools_target_info(conn.default_target_id())
            .is_none()
    );
    assert!(
        conn.target_registry_host_kind(conn.default_target_id())
            .is_none()
    );
}

#[tokio::test]
async fn creating_a_target_preserves_the_published_default_as_a_placeholder() {
    let mut conn = deferred_default_connection();
    let raw = json!({
        "id": 3,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" },
    })
    .to_string();

    let messages = conn.process_message_messages_only_for_test(&raw).await;
    let created_target_id = messages
        .iter()
        .find(|message| message["id"] == json!(3))
        .and_then(|message| message["result"]["targetId"].as_str())
        .expect("createTarget response");

    assert_ne!(created_target_id, conn.default_target_id());
    assert!(
        conn.standalone_navigation_engine.is_materialized(),
        "creating a real Page target is the first operation that needs the renderer runtime"
    );
    assert!(
        conn.browser_context
            .as_ref()
            .and_then(|context| context.page_navigation_engine(created_target_id))
            .is_some(),
        "the created Page must own the materialized navigation engine"
    );
    assert!(conn.default_target_lifecycle.is_placeholder());
    assert!(
        conn.devtools_target_info(conn.default_target_id())
            .is_some()
    );
    assert_eq!(
        conn.browser_context
            .as_ref()
            .map(|context| context.id.as_str()),
        Some(conn.default_browser_context_id())
    );

    let attach_raw = json!({
        "id": 4,
        "method": "Target.attachToTarget",
        "params": { "targetId": conn.default_target_id(), "flatten": true },
    })
    .to_string();
    let attach_messages = conn
        .process_message_messages_only_for_test(&attach_raw)
        .await;

    assert!(attach_messages.iter().any(|message| {
        message["id"] == json!(4) && message["result"]["sessionId"].is_string()
    }));
    assert!(conn.default_target_lifecycle.is_live());
    let browser_context = conn.browser_context.as_ref().expect("default context");
    assert_eq!(
        browser_context.active_target_id(),
        Some(created_target_id),
        "attaching the placeholder must not activate it"
    );
    assert!(
        browser_context
            .background_target(conn.default_target_id())
            .is_some(),
        "first live use should materialize the default behind the catalog entry"
    );
}

#[test]
fn idle_navigation_engine_reset_preserves_mock_layout_policy() {
    let mut conn = CdpConnection::new_with_initial_storage_partition_and_runtime_config(
        crate::CdpInitialStoragePartition::memory(),
        NavigationRuntimeConfig::new(
            FetchConfig::default(),
            OptionalResourceFetchMask::NONE,
            true,
            LayoutPolicy::Mock,
        ),
    );

    assert_eq!(
        conn.standalone_navigation_engine.layout_policy(),
        LayoutPolicy::Mock
    );
    let reset = conn.release_idle_navigation_engine_memory_if_idle();

    assert!(reset.reset);
    assert_eq!(
        conn.standalone_navigation_engine.layout_policy(),
        LayoutPolicy::Mock
    );
}

#[tokio::test]
async fn browser_context_install_and_removal_preserve_mock_layout_policy() {
    let mut conn = CdpConnection::new_with_initial_storage_partition_and_runtime_config(
        crate::CdpInitialStoragePartition::memory(),
        NavigationRuntimeConfig::new(
            FetchConfig::default(),
            OptionalResourceFetchMask::NONE,
            true,
            LayoutPolicy::Mock,
        ),
    );
    conn.insert_browser_context(BrowserContext::new("CTX-layout".to_owned()));

    assert_eq!(
        conn.standalone_navigation_engine.layout_policy(),
        LayoutPolicy::Mock
    );
    let removed = conn
        .remove_browser_context_by_id_restoring_active_async("CTX-layout", None)
        .await;

    assert!(removed.is_some());
    assert_eq!(
        conn.standalone_navigation_engine.layout_policy(),
        LayoutPolicy::Mock
    );
}

#[test]
fn global_io_stream_ids_cross_u32_max_without_reuse() {
    let mut conn = CdpConnection::new();

    conn.next_global_io_stream_id = u32::MAX as u64;
    let handle = conn.open_global_io_stream(b"payload".to_vec());

    assert_eq!(handle, "BROWSER-STREAM-4294967296");
    assert!(conn.global_io_streams.contains_key(&handle));
}

#[test]
#[should_panic(expected = "global IO stream id space exhausted")]
fn global_io_stream_id_allocator_rejects_u64_exhaustion() {
    let mut conn = CdpConnection::new();
    conn.next_global_io_stream_id = u64::MAX;

    let _ = conn.open_global_io_stream(Vec::new());
}

#[test]
#[should_panic(expected = "internal Runtime command id space exhausted")]
fn internal_runtime_command_id_allocator_rejects_u64_exhaustion() {
    let mut conn = CdpConnection::new();
    conn.next_internal_runtime_command_id = u64::MAX;

    let _ = conn.next_internal_runtime_command_id();
}

#[test]
fn replace_root_target_discovery_is_noop_when_already_enabled() {
    let mut conn = CdpConnection::new();
    let filter = vec![DevToolsTargetFilterEntry {
        exclude: false,
        target_type: Some("service_worker".to_owned()),
    }];
    conn.set_target_discovery_for_owner_from_devtools_filter(None, Some(filter.clone()));

    let previous = conn.replace_root_target_discovery_enabled(true);

    assert!(previous);
    assert_eq!(conn.target_discovery_filter_for_owner(None), Some(filter));
}

#[test]
fn command_response_flush_permit_is_unique_and_scoped_to_its_context() {
    let mut conn = CdpConnection::new();

    let (first_permit, first_context) = conn.begin_command_response_flush_permit();
    let first_receiver = first_context
        .receiver()
        .expect("first command should install a response flush observer");

    let (second_permit, second_context) = conn.begin_command_response_flush_permit();
    let second_receiver = second_context
        .receiver()
        .expect("second command should install a response flush observer");

    second_permit.finish();

    assert!(
        !*first_receiver.borrow(),
        "finishing a later command permit must not release observers of an earlier command"
    );
    assert!(
        *second_receiver.borrow(),
        "finishing a command permit should release observers of that same command"
    );

    first_permit.finish();
    assert!(
        *first_receiver.borrow(),
        "the earlier command should remain releasable by its unique permit"
    );
}

#[test]
fn dropping_command_response_flush_permit_cancels_its_observers() {
    let mut conn = CdpConnection::new();
    let (permit, context) = conn.begin_command_response_flush_permit();
    let receiver = context
        .receiver()
        .expect("command should install a response flush observer");

    drop(permit);

    assert!(
        receiver.has_changed().is_err(),
        "dropping the unique permit must close the command-scoped observation"
    );
    assert!(
        !*receiver.borrow(),
        "canceling a command must not falsely publish that its response was flushed"
    );
}

#[test]
fn command_response_flush_permit_runs_deferred_release_exactly_once() {
    let mut conn = CdpConnection::new();
    let (permit, context) = conn.begin_command_response_flush_permit();
    let releases = Arc::new(AtomicUsize::new(0));
    let release_counter = releases.clone();
    context.defer_until_response_flush(move || {
        release_counter.fetch_add(1, Ordering::SeqCst);
    });

    assert_eq!(releases.load(Ordering::SeqCst), 0);
    permit.finish();
    assert_eq!(
        releases.load(Ordering::SeqCst),
        1,
        "the unique permit must release one continuation exactly once"
    );
}

#[test]
fn abandoned_command_response_flush_permit_releases_fail_open() {
    let mut conn = CdpConnection::new();
    let (permit, context) = conn.begin_command_response_flush_permit();
    let releases = Arc::new(AtomicUsize::new(0));
    let release_counter = releases.clone();
    context.defer_until_response_flush(move || {
        release_counter.fetch_add(1, Ordering::SeqCst);
    });

    drop(context);
    assert_eq!(releases.load(Ordering::SeqCst), 0);
    drop(permit);
    assert_eq!(
        releases.load(Ordering::SeqCst),
        1,
        "dropping the unique permit must not leave renderer work parked"
    );
}

#[test]
fn missing_command_response_flush_context_releases_immediately() {
    let context = CommandResponseFlushContext::default();
    let releases = Arc::new(AtomicUsize::new(0));
    let release_counter = releases.clone();
    context.defer_until_response_flush(move || {
        release_counter.fetch_add(1, Ordering::SeqCst);
    });

    assert_eq!(releases.load(Ordering::SeqCst), 1);
}

#[test]
fn background_navigation_completion_sender_routes_explicit_session_owners() {
    let mut conn = CdpConnection::new();
    let mut active = BrowserContext::new("BID-active".to_owned());
    active.set_active_target_id("TID-active");
    active.attach_active_session("SID-active");
    conn.browser_context = Some(active);

    let mut inactive = BrowserContext::new("BID-inactive".to_owned());
    inactive.set_active_target_id("TID-inactive");
    inactive.attach_active_session("SID-inactive");
    conn.inactive_browser_contexts.push(inactive);

    let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
    conn.set_background_navigation_completion_sender(sender);

    assert!(
        conn.background_navigation_completion_sender_for_session_owner(Some("SID-active"))
            .is_some(),
        "a command scoped to a concrete target owner can continue navigation work in the background"
    );
    assert!(
        conn.background_navigation_completion_sender_for_session_owner(Some("SID-inactive"))
            .is_some(),
        "inactive-context target owners should also be routable by explicit session id"
    );
}

#[test]
fn navigation_gate_resolves_websocket_events_to_their_session_target() {
    let mut conn = CdpConnection::new();
    let mut target_a = BrowserContext::new("BID-A".to_owned());
    target_a.set_active_target_id("TID-A");
    target_a.attach_active_session("SID-A");
    let navigation_a = target_a
        .start_document_navigation_for_active_target("LOADER-A".to_owned())
        .expect("target A should accept a navigation request");
    conn.browser_context = Some(target_a);
    assert!(conn.arm_background_navigation_completion(&navigation_a, None));

    let mut target_b = BrowserContext::new("BID-B".to_owned());
    target_b.set_active_target_id("TID-B");
    target_b.attach_active_session("SID-B");
    conn.inactive_browser_contexts.push(target_b);

    let target_b_websocket = BackgroundProtocolEvent::immediate(json!({
        "method": "Network.webSocketClosed",
        "sessionId": "SID-B",
        "params": {
            "requestId": "REQ-B",
            "timestamp": 1.0
        }
    }));

    assert!(target_b_websocket.should_wait_for_background_navigation_completion());
    assert!(conn.has_inflight_background_navigation());
    assert_eq!(
        conn.background_navigation_target_id_for_event(&target_b_websocket)
            .as_deref(),
        Some("TID-B")
    );
    assert!(
        !conn.has_inflight_background_navigation_for_target("TID-B"),
        "target A's navigation must not gate target B's WebSocket events"
    );
}

#[test]
fn none_session_owner_route_override_scope_restores_previous_route_on_drop() {
    let mut conn = CdpConnection::new();
    let previous_route = CdpSessionRoute::PageTarget {
        browser_context_id: "BID-active".to_owned(),
        target_id: "TID-active".to_owned(),
        is_attached_session: false,
    };
    let scoped_route = CdpSessionRoute::PageTarget {
        browser_context_id: "BID-background".to_owned(),
        target_id: "TID-background".to_owned(),
        is_attached_session: false,
    };

    conn.replace_none_session_owner_route_override(Some(previous_route.clone()));
    {
        let mut scope = conn.scoped_none_session_owner_route_override(scoped_route.clone());
        assert_eq!(
            scope.conn_mut().none_session_owner_route_override(),
            Some(scoped_route)
        );
    }

    assert_eq!(
        conn.none_session_owner_route_override(),
        Some(previous_route)
    );
}

#[test]
fn navigation_background_event_queue_drains_current_token() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("CTX-nav".to_owned());
    browser_context.set_active_target_id("TID-nav");
    let token = browser_context
        .start_document_navigation_for_active_target("LOADER-1".to_owned())
        .expect("active target should produce navigation token");
    conn.browser_context = Some(browser_context);
    let message = build_event(
        "Page.frameStartedLoading",
        json!({ "frameId": "TID-nav" }),
        None,
    );

    conn.enqueue_navigation_background_event(NavigationBackgroundEvent::protocol_message(
        token,
        message.clone(),
    ));

    assert_eq!(conn.drain_navigation_background_events(), vec![message]);
    assert!(conn.drain_navigation_background_events().is_empty());
}

#[test]
fn active_browser_context_installs_its_renderer_runtime_on_engine() {
    let mut conn = CdpConnection::new();
    let mut browser_context = conn.new_browser_context("CTX-runtime".to_owned());
    browser_context.set_active_target_id("TID-runtime");
    let renderer_runtime = browser_context.renderer_runtime();

    conn.insert_browser_context(browser_context);

    assert!(
        conn.active_navigation_engine()
            .browser_context_runtime()
            .shares_state_with(&renderer_runtime)
    );
}

#[test]
fn activating_inactive_browser_context_switches_engine_renderer_runtime() {
    let mut conn = CdpConnection::new();
    let mut first = conn.new_browser_context("CTX-first".to_owned());
    first.set_active_target_id("TID-first");
    conn.insert_browser_context(first);
    let mut second = conn.new_browser_context("CTX-second".to_owned());
    second.set_active_target_id("TID-second");
    let second_renderer_runtime = second.renderer_runtime();
    conn.insert_browser_context(second);

    assert!(conn.activate_browser_context_by_id("CTX-second"));

    assert!(
        conn.active_navigation_engine()
            .browser_context_runtime()
            .shares_state_with(&second_renderer_runtime)
    );
}

#[test]
fn activating_another_browser_context_retains_pending_initial_document_engine() {
    let mut conn = CdpConnection::new();
    let mut first = conn.new_browser_context("CTX-pending-first".to_owned());
    first.set_active_target_id("TID-pending-first");
    first.begin_active_target_initial_empty_document("about:blank".to_owned());
    conn.insert_browser_context(first);
    let pending_renderer_owner = conn
        .active_navigation_engine()
        .renderer_owner_id_for_diagnostics();

    let mut second = conn.new_browser_context("CTX-pending-second".to_owned());
    second.set_active_target_id("TID-pending-second");
    conn.insert_browser_context(second);

    assert!(conn.activate_browser_context_by_id("CTX-pending-second"));

    let retained = conn
        .browser_context_by_id("CTX-pending-first")
        .and_then(|context| context.page_navigation_engine("TID-pending-first"))
        .expect("a pending initial document must keep its renderer engine alive");
    assert_eq!(
        retained.renderer_owner_id_for_diagnostics(),
        pending_renderer_owner,
        "context switching must retain the exact engine that owns the pending renderer page"
    );
}

#[test]
fn activating_another_target_retains_pending_initial_document_engine() {
    let mut conn = CdpConnection::new();
    let mut browser_context = conn.new_browser_context("CTX-pending-target".to_owned());
    browser_context.set_active_target_id("TID-pending-active");
    browser_context.begin_active_target_initial_empty_document("about:blank".to_owned());
    conn.insert_browser_context(browser_context);
    let pending_renderer_owner = conn
        .active_navigation_engine()
        .renderer_owner_id_for_diagnostics();
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .stage_background_target(
            "TID-promoted".to_owned(),
            None,
            "about:blank".to_owned(),
            Some("about:blank".to_owned()),
            None,
        );
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .set_active_target_id("TID-promoted");

    let retained = conn
        .browser_context_by_id("CTX-pending-target")
        .and_then(|context| context.page_navigation_engine("TID-pending-active"))
        .expect("a pending initial document must survive target activation");
    assert_eq!(
        retained.renderer_owner_id_for_diagnostics(),
        pending_renderer_owner,
        "target switching must retain the exact engine that owns the pending renderer page"
    );
}

#[tokio::test]
async fn removing_active_browser_context_switches_engine_to_promoted_context() {
    let mut conn = CdpConnection::new();
    let mut first = conn.new_browser_context("CTX-first".to_owned());
    first.set_active_target_id("TID-first");
    conn.insert_browser_context(first);
    let mut second = conn.new_browser_context("CTX-second".to_owned());
    second.set_active_target_id("TID-second");
    let second_renderer_runtime = second.renderer_runtime();
    conn.insert_browser_context(second);

    let removed = conn
        .remove_browser_context_by_id_restoring_active_async("CTX-first", Some("CTX-first"))
        .await
        .expect("active context should be removable");

    assert_eq!(removed.id, "CTX-first");
    assert_eq!(
        conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("CTX-second")
    );
    assert!(
        conn.active_navigation_engine()
            .browser_context_runtime()
            .shares_state_with(&second_renderer_runtime)
    );
}

#[tokio::test]
async fn memory_diagnostics_reports_page_vm_document_isolate_model() {
    let mut conn = CdpConnection::new();
    conn.replace_standalone_navigation_engine(
        NavigationEngine::new_with_page_vm_document_isolate_for_diagnostics(),
    );

    let mut browser_context = conn.new_browser_context("BID-shared-diagnostics".to_owned());
    browser_context.set_active_target_id("TID-shared-diagnostics-active");
    browser_context.stage_background_target(
        "TID-shared-diagnostics-bg".to_owned(),
        Some("SID-shared-diagnostics-bg".to_owned()),
        "data:text/html,<!doctype html><body>second</body>".to_owned(),
        Some("about:blank".to_owned()),
        None,
    );
    conn.insert_browser_context(browser_context);
    let first_page = conn
        .load_page_via_runtime_async("data:text/html,<!doctype html><body>first</body>")
        .await
        .expect("first shared diagnostics page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .set_active_target_id("TID-shared-diagnostics-bg");
    let second_page = conn
        .load_page_via_runtime_async("data:text/html,<!doctype html><body>second</body>")
        .await
        .expect("second shared diagnostics page should load");
    let browser_context = conn.browser_context.as_mut().expect("browser context");
    browser_context
        .page_target_mut("TID-shared-diagnostics-active")
        .expect("active diagnostics target")
        .replace_loaded_page(Some(first_page));
    browser_context
        .page_target_mut("TID-shared-diagnostics-bg")
        .expect("background diagnostics target")
        .replace_loaded_page(Some(second_page));

    let pending_diagnostics = conn
        .start_moli_diagnostics()
        .expect("moli diagnostics dispatch should start");
    let diagnostics = conn.complete_moli_diagnostics(
        pending_diagnostics
            .wait()
            .await
            .expect("moli diagnostics dispatch should finish"),
    );

    let memory_cache = &diagnostics["connection"]["activeNavigationEngine"]["networkMemoryCache"];
    assert!(
        diagnostics["connection"]["activeNavigationEngine"]["resourceRuntimeId"]
            .as_u64()
            .is_some_and(|runtime_id| runtime_id > 0),
        "a materialized ResourceRequestClient should expose its shared browser resource runtime identity"
    );
    assert_eq!(memory_cache["retainedBytes"], json!(0));
    assert_eq!(memory_cache["retainedBytesLimit"], json!(15 * 1024 * 1024));
    assert_eq!(
        memory_cache["resourceBodyBytesLimit"],
        json!(3 * 1024 * 1024)
    );

    assert_eq!(
        diagnostics["isolateScope"]["documentIsolateModel"],
        json!("page-vm")
    );
    assert_eq!(
        diagnostics["isolateScope"]["loadedDocumentPageCount"],
        json!(2)
    );
    assert_eq!(
        diagnostics["isolateScope"]["estimatedRendererOwnerCount"],
        json!(2)
    );
    assert_eq!(
        diagnostics["isolateScope"]["loadedDocumentRendererOwnerCount"],
        json!(2),
        "each loaded PageTargetHost owns an independently schedulable renderer owner"
    );
    assert_eq!(
        diagnostics["isolateScope"]["estimatedDocumentIsolateCount"],
        json!(2),
        "page-vm diagnostics should count one document isolate per loaded page"
    );
    assert_eq!(
        diagnostics["isolateScope"]["estimatedWorkerIsolateCount"],
        json!(0)
    );
    assert_eq!(
        diagnostics["isolateScope"]["estimatedLiveV8IsolateCount"],
        json!(2),
        "two loaded PageVMs should report two live document isolates"
    );
    let isolate_accounting = &diagnostics["isolateScope"]["documentIsolateAccounting"];
    assert_eq!(isolate_accounting["scope"], json!("renderer-process"));
    for counter in ["created", "destroyed", "live", "reserved"] {
        assert!(
            isolate_accounting[counter].is_u64(),
            "document isolate accounting should expose numeric {counter}: {diagnostics:?}"
        );
    }
    assert_eq!(
        diagnostics["isolateScope"]["documentContextCount"],
        json!(2),
        "HeapProfiler.moliDiagnostics should aggregate loaded page document contexts"
    );
    assert_eq!(
        diagnostics["isolateScope"]["isolatedWorldContextCount"],
        json!(0)
    );
    assert_eq!(
        diagnostics["isolateScope"]["childDefaultContextCount"],
        json!(0)
    );
}

#[tokio::test]
async fn replacing_or_retiring_a_loaded_page_changes_its_attachment_identity() {
    let mut conn = CdpConnection::new();
    let first_page = conn
        .load_page_via_runtime_async("data:text/html,<body>first</body>")
        .await
        .expect("first Page");
    let second_page = conn
        .load_page_via_runtime_async("data:text/html,<body>second</body>")
        .await
        .expect("second Page");
    conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-page-attachment",
        "TID-page-attachment",
    ));
    let context = conn.browser_context.as_mut().unwrap();
    assert_eq!(
        context.active_target.runtime_slot.page_attachment_id(),
        None
    );

    assert!(context.replace_loaded_page(Some(first_page)).is_none());
    let first_attachment = context
        .active_target
        .runtime_slot
        .page_attachment_id()
        .expect("first Page attachment");

    let first = context
        .replace_loaded_page(Some(second_page))
        .expect("first Page should be replaced");
    let second_attachment = context
        .active_target
        .runtime_slot
        .page_attachment_id()
        .expect("second Page attachment");
    assert_ne!(second_attachment, first_attachment);
    let _ = first.close_async().await;

    let second = context
        .clear_loaded_page_with_reason(TargetPageAbsenceReason::TargetClosed)
        .expect("second Page should be retired");
    assert_eq!(
        context.active_target.runtime_slot.page_attachment_id(),
        None
    );
    let _ = second.close_async().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn moli_diagnostics_preserves_runtime_observable_diagnostics() {
    let mut ctx = TestContext::new();
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>diagnostics capture</body>")
        .await
        .expect("diagnostics capture page should load");
    let mut browser_context = BrowserContext::new("BID-diagnostics-capture".to_owned());
    browser_context.set_active_target_id("TID-diagnostics-capture");
    browser_context.attach_active_session("SID-diagnostics-capture");
    browser_context.set_target_url(page.final_url().as_str().to_owned());
    let _ = browser_context
        .active_target
        .runtime_slot
        .replace_loaded_page(Some(page));
    ctx.conn.browser_context = Some(browser_context);

    ctx.process_async(json!({
        "id": 44_100,
        "method": "Runtime.enable",
        "sessionId": "SID-diagnostics-capture",
    }))
    .await;
    let enable_response = ctx.take_response_by_id(44_100);
    assert_eq!(enable_response["result"], json!({}));
    ctx.sent.clear();

    ctx.conn
        .evaluate_runtime_expression_for_session_owner_async(
            Some("SID-diagnostics-capture"),
            "console.log('survives diagnostics')",
        )
        .await
        .expect("console expression should evaluate");

    let pending_diagnostics = ctx
        .conn
        .start_moli_diagnostics()
        .expect("moli diagnostics dispatch should start");
    let _ = ctx.conn.complete_moli_diagnostics(
        pending_diagnostics
            .wait()
            .await
            .expect("moli diagnostics dispatch should finish"),
    );

    let first_snapshot = ctx
        .conn
        .page_diagnostics_snapshot_for_session_owner_async(Some("SID-diagnostics-capture"))
        .await
        .expect("runtime observable diagnostics should remain readable after diagnostics");
    assert_eq!(first_snapshot.diagnostics.pending_inspector_messages, 0);
    let first_source = first_snapshot
        .runtime_observable_source()
        .expect("console evaluation should update read-only observable diagnostics")
        .clone();

    let second_snapshot = ctx
        .conn
        .page_diagnostics_snapshot_for_session_owner_async(Some("SID-diagnostics-capture"))
        .await
        .expect("a second read-only diagnostics snapshot should complete");
    assert_eq!(
        second_snapshot.runtime_observable_source(),
        Some(&first_source),
        "Moli diagnostics must not mutate the renderer's read-only observable summary"
    );
}

#[tokio::test]
async fn memory_diagnostics_excludes_empty_page_hosts_from_document_isolates() {
    let mut conn = CdpConnection::new();
    let mut browser_context = conn.new_browser_context("BID-doc-owner-diagnostics".to_owned());
    browser_context.set_active_target_id("TID-doc-owner-diagnostics-active");
    browser_context.stage_background_target(
        "TID-doc-owner-diagnostics-bg".to_owned(),
        Some("SID-doc-owner-diagnostics-bg".to_owned()),
        "data:text/html,<!doctype html><body>second</body>".to_owned(),
        Some("about:blank".to_owned()),
        None,
    );
    conn.insert_browser_context(browser_context);
    let first_page = conn
        .load_page_via_runtime_async("data:text/html,<!doctype html><body>first</body>")
        .await
        .expect("first shared diagnostics page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .set_active_target_id("TID-doc-owner-diagnostics-bg");
    let second_page = conn
        .load_page_via_runtime_async("data:text/html,<!doctype html><body>second</body>")
        .await
        .expect("second shared diagnostics page should load");
    let browser_context = conn.browser_context.as_mut().expect("browser context");
    browser_context
        .page_target_mut("TID-doc-owner-diagnostics-active")
        .expect("active diagnostics target")
        .replace_loaded_page(Some(first_page));
    browser_context
        .page_target_mut("TID-doc-owner-diagnostics-bg")
        .expect("background diagnostics target")
        .replace_loaded_page(Some(second_page));

    let mut empty_context = BrowserContext::new("BID-empty-page".to_owned());
    empty_context.set_active_target_id("TID-empty-page");
    conn.insert_browser_context(empty_context);
    let empty_page_engine = conn
        .browser_context_by_id("BID-empty-page")
        .and_then(|context| context.page_navigation_engine("TID-empty-page"))
        .expect("empty PageTargetHost must own its navigation engine");
    assert!(
        !empty_page_engine.shares_renderer_owner_with(conn.standalone_navigation_engine.ensure()),
        "test setup must keep a distinct renderer owner without a loaded document"
    );

    let diagnostics = conn.moli_memory_diagnostics();

    assert_eq!(
        diagnostics["isolateScope"]["loadedDocumentPageCount"],
        json!(2)
    );
    assert_eq!(
        diagnostics["isolateScope"]["pageNavigationEngineRendererOwnerCount"],
        json!(2)
    );
    assert_eq!(
        diagnostics["isolateScope"]["estimatedRendererOwnerCount"],
        json!(3),
        "the isolated empty Page host still contributes renderer owner fixed cost"
    );
    assert_eq!(
        diagnostics["isolateScope"]["loadedDocumentRendererOwnerCount"],
        json!(2),
        "the two loaded pages remain independently schedulable"
    );
    assert_eq!(
        diagnostics["isolateScope"]["estimatedDocumentIsolateCount"],
        json!(2),
        "each loaded PageVM contributes one document isolate"
    );
    assert_eq!(
        diagnostics["isolateScope"]["estimatedLiveV8IsolateCount"],
        json!(2),
        "empty Page hosts contribute fixed owner cost but no extra live V8 isolate"
    );
}

#[tokio::test]
async fn memory_diagnostics_sync_counts_dedicated_worker_from_cached_page_snapshot() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-sync-dedicated-worker".to_owned()));
    let mut page = conn
        .load_page_via_runtime_async("data:text/html,<!doctype html><body>worker</body>")
        .await
        .expect("sync diagnostics dedicated-worker page should load");
    let start_worker_response = page
        .evaluate_runtime_expression_async(
            r#"
(() => {
  globalThis.__lmSyncDiagnosticsWorkerReady = false;
  const source = "postMessage('ready'); setInterval(() => {}, 1000);";
  const worker = new Worker("data:text/javascript," + encodeURIComponent(source));
  worker.onmessage = () => { globalThis.__lmSyncDiagnosticsWorkerReady = true; };
  globalThis.__lmSyncDiagnosticsWorker = worker;
  return "started";
})()
"#,
        )
        .await
        .expect("dedicated worker should start");
    assert_eq!(
        start_worker_response["value"],
        json!("started"),
        "dedicated worker should be scheduled before sync diagnostics: {start_worker_response:?}"
    );

    let browser_context = conn.browser_context.as_mut().expect("browser context");
    browser_context.set_active_target_id("TID-sync-dedicated-worker");
    browser_context.replace_loaded_page(Some(page));

    for _ in 0..64 {
        let ready_response = conn
            .browser_context
            .as_mut()
            .and_then(|context| context.active_target.runtime_slot.loaded_page_mut())
            .expect("loaded sync diagnostics page")
            .evaluate_runtime_expression_async("globalThis.__lmSyncDiagnosticsWorkerReady === true")
            .await
            .expect("worker ready probe should evaluate");
        if ready_response["value"] != json!(true) {
            continue;
        }

        let diagnostics = conn.moli_memory_diagnostics();
        assert_eq!(
            diagnostics["isolateScope"]["estimatedDocumentIsolateCount"],
            json!(1)
        );
        assert_eq!(
            diagnostics["isolateScope"]["estimatedWorkerIsolateCount"],
            json!(1),
            "sync diagnostics must include page-owned dedicated worker isolates: {diagnostics:?}"
        );
        assert_eq!(
            diagnostics["isolateScope"]["estimatedLiveV8IsolateCount"],
            json!(2),
            "sync diagnostics live V8 total should be one document isolate plus one dedicated worker isolate: {diagnostics:?}"
        );
        return;
    }

    panic!("dedicated worker did not report ready before sync diagnostics assertion");
}

#[tokio::test]
async fn memory_diagnostics_counts_different_browser_context_document_isolates_separately() {
    let mut conn = CdpConnection::new();

    let mut first_context = conn.new_browser_context("BID-doc-owner-first".to_owned());
    first_context.set_active_target_id("TID-doc-owner-first");
    conn.insert_browser_context(first_context);

    let first_page = conn
        .load_page_via_runtime_async("data:text/html,<!doctype html><body>first-context</body>")
        .await
        .expect("first browser-context diagnostics page should load");
    conn.browser_context
        .as_mut()
        .expect("first browser context should be active")
        .replace_loaded_page(Some(first_page));

    let mut second_context = conn.new_browser_context("BID-doc-owner-second".to_owned());
    second_context.set_active_target_id("TID-doc-owner-second");
    conn.insert_browser_context(second_context);
    assert!(conn.activate_browser_context_by_id("BID-doc-owner-second"));

    let second_page = conn
        .load_page_via_runtime_async("data:text/html,<!doctype html><body>second-context</body>")
        .await
        .expect("second browser-context diagnostics page should load");
    conn.browser_context
        .as_mut()
        .expect("second browser context should be active")
        .replace_loaded_page(Some(second_page));

    let pending_diagnostics = conn
        .start_moli_diagnostics()
        .expect("moli diagnostics dispatch should start");
    let diagnostics = conn.complete_moli_diagnostics(
        pending_diagnostics
            .wait()
            .await
            .expect("moli diagnostics dispatch should finish"),
    );

    assert_eq!(
        diagnostics["isolateScope"]["loadedDocumentPageCount"],
        json!(2)
    );
    assert_eq!(
        diagnostics["isolateScope"]["pageNavigationEngineRendererOwnerCount"],
        json!(1),
        "switching browser contexts should retain the first loaded target's renderer owner fixed cost"
    );
    assert_eq!(
        diagnostics["isolateScope"]["loadedDocumentRendererOwnerCount"],
        json!(2),
        "different browser contexts must not collapse their document pages onto one renderer owner"
    );
    assert_eq!(
        diagnostics["isolateScope"]["estimatedDocumentIsolateCount"],
        json!(2),
        "different loaded PageVMs should report separate document isolates"
    );
    assert_eq!(
        diagnostics["isolateScope"]["estimatedLiveV8IsolateCount"],
        json!(2),
        "two live document isolates without workers should report two live V8 isolates"
    );
    assert_eq!(
        diagnostics["isolateScope"]["documentContextCount"],
        json!(2),
        "diagnostics should snapshot both browser-context document pages"
    );
}

#[test]
fn memory_diagnostics_counts_isolated_page_engines_by_renderer_owner() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-shared-diagnostics".to_owned());
    browser_context.set_active_target_id("TID-shared-diagnostics-active");
    browser_context.stage_background_target(
        "TID-shared-diagnostics-bg".to_owned(),
        None,
        "about:blank".to_owned(),
        Some("about:blank".to_owned()),
        None,
    );
    conn.insert_browser_context(browser_context);

    let active = conn.active_navigation_engine();
    let background = conn
        .browser_context
        .as_ref()
        .and_then(|context| context.page_navigation_engine("TID-shared-diagnostics-bg"))
        .expect("background PageTargetHost must own an engine");
    assert!(
        !background.shares_renderer_owner_with(active),
        "same-context PageTargetHost engines must remain independently schedulable"
    );

    let diagnostics = conn.moli_memory_diagnostics();

    assert_eq!(
        diagnostics["connection"]["pageNavigationEngineCount"],
        json!(2),
        "each PageTargetHost must expose one resident NavigationEngine"
    );
    assert_eq!(
        diagnostics["isolateScope"]["pageNavigationEngineRendererOwnerCount"],
        json!(1),
        "the background Page engine must count as another renderer owner"
    );
    assert_eq!(
        diagnostics["isolateScope"]["estimatedRendererOwnerCount"],
        json!(2)
    );
}

#[test]
fn page_navigation_engines_remain_target_local_across_selection() {
    let mut conn = CdpConnection::new();
    let mut browser_context = conn.new_browser_context("BID-engine-residence".to_owned());
    browser_context.set_active_target_id("TID-engine-first");
    browser_context.stage_background_target(
        "TID-engine-second".to_owned(),
        None,
        "about:blank".to_owned(),
        Some("about:blank".to_owned()),
        None,
    );
    conn.insert_browser_context(browser_context);

    conn.active_navigation_engine_mut()
        .set_user_agent_override("Moli/Page-First");

    conn.browser_context
        .as_mut()
        .expect("browser context")
        .set_active_target_id("TID-engine-second");
    assert_ne!(
        conn.active_navigation_engine()
            .fetch_config()
            .browser_identity()
            .user_agent(),
        "Moli/Page-First",
        "selecting another Page must expose that host's own engine"
    );
    conn.active_navigation_engine_mut()
        .set_user_agent_override("Moli/Page-Second");

    conn.browser_context
        .as_mut()
        .expect("browser context")
        .set_active_target_id("TID-engine-first");
    assert_eq!(
        conn.active_navigation_engine()
            .fetch_config()
            .browser_identity()
            .user_agent(),
        "Moli/Page-First",
        "selection must not replace or move a PageTargetHost engine"
    );
    assert_eq!(
        conn.browser_context
            .as_ref()
            .and_then(|context| context.page_navigation_engine("TID-engine-second"))
            .expect("second Page engine")
            .fetch_config()
            .browser_identity()
            .user_agent(),
        "Moli/Page-Second",
        "the unselected Page must retain its engine state in place"
    );
}

#[test]
fn page_target_host_rejects_a_foreign_browser_context_engine() {
    let mut conn = CdpConnection::new();
    let mut context = BrowserContext::new("BID-retain-route".to_owned());
    context.set_active_target_id("TID-retain-route");
    conn.insert_browser_context(context);
    let original_owner = conn
        .active_navigation_engine()
        .renderer_owner_id_for_diagnostics();
    let foreign_context = BrowserContext::new("BID-retain-route-foreign".to_owned());
    let foreign_engine = NavigationEngine::new_with_fetch_config_and_browser_context_access(
        FetchConfig::default(),
        foreign_context.renderer_runtime_owner_access(),
        conn.standalone_navigation_engine
            .ensure()
            .optional_resource_fetch_mask(),
        conn.standalone_navigation_engine
            .ensure()
            .subframe_loading_enabled(),
    )
    .expect("foreign context owner should be live during the regression");

    let error = conn
        .install_page_navigation_engine("BID-retain-route", "TID-retain-route", foreign_engine)
        .expect_err("a PageTargetHost must reject an engine from another BrowserContext");

    assert!(error.contains("does not match BrowserContext `BID-retain-route`"));
    assert_eq!(
        conn.active_navigation_engine()
            .renderer_owner_id_for_diagnostics(),
        original_owner,
        "rejecting a foreign engine must preserve the host's resident engine"
    );
}

#[tokio::test]
async fn memory_diagnostics_splits_pending_inspector_await_counts_by_target_owner() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new(
        "BID-pending-await-diagnostics".to_owned(),
    ));

    let active_page = conn
        .load_page_via_runtime_async("data:text/html,<!doctype html><body>active</body>")
        .await
        .expect("active diagnostics page should load");
    let background_page = conn
        .load_page_via_runtime_async("data:text/html,<!doctype html><body>background</body>")
        .await
        .expect("background diagnostics page should load");

    let browser_context = conn.browser_context.as_mut().expect("browser context");
    browser_context.set_active_target_id("TID-pending-await-active");
    browser_context.attach_active_session("SID-pending-await-active");
    browser_context.replace_loaded_page(Some(active_page));
    browser_context.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .register_pending_inspector_await(10_001, Some("SID-pending-await-active"), None);
    browser_context.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .register_pending_inspector_await(
            10_002,
            Some("SID-pending-await-active"),
            Some("active-group"),
        );

    let mut background = super::PageTargetHost::with_url(
        "TID-pending-await-bg".to_owned(),
        Some("SID-pending-await-bg".to_owned()),
        "data:text/html,<!doctype html><body>background</body>".to_owned(),
    );
    background.replace_loaded_page(Some(background_page));
    background.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .register_pending_inspector_await(20_001, Some("SID-pending-await-bg"), None);
    browser_context.insert_page_target_host(background);

    let shared_worker_instance_id = SharedWorkerInstanceId::from_u64(30_001);
    let mut shared_worker_target = SharedWorkerTargetState::new(
        moli_core::RendererOwnerLocalHostId::new_for_testing(1),
        shared_worker_instance_id,
        "TID-pending-await-sw".to_owned(),
        Some("TID-pending-await-active".to_owned()),
        "https://example.test/sw.js".to_owned(),
        "diagnostics-sw".to_owned(),
    );
    shared_worker_target.attach_session("SID-pending-await-sw".to_owned());
    shared_worker_target.register_pending_inspector_await(
        "SID-pending-await-sw",
        30_001,
        Some("SID-pending-await-sw"),
        None,
    );
    browser_context
        .shared_worker_targets
        .insert(shared_worker_instance_id, shared_worker_target);

    let service_worker_version_id = 40_001;
    let mut service_worker_target = ServiceWorkerTargetState::new(
        40_000,
        service_worker_version_id,
        "TID-pending-await-service-worker".to_owned(),
        "https://example.test/service-worker.js".to_owned(),
        "https://example.test/".to_owned(),
        RendererServiceWorkerVersionStatus::Activated,
        None,
    );
    service_worker_target.attach_session("SID-pending-await-service-worker".to_owned());
    service_worker_target.register_pending_inspector_await(
        "SID-pending-await-service-worker",
        40_001,
        Some("SID-pending-await-service-worker"),
        None,
    );
    browser_context
        .service_worker_targets
        .insert(service_worker_version_id, service_worker_target);

    let diagnostics = conn.moli_memory_diagnostics();

    assert_eq!(
        diagnostics["isolateScope"]["pendingInspectorAwaitCount"],
        json!(5)
    );
    assert_eq!(
        diagnostics["isolateScope"]["pageTargetPendingInspectorAwaitCount"],
        json!(3)
    );
    assert_eq!(
        diagnostics["isolateScope"]["pageTargetWithPendingInspectorAwaitCount"],
        json!(2)
    );
    assert_eq!(
        diagnostics["isolateScope"]["sharedWorkerTargetPendingInspectorAwaitCount"],
        json!(1)
    );
    assert_eq!(
        diagnostics["isolateScope"]["sharedWorkerTargetWithPendingInspectorAwaitCount"],
        json!(1)
    );
    assert_eq!(
        diagnostics["isolateScope"]["serviceWorkerTargetPendingInspectorAwaitCount"],
        json!(1)
    );
    assert_eq!(
        diagnostics["isolateScope"]["serviceWorkerTargetWithPendingInspectorAwaitCount"],
        json!(1)
    );
    assert_eq!(
        diagnostics["activeBrowserContext"]["pendingInspectorAwaitCount"],
        json!(5)
    );
    assert_eq!(
        diagnostics["activeBrowserContext"]["pageTargetPendingInspectorAwaitCount"],
        json!(3)
    );
    assert_eq!(
        diagnostics["activeBrowserContext"]["sharedWorkerTargetPendingInspectorAwaitCount"],
        json!(1)
    );
    assert_eq!(
        diagnostics["activeBrowserContext"]["serviceWorkerTargetPendingInspectorAwaitCount"],
        json!(1)
    );
    assert_eq!(
        diagnostics["activeBrowserContext"]["serviceWorkerTargetWithPendingInspectorAwaitCount"],
        json!(1)
    );
    assert_eq!(
        diagnostics["activeBrowserContext"]["runtimeSession"]["pendingInspectorAwaitCount"],
        json!(2)
    );
    assert_eq!(
        diagnostics["activeBrowserContext"]["runtimeSession"]["primaryPendingInspectorAwaitCount"],
        json!(2)
    );
    assert_eq!(
        diagnostics["activeBrowserContext"]["targetHosts"]["pendingInspectorAwaitCount"],
        json!(1)
    );
}

#[test]
fn navigation_background_event_queue_drops_stale_token() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("CTX-nav".to_owned());
    browser_context.set_active_target_id("TID-nav");
    let stale = browser_context
        .start_document_navigation_for_active_target("LOADER-1".to_owned())
        .expect("active target should produce stale token");
    let current = browser_context
        .start_document_navigation_for_active_target("LOADER-2".to_owned())
        .expect("active target should produce current token");
    conn.browser_context = Some(browser_context);
    let stale_message = build_event(
        "Page.frameStartedLoading",
        json!({ "frameId": "TID-nav", "loaderId": "LOADER-1" }),
        None,
    );
    let current_message = build_event(
        "Page.frameStartedLoading",
        json!({ "frameId": "TID-nav", "loaderId": "LOADER-2" }),
        None,
    );

    conn.enqueue_navigation_background_event(NavigationBackgroundEvent::protocol_message(
        stale,
        stale_message,
    ));
    conn.enqueue_navigation_background_event(NavigationBackgroundEvent::protocol_message(
        current,
        current_message.clone(),
    ));

    assert_eq!(
        conn.drain_navigation_background_events(),
        vec![current_message]
    );
}

#[test]
fn navigation_background_event_queue_preserves_order_for_current_token() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("CTX-nav-order".to_owned());
    browser_context.set_active_target_id("TID-nav-order");
    let stale = browser_context
        .start_document_navigation_for_active_target("LOADER-1".to_owned())
        .expect("active target should produce stale navigation token");
    let current = browser_context
        .start_document_navigation_for_active_target("LOADER-2".to_owned())
        .expect("active target should produce current navigation token");
    conn.browser_context = Some(browser_context);

    let stale_message = build_event(
        "Page.frameStartedLoading",
        json!({ "frameId": "TID-nav-order", "loaderId": "LOADER-1" }),
        None,
    );
    let current_first_message = build_event(
        "Page.frameStartedLoading",
        json!({ "frameId": "TID-nav-order", "loaderId": "LOADER-2", "step": 1 }),
        None,
    );
    let current_second_message = build_event(
        "Page.frameStoppedLoading",
        json!({ "frameId": "TID-nav-order", "loaderId": "LOADER-2", "step": 2 }),
        None,
    );

    conn.enqueue_navigation_background_event(NavigationBackgroundEvent::protocol_message(
        stale,
        stale_message,
    ));
    conn.enqueue_navigation_background_event(NavigationBackgroundEvent::protocol_message(
        current.clone(),
        current_first_message.clone(),
    ));
    conn.enqueue_navigation_background_event(NavigationBackgroundEvent::protocol_message(
        current,
        current_second_message.clone(),
    ));

    assert_eq!(
        conn.drain_navigation_background_events(),
        vec![current_first_message, current_second_message]
    );
}

#[test]
fn navigation_background_event_sender_preserves_typed_sidecar_for_current_token() {
    let mut conn = CdpConnection::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    conn.set_background_event_sender(tx);
    let mut browser_context = BrowserContext::new("CTX-nav-typed".to_owned());
    browser_context.set_active_target_id("TID-nav-typed");
    let current = browser_context
        .start_document_navigation_for_active_target("LOADER-typed".to_owned())
        .expect("active target should produce current navigation token");
    conn.browser_context = Some(browser_context);
    let message = build_event(
        "Page.frameStartedNavigating",
        json!({
            "frameId": "TID-nav-typed",
            "loaderId": "LOADER-typed",
            "url": "https://example.test/",
            "navigationType": "differentDocument"
        }),
        None,
    );
    let automation_event = AutomationEvent::NavigationFrame(NavigationFrameEvent {
        target_id: DevToolsTargetId::from("TID-nav-typed"),
        frame_id: DevToolsFrameId::from("TID-nav-typed"),
        parent_frame_id: None,
        loader_id: Some(DevToolsLoaderId::from("LOADER-typed")),
        url: "https://example.test/".to_owned(),
        kind: NavigationFrameEventKind::StartedNavigating,
        frame_name: None,
        security_origin: None,
        secure_context_type: None,
    });

    conn.send_navigation_background_protocol_event(
        current,
        BackgroundProtocolEvent::immediate_automation_event(
            message.clone(),
            automation_event.clone(),
        ),
    );

    let background_event = rx
        .try_recv()
        .expect("current navigation event should flush to background sender");
    let (actual_message, actual_automation_event) = background_event.into_parts();
    assert_eq!(actual_message, message);
    assert_eq!(actual_automation_event, Some(automation_event));
}

#[tokio::test]
async fn materialized_navigation_completion_drops_stale_token() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("CTX-nav".to_owned());
    browser_context.set_active_target_id("TID-nav");
    let stale = browser_context
        .start_document_navigation_for_active_target("LOADER-1".to_owned())
        .expect("active target should produce stale token");
    let _current = browser_context
        .start_document_navigation_for_active_target("LOADER-2".to_owned())
        .expect("active target should produce current token");
    conn.browser_context = Some(browser_context);
    let state =
        materialized_navigation_test_state(Some(7), "LOADER-1", "https://example.test/stale");
    let navigation =
        MaterializedNavigationLoadOutcome::Failed(MaterializedFailedDocumentProgress {
            error_text: "stale navigation should not emit".to_owned(),
            document_policy: FailedNavigationDocumentPolicy::InvalidateCommittedDocument,
            response_mode: FailedNavigationResponseMode::ProtocolError,
            progress_gate: empty_main_document_progress_gate_for_test(),
        });

    let mut out = Vec::new();
    let mut command_context = CommandDispatchContext::default();
    conn.drain_materialized_navigation_completion_into(
        &mut out,
        MaterializedNavigationCompletion::new(stale, state, navigation),
        &mut command_context,
    )
    .await;

    assert_eq!(out.len(), 1, "stale completion must emit terminal reply");
    let reply = &out[0];
    assert_eq!(reply["id"], serde_json::json!(7));
    assert!(
        reply.get("error").is_none(),
        "CDP reports a superseded Page.navigate as a successful command: {reply:#?}"
    );
    assert_eq!(
        reply["result"],
        serde_json::json!({
            "frameId": "TID-nav",
            "errorText": "net::ERR_ABORTED",
            "isDownload": false
        })
    );
    assert!(
        reply.get("method").is_none(),
        "stale completion must emit a command reply, not an event"
    );
}

#[tokio::test]
async fn materialized_navigation_completion_drops_stale_token_without_navigate_id() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("CTX-nav-none".to_owned());
    browser_context.set_active_target_id("TID-nav-none");
    let stale = browser_context
        .start_document_navigation_for_active_target("LOADER-1".to_owned())
        .expect("active target should produce stale navigation token");
    let _ = browser_context
        .start_document_navigation_for_active_target("LOADER-2".to_owned())
        .expect("active target should produce current navigation token");
    conn.browser_context = Some(browser_context);
    let state =
        materialized_navigation_test_state(None, "LOADER-1", "https://example.test/stale-no-id");
    let navigation =
        MaterializedNavigationLoadOutcome::Failed(MaterializedFailedDocumentProgress {
            error_text: "stale navigation should not emit without a navigate id".to_owned(),
            document_policy: FailedNavigationDocumentPolicy::InvalidateCommittedDocument,
            response_mode: FailedNavigationResponseMode::ProtocolError,
            progress_gate: empty_main_document_progress_gate_for_test(),
        });

    let mut out = Vec::new();
    let mut command_context = CommandDispatchContext::default();
    conn.drain_materialized_navigation_completion_into(
        &mut out,
        MaterializedNavigationCompletion::new(stale, state, navigation),
        &mut command_context,
    )
    .await;

    assert!(
        out.is_empty(),
        "stale completion without navigate id must not emit protocol output"
    );
}

#[tokio::test]
async fn materialized_navigation_completion_drains_current_token() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("CTX-nav".to_owned());
    browser_context.set_active_target_id("TID-nav");
    let current = browser_context
        .start_document_navigation_for_active_target("LOADER-1".to_owned())
        .expect("active target should produce current token");
    conn.browser_context = Some(browser_context);
    let state =
        materialized_navigation_test_state(Some(8), "LOADER-1", "https://example.test/current");
    let navigation =
        MaterializedNavigationLoadOutcome::Failed(MaterializedFailedDocumentProgress {
            error_text: "current navigation should emit".to_owned(),
            document_policy: FailedNavigationDocumentPolicy::InvalidateCommittedDocument,
            response_mode: FailedNavigationResponseMode::ProtocolError,
            progress_gate: empty_main_document_progress_gate_for_test(),
        });

    let mut out = Vec::new();
    let mut command_context = CommandDispatchContext::default();
    conn.drain_materialized_navigation_completion_into(
        &mut out,
        MaterializedNavigationCompletion::new(current, state, navigation),
        &mut command_context,
    )
    .await;

    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["id"], json!(8));
    assert_eq!(out[0]["error"]["code"], json!(-32000));
    assert_eq!(
        out[0]["error"]["message"],
        json!("current navigation should emit")
    );
}

fn materialized_navigation_test_state(
    navigate_id: Option<u64>,
    loader_id: &str,
    requested_url: &str,
) -> NavigationDispatchState {
    NavigationDispatchState {
        navigate_id,
        navigate_session_id: None,
        result_projection: NavigationResultProjection::Cdp(
            json!({ "frameId": "TID-nav", "loaderId": loader_id }),
        ),
        frame_id: "TID-nav".to_owned(),
        session_id: None,
        request_id: Some(loader_id.to_owned()),
        loader_id: loader_id.to_owned(),
        request_announced: true,
        requested_url: Url::parse(requested_url).unwrap(),
        request_method: "GET".to_owned(),
        request_body: None,
        request_body_bytes: None,
        request_headers: Vec::new(),
        request_load_policy: crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
        timestamp: 0.0,
        source_document_security: Default::default(),
    }
}

fn site_summary(
    name: &str,
    cookie_count: usize,
    persistent_cookie_count: usize,
) -> CookieSiteDataSummary {
    CookieSiteDataSummary::new(name.to_owned(), cookie_count, persistent_cookie_count)
}
