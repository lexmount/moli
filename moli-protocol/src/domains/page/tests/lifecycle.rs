use super::*;
use crate::domains::page::{
    PageScreencastCaptureCompletion, PageScreencastCaptureStart, PageScreencastSubscriptionStatus,
};

#[test]
fn child_frame_security_identity_matches_chromium_cdp_url_projection() {
    assert_eq!(
        child_frame_security_identity("about:blank", true, false, "https://top.example", "Secure"),
        ("://".to_owned(), "Secure".to_owned())
    );
    assert_eq!(
        child_frame_security_identity("about:blank", true, true, "https://top.example", "Secure"),
        ("://".to_owned(), "Secure".to_owned())
    );
    assert_eq!(
        child_frame_security_identity("about:srcdoc", true, false, "://", "InsecureScheme"),
        ("://".to_owned(), "InsecureScheme".to_owned())
    );
    assert_eq!(
        child_frame_security_identity(
            "data:text/html,<body>child</body>",
            false,
            false,
            "https://top.example",
            "Secure",
        ),
        ("null".to_owned(), "InsecureScheme".to_owned())
    );
    assert_eq!(
        child_frame_security_identity(
            "https://child.example/",
            false,
            false,
            "https://top.example",
            "Secure",
        ),
        ("https://child.example".to_owned(), "Secure".to_owned())
    );
    assert_eq!(
        child_frame_security_identity(
            "http://localhost/",
            false,
            false,
            "https://top.example",
            "Secure",
        ),
        ("http://localhost".to_owned(), "Secure".to_owned())
    );
}
#[test]
fn page_owner_state_commands_complete_through_command_dispatch() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-PAGE-COMPLETE".into()));

    for (id, method, params) in [
        (
            1210,
            "Page.setBypassCSP",
            json!({
                "enabled": true
            }),
        ),
        (
            1211,
            "Page.setFontFamilies",
            json!({
                "standard": "Times New Roman",
                "fixed": "Courier New"
            }),
        ),
        (
            1212,
            "Page.setInterceptFileChooserDialog",
            json!({
                "enabled": true
            }),
        ),
    ] {
        let raw = json!({
            "id": id,
            "method": method,
            "params": params
        })
        .to_string();
        let CdpCommandTaskStep::Complete(outcome) = ctx.conn.start_command_dispatch(&raw) else {
            panic!("Page owner-state command should complete without renderer wait");
        };
        let (messages, scheduler_events) = outcome.into_parts();
        assert!(
            scheduler_events.is_empty(),
            "Page owner-state command should not enqueue scheduler events: {scheduler_events:?}"
        );
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["id"], json!(id));
        assert_eq!(messages[0]["result"], json!({}));
    }

    let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
    assert!(
        browser_context
            .devtools_session_state
            .page_session_state
            .page_bypass_csp_enabled
    );
    assert_eq!(
        browser_context
            .devtools_session_state
            .page_session_state
            .page_font_families
            .get("standard"),
        Some(&json!("Times New Roman"))
    );
    assert!(
        browser_context
            .devtools_session_state
            .page_session_state
            .page_intercept_file_chooser_dialog_enabled
    );

    {
        let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
        browser_context
            .renderer_runtime()
            .set_javascript_dialog_handler_enabled(true);
        let page_session_state = &mut browser_context.devtools_session_state.page_session_state;
        page_session_state.page_domain_enabled = true;
        page_session_state.page_lifecycle_events = true;
        page_session_state.page_file_chooser_opened_event_enabled = true;
    }

    let raw = json!({
        "id": 1214,
        "method": "Page.disable"
    })
    .to_string();
    let CdpCommandTaskStep::Complete(outcome) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("Page.disable should complete without renderer wait");
    };
    let (messages, scheduler_events) = outcome.into_parts();
    assert!(
        scheduler_events.is_empty(),
        "Page.disable should not enqueue scheduler events: {scheduler_events:?}"
    );
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["id"], json!(1214));
    assert_eq!(messages[0]["result"], json!({}));

    let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_page_domain_disabled(&browser_context.devtools_session_state.page_session_state);
    assert!(
        !browser_context
            .renderer_runtime()
            .javascript_dialog_handler_enabled(),
        "Page.disable should disengage renderer JavaScript dialog handling"
    );
}

fn mark_page_domain_enabled(state: &mut crate::conn::TargetPageSessionState) {
    state.page_domain_enabled = true;
    state.page_lifecycle_events = true;
    state.page_bypass_csp_enabled = true;
    state
        .page_font_families
        .insert("standard".to_owned(), json!("Inter"));
    state.page_file_chooser_opened_event_enabled = true;
    state.page_intercept_file_chooser_dialog_enabled = true;
    state
        .page_screencast
        .start(crate::conn::PageScreencastConfig::default());
    state.javascript_dialog_state.push(target_dialog_for_test(
        crate::conn::TargetPageResidenceIdentity::new_for_test(
            "BID-dialog".to_owned(),
            Some("TID-dialog".to_owned()),
            1,
        ),
        "TID-dialog",
        "alert",
        "pending",
        "",
        None,
    ));
}

fn assert_page_domain_enabled(state: &crate::conn::TargetPageSessionState) {
    assert!(state.page_domain_enabled);
    assert!(state.page_lifecycle_events);
    assert!(state.page_bypass_csp_enabled);
    assert_eq!(
        state.page_font_families.get("standard"),
        Some(&json!("Inter"))
    );
    assert!(state.page_file_chooser_opened_event_enabled);
    assert!(state.page_intercept_file_chooser_dialog_enabled);
    assert!(state.page_screencast.is_active());
    assert!(state.page_screencast.generation() > 0);
    assert!(!state.javascript_dialog_state.is_empty());
}

fn assert_page_domain_disabled(state: &crate::conn::TargetPageSessionState) {
    assert!(!state.page_domain_enabled);
    assert!(!state.page_lifecycle_events);
    assert!(!state.page_bypass_csp_enabled);
    assert!(state.page_font_families.is_empty());
    assert!(!state.page_file_chooser_opened_event_enabled);
    assert!(!state.page_intercept_file_chooser_dialog_enabled);
    assert!(!state.page_screencast.is_active());
    assert!(state.javascript_dialog_state.is_empty());
}

fn enable_page_domain_for_session(conn: &mut crate::conn::CdpConnection, session_id: &str) {
    assert!(conn.set_page_domain_enabled_for_session_owner(Some(session_id), true));
    assert!(matches!(
        conn.set_page_lifecycle_events_enabled_for_session_owner(Some(session_id), true),
        crate::conn::PageLifecycleEventsEnableResult::Handled { .. }
    ));
    assert!(conn.set_page_bypass_csp_enabled_for_session_owner(Some(session_id), true));
    let mut font_families = serde_json::Map::new();
    font_families.insert("standard".to_owned(), json!("Inter"));
    assert!(conn.set_page_font_families_for_session_owner(Some(session_id), font_families));
    assert!(
        conn.set_page_file_chooser_opened_event_enabled_for_session_owner(Some(session_id), true)
    );
    assert!(
        conn.set_page_intercept_file_chooser_dialog_enabled_for_session_owner(
            Some(session_id),
            true
        )
    );
    assert!(
        conn.start_page_screencast_for_session_owner(
            Some(session_id),
            crate::conn::PageScreencastConfig::default(),
        )
        .is_some()
    );
    conn.with_target_devtools_session_state_for_session_mut(Some(session_id), |state| {
        state
            .page_session_state
            .javascript_dialog_state
            .push(target_dialog_for_test(
                crate::conn::TargetPageResidenceIdentity::new_for_test(
                    "BID-dialog".to_owned(),
                    Some("TID-dialog".to_owned()),
                    1,
                ),
                "TID-dialog",
                "alert",
                "pending",
                "",
                None,
            ));
    });
}

async fn install_runtime_document_replacement_test_page(ctx: &mut TestContext) -> i64 {
    load_bc_with_session(
        ctx,
        "BID-runtime-replacement",
        "TID-1",
        "SID-1",
        "about:blank",
    );
    let mut navigation = ctx
        .conn
        .load_navigation_via_runtime_async("data:text/html,<body>initial</body>")
        .await
        .expect("runtime replacement fixture page should load");
    let navigation_engine = navigation.navigation_engine.take();
    let artifacts = navigation.page_creation_artifacts;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("runtime replacement fixture browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(navigation.page);
    let (binding, initial_events) = ctx.conn.bind_renderer_document_lifecycle_for_session_owner(
        Some("SID-1"),
        artifacts,
        None,
        "TID-1".to_owned(),
        LOADER_ID.to_owned(),
    );
    assert!(binding.is_some());
    assert_eq!(
        initial_events.len(),
        2,
        "prepared data navigation commits at DOMContentLoaded; load remains renderer-owned tail work"
    );
    if let Some(navigation_engine) = navigation_engine {
        ctx.conn
            .adopt_loaded_navigation_engine_for_session_owner(Some("SID-1"), navigation_engine);
    }
    ctx.enable_dom_events_for_test(Some("SID-1"));

    ctx.process_async(json!({
        "id": 90,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-1",
        "params": { "enabled": true }
    }))
    .await;
    let _ = ctx.take_all();
    ctx.process_async(json!({
        "id": 91,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = ctx.take_all();
    ctx.process_async(json!({
        "id": 92,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "__chromium_document_write_probe__"
        }
    }))
    .await;
    let isolated_context_id = take_response_by_id(ctx, 92)["result"]["executionContextId"]
        .as_i64()
        .expect("runtime replacement fixture isolated context id");
    ctx.sent.clear();
    isolated_context_id
}

async fn wait_for_visible_renderer_load(
    ctx: &mut TestContext,
    session_id: &str,
) -> moli_core::page::RendererLifecycleEventStamp {
    ctx.wait_until_scheduler_state("renderer load lifecycle", |conn| {
        conn.renderer_document_lifecycle_visible_state_for_session_owner(Some(session_id))
            .is_some_and(|(_, snapshot)| snapshot.load.is_some())
    })
    .await;
    ctx.conn
        .renderer_document_lifecycle_visible_state_for_session_owner(Some(session_id))
        .and_then(|(_, snapshot)| snapshot.load)
        .expect("real scheduler input should make renderer load protocol-visible")
}

async fn wait_for_authoritative_renderer_load(
    ctx: &mut TestContext,
    session_id: &str,
) -> moli_core::page::RendererLifecycleEventStamp {
    ctx.wait_until_scheduler_state("authoritative renderer load lifecycle", |conn| {
        conn.renderer_document_lifecycle_authoritative_state_for_session_owner(Some(session_id))
            .is_some_and(|(_, snapshot)| snapshot.load.is_some())
    })
    .await;
    ctx.conn
        .renderer_document_lifecycle_authoritative_state_for_session_owner(Some(session_id))
        .and_then(|(_, snapshot)| snapshot.load)
        .expect("real scheduler input should make renderer load authoritative")
}

fn take_released_renderer_load_event(
    ctx: &mut TestContext,
    session_id: &str,
    load_stamp: moli_core::page::RendererLifecycleEventStamp,
) -> moli_core::page::RendererDocumentLifecycleEvent {
    let released = ctx
        .conn
        .release_renderer_document_load_visibility_barrier_for_session_owner(
            Some(session_id),
            LOADER_ID,
        )
        .expect("load visibility barrier should remain active");
    let load_event = released
        .into_iter()
        .find(|event| {
            matches!(
                event.kind,
                RendererDocumentLifecycleEventKind::Milestone(
                    RendererDocumentLifecycleMilestone::Load
                )
            )
        })
        .expect("releasing the visibility barrier should publish renderer load");
    assert_eq!(load_event.sequence, load_stamp.sequence);
    assert_eq!(load_event.timestamp_micros, load_stamp.timestamp_micros);
    load_event
}

fn console_message_index(messages: &[serde_json::Value], value: &str) -> usize {
    messages
        .iter()
        .position(|message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["args"][0]["value"] == json!(value)
        })
        .unwrap_or_else(|| panic!("missing Runtime.consoleAPICalled `{value}`: {messages:?}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn lifecycle_events_enable_without_renderer_binding_does_not_synthesize_replay() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-PAGE-LIFECYCLE-COMPLETE",
        "TID-PAGE-LIFECYCLE-COMPLETE",
        "SID-PAGE-LIFECYCLE-COMPLETE",
        "about:blank",
    );
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>lifecycle</body>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = json!({
        "id": 1213,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-PAGE-LIFECYCLE-COMPLETE",
        "params": { "enabled": true }
    })
    .to_string();
    let CdpCommandTaskStep::Complete(outcome) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("Page.setLifecycleEventsEnabled should complete without renderer wait");
    };
    let (messages, scheduler_events) = outcome.into_parts();
    assert!(
        scheduler_events.is_empty(),
        "Page.setLifecycleEventsEnabled replay should not enqueue scheduler events: {scheduler_events:?}"
    );
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["id"], json!(1213));
    assert_eq!(messages[0]["result"], json!({}));
}
/// cdp.page: enable returns empty result
#[tokio::test(flavor = "multi_thread")]
async fn enable_returns_empty_result() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-PAGE-ENABLE",
        "TID-PAGE-ENABLE",
        "SID-PAGE-ENABLE",
        "about:blank",
    );
    ctx.process_async(json!({
        "id": 1,
        "method": "Page.enable",
        "sessionId": "SID-PAGE-ENABLE"
    }))
    .await;
    ctx.expect_result(1, json!({}), Some("SID-PAGE-ENABLE"));
}

