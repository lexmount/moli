use crate::DevToolsRuntimeCommandTaskStep;
use crate::devtools_runtime::{
    AutomationEvent, DevToolsActivateTargetCommand, DevToolsAddNetworkInterceptCommand,
    DevToolsAddPreloadScriptCommand, DevToolsAuthChallengeAction, DevToolsBrowserContextId,
    DevToolsCallFunctionCommand, DevToolsCaptureScreenshotClip, DevToolsCaptureScreenshotCommand,
    DevToolsCloseTargetCommand, DevToolsCommand, DevToolsCommandContext, DevToolsCommandResult,
    DevToolsContinueInterceptedRequestCommand, DevToolsContinueInterceptedResponseCommand,
    DevToolsContinueWithAuthCommand, DevToolsCookieParam, DevToolsCreateBrowserContextCommand,
    DevToolsCreateTargetCommand, DevToolsDeleteCookiesCommand, DevToolsDescribeNodeCommand,
    DevToolsDevicePixelRatioSetting, DevToolsDispatchKeyEventCommand,
    DevToolsDispatchMouseEventCommand, DevToolsDomGeometryCommand, DevToolsDomGeometryOperation,
    DevToolsDomNodeReference, DevToolsErrorKind, DevToolsEvaluateScriptCommand,
    DevToolsFailInterceptedRequestCommand, DevToolsFulfillInterceptedRequestCommand,
    DevToolsGetAttributesCommand, DevToolsGetBrowserContextsCommand, DevToolsGetCookiesCommand,
    DevToolsGetFrameTreeCommand, DevToolsGetLayoutMetricsCommand,
    DevToolsGetNavigationHistoryCommand, DevToolsGetOuterHtmlCommand, DevToolsGetPropertyCommand,
    DevToolsGetRealmsCommand, DevToolsGetTargetsCommand, DevToolsGetTextCommand,
    DevToolsHistoryTraversalDestination, DevToolsKeyEventType, DevToolsLocateNodesCommand,
    DevToolsLocateNodesLocator, DevToolsLocateNodesTextMatch, DevToolsMouseEventType,
    DevToolsNavigateCommand, DevToolsNavigationWait, DevToolsNetworkInterceptId,
    DevToolsNetworkInterceptPattern, DevToolsNetworkInterceptPhase, DevToolsPointerType,
    DevToolsPreloadScriptSource, DevToolsPrintToPdfCommand, DevToolsPrintToPdfTransferMode,
    DevToolsProtocol, DevToolsQuerySelectorCommand, DevToolsReleaseObjectsCommand,
    DevToolsReloadCommand, DevToolsRemoteHandleId, DevToolsRemoteValue,
    DevToolsRemoveBrowserContextCommand, DevToolsRemoveNetworkInterceptCommand,
    DevToolsRemovePreloadScriptCommand, DevToolsRequestId, DevToolsResolveNodeCommand,
    DevToolsResultOwnership, DevToolsScreenshotElementClip, DevToolsScriptResult,
    DevToolsScrollIntoViewIfNeededCommand, DevToolsSerializationOptions, DevToolsSessionId,
    DevToolsSetCookiesCommand, DevToolsSetFileInputFilesCommand, DevToolsSetViewportCommand,
    DevToolsSetWindowStateCommand, DevToolsTargetId, DevToolsTraverseHistoryCommand,
    DevToolsTraverseHistoryResult, DevToolsViewportSetting, DevToolsWindowState,
};
use serde_json::json;

use super::*;

fn complete_messages(step: CdpCommandTaskStep) -> Vec<Value> {
    match step {
        CdpCommandTaskStep::Complete(outcome) => outcome.into_parts().0,
        CdpCommandTaskStep::Pending(_) => panic!("expected complete command dispatch"),
    }
}

