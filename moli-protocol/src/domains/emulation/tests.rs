use crate::conn::{
    BrowserContext, CdpCommandTaskStep, EmulatedGeolocationOverride,
    EmulatedGeolocationOverrideState, PageTargetHost, PendingCdpCommandDispatch,
    TargetIdentityState, TargetPageSlot,
};
use crate::devtools_runtime::{
    DevToolsBrowserContextId, DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult,
    DevToolsProtocol, DevToolsSessionId, DevToolsSetExtraHeadersCommand,
    DevToolsSetLocaleOverrideCommand, DevToolsSetUserAgentOverrideCommand, DevToolsTargetId,
};
use crate::testing::{TestContext, wait_until_message};
use axum::{Router, extract::State, http::HeaderMap, response::IntoResponse, routing::get};
use parking_lot::Mutex;
use serde_json::json;
use std::sync::Arc;
use tokio::{
    net::TcpListener,
    sync::Notify,
    time::{Duration, timeout},
};

async fn complete_pending_command_task_for_test(
    ctx: &mut TestContext,
    pending: PendingCdpCommandDispatch,
) -> Vec<serde_json::Value> {
    ctx.complete_command_task_step_for_test(CdpCommandTaskStep::Pending(Box::new(pending)))
        .await
        .0
}

async fn loaded_page_html_for_test(ctx: &mut TestContext) -> String {
    let page = ctx
        .conn
        .browser_context
        .as_mut()
        .and_then(|bc| bc.active_page_target_mut().runtime_slot.loaded_page_mut())
        .expect("loaded page");
    page.serialize_html_async()
        .await
        .expect("loaded page should serialize HTML")
}

async fn load_session_page_for_pending_emulation_test(ctx: &mut TestContext) {
    load_session_page_for_pending_emulation_test_at_url(
        ctx,
        "data:text/html,<body>emulation</body>",
    )
    .await;
}

async fn load_session_page_for_pending_emulation_test_at_url(ctx: &mut TestContext, url: &str) {
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    install_session_page_for_emulation_test(ctx, bc, url).await;
}

fn install_multi_session_page_state(ctx: &mut TestContext) {
    let mut browser_context = BrowserContext::new("BID-1".into());
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-primary");
    assert!(browser_context.assign_attached_session_to_target("TID-1", "SID-attached".to_owned()));
    ctx.conn.browser_context = Some(browser_context);
    ctx.conn.register_bound_session_for_test("SID-primary");
    ctx.conn.register_bound_session_for_test("SID-attached");
}

async fn expect_session_command_result(
    ctx: &mut TestContext,
    id: u64,
    session_id: &str,
    method: &str,
    params: serde_json::Value,
) {
    ctx.process_async(json!({
        "id": id,
        "method": method,
        "sessionId": session_id,
        "params": params,
    }))
    .await;
    ctx.expect_result(id, json!({}), Some(session_id));
}

async fn expect_session_command_error(
    ctx: &mut TestContext,
    id: u64,
    session_id: &str,
    method: &str,
    params: serde_json::Value,
    message: &str,
) {
    ctx.process_async(json!({
        "id": id,
        "method": method,
        "sessionId": session_id,
        "params": params,
    }))
    .await;
    ctx.expect_error(id, -32000, message);
}

async fn install_session_page_for_emulation_test(
    ctx: &mut TestContext,
    bc: BrowserContext,
    url: &str,
) {
    ctx.conn.browser_context = Some(bc);
    // A production navigation binds the reserved renderer Page to its target
    // before output can arrive. Use the production-shaped fixture transaction
    // instead of inserting a bare Page and racing its first publication.
    ctx.install_navigation_fixture_for_session_owner(url, Some("SID-1"))
        .await;
}

fn bidi_command_context() -> DevToolsCommandContext {
    DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    }
}

async fn execute_set_extra_headers_for_test(
    ctx: &mut TestContext,
    target_ids: Vec<&str>,
    browser_context_ids: Vec<&str>,
    headers: Vec<(&str, &str)>,
) {
    let outcome = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::SetExtraHeaders(
            DevToolsSetExtraHeadersCommand {
                context: bidi_command_context(),
                target_ids: target_ids.into_iter().map(DevToolsTargetId::from).collect(),
                browser_context_ids: browser_context_ids
                    .into_iter()
                    .map(DevToolsBrowserContextId::from)
                    .collect(),
                headers: headers
                    .into_iter()
                    .map(|(name, value)| (name.to_owned(), value.to_owned()))
                    .collect(),
            },
        ))
        .await;
    let (result, events, protocol_events, renderer_output_predecessor) =
        outcome.into_complete_parts();
    assert!(events.is_empty());
    assert!(protocol_events.is_empty());
    assert!(renderer_output_predecessor.is_none());
    assert!(matches!(result, Ok(DevToolsCommandResult::Empty)));
}