async fn assert_page_enable_uses_fresh_initial_document_without_adapter(target_url: &str) {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 120,
        "method": "Target.createTarget",
        "params": { "url": target_url }
    }))
    .await;
    ctx.expect_event("Target.targetCreated", None);
    let create_response = take_response_by_id(&mut ctx, 120);
    assert!(
        create_response["result"]["targetId"].as_str().is_some(),
        "Target.createTarget should return target id: {create_response}"
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|bc| bc.active_target.runtime_slot.loaded_page())
            .is_some_and(|page| page.final_url().as_str() == target_url),
        "Target.createTarget should install the initial about:blank owner page"
    );

    let raw = json!({
        "id": 121,
        "method": "Page.enable"
    })
    .to_string();
    let CdpCommandTaskStep::Complete(outcome) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("Page.enable should observe the already-loaded initial document without pending");
    };
    let (messages, scheduler_events) = outcome.into_parts();
    assert!(
        scheduler_events.is_empty(),
        "{target_url} Page.enable should not enqueue scheduler events: {scheduler_events:?}"
    );
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(121))
        .expect("Page.enable response");
    assert_eq!(response["result"], json!({}));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|bc| bc.active_target.runtime_slot.loaded_page())
            .is_some_and(|page| page.final_url().as_str() == target_url),
        "Page.enable should keep using the target-lifecycle initial about:blank page"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn enable_uses_fresh_about_blank_without_adapter() {
    assert_page_enable_uses_fresh_initial_document_without_adapter("about:blank").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn enable_uses_fresh_about_blank_fragment_without_adapter() {
    assert_page_enable_uses_fresh_initial_document_without_adapter("about:blank#fragment").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn enable_succeeds_without_legacy_materialization_adapter_when_page_missing() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-PAGE-ENABLE-NO-DOCUMENT",
        "TID-PAGE-ENABLE-NO-DOCUMENT",
        "SID-PAGE-ENABLE-NO-DOCUMENT",
        "about:blank",
    );

    ctx.process_async(json!({
        "id": 121,
        "method": "Page.enable",
        "sessionId": "SID-PAGE-ENABLE-NO-DOCUMENT"
    }))
    .await;

    ctx.expect_result(121, json!({}), Some("SID-PAGE-ENABLE-NO-DOCUMENT"));
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target
            .runtime_slot
            .has_loaded_page(),
        "Page.enable should not install a loaded page when target lifecycle did not"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn document_open_exits_initial_empty_document_record() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 130,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .await;
    ctx.expect_event("Target.targetCreated", None);
    let create_response = take_response_by_id(&mut ctx, 130);
    assert!(
        create_response["result"]["targetId"].as_str().is_some(),
        "Target.createTarget should return target id: {create_response}"
    );

    ctx.process_async(json!({
        "id": 131,
        "method": "Page.enable"
    }))
    .await;
    ctx.expect_result(131, json!({}), None);
    let before = ctx
        .conn
        .target_owner_state_for_session(None)
        .and_then(|owner_state| owner_state.initial_empty_document_state())
        .expect("initial empty document record should survive materialization");
    assert!(before.materialized());
    assert!(before.is_on_initial_empty_document());

    ctx.process_async(json!({
        "id": 132,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "document.open(); 'done';",
            "returnByValue": true
        }
    }))
    .await;

    let after = ctx
        .conn
        .target_owner_state_for_session(None)
        .and_then(|owner_state| owner_state.initial_empty_document_state())
        .expect("initial empty document record should remain for diagnostics");
    assert!(after.exited());
    assert!(!after.is_on_initial_empty_document());
}

#[tokio::test(flavor = "multi_thread")]
async fn enable_non_blank_initial_url_loads_through_pending_navigation_path() {
    let mut ctx = TestContext::new();
    let page_url = "data:text/html,<h1>initial</h1>";
    load_bc_with_session(
        &mut ctx,
        "BID-PAGE-ENABLE-DATA",
        "TID-PAGE-ENABLE-DATA",
        "SID-PAGE-ENABLE-DATA",
        page_url,
    );

    let raw = json!({
        "id": 122,
        "method": "Page.enable",
        "sessionId": "SID-PAGE-ENABLE-DATA"
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("non-about:blank Page.enable should start initial URL navigation");
    let (messages, scheduler_events) =
        complete_pending_command_task_for_test(&mut ctx, pending).await;
    assert!(
        scheduler_events.iter().any(|event| matches!(
            event,
            CdpSchedulerEvent::ProtocolWorkPublished { work }
                if work.kind()
                    == crate::domains::activity::ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction
                    && work.main_document_load_session_id()
                        == Some("SID-PAGE-ENABLE-DATA")
        )),
        "Page.enable initial navigation should schedule load completion activity: {scheduler_events:?}"
    );
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(122))
        .expect("Page.enable response");
    assert_eq!(response["sessionId"], json!("SID-PAGE-ENABLE-DATA"));
    assert_eq!(response["result"], json!({}));
    assert!(
        messages.iter().any(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["sessionId"] == json!("SID-PAGE-ENABLE-DATA")
                && message["params"]["frame"]["url"] == json!(page_url)
        }),
        "Page.enable should emit navigation events for the initial URL: {messages:?}"
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|bc| bc.active_target.runtime_slot.loaded_page())
            .is_some_and(|page| page.final_url().as_str() == page_url),
        "Page.enable pending path should install the initial non-blank document"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn enable_accepts_enable_file_chooser_opened_event_param() {
    let mut ctx = TestContext::new();
    load_bc_with_target(&mut ctx, "BID-PAGE", "TID-PAGE", "about:blank");
    ctx.process_async(json!({
        "id": 101,
        "method": "Page.enable",
        "params": { "enableFileChooserOpenedEvent": true }
    }))
    .await;
    ctx.expect_result(101, json!({}), None);
    assert!(ctx.conn.browser_context.as_ref().is_some_and(|bc| {
        bc.devtools_session_state
            .page_session_state
            .page_file_chooser_opened_event_enabled
    }));
}
#[tokio::test(flavor = "multi_thread")]
async fn bring_to_front_returns_empty_result() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 102, "method": "Page.bringToFront"}))
        .await;
    ctx.expect_result(102, json!({}), None);
}
#[tokio::test(flavor = "multi_thread")]
async fn page_set_download_behavior_reuses_browser_download_state() {
    let mut ctx = TestContext::new();
    let mut browser_context = BrowserContext::new("BID-PAGE-DOWNLOAD".into());
    browser_context.set_active_target_id("TID-PAGE-DOWNLOAD");
    ctx.conn.browser_context = Some(browser_context);

    ctx.process_async(json!({
        "id": 103,
        "method": "Page.setDownloadBehavior",
        "params": {
            "behavior": "allow",
            "downloadPath": "/tmp/page-downloads",
            "eventsEnabled": true
        }
    }))
    .await;
    ctx.expect_result(103, json!({}), None);

    let settings = ctx
        .conn
        .download_behavior
        .effective_for_browser_context(Some("BID-PAGE-DOWNLOAD"));
    assert_eq!(settings.behavior, "allow");
    assert_eq!(
        settings.download_path.as_deref(),
        Some("/tmp/page-downloads")
    );
    assert!(
        !settings.automation_events_enabled,
        "Page.setDownloadBehavior delegates to BrowserHandler::DoSetDownloadBehavior and must not enable Browser download events"
    );
    assert_eq!(
        ctx.conn.download_behavior.browser_context_id.as_deref(),
        Some("BID-PAGE-DOWNLOAD")
    );
    assert_eq!(ctx.conn.download_behavior.behavior, "default");
    assert!(!ctx.conn.download_behavior.automation_events_enabled);
}

#[tokio::test(flavor = "multi_thread")]
async fn page_set_download_behavior_uses_current_page_context_not_param_context() {
    let mut ctx = TestContext::new();
    let mut active = BrowserContext::new("BID-PAGE-DOWNLOAD-ACTIVE".into());
    active.set_active_target_id("TID-PAGE-DOWNLOAD-ACTIVE");
    ctx.conn.browser_context = Some(active);
    let mut inactive = BrowserContext::new("BID-PAGE-DOWNLOAD-OTHER".into());
    inactive.set_active_target_id("TID-PAGE-DOWNLOAD-OTHER");
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 105,
        "method": "Page.setDownloadBehavior",
        "params": {
            "behavior": "allow",
            "downloadPath": "/tmp/page-downloads-active",
            "browserContextId": "BID-PAGE-DOWNLOAD-OTHER",
            "eventsEnabled": true
        }
    }))
    .await;
    ctx.expect_result(105, json!({}), None);

    let active_settings = ctx
        .conn
        .download_behavior
        .effective_for_browser_context(Some("BID-PAGE-DOWNLOAD-ACTIVE"));
    assert_eq!(active_settings.behavior, "allow");
    assert_eq!(
        active_settings.download_path.as_deref(),
        Some("/tmp/page-downloads-active")
    );
    assert!(!active_settings.automation_events_enabled);
    assert_eq!(
        ctx.conn
            .download_behavior
            .effective_for_browser_context(Some("BID-PAGE-DOWNLOAD-OTHER")),
        crate::conn::BrowserDownloadBehaviorSettings::default()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn page_set_download_behavior_rejects_invalid_params() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 104,
        "method": "Page.setDownloadBehavior",
        "params": {}
    }))
    .await;
    ctx.expect_error(104, -32602, "InvalidParams");
}

