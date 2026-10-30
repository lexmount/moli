use crate::AutomationCommandTaskStep;
use crate::automation::{
    AutomationCommand, AutomationContext, AutomationEvent, AutomationResult,
    DevToolsActivateTargetCommand, DevToolsAddNetworkInterceptCommand,
    DevToolsAddPreloadScriptCommand, DevToolsAuthChallengeAction, DevToolsBrowserContextId,
    DevToolsCallFunctionCommand, DevToolsCaptureScreenshotClip, DevToolsCaptureScreenshotCommand,
    DevToolsCloseTargetCommand, DevToolsContinueInterceptedRequestCommand,
    DevToolsContinueInterceptedResponseCommand, DevToolsContinueWithAuthCommand,
    DevToolsCookieParam, DevToolsCreateBrowserContextCommand, DevToolsCreateTargetCommand,
    DevToolsDeleteCookiesCommand, DevToolsDescribeNodeCommand, DevToolsDevicePixelRatioSetting,
    DevToolsDispatchKeyEventCommand, DevToolsDispatchMouseEventCommand, DevToolsDomGeometryCommand,
    DevToolsDomGeometryOperation, DevToolsDomNodeReference, DevToolsErrorKind,
    DevToolsEvaluateScriptCommand, DevToolsFailInterceptedRequestCommand,
    DevToolsFulfillInterceptedRequestCommand, DevToolsGetAttributesCommand,
    DevToolsGetBrowserContextsCommand, DevToolsGetCookiesCommand, DevToolsGetFrameTreeCommand,
    DevToolsGetLayoutMetricsCommand, DevToolsGetNavigationHistoryCommand,
    DevToolsGetOuterHtmlCommand, DevToolsGetPropertyCommand, DevToolsGetRealmsCommand,
    DevToolsGetTargetsCommand, DevToolsGetTextCommand, DevToolsHistoryTraversalDestination,
    DevToolsKeyEventType, DevToolsLocateNodesCommand, DevToolsLocateNodesLocator,
    DevToolsLocateNodesTextMatch, DevToolsMouseEventType, DevToolsNavigateCommand,
    DevToolsNavigationWait, DevToolsNetworkInterceptId, DevToolsNetworkInterceptPattern,
    DevToolsNetworkInterceptPhase, DevToolsPointerType, DevToolsPreloadScriptSource,
    DevToolsPrintToPdfCommand, DevToolsPrintToPdfTransferMode, DevToolsQuerySelectorCommand,
    DevToolsReleaseObjectsCommand, DevToolsReloadCommand, DevToolsRemoteHandleId,
    DevToolsRemoteValue, DevToolsRemoveBrowserContextCommand,
    DevToolsRemoveNetworkInterceptCommand, DevToolsRemovePreloadScriptCommand, DevToolsRequestId,
    DevToolsResolveNodeCommand, DevToolsResultOwnership, DevToolsScreenshotElementClip,
    DevToolsScriptResult, DevToolsScrollIntoViewIfNeededCommand, DevToolsSerializationOptions,
    DevToolsSessionId, DevToolsSetCookiesCommand, DevToolsSetFileInputFilesCommand,
    DevToolsSetViewportCommand, DevToolsSetWindowStateCommand, DevToolsTargetId,
    DevToolsTraverseHistoryCommand, DevToolsTraverseHistoryResult, DevToolsViewportSetting,
    DevToolsWindowState, FrontendProtocol,
};
use serde_json::json;

use super::*;