#[tokio::test(flavor = "multi_thread")]
async fn bidi_set_extra_headers_merges_global_user_context_and_context_layers() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    ctx.conn.browser_context = Some(bc);

    execute_set_extra_headers_for_test(
        &mut ctx,
        Vec::new(),
        Vec::new(),
        vec![("some_header_name", "global"), ("global_header", "1")],
    )
    .await;
    let future_context = ctx.conn.new_browser_context("BID-future".to_owned());
    assert_eq!(
        future_context.effective_extra_headers(),
        vec![
            ("some_header_name".to_owned(), "global".to_owned()),
            ("global_header".to_owned(), "1".to_owned())
        ]
    );

    execute_set_extra_headers_for_test(
        &mut ctx,
        Vec::new(),
        vec!["BID-1"],
        vec![("some_header_name", "user"), ("user_context_header", "1")],
    )
    .await;
    execute_set_extra_headers_for_test(
        &mut ctx,
        vec!["TID-1"],
        Vec::new(),
        vec![("some_header_name", "context"), ("context_header", "1")],
    )
    .await;

    let headers = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context")
        .effective_extra_headers();
    assert_eq!(
        headers,
        vec![
            ("global_header".to_owned(), "1".to_owned()),
            ("user_context_header".to_owned(), "1".to_owned()),
            ("some_header_name".to_owned(), "context".to_owned()),
            ("context_header".to_owned(), "1".to_owned())
        ]
    );

    execute_set_extra_headers_for_test(&mut ctx, vec!["TID-1"], Vec::new(), Vec::new()).await;
    let headers = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("active browser context")
        .effective_extra_headers();
    assert_eq!(
        headers,
        vec![
            ("global_header".to_owned(), "1".to_owned()),
            ("some_header_name".to_owned(), "user".to_owned()),
            ("user_context_header".to_owned(), "1".to_owned())
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn script_execution_disabled_completes_through_io_pending_dispatch() {
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test(&mut ctx).await;

    let raw = json!({
        "id": 9101,
        "sessionId": "SID-1",
        "method": "Emulation.setScriptExecutionDisabled",
        "params": { "value": true }
    })
    .to_string();
    let response_start = ctx.sent.len();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("the script execution override should use IO pending dispatch");
    let mut messages = complete_pending_command_task_for_test(&mut ctx, pending).await;
    if !messages.iter().any(|message| message["id"] == json!(9101)) {
        ctx.wait_for_test_command_response(9101, response_start)
            .await;
        messages.push(ctx.take_response_by_id(9101));
    }

    assert!(messages.iter().any(|message| {
        message["id"] == json!(9101)
            && message["sessionId"] == json!("SID-1")
            && message["result"] == json!({})
    }));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .script_execution_disabled
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attached_session_first_io_emulation_response_uses_its_session_host() {
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test(&mut ctx).await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );
    let owner = crate::conn::CommandOwnerScope::for_route(
        ctx.conn
            .bound_session_route_for_test("SID-attached", Some("TID-1"))
            .expect("attached test session must own its Page route"),
    );
    ctx.conn
        .apply_runtime_binding_state_for_owner_async(&owner)
        .await
        .expect("target attachment should establish the attached renderer session");

    ctx.process_async(json!({
        "id": 9_111,
        "sessionId": "SID-attached",
        "method": "Emulation.setScriptExecutionDisabled",
        "params": { "value": true }
    }))
    .await;

    ctx.expect_result(9_111, json!({}), Some("SID-attached"));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .script_execution_disabled
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn timezone_override_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test(&mut ctx).await;

    let raw = json!({
        "id": 9102,
        "sessionId": "SID-1",
        "method": "Emulation.setTimezoneOverride",
        "params": { "timezoneId": "Asia/Shanghai" }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("timezone override should use pending command dispatch");
    let messages = complete_pending_command_task_for_test(&mut ctx, pending).await;

    assert!(messages.iter().any(|message| {
        message["id"] == json!(9102)
            && message["sessionId"] == json!("SID-1")
            && message["result"] == json!({})
    }));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .effective_policy()
            .timezone_override(),
        Some("Asia/Shanghai")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn emulated_media_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test(&mut ctx).await;

    let raw = json!({
        "id": 9103,
        "sessionId": "SID-1",
        "method": "Emulation.setEmulatedMedia",
        "params": {
            "media": "screen",
            "features": [
                { "name": "prefers-color-scheme", "value": "dark" }
            ]
        }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("emulated media should use pending command dispatch");
    let messages = complete_pending_command_task_for_test(&mut ctx, pending).await;

    assert!(messages.iter().any(|message| {
        message["id"] == json!(9103)
            && message["sessionId"] == json!("SID-1")
            && message["result"] == json!({})
    }));
    let media = &ctx
        .conn
        .browser_context
        .as_ref()
        .unwrap()
        .active_page_target()
        .emulated_media;
    assert_eq!(media.media.as_deref(), Some("screen"));
    assert_eq!(media.color_scheme.as_deref(), Some("dark"));
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_emulation_completion_follows_the_exact_target_across_activation_and_navigation() {
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test(&mut ctx).await;
    let dispatched_attachment_id = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|browser_context| browser_context.page_target("TID-1"))
        .and_then(PageTargetHost::loaded_page)
        .and_then(moli_core::page::Page::renderer_agent_attachment_id)
        .expect("the original target should have a renderer attachment");
    let target = super::PendingEmulationPageTarget::BrowserContextTarget {
        browser_context_id: "BID-1".to_owned(),
        target_id: "TID-1".to_owned(),
    };

    let browser_context = ctx.conn.browser_context.as_mut().unwrap();
    assert!(
        browser_context.insert_page_target_host(PageTargetHost::with_url(
            "TID-2".to_owned(),
            None,
            "about:blank".to_owned(),
        ))
    );
    browser_context.set_active_target_id("TID-2");
    assert!(
        !super::pending_emulation_page_configuration_will_be_replayed(
            &ctx.conn,
            &target,
            &super::PendingEmulationPageOperation::SetTimezoneOverride,
            Some(dispatched_attachment_id),
        ),
        "changing foreground selection must not make an error from the same Page look stale"
    );

    ctx.process_async(json!({
        "id": 9_104,
        "sessionId": "SID-1",
        "method": "Page.navigate",
        "params": { "url": "data:text/html,<body>replacement</body>" }
    }))
    .await;
    assert!(ctx.take_response_by_id(9_104)["result"]["loaderId"].is_string());
    assert!(
        super::pending_emulation_page_configuration_will_be_replayed(
            &ctx.conn,
            &target,
            &super::PendingEmulationPageOperation::SetTimezoneOverride,
            Some(dispatched_attachment_id),
        ),
        "only replacement of the exact target attachment may retire its renderer error"
    );
    assert!(
        !super::pending_emulation_page_configuration_will_be_replayed(
            &ctx.conn,
            &target,
            &super::PendingEmulationPageOperation::SetIdleOverride,
            Some(dispatched_attachment_id),
        ),
        "frame-host idle state must not use the target-policy replay path",
    );

    let result = super::complete_pending_devtools_emulation_command(
        &mut ctx.conn,
        super::CompletedEmulationCommandDispatch {
            command_id: None,
            session_id: Some("SID-1".to_owned()),
            completed: super::CompletedEmulationRendererDispatch::Pages(vec![
                super::CompletedEmulationPageCommand {
                    target,
                    operation: super::PendingEmulationPageOperation::SetTimezoneOverride,
                    dispatched_attachment_id: Some(dispatched_attachment_id),
                    completed: Err("renderer attachment retired".to_owned()),
                },
            ]),
        },
    )
    .expect("the stored target policy should be replayed into the replacement document");
    assert!(matches!(result, DevToolsCommandResult::Empty));
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_idle_override_response_does_not_replay_into_replacement_page() {
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test(&mut ctx).await;

    let raw = json!({
        "id": 9_105,
        "sessionId": "SID-1",
        "method": "Emulation.setIdleOverride",
        "params": { "isUserActive": false, "isScreenUnlocked": false }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("the loaded Page should receive the idle override command");
    };
    let completed = pending.wait().await;

    ctx.process_async(json!({
        "id": 9_106,
        "sessionId": "SID-1",
        "method": "Page.navigate",
        "params": { "url": "data:text/html,<body>replacement</body>" }
    }))
    .await;
    assert!(ctx.take_response_by_id(9_106)["result"]["loaderId"].is_string());

    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!("the retired idle override should settle in one protocol phase");
    };
    assert!(outcome.into_parts().0.iter().any(|message| {
        message["id"] == json!(9_105)
            && message["sessionId"] == json!("SID-1")
            && message["result"] == json!({})
    }));
    let page = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|context| context.page_target("TID-1"))
        .and_then(PageTargetHost::loaded_page)
        .expect("replacement Page");
    assert_eq!(
        page.idle_override(),
        None,
        "a settled command on the retired frame host must not become target-level policy",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn admitted_idle_override_is_visible_to_concurrent_same_site_navigation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/",
                get(|| async { "<!doctype html><title>idle navigation</title>" }),
            ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test_at_url(
        &mut ctx,
        &format!("http://{address}/?initial"),
    )
    .await;

    let raw = json!({
        "id": 9_107,
        "sessionId": "SID-1",
        "method": "Emulation.setIdleOverride",
        "params": { "isUserActive": false, "isScreenUnlocked": false }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("the loaded Page should receive the idle override command");
    };
    let completed = pending.wait().await;

    ctx.process_async(json!({
        "id": 9_108,
        "sessionId": "SID-1",
        "method": "Page.navigate",
        "params": { "url": format!("http://{address}/?replacement") }
    }))
    .await;
    assert!(ctx.take_response_by_id(9_108)["result"]["loaderId"].is_string());

    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!("the retired idle override should settle in one protocol phase");
    };
    assert!(outcome.into_parts().0.iter().any(|message| {
        message["id"] == json!(9_107)
            && message["sessionId"] == json!("SID-1")
            && message["result"] == json!({})
    }));
    let page = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|context| context.page_target("TID-1"))
        .and_then(PageTargetHost::loaded_page)
        .expect("replacement Page");
    assert_eq!(
        page.idle_override(),
        Some(moli_core::page::EmulatedIdleOverride {
            is_user_active: false,
            is_screen_unlocked: false,
        }),
        "same-site commit must read admitted state from the outgoing document handle",
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn idle_override_updates_idle_detector_and_clear_restores_actual_state() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/",
                get(|| async { "<!doctype html><title>idle detector</title>" }),
            ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test_at_url(&mut ctx, &format!("http://{address}/"))
        .await;

    {
        let page = ctx
            .conn
            .browser_context
            .as_mut()
            .and_then(|context| {
                context
                    .active_page_target_mut()
                    .runtime_slot
                    .loaded_page_mut()
            })
            .expect("loaded page");
        page.set_permission_overrides_async(&[moli_core::page::PermissionOverrideRegistration {
            permission: json!("idleDetection"),
            setting: "granted".to_owned(),
            origin: None,
            embedded_origin: None,
        }])
        .await
        .expect("idle detection permission should reach the renderer");
        assert_eq!(
            page.evaluate_runtime_expression_async(
                "globalThis.idleEvents=[];globalThis.idleDetector=new IdleDetector();idleDetector.addEventListener('change',()=>idleEvents.push(idleDetector.userState+'/'+idleDetector.screenState));idleDetector.start();JSON.stringify([idleDetector.userState,idleDetector.screenState,idleEvents])"
            )
            .await
            .expect("IdleDetector should start"),
            json!({
                "type": "string",
                "value": r#"["active","unlocked",["active/unlocked"]]"#
            })
        );
    }

    let set_raw = json!({
        "id": 9104,
        "sessionId": "SID-1",
        "method": "Emulation.setIdleOverride",
        "params": { "isUserActive": false, "isScreenUnlocked": false }
    })
    .to_string();
    let set_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&set_raw)
        .expect("idle override should use pending command dispatch");
    let set_messages = complete_pending_command_task_for_test(&mut ctx, set_pending).await;
    assert!(set_messages.iter().any(|message| {
        message["id"] == json!(9104)
            && message["sessionId"] == json!("SID-1")
            && message["result"] == json!({})
    }));

    {
        let page = ctx
            .conn
            .browser_context
            .as_mut()
            .and_then(|context| {
                context
                    .active_page_target_mut()
                    .runtime_slot
                    .loaded_page_mut()
            })
            .expect("loaded page");
        assert_eq!(
            page.evaluate_runtime_expression_async(
                "JSON.stringify([idleDetector.userState,idleDetector.screenState,idleEvents])"
            )
            .await
            .expect("overridden IdleDetector state should evaluate"),
            json!({
                "type": "string",
                "value": r#"["idle","locked",["active/unlocked","idle/locked"]]"#
            })
        );
    }

    let clear_raw = json!({
        "id": 9105,
        "sessionId": "SID-1",
        "method": "Emulation.clearIdleOverride",
        "params": {}
    })
    .to_string();
    let clear_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&clear_raw)
        .expect("clearing idle override should use pending command dispatch");
    let clear_messages = complete_pending_command_task_for_test(&mut ctx, clear_pending).await;
    assert!(clear_messages.iter().any(|message| {
        message["id"] == json!(9105)
            && message["sessionId"] == json!("SID-1")
            && message["result"] == json!({})
    }));
    let page = ctx
        .conn
        .browser_context
        .as_mut()
        .and_then(|context| {
            context
                .active_page_target_mut()
                .runtime_slot
                .loaded_page_mut()
        })
        .expect("loaded page");
    assert_eq!(
        page.evaluate_runtime_expression_async(
            "JSON.stringify([idleDetector.userState,idleDetector.screenState,idleEvents])"
        )
        .await
        .expect("cleared IdleDetector state should evaluate"),
        json!({
            "type": "string",
            "value": r#"["active","unlocked",["active/unlocked","idle/locked","active/unlocked"]]"#
        })
    );

    let set_raw = json!({
        "id": 9106,
        "sessionId": "SID-1",
        "method": "Emulation.setIdleOverride",
        "params": { "isUserActive": false, "isScreenUnlocked": false }
    })
    .to_string();
    let set_pending = ctx
        .conn
        .try_start_pending_command_dispatch(&set_raw)
        .expect("idle override should use pending command dispatch");
    let set_messages = complete_pending_command_task_for_test(&mut ctx, set_pending).await;
    assert!(set_messages.iter().any(|message| {
        message["id"] == json!(9106)
            && message["sessionId"] == json!("SID-1")
            && message["result"] == json!({})
    }));

    ctx.conn
        .start_document_navigation_for_owner(
            &crate::conn::CommandOwnerScope::for_session("SID-1"),
            "LID-idle-cross-document".to_owned(),
        )
        .expect("cross-Document navigation should enter the pending state");
    let configuration = ctx
        .conn
        .prepared_document_commit_configuration_for_owner(
            &crate::conn::CommandOwnerScope::for_session("SID-1"),
            &url::Url::parse("http://127.0.0.1:65530/same-site-different-origin").unwrap(),
        )
        .expect("commit configuration should resolve the target resource runtime");
    assert_eq!(
        configuration.idle_override,
        Some(moli_core::page::EmulatedIdleOverride {
            is_user_active: false,
            is_screen_unlocked: false,
        })
    );
    let cross_site_configuration = ctx
        .conn
        .prepared_document_commit_configuration_for_owner(
            &crate::conn::CommandOwnerScope::for_session("SID-1"),
            &url::Url::parse("http://idle-override-cross-site.test/").unwrap(),
        )
        .expect("cross-site commit configuration should resolve the target resource runtime");
    assert_eq!(
        cross_site_configuration.idle_override, None,
        "a cross-site renderer replacement must not inherit frame-host idle state",
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn pure_state_emulation_commands_complete_through_command_dispatch() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    for (id, method, params) in [
        (
            9111,
            "Emulation.setFocusEmulationEnabled",
            json!({ "enabled": true }),
        ),
        (
            9112,
            "Emulation.setTouchEmulationEnabled",
            json!({ "enabled": true }),
        ),
        (
            9113,
            "Emulation.setEmitTouchEventsForMouse",
            json!({ "enabled": true, "configuration": "mobile" }),
        ),
    ] {
        let raw = json!({
            "id": id,
            "sessionId": "SID-1",
            "method": method,
            "params": params
        })
        .to_string();
        let CdpCommandTaskStep::Complete(outcome) = ctx.conn.start_command_dispatch(&raw) else {
            panic!("pure emulation command should complete without renderer wait");
        };
        let messages = outcome.into_parts().0;
        assert!(messages.iter().any(|message| {
            message["id"] == json!(id)
                && message["sessionId"] == json!("SID-1")
                && message["result"] == json!({})
        }));
    }

    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    assert!(browser_context.active_page_target().focus_emulation_enabled);
    assert!(browser_context.active_page_target().touch_emulation_enabled);
    assert!(
        browser_context
            .active_page_target()
            .emit_touch_events_for_mouse
    );
    assert_eq!(
        browser_context.active_page_target().cpu_throttling_rate,
        1.0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_cpu_throttling_rate_rejects_invalid_params() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 9115,
        "sessionId": "SID-1",
        "method": "Emulation.setCPUThrottlingRate",
        "params": {}
    }))
    .await;
    ctx.expect_error(9115, -32602, "InvalidParams");

    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .cpu_throttling_rate,
        1.0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn live_apply_emulation_commands_without_loaded_page_do_not_use_legacy_fallback() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    for (id, method, params) in [
        (
            9121,
            "Emulation.setDeviceMetricsOverride",
            json!({ "width": 800, "height": 600, "deviceScaleFactor": 1, "mobile": false }),
        ),
        (9122, "Emulation.clearDeviceMetricsOverride", json!({})),
        (
            9123,
            "Emulation.setEmulatedMedia",
            json!({ "media": "screen" }),
        ),
        (
            9124,
            "Emulation.setTimezoneOverride",
            json!({ "timezoneId": "Asia/Shanghai" }),
        ),
        (
            9125,
            "Emulation.setScriptExecutionDisabled",
            json!({ "value": true }),
        ),
        (
            9126,
            "Emulation.setGeolocationOverride",
            json!({ "latitude": 48.85837, "longitude": 2.294481, "accuracy": 7 }),
        ),
        (
            9128,
            "Emulation.setCPUThrottlingRate",
            json!({ "rate": 2.5 }),
        ),
    ] {
        let raw = json!({
            "id": id,
            "sessionId": "SID-1",
            "method": method,
            "params": params
        })
        .to_string();
        let CdpCommandTaskStep::Complete(outcome) = ctx.conn.start_command_dispatch(&raw) else {
            panic!("{method} should not wait without a loaded page");
        };
        let messages = outcome.into_parts().0;
        assert!(messages.iter().any(|message| {
            message["id"] == json!(id)
                && message["sessionId"] == json!("SID-1")
                && message["result"] == json!({})
        }));
    }

    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .cpu_throttling_rate,
        2.5
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn live_geolocation_override_uses_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    install_session_page_for_emulation_test(&mut ctx, bc, "data:text/html,<body>geo</body>").await;

    let raw = json!({
        "id": 9127,
        "sessionId": "SID-1",
        "method": "Emulation.setGeolocationOverride",
        "params": { "latitude": 48.85837, "longitude": 2.294481, "accuracy": 7 }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("loaded Emulation.setGeolocationOverride should update the live page");
    };
    let completed = pending.wait().await;
    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!("geolocation override should complete in one renderer phase");
    };
    let messages = outcome.into_parts().0;
    assert!(messages.iter().any(|message| {
        message["id"] == json!(9127)
            && message["sessionId"] == json!("SID-1")
            && message["result"] == json!({})
    }));
}

#[tokio::test(flavor = "multi_thread")]
async fn device_metrics_completion_survives_initial_page_replacement() {
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test_at_url(
        &mut ctx,
        "data:text/html,<body>initial</body>",
    )
    .await;

    let raw = json!({
        "id": 9128,
        "sessionId": "SID-1",
        "method": "Emulation.setDeviceMetricsOverride",
        "params": {
            "width": 640,
            "height": 360,
            "deviceScaleFactor": 2,
            "mobile": false
        }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("loaded Emulation.setDeviceMetricsOverride should update the renderer Page");
    };
    let completed = pending.wait().await;

    ctx.process_async(json!({
        "id": 9129,
        "sessionId": "SID-1",
        "method": "Page.navigate",
        "params": { "url": "data:text/html,<body>replacement</body>" }
    }))
    .await;
    let navigate = ctx.take_response_by_id(9129);
    assert_eq!(navigate["result"]["frameId"], json!("TID-1"));
    assert!(navigate["result"]["loaderId"].is_string());

    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!("device metrics completion should settle after Page replacement");
    };
    let messages = outcome.into_parts().0;
    assert!(messages.iter().any(|message| {
        message["id"] == json!(9128)
            && message["sessionId"] == json!("SID-1")
            && message["result"] == json!({})
    }));

    ctx.process_async(json!({
        "id": 9130,
        "sessionId": "SID-1",
        "method": "Runtime.evaluate",
        "params": {
            "expression": "JSON.stringify([innerWidth, innerHeight, devicePixelRatio])",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        ctx.take_response_by_id(9130)["result"]["result"]["value"],
        json!("[640,360,2]")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn live_cpu_throttling_rate_uses_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test(&mut ctx).await;

    let raw = json!({
        "id": 9129,
        "sessionId": "SID-1",
        "method": "Emulation.setCPUThrottlingRate",
        "params": { "rate": 3.0 }
    })
    .to_string();
    let CdpCommandTaskStep::Pending(pending) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("loaded Emulation.setCPUThrottlingRate should update the live renderer page");
    };
    let completed = pending.wait().await;
    let CdpCommandTaskStep::Complete(outcome) =
        ctx.conn.complete_pending_command_dispatch(completed).await
    else {
        panic!("CPU throttling rate should complete in one renderer phase");
    };
    let messages = outcome.into_parts().0;
    assert!(messages.iter().any(|message| {
        message["id"] == json!(9129)
            && message["sessionId"] == json!("SID-1")
            && message["result"] == json!({})
    }));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .cpu_throttling_rate,
        3.0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_timezone_override_without_loaded_browser_context_errors() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 7,
        "method": "Emulation.setTimezoneOverride",
        "params": { "timezoneId": "Asia/Shanghai" }
    }))
    .await;
    ctx.expect_error(7, -31998, "BrowserContextNotLoaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_session_locale_and_timezone_claims_match_chromium() {
    const LOCALE: &str = "Emulation.setLocaleOverride";
    const TIMEZONE: &str = "Emulation.setTimezoneOverride";
    let mut ctx = TestContext::new();
    install_multi_session_page_state(&mut ctx);

    expect_session_command_result(
        &mut ctx,
        70_001,
        "SID-primary",
        LOCALE,
        json!({
            "locale": "fr-FR"
        }),
    )
    .await;
    expect_session_command_error(
        &mut ctx,
        70_002,
        "SID-attached",
        LOCALE,
        json!({ "locale": "de-DE" }),
        "Another locale override is already in effect",
    )
    .await;
    expect_session_command_result(
        &mut ctx,
        70_003,
        "SID-primary",
        TIMEZONE,
        json!({
            "timezoneId": "Europe/Paris"
        }),
    )
    .await;
    expect_session_command_error(
        &mut ctx,
        70_004,
        "SID-attached",
        TIMEZONE,
        json!({ "timezoneId": "America/New_York" }),
        "Timezone override is already in effect",
    )
    .await;
    expect_session_command_result(
        &mut ctx,
        70_005,
        "SID-attached",
        TIMEZONE,
        json!({ "timezoneId": "" }),
    )
    .await;

    let page_state = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("browser context")
        .active_page_target();
    assert_eq!(
        page_state.effective_policy().locale_override(),
        Some("fr-FR")
    );
    assert_eq!(
        page_state.effective_policy().timezone_override(),
        Some("Europe/Paris")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_session_browser_identity_uses_attachment_order_and_field_contributions() {
    const EMULATION_UA: &str = "Emulation.setUserAgentOverride";
    const NETWORK_UA: &str = "Network.setUserAgentOverride";
    let mut ctx = TestContext::new();
    install_multi_session_page_state(&mut ctx);

    for (id, method, session_id, user_agent) in [
        (71_001, EMULATION_UA, "SID-attached", "Moli/Aux-1"),
        (71_002, NETWORK_UA, "SID-primary", "Moli/Primary-1"),
        (71_003, EMULATION_UA, "SID-primary", "Moli/Primary-2"),
    ] {
        expect_session_command_result(
            &mut ctx,
            id,
            session_id,
            method,
            json!({ "userAgent": user_agent }),
        )
        .await;
    }
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .effective_policy()
            .browser_identity_override()
            .map(|identity| identity.user_agent()),
        Some("Moli/Aux-1")
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .effective_renderer_browser_identity_override_owned()
            .expect("renderer identity")
            .user_agent(),
        "Moli/Aux-1",
        "renderer agents use attachment order rather than setter order"
    );

    expect_session_command_result(
        &mut ctx,
        71_004,
        "SID-attached",
        NETWORK_UA,
        json!({ "userAgent": "" }),
    )
    .await;
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .effective_policy()
            .browser_identity_override()
            .map(|identity| identity.user_agent()),
        Some("Moli/Primary-2")
    );

    expect_session_command_result(
        &mut ctx,
        71_005,
        "SID-attached",
        EMULATION_UA,
        json!({
            "userAgent": "",
            "acceptLanguage": "fr-FR",
            "platform": "AuxPlatform"
        }),
    )
    .await;
    let effective_policy = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("browser context")
        .active_page_target()
        .effective_policy();
    let identity = effective_policy
        .browser_identity_override()
        .expect("UA and per-field contributions should compose an identity");
    assert_eq!(identity.user_agent(), "Moli/Primary-2");
    assert_eq!(identity.accept_language(), "fr-FR");
    assert_eq!(identity.navigator_platform(), "AuxPlatform");

    expect_session_command_result(
        &mut ctx,
        71_006,
        "SID-attached",
        NETWORK_UA,
        json!({ "userAgent": "Moli/Aux-2" }),
    )
    .await;
    expect_session_command_result(
        &mut ctx,
        71_007,
        "SID-attached",
        "Network.disable",
        json!({}),
    )
    .await;
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .effective_policy()
            .browser_identity_override()
            .map(|identity| identity.user_agent()),
        Some("Moli/Aux-2"),
        "Network.disable must not dispose the shared Emulation agent state"
    );

    ctx.conn
        .clear_devtools_emulation_session_policy_async("SID-attached")
        .await
        .expect("detaching the attached session should restore the previous UA");
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .effective_policy()
            .browser_identity_override()
            .map(|identity| identity.user_agent()),
        Some("Moli/Primary-2")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn async_emulation_device_state_updates_browser_context() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new_with_page_for_test("BID-1", "TID-1"));

    ctx.process_async(json!({
        "id": 501,
        "method": "Emulation.setFocusEmulationEnabled",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(501, json!({}), None);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .focus_emulation_enabled
    );

    ctx.process_async(json!({
        "id": 502,
        "method": "Emulation.setTouchEmulationEnabled",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(502, json!({}), None);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .touch_emulation_enabled
    );

    ctx.process_async(json!({
        "id": 505,
        "method": "Emulation.setEmitTouchEventsForMouse",
        "params": { "enabled": true, "configuration": "desktop" }
    }))
    .await;
    ctx.expect_result(505, json!({}), None);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .emit_touch_events_for_mouse
    );

    ctx.process_async(json!({
        "id": 506,
        "method": "Emulation.setGeolocationOverride",
        "params": { "latitude": 37.33182, "longitude": -122.03118, "accuracy": 10 }
    }))
    .await;
    ctx.expect_result(506, json!({}), None);
    let geolocation = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.active_page_target().geolocation_override.as_ref())
        .and_then(EmulatedGeolocationOverrideState::position)
        .expect("geolocation override should be set");
    assert_eq!(geolocation.latitude, 37.33182);
    assert_eq!(geolocation.longitude, -122.03118);
    assert_eq!(geolocation.accuracy, 10.0);

    ctx.process_async(json!({
        "id": 503,
        "method": "Emulation.setDeviceMetricsOverride",
        "params": { "width": 800, "height": 600, "deviceScaleFactor": 1, "mobile": false }
    }))
    .await;
    ctx.expect_result(503, json!({}), None);
    let metrics = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.active_page_target().emulated_device_metrics.as_ref())
        .expect("device metrics should be set");
    assert_eq!(metrics.width, 800);
    assert_eq!(metrics.height, 600);

    ctx.process_async(json!({
        "id": 504,
        "method": "Emulation.clearDeviceMetricsOverride",
        "params": {}
    }))
    .await;
    ctx.expect_result(504, json!({}), None);
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .emulated_device_metrics
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_emit_touch_events_for_mouse_rejects_invalid_params() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 91,
        "method": "Emulation.setEmitTouchEventsForMouse",
        "params": { "configuration": "mobile" }
    }))
    .await;
    ctx.expect_error(91, -32602, "InvalidParams");

    ctx.process_async(json!({
        "id": 92,
        "method": "Emulation.setEmitTouchEventsForMouse",
        "params": { "enabled": true, "configuration": "tablet" }
    }))
    .await;
    ctx.expect_error(92, -32602, "InvalidParams");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_user_agent_override_applies_to_subsequent_navigation_requests() {
    async fn handler(
        State(seen): State<Arc<Mutex<Option<String>>>>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let user_agent = headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        *seen.lock() = user_agent;
        "<!doctype html><html><body>ok</body></html>"
    }

    let seen = Arc::new(Mutex::new(None));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 6,
        "method": "Emulation.setUserAgentOverride",
        "params": { "userAgent": "moli-cdp-test" }
    }))
    .await;
    ctx.expect_result(6, json!({}), None);

    ctx.process_async(json!({
        "id": 7,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    let _ = ctx.take_all();
    assert_eq!(seen.lock().as_deref(), Some("moli-cdp-test"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_user_agent_override_rejects_invalid_params() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 8,
        "method": "Emulation.setUserAgentOverride",
        "params": {}
    }))
    .await;
    ctx.expect_error(8, -32602, "InvalidParams");
}

#[tokio::test(flavor = "multi_thread")]
async fn emulation_user_agent_override_replaces_network_override() {
    async fn handler(
        State(seen): State<Arc<Mutex<Option<String>>>>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let user_agent = headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        *seen.lock() = user_agent;
        "<!doctype html><html><body>ok</body></html>"
    }

    let seen = Arc::new(Mutex::new(None));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 9,
        "method": "Network.setUserAgentOverride",
        "params": { "userAgent": "moli-network-first" }
    }))
    .await;
    ctx.expect_result(9, json!({}), None);

    ctx.process_async(json!({
        "id": 10,
        "method": "Emulation.setUserAgentOverride",
        "params": { "userAgent": "moli-emulation-final" }
    }))
    .await;
    ctx.expect_result(10, json!({}), None);

    ctx.process_async(json!({
        "id": 11,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    let _ = ctx.take_all();
    assert_eq!(seen.lock().as_deref(), Some("moli-emulation-final"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_user_agent_override_applies_to_current_page_xhr_requests() {
    async fn handler(
        State((seen, seen_notify)): State<(Arc<Mutex<Option<String>>>, Arc<Notify>)>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let user_agent = headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        *seen.lock() = user_agent;
        seen_notify.notify_one();
        "ok"
    }

    let seen = Arc::new(Mutex::new(None));
    let seen_notify = Arc::new(Notify::new());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server_seen_notify = Arc::clone(&seen_notify);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/xhr", get(handler))
                .with_state((server_seen, server_seen_notify)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>ok</body>",
        Some("SID-1"),
    )
    .await;

    ctx.process_async(json!({
        "id": 12,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 13,
        "method": "Emulation.setUserAgentOverride",
        "sessionId": "SID-1",
        "params": { "userAgent": "moli-emulation-live-ua" }
    }))
    .await;
    ctx.expect_result(13, json!({}), Some("SID-1"));

    ctx.process_async(json!({
            "id": 14,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "awaitPromise": true,
                "expression": format!(
                    "(async () => {{ const xhr = new XMLHttpRequest(); await new Promise((resolve, reject) => {{ xhr.addEventListener('load', resolve, {{ once: true }}); xhr.addEventListener('error', () => reject(new Error('xhr failed')), {{ once: true }}); xhr.open('GET', 'http://{addr}/xhr'); xhr.send(); }}); return xhr.responseText; }})()"
                )
            }
        })).await;
    let _ = ctx.take_all();

    timeout(Duration::from_secs(1), seen_notify.notified())
        .await
        .expect("XHR handler should observe the live user agent override");
    assert_eq!(seen.lock().as_deref(), Some("moli-emulation-live-ua"));

    ctx.process_async(json!({
        "id": 15,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "navigator.userAgent" }
    }))
    .await;
    let response = ctx.take_response_by_id(15);
    assert_eq!(
        response["result"]["result"]["value"],
        json!("moli-emulation-live-ua")
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_user_agent_override_applies_complete_chromium_identity_profile() {
    async fn handler(
        State(seen): State<Arc<Mutex<Option<HeaderMap>>>>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        *seen.lock() = Some(headers);
        "<!doctype html><html><body>identity</body></html>"
    }

    let seen = Arc::new(Mutex::new(None));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let natural_identity = ctx.conn.base_browser_identity().clone();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 200,
        "method": "Emulation.setUserAgentOverride",
        "sessionId": "SID-1",
        "params": {
            "userAgent": "LinuxChrome/145",
            "acceptLanguage": "fr-CA,fr;q=0.9",
            "platform": "Linux x86_64",
            "userAgentMetadata": {
                "brands": [
                    { "brand": "Chromium", "version": "145" },
                    { "brand": "Not:A-Brand", "version": "99" }
                ],
                "fullVersionList": [
                    { "brand": "Chromium", "version": "145.0.7632.116" },
                    { "brand": "Not:A-Brand", "version": "99.0.0.0" }
                ],
                "fullVersion": "145.0.9000.1",
                "platform": "Linux",
                "platformVersion": "",
                "architecture": "x86",
                "model": "",
                "mobile": false,
                "bitness": "64",
                "wow64": false,
                "formFactors": ["Desktop"]
            }
        }
    }))
    .await;
    ctx.expect_result(200, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 201,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 202,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "awaitPromise": true,
            "returnByValue": true,
            "expression": r#"(async () => JSON.stringify({
                userAgent: navigator.userAgent,
                platform: navigator.platform,
                language: navigator.language,
                languages: navigator.languages,
                base: navigator.userAgentData.toJSON(),
                high: await navigator.userAgentData.getHighEntropyValues([
                    'architecture', 'bitness', 'formFactors', 'fullVersionList',
                    'platformVersion', 'uaFullVersion', 'wow64'
                ])
            }))()"#
        }
    }))
    .await;
    let response = ctx.take_response_by_id(202);
    let identity: serde_json::Value = serde_json::from_str(
        response["result"]["result"]["value"]
            .as_str()
            .expect("identity result should be JSON"),
    )
    .expect("identity result should parse");
    assert_eq!(identity["userAgent"], json!("LinuxChrome/145"));
    assert_eq!(identity["platform"], json!("Linux x86_64"));
    assert_eq!(identity["language"], json!("fr-CA"));
    assert_eq!(identity["languages"], json!(["fr-CA", "fr;q=0.9"]));
    assert_eq!(identity["base"]["platform"], json!("Linux"));
    assert_eq!(identity["base"]["brands"][0]["brand"], json!("Chromium"));
    assert_eq!(identity["high"]["architecture"], json!("x86"));
    assert_eq!(identity["high"]["bitness"], json!("64"));
    assert_eq!(identity["high"]["formFactors"], json!(["Desktop"]));
    assert_eq!(identity["high"]["uaFullVersion"], json!("145.0.9000.1"));

    let headers = seen
        .lock()
        .clone()
        .expect("server should observe navigation");
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    assert_eq!(header("user-agent").as_deref(), Some("LinuxChrome/145"));
    assert_eq!(header("accept-language").as_deref(), Some("fr-CA,fr;q=0.9"));
    assert_eq!(header("sec-ch-ua-platform").as_deref(), Some("\"Linux\""));
    assert_eq!(
        header("sec-ch-ua").as_deref(),
        Some("\"Chromium\";v=\"145\", \"Not:A-Brand\";v=\"99\"")
    );

    ctx.process_async(json!({
        "id": 203,
        "method": "Emulation.setUserAgentOverride",
        "sessionId": "SID-1",
        "params": { "userAgent": "CustomAgent/1.0" }
    }))
    .await;
    ctx.expect_result(203, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 204,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "returnByValue": true,
            "expression": "JSON.stringify({ platform: navigator.platform, languages: navigator.languages, uaData: navigator.userAgentData.toJSON() })"
        }
    }))
    .await;
    let response = ctx.take_response_by_id(204);
    let identity: serde_json::Value = serde_json::from_str(
        response["result"]["result"]["value"]
            .as_str()
            .expect("identity result should be JSON"),
    )
    .expect("identity result should parse");
    assert_eq!(identity["platform"], json!("Win32"));
    assert_eq!(identity["languages"], json!(["en-US", "en"]));
    assert_eq!(identity["uaData"]["brands"], json!([]));
    assert_eq!(identity["uaData"]["platform"], json!(""));

    ctx.process_async(json!({
        "id": 205,
        "method": "Emulation.setUserAgentOverride",
        "sessionId": "SID-1",
        "params": {
            "userAgent": "",
            "acceptLanguage": "",
            "platform": ""
        }
    }))
    .await;
    ctx.expect_result(205, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 206,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "returnByValue": true,
            "expression": "JSON.stringify({ userAgent: navigator.userAgent, platform: navigator.platform, languages: navigator.languages, uaData: navigator.userAgentData.toJSON() })"
        }
    }))
    .await;
    let response = ctx.take_response_by_id(206);
    let identity: serde_json::Value = serde_json::from_str(
        response["result"]["result"]["value"]
            .as_str()
            .expect("identity result should be JSON"),
    )
    .expect("identity result should parse");
    assert_eq!(identity["userAgent"], json!(natural_identity.user_agent()));
    assert_eq!(
        identity["platform"],
        json!(natural_identity.navigator_platform())
    );
    assert_eq!(identity["languages"], json!(natural_identity.languages()));
    assert_eq!(
        identity["uaData"]["platform"],
        json!(natural_identity.platform())
    );
    assert_eq!(
        identity["uaData"]["brands"][0]["brand"],
        json!(natural_identity.brands()[0].brand)
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_user_agent_override_rejects_chromium_invalid_identity_values() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 207,
        "method": "Emulation.setUserAgentOverride",
        "params": { "userAgent": "invalid\nagent" }
    }))
    .await;
    ctx.expect_error(207, -32602, "Invalid characters found in userAgent");

    ctx.process_async(json!({
        "id": 208,
        "method": "Emulation.setUserAgentOverride",
        "params": {
            "userAgent": "ValidAgent/1.0",
            "acceptLanguage": "en-US\rmalformed"
        }
    }))
    .await;
    ctx.expect_error(208, -32602, "Invalid characters found in acceptLanguage");

    ctx.process_async(json!({
        "id": 209,
        "method": "Emulation.setUserAgentOverride",
        "params": {
            "userAgent": "ValidAgent/1.0",
            "userAgentMetadata": {
                "brands": [{ "brand": "bad\u{001f}brand", "version": "1" }],
                "platform": "Linux",
                "platformVersion": "",
                "architecture": "x86",
                "model": "",
                "mobile": false
            }
        }
    }))
    .await;
    ctx.expect_error(209, -32602, "Invalid brand string");

    ctx.process_async(json!({
        "id": 210,
        "method": "Emulation.setUserAgentOverride",
        "params": {
            "userAgent": "",
            "userAgentMetadata": {
                "platform": "Linux",
                "platformVersion": "",
                "architecture": "x86",
                "model": "",
                "mobile": false
            }
        }
    }))
    .await;
    ctx.expect_error(
        210,
        -32602,
        "Empty userAgent invalid with userAgentMetadata provided",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn emulation_async_dispatch_updates_live_page_user_agent_and_xhr_header() {
    async fn handler(
        State((seen, seen_notify)): State<(Arc<Mutex<Option<String>>>, Arc<Notify>)>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let user_agent = headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        *seen.lock() = user_agent;
        seen_notify.notify_one();
        "ok"
    }

    let seen = Arc::new(Mutex::new(None));
    let seen_notify = Arc::new(Notify::new());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server_seen_notify = Arc::clone(&seen_notify);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/xhr", get(handler))
                .with_state((server_seen, server_seen_notify)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    install_session_page_for_emulation_test(&mut ctx, bc, "data:text/html,<body>ok</body>").await;

    ctx.process_async(json!({
        "id": 130,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 131,
        "method": "Emulation.setUserAgentOverride",
        "sessionId": "SID-1",
        "params": { "userAgent": "moli-emulation-async-ua" }
    }))
    .await;
    ctx.expect_result(131, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 132,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "awaitPromise": true,
            "expression": format!(
                "(async () => {{ const xhr = new XMLHttpRequest(); await new Promise((resolve, reject) => {{ xhr.addEventListener('load', resolve, {{ once: true }}); xhr.addEventListener('error', () => reject(new Error('xhr failed')), {{ once: true }}); xhr.open('GET', 'http://{addr}/xhr'); xhr.send(); }}); return xhr.responseText; }})()"
            )
        }
    }))
    .await;
    let _ = ctx.take_all();

    timeout(Duration::from_secs(1), seen_notify.notified())
        .await
        .expect("XHR handler should observe the async user agent override");
    assert_eq!(seen.lock().as_deref(), Some("moli-emulation-async-ua"));

    ctx.process_async(json!({
        "id": 133,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": { "expression": "navigator.userAgent" }
    }))
    .await;
    let response = ctx.take_response_by_id(133);
    assert_eq!(
        response["result"]["result"]["value"],
        json!("moli-emulation-async-ua")
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn emulation_async_dispatch_updates_live_page_surface_without_mutating_accept_language() {
    async fn page_handler() -> impl IntoResponse {
        "<!doctype html><html><body>ok</body></html>"
    }

    async fn xhr_handler(
        State(seen): State<Arc<Mutex<Option<String>>>>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        let accept_language = headers
            .get(axum::http::header::ACCEPT_LANGUAGE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        *seen.lock() = accept_language;
        "ok"
    }

    let seen = Arc::new(Mutex::new(None));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_seen = Arc::clone(&seen);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page_handler))
                .route("/xhr", get(xhr_handler))
                .with_state(server_seen),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(&format!("http://{addr}/page"), Some("SID-1"))
        .await;

    ctx.process_async(json!({
        "id": 120,
        "method": "Emulation.setLocaleOverride",
        "sessionId": "SID-1",
        "params": { "locale": "fr-FR" }
    }))
    .await;
    ctx.expect_result(120, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 121,
        "method": "Emulation.setTimezoneOverride",
        "sessionId": "SID-1",
        "params": { "timezoneId": "Asia/Shanghai" }
    }))
    .await;
    ctx.expect_result(121, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 122,
        "method": "Emulation.setEmulatedMedia",
        "sessionId": "SID-1",
        "params": {
            "media": "screen",
            "features": [
                { "name": "prefers-color-scheme", "value": "dark" }
            ]
        }
    }))
    .await;
    ctx.expect_result(122, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 123,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "awaitPromise": true,
            "expression": format!(
                "(async () => {{ const xhr = new XMLHttpRequest(); await new Promise((resolve, reject) => {{ xhr.addEventListener('load', resolve, {{ once: true }}); xhr.addEventListener('error', () => reject(new Error('xhr failed')), {{ once: true }}); xhr.open('GET', 'http://{addr}/xhr'); xhr.send(); }}); return JSON.stringify({{ localized: new Date('2020-01-02T03:04:05Z').toLocaleString(), dark: matchMedia('(prefers-color-scheme: dark)').matches, light: matchMedia('(prefers-color-scheme: light)').matches }}); }})()"
            )
        }
    }))
    .await;
    wait_until_message(
        &mut ctx,
        "SID-1",
        "Runtime.evaluate response 123",
        |message| message["id"] == json!(123),
    )
    .await;
    let response = ctx.take_response_by_id(123);
    let payload = response["result"]["result"]["value"]
        .as_str()
        .expect("runtime payload should be string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("runtime payload should be valid json");
    assert_eq!(payload["localized"], json!("02/01/2020 11:04:05"));
    assert_eq!(payload["dark"], json!(true));
    assert_eq!(payload["light"], json!(false));
    assert_eq!(seen.lock().as_deref(), Some("en-US,en;q=0.9"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_script_execution_disabled_rejects_invalid_params() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 81,
        "method": "Emulation.setScriptExecutionDisabled",
        "params": {}
    }))
    .await;
    ctx.expect_error(81, -32602, "InvalidParams");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_geolocation_override_accepts_missing_params() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 82,
        "method": "Emulation.setGeolocationOverride"
    }))
    .await;
    ctx.expect_result(82, json!({}), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn set_geolocation_override_rejects_invalid_params() {
    let mut ctx = TestContext::new();

    let raw = json!({
        "id": 83,
        "method": "Emulation.setGeolocationOverride",
        "params": "invalid"
    })
    .to_string();
    let outcome = ctx.conn.process_message_with_turn_outcome_async(&raw).await;
    let (messages, scheduler_events) = ctx.route_completed_command_outcome_for_test(outcome).await;
    assert!(scheduler_events.is_empty());
    assert_eq!(
        messages,
        vec![json!({
            "id": 83,
            "error": {"code": -32600, "message": "Invalid Request"}
        })]
    );

    ctx.process_async(json!({
        "id": 84,
        "method": "Emulation.setGeolocationOverride",
        "params": { "latitude": 91, "longitude": 0, "accuracy": 1 }
    }))
    .await;
    ctx.expect_error(84, -32602, "InvalidParams");

    ctx.process_async(json!({
        "id": 85,
        "method": "Emulation.setGeolocationOverride",
        "params": { "latitude": 0, "longitude": -181, "accuracy": 1 }
    }))
    .await;
    ctx.expect_error(85, -32602, "InvalidParams");

    ctx.process_async(json!({
        "id": 86,
        "method": "Emulation.setGeolocationOverride",
        "params": { "latitude": 0, "longitude": 0, "accuracy": -1 }
    }))
    .await;
    ctx.expect_error(86, -32602, "InvalidParams");
}

async fn evaluate_geolocation_once(ctx: &mut TestContext, id: u64) -> serde_json::Value {
    evaluate_geolocation_once_for_session(ctx, id, "SID-1").await
}

async fn evaluate_geolocation_once_for_session(
    ctx: &mut TestContext,
    id: u64,
    session_id: &str,
) -> serde_json::Value {
    ctx.process_async(json!({
        "id": id,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": {
            "awaitPromise": true,
            "returnByValue": true,
            "expression": r#"
                new Promise((resolve) => {
                    navigator.geolocation.getCurrentPosition(
                        (position) => resolve(JSON.stringify({
                            latitude: position.coords.latitude,
                            longitude: position.coords.longitude,
                            accuracy: position.coords.accuracy,
                            altitude: position.coords.altitude,
                            timestampType: typeof position.timestamp
                        })),
                        (error) => resolve(`error:${error.code}:${error.message}`)
                    );
                })
            "#
        }
    }))
    .await;
    ctx.take_response_by_id(id)["result"]["result"]["value"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn set_geolocation_override_updates_loaded_page_geolocation_surface() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    install_session_page_for_emulation_test(&mut ctx, bc, "data:text/html,<body>ok</body>").await;

    ctx.process_async(json!({
        "id": 87,
        "method": "Emulation.setGeolocationOverride",
        "sessionId": "SID-1",
        "params": { "latitude": 48.85837, "longitude": 2.294481, "accuracy": 7 }
    }))
    .await;
    ctx.expect_result(87, json!({}), Some("SID-1"));

    let value = evaluate_geolocation_once(&mut ctx, 88).await;
    let payload: serde_json::Value =
        serde_json::from_str(value.as_str().expect("geolocation should return json"))
            .expect("geolocation payload should be valid json");
    assert_eq!(payload["latitude"], json!(48.85837));
    assert_eq!(payload["longitude"], json!(2.294481));
    assert_eq!(payload["accuracy"], json!(7));
    assert_eq!(payload["altitude"], json!(null));
    assert_eq!(payload["timestampType"], json!("number"));
}

#[tokio::test(flavor = "multi_thread")]
async fn set_geolocation_override_applies_to_subsequent_navigation_surface() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 89,
        "method": "Emulation.setGeolocationOverride",
        "sessionId": "SID-1",
        "params": { "latitude": 35.658581, "longitude": 139.745433, "accuracy": 3 }
    }))
    .await;
    ctx.expect_result(89, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 90,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>geo</body>" }
    }))
    .await;
    let _ = ctx.take_all();

    let value = evaluate_geolocation_once(&mut ctx, 91).await;
    let payload: serde_json::Value =
        serde_json::from_str(value.as_str().expect("geolocation should return json"))
            .expect("geolocation payload should be valid json");
    assert_eq!(payload["latitude"], json!(35.658581));
    assert_eq!(payload["longitude"], json!(139.745433));
    assert_eq!(payload["accuracy"], json!(3));
}