#[tokio::test(flavor = "multi_thread")]
async fn page_set_download_behavior_rejects_without_page_owner() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 106,
        "method": "Page.setDownloadBehavior",
        "params": {
            "behavior": "allow",
            "downloadPath": "/tmp/page-downloads"
        }
    }))
    .await;
    ctx.expect_error(106, -32000, "Could not fetch browser context");
}
/// cdp.page: setLifecycleEventsEnabled sets the flag
#[tokio::test(flavor = "multi_thread")]
async fn set_lifecycle_events_enabled_sets_flag() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    ctx.conn.browser_context = Some(bc);
    ctx.process_async(json!({"id": 1, "method": "Page.setLifecycleEventsEnabled",
                           "params": {"enabled": true}}))
        .await;
    ctx.expect_result(1, json!({}), None);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .devtools_session_state
            .page_session_state
            .page_lifecycle_events
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn set_lifecycle_events_enabled_replays_loaded_page_events() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let mut navigation = ctx
        .conn
        .load_navigation_via_runtime_async("data:text/html,<body>hello</body>")
        .await
        .expect("page should load");
    let navigation_engine = navigation.navigation_engine.take();
    let dom_timestamp = navigation
        .page_creation_artifacts
        .lifecycle_snapshot
        .dom_content_loaded
        .expect("loaded page should have renderer DCL")
        .timestamp_micros as f64
        / 1_000_000.0;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(navigation.page);
    let (binding, initial_events) = ctx.conn.bind_renderer_document_lifecycle_for_session_owner(
        Some("SID-1"),
        navigation.page_creation_artifacts,
        None,
        "TID-1".to_owned(),
        LOADER_ID.to_owned(),
    );
    assert!(binding.is_some());
    assert_eq!(initial_events.len(), 2);
    if let Some(navigation_engine) = navigation_engine {
        ctx.conn
            .adopt_loaded_navigation_engine_for_session_owner(Some("SID-1"), navigation_engine);
    }
    let load_timestamp = wait_for_visible_renderer_load(&mut ctx, "SID-1")
        .await
        .timestamp_micros as f64
        / 1_000_000.0;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 2,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-1",
        "params": { "enabled": true }
    }))
    .await;

    let dom = ctx.take_one();
    assert_eq!(dom["method"], "Page.lifecycleEvent");
    assert_eq!(dom["sessionId"], "SID-1");
    assert_eq!(dom["params"]["name"], "DOMContentLoaded");
    assert_eq!(dom["params"]["frameId"], "TID-1");
    assert_eq!(dom["params"]["loaderId"], LOADER_ID);
    assert_eq!(dom["params"]["timestamp"], json!(dom_timestamp));

    let load = ctx.take_one();
    assert_eq!(load["method"], "Page.lifecycleEvent");
    assert_eq!(load["sessionId"], "SID-1");
    assert_eq!(load["params"]["name"], "load");
    assert_eq!(load["params"]["frameId"], "TID-1");
    assert_eq!(load["params"]["loaderId"], LOADER_ID);
    assert_eq!(load["params"]["timestamp"], json!(load_timestamp));

    let result = ctx.take_one();
    assert_eq!(result["id"], 2);
    assert_eq!(result["sessionId"], "SID-1");
    assert_eq!(result["result"], json!({}));
    assert!(ctx.sent.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn set_lifecycle_events_enabled_replays_only_protocol_visible_load_state() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-visible",
        "TID-visible",
        "SID-visible",
        "about:blank",
    );
    let mut navigation = ctx
        .conn
        .load_navigation_via_runtime_async("data:text/html,<body>visible lifecycle</body>")
        .await
        .expect("page should load");
    let navigation_engine = navigation.navigation_engine.take();
    let artifacts = navigation.page_creation_artifacts;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(navigation.page);
    let (binding, initial_events) = ctx.conn.bind_renderer_document_lifecycle_for_session_owner(
        Some("SID-visible"),
        artifacts,
        None,
        "TID-visible".to_owned(),
        LOADER_ID.to_owned(),
    );
    assert!(binding.is_some());
    assert_eq!(initial_events.len(), 2);
    if let Some(navigation_engine) = navigation_engine {
        ctx.conn.adopt_loaded_navigation_engine_for_session_owner(
            Some("SID-visible"),
            navigation_engine,
        );
    }
    assert!(
        ctx.conn
            .begin_renderer_document_load_visibility_barrier_for_session_owner(
                Some("SID-visible"),
                LOADER_ID,
            )
    );
    let load_stamp = wait_for_authoritative_renderer_load(&mut ctx, "SID-visible").await;
    assert!(
        ctx.conn
            .renderer_document_lifecycle_authoritative_state_for_session_owner(Some("SID-visible"))
            .is_some_and(|(_, snapshot)| snapshot.load.is_some()),
        "authoritative lifecycle state should reach load while delivery is gated"
    );
    assert!(
        ctx.conn
            .renderer_document_lifecycle_visible_state_for_session_owner(Some("SID-visible"))
            .is_some_and(|(_, snapshot)| snapshot.load.is_none()),
        "protocol-visible lifecycle state must not cross the load barrier"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 3,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-visible",
        "params": { "enabled": true }
    }))
    .await;
    let dcl = ctx.take_one();
    assert_eq!(dcl["method"], "Page.lifecycleEvent");
    assert_eq!(dcl["params"]["name"], "DOMContentLoaded");
    ctx.expect_result(3, json!({}), Some("SID-visible"));
    assert!(ctx.sent.is_empty(), "hidden load must not be replayed");

    ctx.process_async(json!({
        "id": 4,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-visible",
        "params": { "enabled": false }
    }))
    .await;
    ctx.expect_result(4, json!({}), Some("SID-visible"));
    let load_event = take_released_renderer_load_event(&mut ctx, "SID-visible", load_stamp);

    ctx.process_async(json!({
        "id": 5,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-visible",
        "params": { "enabled": true }
    }))
    .await;
    let dcl = ctx.take_one();
    assert_eq!(dcl["params"]["name"], "DOMContentLoaded");
    let load = ctx.take_one();
    assert_eq!(load["params"]["name"], "load");
    assert_eq!(
        load["params"]["timestamp"],
        json!(load_event.timestamp_micros as f64 / 1_000_000.0)
    );
    ctx.expect_result(5, json!({}), Some("SID-visible"));
    assert!(ctx.sent.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn set_lifecycle_events_enabled_is_session_local_for_active_auxiliary_session() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-aux-lifecycle",
        "TID-active",
        "SID-primary",
        "about:blank",
    );
    let mut navigation = ctx
        .conn
        .load_navigation_via_runtime_async("data:text/html,<body>active auxiliary lifecycle</body>")
        .await
        .expect("page should load");
    let navigation_engine = navigation.navigation_engine.take();
    let artifacts = navigation.page_creation_artifacts;
    let browser_context = ctx.conn.browser_context.as_mut().unwrap();
    browser_context
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(navigation.page);
    assert!(browser_context.assign_auxiliary_session_to_target("TID-active", "SID-aux".to_owned()));
    let (binding, initial_events) = ctx.conn.bind_renderer_document_lifecycle_for_session_owner(
        Some("SID-aux"),
        artifacts,
        None,
        "TID-active".to_owned(),
        LOADER_ID.to_owned(),
    );
    assert!(binding.is_some());
    assert_eq!(initial_events.len(), 2);
    if let Some(navigation_engine) = navigation_engine {
        ctx.conn
            .adopt_loaded_navigation_engine_for_session_owner(Some("SID-aux"), navigation_engine);
    }
    let _ = wait_for_visible_renderer_load(&mut ctx, "SID-aux").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 20,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-aux",
        "params": { "enabled": true }
    }))
    .await;

    let lifecycle_events = ctx
        .sent
        .iter()
        .filter(|message| message["method"] == json!("Page.lifecycleEvent"))
        .collect::<Vec<_>>();
    assert_eq!(lifecycle_events.len(), 2);
    assert!(lifecycle_events.iter().all(|message| {
        message["sessionId"] == json!("SID-aux")
            && message["params"]["frameId"] == json!("TID-active")
    }));
    assert_eq!(
        lifecycle_events
            .iter()
            .map(|message| message["params"]["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["DOMContentLoaded", "load"]
    );
    ctx.expect_result(20, json!({}), Some("SID-aux"));

    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    assert!(
        !browser_context
            .devtools_session_state
            .page_session_state
            .page_lifecycle_events,
        "primary page session should stay disabled"
    );
    assert!(
        ctx.conn
            .target_page_session_state_for_session(Some("SID-aux"))
            .expect("auxiliary page session state")
            .page_lifecycle_events,
        "auxiliary page session should own lifecycle enable"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_lifecycle_events_enabled_is_session_local_for_background_auxiliary_session() {
    let mut ctx = TestContext::new();
    let background = BackgroundTarget::with_url(
        "TID-background".to_owned(),
        Some("SID-background-primary".to_owned()),
        "about:blank".to_owned(),
    );

    let mut browser_context = BrowserContext::new("BID-background-aux-lifecycle".to_owned());
    browser_context.set_active_target_id("TID-active".to_owned());
    browser_context.attach_active_session("SID-active".to_owned());
    browser_context.background_targets.push(background);
    assert!(
        browser_context
            .assign_auxiliary_session_to_target("TID-background", "SID-background-aux".to_owned())
    );
    ctx.conn.browser_context = Some(browser_context);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>background auxiliary lifecycle</body>",
        Some("SID-background-aux"),
    )
    .await;
    let _ = wait_for_visible_renderer_load(&mut ctx, "SID-background-aux").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 21,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-background-aux",
        "params": { "enabled": true }
    }))
    .await;

    let lifecycle_events = ctx
        .sent
        .iter()
        .filter(|message| message["method"] == json!("Page.lifecycleEvent"))
        .collect::<Vec<_>>();
    assert_eq!(lifecycle_events.len(), 2);
    assert!(lifecycle_events.iter().all(|message| {
        message["sessionId"] == json!("SID-background-aux")
            && message["params"]["frameId"] == json!("TID-background")
    }));
    assert_eq!(
        lifecycle_events
            .iter()
            .map(|message| message["params"]["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["DOMContentLoaded", "load"]
    );
    ctx.expect_result(21, json!({}), Some("SID-background-aux"));

    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    let parked = browser_context
        .parked_page_session_state("TID-background")
        .expect("background target should retain parked page session state");
    assert!(
        !parked
            .devtools_session_state
            .page_session_state
            .page_lifecycle_events,
        "background primary page session should stay disabled"
    );
    assert!(
        ctx.conn
            .target_page_session_state_for_session(Some("SID-background-aux"))
            .expect("background auxiliary page session state")
            .page_lifecycle_events,
        "background auxiliary page session should own lifecycle enable"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn page_disable_clears_page_handler_state_for_active_auxiliary_session() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-aux-page-disable",
        "TID-active",
        "SID-primary",
        "about:blank",
    );
    let browser_context = ctx.conn.browser_context.as_mut().unwrap();
    mark_page_domain_enabled(&mut browser_context.devtools_session_state.page_session_state);
    assert!(browser_context.assign_auxiliary_session_to_target("TID-active", "SID-aux".to_owned()));

    enable_page_domain_for_session(&mut ctx.conn, "SID-aux");
    assert_page_domain_enabled(
        ctx.conn
            .target_page_session_state_for_session(Some("SID-aux"))
            .expect("active auxiliary page session state"),
    );

    ctx.process_async(json!({
        "id": 22,
        "method": "Page.disable",
        "sessionId": "SID-aux"
    }))
    .await;
    ctx.expect_result(22, json!({}), Some("SID-aux"));

    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    assert_page_domain_enabled(&browser_context.devtools_session_state.page_session_state);
    assert_page_domain_disabled(
        ctx.conn
            .target_page_session_state_for_session(Some("SID-aux"))
            .expect("active auxiliary page session state"),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn page_disable_clears_page_handler_state_for_background_auxiliary_session() {
    let mut ctx = TestContext::new();
    let background = BackgroundTarget::with_url(
        "TID-background".to_owned(),
        Some("SID-background-primary".to_owned()),
        "about:blank#background".to_owned(),
    );
    let mut browser_context = BrowserContext::new("BID-background-page-disable".to_owned());
    browser_context.set_active_target_id("TID-active".to_owned());
    browser_context.attach_active_session("SID-active".to_owned());
    browser_context.background_targets.push(background);
    assert!(
        browser_context
            .assign_auxiliary_session_to_target("TID-background", "SID-background-aux".to_owned())
    );
    browser_context.mutate_parked_page_session_state("TID-background", |state| {
        mark_page_domain_enabled(&mut state.devtools_session_state.page_session_state);
    });
    ctx.conn.browser_context = Some(browser_context);

    enable_page_domain_for_session(&mut ctx.conn, "SID-background-aux");
    assert_page_domain_enabled(
        ctx.conn
            .target_page_session_state_for_session(Some("SID-background-aux"))
            .expect("background auxiliary page session state"),
    );

    ctx.process_async(json!({
        "id": 23,
        "method": "Page.disable",
        "sessionId": "SID-background-aux"
    }))
    .await;
    ctx.expect_result(23, json!({}), Some("SID-background-aux"));

    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    let parked = browser_context
        .parked_page_session_state("TID-background")
        .expect("background target should retain parked page session state");
    assert_page_domain_enabled(&parked.devtools_session_state.page_session_state);
    assert_page_domain_disabled(
        ctx.conn
            .target_page_session_state_for_session(Some("SID-background-aux"))
            .expect("background auxiliary page session state"),
    );
}

// This integration keeps a foreground V8 owner and detached module
// continuation live at the same time. Two Tokio workers are sufficient for
// that contract; using Tokio's host-CPU default here creates one 32-thread
// runtime per nextest process and can starve the continuation under the full
// workspace matrix.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_document_close_emits_lifecycle_for_repeated_playwright_set_content_flow() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>initial</body>",
        Some("SID-1"),
    )
    .await;
    // The fixture returns once the initial Document is committed, while its
    // concrete lifecycle publication may still be queued. Settle that exact
    // generation before observing document.open(); otherwise a delayed
    // about:blank load can be mistaken for the replacement Document's load.
    wait_until_renderer_document_load(&mut ctx, Some("SID-1"), "TID-1", LOADER_ID).await;
    ctx.enable_dom_events_for_test(Some("SID-1"));

    ctx.process_async(json!({
        "id": 10,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-1",
        "params": { "enabled": true }
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 11,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 12,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "__playwright_utility_world__"
        }
    }))
    .await;
    let isolated_context_id = take_response_by_id(&mut ctx, 12)["result"]["executionContextId"]
        .as_i64()
        .expect("isolated context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 120,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "window.__pwMainBeforeWrite = 'main-before-write'",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 120)["result"]["result"]["value"],
        json!("main-before-write")
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 13,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": isolated_context_id,
            "expression": "window.__pwIsolatedOnly = 'isolated-only'; document.open(); console.debug('--playwright--set--content--test--'); document.write('<main id=\"pw-set-content\">ok</main><script id=\"pw-set-content-inline\">window.__pwSetContentInlineRan = (window.__pwSetContentInlineRan || 0) + 1; window.__pwSetContentWorldResult = [window.__pwMainBeforeWrite, typeof window.__pwIsolatedOnly, document.currentScript && document.currentScript.id].join(\"|\"); console.log(\"set-content-inline-script-ran\");<\\/script><script id=\"pw-set-content-module\" type=\"module\">window.__pwSetContentModuleRan = (window.__pwSetContentModuleRan || 0) + 1; window.__pwSetContentModuleWorldResult = [window.__pwMainBeforeWrite, typeof window.__pwIsolatedOnly, document.currentScript === null ? \"null\" : document.currentScript.id].join(\"|\"); console.log(\"set-content-module-script-ran\");<\\/script>'); document.close(); 'done';",
            "returnByValue": true
        }
    }))
    .await;
    wait_until_scheduler_message(
        &mut ctx,
        "renderer document.close replacement DOM update",
        |message| message["method"] == json!("DOM.documentUpdated"),
    )
    .await;
    wait_until_scheduler_message(
        &mut ctx,
        "renderer document.close replacement load lifecycle",
        |message| message["method"] == json!("Page.loadEventFired"),
    )
    .await;

    let sent = ctx.take_all();
    let response_index = sent
        .iter()
        .position(|message| {
            message["id"] == json!(13) && message["result"]["result"]["value"] == json!("done")
        })
        .unwrap_or_else(|| panic!("Runtime.evaluate should complete successfully: {sent:?}"));
    let console_tag_index = sent
        .iter()
        .position(|message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["executionContextId"] == json!(isolated_context_id)
                && message["params"]["type"] == json!("debug")
                && message["params"]["args"][0]["value"]
                    == json!("--playwright--set--content--test--")
        })
        .unwrap_or_else(|| panic!("Playwright setContent should emit its console tag: {sent:?}"));
    let lifecycle_init_index = sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("init")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["loaderId"] == json!(LOADER_ID)
        })
        .unwrap_or_else(|| panic!("document.open should restart the renderer lifecycle: {sent:?}"));
    let inline_script_console_index = sent
        .iter()
        .position(|message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["type"] == json!("log")
                && message["params"]["args"][0]["value"] == json!("set-content-inline-script-ran")
        })
        .unwrap_or_else(|| {
            panic!("document.write inline script should run during setContent flush: {sent:?}")
        });
    assert_ne!(
        sent[inline_script_console_index]["params"]["executionContextId"],
        json!(isolated_context_id),
        "parser-created script console output must come from the document main world: {sent:?}"
    );
    let module_script_console_index = sent
        .iter()
        .position(|message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["type"] == json!("log")
                && message["params"]["args"][0]["value"] == json!("set-content-module-script-ran")
        })
        .unwrap_or_else(|| {
            panic!("document.write module script should run before lifecycle completion: {sent:?}")
        });
    assert_ne!(
        sent[module_script_console_index]["params"]["executionContextId"],
        json!(isolated_context_id),
        "parser-created module console output must come from the document main world: {sent:?}"
    );
    let document_updated_index = sent
        .iter()
        .position(|message| message["method"] == json!("DOM.documentUpdated"))
        .unwrap_or_else(|| {
            panic!("document replacement should update DOM domain observers: {sent:?}")
        });
    let dom_content_event_index = sent
        .iter()
        .position(|message| message["method"] == json!("Page.domContentEventFired"))
        .unwrap_or_else(|| {
            panic!("document.close should emit DOMContentLoaded timestamp: {sent:?}")
        });
    assert!(
        inline_script_console_index < dom_content_event_index,
        "document.write inline scripts should run before setContent DOMContentLoaded: {sent:?}"
    );
    assert!(
        module_script_console_index < dom_content_event_index,
        "document.write module scripts should run before setContent DOMContentLoaded: {sent:?}"
    );
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("DOMContentLoaded")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["loaderId"] == json!(LOADER_ID)
        }),
        "document.close should emit DOMContentLoaded lifecycle: {sent:?}"
    );
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("load")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["loaderId"] == json!(LOADER_ID)
        }),
        "document.close should emit load lifecycle: {sent:?}"
    );
    let load_event_index = sent
        .iter()
        .position(|message| message["method"] == json!("Page.loadEventFired"))
        .unwrap_or_else(|| panic!("document.close should emit load timestamp: {sent:?}"));
    assert!(
        lifecycle_init_index < console_tag_index,
        "document.open lifecycle init should precede subsequent script output: {sent:?}"
    );
    assert!(
        console_tag_index < inline_script_console_index
            && inline_script_console_index < response_index,
        "utility-world output and synchronous parser-blocking classic script output should precede the Runtime.evaluate response: {sent:?}"
    );
    assert!(
        response_index < module_script_console_index,
        "parser-created module execution is a later task than the Runtime.evaluate response: {sent:?}"
    );
    assert!(
        module_script_console_index < document_updated_index
            && document_updated_index < dom_content_event_index
            && dom_content_event_index < load_event_index,
        "module execution and DOM observer update must precede DOMContentLoaded and load: {sent:?}"
    );
    ctx.process_async(json!({
        "id": 14,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "window.__pwSetContentInlineRan",
            "returnByValue": true
        }
    }))
    .await;
    let inline_state = take_response_by_id(&mut ctx, 14);
    assert_eq!(inline_state["result"]["result"]["value"], json!(1));

    ctx.process_async(json!({
        "id": 140,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "window.__pwSetContentWorldResult",
            "returnByValue": true
        }
    }))
    .await;
    let world_state = take_response_by_id(&mut ctx, 140);
    assert_eq!(
        world_state["result"]["result"]["value"],
        json!("main-before-write|undefined|pw-set-content-inline")
    );

    ctx.process_async(json!({
        "id": 142,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "[window.__pwSetContentModuleRan, window.__pwSetContentModuleWorldResult].join('::')",
            "returnByValue": true
        }
    }))
    .await;
    let module_world_state = take_response_by_id(&mut ctx, 142);
    assert_eq!(
        module_world_state["result"]["result"]["value"],
        json!("1::main-before-write|undefined|null")
    );

    ctx.process_async(json!({
        "id": 141,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": isolated_context_id,
            "expression": "[typeof window.__pwSetContentInlineRan, typeof window.__pwSetContentModuleRan, window.__pwIsolatedOnly].join('|')",
            "returnByValue": true
        }
    }))
    .await;
    let isolated_world_state = take_response_by_id(&mut ctx, 141);
    assert_eq!(
        isolated_world_state["result"]["result"]["value"],
        json!("undefined|undefined|isolated-only")
    );

    ctx.process_async(json!({
        "id": 15,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.readyState",
            "returnByValue": true
        }
    }))
    .await;
    let ready_state = take_response_by_id(&mut ctx, 15);
    assert_eq!(ready_state["result"]["result"]["value"], json!("complete"));

    ctx.process_async(json!({
        "id": 16,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "new Promise(resolve => requestAnimationFrame(() => resolve({ source: 'raf-after-document-close', readyState: document.readyState })))",
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    wait_until_message(
        &mut ctx,
        "SID-1",
        "requestAnimationFrame response after document.close",
        |message| message["id"] == json!(16),
    )
    .await;
    let raf = take_response_by_id(&mut ctx, 16);
    assert_eq!(
        raf["result"]["result"]["value"],
        json!({
            "source": "raf-after-document-close",
            "readyState": "complete"
        })
    );

    for iteration in 0..32 {
        let previous_document = ctx
            .conn
            .target_root_document_lifecycle_identity_for_session(Some("SID-1"))
            .expect("the previous replacement must retain an exact Document identity");
        let id = 100 + iteration;
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "contextId": isolated_context_id,
                "expression": format!(
                    "document.open(); document.write('<main data-iteration=\"{iteration}\">replacement</main>'); document.close(); 'replacement-{iteration}';"
                ),
                "returnByValue": true
            }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        assert_eq!(
            response["result"]["result"]["value"],
            json!(format!("replacement-{iteration}")),
            "repeated setContent-style replacement {iteration} should retain its own Runtime lifecycle continuation"
        );
        ctx.wait_until_scheduler_state(
            &format!("replacement {iteration} exact Document load"),
            |conn| {
                conn.renderer_document_lifecycle_authoritative_state_for_session_owner(Some(
                    "SID-1",
                ))
                .is_some_and(|(binding, snapshot)| {
                    binding.renderer_document_identity() != previous_document
                        && snapshot.load.is_some()
                })
            },
        )
        .await;

        let ready_state_id = 200 + iteration;
        ctx.process_async(json!({
            "id": ready_state_id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "document.readyState",
                "returnByValue": true
            }
        }))
        .await;
        let ready_state = take_response_by_id(&mut ctx, ready_state_id);
        assert_eq!(
            ready_state["result"]["result"]["value"],
            json!("complete"),
            "replacement {iteration} should reach Load before the next Runtime command"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_document_close_releases_lifecycle_at_response_flush() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>initial</body>",
        Some("SID-1"),
    )
    .await;
    ctx.wait_until_scheduler_state("initial document load before held response flush", |conn| {
        conn.renderer_document_lifecycle_authoritative_state_for_session_owner(Some("SID-1"))
            .is_some_and(|(_, snapshot)| snapshot.load.is_some())
    })
    .await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned())
    );

    ctx.process_async(json!({
        "id": 30,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-1",
        "params": { "enabled": true }
    }))
    .await;
    ctx.process_async(json!({
        "id": 31,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-aux",
        "params": { "enabled": true }
    }))
    .await;
    ctx.process_async(json!({
        "id": 32,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.process_async(json!({
        "id": 33,
        "method": "Runtime.enable",
        "sessionId": "SID-aux"
    }))
    .await;
    ctx.sent.clear();

    let response_flush = ctx
        .process_command_holding_response_flush_for_test(json!({
            "id": 34,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": r#"document.open(); document.write('<main id="runtime-boundary">replacement</main><script type="module">console.log("runtime-module-after-response")<\/script>'); document.close(); 'done'"#,
                "returnByValue": true
            }
        }))
        .await;

    let immediate = ctx.take_all();
    let init_messages = immediate
        .iter()
        .enumerate()
        .filter(|(_, message)| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("init")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        init_messages.len(),
        2,
        "both attached Page sessions should receive the synchronous init: {immediate:?}"
    );
    assert!(
        init_messages
            .iter()
            .any(|(_, message)| message["sessionId"] == json!("SID-1"))
    );
    assert!(
        init_messages
            .iter()
            .any(|(_, message)| message["sessionId"] == json!("SID-aux"))
    );
    let response_index = immediate
        .iter()
        .position(|message| {
            message["id"] == json!(34) && message["result"]["result"]["value"] == json!("done")
        })
        .unwrap_or_else(|| panic!("Runtime.evaluate response should be present: {immediate:?}"));
    assert!(
        init_messages
            .iter()
            .all(|(init_index, _)| *init_index < response_index),
        "synchronous document.open output must precede the Runtime response: {immediate:?}"
    );
    assert_eq!(
        immediate
            .iter()
            .filter(|message| message["id"] == json!(34))
            .count(),
        1,
        "the command response must remain local to its initiating session: {immediate:?}"
    );
    assert_eq!(immediate[response_index]["sessionId"], json!("SID-1"));
    assert!(
        immediate.iter().all(|message| {
            message["method"] != json!("Page.domContentEventFired")
                && message["method"] != json!("Page.loadEventFired")
                && !(message["method"] == json!("Runtime.consoleAPICalled")
                    && message["params"]["args"][0]["value"]
                        == json!("runtime-module-after-response"))
        }),
        "module and lifecycle continuation must remain parked while response flush is held: {immediate:?}"
    );

    ctx.finish_held_command_response_flush_for_test(response_flush)
        .await;
    wait_until_message(
        &mut ctx,
        "SID-1",
        "runtime document.close load after response flush",
        |message| message["method"] == json!("Page.loadEventFired"),
    )
    .await;
    let after_flush = ctx.take_all();
    let module_sessions = after_flush
        .iter()
        .filter(|message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["args"][0]["value"] == json!("runtime-module-after-response")
        })
        .map(|message| message["sessionId"].clone())
        .collect::<Vec<_>>();
    assert!(module_sessions.contains(&json!("SID-1")));
    assert!(module_sessions.contains(&json!("SID-aux")));
}