#[test]
fn command_wait_failure_preserves_context_events_and_does_not_ack_early() {
    let mut conn = CdpConnection::new();
    let (permit, flush) = conn.begin_command_response_flush_permit();
    let observer = flush.receiver().unwrap();
    let mut context = CommandDispatchContext::new(flush);
    context.push_protocol_event(BackgroundProtocolEvent::immediate(json!({
        "method": "Test.beforeResponse", "params": {}
    })));
    context.extend_post_response_events([BackgroundProtocolEvent::immediate(json!({
        "method": "Test.afterResponse", "params": {}
    }))]);

    let CdpCommandTaskStep::Complete(outcome) = conn.complete_failed_command_wait_with_context(
        Some(42),
        Some("SID-output"),
        "CommandWaitPanicked",
        &mut context,
    ) else {
        panic!("wait failure must terminate the command");
    };
    let (before, renderer_tail, boundary, after, _, predecessor) =
        outcome.into_renderer_owner_turn_parts();
    assert!(renderer_tail.is_empty());
    assert!(boundary.is_none() && predecessor.is_none());
    let before = before
        .into_iter()
        .map(BackgroundProtocolEvent::into_protocol_message)
        .collect::<Vec<_>>();
    let after = after
        .into_iter()
        .map(BackgroundProtocolEvent::into_protocol_message)
        .collect::<Vec<_>>();
    assert_eq!(before.len(), 2);
    assert_eq!(before[0]["method"], "Test.beforeResponse");
    assert_eq!(
        before[1],
        json!({
            "id": 42, "sessionId": "SID-output",
            "error": {"code": -32000, "message": "CommandWaitPanicked"}
        })
    );
    assert_eq!(after.len(), 1);
    assert_eq!(after[0]["method"], "Test.afterResponse");
    assert!(
        !*observer.borrow(),
        "buffered errors do not prove wire delivery"
    );
    permit.finish();
    assert!(*observer.borrow());
}

#[test]
fn command_wait_failure_delivery_drop_cancels_instead_of_claiming_ack() {
    let mut conn = CdpConnection::new();
    let (permit, flush) = conn.begin_command_response_flush_permit();
    let observer = flush.receiver().unwrap();
    let mut context = CommandDispatchContext::new(flush);
    let _ = conn.complete_failed_command_wait_with_context(
        Some(42),
        None,
        "CommandWaitCancelled",
        &mut context,
    );
    assert!(!*observer.borrow());
    drop(permit);
    assert!(
        observer.has_changed().is_err(),
        "failed delivery must release flush observers"
    );
}

fn complete_messages(step: CdpCommandTaskStep) -> Vec<Value> {
    match step {
        CdpCommandTaskStep::Complete(outcome) => outcome.into_parts().0,
        CdpCommandTaskStep::Pending(_) => panic!("expected complete command dispatch"),
    }
}

fn expect_script_value_result(result: AutomationResult, message: &str) -> DevToolsRemoteValue {
    let AutomationResult::Script(result) = result else {
        panic!("{message}");
    };
    let DevToolsScriptResult::Value(value) = *result else {
        panic!("{message}");
    };
    value
}

fn is_command_response_sidecar_event(event: &BackgroundProtocolEvent) -> bool {
    event.protocol_message().is_some_and(|message| {
        message.get("id").is_some()
            && (message.get("result").is_some() || message.get("error").is_some())
    })
}

fn bidi_fetch_command_context() -> AutomationContext {
    AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    }
}

async fn execute_direct_automation_command_through_renderer_fence_for_test(
    ctx: &mut crate::testing::TestContext,
    command: AutomationCommand,
) -> Result<AutomationResult, crate::automation::DevToolsError> {
    ctx.execute_automation_command_through_renderer_fence_for_test(command)
        .await
}

async fn evaluate_string_through_renderer_fence_for_test(
    ctx: &mut crate::testing::TestContext,
    context: AutomationContext,
    expression: &str,
    label: &str,
) -> String {
    let result = execute_direct_automation_command_through_renderer_fence_for_test(
        ctx,
        AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
            context,
            realm_id: None,
            world_name: None,
            expression: expression.to_owned(),
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        }),
    )
    .await;
    let value = expect_script_value_result(
        result.unwrap_or_else(|error| panic!("{label} should evaluate: {error:?}")),
        "expected string script value",
    );
    value
        .value
        .as_str()
        .unwrap_or_else(|| panic!("{label} should be a string remote value: {value:?}"))
        .to_owned()
}