#[tokio::test(flavor = "multi_thread")]
async fn set_geolocation_override_missing_position_reports_unavailable() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    install_session_page_for_emulation_test(&mut ctx, bc, "data:text/html,<body>ok</body>").await;

    ctx.process_async(json!({
        "id": 92,
        "method": "Emulation.setGeolocationOverride",
        "sessionId": "SID-1",
        "params": {}
    }))
    .await;
    ctx.expect_result(92, json!({}), Some("SID-1"));

    let value = evaluate_geolocation_once(&mut ctx, 93).await;
    assert_eq!(value, json!("error:2:Position unavailable"));
}

#[tokio::test(flavor = "multi_thread")]
async fn clear_geolocation_override_restores_default_after_explicit_unavailable() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    bc.default_geolocation_override = Some(EmulatedGeolocationOverrideState::Position(
        EmulatedGeolocationOverride {
            latitude: 37.33182,
            longitude: -122.03118,
            accuracy: 4.0,
            altitude: None,
            altitude_accuracy: None,
            heading: None,
            speed: None,
        },
    ));
    install_session_page_for_emulation_test(&mut ctx, bc, "data:text/html,<body>geo</body>").await;

    ctx.process_async(json!({
        "id": 97,
        "method": "Emulation.setGeolocationOverride",
        "sessionId": "SID-1",
        "params": {}
    }))
    .await;
    ctx.expect_result(97, json!({}), Some("SID-1"));
    assert!(matches!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context
                .active_page_target()
                .geolocation_override
                .as_ref()),
        Some(EmulatedGeolocationOverrideState::PositionUnavailable)
    ));
    assert_eq!(
        evaluate_geolocation_once(&mut ctx, 98).await,
        json!("error:2:Position unavailable")
    );

    ctx.process_async(json!({
        "id": 99,
        "method": "Emulation.clearGeolocationOverride",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(99, json!({}), Some("SID-1"));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .geolocation_override
            .is_none()
    );

    let value = evaluate_geolocation_once(&mut ctx, 100).await;
    let payload: serde_json::Value =
        serde_json::from_str(value.as_str().expect("geolocation should return json"))
            .expect("geolocation payload should be valid json");
    assert_eq!(payload["latitude"], json!(37.33182));
    assert_eq!(payload["longitude"], json!(-122.03118));
    assert_eq!(payload["accuracy"], json!(4));
}

#[tokio::test(flavor = "multi_thread")]
async fn set_geolocation_override_respects_denied_permission() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    install_session_page_for_emulation_test(&mut ctx, bc, "data:text/html,<body>ok</body>").await;

    ctx.process_async(json!({
        "id": 94,
        "method": "Emulation.setGeolocationOverride",
        "sessionId": "SID-1",
        "params": { "latitude": 1, "longitude": 2, "accuracy": 3 }
    }))
    .await;
    ctx.expect_result(94, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 95,
        "method": "Browser.setPermission",
        "params": {
            "permission": { "name": "geolocation" },
            "setting": "denied",
            "browserContextId": "BID-1"
        }
    }))
    .await;
    ctx.expect_result(95, json!({}), None);

    let value = evaluate_geolocation_once(&mut ctx, 96).await;
    assert_eq!(value, json!("error:1:User denied Geolocation"));
}