/// Chromium starts a parser-owned module before DOMContentLoaded, but a
/// top-level await continuation does not keep DOMContentLoaded/load blocked.
#[tokio::test(flavor = "multi_thread")]
async fn runtime_document_write_module_top_level_await_does_not_block_lifecycle() {
    let mut ctx = TestContext::new();
    let isolated_context_id = install_runtime_document_replacement_test_page(&mut ctx).await;

    ctx.process_async(json!({
        "id": 93,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "window.__tlaTrace = []",
            "returnByValue": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 93);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 94,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": isolated_context_id,
            "expression": r#"document.open(); document.write('<script type="module">window.__tlaTrace.push("module-start"); console.log("tla-module-start"); await new Promise(resolve => { window.__releaseTla = resolve; }); window.__tlaTrace.push("module-end"); console.log("tla-module-end");<\/script>'); document.close(); 'done'"#,
            "returnByValue": true
        }
    }))
    .await;
    // A load event from the preceding Document can still share this target's
    // frame/loader identifiers. The module's concrete console record proves
    // that the replacement parser-owned module has started before this test
    // accepts its later lifecycle output.
    wait_until_message(
        &mut ctx,
        "SID-1",
        "document.write TLA module start",
        |message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["args"][0]["value"] == json!("tla-module-start")
        },
    )
    .await;
    wait_until_message(
        &mut ctx,
        "SID-1",
        "document.write TLA module load lifecycle",
        |message| message["method"] == json!("Page.loadEventFired"),
    )
    .await;

    let messages = ctx.take_all();
    let response_index = messages
        .iter()
        .position(|message| {
            message["id"] == json!(94) && message["result"]["result"]["value"] == json!("done")
        })
        .unwrap_or_else(|| panic!("missing Runtime.evaluate response: {messages:?}"));
    let module_start_index = console_message_index(&messages, "tla-module-start");
    let document_updated_index = messages
        .iter()
        .position(|message| message["method"] == json!("DOM.documentUpdated"))
        .unwrap_or_else(|| panic!("missing DOM.documentUpdated: {messages:?}"));
    let dom_content_loaded_index = messages
        .iter()
        .position(|message| message["method"] == json!("Page.domContentEventFired"))
        .unwrap_or_else(|| panic!("missing DOMContentLoaded: {messages:?}"));
    let load_index = messages
        .iter()
        .position(|message| message["method"] == json!("Page.loadEventFired"))
        .unwrap_or_else(|| panic!("missing load: {messages:?}"));
    assert!(
        response_index < module_start_index
            && module_start_index < document_updated_index
            && document_updated_index < dom_content_loaded_index
            && dom_content_loaded_index < load_index,
        "TLA module start should follow the command response and precede lifecycle completion: {messages:?}"
    );
    assert_ne!(
        messages[module_start_index]["params"]["executionContextId"],
        json!(isolated_context_id),
        "parser-created TLA module should start in the document main world"
    );
    assert!(
        messages.iter().all(|message| {
            message["method"] != json!("Runtime.consoleAPICalled")
                || message["params"]["args"][0]["value"] != json!("tla-module-end")
        }),
        "top-level await continuation should still be suspended at load: {messages:?}"
    );

    ctx.process_async(json!({
        "id": 95,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "[document.readyState, window.__tlaTrace.join(','), typeof window.__releaseTla].join('|')",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 95)["result"]["result"]["value"],
        json!("complete|module-start|function")
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 96,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "window.__releaseTla(); 'released'",
            "returnByValue": true
        }
    }))
    .await;
    wait_until_message(&mut ctx, "SID-1", "TLA module continuation", |message| {
        message["method"] == json!("Runtime.consoleAPICalled")
            && message["params"]["args"][0]["value"] == json!("tla-module-end")
    })
    .await;
    let resumed = ctx.take_all();
    let module_end_index = console_message_index(&resumed, "tla-module-end");
    assert_ne!(
        resumed[module_end_index]["params"]["executionContextId"],
        json!(isolated_context_id),
        "TLA continuation should resume in the document main world"
    );

    ctx.process_async(json!({
        "id": 97,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "window.__tlaTrace.join(',')",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 97)["result"]["result"]["value"],
        json!("module-start,module-end")
    );
}