async fn create_target_in_browser_context_through_renderer_fence_for_test(
    ctx: &mut crate::testing::TestContext,
    context: &AutomationContext,
    browser_context_id: &str,
    label: &str,
) -> DevToolsTargetId {
    let create_result = execute_direct_automation_command_through_renderer_fence_for_test(
        ctx,
        AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: Some(DevToolsBrowserContextId::from(browser_context_id)),
            activate: true,
        }),
    )
    .await;
    let AutomationResult::CreateTarget(create_result) =
        create_result.unwrap_or_else(|error| panic!("create {label} should succeed: {error:?}"))
    else {
        panic!("expected create target result for {label}");
    };
    create_result.target_id
}

async fn materialize_bidi_target_node_for_test(
    ctx: &mut crate::testing::TestContext,
    html: &str,
    selector: &str,
) -> (AutomationContext, DevToolsRemoteHandleId, u32) {
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let create_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        )
        .await;
    let AutomationResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = AutomationContext {
        target_id: Some(create_result.target_id),
        ..context
    };

    let navigate_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::Navigate(
            DevToolsNavigateCommand {
                context: target_context.clone(),
                url: format!("data:text/html,{html}"),
                referrer: None,
                wait: DevToolsNavigationWait::Load,
            },
        ))
        .await;
    navigate_result.expect("navigate should succeed");

    let evaluate_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
                context: target_context.clone(),
                realm_id: None,
                world_name: None,
                expression: format!("document.querySelector({})", json!(selector)),
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::Root,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            }),
        )
        .await;
    let AutomationResult::Script(evaluate_result) =
        evaluate_result.expect("node evaluate should succeed")
    else {
        panic!("expected script result");
    };
    let DevToolsScriptResult::Value(remote_value) = *evaluate_result else {
        panic!("expected remote node value");
    };
    let shared_id = remote_value
        .shared_id
        .expect("node remote value should expose sharedId");
    let backend_node_id = remote_value
        .backend_node_id
        .expect("node remote value should expose backendNodeId");
    (target_context, shared_id, backend_node_id)
}

async fn materialize_bidi_target_input_node_for_test(
    ctx: &mut crate::testing::TestContext,
    html: &str,
) -> (AutomationContext, DevToolsRemoteHandleId, u32) {
    materialize_bidi_target_node_for_test(ctx, html, "#target").await
}

async fn ensure_initial_document_for_target_id_for_test(
    ctx: &mut crate::testing::TestContext,
    target_id: &DevToolsTargetId,
) {
    let route = ctx.with_fixture(|conn| {
        conn.target_session_route_for_target_id(target_id.as_str())
            .unwrap_or_else(|| panic!("target route should exist for {}", target_id.as_str()))
    });
    let owner = CommandOwnerScope::for_route(route);
    let pending = ctx.with_fixture(|conn| {
        conn.start_initial_document_page_ensure_for_owner(&owner)
            .unwrap_or_else(|message| {
                panic!(
                    "target lifecycle ensure should start for {}: {message}",
                    target_id.as_str()
                )
            })
    });
    let Some(pending) = pending else {
        return;
    };
    let completed = pending
        .wait()
        .await
        .unwrap_or_else(|message| panic!("initial document build should complete: {message}"));
    let mut driver = ProtocolWorkflowDriver::new(ctx.handle());
    ctx.drive_owned_wait(completed.install_using(&mut driver))
        .await
        .unwrap_or_else(|message| panic!("initial document should install: {message}"));
}

async fn evaluate_string_for_test(
    ctx: &mut crate::testing::TestContext,
    context: AutomationContext,
    expression: &str,
    label: &str,
) -> String {
    let evaluate_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
                context,
                realm_id: None,
                world_name: None,
                expression: expression.to_owned(),
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            }),
        )
        .await;
    let value = expect_script_value_result(
        evaluate_result.unwrap_or_else(|error| panic!("{label} should evaluate: {error:?}")),
        "expected string script value",
    );
    value
        .value
        .as_str()
        .unwrap_or_else(|| panic!("{label} should be a string remote value: {value:?}"))
        .to_owned()
}