fn expect_script_value_result(result: DevToolsCommandResult, message: &str) -> DevToolsRemoteValue {
    let DevToolsCommandResult::Script(result) = result else {
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

fn bidi_fetch_command_context() -> DevToolsCommandContext {
    DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    }
}

async fn execute_direct_devtools_command_through_renderer_fence_for_test(
    ctx: &mut crate::testing::TestContext,
    command: DevToolsCommand,
) -> Result<DevToolsCommandResult, crate::devtools_runtime::DevToolsError> {
    ctx.execute_devtools_command_through_renderer_fence_for_test(command)
        .await
}

async fn evaluate_string_through_renderer_fence_for_test(
    ctx: &mut crate::testing::TestContext,
    context: DevToolsCommandContext,
    expression: &str,
    label: &str,
) -> String {
    let result = execute_direct_devtools_command_through_renderer_fence_for_test(
        ctx,
        DevToolsCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
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
    context: &DevToolsCommandContext,
    browser_context_id: &str,
    label: &str,
) -> DevToolsTargetId {
    let create_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        ctx,
        DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: Some(DevToolsBrowserContextId::from(browser_context_id)),
            activate: true,
        }),
    )
    .await;
    let DevToolsCommandResult::CreateTarget(create_result) =
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
) -> (DevToolsCommandContext, DevToolsRemoteHandleId, u32) {
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let create_result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            },
        ))
        .await;
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = DevToolsCommandContext {
        target_id: Some(create_result.target_id),
        ..context
    };

    let navigate_result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::Navigate(
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
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
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
            },
        ))
        .await;
    let DevToolsCommandResult::Script(evaluate_result) =
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
) -> (DevToolsCommandContext, DevToolsRemoteHandleId, u32) {
    materialize_bidi_target_node_for_test(ctx, html, "#target").await
}

async fn ensure_initial_document_for_target_id_for_test(
    conn: &mut CdpConnection,
    target_id: &DevToolsTargetId,
) {
    let route = conn
        .target_session_route_for_target_id(target_id.as_str())
        .unwrap_or_else(|| panic!("target route should exist for {}", target_id.as_str()));
    let owner = CommandOwnerScope::for_route(route);
    let pending = conn
        .start_initial_document_page_ensure_for_owner(&owner)
        .unwrap_or_else(|message| {
            panic!(
                "target lifecycle ensure should start for {}: {message}",
                target_id.as_str()
            )
        });
    let Some(pending) = pending else {
        return;
    };
    let completed = pending
        .wait()
        .await
        .unwrap_or_else(|message| panic!("initial document build should complete: {message}"));
    conn.complete_initial_document_page_build_for_owner(completed)
        .await
        .unwrap_or_else(|message| panic!("initial document should install: {message}"));
}

async fn evaluate_string_for_test(
    conn: &mut CdpConnection,
    context: DevToolsCommandContext,
    expression: &str,
    label: &str,
) -> String {
    let (evaluate_result, _) = conn
        .execute_devtools_command(DevToolsCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
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
            },
        ))
        .await
        .into_parts();
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
    command: DevToolsCommand,
    request_id: &str,
) {
    let mut conn = connection_with_background_pending_fetch_action(request_id);
    assert!(matches!(
        conn.pending_fetch_request_session_route(request_id),
        Some(CdpSessionRoute::PageTarget { target_id, .. }) if target_id == "TID-background"
    ));

    let (result, events, protocol_events, renderer_output_predecessor) = conn
        .execute_devtools_command(command)
        .await
        .into_complete_parts();

    assert!(events.is_empty());
    assert!(protocol_events.is_empty());
    assert!(renderer_output_predecessor.is_none());
    assert_eq!(
        result.expect("BiDi request action should resolve background owner"),
        DevToolsCommandResult::Empty
    );
    assert!(
        conn.pending_fetch_request_session_route(request_id)
            .is_none(),
        "resolved background request should be consumed"
    );
    assert_eq!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .active_target_id(),
        Some("TID-active"),
        "resolving a BiDi request id must not activate the background target"
    );
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
        last_access_index: 0,
    }
}

async fn complete_command_task_for_test(
    conn: &mut CdpConnection,
    mut pending: PendingCdpCommandDispatch,
) -> Vec<Value> {
    loop {
        match conn
            .complete_pending_command_dispatch(pending.wait().await)
            .await
        {
            CdpCommandTaskStep::Complete(outcome) => return outcome.into_parts().0,
            CdpCommandTaskStep::Pending(next) => pending = *next,
        }
    }
}

async fn attach_page_session_for_test(conn: &mut CdpConnection, target_id: &str) -> String {
    let raw = serde_json::to_string(&json!({
        "id": 9_900_001,
        "method": "Target.attachToTarget",
        "params": { "targetId": target_id, "flatten": true }
    }))
    .expect("attach command should serialize");
    let messages = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => {
            complete_command_task_for_test(conn, *pending).await
        }
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
    conn: &mut CdpConnection,
    context: DevToolsCommandContext,
) -> serde_json::Value {
    let (result, _) = conn
        .execute_devtools_command(DevToolsCommand::EvaluateScript(
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
        .into_parts();
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