/// Ports Chromium/WPT's multiple document.write script ordering contract:
/// classic scripts run synchronously, modules retain tree order in later
/// tasks, and every module still precedes DOMContentLoaded.
#[tokio::test(flavor = "multi_thread")]
async fn runtime_document_write_preserves_multiple_classic_and_module_script_order() {
    let mut ctx = TestContext::new();
    let isolated_context_id = install_runtime_document_replacement_test_page(&mut ctx).await;

    ctx.process_async(json!({
        "id": 100,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "window.__scriptOrder = []",
            "returnByValue": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 100);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 101,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": isolated_context_id,
            "expression": r#"document.open(); document.write('<script>window.__scriptOrder.push("classic-1"); console.log("ordered-classic-1");<\/script><script type="module">window.__scriptOrder.push("module-1"); console.log("ordered-module-1");<\/script><script>window.__scriptOrder.push("classic-2"); console.log("ordered-classic-2");<\/script><script type="module">window.__scriptOrder.push("module-2"); console.log("ordered-module-2");<\/script>'); document.close(); 'done'"#,
            "returnByValue": true
        }
    }))
    .await;
    // `Page.loadEventFired` left by the initial Document may cross the test
    // transport after `sent.clear()`. The evaluate response only fences output
    // produced by that command turn; parser-scheduled module tasks complete
    // later. Synchronize with concrete output from this replacement Document
    // instead of treating any load notification as its completion.
    wait_until_messages(
        &mut ctx,
        "SID-1",
        "multiple document.write module scripts and DOMContentLoaded",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Runtime.consoleAPICalled")
                    && message["params"]["args"][0]["value"] == json!("ordered-module-2")
            }) && messages
                .iter()
                .any(|message| message["method"] == json!("Page.domContentEventFired"))
        },
    )
    .await;

    let messages = ctx.take_all();
    let response_index = messages
        .iter()
        .position(|message| {
            message["id"] == json!(101) && message["result"]["result"]["value"] == json!("done")
        })
        .unwrap_or_else(|| panic!("missing Runtime.evaluate response: {messages:?}"));
    let classic_1 = console_message_index(&messages, "ordered-classic-1");
    let classic_2 = console_message_index(&messages, "ordered-classic-2");
    let module_1 = console_message_index(&messages, "ordered-module-1");
    let module_2 = console_message_index(&messages, "ordered-module-2");
    let document_updated = messages
        .iter()
        .position(|message| message["method"] == json!("DOM.documentUpdated"))
        .unwrap_or_else(|| panic!("missing DOM.documentUpdated: {messages:?}"));
    let dom_content_loaded = messages
        .iter()
        .position(|message| message["method"] == json!("Page.domContentEventFired"))
        .unwrap_or_else(|| panic!("missing DOMContentLoaded: {messages:?}"));
    assert!(
        classic_1 < classic_2
            && classic_2 < response_index
            && response_index < module_1
            && module_1 < module_2
            && module_2 < document_updated
            && document_updated < dom_content_loaded,
        "classic/module execution should preserve Chromium task and tree order: {messages:?}"
    );
    for index in [classic_1, classic_2, module_1, module_2] {
        assert_ne!(
            messages[index]["params"]["executionContextId"],
            json!(isolated_context_id),
            "parser-created scripts should execute in the document main world: {messages:?}"
        );
    }

    ctx.process_async(json!({
        "id": 102,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "window.__scriptOrder.join(',')",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 102)["result"]["result"]["value"],
        json!("classic-1,classic-2,module-1,module-2")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn add_script_run_immediately_creates_top_level_world_even_when_child_world_name_matches() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async(
            // Keep the child on its initial empty document. A `srcdoc`
            // navigation may commit after the preload is registered, in
            // which case the new-document script correctly runs in that
            // future child document and no longer isolates runImmediately's
            // top-level-world behavior.
            "data:text/html,<body>parent-frame<iframe></iframe></body>",
        )
        .await
        .expect("page should load");
    let bc = ctx.conn.browser_context.as_mut().expect("browser context");
    let _ = bc
        .active_target
        .runtime_slot
        .replace_loaded_page(Some(page));
    ctx.process_async(json!({
        "id": 4120,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(4120, json!({}), Some("SID-1"));
    ctx.sent.clear();

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 4121).await;
    ctx.process_async(json!({
        "id": 4122,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": child_frame_id,
            "worldName": "utility-shared"
        }
    }))
    .await;
    let child_context_id = take_response_by_id(&mut ctx, 4122)["result"]["executionContextId"]
        .as_i64()
        .expect("child isolated execution context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4123,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_same_name_world = document.body.textContent.includes('parent-frame') ? 'top-level' : 'bad';",
            "worldName": "utility-shared",
            "runImmediately": true
        }
    })).await;
    let created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-shared")
                && message["params"]["context"]["auxData"]["frameId"] == json!("TID-1")
        })
        .cloned()
        .expect("runImmediately should create a top-level world even if a child world shares the same name");
    let top_level_context_id = created["params"]["context"]["id"]
        .as_i64()
        .expect("top-level isolated execution context id");
    assert_ne!(top_level_context_id, child_context_id);
    ctx.expect_result(4123, json!({ "identifier": "1" }), Some("SID-1"));

    ctx.process_async(json!({
        "id": 4124,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": top_level_context_id,
            "expression": "globalThis.__lm_same_name_world"
        }
    }))
    .await;
    let top_level_result = take_response_by_id(&mut ctx, 4124);
    assert_eq!(
        top_level_result["result"]["result"]["value"],
        json!("top-level")
    );

    ctx.process_async(json!({
        "id": 4125,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_context_id,
            "expression": "typeof globalThis.__lm_same_name_world"
        }
    }))
    .await;
    let child_result = take_response_by_id(&mut ctx, 4125);
    assert_eq!(
        child_result["result"]["result"]["value"],
        json!("undefined")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn add_script_run_immediately_installs_matching_bindings_into_new_top_level_world() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>parent-frame<iframe srcdoc=\"<body>child-frame</body>\"></iframe></body>",
        Some("SID-1"),
    )
    .await;
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 4160,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(4160, json!({}), Some("SID-1"));
    ctx.sent.clear();

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 4161).await;
    ctx.process_async(json!({
        "id": 4162,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": child_frame_id,
            "worldName": "utility-shared-binding"
        }
    }))
    .await;
    let child_context_id = take_response_by_id(&mut ctx, 4162)["result"]["executionContextId"]
        .as_i64()
        .expect("child isolated execution context id");

    ctx.process_async(json!({
        "id": 4163,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "sharedUtilityBinding",
            "executionContextName": "utility-shared-binding"
        }
    }))
    .await;
    let binding_result = take_response_by_id(&mut ctx, 4163);
    assert_eq!(binding_result["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4164,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.sharedUtilityBinding('top-level-binding');",
            "worldName": "utility-shared-binding",
            "runImmediately": true
        }
    }))
    .await;
    let created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-shared-binding")
                && message["params"]["context"]["auxData"]["frameId"] == json!("TID-1")
        })
        .cloned()
        .expect("runImmediately should create a top-level world for matching binding replay");
    let top_level_context_id = created["params"]["context"]["id"]
        .as_i64()
        .expect("top-level isolated execution context id");
    assert_ne!(top_level_context_id, child_context_id);
    ctx.expect_result(4164, json!({ "identifier": "1" }), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4165,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": top_level_context_id,
            "expression": "globalThis.sharedUtilityBinding('top-level-binding'); 'done';"
        }
    }))
    .await;
    let eval_result = take_response_by_id(&mut ctx, 4165);
    assert_eq!(eval_result["result"]["result"]["value"], json!("done"));
    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("sharedUtilityBinding")
        })
        .cloned()
        .expect("matching binding should be installed into the new top-level world");
    assert_eq!(
        binding_called["params"]["payload"],
        json!("top-level-binding")
    );
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(top_level_context_id)
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn document_start_script_run_immediately_targets_loaded_background_owner_without_promotion() {
    let mut ctx = TestContext::new();
    let background = BackgroundTarget::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );

    let mut bc = BrowserContext::new("BID-1".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.background_targets.push(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>background</body>",
        Some("SID-background"),
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .mutate_parked_page_session_state("TID-background", |state| {
            state
                .devtools_session_state
                .runtime_session_state
                .runtime_frontend_enabled = true;
        });
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 432,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    ctx.expect_result(432, json!({}), Some("SID-background"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 433,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-background",
        "params": {
            "source": "globalThis.__backgroundPreload = 'ready-now';",
            "runImmediately": true
        }
    }))
    .await;
    ctx.expect_result(433, json!({ "identifier": "1" }), Some("SID-background"));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Page.addScriptToEvaluateOnNewDocument should not promote the target"
    );

    ctx.process_async(json!({
        "id": 434,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "globalThis.__backgroundPreload",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 434);
    assert_eq!(
        response["result"]["result"]["value"],
        json!("ready-now"),
        "background runtime evaluate response: {response:?}"
    );

    assert!(
        ctx.conn
            .target_owner_state_for_session(Some("SID-background"))
            .expect("background owner state should be readable")
            .document_start_scripts
            .iter()
            .any(|(identifier, script)| identifier == "1"
                && script.source == "globalThis.__backgroundPreload = 'ready-now';"),
        "background addScriptToEvaluateOnNewDocument should persist on the owner state"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn bare_isolated_worlds_do_not_persist_across_navigation() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>hello</body>")
        .await
        .expect("page should load");
    let bc = ctx.conn.browser_context.as_mut().expect("browser context");
    let _ = bc
        .active_target
        .runtime_slot
        .replace_loaded_page(Some(page));
    ctx.process_async(json!({
        "id": 49,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(49, json!({}), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 50,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility-a"
        }
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 51,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility-b",
            "grantUniversalAccess": true
        }
    }))
    .await;
    let _ = ctx.take_all();
    ctx.process_async(json!({
        "id": 52,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>next</body>" }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
        }),
        "navigation should create the new default context: {sent:?}"
    );
    assert!(
        sent.iter().all(|message| {
            message["method"] != json!("Runtime.executionContextCreated")
                || !matches!(
                    message["params"]["context"]["name"].as_str(),
                    Some("utility-a") | Some("utility-b")
                )
        }),
        "Page.createIsolatedWorld is document-scoped and must not recreate bare worlds: {sent:?}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn network_navigations_use_unique_document_loader_ids() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route(
                    "/first",
                    axum::routing::get(|| async { "<!doctype html><main>first</main>" }),
                )
                .route(
                    "/second",
                    axum::routing::get(|| async { "<!doctype html><main>second</main>" }),
                ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_target
        .runtime_slot
        .enable_primary_network_events();

    let first_url = format!("http://{addr}/first");
    let second_url = format!("http://{addr}/second");

    ctx.process_async(json!({
        "id": 30,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": first_url }
    }))
    .await;
    let first_messages = ctx.take_all();

    ctx.process_async(json!({
        "id": 31,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": second_url }
    }))
    .await;
    let second_messages = ctx.take_all();

    fn document_loader_for(messages: &[serde_json::Value], id: u64) -> String {
        let result_loader_id = messages
            .iter()
            .find(|message| message["id"] == json!(id))
            .and_then(|message| message["result"]["loaderId"].as_str())
            .expect("Page.navigate result loaderId")
            .to_owned();
        let request = messages
            .iter()
            .find(|message| {
                message["method"] == "Network.requestWillBeSent"
                    && message["params"]["type"] == "Document"
            })
            .expect("main document requestWillBeSent");
        assert_eq!(request["params"]["requestId"], result_loader_id);
        assert_eq!(request["params"]["loaderId"], result_loader_id);
        let started = messages
            .iter()
            .find(|message| message["method"] == "Page.frameStartedNavigating")
            .expect("frameStartedNavigating");
        assert_eq!(started["params"]["loaderId"], result_loader_id);
        result_loader_id
    }

    let first_loader_id = document_loader_for(&first_messages, 30);
    let second_loader_id = document_loader_for(&second_messages, 31);
    assert_ne!(first_loader_id, second_loader_id);
    assert_eq!(first_loader_id, "LID-0000000001");
    assert_eq!(second_loader_id, "LID-0000000002");
}

#[tokio::test(flavor = "multi_thread")]
async fn navigations_without_network_domain_still_use_unique_loader_ids() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route(
                    "/first",
                    axum::routing::get(|| async { "<!doctype html><main>first</main>" }),
                )
                .route(
                    "/second",
                    axum::routing::get(|| async { "<!doctype html><main>second</main>" }),
                ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    let first_url = format!("http://{addr}/first");
    let second_url = format!("http://{addr}/second");

    ctx.process_async(json!({
        "id": 30,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": first_url }
    }))
    .await;
    let first_messages = ctx.take_all();

    ctx.process_async(json!({
        "id": 31,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": second_url }
    }))
    .await;
    let second_messages = ctx.take_all();

    fn document_loader_for(messages: &[serde_json::Value], id: u64) -> String {
        let result_loader_id = messages
            .iter()
            .find(|message| message["id"] == json!(id))
            .and_then(|message| message["result"]["loaderId"].as_str())
            .expect("Page.navigate result loaderId")
            .to_owned();
        let frame_navigated = messages
            .iter()
            .find(|message| message["method"] == "Page.frameNavigated")
            .expect("frameNavigated");
        assert_eq!(
            frame_navigated["params"]["frame"]["loaderId"],
            result_loader_id
        );
        assert!(
            messages
                .iter()
                .all(|message| message["method"] != "Network.requestWillBeSent"),
            "Network disabled navigation should not emit requestWillBeSent: {messages:?}"
        );
        result_loader_id
    }

    let first_loader_id = document_loader_for(&first_messages, 30);
    let second_loader_id = document_loader_for(&second_messages, 31);
    assert_ne!(first_loader_id, second_loader_id);
    assert_eq!(first_loader_id, "LID-0000000001");
    assert_eq!(second_loader_id, "LID-0000000002");
}

#[tokio::test(flavor = "multi_thread")]
async fn crash_requires_browser_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 246, "method": "Page.crash"}))
        .await;
    ctx.expect_error(246, -31998, "BrowserContextNotLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn crash_notifies_all_attached_sessions_and_marks_browser_context_crashed() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>crash-me</body>")
        .await
        .expect("page should load");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-aux".into()));
    let _ = bc
        .active_target
        .runtime_slot
        .replace_loaded_page(Some(page));
    let active_document_token = bc
        .start_document_navigation_for_active_target("LOADER-crash".to_owned())
        .expect("active target should start document navigation");
    bc.commit_document_navigation_if_matches(&active_document_token);
    assert!(bc.accepts_document_body_completion_event(&active_document_token));
    bc.record_captured_response_body("REQ-old".into(), "body".into(), [Some("SID-1".into())]);
    bc.insert_io_stream("STREAM-old".into(), b"body".to_vec(), 0);

    ctx.process_async(json!({
        "id": 247,
        "method": "Page.crash",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(247, json!({}), Some("SID-1"));

    let inspector = ctx.take_one();
    assert_eq!(inspector["method"], "Inspector.targetCrashed");
    assert_eq!(inspector["sessionId"], "SID-1");
    let auxiliary_inspector = ctx.take_one();
    assert_eq!(auxiliary_inspector["method"], "Inspector.targetCrashed");
    assert_eq!(auxiliary_inspector["sessionId"], "SID-aux");

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.active_target.owner_state.target_crash_state.is_crashed());
    assert!(
        bc.devtools_session_state
            .runtime_session_state
            .inspector_target_crashed_delivered()
    );
    assert!(
        bc.auxiliary_devtools_session_states
            .get("SID-aux")
            .is_some_and(|state| state
                .runtime_session_state
                .inspector_target_crashed_delivered())
    );
    assert!(!bc.has_loaded_page());
    assert!(
        !bc.accepts_document_body_completion_event(&active_document_token),
        "Page.crash must reject late body completions for the crashed document"
    );
    assert!(bc.captured_response_bodies_empty_for_test());
    assert!(bc.io_streams_empty_for_test());

    ctx.process_async(json!({
        "id": 2471,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<body>recovered</body>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 2471);
    let recovery_events = ctx.take_all();
    for session_id in ["SID-1", "SID-aux"] {
        assert!(
            recovery_events.iter().any(|message| {
                message["method"] == json!("Inspector.targetReloadedAfterCrash")
                    && message["sessionId"] == json!(session_id)
            }),
            "session {session_id} that observed the crash must observe recovery: {recovery_events:?}"
        );
    }
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target
            .owner_state
            .target_crash_state
            .is_crashed()
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn crash_targets_background_owner_without_promotion() {
    let mut ctx = TestContext::new();
    let background_page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>background-crash</body>")
        .await
        .expect("background page should load");
    let mut background = BackgroundTarget::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        background_page.final_url().as_str().to_owned(),
    );
    background.replace_loaded_page(Some(background_page));

    let mut bc = BrowserContext::new("BID-1".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.set_target_url("data:text/html,<title>Active</title><main>active</main>".to_owned());
    bc.background_targets.push(background);
    bc.replace_parked_page_session_state(
        "TID-background".to_owned(),
        crate::conn::ParkedPageSessionState {
            devtools_session_state: crate::conn::DevToolsSessionState {
                runtime_session_state: crate::conn::TargetRuntimeSessionState {
                    inspector_enabled: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let background_document_token = bc
        .start_document_navigation_for_target(
            "TID-background",
            "LOADER-background-crash".to_owned(),
        )
        .expect("background target should start document navigation");
    bc.commit_document_navigation_if_matches(&background_document_token);
    assert!(bc.accepts_document_body_completion_event(&background_document_token));
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 248,
        "method": "Page.crash",
        "sessionId": "SID-background"
    }))
    .await;
    ctx.expect_result(248, json!({}), Some("SID-background"));

    let inspector = ctx.take_one();
    assert_eq!(inspector["method"], "Inspector.targetCrashed");
    assert_eq!(inspector["sessionId"], "SID-background");

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(
        bc.active_target_id(),
        Some("TID-active"),
        "background Page.crash should not promote the target"
    );
    let background = bc
        .background_target("TID-background")
        .expect("background target should remain parked");
    assert!(!background.has_loaded_page());
    assert!(
        !bc.accepts_document_body_completion_event(&background_document_token),
        "background Page.crash must reject late body completions for the crashed document"
    );
    assert!(
        bc.parked_target_owner_state_or_default("TID-background")
            .target_crash_state
            .is_crashed()
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn crash_aborts_paused_request_stage_navigation() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();

    ctx.process_async(json!({
        "id": 248,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(248, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 249,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "http://example.test/crash-request-stage" }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    let network_id = paused["params"]["networkId"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 250,
        "method": "Page.crash",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(250, json!({}), Some("SID-1"));

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["type"], "Document");
    assert_eq!(failed["params"]["errorText"], "Page crashed");

    let error = ctx.take_one();
    assert_eq!(error["id"], 249);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Page crashed");

    let inspector = ctx.take_one();
    assert_eq!(inspector["method"], "Inspector.targetCrashed");
    assert_eq!(inspector["sessionId"], "SID-1");
}
#[tokio::test(flavor = "multi_thread")]
async fn crash_aborts_background_paused_navigation_without_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-active", "SID-active", "about:blank");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .background_targets
        .push(BackgroundTarget::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "about:blank#background".to_owned(),
        ));

    ctx.process_async(json!({
        "id": 252,
        "method": "Network.enable",
        "sessionId": "SID-background"
    }))
    .await;
    ctx.expect_result(252, json!({}), Some("SID-background"));

    ctx.process_async(json!({
        "id": 253,
        "method": "Fetch.enable",
        "sessionId": "SID-background"
    }))
    .await;
    ctx.expect_result(253, json!({}), Some("SID-background"));

    ctx.process_async(json!({
        "id": 254,
        "method": "Page.navigate",
        "sessionId": "SID-background",
        "params": { "url": "http://example.test/background-crash-request-stage" }
    }))
    .await;
    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["sessionId"], "SID-background");
    let network_id = paused["params"]["networkId"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 255,
        "method": "Page.crash",
        "sessionId": "SID-background"
    }))
    .await;
    ctx.expect_result(255, json!({}), Some("SID-background"));

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["sessionId"], "SID-background");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["errorText"], "Page crashed");

    let error = ctx.take_one();
    assert_eq!(error["id"], 254);
    assert_eq!(error["sessionId"], "SID-background");
    assert_eq!(error["error"]["message"], "Page crashed");

    let inspector = ctx.take_one();
    assert_eq!(inspector["method"], "Inspector.targetCrashed");
    assert_eq!(inspector["sessionId"], "SID-background");

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(
        bc.active_target_id(),
        Some("TID-active"),
        "background Page.crash should not promote the target"
    );
    assert!(
        bc.parked_fetch_state("TID-background")
            .is_none_or(|state| state.is_empty())
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn crash_without_target_loaded_errors() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 251,
        "method": "Page.crash"
    }))
    .await;

    ctx.expect_error(251, -31998, "TargetNotLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn crash_aborts_paused_response_stage_navigation() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [
                (axum::http::header::CONTENT_TYPE.as_str(), "text/html"),
                ("x-stage", "response"),
            ],
            "<!doctype html><html><body>crash-response</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();

    ctx.process_async(json!({
            "id": 251,
            "method": "Fetch.enable",
            "sessionId": "SID-1",
            "params": {
                "patterns": [{ "urlPattern": "*", "requestStage": "Response", "resourceType": "Document" }]
            }
        })).await;
    ctx.expect_result(251, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 252,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    consume_main_document_navigation_start(&mut ctx);
    let request = ctx.take_one();
    assert_eq!(request["method"], "Network.requestWillBeSent");
    let network_id = request["params"]["requestId"].clone();

    let paused = take_main_document_response_pause_after_extra_info(&mut ctx, &network_id, 200);
    assert_eq!(paused["params"]["networkId"], network_id);
    assert_eq!(paused["params"]["responseStatusCode"], 200);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 253,
        "method": "Page.crash",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(253, json!({}), Some("SID-1"));

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["type"], "Document");
    assert_eq!(failed["params"]["errorText"], "Page crashed");

    let error = ctx.take_one();
    assert_eq!(error["id"], 252);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Page crashed");

    let inspector = ctx.take_one();
    assert_eq!(inspector["method"], "Inspector.targetCrashed");
    assert_eq!(inspector["sessionId"], "SID-1");

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn close_clears_loaded_page_state_and_emits_detached_events() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>close-me</body>")
        .await
        .expect("page should load");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.devtools_session_state
        .page_session_state
        .page_lifecycle_events = true;
    bc.devtools_session_state
        .runtime_session_state
        .runtime_frontend_enabled = true;
    bc.devtools_session_state
        .runtime_session_state
        .inspector_enabled = true;
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    bc.network_policy.set_cache_disabled(true);
    bc.network_policy.set_bypass_service_worker(true);
    bc.css_enabled = true;
    bc.active_target.fetch_owner.configure(
        None,
        true,
        vec![FetchInterceptionPattern {
            url_pattern: "*".into(),
            resource_type_filter: None,
            request_stage: FetchRequestStage::Response,
        }],
    );
    bc.network_policy
        .push_extra_header(("X-Test".into(), "1".into()));
    bc.active_target
        .owner_state
        .target_crash_state
        .mark_crashed();
    let _ = bc
        .active_target
        .runtime_slot
        .replace_loaded_page(Some(page));
    let close_document_token = bc
        .start_document_navigation_for_active_target("LOADER-close".to_owned())
        .expect("active target should start document navigation");
    bc.commit_document_navigation_if_matches(&close_document_token);
    assert!(bc.accepts_document_body_completion_event(&close_document_token));
    bc.set_target_security_origin("null".into());
    bc.set_target_secure_context_type("InsecureScheme".into());
    bc.set_next_network_request_sequence_for_test(41);
    bc.set_subresource_network_emitted_record_count_for_test(12);
    bc.set_next_io_stream_sequence_for_test(7);
    bc.active_target
        .runtime_slot
        .set_next_subresource_fetch_request_id_for_test(5);
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-aux".into()));
    bc.remember_target_window_name("close-me", "TID-1");
    bc.target_opener_ids
        .insert("TID-popup-after-close".into(), "TID-1".into());
    bc.target_opener_frame_ids
        .insert("TID-popup-after-close".into(), "FRAME-1".into());
    bc.record_captured_response_body("REQ-old".into(), "body".into(), [Some("SID-1".into())]);
    bc.insert_io_stream("STREAM-old".into(), b"body".to_vec(), 0);

    ctx.process_async(json!({
        "id": 242,
        "method": "Page.close",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(242, json!({}), Some("SID-1"));

    let detached = ctx.take_all();
    assert_eq!(detached.len(), 4);
    assert_eq!(detached[0]["method"], "Inspector.detached");
    assert_eq!(detached[0]["sessionId"], "SID-1");
    assert_eq!(detached[1]["method"], "Inspector.detached");
    assert_eq!(detached[1]["sessionId"], "SID-aux");
    assert_eq!(detached[2]["method"], "Target.detachedFromTarget");
    assert_eq!(detached[2]["params"]["targetId"], "TID-1");
    assert_eq!(detached[2]["params"]["sessionId"], "SID-1");
    assert_eq!(detached[3]["method"], "Target.detachedFromTarget");
    assert_eq!(detached[3]["params"]["targetId"], "TID-1");
    assert_eq!(detached[3]["params"]["sessionId"], "SID-aux");

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_active_target());
    assert!(!bc.has_active_session());
    assert!(bc.auxiliary_target_id_for_session("SID-aux").is_none());
    assert!(bc.target_id_for_window_name("close-me").is_none());
    assert!(!bc.target_opener_ids.contains_key("TID-popup-after-close"));
    assert!(
        !bc.target_opener_frame_ids
            .contains_key("TID-popup-after-close")
    );
    assert!(!bc.has_loaded_page());
    assert!(
        !bc.accepts_document_body_completion_event(&close_document_token),
        "Page.close must reject late body completions for the closed document"
    );
    assert_eq!(bc.target_url(), "about:blank");
    assert_eq!(bc.target_security_origin(), crate::conn::URL_BASE);
    assert_eq!(bc.target_secure_context_type(), "Secure");
    assert_eq!(bc.next_network_request_sequence_for_test(), 0);
    assert_eq!(bc.subresource_network_emitted_record_count_for_test(), 0);
    assert_eq!(bc.next_io_stream_sequence_for_test(), 0);
    assert_eq!(
        bc.active_target
            .runtime_slot
            .next_subresource_fetch_request_id_for_test(),
        0
    );
    assert!(!bc.active_target.owner_state.target_crash_state.is_crashed());
    assert!(
        !bc.devtools_session_state
            .page_session_state
            .page_lifecycle_events
    );
    assert!(
        !bc.devtools_session_state
            .runtime_session_state
            .runtime_frontend_enabled
    );
    assert!(
        !bc.devtools_session_state
            .runtime_session_state
            .inspector_enabled
    );
    assert!(
        !bc.active_target
            .runtime_slot
            .primary_network_events_enabled()
    );
    assert!(!bc.network_policy.cache_disabled());
    assert!(!bc.network_policy.bypass_service_worker());
    assert!(!bc.css_enabled);
    assert!(!bc.active_target.fetch_owner.is_enabled());
    assert!(!bc.active_target.fetch_owner.handle_auth_requests());
    assert!(
        bc.active_target
            .fetch_owner
            .config_snapshot()
            .patterns()
            .is_empty()
    );
    assert!(bc.network_policy.extra_headers().is_empty());
    assert!(bc.captured_response_bodies_empty_for_test());
    assert!(bc.io_streams_empty_for_test());
    assert!(
        !bc.active_target
            .fetch_owner
            .has_pending_fetch_state_for_test()
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn close_aborts_paused_request_stage_navigation_and_clears_state() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.devtools_session_state
        .runtime_session_state
        .inspector_enabled = true;
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();

    ctx.process_async(json!({
        "id": 243,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(243, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 244,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "http://example.test/page-close" }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    assert_eq!(paused["sessionId"], "SID-1");
    let network_id = paused["params"]["networkId"].clone();

    ctx.process_async(json!({
        "id": 245,
        "method": "Page.close",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(245, json!({}), Some("SID-1"));

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["errorText"], "Page closed");

    let error = ctx.take_one();
    assert_eq!(error["id"], 244);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Page closed");

    let inspector = ctx.take_one();
    assert_eq!(inspector["method"], "Inspector.detached");
    assert_eq!(inspector["sessionId"], "SID-1");

    let target = ctx.take_one();
    assert_eq!(target["method"], "Target.detachedFromTarget");
    assert_eq!(target["params"]["targetId"], "TID-1");
    assert_eq!(target["params"]["sessionId"], "SID-1");

    let messages = ctx.take_all();
    for method in [
        "Page.frameClearedScheduledNavigation",
        "Page.frameNavigated",
        "DOM.documentUpdated",
        "Page.domContentEventFired",
        "Page.loadEventFired",
        "Page.lifecycleEvent",
        "Page.frameStoppedLoading",
        "Network.responseReceived",
        "Network.loadingFinished",
    ] {
        assert!(
            !messages
                .iter()
                .any(|message| message["method"] == json!(method)),
            "unexpected completion event after Page.close: {method}"
        );
    }

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.has_active_target());
    assert!(!bc.has_active_session());
    assert!(!bc.has_loaded_page());
    assert!(
        !bc.active_target
            .fetch_owner
            .has_pending_fetch_state_for_test()
    );
    assert_eq!(bc.target_url(), "about:blank");
}
#[tokio::test(flavor = "multi_thread")]
async fn close_aborts_background_paused_navigation_without_promotion() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-active", "SID-active", "about:blank");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.background_targets.push(BackgroundTarget::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank#background".to_owned(),
    ));
    assert!(bc.assign_auxiliary_session_to_target("TID-background", "SID-aux".to_owned()));
    bc.replace_parked_page_session_state(
        "TID-background".to_owned(),
        crate::conn::ParkedPageSessionState {
            devtools_session_state: crate::conn::DevToolsSessionState {
                runtime_session_state: crate::conn::TargetRuntimeSessionState {
                    inspector_enabled: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        },
    );

    ctx.process_async(json!({
        "id": 246,
        "method": "Network.enable",
        "sessionId": "SID-background"
    }))
    .await;
    ctx.expect_result(246, json!({}), Some("SID-background"));

    ctx.process_async(json!({
        "id": 247,
        "method": "Fetch.enable",
        "sessionId": "SID-background"
    }))
    .await;
    ctx.expect_result(247, json!({}), Some("SID-background"));

    ctx.process_async(json!({
        "id": 248,
        "method": "Page.navigate",
        "sessionId": "SID-background",
        "params": { "url": "http://example.test/background-page-close" }
    }))
    .await;
    let paused =
        ctx.take_first_matching("background main-document Fetch.requestPaused", |message| {
            message["method"] == json!("Fetch.requestPaused")
                && message["sessionId"] == json!("SID-background")
                && message["params"]["request"]["url"]
                    == json!("http://example.test/background-page-close")
        });
    assert_eq!(paused["sessionId"], "SID-background");
    let network_id = paused["params"]["networkId"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 249,
        "method": "Page.close",
        "sessionId": "SID-background"
    }))
    .await;
    ctx.expect_result(249, json!({}), Some("SID-background"));

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["sessionId"], "SID-background");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["errorText"], "Page closed");

    let error = ctx.take_one();
    assert_eq!(error["id"], 248);
    assert_eq!(error["sessionId"], "SID-background");
    assert_eq!(error["error"]["message"], "Page closed");

    let inspector = ctx.take_one();
    assert_eq!(inspector["method"], "Inspector.detached");
    assert_eq!(inspector["sessionId"], "SID-background");

    let auxiliary_inspector = ctx.take_one();
    assert_eq!(auxiliary_inspector["method"], "Inspector.detached");
    assert_eq!(auxiliary_inspector["sessionId"], "SID-aux");

    let primary_detached = ctx.take_one();
    assert_eq!(primary_detached["method"], "Target.detachedFromTarget");
    assert_eq!(primary_detached["params"]["targetId"], "TID-background");
    assert_eq!(primary_detached["params"]["sessionId"], "SID-background");

    let auxiliary_detached = ctx.take_one();
    assert_eq!(auxiliary_detached["method"], "Target.detachedFromTarget");
    assert_eq!(auxiliary_detached["params"]["targetId"], "TID-background");
    assert_eq!(auxiliary_detached["params"]["sessionId"], "SID-aux");

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(
        bc.active_target_id(),
        Some("TID-active"),
        "background Page.close should not promote the target"
    );
    assert!(bc.background_target("TID-background").is_none());
    assert!(bc.auxiliary_target_id_for_session("SID-aux").is_none());
    assert!(bc.parked_page_session_state("TID-background").is_none());
    assert!(
        bc.parked_fetch_state("TID-background")
            .is_none_or(|state| state.is_empty())
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn close_without_target_loaded_errors() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 2460,
        "method": "Page.close"
    }))
    .await;

    ctx.expect_error(2460, -31998, "TargetNotLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn close_aborts_paused_auth_navigation_and_clears_state() {
    async fn auth(headers: axum::http::HeaderMap) -> impl axum::response::IntoResponse {
        let expected = "Basic YWxhZGRpbjpvcGVuc2VzYW1l";
        let authorization = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok());
        if authorization != Some(expected) {
            return axum::response::IntoResponse::into_response((
                axum::http::StatusCode::UNAUTHORIZED,
                [
                    (
                        axum::http::header::WWW_AUTHENTICATE.as_str(),
                        r#"Basic realm="close-area""#,
                    ),
                    (axum::http::header::CONTENT_TYPE.as_str(), "text/plain"),
                ],
                "auth required",
            ));
        }

        axum::response::IntoResponse::into_response((
            axum::http::StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>authorized</body></html>",
        ))
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/auth", axum::routing::get(auth)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.devtools_session_state
        .runtime_session_state
        .inspector_enabled = true;
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();

    ctx.process_async(json!({
        "id": 259,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(259, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 260,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/auth") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = paused["params"]["networkId"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 261,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(261, json!({}), Some("SID-1"));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], json!(request_id));
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["resourceType"], "Document");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 262,
        "method": "Page.close",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(262, json!({}), Some("SID-1"));

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["type"], "Document");
    assert_eq!(failed["params"]["errorText"], "Page closed");

    let error = ctx.take_one();
    assert_eq!(error["id"], 260);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Page closed");

    let inspector = ctx.take_one();
    assert_eq!(inspector["method"], "Inspector.detached");
    assert_eq!(inspector["sessionId"], "SID-1");

    let target = ctx.take_one();
    assert_eq!(target["method"], "Target.detachedFromTarget");
    assert_eq!(target["params"]["targetId"], "TID-1");
    assert_eq!(target["params"]["sessionId"], "SID-1");

    ctx.process_async(json!({
        "id": 263,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "aladdin",
                "password": "opensesame"
            }
        }
    }))
    .await;
    ctx.expect_error(263, -32001, "Unknown sessionId");

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn close_without_inspector_enabled_emits_inspector_detached_event() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 246,
        "method": "Page.close",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(246, json!({}), Some("SID-1"));

    let detached = ctx.take_all();
    assert_eq!(detached.len(), 2);
    assert_eq!(detached[0]["method"], "Inspector.detached");
    assert_eq!(detached[0]["sessionId"], "SID-1");
    assert_eq!(detached[0]["params"]["reason"], "Render process gone.");
    assert_eq!(detached[1]["method"], "Target.detachedFromTarget");
    assert_eq!(detached[1]["params"]["targetId"], "TID-1");
    assert_eq!(detached[1]["params"]["sessionId"], "SID-1");
}

#[tokio::test(flavor = "multi_thread")]
async fn close_command_background_events_keep_target_detached_sidecar() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-typed-close",
        "TID-typed-close",
        "SID-typed-close",
        "about:blank",
    );

    let raw = json!({
        "id": 247,
        "method": "Page.close",
        "sessionId": "SID-typed-close"
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("Page.close should complete through pending dispatch");
    };
    let CdpCommandTaskStep::Complete(outcome) = ctx
        .conn
        .complete_pending_command_dispatch(pending.wait().await)
        .await
    else {
        panic!("Page.close pending dispatch should complete");
    };
    let (
        mut events,
        post_renderer_output_events,
        renderer_output_boundary,
        post_response_events,
        scheduler_events,
        renderer_output_predecessor,
    ) = outcome.into_renderer_owner_turn_parts();
    assert!(renderer_output_boundary.is_none());
    assert!(post_renderer_output_events.is_empty());
    assert!(renderer_output_predecessor.is_none());
    events.extend(post_response_events);
    let [CdpSchedulerEvent::ProtocolWorkPublished { work }] = <[_; 1]>::try_from(scheduler_events)
        .expect("Page.close should publish exactly one target-termination owner action")
    else {
        unreachable!("array pattern fixes the only event kind")
    };
    assert_eq!(
        work.kind(),
        crate::domains::activity::ProtocolSchedulerWorkKind::PageTargetTerminationOwnerAction
    );

    // The command response and target retirement are intentionally separate
    // transactions. Page.close first settles any final renderer publication;
    // only then may this owner action retire the session route and materialize
    // the detach sidecar. Exercise that real scheduler boundary instead of
    // expecting the old command-local destructive drain.
    let (mut termination_events, nested_scheduler_events) = ctx
        .conn
        .complete_ready_protocol_scheduler_work_turn(work)
        .await
        .into_protocol_event_parts();
    assert!(nested_scheduler_events.is_empty());
    events.append(&mut termination_events);

    let response = events
        .iter()
        .find(|event| event.protocol_message_id() == Some(247))
        .cloned()
        .expect("Page.close command response")
        .into_protocol_message();
    assert_eq!(response["result"], json!({}));
    assert_eq!(response["sessionId"], "SID-typed-close");

    let (message, automation_event) = events
        .into_iter()
        .find_map(|event| {
            let (message, automation_event) = event.into_parts();
            (message["method"] == json!("Target.detachedFromTarget"))
                .then_some((message, automation_event))
        })
        .expect("Target.detachedFromTarget background event");
    assert_eq!(message["params"]["targetId"], "TID-typed-close");
    assert_eq!(message["params"]["sessionId"], "SID-typed-close");
    assert!(message["params"].get("reason").is_none());

    let Some(AutomationEvent::TargetDetached(event)) = automation_event else {
        panic!("expected TargetDetached automation sidecar");
    };
    assert_eq!(event.target_id.as_str(), "TID-typed-close");
    assert_eq!(event.session_id.as_str(), "SID-typed-close");
    assert_eq!(event.reason.as_deref(), Some("Render process gone."));
}