fn connection_with_background_pending_fetch_action(request_id: &str) -> CdpConnection {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-fetch-background".to_owned());
    browser_context.set_active_target_id("TID-active".to_owned());
    browser_context.attach_active_session("SID-active".to_owned());
    browser_context.insert_page_target_host(PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "https://example.test/background".to_owned(),
    ));
    browser_context
        .background_target_mut("TID-background")
        .expect("background target")
        .fetch_owner
        .pending_state_mut()
        .insert_pending_fetch_request_id_for_test(request_id.to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);
    conn
}

async fn assert_bidi_fetch_action_consumes_background_request(
    command: AutomationCommand,
    request_id: &str,
) {
    let mut ctx = crate::testing::TestContext::from_conn(
        connection_with_background_pending_fetch_action(request_id),
    );
    ctx.with_fixture(|conn| {
        assert!(matches!(
            conn.pending_fetch_request_session_route(request_id),
            Some(CdpSessionRoute::PageTarget { target_id, .. }) if target_id == "TID-background"
        ))
    });

    let (result, events, protocol_events, renderer_output_predecessor) = ctx
        .execute_owned_automation_command_for_test(command)
        .await
        .into_complete_parts();

    assert!(events.is_empty());
    assert!(protocol_events.is_empty());
    assert!(renderer_output_predecessor.is_none());
    assert_eq!(
        result.expect("BiDi request action should resolve background owner"),
        AutomationResult::Empty
    );
    ctx.with_fixture(|conn| {
        assert!(
            conn.pending_fetch_request_session_route(request_id)
                .is_none(),
            "resolved background request should be consumed"
        )
    });
    ctx.with_fixture(|conn| {
        assert_eq!(
            conn.browser_context
                .as_ref()
                .expect("browser context")
                .active_target_id(),
            Some("TID-active"),
            "resolving a BiDi request id must not activate the background target"
        )
    });
}

fn stored_cookie_for_dispatch_test(name: &str, value: &str) -> moli_cookie_jar::StoredCookie {
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
        creation_time: None,
        last_access_index: 0,
    }
}

async fn complete_command_task_for_test(
    ctx: &mut crate::testing::TestContext,
    pending: PendingCdpCommandDispatch,
) -> Vec<Value> {
    ctx.complete_command_task_step_for_test(CdpCommandTaskStep::Pending(Box::new(pending)))
        .await
        .0
}

async fn attach_page_session_for_test(
    ctx: &mut crate::testing::TestContext,
    target_id: &str,
) -> String {
    let raw = serde_json::to_string(&json!({
        "id": 9_900_001,
        "method": "Target.attachToTarget",
        "params": { "targetId": target_id, "flatten": true }
    }))
    .expect("attach command should serialize");
    let messages = match ctx.with_fixture(|conn| conn.start_command_dispatch(&raw)) {
        CdpCommandTaskStep::Pending(pending) => complete_command_task_for_test(ctx, *pending).await,
        CdpCommandTaskStep::Complete(outcome) => outcome.into_parts().0,
    };
    messages
        .iter()
        .find(|message| message["id"] == json!(9_900_001))
        .and_then(|message| message["result"]["sessionId"].as_str())
        .unwrap_or_else(|| panic!("attachToTarget should return a session: {messages:?}"))
        .to_owned()
}

async fn evaluate_document_surface_payload(
    ctx: &mut crate::testing::TestContext,
    context: AutomationContext,
) -> serde_json::Value {
    let result = ctx.execute_automation_command_through_renderer_fence_for_test(AutomationCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context,
                realm_id: None,
                world_name: None,
                expression: "JSON.stringify({ hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState, hasFullScreen: 'fullScreen' in window, hasWebkitIsFullScreen: 'webkitIsFullScreen' in document })"
                    .to_owned(),
                await_promise: true,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            },
        ))
        .await
        ;
    let result = expect_script_value_result(
        result.expect("document surface evaluate should succeed"),
        "expected document surface JSON string",
    );
    serde_json::from_str(
        result
            .value
            .as_str()
            .expect("document surface should be a JSON string"),
    )
    .expect("document surface JSON should parse")
}

mod bidi;
mod browser_context;
mod command_dispatch;
mod commands;
mod dom;
mod fetch;
mod page;
mod runtime;
mod storage;
mod target;