#[tokio::test(flavor = "multi_thread")]
async fn device_metrics_override_updates_layout_metrics() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 12,
        "method": "Emulation.setDeviceMetricsOverride",
        "params": {
            "width": 1280,
            "height": 720,
            "deviceScaleFactor": 2,
            "screenWidth": 1440,
            "screenHeight": 900,
            "mobile": false
        }
    }))
    .await;
    ctx.expect_result(12, json!({}), None);

    ctx.process_async(json!({
        "id": 13,
        "method": "Page.getLayoutMetrics"
    }))
    .await;
    ctx.expect_result(
        13,
        json!({
            "layoutViewport": {
                "pageX": 0.0,
                "pageY": 0.0,
                "clientWidth": 1280,
                "clientHeight": 720,
            },
            "visualViewport": {
                "offsetX": 0,
                "offsetY": 0,
                "pageX": 0.0,
                "pageY": 0.0,
                "clientWidth": 1280,
                "clientHeight": 720,
                "scale": 2.0,
                "zoom": 1,
            },
            "contentSize": { "x": 0, "y": 0, "width": 1280.0, "height": 720.0 },
            "cssLayoutViewport": {
                "pageX": 0.0,
                "pageY": 0.0,
                "clientWidth": 1280,
                "clientHeight": 720,
            },
            "cssVisualViewport": {
                "offsetX": 0,
                "offsetY": 0,
                "pageX": 0.0,
                "pageY": 0.0,
                "clientWidth": 1280,
                "clientHeight": 720,
                "scale": 2.0,
                "zoom": 1,
            },
            "cssContentSize": { "x": 0, "y": 0, "width": 1280.0, "height": 720.0 },
        }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn locale_override_updates_intl_without_mutating_language_surfaces() {
    async fn handler(headers: HeaderMap) -> impl IntoResponse {
        let accept_language = headers
            .get(axum::http::header::ACCEPT_LANGUAGE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        format!(
            "<!doctype html><html><body data-accept-language=\"{accept_language}\"><script>document.body.textContent = [Intl.DateTimeFormat().resolvedOptions().locale, navigator.language, navigator.languages.join(','), document.body.dataset.acceptLanguage].join('|');</script></body></html>"
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 14,
        "method": "Emulation.setLocaleOverride",
        "params": { "locale": "fr-FR" }
    }))
    .await;
    ctx.expect_result(14, json!({}), None);

    ctx.process_async(json!({
        "id": 15,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    let _ = ctx.take_all();
    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        html.contains(">fr-FR|en-US|en-US,en|en-US,en;q=0.9<"),
        "got {html}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn bidi_user_context_locale_composes_with_user_agent_on_all_identity_surfaces() {
    async fn handler(headers: HeaderMap) -> impl IntoResponse {
        let user_agent = headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let accept_language = headers
            .get(axum::http::header::ACCEPT_LANGUAGE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        format!(
            "<!doctype html><html><body data-user-agent=\"{user_agent}\" data-accept-language=\"{accept_language}\"><script>document.body.textContent = [navigator.userAgent, Intl.DateTimeFormat().resolvedOptions().locale, navigator.language, navigator.languages.join(','), document.body.dataset.acceptLanguage].join('|');</script></body></html>"
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    for command in [
        DevToolsCommand::SetUserAgentOverride(DevToolsSetUserAgentOverrideCommand {
            context: bidi_command_context(),
            target_ids: Vec::new(),
            browser_context_ids: vec![DevToolsBrowserContextId::from("BID-1")],
            user_agent: Some("MoliBiDi/1.0".to_owned()),
        }),
        DevToolsCommand::SetLocaleOverride(DevToolsSetLocaleOverrideCommand {
            context: bidi_command_context(),
            target_ids: Vec::new(),
            browser_context_ids: vec![DevToolsBrowserContextId::from("BID-1")],
            locale: Some("fr-FR".to_owned()),
        }),
    ] {
        let outcome = ctx.conn.execute_devtools_command(command).await;
        let (result, events, protocol_events, renderer_output_predecessor) =
            outcome.into_complete_parts();
        assert!(matches!(result, Ok(DevToolsCommandResult::Empty)));
        assert!(events.is_empty());
        assert!(protocol_events.is_empty());
        assert!(renderer_output_predecessor.is_none());
    }

    ctx.process_async(json!({
        "id": 155,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    let _ = ctx.take_all();
    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        html.contains(">MoliBiDi/1.0|fr-FR|fr-FR|fr-FR|fr-FR<"),
        "got {html}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn live_locale_override_updates_intl_without_mutating_navigator() {
    let mut ctx = TestContext::new();
    load_session_page_for_pending_emulation_test(&mut ctx).await;

    ctx.process_async(json!({
        "id": 151,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 152,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "Object.defineProperty = function() { throw new Error('defineProperty blocked'); }; 'tampered';"
        }
    }))
    .await;
    let response = ctx.take_response_by_id(152);
    assert_eq!(response["result"]["result"]["value"], json!("tampered"));

    ctx.process_async(json!({
        "id": 153,
        "method": "Emulation.setLocaleOverride",
        "sessionId": "SID-1",
        "params": { "locale": "fr-FR" }
    }))
    .await;
    ctx.expect_result(153, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 154,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "JSON.stringify({ intlLocale: Intl.DateTimeFormat().resolvedOptions().locale, language: navigator.language, languages: Array.from(navigator.languages || []), reflectedLocaleSlot: Object.prototype.hasOwnProperty.call(globalThis, '__moliLocaleOverride') })"
        }
    }))
    .await;
    let response = ctx.take_response_by_id(154);
    let payload = response["result"]["result"]["value"]
        .as_str()
        .expect("runtime evaluate should return a JSON string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("runtime evaluate payload should be json");
    assert_eq!(payload["intlLocale"], json!("fr-FR"));
    assert_eq!(payload["language"], json!("en-US"));
    assert_eq!(payload["languages"], json!(["en-US", "en"]));
    assert_eq!(payload["reflectedLocaleSlot"], json!(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn touch_and_timezone_overrides_apply_to_document_start_surface() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 16,
        "method": "Emulation.setTouchEmulationEnabled",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(16, json!({}), None);

    ctx.process_async(json!({
        "id": 17,
        "method": "Emulation.setTimezoneOverride",
        "params": { "timezoneId": "Asia/Shanghai" }
    }))
    .await;
    ctx.expect_result(17, json!({}), None);

    ctx.process_async(json!({
            "id": 18,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<body><script>document.body.textContent = [String(navigator.maxTouchPoints), Intl.DateTimeFormat().resolvedOptions().timeZone].join('|');</script></body>"
            }
        }))
    .await;

    let _ = ctx.take_all();
    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(html.contains(">1|Asia/Shanghai<"), "got {html}");
}

#[tokio::test(flavor = "multi_thread")]
async fn locale_and_timezone_overrides_apply_to_locale_date_formatting() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 181,
        "method": "Emulation.setLocaleOverride",
        "params": { "locale": "fr-FR" }
    }))
    .await;
    ctx.expect_result(181, json!({}), None);

    ctx.process_async(json!({
        "id": 182,
        "method": "Emulation.setTimezoneOverride",
        "params": { "timezoneId": "Asia/Shanghai" }
    }))
    .await;
    ctx.expect_result(182, json!({}), None);

    ctx.process_async(json!({
            "id": 183,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<body><script>const d = new Date('2020-01-02T03:04:05Z'); document.body.textContent = d.toLocaleString();</script></body>"
            }
        }))
    .await;

    let _ = ctx.take_all();
    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(html.contains("02/01/2020"), "got {html}");
    assert!(html.contains("11:04:05"), "got {html}");
}

#[tokio::test(flavor = "multi_thread")]
async fn context_emulated_media_applies_to_loaded_background_page_without_activation() {
    let mut ctx = TestContext::new();
    let background = PageTargetHost::new(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    );

    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active");
    bc.insert_page_target_host(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>background</body>",
        Some("SID-background"),
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 188,
        "method": "Emulation.setEmulatedMedia",
        "params": {
            "media": "screen",
            "features": [
                { "name": "prefers-color-scheme", "value": "dark" }
            ]
        }
    }))
    .await;
    ctx.expect_result(188, json!({}), None);

    ctx.process_async(json!({
        "id": 189,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    ctx.expect_result(189, json!({}), Some("SID-background"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 190,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "JSON.stringify({ dark: matchMedia('(prefers-color-scheme: dark)').matches, light: matchMedia('(prefers-color-scheme: light)').matches })"
        }
    }))
    .await;
    let response = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(190))
        .cloned()
        .expect("runtime evaluate result");
    let payload = response["result"]["result"]["value"]
        .as_str()
        .expect("runtime evaluate should return string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("runtime evaluate payload should be json");
    assert_eq!(payload["dark"], json!(true));
    assert_eq!(payload["light"], json!(false));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "context-wide overrides should not activate the loaded background target"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn context_locale_override_applies_to_loaded_background_page_without_activation() {
    let mut ctx = TestContext::new();
    let background = PageTargetHost::new(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    );

    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active");
    bc.insert_page_target_host(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>background</body>",
        Some("SID-background"),
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 196,
        "method": "Emulation.setLocaleOverride",
        "params": { "locale": "fr-FR" }
    }))
    .await;
    ctx.expect_result(196, json!({}), None);

    ctx.process_async(json!({
        "id": 197,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    ctx.expect_result(197, json!({}), Some("SID-background"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 198,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "JSON.stringify({ date: new Date('2020-01-02T03:04:05Z').toLocaleDateString() })"
        }
    }))
    .await;
    let response = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(198))
        .cloned()
        .expect("runtime evaluate result");
    let payload = response["result"]["result"]["value"]
        .as_str()
        .expect("runtime evaluate should return string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("runtime evaluate payload should be json");
    assert_eq!(payload["date"], json!("02/01/2020"));

    let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        browser_context.active_target_id(),
        Some("TID-active"),
        "context-wide overrides should not activate the loaded background target"
    );
    assert_eq!(
        browser_context
            .active_page_target()
            .effective_policy()
            .locale_override(),
        Some("fr-FR")
    );
    assert!(
        browser_context
            .background_target("TID-background")
            .filter(|target| target.has_non_default_session_state())
            .and_then(|state| {
                state
                    .effective_policy()
                    .locale_override()
                    .map(str::to_owned)
            })
            .is_none(),
        "context-wide locale remains browser-context state, not background session state"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn session_emulation_routes_to_loaded_background_owner_without_activation() {
    let mut ctx = TestContext::new();
    let background = PageTargetHost::new(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        TargetIdentityState::about_blank(),
        TargetPageSlot::empty_for_test_fixture(),
    );

    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active");
    bc.insert_page_target_host(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>background</body>",
        Some("SID-background"),
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 191,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    ctx.expect_result(191, json!({}), Some("SID-background"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 192,
        "method": "Emulation.setEmulatedMedia",
        "sessionId": "SID-background",
        "params": {
            "media": "screen",
            "features": [
                { "name": "prefers-color-scheme", "value": "dark" }
            ]
        }
    }))
    .await;
    ctx.expect_result(192, json!({}), Some("SID-background"));

    ctx.process_async(json!({
        "id": 193,
        "method": "Emulation.setLocaleOverride",
        "sessionId": "SID-background",
        "params": { "locale": "zh-CN" }
    }))
    .await;
    ctx.expect_result(193, json!({}), Some("SID-background"));

    ctx.process_async(json!({
        "id": 195,
        "method": "Emulation.setGeolocationOverride",
        "sessionId": "SID-background",
        "params": { "latitude": 35.6586, "longitude": 139.7454, "accuracy": 9 }
    }))
    .await;
    ctx.expect_result(195, json!({}), Some("SID-background"));

    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "session-scoped Emulation should not activate the loaded background target"
    );

    ctx.process_async(json!({
        "id": 194,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "JSON.stringify({ dark: matchMedia('(prefers-color-scheme: dark)').matches })"
        }
    }))
    .await;
    let response = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(194))
        .cloned()
        .expect("runtime evaluate result");
    let payload = response["result"]["result"]["value"]
        .as_str()
        .expect("runtime evaluate should return string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("runtime evaluate payload should be json");
    assert_eq!(payload["dark"], json!(true));

    let value = evaluate_geolocation_once_for_session(&mut ctx, 196, "SID-background").await;
    let payload: serde_json::Value =
        serde_json::from_str(value.as_str().expect("geolocation should return json"))
            .expect("geolocation payload should be valid json");
    assert_eq!(payload["latitude"], json!(35.6586));
    assert_eq!(payload["longitude"], json!(139.7454));
    assert_eq!(payload["accuracy"], json!(9));

    let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
    assert!(
        browser_context
            .active_page_target()
            .emulated_media
            .color_scheme
            .is_none(),
        "background Emulation should not mutate the active target media override"
    );
    assert!(
        browser_context
            .active_page_target()
            .effective_policy()
            .locale_override()
            .is_none(),
        "background Emulation should not mutate the active target locale override"
    );
    let background = browser_context
        .background_target("TID-background")
        .filter(|target| target.has_non_default_session_state())
        .expect("background target state");
    assert_eq!(
        background.emulated_media.color_scheme.as_deref(),
        Some("dark")
    );
    assert_eq!(
        background.effective_policy().locale_override(),
        Some("zh-CN")
    );
    assert_eq!(
        background
            .geolocation_override
            .as_ref()
            .and_then(EmulatedGeolocationOverrideState::position)
            .map(|position| (position.latitude, position.longitude, position.accuracy)),
        Some((35.6586, 139.7454, 9.0))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn emulated_media_updates_existing_media_query_list_matches() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body><script>globalThis.events = []; globalThis.darkMql = matchMedia('(prefers-color-scheme: dark)'); globalThis.lightMql = matchMedia('(prefers-color-scheme: light)'); darkMql.addEventListener('change', event => events.push(['dark', event.matches, event.media, event.target === darkMql])); lightMql.onchange = event => events.push(['light', event.matches, event.media, event.target === lightMql]);</script></body>",
        Some("SID-1"),
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 184,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "JSON.stringify({ dark: darkMql.matches, light: lightMql.matches, events })"
        }
    }))
    .await;
    let response = ctx.take_response_by_id(184);
    let payload = response["result"]["result"]["value"]
        .as_str()
        .expect("runtime evaluate should return string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("runtime payload should be json");
    assert_eq!(payload["dark"], json!(false));
    assert_eq!(payload["light"], json!(true));
    assert_eq!(payload["events"], json!([]));

    ctx.process_async(json!({
        "id": 185,
        "method": "Emulation.setEmulatedMedia",
        "sessionId": "SID-1",
        "params": {
            "media": "screen",
            "features": [
                { "name": "prefers-color-scheme", "value": "dark" }
            ]
        }
    }))
    .await;
    ctx.expect_result(185, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 186,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "JSON.stringify({ dark: darkMql.matches, light: lightMql.matches, events })"
        }
    }))
    .await;
    let response = ctx.take_response_by_id(186);
    let payload = response["result"]["result"]["value"]
        .as_str()
        .expect("runtime evaluate should return string");
    let payload: serde_json::Value =
        serde_json::from_str(payload).expect("runtime payload should be json");
    assert_eq!(payload["dark"], json!(true));
    assert_eq!(payload["light"], json!(false));
    assert_eq!(
        payload["events"],
        json!([
            ["dark", true, "(prefers-color-scheme: dark)", true],
            ["light", false, "(prefers-color-scheme: light)", true]
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn generated_surface_refresh_does_not_freeze_match_media_override() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner("data:text/html,<body></body>", Some("SID-1"))
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 187,
        "method": "Emulation.setEmulatedMedia",
        "sessionId": "SID-1",
        "params": {
            "features": [
                { "name": "prefers-color-scheme", "value": "light" }
            ]
        }
    }))
    .await;
    ctx.expect_result(187, json!({}), Some("SID-1"));

    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .apply_surface_overrides_to_loaded_page_async()
        .await
        .expect("surface refresh should succeed");

    ctx.process_async(json!({
        "id": 188,
        "method": "Emulation.setEmulatedMedia",
        "sessionId": "SID-1",
        "params": {
            "features": [
                { "name": "prefers-color-scheme", "value": "dark" }
            ]
        }
    }))
    .await;
    ctx.expect_result(188, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 189,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "matchMedia('(prefers-color-scheme: dark)').matches"
        }
    }))
    .await;
    let response = ctx.take_response_by_id(189);
    assert_eq!(response["result"]["result"]["value"], json!(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn target_session_detach_disposes_non_aggregated_emulation_state_before_reattach() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner("data:text/html,<body></body>", None)
        .await;
    ctx.sent.clear();
    ctx.conn.register_top_level_page_target("TID-1");

    ctx.process_async(json!({
        "id": 190,
        "method": "Target.attachToTarget",
        "params": { "targetId": "TID-1", "flatten": true }
    }))
    .await;
    let session_id = ctx.take_response_by_id(190)["result"]["sessionId"]
        .as_str()
        .expect("target session id")
        .to_owned();
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 1901,
        "method": "Emulation.setCPUThrottlingRate",
        "sessionId": session_id,
        "params": { "rate": 4 }
    }))
    .await;
    ctx.expect_result(1901, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 191,
        "method": "Emulation.setEmulatedMedia",
        "sessionId": session_id,
        "params": {
            "features": [
                { "name": "prefers-color-scheme", "value": "dark" }
            ]
        }
    }))
    .await;
    ctx.expect_result(191, json!({}), Some(&session_id));

    ctx.process_async(json!({
        "id": 192,
        "method": "Target.detachFromTarget",
        "params": { "sessionId": session_id }
    }))
    .await;
    ctx.expect_result(192, json!({}), None);
    let _ = ctx.take_all();
    let target = ctx
        .conn
        .browser_context
        .as_ref()
        .and_then(|browser_context| browser_context.page_target("TID-1"))
        .expect("detached target remains addressable");
    assert_eq!(target.cpu_throttling_rate, 1.0);
    assert_eq!(
        target.emulated_media,
        crate::conn::EmulatedMediaOverrides::default()
    );

    ctx.process_async(json!({
        "id": 193,
        "method": "Target.attachToTarget",
        "params": { "targetId": "TID-1", "flatten": true }
    }))
    .await;
    let replacement_session_id = ctx.take_response_by_id(193)["result"]["sessionId"]
        .as_str()
        .expect("replacement target session id")
        .to_owned();
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 194,
        "method": "Runtime.evaluate",
        "sessionId": replacement_session_id,
        "params": {
            "expression": "matchMedia('(prefers-color-scheme: dark)').matches"
        }
    }))
    .await;
    let response = ctx.take_response_by_id(194);
    assert_eq!(response["result"]["result"]["value"], json!(false));

    ctx.process_async(json!({
        "id": 195,
        "method": "Target.attachToTarget",
        "params": { "targetId": "TID-1", "flatten": true }
    }))
    .await;
    let attached_session_id = ctx.take_response_by_id(195)["result"]["sessionId"]
        .as_str()
        .expect("attached target session id")
        .to_owned();
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 196,
        "method": "Emulation.setEmulatedMedia",
        "sessionId": attached_session_id,
        "params": {
            "features": [
                { "name": "prefers-color-scheme", "value": "dark" }
            ]
        }
    }))
    .await;
    ctx.expect_result(196, json!({}), Some(&attached_session_id));

    ctx.process_async(json!({
        "id": 197,
        "method": "Runtime.evaluate",
        "sessionId": replacement_session_id,
        "params": {
            "expression": "matchMedia('(prefers-color-scheme: dark)').matches"
        }
    }))
    .await;
    let response = ctx.take_response_by_id(197);
    assert_eq!(response["result"]["result"]["value"], json!(true));

    ctx.process_async(json!({
        "id": 198,
        "method": "Target.detachFromTarget",
        "params": { "sessionId": attached_session_id }
    }))
    .await;
    ctx.expect_result(198, json!({}), None);
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 199,
        "method": "Runtime.evaluate",
        "sessionId": replacement_session_id,
        "params": {
            "expression": "matchMedia('(prefers-color-scheme: dark)').matches"
        }
    }))
    .await;
    let response = ctx.take_response_by_id(199);
    assert_eq!(response["result"]["result"]["value"], json!(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn emulated_media_color_scheme_applies_to_match_media_surface() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 184,
        "method": "Emulation.setEmulatedMedia",
        "params": {
            "media": "screen",
            "features": [
                { "name": "prefers-color-scheme", "value": "dark" }
            ]
        }
    }))
    .await;
    ctx.expect_result(184, json!({}), None);

    ctx.process_async(json!({
            "id": 185,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<body><script>document.body.textContent = [String(matchMedia('(prefers-color-scheme: dark)').matches), String(matchMedia('(prefers-color-scheme: light)').matches), String(matchMedia('screen').matches), String(matchMedia('print').matches)].join('|');</script></body>"
            }
        }))
    .await;

    let _ = ctx.take_all();
    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(html.contains(">true|false|true|false<"), "got {html}");
}

#[tokio::test(flavor = "multi_thread")]
async fn active_document_start_surface_reports_active_focus_and_visibility() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
            "id": 18,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<body><script>document.body.textContent = [String(document.hasFocus()), String(document.hidden), document.visibilityState].join('|');</script></body>"
            }
        }))
    .await;

    let _ = ctx.take_all();
    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(html.contains(">true|false|visible<"), "got {html}");
}

#[tokio::test(flavor = "multi_thread")]
async fn focus_emulation_override_applies_to_document_start_surface() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 19,
        "method": "Emulation.setFocusEmulationEnabled",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(19, json!({}), None);

    ctx.process_async(json!({
            "id": 20,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<body><script>document.body.textContent = [String(document.hasFocus()), String(document.hidden), document.visibilityState].join('|');</script></body>"
            }
        }))
    .await;

    let _ = ctx.take_all();
    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(html.contains(">true|false|visible<"), "got {html}");
}