#[tokio::test(flavor = "multi_thread")]
async fn set_bypass_csp_accepts_valid_params_and_returns_empty_result() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-PAGE".into()));
    ctx.process_async(json!({
        "id": 2,
        "method": "Page.setBypassCSP",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(2, json!({}), None);
    assert!(ctx.conn.browser_context.as_ref().is_some_and(|bc| {
        bc.devtools_session_state
            .page_session_state
            .page_bypass_csp_enabled
    }));
}
#[tokio::test(flavor = "multi_thread")]
async fn set_bypass_csp_rejects_invalid_params() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 3,
        "method": "Page.setBypassCSP",
        "params": {}
    }))
    .await;
    ctx.expect_error(3, -32602, "InvalidParams");
}
#[tokio::test(flavor = "multi_thread")]
async fn set_font_families_accepts_object_params_and_returns_empty_result() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-PAGE".into()));
    ctx.process_async(json!({
        "id": 4,
        "method": "Page.setFontFamilies",
        "params": {
            "standard": "Times New Roman",
            "fixed": "Courier New"
        }
    }))
    .await;
    ctx.expect_result(4, json!({}), None);
    let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        browser_context
            .devtools_session_state
            .page_session_state
            .page_font_families
            .get("standard"),
        Some(&json!("Times New Roman"))
    );
    assert_eq!(
        browser_context
            .devtools_session_state
            .page_session_state
            .page_font_families
            .get("fixed"),
        Some(&json!("Courier New"))
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn set_font_families_rejects_invalid_params() {
    let mut ctx = TestContext::new();
    let raw = json!({
        "id": 5,
        "method": "Page.setFontFamilies",
        "params": []
    })
    .to_string();
    let outcome = ctx.conn.process_message_with_turn_outcome_async(&raw).await;
    let (messages, scheduler_events) = ctx.route_completed_command_outcome_for_test(outcome).await;
    assert!(scheduler_events.is_empty());
    assert_eq!(
        messages,
        vec![json!({
            "id": 5,
            "error": {"code": -32600, "message": "Invalid Request"}
        })]
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn set_intercept_file_chooser_dialog_accepts_valid_params_and_returns_empty_result() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-PAGE".into()));
    ctx.process_async(json!({
        "id": 51,
        "method": "Page.setInterceptFileChooserDialog",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(51, json!({}), None);
    assert!(ctx.conn.browser_context.as_ref().is_some_and(|bc| {
        bc.devtools_session_state
            .page_session_state
            .page_intercept_file_chooser_dialog_enabled
    }));
}
#[tokio::test(flavor = "multi_thread")]
async fn set_intercept_file_chooser_dialog_rejects_invalid_params() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 52,
        "method": "Page.setInterceptFileChooserDialog",
        "params": {}
    }))
    .await;
    ctx.expect_error(52, -32602, "InvalidParams");
}

#[tokio::test(flavor = "multi_thread")]
async fn screencast_commands_update_page_session_state() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-screencast",
        "TID-screencast",
        "SID-screencast",
        "about:blank",
    );

    ctx.process_async(json!({
        "id": 53,
        "method": "Page.startScreencast",
        "sessionId": "SID-screencast",
        "params": {
            "format": "png",
            "quality": 80,
            "maxWidth": 800,
            "maxHeight": 600,
            "everyNthFrame": 1
        }
    }))
    .await;
    let visibility = ctx.take_first_matching("screencast visibility event", |message| {
        message["method"] == json!("Page.screencastVisibilityChanged")
    });
    assert_eq!(visibility["sessionId"], "SID-screencast");
    assert_eq!(visibility["params"]["visible"], true);
    ctx.expect_result(53, json!({}), Some("SID-screencast"));
    let state = ctx
        .conn
        .target_page_session_state_for_session(Some("SID-screencast"))
        .expect("page session state");
    assert!(state.page_screencast.is_active());
    assert_eq!(state.page_screencast.generation(), 1);
    let config = state
        .page_screencast
        .config()
        .expect("normalized screencast config");
    assert_eq!(config.format(), crate::conn::PageScreencastFormat::Png);
    assert_eq!(config.quality(), 80);
    assert_eq!(config.max_width(), Some(800));
    assert_eq!(config.max_height(), Some(600));
    assert_eq!(config.every_nth_frame(), 1);

    assert_eq!(
        ctx.conn
            .begin_page_screencast_capture_for_session_owner(Some("SID-screencast"), 1),
        Some(true)
    );
    assert_eq!(
        ctx.conn.complete_page_screencast_capture_for_session_owner(
            Some("SID-screencast"),
            1,
            true,
        ),
        Some(true)
    );

    ctx.process_async(json!({
        "id": 54,
        "method": "Page.screencastFrameAck",
        "sessionId": "SID-screencast",
        "params": { "sessionId": 1 }
    }))
    .await;
    ctx.expect_result(54, json!({}), Some("SID-screencast"));
    assert!(
        !ctx.conn
            .target_page_session_state_for_session(Some("SID-screencast"))
            .expect("page session state after ACK")
            .page_screencast
            .awaiting_ack()
    );

    ctx.process_async(json!({
        "id": 55,
        "method": "Page.stopScreencast",
        "sessionId": "SID-screencast"
    }))
    .await;
    ctx.expect_result(55, json!({}), Some("SID-screencast"));
    let state = ctx
        .conn
        .target_page_session_state_for_session(Some("SID-screencast"))
        .expect("page session state");
    assert!(!state.page_screencast.is_active());
    assert_eq!(state.page_screencast.generation(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn screencast_capture_materializes_jpeg_frame_and_ack_budget() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-screencast-frame",
        "TID-screencast-frame",
        "SID-screencast-frame",
        "about:blank",
    );
    ensure_initial_document_for_session(&mut ctx, Some("SID-screencast-frame")).await;

    let raw = json!({
        "id": 551,
        "method": "Page.startScreencast",
        "sessionId": "SID-screencast-frame",
        "params": {
            "format": "jpeg",
            "quality": 80,
            "maxWidth": 320,
            "maxHeight": 240
        }
    })
    .to_string();
    let CdpCommandTaskStep::Complete(outcome) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("Page.startScreencast should complete synchronously");
    };
    let (messages, scheduler_events) = outcome.into_parts();
    assert!(messages.iter().any(|message| {
        message["id"] == json!(551)
            && message["result"] == json!({})
            && message["sessionId"] == json!("SID-screencast-frame")
    }));
    let [CdpSchedulerEvent::PageScreencastStarted { registration }] =
        <[_; 1]>::try_from(scheduler_events)
            .expect("startScreencast should register exactly one scheduler subscription")
    else {
        unreachable!("array pattern fixes the only scheduler event kind")
    };

    assert_eq!(
        ctx.conn.page_screencast_subscription_status(&registration),
        PageScreencastSubscriptionStatus::Ready
    );
    let PageScreencastCaptureStart::Pending(capture) =
        ctx.conn.start_page_screencast_frame_capture(&registration)
    else {
        panic!("loaded screencast should start an async renderer capture");
    };
    assert_eq!(
        ctx.conn.page_screencast_subscription_status(&registration),
        PageScreencastSubscriptionStatus::CaptureInProgress
    );

    let PageScreencastCaptureCompletion::Frame(frame) = ctx
        .conn
        .complete_page_screencast_frame_capture(capture.wait().await)
    else {
        panic!("renderer capture should materialize a frame event");
    };
    let (frame, automation_event) = frame.into_parts();
    assert_eq!(frame["method"], json!("Page.screencastFrame"));
    assert_eq!(frame["sessionId"], json!("SID-screencast-frame"));
    assert_eq!(frame["params"]["sessionId"], json!(1));
    assert!(
        frame["params"]["metadata"]["deviceWidth"]
            .as_f64()
            .is_some_and(|width| width > 0.0)
    );
    assert!(
        frame["params"]["metadata"]["deviceHeight"]
            .as_f64()
            .is_some_and(|height| height > 0.0)
    );
    assert_eq!(automation_event, None);
    let encoded = BASE64_STANDARD
        .decode(frame["params"]["data"].as_str().expect("frame data"))
        .expect("frame data should be base64");
    assert_eq!(&encoded[..2], &[0xff, 0xd8]);
    assert_eq!(&encoded[encoded.len() - 2..], &[0xff, 0xd9]);
    assert_eq!(
        ctx.conn.page_screencast_subscription_status(&registration),
        PageScreencastSubscriptionStatus::AwaitingAck
    );
    assert_eq!(
        ctx.conn
            .acknowledge_page_screencast_frame_for_session_owner(
                Some("SID-screencast-frame"),
                registration.generation(),
            ),
        Some(true)
    );
    assert_eq!(
        ctx.conn.page_screencast_subscription_status(&registration),
        PageScreencastSubscriptionStatus::Ready
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn start_screencast_is_session_local_for_active_auxiliary_session() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-active-screencast",
        "TID-active",
        "SID-primary",
        "about:blank",
    );
    let browser_context = ctx.conn.browser_context.as_mut().unwrap();
    assert!(browser_context.assign_auxiliary_session_to_target("TID-active", "SID-aux".to_owned()));

    ctx.process_async(json!({
        "id": 56,
        "method": "Page.startScreencast",
        "sessionId": "SID-aux",
        "params": {}
    }))
    .await;
    let visibility = ctx.take_first_matching("active auxiliary screencast visibility", |message| {
        message["method"] == json!("Page.screencastVisibilityChanged")
    });
    assert_eq!(visibility["sessionId"], "SID-aux");
    ctx.expect_result(56, json!({}), Some("SID-aux"));

    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    assert!(
        !browser_context
            .devtools_session_state
            .page_session_state
            .page_screencast
            .is_active(),
        "primary page session should stay stopped"
    );
    let auxiliary = ctx
        .conn
        .target_page_session_state_for_session(Some("SID-aux"))
        .expect("active auxiliary page session state");
    assert!(auxiliary.page_screencast.is_active());
    assert_eq!(auxiliary.page_screencast.generation(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn start_screencast_is_session_local_for_background_auxiliary_session() {
    let mut ctx = TestContext::new();
    let background = BackgroundTarget::with_url(
        "TID-background".to_owned(),
        Some("SID-background-primary".to_owned()),
        "about:blank#background".to_owned(),
    );
    let mut browser_context = BrowserContext::new("BID-background-screencast".to_owned());
    browser_context.set_active_target_id("TID-active".to_owned());
    browser_context.attach_active_session("SID-active".to_owned());
    browser_context.background_targets.push(background);
    assert!(
        browser_context
            .assign_auxiliary_session_to_target("TID-background", "SID-background-aux".to_owned())
    );
    ctx.conn.browser_context = Some(browser_context);

    ctx.process_async(json!({
        "id": 57,
        "method": "Page.startScreencast",
        "sessionId": "SID-background-aux",
        "params": {}
    }))
    .await;
    let visibility = ctx
        .take_first_matching("background auxiliary screencast visibility", |message| {
            message["method"] == json!("Page.screencastVisibilityChanged")
        });
    assert_eq!(visibility["sessionId"], "SID-background-aux");
    ctx.expect_result(57, json!({}), Some("SID-background-aux"));

    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    let parked = browser_context
        .parked_page_session_state("TID-background")
        .expect("background target should retain parked page session state");
    assert!(
        !parked
            .devtools_session_state
            .page_session_state
            .page_screencast
            .is_active(),
        "background primary page session should stay stopped"
    );
    let auxiliary = ctx
        .conn
        .target_page_session_state_for_session(Some("SID-background-aux"))
        .expect("background auxiliary page session state");
    assert!(auxiliary.page_screencast.is_active());
    assert_eq!(auxiliary.page_screencast.generation(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn start_screencast_rejects_invalid_params() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-screencast-invalid",
        "TID-screencast-invalid",
        "SID-screencast-invalid",
        "about:blank",
    );
    for (id, params) in [
        (58, json!({ "format": "webp" })),
        (59, json!({ "quality": 101 })),
        (60, json!({ "maxWidth": -1 })),
        (61, json!({ "everyNthFrame": 0 })),
        (62, json!({ "maxHeight": i64::from(u32::MAX) + 1 })),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.startScreencast",
            "sessionId": "SID-screencast-invalid",
            "params": params
        }))
        .await;
        ctx.expect_error(id, -32602, "InvalidParams");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_start_invalidates_old_screencast_ack_generation() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-screencast-generation",
        "TID-screencast-generation",
        "SID-screencast-generation",
        "about:blank",
    );

    for (id, params) in [
        (63, json!({ "format": "png" })),
        (
            64,
            json!({
                "format": "jpeg",
                "quality": 72,
                "maxWidth": 0,
                "maxHeight": 480,
                "everyNthFrame": 2
            }),
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.startScreencast",
            "sessionId": "SID-screencast-generation",
            "params": params
        }))
        .await;
        let visibility = ctx.take_first_matching("screencast visibility event", |message| {
            message["method"] == json!("Page.screencastVisibilityChanged")
        });
        assert_eq!(visibility["sessionId"], "SID-screencast-generation");
        ctx.expect_result(id, json!({}), Some("SID-screencast-generation"));

        let generation = i32::try_from(id - 62).expect("small screencast generation");
        assert_eq!(
            ctx.conn.begin_page_screencast_capture_for_session_owner(
                Some("SID-screencast-generation"),
                generation,
            ),
            Some(true)
        );
        assert_eq!(
            ctx.conn.complete_page_screencast_capture_for_session_owner(
                Some("SID-screencast-generation"),
                generation,
                true,
            ),
            Some(true)
        );
    }

    let state = ctx
        .conn
        .target_page_session_state_for_session(Some("SID-screencast-generation"))
        .expect("current screencast state");
    assert_eq!(state.page_screencast.generation(), 2);
    let config = state.page_screencast.config().expect("current config");
    assert_eq!(config.format(), crate::conn::PageScreencastFormat::Jpeg);
    assert_eq!(config.quality(), 72);
    assert_eq!(config.max_width(), None);
    assert_eq!(config.max_height(), Some(480));
    assert_eq!(config.every_nth_frame(), 2);
    assert!(state.page_screencast.awaiting_ack());

    ctx.process_async(json!({
        "id": 65,
        "method": "Page.screencastFrameAck",
        "sessionId": "SID-screencast-generation",
        "params": { "sessionId": 1 }
    }))
    .await;
    ctx.expect_result(65, json!({}), Some("SID-screencast-generation"));
    assert!(
        ctx.conn
            .target_page_session_state_for_session(Some("SID-screencast-generation"))
            .expect("state after stale ACK")
            .page_screencast
            .awaiting_ack(),
        "an old generation ACK must not release the current frame"
    );

    ctx.process_async(json!({
        "id": 66,
        "method": "Page.screencastFrameAck",
        "sessionId": "SID-screencast-generation",
        "params": { "sessionId": 2 }
    }))
    .await;
    ctx.expect_result(66, json!({}), Some("SID-screencast-generation"));
    assert!(
        !ctx.conn
            .target_page_session_state_for_session(Some("SID-screencast-generation"))
            .expect("state after current ACK")
            .page_screencast
            .awaiting_ack()
    );

    ctx.process_async(json!({
        "id": 67,
        "method": "Page.screencastFrameAck",
        "sessionId": "SID-screencast-generation",
        "params": { "sessionId": 0 }
    }))
    .await;
    ctx.expect_error(67, -32602, "InvalidParams");
}

#[tokio::test(flavor = "multi_thread")]
async fn start_screencast_rejects_mock_layout_without_activating_state() {
    let conn = crate::conn::CdpConnection::new_with_initial_storage_partition_and_runtime_config(
        crate::conn::CdpInitialStoragePartition::memory(),
        moli_core::runtime::NavigationRuntimeConfig::new(
            moli_fetch::FetchConfig::default(),
            moli_core::OptionalResourceFetchMask::NONE,
            true,
            moli_core::LayoutPolicy::Mock,
        ),
    );
    let mut ctx = TestContext::from_conn(conn);
    load_bc_with_session(
        &mut ctx,
        "BID-screencast-mock",
        "TID-screencast-mock",
        "SID-screencast-mock",
        "about:blank",
    );

    ctx.process_async(json!({
        "id": 68,
        "method": "Page.startScreencast",
        "sessionId": "SID-screencast-mock",
        "params": { "format": "jpeg", "quality": 80 }
    }))
    .await;
    ctx.expect_error(
        68,
        -32000,
        "Page.startScreencast is not supported: renderer layout is disabled.",
    );
    assert!(
        !ctx.conn
            .target_page_session_state_for_session(Some("SID-screencast-mock"))
            .expect("mock page session state")
            .page_screencast
            .is_active()
    );
    assert!(
        ctx.sent
            .iter()
            .all(|message| { message["method"] != json!("Page.screencastVisibilityChanged") })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_chromium_page_init_command_sequence_returns_results() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 6,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-1",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(6, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 7,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7, json!({}), Some("SID-1"));
    let default_context = ctx.take_first_matching(
        "Runtime.executionContextCreated for initial document",
        |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!("SID-1")
        },
    );
    assert_eq!(default_context["method"], "Runtime.executionContextCreated");
    assert_eq!(default_context["sessionId"], "SID-1");
    assert_eq!(
        default_context["params"]["context"]["name"],
        json!("about:blank")
    );
    assert_eq!(
        default_context["params"]["context"]["auxData"]["isDefault"],
        json!(true)
    );

    ctx.process_async(json!({
        "id": 8,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "",
            "worldName": "__playwright_utility_world__"
        }
    }))
    .await;
    let add_script = take_response_by_id(&mut ctx, 8);
    assert_eq!(add_script["sessionId"], "SID-1");
    assert!(add_script["result"]["identifier"].as_str().is_some());

    ctx.process_async(json!({
        "id": 9,
        "method": "Target.setAutoAttach",
        "sessionId": "SID-1",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": true,
            "flatten": true
        }
    }))
    .await;
    ctx.expect_result(9, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 10,
        "method": "Emulation.setFocusEmulationEnabled",
        "sessionId": "SID-1",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(10, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 11,
        "method": "Security.setIgnoreCertificateErrors",
        "sessionId": "SID-1",
        "params": { "ignore": true }
    }))
    .await;
    ctx.expect_result(11, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 12,
        "method": "Emulation.setScriptExecutionDisabled",
        "sessionId": "SID-1",
        "params": { "value": true }
    }))
    .await;
    ctx.expect_result(12, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 13,
        "method": "Emulation.setLocaleOverride",
        "sessionId": "SID-1",
        "params": { "locale": "en-US" }
    }))
    .await;
    ctx.expect_result(13, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 14,
        "method": "Emulation.setTimezoneOverride",
        "sessionId": "SID-1",
        "params": { "timezoneId": "UTC" }
    }))
    .await;
    ctx.expect_result(14, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 15,
        "method": "Page.setFontFamilies",
        "sessionId": "SID-1",
        "params": {
            "standard": "Times New Roman",
            "fixed": "Courier New"
        }
    }))
    .await;
    ctx.expect_result(15, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 16,
        "method": "Emulation.setGeolocationOverride",
        "sessionId": "SID-1",
        "params": {}
    }))
    .await;
    ctx.expect_result(16, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 17,
        "method": "Emulation.setEmulatedMedia",
        "sessionId": "SID-1",
        "params": {
            "features": [
                { "name": "prefers-color-scheme", "value": "dark" }
            ]
        }
    }))
    .await;
    ctx.expect_result(17, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 18,
        "method": "Page.setInterceptFileChooserDialog",
        "sessionId": "SID-1",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(18, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 19,
        "method": "Runtime.runIfWaitingForDebugger",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(19, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 20,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "__playwright_utility_world_page",
            "grantUniveralAccess": true
        }
    }))
    .await;
    let isolated = take_response_by_id(&mut ctx, 20);
    assert_eq!(isolated["sessionId"], "SID-1");
    assert!(isolated["result"]["executionContextId"].as_i64().is_some());
    let created = ctx.take_first_matching(
        "Runtime.executionContextCreated for Playwright utility world",
        |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("__playwright_utility_world_page")
        },
    );
    assert_eq!(created["method"], "Runtime.executionContextCreated");
    assert_eq!(
        created["params"]["context"]["name"],
        json!("__playwright_utility_world_page")
    );
    assert_eq!(
        created["params"]["context"]["auxData"]["grantUniversalAccess"],
        json!(true)
    );

    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] == json!("Page.lifecycleEvent")),
        "expected only initial-document lifecycle replay events, got {:?}",
        ctx.sent
    );
    ctx.sent.clear();
}
