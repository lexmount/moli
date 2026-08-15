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
    let pending = {
        let previous_route = conn.replace_none_session_owner_route_override(Some(route));
        let pending = conn
            .start_initial_document_page_ensure_for_session_owner(None)
            .unwrap_or_else(|message| {
                panic!(
                    "target lifecycle ensure should start for {}: {message}",
                    target_id.as_str()
                )
            });
        conn.replace_none_session_owner_route_override(previous_route);
        pending
    };
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
    browser_context
        .background_targets
        .push(BackgroundTarget::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "https://example.test/background".to_owned(),
        ));
    let mut fetch_state = browser_context.take_parked_fetch_state("TID-background");
    fetch_state.insert_pending_fetch_request_id_for_test(request_id.to_owned());
    browser_context.replace_parked_fetch_state("TID-background".to_owned(), fetch_state);
    conn.browser_context = Some(browser_context);
    conn
}

async fn assert_bidi_fetch_action_consumes_background_request(
    command: DevToolsCommand,
    request_id: &str,
) {
    let mut conn = connection_with_background_pending_fetch_action(request_id);
    assert!(matches!(
        conn.pending_fetch_request_session_route(request_id),
        Some(CdpSessionRoute::BackgroundTarget { target_id, .. }) if target_id == "TID-background"
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
        "resolving a BiDi request id must not promote the background target"
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

#[tokio::test]
async fn devtools_browser_context_create_uses_ephemeral_storage_partition() {
    let initial_storage_owner = StoragePartitionState::open(None).expect("memory partition");
    let initial_shared_storage = initial_storage_owner.shared_storage_handles();
    let initial_local_storage = initial_shared_storage.web_storage_store();
    assert!(initial_local_storage.lock().set_item(
        "https://example.com",
        "profile-key",
        "profile-value"
    ));
    let initial_storage_partition = CdpInitialStoragePartition::from_storage_partition(
        vec![stored_cookie_for_dispatch_test("sid", "seeded")],
        &initial_storage_owner,
    );
    let mut conn = CdpConnection::new_with_initial_storage_partition(initial_storage_partition);
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (create_result, create_events) = conn
        .execute_devtools_command(DevToolsCommand::CreateBrowserContext(
            DevToolsCreateBrowserContextCommand {
                context,
                browser_context_id: None,
                accept_insecure_certs: None,
                proxy_server: None,
                proxy_bypass_list: None,
                proxy_autoconfig_url: None,
                proxy_socks_version: None,
                persistent_partition_id: None,
            },
        ))
        .await
        .into_parts();

    assert!(create_events.is_empty());
    let DevToolsCommandResult::CreateBrowserContext(create_result) =
        create_result.expect("create browser context should succeed")
    else {
        panic!("expected create browser context result");
    };
    let created_context = conn
        .browser_context_by_id(create_result.browser_context_id.as_str())
        .expect("created browser context");
    assert!(!created_context.is_profile_backed_storage_partition());
    assert!(created_context.snapshot_cookies().is_empty());
    assert_eq!(
        created_context
            .web_storage_store_for_test()
            .lock()
            .get_item("https://example.com", "profile-key"),
        None
    );
    assert!(conn.snapshot_profile_backed_cookies().is_none());
}

#[tokio::test]
async fn devtools_browser_context_create_rejects_persistent_partition_id() {
    for (partition_id, expected_message) in [
        ("tenant-a", "PersistentBrowserContextNotSupported"),
        ("default", "DefaultPersistentBrowserContextNotAllowed"),
        ("tenant/a", "InvalidPersistentBrowserContextId"),
    ] {
        let mut conn = CdpConnection::new();
        let context = DevToolsCommandContext {
            protocol: DevToolsProtocol::WebDriverBidi,
            session_id: Some(DevToolsSessionId::from("bidi-session-1")),
            target_id: None,
            browser_context_id: None,
        };

        let (create_result, create_events) = conn
            .execute_devtools_command(DevToolsCommand::CreateBrowserContext(
                DevToolsCreateBrowserContextCommand {
                    context,
                    browser_context_id: None,
                    accept_insecure_certs: None,
                    proxy_server: None,
                    proxy_bypass_list: None,
                    proxy_autoconfig_url: None,
                    proxy_socks_version: None,
                    persistent_partition_id: Some(partition_id.to_owned()),
                },
            ))
            .await
            .into_parts();

        assert!(create_events.is_empty());
        let error = create_result.expect_err("persistent partition id should fail closed");
        assert_eq!(error.kind, DevToolsErrorKind::InvalidArgument);
        assert_eq!(error.message, expected_message);
        assert!(
            conn.browser_contexts().next().is_none(),
            "{partition_id:?} must not create an ephemeral fallback context"
        );
    }
}

#[tokio::test]
async fn devtools_command_executes_target_create_and_get_targets() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let create = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await;
    let (create_result, create_events) = create.into_parts();
    assert!(create_events.is_empty());
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    assert_eq!(create_result.target_id.as_str(), "TID-1");

    let get_targets = conn
        .execute_devtools_command(DevToolsCommand::GetTargets(DevToolsGetTargetsCommand {
            context,
            root: Some(create_result.target_id.clone()),
            max_depth: None,
            filter: None,
        }))
        .await;
    let (get_targets_result, get_targets_events) = get_targets.into_parts();
    assert!(get_targets_events.is_empty());
    let DevToolsCommandResult::GetTargets(get_targets_result) =
        get_targets_result.expect("get targets should succeed")
    else {
        panic!("expected get targets result");
    };

    assert_eq!(get_targets_result.targets.len(), 1);
    let target = &get_targets_result.targets[0];
    assert_eq!(
        target.target_id.as_ref().map(|id| id.as_str()),
        Some("TID-1")
    );
    assert_eq!(target.url, "about:blank");
    assert_eq!(
        target.browser_context_id.as_ref().map(|id| id.as_str()),
        Some("BID-1")
    );
}

#[tokio::test]
async fn devtools_script_navigation_exact_cursor_rejects_replaced_page_owner_action() {
    let mut ctx = crate::testing::TestContext::new_with_target_discovery(false);
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let create_outcome = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: true,
        }))
        .await;
    let create_predecessor = create_outcome.renderer_output_predecessor();
    let (create_result, create_events) = create_outcome.into_parts();
    assert!(create_events.is_empty());
    if let Some(predecessor) = create_predecessor {
        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
            .await;
    }
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id;
    ctx.install_navigation_fixture_for_session_owner("about:blank", None)
        .await;
    let target_context = DevToolsCommandContext {
        target_id: Some(target_id.clone()),
        ..context
    };

    let outcome = ctx
        .conn
        .execute_devtools_command_with_protocol_events(DevToolsCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: target_context,
                realm_id: None,
                world_name: None,
                expression: "location.href = 'https://example.test/next'; 'navigation-queued'"
                    .to_owned(),
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            },
        ))
        .await;
    let (result, scheduler_events, protocol_events, predecessor) = outcome.into_complete_parts();
    let predecessor =
        predecessor.expect("script navigation should settle one exact renderer cursor");
    let value = expect_script_value_result(
        result.expect("script navigation should evaluate"),
        "expected script result",
    );
    assert_eq!(value.value, json!("navigation-queued"));
    assert!(
        protocol_events.is_empty(),
        "producing an owner action must not emit a listener-shaped protocol event: {protocol_events:?}"
    );
    assert!(
        scheduler_events.is_empty(),
        "the owner action belongs to the concrete renderer publication, not a command-local side channel"
    );

    let route = ctx
        .conn
        .target_session_route_for_target_id(target_id.as_str())
        .expect("created target route");
    {
        let mut route_scope = ctx
            .conn
            .scoped_none_session_owner_route_override(route.clone());
        route_scope
            .conn_mut()
            .runtime_session_owner_slot_mut(None)
            .expect("created target runtime slot")
            .replace_page_attachment_id_for_test();
    }

    let sent_start = ctx.sent.len();
    ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
        .await;
    ctx.wait_for_direct_command_work_completion_for_test("stale script-navigation owner action")
        .await;
    assert!(
        ctx.sent[sent_start..].is_empty(),
        "work claimed from a replaced Page must not emit protocol output for the replacement: {:?}",
        &ctx.sent[sent_start..]
    );
}

#[tokio::test]
async fn devtools_fetch_control_command_routes_through_fetch_owner() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-fetch-control".to_owned()));
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let outcome = conn
        .execute_devtools_command(DevToolsCommand::FailInterceptedRequest(
            DevToolsFailInterceptedRequestCommand {
                context: context.clone(),
                request_id: DevToolsRequestId::from("INT-99"),
                error_text: "Failed".to_owned(),
            },
        ))
        .await;
    let (result, events) = outcome.into_parts();

    assert!(events.is_empty());
    let error = result.expect_err("missing Fetch request should be a Fetch owner error");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchRequest);
    assert_eq!(error.message, "RequestNotFound");

    let outcome = conn
        .execute_devtools_command(DevToolsCommand::ContinueWithAuth(
            DevToolsContinueWithAuthCommand {
                context,
                request_id: DevToolsRequestId::from("foo"),
                action: DevToolsAuthChallengeAction::Cancel,
                username: None,
                password: None,
            },
        ))
        .await;
    let (result, events) = outcome.into_parts();

    assert!(events.is_empty());
    let error =
        result.expect_err("BiDi opaque missing auth request id should be a no-such-request error");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchRequest);
    assert_eq!(error.message, "RequestNotFound");
}

#[tokio::test]
async fn bidi_fetch_control_resolves_background_request_owner() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-fetch-background".to_owned());
    browser_context.set_active_target_id("TID-active".to_owned());
    browser_context.attach_active_session("SID-active".to_owned());
    browser_context
        .background_targets
        .push(BackgroundTarget::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "https://example.test/background".to_owned(),
        ));
    conn.browser_context = Some(browser_context);

    conn.register_pending_fetch_navigation_request_for_session_owner(
        Some("SID-background"),
        PendingFetchNavigation {
            fetch_request_id: "FETCH-background".to_owned(),
            interception_session_id: Some("bidi-session-1".to_owned()),
            document_navigation_token: None,
            navigation: NavigationDispatchState {
                navigate_id: None,
                navigate_session_id: Some("SID-background".to_owned()),
                result_projection: NavigationResultProjection::WebDriverBidi(json!({})),
                frame_id: "TID-background".to_owned(),
                session_id: Some("SID-background".to_owned()),
                request_id: Some("NETWORK-background".to_owned()),
                loader_id: "LOADER-background".to_owned(),
                request_announced: false,
                requested_url: url::Url::parse("https://example.test/background").unwrap(),
                request_method: "GET".to_owned(),
                request_body: None,
                request_body_bytes: None,
                request_headers: Vec::new(),
                request_load_policy: crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
                timestamp: 0.0,
                source_document_security: Default::default(),
            },
            request_cookie_report: None,
            intercept_response: false,
            response_stage_url_match_policy: ResponseStageUrlMatchPolicy::AlreadyMatched,
            auth_required_blocked_intercepts: Vec::new(),
        },
    )
    .expect("background pending navigation should register");

    assert!(matches!(
        conn.pending_fetch_request_session_route("FETCH-background"),
        Some(CdpSessionRoute::BackgroundTarget { target_id, .. }) if target_id == "TID-background"
    ));

    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let outcome = conn
        .execute_devtools_command(DevToolsCommand::FailInterceptedRequest(
            DevToolsFailInterceptedRequestCommand {
                context,
                request_id: DevToolsRequestId::from("FETCH-background"),
                error_text: "Failed".to_owned(),
            },
        ))
        .await;
    let (result, _) = outcome.into_parts();

    assert_eq!(
        result.expect("BiDi failRequest should resolve background owner"),
        DevToolsCommandResult::Empty
    );
    assert!(
        conn.pending_fetch_request_session_route("FETCH-background")
            .is_none(),
        "resolved background request should be consumed"
    );
    assert_eq!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .active_target_id(),
        Some("TID-active"),
        "resolving a BiDi request id must not promote the background target"
    );
}

#[tokio::test]
async fn bidi_fetch_request_actions_resolve_background_request_owner() {
    assert_bidi_fetch_action_consumes_background_request(
        DevToolsCommand::ContinueInterceptedRequest(DevToolsContinueInterceptedRequestCommand {
            context: bidi_fetch_command_context(),
            request_id: DevToolsRequestId::from("FETCH-background-continue-request"),
            url: None,
            method: None,
            post_data: None,
            headers: None,
            intercept_response: false,
        }),
        "FETCH-background-continue-request",
    )
    .await;

    assert_bidi_fetch_action_consumes_background_request(
        DevToolsCommand::ContinueInterceptedResponse(DevToolsContinueInterceptedResponseCommand {
            context: bidi_fetch_command_context(),
            request_id: DevToolsRequestId::from("FETCH-background-continue-response"),
            response_code: None,
            response_headers: None,
            response_phrase: None,
            auth_credentials: None,
        }),
        "FETCH-background-continue-response",
    )
    .await;

    assert_bidi_fetch_action_consumes_background_request(
        DevToolsCommand::FulfillInterceptedRequest(DevToolsFulfillInterceptedRequestCommand {
            context: bidi_fetch_command_context(),
            request_id: DevToolsRequestId::from("FETCH-background-provide-response"),
            response_code: 204,
            response_headers: Vec::new(),
            body: None,
            response_phrase: None,
        }),
        "FETCH-background-provide-response",
    )
    .await;
}

#[tokio::test]
async fn devtools_browser_context_commands_create_list_and_remove_user_context() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (create_result, create_events) = conn
        .execute_devtools_command(DevToolsCommand::CreateBrowserContext(
            DevToolsCreateBrowserContextCommand {
                context: context.clone(),
                browser_context_id: None,
                accept_insecure_certs: Some(true),
                proxy_server: Some("127.0.0.1:80".to_owned()),
                proxy_bypass_list: Some("localhost,127.0.0.1".to_owned()),
                proxy_autoconfig_url: None,
                proxy_socks_version: None,
                persistent_partition_id: None,
            },
        ))
        .await
        .into_parts();
    assert!(create_events.is_empty());
    let DevToolsCommandResult::CreateBrowserContext(create_result) =
        create_result.expect("create browser context should succeed")
    else {
        panic!("expected create browser context result");
    };
    assert_eq!(create_result.browser_context_id.as_str(), "user-context-1");
    let created_context = conn
        .browser_context_by_id("user-context-1")
        .expect("created browser context");
    assert_eq!(created_context.tls_verify_host_override, Some(false));
    assert_eq!(
        created_context.http_proxy_override.as_deref(),
        Some("127.0.0.1:80")
    );
    assert_eq!(
        created_context.http_no_proxy_override.as_deref(),
        Some("localhost,127.0.0.1")
    );
    assert_eq!(created_context.proxy_autoconfig_url, None);
    assert_eq!(created_context.proxy_socks_version, None);

    let (get_contexts_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetBrowserContexts(
            DevToolsGetBrowserContextsCommand {
                context: context.clone(),
            },
        ))
        .await
        .into_parts();
    let DevToolsCommandResult::GetBrowserContexts(get_contexts_result) =
        get_contexts_result.expect("get browser contexts should succeed")
    else {
        panic!("expected get browser contexts result");
    };
    assert!(
        get_contexts_result
            .browser_context_ids
            .iter()
            .any(|id| id.as_str() == "user-context-1")
    );

    let (create_target_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: Some(create_result.browser_context_id.clone()),
            activate: true,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_target_result) =
        create_target_result.expect("create target in user context should succeed")
    else {
        panic!("expected create target result");
    };

    let (remove_result, _, remove_events) = conn
        .execute_devtools_command_with_protocol_events(DevToolsCommand::RemoveBrowserContext(
            DevToolsRemoveBrowserContextCommand {
                context,
                browser_context_id: create_result.browser_context_id,
            },
        ))
        .await
        .into_parts_with_protocol_events();
    assert_eq!(
        remove_result.expect("remove browser context should succeed"),
        DevToolsCommandResult::Empty
    );
    assert!(!conn.has_browser_context_id("user-context-1"));
    assert!(
        remove_events.iter().all(|event| event
            .protocol_message()
            .and_then(|message| message.get("id"))
            != Some(&json!(null))),
        "direct BrowserContext removal must not route dispose command responses as protocol events"
    );
    let mut saw_destroyed = false;
    for event in remove_events {
        let (message, automation_event) = event.into_parts();
        let Some(AutomationEvent::TargetDestroyed(event)) = automation_event else {
            continue;
        };
        if event.target_id.as_str() != create_target_result.target_id.as_str() {
            continue;
        }
        assert_eq!(message["method"], json!("Moli.automationOnly"));
        assert_eq!(
            event.browser_context_id.as_ref().map(|id| id.as_str()),
            Some("user-context-1")
        );
        saw_destroyed = true;
    }
    assert!(
        saw_destroyed,
        "removing a user context should emit TargetDestroyed automation for owned targets"
    );
}

#[tokio::test]
async fn devtools_browser_context_create_preserves_proxy_autoconfig_and_socks_metadata() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (create_result, create_events) = conn
        .execute_devtools_command(DevToolsCommand::CreateBrowserContext(
            DevToolsCreateBrowserContextCommand {
                context,
                browser_context_id: None,
                accept_insecure_certs: None,
                proxy_server: Some("socks5://[::1]:1080".to_owned()),
                proxy_bypass_list: None,
                proxy_autoconfig_url: Some("http://proxy.test/proxy.pac".to_owned()),
                proxy_socks_version: Some(5),
                persistent_partition_id: None,
            },
        ))
        .await
        .into_parts();
    assert!(create_events.is_empty());
    let DevToolsCommandResult::CreateBrowserContext(create_result) =
        create_result.expect("create browser context should succeed")
    else {
        panic!("expected create browser context result");
    };
    let created_context = conn
        .browser_context_by_id(create_result.browser_context_id.as_str())
        .expect("created browser context");
    assert_eq!(
        created_context.http_proxy_override.as_deref(),
        Some("socks5://[::1]:1080")
    );
    assert_eq!(
        created_context.proxy_autoconfig_url.as_deref(),
        Some("http://proxy.test/proxy.pac")
    );
    assert_eq!(created_context.proxy_socks_version, Some(5));
}

#[tokio::test]
async fn devtools_remove_browser_context_rejects_unknown_user_context() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (remove_result, _, remove_events) = conn
        .execute_devtools_command_with_protocol_events(DevToolsCommand::RemoveBrowserContext(
            DevToolsRemoveBrowserContextCommand {
                context,
                browser_context_id: DevToolsBrowserContextId::from("missing-user-context"),
            },
        ))
        .await
        .into_parts_with_protocol_events();

    assert!(remove_events.is_empty());
    let error = remove_result.expect_err("unknown user context should fail");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchTarget);
    assert_eq!(error.message, "UnknownBrowserContextId");
}

#[tokio::test]
async fn devtools_create_target_explicit_default_browser_context_materializes_default_owner() {
    let mut conn = CdpConnection::new();
    let user_context = conn.new_browser_context("user-context-1".to_owned());
    conn.insert_browser_context(user_context);

    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                target_id: None,
                browser_context_id: Some(DevToolsBrowserContextId::from("BID-default")),
            },
            url: "about:blank".to_owned(),
            browser_context_id: Some(DevToolsBrowserContextId::from("BID-default")),
            activate: true,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target in explicit default context should succeed")
    else {
        panic!("expected create target result");
    };

    assert_eq!(
        conn.browser_context
            .as_ref()
            .expect("active browser context")
            .id,
        "user-context-1",
        "explicit default target creation should restore the previously active browser context"
    );
    let default_context = conn
        .browser_context_by_id("BID-default")
        .expect("default browser context should be materialized");
    assert_eq!(
        default_context.active_target_id(),
        Some(create_result.target_id.as_str()),
        "new target should belong to the default browser context"
    );
}

#[tokio::test]
async fn devtools_create_target_uses_reference_target_browser_context_when_unspecified() {
    let mut conn = CdpConnection::new();
    let mut default_context = BrowserContext::new("BID-default".to_owned());
    default_context.set_active_target_id("TID-default".to_owned());
    let mut reference_context = BrowserContext::new("BID-reference".to_owned());
    reference_context.set_active_target_id("TID-reference".to_owned());
    conn.insert_browser_context(default_context);
    conn.insert_browser_context(reference_context);

    let (create_result, create_events) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                target_id: Some(DevToolsTargetId::from("TID-reference")),
                browser_context_id: None,
            },
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: true,
        }))
        .await
        .into_parts();
    assert!(create_events.is_empty());
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target from reference context should succeed")
    else {
        panic!("expected create target result");
    };

    assert_eq!(
        conn.browser_context
            .as_ref()
            .expect("active browser context")
            .id,
        "BID-default",
        "reference-context target creation should restore the previously active browser context"
    );
    let reference_context = conn
        .browser_context_by_id("BID-reference")
        .expect("reference browser context");
    assert_eq!(
        reference_context.active_target_id(),
        Some(create_result.target_id.as_str()),
        "new target should be active in the reference context's browser context"
    );
    assert!(
        reference_context
            .background_target("TID-reference")
            .is_some(),
        "the reference context's previous active target should be demoted inside the same browser context"
    );
}

#[tokio::test]
async fn devtools_create_target_explicit_browser_context_overrides_reference_target() {
    let mut conn = CdpConnection::new();
    let mut default_context = BrowserContext::new("BID-default".to_owned());
    default_context.set_active_target_id("TID-default".to_owned());
    let mut reference_context = BrowserContext::new("BID-reference".to_owned());
    reference_context.set_active_target_id("TID-reference".to_owned());
    let mut explicit_context = BrowserContext::new("BID-explicit".to_owned());
    explicit_context.set_active_target_id("TID-explicit".to_owned());
    conn.insert_browser_context(default_context);
    conn.insert_browser_context(reference_context);
    conn.insert_browser_context(explicit_context);

    let (create_result, create_events) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                target_id: Some(DevToolsTargetId::from("TID-reference")),
                browser_context_id: Some(DevToolsBrowserContextId::from("BID-explicit")),
            },
            url: "about:blank".to_owned(),
            browser_context_id: Some(DevToolsBrowserContextId::from("BID-explicit")),
            activate: true,
        }))
        .await
        .into_parts();
    assert!(create_events.is_empty());
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("explicit browser context target creation should succeed")
    else {
        panic!("expected create target result");
    };

    assert_eq!(
        conn.browser_context
            .as_ref()
            .expect("active browser context")
            .id,
        "BID-default",
        "explicit browser-context target creation should restore the previously active browser context"
    );
    let explicit_context = conn
        .browser_context_by_id("BID-explicit")
        .expect("explicit browser context");
    assert_eq!(
        explicit_context.active_target_id(),
        Some(create_result.target_id.as_str()),
        "new target should be active in the explicit browser context"
    );
    let reference_context = conn
        .browser_context_by_id("BID-reference")
        .expect("reference browser context");
    assert_eq!(
        reference_context.active_target_id(),
        Some("TID-reference"),
        "reference context should not be demoted when explicit browser context is provided"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn devtools_runtime_call_function_popup_activity_drains_from_protocol_neutral_command() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    ctx.conn.set_root_target_discovery_enabled(true);
    let mut browser_context = BrowserContext::new("BID-neutral-popup".to_owned());
    browser_context.set_active_target_id("TID-neutral-popup-opener".to_owned());
    browser_context.attach_active_session("SID-neutral-popup-opener");
    ctx.conn.browser_context = Some(browser_context);
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<p>neutral popup opener</p>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("SID-neutral-popup-opener")),
        target_id: Some(DevToolsTargetId::from("TID-neutral-popup-opener")),
        browser_context_id: None,
    };
    let (call_result, scheduler_events, protocol_events, renderer_output_predecessor) = ctx
        .conn
        .execute_devtools_command_with_protocol_events(DevToolsCommand::CallFunction(
            DevToolsCallFunctionCommand {
            context,
            realm_id: None,
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration:
                "() => window.open('data:text/html,<main>neutral popup</main>', '_blank') !== null"
                    .to_owned(),
            arguments: Vec::new(),
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::None,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
            },
        ))
        .await
        .into_complete_parts();
    let call_result = expect_script_value_result(
        call_result.expect("protocol-neutral callFunction should succeed"),
        "expected callFunction value result",
    );
    assert_eq!(call_result.value, json!(true));

    if let Some(predecessor) = renderer_output_predecessor {
        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
            .await;
    }
    ctx.route_direct_command_output_for_test(protocol_events, scheduler_events)
        .await;
    let created = ctx
        .wait_for_scheduler_message("protocol-neutral popup Target.targetCreated", |message| {
            message["method"] == json!("Target.targetCreated")
        })
        .await;
    assert_eq!(
        created["params"]["targetInfo"]["browserContextId"],
        json!("BID-neutral-popup")
    );
    assert_eq!(
        created["params"]["targetInfo"]["openerId"],
        json!("TID-neutral-popup-opener")
    );
    let popup_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("popup target id")
        .to_owned();
    let popup_url = "data:text/html,<main>neutral popup</main>".to_owned();
    let popup_navigate = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("SID-neutral-popup-opener")),
                target_id: Some(DevToolsTargetId::from(popup_target_id.clone())),
                browser_context_id: None,
            },
            url: popup_url,
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await;
    popup_navigate.expect("popup target navigate should load inline document");
    let popup_eval_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("SID-neutral-popup-opener")),
                target_id: Some(DevToolsTargetId::from(popup_target_id)),
                browser_context_id: None,
            },
            realm_id: None,
            world_name: None,
            expression: "document.querySelector('main').textContent".to_owned(),
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
    let popup_eval_result = expect_script_value_result(
        popup_eval_result.expect("popup target evaluate should observe loaded inline document"),
        "expected popup target value result",
    );
    assert_eq!(popup_eval_result.value, json!("neutral popup"));
}

#[tokio::test]
async fn devtools_create_target_rejects_unknown_reference_target() {
    let mut conn = CdpConnection::new();
    let mut default_context = BrowserContext::new("BID-default".to_owned());
    default_context.set_active_target_id("TID-default".to_owned());
    conn.insert_browser_context(default_context);

    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                target_id: Some(DevToolsTargetId::from("TID-missing")),
                browser_context_id: None,
            },
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: true,
        }))
        .await
        .into_parts();
    let error = create_result.expect_err("unknown reference context should fail");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchTarget);
}

#[tokio::test]
async fn devtools_command_preserves_target_create_typed_sidecar() {
    let mut conn = CdpConnection::new();
    conn.set_root_target_discovery_enabled(true);
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (result, scheduler_events, mut protocol_events) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts_with_protocol_events();

    assert!(scheduler_events.is_empty());
    result.expect("create target should succeed");
    assert_eq!(protocol_events.len(), 1);
    let (_message, automation_event) = protocol_events.remove(0).into_parts();
    let Some(AutomationEvent::TargetCreated(event)) = automation_event else {
        panic!("expected targetCreated typed sidecar");
    };
    assert_eq!(event.target_id.as_str(), "TID-1");
}

#[tokio::test]
async fn devtools_command_preserves_target_close_detached_typed_sidecar() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-close-sidecar".to_owned());
    browser_context.set_active_target_id("TID-close-sidecar".to_owned());
    browser_context.attach_active_session("SID-close-sidecar".to_owned());
    conn.browser_context = Some(browser_context);

    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: Some(DevToolsTargetId::from("TID-close-sidecar")),
        browser_context_id: None,
    };
    let (result, scheduler_events, protocol_events) = conn
        .execute_devtools_command_with_protocol_events(DevToolsCommand::CloseTarget(
            DevToolsCloseTargetCommand {
                context,
                target_id: DevToolsTargetId::from("TID-close-sidecar"),
            },
        ))
        .await
        .into_parts_with_protocol_events();

    assert!(scheduler_events.is_empty());
    assert_eq!(
        result.expect("close target should succeed"),
        DevToolsCommandResult::CloseTarget(crate::devtools_runtime::DevToolsCloseTargetResult {
            success: true,
        })
    );
    let mut saw_detached = false;
    for event in protocol_events {
        let (message, automation_event) = event.into_parts();
        if message["method"] != json!("Target.detachedFromTarget") {
            continue;
        }
        let Some(AutomationEvent::TargetDetached(event)) = automation_event else {
            panic!("target detached protocol event should retain typed sidecar");
        };
        assert_eq!(event.target_id.as_str(), "TID-close-sidecar");
        assert_eq!(event.session_id.as_str(), "SID-close-sidecar");
        assert_eq!(event.reason.as_deref(), Some("Render process gone."));
        saw_detached = true;
    }
    assert!(
        saw_detached,
        "close target should emit TargetDetached sidecar"
    );
}

#[tokio::test]
async fn devtools_command_preserves_remove_browser_context_detached_typed_sidecar() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("user-context-detach-sidecar".to_owned());
    browser_context.set_active_target_id("TID-dispose-sidecar".to_owned());
    browser_context.attach_active_session("SID-dispose-sidecar".to_owned());
    conn.insert_browser_context(browser_context);

    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (result, scheduler_events, protocol_events) = conn
        .execute_devtools_command_with_protocol_events(DevToolsCommand::RemoveBrowserContext(
            DevToolsRemoveBrowserContextCommand {
                context,
                browser_context_id: DevToolsBrowserContextId::from("user-context-detach-sidecar"),
            },
        ))
        .await
        .into_parts_with_protocol_events();

    assert!(scheduler_events.is_empty());
    assert_eq!(
        result.expect("remove browser context should succeed"),
        DevToolsCommandResult::Empty
    );
    let mut saw_detached = false;
    for event in protocol_events {
        let (message, automation_event) = event.into_parts();
        if message["method"] != json!("Target.detachedFromTarget") {
            continue;
        }
        let Some(AutomationEvent::TargetDetached(event)) = automation_event else {
            panic!("target detached protocol event should retain typed sidecar");
        };
        assert_eq!(event.target_id.as_str(), "TID-dispose-sidecar");
        assert_eq!(event.session_id.as_str(), "SID-dispose-sidecar");
        assert_eq!(event.reason.as_deref(), Some("Render process gone."));
        saw_detached = true;
    }
    assert!(
        saw_detached,
        "remove browser context should emit TargetDetached sidecar"
    );
}

#[tokio::test]
async fn devtools_command_executes_target_pending_activate_and_close() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    for _ in 0..2 {
        let (result, _) = conn
            .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }))
            .await
            .into_parts();
        result.expect("create target should succeed");
    }

    let (activate_result, _) = conn
        .execute_devtools_command(DevToolsCommand::ActivateTarget(
            DevToolsActivateTargetCommand {
                context: context.clone(),
                target_id: DevToolsTargetId::from("TID-2"),
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        activate_result.expect("activate should succeed"),
        DevToolsCommandResult::Empty
    );

    let (close_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CloseTarget(DevToolsCloseTargetCommand {
            context: context.clone(),
            target_id: DevToolsTargetId::from("TID-2"),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CloseTarget(close_result) =
        close_result.expect("close should succeed")
    else {
        panic!("expected close target result");
    };
    assert!(close_result.success);

    let (remaining_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetTargets(DevToolsGetTargetsCommand {
            context,
            root: Some(DevToolsTargetId::from("TID-2")),
            max_depth: None,
            filter: None,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::GetTargets(remaining_result) =
        remaining_result.expect("get targets should succeed")
    else {
        panic!("expected get targets result");
    };
    assert!(remaining_result.targets.is_empty());
}

#[tokio::test]
async fn devtools_runtime_command_uses_background_initial_document_without_resolver_fallback() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (first_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
    let DevToolsCommandResult::CreateTarget(first_result) =
        first_result.expect("initial target create should succeed")
    else {
        panic!("expected create target result");
    };
    let first_target_id = first_result.target_id;
    ensure_initial_document_for_target_id_for_test(&mut conn, &first_target_id).await;

    let (second_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank#background".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(second_result) =
        second_result.expect("background target create should succeed")
    else {
        panic!("expected create target result");
    };
    let second_target_id = second_result.target_id;
    ensure_initial_document_for_target_id_for_test(&mut conn, &second_target_id).await;

    assert_eq!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .active_target_id(),
        Some(first_target_id.as_str())
    );
    let background_load_inputs = {
        let route = conn
            .target_session_route_for_target_id(second_target_id.as_str())
            .expect("background target route");
        let previous_route = conn.replace_none_session_owner_route_override(Some(route));
        let load_inputs = conn.navigation_load_inputs_for_session_owner(None);
        conn.replace_none_session_owner_route_override(previous_route);
        load_inputs
    };
    assert!(
        background_load_inputs
            .document_start_scripts
            .iter()
            .any(|script| script
                .source
                .contains("defineGetter(document, 'hidden', () => true)")),
        "background initial document lifecycle should include parked document surface script"
    );

    let (name_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
            context: DevToolsCommandContext {
                target_id: Some(second_target_id.clone()),
                ..context.clone()
            },
            realm_id: None,
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration:
                "function() { return [location.href, window.name, window.opener]; }".to_owned(),
            arguments: Vec::new(),
            await_promise: true,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::None,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        }))
        .await
        .into_parts();
    let name_result = expect_script_value_result(
        name_result.expect("background about:blank function should succeed"),
        "expected script value",
    );
    assert_eq!(
        name_result.value,
        json!(["about:blank#background", "", null])
    );

    let (surface_result, _) = conn
        .execute_devtools_command(DevToolsCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: DevToolsCommandContext {
                    target_id: Some(second_target_id.clone()),
                    ..context.clone()
                },
                realm_id: None,
                world_name: None,
                expression:
                    "JSON.stringify({ hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState })"
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
    let surface_result = expect_script_value_result(
        surface_result.expect("background surface evaluate should succeed"),
        "expected background surface value",
    );
    let surface_payload = surface_result
        .value
        .as_str()
        .expect("background surface should be a JSON string");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(surface_payload)
            .expect("background surface JSON should parse"),
        json!({
            "hasFocus": false,
            "hidden": true,
            "visibilityState": "hidden"
        }),
        "default background initial document should install parked document surfaces"
    );

    let browser_context = conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        browser_context.active_target_id(),
        Some(first_target_id.as_str()),
        "protocol-neutral runtime commands should not activate background targets"
    );
    assert!(
        browser_context
            .background_target(second_target_id.as_str())
            .is_some_and(|target| target.has_loaded_page()),
        "background initial document should already be available for script execution"
    );
}

#[tokio::test]
async fn protocol_neutral_await_promise_keeps_background_owner_route_across_pending_completion() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-owner-route")),
        target_id: None,
        browser_context_id: None,
    };

    let (first_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
    let DevToolsCommandResult::CreateTarget(first_result) =
        first_result.expect("initial target create should succeed")
    else {
        panic!("expected create target result");
    };
    let first_target_id = first_result.target_id;
    ensure_initial_document_for_target_id_for_test(&mut conn, &first_target_id).await;

    let (second_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(second_result) =
        second_result.expect("background target create should succeed")
    else {
        panic!("expected create target result");
    };
    let second_target_id = second_result.target_id;
    ensure_initial_document_for_target_id_for_test(&mut conn, &second_target_id).await;

    let first_context = DevToolsCommandContext {
        target_id: Some(first_target_id.clone()),
        ..context.clone()
    };
    let second_context = DevToolsCommandContext {
        target_id: Some(second_target_id.clone()),
        ..context.clone()
    };
    let first_owner = evaluate_string_for_test(
        &mut conn,
        first_context.clone(),
        "globalThis.__ownerRouteProbe = 'active'; globalThis.__ownerRouteProbe",
        "active owner marker",
    )
    .await;
    assert_eq!(first_owner, "active");
    let second_owner = evaluate_string_for_test(
        &mut conn,
        second_context.clone(),
        "globalThis.__ownerRouteProbe = 'background'; globalThis.__ownerRouteProbe",
        "background owner marker",
    )
    .await;
    assert_eq!(second_owner, "background");

    let step = conn
        .start_devtools_runtime_command_dispatch(DevToolsCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: second_context,
                realm_id: None,
                world_name: None,
                expression: "Promise.resolve(globalThis.__ownerRouteProbe)".to_owned(),
                await_promise: true,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            },
        ))
        .await;
    let pending = match step {
        DevToolsRuntimeCommandTaskStep::Pending(pending) => *pending,
        DevToolsRuntimeCommandTaskStep::Complete(_) => {
            panic!("awaitPromise protocol-neutral runtime command should pend")
        }
    };

    let active_route = conn
        .target_session_route_for_target_id(first_target_id.as_str())
        .expect("active target route");
    let previous_route = conn.replace_none_session_owner_route_override(Some(active_route));
    assert!(
        !conn.has_pending_inspector_awaits_for_session_owner(None),
        "internal id 0 pending await must not be registered on the active owner"
    );
    conn.replace_none_session_owner_route_override(previous_route);

    let background_route = conn
        .target_session_route_for_target_id(second_target_id.as_str())
        .expect("background target route");
    let previous_route =
        conn.replace_none_session_owner_route_override(Some(background_route.clone()));
    assert!(
        conn.has_pending_inspector_awaits_for_session_owner(None),
        "internal id 0 pending await must be registered on the targeted background owner"
    );
    conn.replace_none_session_owner_route_override(previous_route);

    let completed = pending.wait().await;
    let step = conn
        .complete_devtools_runtime_command_dispatch(completed)
        .await;
    let outcome = match step {
        DevToolsRuntimeCommandTaskStep::Complete(outcome) => outcome,
        DevToolsRuntimeCommandTaskStep::Pending(mut pending) => {
            pending
                .wait_for_scheduler_deferred_inspector_reply_receiver(&mut conn)
                .await
                .expect("settled awaitPromise protocol-neutral receiver should complete");
            let completed = pending.complete_scheduler_deferred_inspector_reply(&mut conn);
            match conn
                .complete_devtools_runtime_command_dispatch(completed)
                .await
            {
                DevToolsRuntimeCommandTaskStep::Complete(outcome) => outcome,
                DevToolsRuntimeCommandTaskStep::Pending(_) => {
                    panic!("settled awaitPromise protocol-neutral command should complete")
                }
            }
        }
    };
    let (result, _) = outcome.into_parts();
    let result = expect_script_value_result(
        result.expect("background awaitPromise evaluate should succeed"),
        "expected background owner string",
    );
    assert_eq!(
        result.value,
        json!("background"),
        "deferred reply for internal id 0 should be read from the original background owner"
    );
    assert!(
        !conn.has_pending_inspector_awaits(),
        "completed internal id 0 awaitPromise should clear the pending await registry"
    );
    assert_eq!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .active_target_id(),
        Some(first_target_id.as_str()),
        "protocol-neutral awaitPromise must not activate the background target"
    );
}

#[tokio::test]
async fn pending_runtime_binding_page_phase_keeps_background_owner_route_across_completion() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-binding-owner-route")),
        target_id: None,
        browser_context_id: None,
    };

    let (first_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
    let DevToolsCommandResult::CreateTarget(first_result) =
        first_result.expect("initial target create should succeed")
    else {
        panic!("expected create target result");
    };
    let first_target_id = first_result.target_id;
    ensure_initial_document_for_target_id_for_test(&mut conn, &first_target_id).await;

    let (second_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(second_result) =
        second_result.expect("background target create should succeed")
    else {
        panic!("expected create target result");
    };
    let second_target_id = second_result.target_id;
    ensure_initial_document_for_target_id_for_test(&mut conn, &second_target_id).await;

    let first_context = DevToolsCommandContext {
        target_id: Some(first_target_id.clone()),
        ..context.clone()
    };
    let second_context = DevToolsCommandContext {
        target_id: Some(second_target_id.clone()),
        ..context
    };
    assert_eq!(
        evaluate_string_for_test(
            &mut conn,
            first_context,
            "globalThis.__bindingOwnerProbe = 'active'; globalThis.__bindingOwnerProbe",
            "active binding owner marker",
        )
        .await,
        "active"
    );
    assert_eq!(
        evaluate_string_for_test(
            &mut conn,
            second_context,
            "globalThis.__bindingOwnerProbe = 'background'; globalThis.__bindingOwnerProbe",
            "background binding owner marker",
        )
        .await,
        "background"
    );

    let background_route = conn
        .target_session_route_for_target_id(second_target_id.as_str())
        .expect("background target route");
    let raw = serde_json::to_string(&json!({
        "id": 1269,
        "method": "Runtime.addBinding",
        "params": { "name": "backgroundRouteBinding" }
    }))
    .unwrap();
    let pending = {
        let previous_route =
            conn.replace_none_session_owner_route_override(Some(background_route.clone()));
        let step = conn.start_command_dispatch(&raw);
        conn.replace_none_session_owner_route_override(previous_route);
        match step {
            CdpCommandTaskStep::Pending(pending) => pending,
            CdpCommandTaskStep::Complete(outcome) => {
                panic!(
                    "background Runtime.addBinding should pend through renderer binding state apply: {:?}",
                    outcome.into_parts().0
                )
            }
        }
    };

    let messages = complete_command_task_for_test(&mut conn, *pending).await;
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == json!(1269) && message.get("error").is_none()),
        "Runtime.addBinding should complete successfully on the original background owner: {messages:?}"
    );
    assert!(
        conn.target_devtools_session_state_for_session(None)
            .is_none_or(|state| state
                .runtime_bindings
                .iter()
                .all(|binding| binding.name != "backgroundRouteBinding")),
        "binding state apply completion must not write the active owner"
    );
    let previous_route = conn.replace_none_session_owner_route_override(Some(background_route));
    assert!(
        conn.target_devtools_session_state_for_session(None)
            .is_some_and(|state| state
                .runtime_bindings
                .iter()
                .any(|binding| binding.name == "backgroundRouteBinding")),
        "binding state apply completion should persist on the original background owner"
    );
    conn.replace_none_session_owner_route_override(previous_route);
    assert_eq!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .active_target_id(),
        Some(first_target_id.as_str()),
        "background Runtime.addBinding completion must not activate the background target"
    );
}

#[tokio::test]
async fn runtime_enable_uses_background_initial_document_without_adapter() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-runtime-enable-normal-route")),
        target_id: None,
        browser_context_id: None,
    };

    let (first_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
    let DevToolsCommandResult::CreateTarget(first_result) =
        first_result.expect("initial target create should succeed")
    else {
        panic!("expected create target result");
    };
    let first_target_id = first_result.target_id;

    let (second_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context,
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
    let DevToolsCommandResult::CreateTarget(second_result) =
        second_result.expect("background target create should succeed")
    else {
        panic!("expected create target result");
    };
    let second_target_id = second_result.target_id;

    let background_route = conn
        .target_session_route_for_target_id(second_target_id.as_str())
        .expect("background target route");
    let pending_initial_document = {
        let previous_route =
            conn.replace_none_session_owner_route_override(Some(background_route.clone()));
        let pending = conn
            .start_initial_document_page_ensure_for_session_owner(None)
            .expect("background target lifecycle ensure should start")
            .expect("fresh background target should need an initial document page build");
        conn.replace_none_session_owner_route_override(previous_route);
        pending
    };
    let completed_initial_document = pending_initial_document
        .wait()
        .await
        .expect("background initial document page build should complete");
    conn.complete_initial_document_page_build_for_owner(completed_initial_document)
        .await
        .expect("background initial document should install on captured owner");

    let raw = serde_json::to_string(&json!({
        "id": 1270,
        "method": "Runtime.enable"
    }))
    .unwrap();
    let pending = {
        let previous_route =
            conn.replace_none_session_owner_route_override(Some(background_route.clone()));
        let step = conn.start_command_dispatch(&raw);
        conn.replace_none_session_owner_route_override(previous_route);
        match step {
            CdpCommandTaskStep::Pending(pending) => pending,
            CdpCommandTaskStep::Complete(outcome) => {
                panic!(
                    "background Runtime.enable should replay the existing initial context through V8: {:?}",
                    outcome.into_parts().0
                )
            }
        }
    };

    let messages = complete_command_task_for_test(&mut conn, *pending).await;
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == json!(1270) && message.get("error").is_none()),
        "Runtime.enable should complete successfully on the original background owner: {messages:?}"
    );
    let browser_context = conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        browser_context.active_target_id(),
        Some(first_target_id.as_str()),
        "background Runtime.enable must not activate or overwrite the active target"
    );
    assert!(
        browser_context
            .background_target(second_target_id.as_str())
            .is_some_and(|target| target.has_loaded_page()),
        "background target should keep its target-lifecycle initial page"
    );
    assert!(
        !browser_context.active_target.runtime_slot.has_loaded_page(),
        "Runtime.enable must not install a page on the active target"
    );
    assert!(
        conn.target_runtime_session_state_for_session(None)
            .is_none_or(|state| !state.runtime_frontend_enabled),
        "Runtime.enable completion must not enable Runtime on the active owner"
    );
    let previous_route = conn.replace_none_session_owner_route_override(Some(background_route));
    assert!(
        conn.target_runtime_session_state_for_session(None)
            .is_some_and(|state| state.runtime_frontend_enabled),
        "Runtime.enable completion should enable Runtime on the original background owner"
    );
    conn.replace_none_session_owner_route_override(previous_route);
}

#[tokio::test]
async fn page_enable_uses_background_initial_document_without_adapter() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-page-normal-route")),
        target_id: None,
        browser_context_id: None,
    };

    let (first_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
    let DevToolsCommandResult::CreateTarget(first_result) =
        first_result.expect("initial target create should succeed")
    else {
        panic!("expected create target result");
    };
    let first_target_id = first_result.target_id;

    let (second_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context,
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
    let DevToolsCommandResult::CreateTarget(second_result) =
        second_result.expect("background target create should succeed")
    else {
        panic!("expected create target result");
    };
    let second_target_id = second_result.target_id;

    let background_route = conn
        .target_session_route_for_target_id(second_target_id.as_str())
        .expect("background target route");
    let pending_initial_document = {
        let previous_route =
            conn.replace_none_session_owner_route_override(Some(background_route.clone()));
        let pending = conn
            .start_initial_document_page_ensure_for_session_owner(None)
            .expect("background target lifecycle ensure should start")
            .expect("fresh background target should need an initial document page build");
        conn.replace_none_session_owner_route_override(previous_route);
        pending
    };
    let completed_initial_document = pending_initial_document
        .wait()
        .await
        .expect("background initial document page build should complete");
    conn.complete_initial_document_page_build_for_owner(completed_initial_document)
        .await
        .expect("background initial document should install on captured owner");

    let raw = serde_json::to_string(&json!({
        "id": 1268,
        "method": "Page.enable"
    }))
    .unwrap();
    let pending = {
        let previous_route = conn.replace_none_session_owner_route_override(Some(background_route));
        let step = conn.start_command_dispatch(&raw);
        conn.replace_none_session_owner_route_override(previous_route);
        match step {
            CdpCommandTaskStep::Pending(_) => {
                panic!(
                    "background Page.enable should not start a pending initial document page build"
                )
            }
            CdpCommandTaskStep::Complete(outcome) => outcome,
        }
    };
    let messages = pending.into_parts().0;
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == json!(1268) && message.get("error").is_none()),
        "Page.enable should complete successfully on the original background owner: {messages:?}"
    );
    let browser_context = conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        browser_context.active_target_id(),
        Some(first_target_id.as_str()),
        "background Page.enable must not activate or overwrite the active target"
    );
    assert!(
        browser_context
            .background_target(second_target_id.as_str())
            .is_some_and(|target| target.has_loaded_page()),
        "background target should keep its target-lifecycle initial page"
    );
    assert!(
        !browser_context.active_target.runtime_slot.has_loaded_page(),
        "Page.enable must not install a page on the active target"
    );
}

#[tokio::test]
async fn initial_document_page_ensure_completion_uses_captured_owner() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-initial-owner")),
        target_id: None,
        browser_context_id: None,
    };

    let (first_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
    let DevToolsCommandResult::CreateTarget(first_result) =
        first_result.expect("initial target create should succeed")
    else {
        panic!("expected create target result");
    };
    let first_target_id = first_result.target_id;

    let (second_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context,
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
    let DevToolsCommandResult::CreateTarget(second_result) =
        second_result.expect("background target create should succeed")
    else {
        panic!("expected create target result");
    };
    let second_target_id = second_result.target_id;

    let background_route = conn
        .target_session_route_for_target_id(second_target_id.as_str())
        .expect("background target route");
    let pending = {
        let previous_route = conn.replace_none_session_owner_route_override(Some(background_route));
        let pending = conn
            .start_initial_document_page_ensure_for_session_owner(None)
            .expect("background initial document page ensure should start")
            .expect("background initial document page ensure should pend");
        conn.replace_none_session_owner_route_override(previous_route);
        pending
    };

    let active_route = conn
        .target_session_route_for_target_id(first_target_id.as_str())
        .expect("active target route");
    let previous_route = conn.replace_none_session_owner_route_override(Some(active_route));
    let completed = pending
        .wait()
        .await
        .expect("initial document page build should complete");
    conn.complete_initial_document_page_build_for_owner(completed)
        .await
        .expect("completion should install on captured owner");
    conn.replace_none_session_owner_route_override(previous_route);

    let browser_context = conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        browser_context.active_target_id(),
        Some(first_target_id.as_str()),
        "completion must not activate the background target"
    );
    assert!(
        browser_context
            .background_target(second_target_id.as_str())
            .is_some_and(|target| target.has_loaded_page()),
        "completion should install the materialized page on the captured background target"
    );
    assert!(
        !browser_context.active_target.runtime_slot.has_loaded_page(),
        "completion must not install the materialized page on the ambient active target"
    );
}

#[tokio::test]
async fn target_lifecycle_ensure_installs_initial_about_blank_page_for_active_target() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-lifecycle-active")),
        target_id: None,
        browser_context_id: None,
    };

    let (create_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context,
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: true,
            }),
        );
    let DevToolsCommandResult::CreateTarget(_) =
        create_result.expect("active target create should succeed")
    else {
        panic!("expected create target result");
    };
    assert!(
        !conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .has_loaded_page(),
        "immediate target create path should still only stage owner metadata before lifecycle ensure"
    );

    let pending = conn
        .start_initial_document_page_ensure_for_session_owner(None)
        .expect("target lifecycle ensure should start active initial page")
        .expect("fresh initial target should pend active initial document page build");
    let joined_pending = conn
        .start_initial_document_page_ensure_for_session_owner(None)
        .expect("second ensure should join the active initial page build")
        .expect("second ensure should wait for the active initial page build");
    let completed = pending
        .wait()
        .await
        .expect("active initial document page build should complete");
    conn.complete_initial_document_page_build_for_owner(completed)
        .await
        .expect("active initial page should install");
    let joined_completed = joined_pending
        .wait()
        .await
        .expect("joined initial document page build should observe completion");
    conn.complete_initial_document_page_build_for_owner(joined_completed)
        .await
        .expect("joined initial page completion should be a no-op");
    assert!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .has_loaded_page(),
        "ensure should install loaded page on active target"
    );

    assert!(
        conn.start_initial_document_page_ensure_for_session_owner(None)
            .expect("second target lifecycle ensure should succeed")
            .is_none(),
        "second ensure should be a no-op"
    );
}

#[tokio::test]
async fn stale_initial_document_page_build_does_not_overwrite_committed_page() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-lifecycle-stale-initial")),
        target_id: None,
        browser_context_id: None,
    };

    let (create_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context,
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: true,
            }),
        );
    let DevToolsCommandResult::CreateTarget(_) =
        create_result.expect("active target create should succeed")
    else {
        panic!("expected create target result");
    };

    let pending = conn
        .start_initial_document_page_ensure_for_session_owner(None)
        .expect("target lifecycle ensure should start active initial page")
        .expect("fresh initial target should pend active initial document page build");
    let real_page_url = "data:text/html,<title>real-page</title>";
    let parsed_real_page_url = url::Url::parse(real_page_url).expect("data URL should parse");
    let real_page = conn
        .load_page_via_runtime_async(real_page_url)
        .await
        .expect("real navigation page should build");
    conn.commit_loaded_navigation_page_for_session_owner_async(
        None,
        real_page,
        crate::conn::LoadedNavigationRendererAttachmentCommit::Prepare(None),
        &parsed_real_page_url,
    )
    .await
    .expect("real navigation page owner should exist")
    .expect("real navigation page Inspector binding should activate");
    let real_page_commit = moli_core::page::RendererMainDocumentCommit {
        frame_id: "TID-1".to_owned(),
        loader_id: "LOADER-real-page".to_owned(),
        url: parsed_real_page_url.to_string(),
        unreachable_url: None,
        security_origin: "null".to_owned(),
        secure_context_type: "InsecureScheme".to_owned(),
        timestamp: 0.0,
    };
    conn.commit_loaded_navigation_target_identity_for_session_owner(
        None,
        &real_page_commit,
        &parsed_real_page_url,
    )
    .expect("real navigation identity should commit");
    let attachment_after_real_page = conn
        .browser_context
        .as_ref()
        .expect("browser context")
        .page_attachment_id();

    let completed = pending
        .wait()
        .await
        .expect("stale initial document page build should complete");
    conn.complete_initial_document_page_build_for_owner(completed)
        .await
        .expect("stale initial document page build should be discarded");
    let messages = conn
        .dispatch_runtime_helper_protocol_message_for_session_owner_async(
            None,
            r#"{"id":7001,"method":"Runtime.evaluate","params":{"expression":"document.title","returnByValue":true}}"#,
            7001,
        )
        .await
        .expect("replacement Inspector context must survive stale initial Page teardown");
    let evaluation = messages
        .iter()
        .find_map(|message| {
            let message = message.clone().into_v8_inspector_message();
            (message["id"] == json!(7001)).then_some(message)
        })
        .expect("replacement evaluation should return a protocol response");
    assert_eq!(evaluation["result"]["result"]["value"], json!("real-page"));

    let large_frontend_command_id = i32::MAX as u64 + 73;
    let large_id_messages = conn
        .dispatch_runtime_helper_protocol_message_for_session_owner_async(
            None,
            &json!({
                "id": large_frontend_command_id,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "6 * 7",
                    "returnByValue": true
                }
            })
            .to_string(),
            large_frontend_command_id,
        )
        .await
        .expect("large frontend command id should use an internal renderer call id");
    let large_id_evaluation = large_id_messages
        .iter()
        .find_map(|message| {
            let message = message.clone().into_v8_inspector_message();
            (message["id"] == json!(large_frontend_command_id)).then_some(message)
        })
        .expect("renderer response should restore the large frontend command id");
    assert_eq!(large_id_evaluation["result"]["result"]["value"], json!(42));

    let current_attachment_id = conn
        .browser_context
        .as_ref()
        .and_then(|context| context.loaded_page())
        .and_then(moli_core::page::Page::renderer_agent_attachment_id)
        .expect("loaded page should have a renderer attachment");
    let stale_attachment_id = moli_core::page::RendererAgentAttachmentId::allocate();
    let attachment_test_frontend_id = 8_101;
    let correlation = conn
        .try_register_renderer_call_for_session_owner(
            None,
            attachment_test_frontend_id,
            Some(current_attachment_id),
            RendererCommandDescriptor::from_synthesized_payload(
                json!({
                    "id": attachment_test_frontend_id,
                    "method": "Runtime.evaluate",
                    "params": { "expression": "1" },
                })
                .to_string(),
            )
            .unwrap(),
        )
        .unwrap()
        .correlation();
    let response_ready =
        |renderer_call_id: moli_page_types::RendererCallId,
         attachment_id: moli_core::page::RendererAgentAttachmentId| {
            let mut output = moli_core::page::RendererRuntimeCommandOutput::from_inspector_message(
                moli_core::page::RendererRuntimeInspectorMessage::protocol(json!({
                    "id": renderer_call_id.get(),
                    "result": {}
                })),
            );
            output.bind_renderer_agent_attachment(attachment_id);
            RuntimeInspectorResponseReady::new(
                attachment_test_frontend_id,
                None,
                Ok(
                    moli_core::RendererRuntimeInspectorAsyncCompletion::from_command_output(
                        renderer_call_id.get(),
                        output,
                    ),
                ),
            )
        };

    assert!(
        conn.resolve_runtime_inspector_response_ready(response_ready(
            correlation.renderer_call_id(),
            stale_attachment_id,
        ))
        .is_none(),
        "stale attachment response must not consume the pending correlation"
    );
    assert!(
        conn.resolve_runtime_inspector_response_ready(response_ready(
            moli_page_types::RendererCallId::new(correlation.renderer_call_id().get() + 1,),
            current_attachment_id,
        ))
        .is_none(),
        "wrong renderer call id must not consume the pending correlation"
    );
    let resolved = conn
        .resolve_runtime_inspector_response_ready(response_ready(
            correlation.renderer_call_id(),
            current_attachment_id,
        ))
        .expect("matching session, renderer call, and attachment should resolve");
    assert_eq!(
        resolved.into_protocol_message_for_typed_runtime_route()["id"],
        json!(attachment_test_frontend_id)
    );

    let browser_context = conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        browser_context.page_attachment_id(),
        attachment_after_real_page,
        "discarding stale initial document build must not replace the current page"
    );
    assert_eq!(
        browser_context
            .loaded_page()
            .expect("real page should stay installed")
            .final_url()
            .as_str(),
        parsed_real_page_url.as_str(),
        "current page should remain the committed navigation page"
    );
    let initial = browser_context
        .active_target
        .owner_state
        .initial_empty_document_state()
        .expect("initial empty document state should remain recorded");
    assert!(
        initial.exited(),
        "real navigation should have exited the initial empty document"
    );
    assert!(
        !initial.materialized(),
        "discarded initial build must not mark the exited initial document materialized"
    );
}

#[tokio::test]
async fn bidi_create_target_installs_initial_about_blank_page_without_default_preload() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-lifecycle-create")),
        target_id: None,
        browser_context_id: None,
    };

    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context,
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: true,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(_) =
        create_result.expect("active target create should succeed")
    else {
        panic!("expected create target result");
    };
    assert!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .has_loaded_page(),
        "BiDi Target.createTarget should install initial page even without default preload scripts"
    );
}

#[tokio::test]
async fn target_lifecycle_ensure_installs_initial_about_blank_page_for_background_target() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-lifecycle-background")),
        target_id: None,
        browser_context_id: None,
    };

    let (first_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
    let DevToolsCommandResult::CreateTarget(first_result) =
        first_result.expect("initial target create should succeed")
    else {
        panic!("expected create target result");
    };
    let first_target_id = first_result.target_id;

    let (second_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context,
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
    let DevToolsCommandResult::CreateTarget(second_result) =
        second_result.expect("background target create should succeed")
    else {
        panic!("expected create target result");
    };
    let second_target_id = second_result.target_id;

    let background_route = conn
        .target_session_route_for_target_id(second_target_id.as_str())
        .expect("background target route");
    let previous_route = conn.replace_none_session_owner_route_override(Some(background_route));
    let pending = conn
        .start_initial_document_page_ensure_for_session_owner(None)
        .expect("target lifecycle ensure should start background initial page")
        .expect("fresh background initial target should pend initial document page build");
    conn.replace_none_session_owner_route_override(previous_route);

    let completed = pending
        .wait()
        .await
        .expect("background initial document page build should complete");
    conn.complete_initial_document_page_build_for_owner(completed)
        .await
        .expect("background initial page should install on captured owner");

    let browser_context = conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        browser_context.active_target_id(),
        Some(first_target_id.as_str()),
        "background ensure must not activate the background target"
    );
    assert!(
        browser_context
            .background_target(second_target_id.as_str())
            .is_some_and(|target| target.has_loaded_page()),
        "ensure should install loaded page on background target"
    );
    assert!(
        !browser_context.active_target.runtime_slot.has_loaded_page(),
        "background ensure must not install the materialized page on the active target"
    );
}

#[tokio::test]
async fn devtools_get_realms_observes_create_target_initial_about_blank_page() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: true,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    assert!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .has_active_target()
            && conn
                .browser_context
                .as_ref()
                .expect("browser context")
                .has_loaded_page(),
        "BiDi createTarget should install the initial about:blank page before getRealms"
    );

    let (realms_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context: DevToolsCommandContext {
                target_id: Some(create_result.target_id.clone()),
                ..context
            },
            realm_type: Some("window".to_owned()),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::Realms(realms_result) =
        realms_result.expect("getRealms should observe initial about:blank")
    else {
        panic!("expected realms result");
    };
    assert!(
        realms_result.realms.iter().any(|realm| {
            realm.realm_id.is_some()
                && realm.frame_id.as_ref().map(|id| id.as_str())
                    == Some(create_result.target_id.as_str())
                && realm.context_type.as_deref() == Some("default")
        }),
        "getRealms should expose the default window realm"
    );
    assert!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .has_loaded_page(),
        "initial about:blank page should remain installed"
    );
}

#[tokio::test]
async fn devtools_get_realms_succeeds_when_page_loaded_before_session_attach() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-late-realms".to_owned());
    browser_context.set_active_target_id("TID-late-realms".to_owned());
    conn.browser_context = Some(browser_context);

    let page = conn
        .load_page_via_runtime_async("data:text/html,<title>late-realms</title>")
        .await
        .expect("page should load before DevTools session attaches");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .attach_active_session("SID-late-realms".to_owned());

    let (realms_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("SID-late-realms")),
                target_id: Some(DevToolsTargetId::from("TID-late-realms")),
                browser_context_id: None,
            },
            realm_type: Some("window".to_owned()),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::Realms(realms_result) =
        realms_result.expect("getRealms should not fail when realm uniqueId was never captured")
    else {
        panic!("expected realms result");
    };
    assert!(
        realms_result.realms.iter().any(|realm| {
            realm.context_id.is_some()
                && realm.frame_id.as_ref().map(|id| id.as_str()) == Some("TID-late-realms")
                && realm.context_type.as_deref() == Some("default")
        }),
        "getRealms should expose the loaded target context even without a captured realm id"
    );
}

#[tokio::test]
async fn devtools_runtime_evaluate_uses_fresh_initial_document_without_resolver_fallback() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: true,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };

    let (evaluate_result, _) = conn
        .execute_devtools_command(DevToolsCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: DevToolsCommandContext {
                    target_id: Some(create_result.target_id.clone()),
                    ..context
                },
                realm_id: None,
                world_name: None,
                expression: "location.href + '|' + document.body.childNodes.length".to_owned(),
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
    let evaluate_result = expect_script_value_result(
        evaluate_result.expect("evaluate should observe the target-lifecycle initial document"),
        "expected script value result",
    );
    assert_eq!(evaluate_result.value, json!("about:blank|0"));
}

#[tokio::test]
async fn classic_create_target_ensures_fresh_initial_document_without_resolver_fallback() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("classic create target should succeed")
    else {
        panic!("expected create target result");
    };
    assert!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .has_loaded_page(),
        "classic create target should install its initial document during target lifecycle"
    );

    let (evaluate_result, _) = conn
        .execute_devtools_command(DevToolsCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: DevToolsCommandContext {
                    target_id: Some(create_result.target_id.clone()),
                    ..context
                },
                realm_id: None,
                world_name: None,
                expression: "location.href".to_owned(),
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
    let evaluate_result = expect_script_value_result(
        evaluate_result.expect("classic evaluate should observe the target-lifecycle document"),
        "expected script value result",
    );
    assert_eq!(evaluate_result.value, json!("about:blank"));
}

#[tokio::test]
async fn devtools_runtime_evaluate_reports_no_document_without_resolver_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-no-page-runtime".to_owned());
    browser_context.set_active_target_id("TID-no-page-runtime".to_owned());
    browser_context.attach_active_session("SID-no-page-runtime".to_owned());
    browser_context.set_target_url("about:blank".to_owned());
    conn.browser_context = Some(browser_context);

    let (evaluate_result, _) = conn
        .execute_devtools_command(DevToolsCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("SID-no-page-runtime")),
                    target_id: Some(DevToolsTargetId::from("TID-no-page-runtime")),
                    browser_context_id: None,
                },
                realm_id: None,
                world_name: None,
                expression: "1".to_owned(),
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
    let error = evaluate_result.expect_err("manual no-page target should not be repaired");
    assert_eq!(error.kind, DevToolsErrorKind::Internal);
    assert_eq!(error.message, "NoDocumentLoaded");
    assert!(
        !conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .has_loaded_page(),
        "runtime target resolver should not install an initial document"
    );
}

#[tokio::test]
async fn element_screenshot_reports_unsupported_without_placeholder_payload() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("SID-element-shot")),
        target_id: None,
        browser_context_id: None,
    };

    let create_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: true,
        }),
    )
    .await;
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = DevToolsCommandContext {
        target_id: Some(create_result.target_id.clone()),
        ..context
    };

    let navigate_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: target_context.clone(),
            url: "data:text/html,<div id='target' style='width:120px;height:80px'>shot</div>"
                .to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await;
    navigate_result.expect("navigate should succeed");

    let evaluate_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
            context: target_context.clone(),
            realm_id: None,
            world_name: None,
            expression: "document.getElementById('target')".to_owned(),
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
    let DevToolsCommandResult::Script(evaluate_result) =
        evaluate_result.expect("element evaluate should succeed")
    else {
        panic!("expected script result");
    };
    let DevToolsScriptResult::Value(remote_value) = *evaluate_result else {
        panic!("expected element remote value");
    };
    let shared_id = remote_value
        .shared_id
        .expect("element remote value should expose sharedId");

    let screenshot_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::CaptureScreenshot(DevToolsCaptureScreenshotCommand {
            context: target_context,
            format: Some("png".to_owned()),
            quality: None,
            clip: Some(DevToolsCaptureScreenshotClip::Element(
                DevToolsScreenshotElementClip { shared_id },
            )),
            capture_beyond_viewport: false,
            optimize_for_speed: false,
        }),
    )
    .await;
    let error =
        screenshot_result.expect_err("element screenshot should not return placeholder data");
    assert_eq!(error.kind, DevToolsErrorKind::Unsupported);
    assert_eq!(
        error.message,
        "Page.captureScreenshot is not supported: renderer screenshots are not implemented."
    );
}

#[tokio::test]
async fn element_screenshot_reports_unsupported_without_initial_document_repair() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-no-page-element-shot".to_owned());
    browser_context.set_active_target_id("TID-no-page-element-shot".to_owned());
    browser_context.attach_active_session("SID-no-page-element-shot".to_owned());
    browser_context.set_target_url("about:blank".to_owned());
    conn.browser_context = Some(browser_context);

    let (screenshot_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CaptureScreenshot(
            DevToolsCaptureScreenshotCommand {
                context: DevToolsCommandContext {
                    protocol: DevToolsProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("SID-no-page-element-shot")),
                    target_id: Some(DevToolsTargetId::from("TID-no-page-element-shot")),
                    browser_context_id: None,
                },
                format: Some("png".to_owned()),
                quality: None,
                clip: Some(DevToolsCaptureScreenshotClip::Element(
                    DevToolsScreenshotElementClip {
                        shared_id: DevToolsRemoteHandleId::from("missing-node-shared-id"),
                    },
                )),
                capture_beyond_viewport: false,
                optimize_for_speed: false,
            },
        ))
        .await
        .into_parts();
    let error = screenshot_result.expect_err("manual no-page target should not be repaired");
    assert_eq!(error.kind, DevToolsErrorKind::Unsupported);
    assert_eq!(
        error.message,
        "Page.captureScreenshot is not supported: renderer screenshots are not implemented."
    );
    assert!(
        !conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .has_loaded_page(),
        "element screenshot should not install an initial document"
    );
}

#[tokio::test]
async fn devtools_create_target_can_activate_created_target() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (first_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(first_result) =
        first_result.expect("first create target should succeed")
    else {
        panic!("expected first create target result");
    };
    let first_target_id = first_result.target_id.clone();

    let (first_navigate, _, _, _) = conn
        .execute_devtools_command(DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: DevToolsCommandContext {
                target_id: Some(first_target_id.clone()),
                ..context.clone()
            },
            url: "data:text/html,<title>first</title>first".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }))
        .await
        .into_complete_parts();
    first_navigate.expect("first navigate should succeed");

    let (second_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: true,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(second_result) =
        second_result.expect("second create target should succeed")
    else {
        panic!("expected second create target result");
    };
    let second_target_id = second_result.target_id.clone();

    let browser_context = conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        browser_context.active_target_id(),
        Some("TID-2"),
        "activate=true should promote the created target instead of staging it"
    );
    assert!(
        browser_context
            .background_targets
            .iter()
            .any(|target| target.target_id() == "TID-1"),
        "the previous active target should be preserved as a background target"
    );
    assert!(
        browser_context
            .background_targets
            .iter()
            .find(|target| target.target_id() == "TID-1")
            .is_some_and(|target| target.has_loaded_page()),
        "the previous active page should move into the demoted target slot"
    );

    let (first_realms_before_second_navigation, _) = conn
        .execute_devtools_command(DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context: DevToolsCommandContext {
                target_id: Some(first_target_id.clone()),
                ..context.clone()
            },
            realm_type: Some("window".to_owned()),
        }))
        .await
        .into_parts();
    first_realms_before_second_navigation
        .expect("demoted first target realms should remain readable before second navigation");
    assert!(
        conn.none_session_owner_route_override().is_none(),
        "realm lookup must restore the ambient target route"
    );

    let (second_navigate, _, _, _) = conn
        .execute_devtools_command(DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: DevToolsCommandContext {
                target_id: Some(second_target_id.clone()),
                ..context.clone()
            },
            url: "data:text/html,<title>second</title>second".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }))
        .await
        .into_complete_parts();
    second_navigate.expect("second navigate should succeed");

    let (first_realms, _) = conn
        .execute_devtools_command(DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context: DevToolsCommandContext {
                target_id: Some(first_target_id.clone()),
                ..context.clone()
            },
            realm_type: Some("window".to_owned()),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::Realms(first_realms) =
        first_realms.expect("demoted first target realms should remain readable")
    else {
        panic!("expected realms result");
    };
    assert!(
        first_realms.realms.iter().any(|realm| {
            realm.frame_id.as_ref().map(|frame_id| frame_id.as_str()) == Some("TID-1")
        }),
        "demoted target should keep its window realm"
    );

    let (all_realms, _) = conn
        .execute_devtools_command(DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context,
            realm_type: Some("window".to_owned()),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::Realms(all_realms) =
        all_realms.expect("all-context getRealms should enumerate active and background targets")
    else {
        panic!("expected realms result");
    };
    let first_realm_id = all_realms
        .realms
        .iter()
        .find(|realm| {
            realm.frame_id.as_ref().map(|frame_id| frame_id.as_str())
                == Some(first_target_id.as_str())
                && realm.context_type.as_deref() == Some("default")
        })
        .and_then(|realm| realm.realm_id.as_ref())
        .expect("all-context getRealms should include the demoted target default realm");
    let second_realm_id = all_realms
        .realms
        .iter()
        .find(|realm| {
            realm.frame_id.as_ref().map(|frame_id| frame_id.as_str())
                == Some(second_target_id.as_str())
                && realm.context_type.as_deref() == Some("default")
        })
        .and_then(|realm| realm.realm_id.as_ref())
        .expect("all-context getRealms should include the active target default realm");
    assert_ne!(
        first_realm_id, second_realm_id,
        "realm ids must stay globally unique across active and background page runtimes"
    );
}

#[tokio::test]
async fn devtools_command_executes_page_navigation_and_reload() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let url = "data:text/html,bidi-nav".to_owned();
    let target_id = create_result.target_id.clone();

    let (navigate_result, _, _, _) = conn
        .execute_devtools_command(DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: DevToolsCommandContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            url: url.clone(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }))
        .await
        .into_complete_parts();
    let DevToolsCommandResult::Navigate(navigate_result) =
        navigate_result.expect("navigate should succeed")
    else {
        panic!("expected navigate result");
    };
    assert_eq!(navigate_result.url, url);

    let (reload_result, _, _, _) = conn
        .execute_devtools_command(DevToolsCommand::Reload(DevToolsReloadCommand {
            context: DevToolsCommandContext {
                target_id: Some(target_id),
                ..context
            },
            ignore_cache: false,
            script_to_evaluate_on_load: None,
            wait: DevToolsNavigationWait::Load,
        }))
        .await
        .into_complete_parts();
    let DevToolsCommandResult::Navigate(reload_result) =
        reload_result.expect("reload should succeed")
    else {
        panic!("expected reload navigation result");
    };
    assert_eq!(reload_result.url, url);
}

#[tokio::test]
async fn devtools_command_executes_page_navigation_without_cdp_response_sidecar() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = DevToolsCommandContext {
        target_id: Some(create_result.target_id),
        ..context
    };
    let url = "data:text/html,direct-nav-no-sidecar".to_owned();

    let (navigate_result, _scheduler_events, protocol_events, _renderer_output_predecessor) = conn
        .execute_devtools_command_with_protocol_events(DevToolsCommand::Navigate(
            DevToolsNavigateCommand {
                context: target_context,
                url: url.clone(),
                referrer: None,
                wait: DevToolsNavigationWait::Load,
            },
        ))
        .await
        .into_complete_parts();
    let DevToolsCommandResult::Navigate(navigate_result) =
        navigate_result.expect("navigate should succeed")
    else {
        panic!("expected navigate result");
    };
    assert_eq!(navigate_result.url, url);
    assert!(
        protocol_events
            .iter()
            .all(|event| !is_command_response_sidecar_event(event)),
        "direct navigation must not emit a command response as a protocol sidecar: {protocol_events:?}"
    );
}

#[tokio::test]
async fn devtools_command_executes_child_frame_navigation_without_cdp_response_sidecar() {
    let mut ctx = crate::testing::TestContext::new_with_target_discovery(false);
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let create_outcome = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await;
    let (create_result, _, _, create_predecessor) = create_outcome.into_complete_parts();
    if let Some(predecessor) = create_predecessor {
        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
            .await;
    }
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = DevToolsCommandContext {
        target_id: Some(create_result.target_id.clone()),
        ..context.clone()
    };

    let parent_url =
        "data:text/html,<iframe srcdoc='<p id=\"child\">initial</p>'></iframe>".to_owned();
    let parent_outcome = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: target_context.clone(),
            url: parent_url,
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }))
        .await;
    let (parent_result, _, _, parent_predecessor) = parent_outcome.into_complete_parts();
    if let Some(predecessor) = parent_predecessor {
        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
            .await;
    }
    parent_result.expect("parent navigation should succeed");

    let (frame_tree_result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::GetFrameTree(DevToolsGetFrameTreeCommand {
            context: target_context,
            max_depth: None,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::GetFrameTree(frame_tree_result) =
        frame_tree_result.expect("frame tree should be readable")
    else {
        panic!("expected frame tree result");
    };
    let child_frame_id = frame_tree_result.frame_tree["childFrames"]
        .as_array()
        .and_then(|child_frames| child_frames.first())
        .and_then(|child_frame| child_frame["frame"]["id"].as_str())
        .expect("parent navigation should create one child frame")
        .to_owned();
    ctx.wait_until_scheduler_state("child frame attachment", |conn| {
        conn.has_attached_child_frame_id(&child_frame_id)
    })
    .await;

    let child_url = "data:text/html,<p id='child'>updated</p>".to_owned();
    let child_outcome = ctx
        .conn
        .execute_devtools_command_with_protocol_events(DevToolsCommand::Navigate(
            DevToolsNavigateCommand {
                context: DevToolsCommandContext {
                    target_id: Some(DevToolsTargetId::from(child_frame_id)),
                    ..context
                },
                url: child_url.clone(),
                referrer: None,
                wait: DevToolsNavigationWait::Load,
            },
        ))
        .await;
    let (navigate_result, _scheduler_events, protocol_events, child_predecessor) =
        child_outcome.into_complete_parts();
    if let Some(predecessor) = child_predecessor {
        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
            .await;
    }
    let DevToolsCommandResult::Navigate(navigate_result) =
        navigate_result.expect("child frame navigate should succeed")
    else {
        panic!("expected navigate result");
    };
    assert_eq!(navigate_result.url, child_url);
    assert!(
        protocol_events
            .iter()
            .all(|event| !is_command_response_sidecar_event(event)),
        "direct child-frame navigation must not emit a command response as a protocol sidecar: {protocol_events:?}"
    );
}

#[tokio::test]
async fn devtools_command_reports_invalid_navigation_without_cdp_response_parser() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = DevToolsCommandContext {
        target_id: Some(create_result.target_id),
        ..context
    };

    let (navigate_result, _scheduler_events, protocol_events) = conn
        .execute_devtools_command_with_protocol_events(DevToolsCommand::Navigate(
            DevToolsNavigateCommand {
                context: target_context,
                url: "not a valid navigation url".to_owned(),
                referrer: None,
                wait: DevToolsNavigationWait::Load,
            },
        ))
        .await
        .into_parts_with_protocol_events();
    let error = navigate_result.expect_err("invalid navigate should fail");
    assert_eq!(error.kind, DevToolsErrorKind::Internal);
    assert_eq!(error.message, "Invalid navigation URL");
    assert!(
        protocol_events
            .iter()
            .all(|event| !is_command_response_sidecar_event(event)),
        "direct navigation error must not be surfaced as a CDP response sidecar: {protocol_events:?}"
    );
}

#[tokio::test]
async fn devtools_command_executes_preload_without_cdp_response_sidecar() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = DevToolsCommandContext {
        target_id: Some(create_result.target_id.clone()),
        ..context.clone()
    };

    let (add_result, _scheduler_events, add_protocol_events) = conn
        .execute_devtools_command_with_protocol_events(DevToolsCommand::AddPreloadScript(
            DevToolsAddPreloadScriptCommand {
                context: target_context,
                source: DevToolsPreloadScriptSource::FunctionDeclaration {
                    function_declaration: "() => { globalThis.__directPreload = true; }".to_owned(),
                    arguments: Vec::new(),
                },
                world_name: None,
                target_ids: Some(vec![create_result.target_id.clone()]),
                browser_context_ids: Vec::new(),
                run_immediately: false,
                include_command_line_api: false,
            },
        ))
        .await
        .into_parts_with_protocol_events();
    let DevToolsCommandResult::AddPreloadScript(add_result) =
        add_result.expect("addPreloadScript should succeed")
    else {
        panic!("expected add preload script result");
    };
    assert!(
        add_result
            .script_id
            .as_str()
            .starts_with(create_result.target_id.as_str()),
        "BiDi target-scoped preload ids should remain target-qualified"
    );
    assert!(
        add_protocol_events
            .iter()
            .all(|event| !is_command_response_sidecar_event(event)),
        "direct preload add must not emit a CDP response sidecar: {add_protocol_events:?}"
    );

    let (remove_result, _scheduler_events, remove_protocol_events) = conn
        .execute_devtools_command_with_protocol_events(DevToolsCommand::RemovePreloadScript(
            DevToolsRemovePreloadScriptCommand {
                context,
                script_id: add_result.script_id,
            },
        ))
        .await
        .into_parts_with_protocol_events();
    assert_eq!(
        remove_result.expect("removePreloadScript should succeed"),
        DevToolsCommandResult::Empty
    );
    assert!(
        remove_protocol_events
            .iter()
            .all(|event| !is_command_response_sidecar_event(event)),
        "direct preload remove must not emit a CDP response sidecar: {remove_protocol_events:?}"
    );
}

#[tokio::test]
async fn devtools_command_reports_invalid_preload_without_cdp_response_parser() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (add_result, _scheduler_events, protocol_events) = conn
        .execute_devtools_command_with_protocol_events(DevToolsCommand::AddPreloadScript(
            DevToolsAddPreloadScriptCommand {
                context,
                source: DevToolsPreloadScriptSource::FunctionDeclaration {
                    function_declaration: "(value) => value".to_owned(),
                    arguments: vec![json!({"handle": "remote-object-1"})],
                },
                world_name: None,
                target_ids: None,
                browser_context_ids: Vec::new(),
                run_immediately: false,
                include_command_line_api: false,
            },
        ))
        .await
        .into_parts_with_protocol_events();
    let error = add_result.expect_err("invalid preload argument should fail");
    assert_eq!(error.kind, DevToolsErrorKind::Internal);
    assert_eq!(error.message, "UnsupportedPreloadScriptArguments");
    assert!(
        protocol_events
            .iter()
            .all(|event| !is_command_response_sidecar_event(event)),
        "direct preload error must not be surfaced as a CDP response sidecar: {protocol_events:?}"
    );
}

#[tokio::test]
async fn devtools_command_navigates_explicit_about_blank_without_fetch() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let create_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }),
    )
    .await;
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id.clone();
    let target_context = DevToolsCommandContext {
        target_id: Some(target_id),
        ..context
    };

    let data_navigate = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: target_context.clone(),
            url: "data:text/html,<script>window.marker='old'</script><p>old</p>".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await;
    data_navigate.expect("data navigate should succeed");

    assert_eq!(
        evaluate_string_through_renderer_fence_for_test(
            &mut ctx,
            target_context.clone(),
            "window.marker",
            "data page marker"
        )
        .await,
        "old"
    );

    let blank_navigate = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: target_context.clone(),
            url: "about:blank".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await;
    let DevToolsCommandResult::Navigate(blank_navigate) =
        blank_navigate.expect("about:blank navigate should succeed")
    else {
        panic!("expected navigate result");
    };
    assert_eq!(blank_navigate.url, "about:blank");
    assert!(
        blank_navigate.navigation_id.is_some(),
        "explicit about:blank navigate should keep a navigation id"
    );

    assert_eq!(
        evaluate_string_through_renderer_fence_for_test(
            &mut ctx,
            target_context,
            "location.href + '|' + document.body.childNodes.length + '|' + document.title + '|' + (window.marker === undefined)",
            "about:blank page state"
        )
        .await,
        "about:blank|0||true"
    );
}

#[tokio::test]
async fn devtools_call_function_node_shared_id_failure_precedes_handle() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let create_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }),
    )
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

    let navigate_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: target_context.clone(),
            url: "data:text/html,<img id='target'>".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await;
    navigate_result.expect("navigate should succeed");

    let evaluate_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
            context: target_context.clone(),
            realm_id: None,
            world_name: None,
            expression: "document.querySelector('img')".to_owned(),
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
    let DevToolsCommandResult::Script(evaluate_result) =
        evaluate_result.expect("node evaluate should succeed")
    else {
        panic!("expected script result");
    };
    let DevToolsScriptResult::Value(remote_value) = *evaluate_result else {
        panic!("expected remote node value");
    };
    let handle = remote_value.handle.expect("root node should retain handle");
    assert!(
        remote_value.shared_id.is_some(),
        "node remote value should expose sharedId"
    );

    let call_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
            context: target_context,
            realm_id: None,
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration: "(node) => node.nodeType".to_owned(),
            arguments: vec![json!({
                "type": "node",
                "sharedId": "missing-node-shared-id",
                "handle": handle.as_str()
            })],
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::None,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        }),
    )
    .await;
    let error = call_result.expect_err("invalid sharedId should fail before valid handle");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchNode);
}

#[tokio::test]
async fn bidi_node_remote_value_registers_renderer_shared_node_binding() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let (_target_context, shared_id, backend_node_id) =
        materialize_bidi_target_input_node_for_test(
            &mut ctx,
            "<input id='target' data-state='ready'>",
        )
        .await;
    assert!(
        moli_core::page::is_renderer_backend_node_id(backend_node_id),
        "BiDi node remote value should carry renderer-owned backend id"
    );

    ctx.conn
        .clear_runtime_remote_object_tracking_for_session_owner(None);

    let renderer_binding = ctx
        .conn
        .document_bidi_node_binding_for_session_owner_async(None, shared_id.as_str())
        .await
        .expect("renderer BiDi shared-node binding lookup should run");
    assert_eq!(
        renderer_binding,
        moli_core::page::RendererDomBidiNodeBindingResolution::BackendNodeId(backend_node_id),
        "renderer DOM agent should preserve the exact backend id for the BiDi shared id"
    );
}

#[tokio::test]
async fn bidi_node_remote_value_reuses_renderer_frontend_node_id() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = bidi_fetch_command_context();
    let create_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }),
    )
    .await;
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id.as_str().to_owned();
    let target_context = DevToolsCommandContext {
        target_id: Some(create_result.target_id),
        ..context
    };

    let navigate_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: target_context.clone(),
            url: "data:text/html,<main id='target'>target</main>".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await;
    let DevToolsCommandResult::Navigate(navigation) =
        navigate_result.expect("navigate should succeed")
    else {
        panic!("expected navigate result");
    };
    let navigation_id = navigation
        .navigation_id
        .as_ref()
        .expect("WebDriver BiDi navigation id");
    let loader_id = navigation_id
        .as_str()
        .strip_prefix("navigation-")
        .expect("WebDriver BiDi navigation id should encode the loader id");
    crate::testing::wait_until_renderer_document_load(&mut ctx, None, &target_id, loader_id).await;

    let query_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::QuerySelector(DevToolsQuerySelectorCommand {
            context: target_context.clone(),
            root: None,
            selector: "#target".to_owned(),
            multiple: false,
        }),
    )
    .await;
    let DevToolsCommandResult::QuerySelector(query_result) =
        query_result.expect("query selector should succeed")
    else {
        panic!("expected query selector result");
    };
    let frontend_node_id = query_result.node_ids[0];

    let evaluate_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
            context: target_context,
            realm_id: None,
            world_name: None,
            expression: "document.querySelector('#target')".to_owned(),
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
    let remote_value = expect_script_value_result(
        evaluate_result.expect("node evaluate should succeed"),
        "expected node script result",
    );
    assert_eq!(
        remote_value.node_id,
        Some(frontend_node_id),
        "BiDi node remote metadata should reuse the renderer DOM frontend binding"
    );
    let shared_id = remote_value
        .shared_id
        .as_ref()
        .expect("node remote value should expose sharedId");
    assert!(
        !shared_id.as_str().contains("moli:bidi-node:"),
        "node sharedId must not encode a legacy storage node index: {shared_id}"
    );
}

#[tokio::test]
async fn bidi_node_remote_value_registers_child_shared_node_bindings() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let (target_context, _shared_id, _backend_node_id) = materialize_bidi_target_node_for_test(
        &mut ctx,
        "<section id='target'><a id='inside'>Inside</a></section>",
        "#target",
    )
    .await;

    let evaluate_result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: target_context.clone(),
                realm_id: None,
                world_name: None,
                expression: "document.querySelector('#target')".to_owned(),
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::Root,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: Some(DevToolsSerializationOptions {
                    max_object_depth: None,
                    max_dom_depth: Some(1),
                    include_shadow_tree: None,
                }),
            },
        ))
        .await;
    let remote_value = expect_script_value_result(
        evaluate_result.expect("node evaluate should succeed"),
        "expected node script result",
    );
    let node_value = remote_value
        .node_value
        .expect("node remote value should expose serialized node value");
    let child_shared_id = node_value["children"][0]["sharedId"]
        .as_str()
        .unwrap_or_else(|| panic!("serialized child should expose sharedId: {node_value}"))
        .to_owned();

    ctx.conn
        .clear_runtime_remote_object_tracking_for_session_owner(None);

    let renderer_binding = ctx
        .conn
        .document_bidi_node_binding_for_session_owner_async(None, &child_shared_id)
        .await
        .expect("child shared-node binding lookup should run");
    assert!(
        matches!(
            renderer_binding,
            moli_core::page::RendererDomBidiNodeBindingResolution::BackendNodeId(_)
        ),
        "renderer DOM agent should register child sharedId bindings: {renderer_binding:?}"
    );

    let call_result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::CallFunction(
            DevToolsCallFunctionCommand {
                context: target_context,
                realm_id: None,
                world_name: None,
                object_id: None,
                this_parameter: None,
                function_declaration: "(node) => `${node.localName}:${node.id}`".to_owned(),
                arguments: vec![json!({
                    "type": "node",
                    "sharedId": child_shared_id
                })],
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                object_group: None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            },
        ))
        .await;
    let remote_value = expect_script_value_result(
        call_result.expect("callFunction should resolve child sharedId via renderer binding"),
        "expected callFunction value result",
    );
    assert_eq!(remote_value.value, json!("a:inside"));
}

#[tokio::test]
async fn set_file_input_files_shared_id_uses_renderer_binding_without_protocol_registry() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let (target_context, _shared_id, backend_node_id) =
        materialize_bidi_target_input_node_for_test(&mut ctx, "<input id='target' type='file'>")
            .await;
    let fake_shared_id =
        crate::devtools_runtime::webdriver_bidi_node_shared_id_for_backend_node_id(backend_node_id);
    ctx.conn
        .register_document_bidi_node_binding_for_session_owner_async(
            None,
            fake_shared_id.as_str(),
            backend_node_id,
        )
        .await
        .expect("renderer fake shared-node binding registration should run");

    let upload_bytes = b"from renderer registry";
    let set_result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(
            DevToolsCommand::SetFileInputFiles(DevToolsSetFileInputFilesCommand {
                context: target_context.clone(),
                object_id: fake_shared_id,
                files: vec![moli_core::page::SelectedFile {
                    bytes: upload_bytes.to_vec(),
                    mime_type: "text/plain".to_owned(),
                    name: "renderer-binding.txt".to_owned(),
                    last_modified: 0.0,
                }],
                append: false,
            }),
        )
        .await;
    set_result.expect("setFileInputFiles should use renderer shared-node binding");

    let evaluate_result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: target_context,
                realm_id: None,
                world_name: None,
                expression: "(() => { const files = document.querySelector('#target').files; return `${files.length}:${files[0].name}:${files[0].size}`; })()".to_owned(),
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            },
        ))
        .await;
    let remote_value = expect_script_value_result(
        evaluate_result.expect("file input state evaluate should succeed"),
        "expected file input state script result",
    );
    assert_eq!(
        remote_value.value,
        json!(format!("1:renderer-binding.txt:{}", upload_bytes.len()))
    );
}

#[tokio::test]
async fn locate_nodes_start_node_shared_id_uses_renderer_binding_without_protocol_registry() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let (target_context, _shared_id, backend_node_id) = materialize_bidi_target_node_for_test(
        &mut ctx,
        "<section id='target'><a id='inside'>Inside</a></section><a id='outside'>Outside</a>",
        "#target",
    )
    .await;
    let fake_shared_id =
        crate::devtools_runtime::webdriver_bidi_node_shared_id_for_backend_node_id(backend_node_id);
    ctx.conn
        .register_document_bidi_node_binding_for_session_owner_async(
            None,
            fake_shared_id.as_str(),
            backend_node_id,
        )
        .await
        .expect("renderer fake shared-node binding registration should run");

    let locate_result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::LocateNodes(
            DevToolsLocateNodesCommand {
                context: target_context,
                locator: DevToolsLocateNodesLocator::Css("a".to_owned()),
                max_node_count: None,
                start_nodes: vec![json!({
                    "type": "node",
                    "sharedId": fake_shared_id.as_str()
                })],
                start_node_references: Vec::new(),
                serialization_options: None,
            },
        ))
        .await;
    let DevToolsCommandResult::LocateNodes(locate_result) =
        locate_result.expect("locateNodes should use renderer shared-node binding")
    else {
        panic!("expected locateNodes result");
    };
    assert_eq!(
        locate_result.node_ids.len(),
        1,
        "locateNodes should search under the renderer-bound start node only"
    );
    assert_eq!(locate_result.nodes.len(), 1);
}

#[tokio::test]
async fn call_function_shared_id_uses_renderer_binding_without_protocol_registry() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let (target_context, _shared_id, backend_node_id) = materialize_bidi_target_node_for_test(
        &mut ctx,
        "<section id='target'><a id='inside'>Inside</a></section>",
        "#target",
    )
    .await;
    let fake_shared_id =
        crate::devtools_runtime::webdriver_bidi_node_shared_id_for_backend_node_id(backend_node_id);
    ctx.conn
        .register_document_bidi_node_binding_for_session_owner_async(
            None,
            fake_shared_id.as_str(),
            backend_node_id,
        )
        .await
        .expect("renderer fake shared-node binding registration should run");

    let call_result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::CallFunction(
            DevToolsCallFunctionCommand {
                context: target_context,
                realm_id: None,
                world_name: None,
                object_id: None,
                this_parameter: None,
                function_declaration: "(node) => `${node.id}:${node.querySelector('a').id}`"
                    .to_owned(),
                arguments: vec![json!({
                    "type": "node",
                    "sharedId": fake_shared_id.as_str()
                })],
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                object_group: None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            },
        ))
        .await;
    let remote_value = expect_script_value_result(
        call_result.expect("callFunction should use renderer shared-node binding"),
        "expected callFunction value result",
    );
    assert_eq!(remote_value.value, json!("target:inside"));
}

#[tokio::test]
async fn devtools_command_rejects_page_print_to_pdf_without_placeholder_payload() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id.clone();
    let url = "data:text/html,bidi-print".to_owned();
    let (navigate_result, _, _, _) = conn
        .execute_devtools_command(DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: DevToolsCommandContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            url,
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }))
        .await
        .into_complete_parts();
    navigate_result.expect("navigate should succeed");

    let (print_result, _) = conn
        .execute_devtools_command(DevToolsCommand::PrintToPdf(DevToolsPrintToPdfCommand {
            context: DevToolsCommandContext {
                target_id: Some(target_id),
                ..context
            },
            landscape: Some(false),
            print_background: Some(true),
            scale: Some(1.0),
            paper_width: Some(8.5),
            paper_height: Some(11.0),
            margin_top: Some(0.25),
            margin_bottom: Some(0.25),
            margin_left: Some(0.25),
            margin_right: Some(0.25),
            page_ranges: Some("1".to_owned()),
            shrink_to_fit: Some(true),
            transfer_mode: Some(DevToolsPrintToPdfTransferMode::ReturnAsBase64),
        }))
        .await
        .into_parts();
    let error = print_result.expect_err("print should not return a placeholder PDF");
    assert_eq!(error.kind, DevToolsErrorKind::Unsupported);
    assert_eq!(
        error.message,
        "Page.printToPDF is not supported: PDF generation is not implemented."
    );
}

#[tokio::test]
async fn devtools_command_executes_context_viewport_override() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id.clone();

    let (viewport_result, _) = conn
        .execute_devtools_command(DevToolsCommand::SetViewport(DevToolsSetViewportCommand {
            context: DevToolsCommandContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            browser_context_ids: Vec::new(),
            viewport: DevToolsViewportSetting::Dimensions {
                width: 800,
                height: 600,
            },
            device_pixel_ratio: DevToolsDevicePixelRatioSetting::Scale(2.0),
            screen_width: None,
            screen_height: None,
        }))
        .await
        .into_parts();
    assert_eq!(
        viewport_result.expect("set viewport should succeed"),
        DevToolsCommandResult::Empty
    );

    let metrics = conn
        .target_session_owner_emulated_device_metrics(None)
        .expect("active target should hold emulated device metrics");
    assert_eq!(metrics.width, 800);
    assert_eq!(metrics.height, 600);
    assert_eq!(metrics.device_scale_factor, 2.0);

    let (layout_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetLayoutMetrics(
            DevToolsGetLayoutMetricsCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id),
                    ..context
                },
            },
        ))
        .await
        .into_parts();
    let DevToolsCommandResult::LayoutMetrics(layout_metrics) =
        layout_result.expect("target layout metrics should succeed")
    else {
        panic!("expected layout metrics result");
    };
    assert_eq!(layout_metrics.layout_viewport_width, 800);
    assert_eq!(layout_metrics.layout_viewport_height, 600);
    assert_eq!(layout_metrics.device_pixel_ratio, 2.0);
}

#[tokio::test]
async fn devtools_command_applies_window_state_to_document_surface() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id.clone();
    let target_context = DevToolsCommandContext {
        target_id: Some(target_id),
        ..context
    };

    let (minimize_result, _) = conn
        .execute_devtools_command(DevToolsCommand::SetWindowState(
            DevToolsSetWindowStateCommand {
                context: target_context.clone(),
                state: DevToolsWindowState::Minimized,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        minimize_result.expect("minimize surface state should succeed"),
        DevToolsCommandResult::Empty
    );
    assert!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .active_target
            .owner_state
            .window_document_hidden(),
        "SetWindowState must update the target owner state before applying document surfaces"
    );
    assert_eq!(
        evaluate_document_surface_payload(&mut conn, target_context.clone()).await,
        json!({
            "hasFocus": false,
            "hidden": true,
            "visibilityState": "hidden",
            "fullScreen": false,
            "fullScreenType": "boolean",
            "webkitIsFullScreen": false,
            "webkitIsFullScreenType": "boolean"
        })
    );

    let (fullscreen_result, _) = conn
        .execute_devtools_command(DevToolsCommand::SetWindowState(
            DevToolsSetWindowStateCommand {
                context: target_context.clone(),
                state: DevToolsWindowState::Fullscreen,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        fullscreen_result.expect("fullscreen surface state should succeed"),
        DevToolsCommandResult::Empty
    );
    assert!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .active_target
            .owner_state
            .window_fullscreen(),
        "SetWindowState fullscreen must update the target owner before applying document surfaces"
    );
    assert_eq!(
        evaluate_document_surface_payload(&mut conn, target_context.clone()).await,
        json!({
            "hasFocus": true,
            "hidden": false,
            "visibilityState": "visible",
            "fullScreen": true,
            "fullScreenType": "boolean",
            "webkitIsFullScreen": true,
            "webkitIsFullScreenType": "boolean"
        })
    );

    let (normal_result, _) = conn
        .execute_devtools_command(DevToolsCommand::SetWindowState(
            DevToolsSetWindowStateCommand {
                context: target_context.clone(),
                state: DevToolsWindowState::Normal,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        normal_result.expect("normal surface state should succeed"),
        DevToolsCommandResult::Empty
    );
    assert_eq!(
        evaluate_document_surface_payload(&mut conn, target_context).await,
        json!({
            "hasFocus": true,
            "hidden": false,
            "visibilityState": "visible",
            "fullScreen": false,
            "fullScreenType": "boolean",
            "webkitIsFullScreen": false,
            "webkitIsFullScreenType": "boolean"
        })
    );
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
                expression: "JSON.stringify({ hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState, fullScreen: window.fullScreen, fullScreenType: typeof window.fullScreen, webkitIsFullScreen: document.webkitIsFullScreen, webkitIsFullScreenType: typeof document.webkitIsFullScreen })"
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

#[tokio::test]
async fn devtools_command_rejects_missing_user_context_viewport_override() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (viewport_result, _) = conn
        .execute_devtools_command(DevToolsCommand::SetViewport(DevToolsSetViewportCommand {
            context: context.clone(),
            browser_context_ids: vec![crate::devtools_runtime::DevToolsBrowserContextId::from(
                "custom-user-context",
            )],
            viewport: DevToolsViewportSetting::Dimensions {
                width: 800,
                height: 600,
            },
            device_pixel_ratio: DevToolsDevicePixelRatioSetting::Unchanged,
            screen_width: None,
            screen_height: None,
        }))
        .await
        .into_parts();

    let error = viewport_result.expect_err("missing userContext should fail");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchTarget);
    assert_eq!(error.message, "UnknownBrowserContextId");
}

#[tokio::test]
async fn devtools_command_applies_known_user_context_viewport_default() {
    let mut conn = CdpConnection::new();
    let browser_context = conn.new_browser_context("custom-user-context".to_owned());
    conn.insert_browser_context(browser_context);
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (viewport_result, _) = conn
        .execute_devtools_command(DevToolsCommand::SetViewport(DevToolsSetViewportCommand {
            context: context.clone(),
            browser_context_ids: vec![crate::devtools_runtime::DevToolsBrowserContextId::from(
                "custom-user-context",
            )],
            viewport: DevToolsViewportSetting::Dimensions {
                width: 800,
                height: 600,
            },
            device_pixel_ratio: DevToolsDevicePixelRatioSetting::Scale(2.0),
            screen_width: None,
            screen_height: None,
        }))
        .await
        .into_parts();

    assert_eq!(
        viewport_result.expect("known userContext setViewport should succeed"),
        DevToolsCommandResult::Empty
    );
    let browser_context = conn
        .browser_context_by_id("custom-user-context")
        .expect("custom user context should exist");
    assert!(
        browser_context.emulated_device_metrics.is_none(),
        "userContext viewport should not install a target-scoped active override"
    );
    let default_metrics = browser_context
        .default_emulated_device_metrics
        .as_ref()
        .expect("userContext should hold default emulated device metrics");
    assert_eq!(default_metrics.width, 800);
    assert_eq!(default_metrics.height, 600);
    assert_eq!(default_metrics.device_scale_factor, 2.0);

    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: Some(DevToolsBrowserContextId::from("custom-user-context")),
            activate: true,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target in userContext should succeed")
    else {
        panic!("expected create target result");
    };
    let route = conn
        .target_session_route_for_target_id(create_result.target_id.as_str())
        .expect("created target route");
    let previous_route = conn.replace_none_session_owner_route_override(Some(route));
    let inherited_metrics = conn
        .target_session_owner_emulated_device_metrics(None)
        .expect("new target should inherit userContext default metrics");
    conn.replace_none_session_owner_route_override(previous_route);
    assert_eq!(
        (
            inherited_metrics.width,
            inherited_metrics.height,
            inherited_metrics.device_scale_factor,
        ),
        (800, 600, 2.0)
    );
}

#[tokio::test]
async fn devtools_command_executes_dom_outer_html_for_document_source() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id.clone();
    let url = "data:text/html,<title>DOMSource</title><main>source-owner</main>".to_owned();
    let _ = conn
        .execute_devtools_command(DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: DevToolsCommandContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            url,
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }))
        .await;

    let (source_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetOuterHtml(DevToolsGetOuterHtmlCommand {
            context: DevToolsCommandContext {
                target_id: Some(target_id),
                ..context
            },
            reference: None,
            include_shadow_dom: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::GetOuterHtml(source_result) =
        source_result.expect("get outer html should succeed")
    else {
        panic!("expected get outer html result");
    };
    assert!(
        source_result
            .outer_html
            .contains("<title>DOMSource</title>")
    );
    assert!(source_result.outer_html.contains("source-owner"));
}

#[tokio::test]
async fn devtools_command_executes_dom_query_selector_for_document_root() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id.clone();
    let url = "data:text/html,<main id='target'>target</main><section id='root'><a id='child-link' href='child.html'>Child Link</a></section><a id='top-link' href='top.html'>Top Link</a><input id='field' value='initial'><script>document.getElementById('field').value='changed'</script><p class='item'></p><p class='item'></p>".to_owned();
    let navigation = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: DevToolsCommandContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            url,
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await;
    let DevToolsCommandResult::Navigate(navigation) = navigation.expect("navigate should succeed")
    else {
        panic!("expected navigate result");
    };
    let loader_id = navigation.loader_id.as_ref().expect("navigation loader id");
    crate::testing::wait_until_renderer_document_load(
        &mut ctx,
        None,
        target_id.as_str(),
        loader_id.as_str(),
    )
    .await;
    let conn = &mut ctx.conn;

    let (single_result, _) = conn
        .execute_devtools_command(DevToolsCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                root: None,
                selector: "#target".to_owned(),
                multiple: false,
            },
        ))
        .await
        .into_parts();
    let DevToolsCommandResult::QuerySelector(single_result) =
        single_result.expect("query selector should succeed")
    else {
        panic!("expected query selector result");
    };
    assert_eq!(single_result.node_ids.len(), 1);
    assert!(!single_result.multiple);
    let target_node_id = single_result.node_ids[0];

    let (attributes_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetAttributes(
            DevToolsGetAttributesCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                reference: DevToolsDomNodeReference::FrontendNodeId(target_node_id),
            },
        ))
        .await
        .into_parts();
    let DevToolsCommandResult::GetAttributes(attributes_result) =
        attributes_result.expect("get attributes should succeed")
    else {
        panic!("expected get attributes result");
    };
    assert!(
        attributes_result
            .attributes
            .iter()
            .any(|attribute| { attribute.name == "id" && attribute.value == "target" })
    );

    let (text_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetText(DevToolsGetTextCommand {
            context: DevToolsCommandContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            reference: DevToolsDomNodeReference::FrontendNodeId(target_node_id),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::GetText(text_result) = text_result.expect("get text should succeed")
    else {
        panic!("expected get text result");
    };
    assert_eq!(text_result.text, "target");

    let (property_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetProperty(DevToolsGetPropertyCommand {
            context: DevToolsCommandContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            reference: DevToolsDomNodeReference::FrontendNodeId(target_node_id),
            name: "id".to_owned(),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::GetProperty(property_result) =
        property_result.expect("get property should succeed")
    else {
        panic!("expected get property result");
    };
    assert_eq!(property_result.value, json!("target"));

    let (field_result, _) = conn
        .execute_devtools_command(DevToolsCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                root: None,
                selector: "#field".to_owned(),
                multiple: false,
            },
        ))
        .await
        .into_parts();
    let DevToolsCommandResult::QuerySelector(field_result) =
        field_result.expect("field query selector should succeed")
    else {
        panic!("expected field query selector result");
    };
    let field_node_id = field_result.node_ids[0];

    let (field_value_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetProperty(DevToolsGetPropertyCommand {
            context: DevToolsCommandContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            reference: DevToolsDomNodeReference::FrontendNodeId(field_node_id),
            name: "value".to_owned(),
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::GetProperty(field_value_result) =
        field_value_result.expect("field value property should succeed")
    else {
        panic!("expected field value property result");
    };
    assert_eq!(field_value_result.value, json!("changed"));

    let (resolve_result, _, resolve_events) = conn
        .execute_devtools_command_with_protocol_events(DevToolsCommand::ResolveNode(
            DevToolsResolveNodeCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                reference: DevToolsDomNodeReference::FrontendNodeId(target_node_id),
                execution_context_id: None,
                object_group: Some("dispatch-test".to_owned()),
            },
        ))
        .await
        .into_parts_with_protocol_events();
    assert!(
        resolve_events
            .iter()
            .all(|event| event.protocol_message().is_none()),
        "direct DOM.resolveNode must not route its command response as a protocol event"
    );
    let DevToolsCommandResult::ResolveNode(resolve_result) =
        resolve_result.expect("resolve node should succeed")
    else {
        panic!("expected resolve node result");
    };
    assert_eq!(resolve_result.object["subtype"], json!("node"));
    let resolved_object_id = resolve_result.object["objectId"]
        .as_str()
        .expect("resolved object id")
        .to_owned();

    let (content_quads_result, _, content_quads_events) = conn
        .execute_devtools_command_with_protocol_events(DevToolsCommand::DomGeometry(
            DevToolsDomGeometryCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                reference: DevToolsDomNodeReference::FrontendNodeId(target_node_id),
                operation: DevToolsDomGeometryOperation::GetContentQuads,
            },
        ))
        .await
        .into_parts_with_protocol_events();
    assert!(
        content_quads_events
            .iter()
            .all(|event| event.protocol_message().is_none()),
        "direct DOM.getContentQuads must not route its command response as a protocol event"
    );
    let DevToolsCommandResult::DomGeometry(content_quads_result) =
        content_quads_result.expect("DOM content quads should succeed")
    else {
        panic!("expected DOM geometry result");
    };
    assert_eq!(content_quads_result.quads.len(), 1);
    assert!(content_quads_result.width.is_none());
    assert!(content_quads_result.height.is_none());

    let (object_geometry_result, _, object_geometry_events) = conn
        .execute_devtools_command_with_protocol_events(DevToolsCommand::DomObjectReference(
            crate::devtools_runtime::DevToolsDomObjectReferenceCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                object_id: DevToolsRemoteHandleId::from(resolved_object_id.clone()),
                operation:
                    crate::devtools_runtime::DevToolsDomObjectReferenceOperation::GetBoxModel,
            },
        ))
        .await
        .into_parts_with_protocol_events();
    assert!(
        object_geometry_events
            .iter()
            .all(|event| event.protocol_message().is_none()),
        "direct object-reference DOM.getBoxModel must not route its command response as a protocol event"
    );
    let DevToolsCommandResult::DomGeometry(object_geometry_result) =
        object_geometry_result.expect("object-reference DOM geometry should succeed")
    else {
        panic!("expected DOM geometry result");
    };
    assert_eq!(
        object_geometry_result
            .box_model
            .as_ref()
            .map(|model| model.border.points.len()),
        Some(8)
    );
    assert!(object_geometry_result.quads.is_empty());

    let _ = conn;
    let resolved_property_result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::CallFunction(
            DevToolsCallFunctionCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                realm_id: None,
                world_name: None,
                object_id: Some(DevToolsRemoteHandleId::from(resolved_object_id)),
                this_parameter: None,
                function_declaration: "function() { return this.id; }".to_owned(),
                arguments: Vec::new(),
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::ByValue,
                object_group: None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            },
        ))
        .await;
    let resolved_property = expect_script_value_result(
        resolved_property_result.expect("resolved call function should succeed"),
        "expected resolved call function value",
    );
    assert_eq!(resolved_property.value, json!("target"));

    let conn = &mut ctx.conn;
    let (scroll_result, _) = conn
        .execute_devtools_command(DevToolsCommand::ScrollIntoViewIfNeeded(
            DevToolsScrollIntoViewIfNeededCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                reference: Some(DevToolsDomNodeReference::FrontendNodeId(target_node_id)),
                rect: None,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        scroll_result.expect("scroll into view should succeed"),
        DevToolsCommandResult::Empty
    );

    let (geometry_result, _) = conn
        .execute_devtools_command(DevToolsCommand::DomGeometry(DevToolsDomGeometryCommand {
            context: DevToolsCommandContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            reference: DevToolsDomNodeReference::FrontendNodeId(target_node_id),
            operation: DevToolsDomGeometryOperation::GetBoxModel,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::DomGeometry(geometry_result) =
        geometry_result.expect("DOM geometry should succeed")
    else {
        panic!("expected DOM geometry result");
    };
    let model = geometry_result
        .box_model
        .as_ref()
        .expect("DOM.getBoxModel should return a box model");
    assert_eq!(model.border.points.len(), 8);
    assert!(model.width > 0);
    assert!(model.height > 0);
    assert!(geometry_result.quads.is_empty());

    let (multiple_result, _) = conn
        .execute_devtools_command(DevToolsCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                root: None,
                selector: ".item".to_owned(),
                multiple: true,
            },
        ))
        .await
        .into_parts();
    let DevToolsCommandResult::QuerySelector(multiple_result) =
        multiple_result.expect("query selector all should succeed")
    else {
        panic!("expected query selector result");
    };
    assert_eq!(multiple_result.node_ids.len(), 2);
    assert!(multiple_result.multiple);

    let _ = conn;
    let xpath_result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::LocateNodes(
            DevToolsLocateNodesCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                locator: DevToolsLocateNodesLocator::XPath("//main[@id='target']".to_owned()),
                max_node_count: Some(1),
                start_nodes: Vec::new(),
                start_node_references: Vec::new(),
                serialization_options: None,
            },
        ))
        .await;
    let DevToolsCommandResult::LocateNodes(xpath_result) =
        xpath_result.expect("xpath locate nodes should succeed")
    else {
        panic!("expected locate nodes result");
    };
    assert_eq!(xpath_result.nodes.len(), 1);
    let xpath_backend_node_id = xpath_result.nodes[0]
        .backend_node_id
        .expect("locateNodes should materialize a renderer backendNodeId");
    assert!(
        moli_core::page::is_renderer_backend_node_id(xpath_backend_node_id),
        "locateNodes should carry renderer backend id, got {xpath_backend_node_id}"
    );
    assert_eq!(xpath_result.node_ids, vec![target_node_id]);

    let link_result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::LocateNodes(
            DevToolsLocateNodesCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                locator: DevToolsLocateNodesLocator::LinkText {
                    value: "Top Link".to_owned(),
                    match_type: DevToolsLocateNodesTextMatch::Full,
                },
                max_node_count: Some(1),
                start_nodes: Vec::new(),
                start_node_references: Vec::new(),
                serialization_options: None,
            },
        ))
        .await;
    let DevToolsCommandResult::LocateNodes(link_result) =
        link_result.expect("link text locate nodes should succeed")
    else {
        panic!("expected locate nodes result");
    };
    assert_eq!(link_result.node_ids.len(), 1);

    let root_result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                root: None,
                selector: "#root".to_owned(),
                multiple: false,
            },
        ))
        .await;
    let DevToolsCommandResult::QuerySelector(root_result) =
        root_result.expect("root query selector should succeed")
    else {
        panic!("expected query selector result");
    };
    let root_node_id = root_result.node_ids[0];

    let sent_before_rooted_query = ctx.sent.len();
    let rooted_query_result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                root: Some(DevToolsDomNodeReference::FrontendNodeId(root_node_id)),
                selector: "a".to_owned(),
                multiple: false,
            },
        ))
        .await;
    assert!(
        ctx.sent[sent_before_rooted_query..]
            .iter()
            .all(|message| message.get("id").is_none()),
        "direct rooted DOM.querySelector must not route child-node sidecars as protocol events"
    );
    let DevToolsCommandResult::QuerySelector(rooted_query_result) =
        rooted_query_result.expect("rooted query selector should succeed")
    else {
        panic!("expected query selector result");
    };
    assert_eq!(rooted_query_result.node_ids.len(), 1);
    assert_ne!(rooted_query_result.node_ids[0], link_result.node_ids[0]);

    let root_link_result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::LocateNodes(
            DevToolsLocateNodesCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id),
                    ..context
                },
                locator: DevToolsLocateNodesLocator::LinkText {
                    value: "Link".to_owned(),
                    match_type: DevToolsLocateNodesTextMatch::Partial,
                },
                max_node_count: None,
                start_nodes: Vec::new(),
                start_node_references: vec![DevToolsDomNodeReference::FrontendNodeId(root_node_id)],
                serialization_options: None,
            },
        ))
        .await;
    let DevToolsCommandResult::LocateNodes(root_link_result) =
        root_link_result.expect("rooted link text locate nodes should succeed")
    else {
        panic!("expected locate nodes result");
    };
    assert_eq!(root_link_result.node_ids.len(), 1);
    assert_ne!(root_link_result.node_ids[0], link_result.node_ids[0]);
}

#[tokio::test]
async fn devtools_command_low_backend_node_refs_miss_without_backend_binding() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id.clone();
    let target_context = DevToolsCommandContext {
        target_id: Some(target_id),
        ..context
    };
    let _ = conn
        .execute_devtools_command(DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: target_context.clone(),
            url: "data:text/html,<!doctype html><html><body></body></html>".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }))
        .await;

    let backend_node_id = moli_core::page::RENDERER_BACKEND_NODE_ID_START - 1;

    let mutation = json!({
        "id": 2,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "(() => { const target = document.createElement('button'); target.id = 'fresh-push'; target.setAttribute('data-state', 'live'); const child = document.createElement('span'); child.className = 'fresh-child'; child.textContent = 'fresh text'; target.appendChild(child); document.body.appendChild(target); return 'done'; })()",
            "returnByValue": true
        }
    });
    let pending_mutation = {
        let page = conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .active_target
            .runtime_slot
            .loaded_page_mut()
            .expect("loaded page");
        page.start_runtime_protocol_message(mutation.to_string())
            .expect("runtime mutation should start")
    };
    let mutation_completion = pending_mutation
        .wait()
        .await
        .expect("runtime mutation should complete");

    let (attributes_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetAttributes(
            DevToolsGetAttributesCommand {
                context: target_context.clone(),
                reference: DevToolsDomNodeReference::BackendNodeId(backend_node_id),
            },
        ))
        .await
        .into_parts();
    let error = attributes_result.expect_err("low backendNodeId get attributes should miss");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchNode);
    assert_eq!(error.message, "Could not find node with given id");

    let (text_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetText(DevToolsGetTextCommand {
            context: target_context.clone(),
            reference: DevToolsDomNodeReference::BackendNodeId(backend_node_id),
        }))
        .await
        .into_parts();
    let error = text_result.expect_err("low backendNodeId get text should miss");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchNode);
    assert_eq!(error.message, "Could not find node with given id");

    let (property_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetProperty(DevToolsGetPropertyCommand {
            context: target_context.clone(),
            reference: DevToolsDomNodeReference::BackendNodeId(backend_node_id),
            name: "id".to_owned(),
        }))
        .await
        .into_parts();
    let error = property_result.expect_err("low backendNodeId get property should miss");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchNode);
    assert_eq!(error.message, "Could not find node with given id");

    let (outer_html_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetOuterHtml(DevToolsGetOuterHtmlCommand {
            context: target_context.clone(),
            reference: Some(DevToolsDomNodeReference::BackendNodeId(backend_node_id)),
            include_shadow_dom: false,
        }))
        .await
        .into_parts();
    let error = outer_html_result.expect_err("low backendNodeId get outerHTML should miss");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchNode);
    assert_eq!(error.message, "Could not find node with given id");

    let (describe_result, _) = conn
        .execute_devtools_command(DevToolsCommand::DescribeNode(DevToolsDescribeNodeCommand {
            context: target_context.clone(),
            reference: Some(DevToolsDomNodeReference::BackendNodeId(backend_node_id)),
            depth: 0,
            pierce: false,
        }))
        .await
        .into_parts();
    let error = describe_result.expect_err("low backendNodeId describe should miss");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchNode);
    assert_eq!(error.message, "Could not find node with given id");

    let (rooted_query_result, _) = conn
        .execute_devtools_command(DevToolsCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context: target_context,
                root: Some(DevToolsDomNodeReference::BackendNodeId(backend_node_id)),
                selector: ".fresh-child".to_owned(),
                multiple: false,
            },
        ))
        .await
        .into_parts();
    let error = rooted_query_result.expect_err("low backendNodeId rooted query should miss");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchNode);
    assert_eq!(error.message, "Could not find node with given id");

    let page = conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .loaded_page_mut()
        .expect("loaded page");
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}

#[tokio::test]
async fn devtools_command_dispatches_coordinate_mouse_input_for_target() {
    let mut ctx =
        crate::testing::TestContext::from_conn(crate::testing::real_layout_test_connection());
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let create_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }),
    )
    .await;
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id.clone();
    let url = "data:text/html,<body style='margin:0'><button style='width:80px;height:80px' onclick='window.__clicked = true'>go</button></body>";
    execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: DevToolsCommandContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            url: url.to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await
    .expect("navigation should succeed");

    for (event_type, buttons) in [
        (DevToolsMouseEventType::Pressed, Some(1)),
        (DevToolsMouseEventType::Released, Some(0)),
    ] {
        let result = execute_direct_devtools_command_through_renderer_fence_for_test(
            &mut ctx,
            DevToolsCommand::DispatchMouseEvent(DevToolsDispatchMouseEventCommand {
                context: DevToolsCommandContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                event_type,
                pointer_type: DevToolsPointerType::Mouse,
                x: 20.0,
                y: 20.0,
                button: 0,
                buttons,
                click_count: 1,
                delta_x: 0.0,
                delta_y: 0.0,
                force: 0.0,
                tangential_pressure: 0.0,
                tilt_x: 0.0,
                tilt_y: 0.0,
                twist: 0.0,
                modifiers: 0,
            }),
        )
        .await;
        assert!(
            matches!(result, Ok(DevToolsCommandResult::Empty)),
            "coordinate mouse dispatch should complete through the target renderer: {result:?}"
        );
    }

    let clicked_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
            context: DevToolsCommandContext {
                target_id: Some(target_id),
                ..context
            },
            realm_id: None,
            world_name: None,
            expression: "String(window.__clicked)".to_owned(),
            await_promise: true,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        }),
    )
    .await;
    let clicked_result = expect_script_value_result(
        clicked_result.expect("clicked evaluation should succeed"),
        "expected script value",
    );
    assert_eq!(clicked_result.value, json!("true"));
}

#[tokio::test]
async fn devtools_command_executes_input_key_command_without_cdp_sidecar() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id.clone();
    let target_context = DevToolsCommandContext {
        target_id: Some(target_id.clone()),
        ..context.clone()
    };
    // Programmatic focus is part of parser execution. In contrast, `autofocus`
    // is a post-DOMContentLoaded rendering update and can race the first input
    // command sent to this intentionally inactive target.
    let url = "data:text/html,<input id='field'><script>document.getElementById('field').focus()</script>";
    let (navigate_result, scheduler_events, protocol_events) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: target_context.clone(),
            url: url.to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }))
        .await
        .into_parts_with_protocol_events();
    navigate_result.expect("navigation should succeed");
    ctx.sent
        .extend(crate::testing::protocol_events_into_internal_messages(
            protocol_events,
        ));
    ctx.route_direct_command_output_for_test(Vec::new(), scheduler_events)
        .await;
    ctx.wait_for_direct_command_work_completion_for_test(
        "protocol-neutral navigation load owner action",
    )
    .await;
    ctx.sent.clear();

    let (key_result, _scheduler_events, protocol_events) = ctx
        .conn
        .execute_devtools_command(DevToolsCommand::DispatchKeyEvent(
            DevToolsDispatchKeyEventCommand {
                context: target_context.clone(),
                event_type: DevToolsKeyEventType::KeyPress,
                key: "Z".to_owned(),
                code: "KeyZ".to_owned(),
                text: "Z".to_owned(),
                modifiers: 0,
                auto_repeat: false,
                should_insert_text: true,
            },
        ))
        .await
        .into_parts_with_protocol_events();
    assert_eq!(
        key_result.expect("key dispatch should succeed"),
        DevToolsCommandResult::Empty
    );
    assert!(
        protocol_events.is_empty(),
        "direct input key dispatch must not emit CDP-shaped sidecar messages: {protocol_events:?}"
    );

    let value_result = ctx
        .execute_devtools_command_through_renderer_fence_for_test(DevToolsCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: target_context,
                realm_id: None,
                world_name: None,
                expression: "document.getElementById('field').value".to_owned(),
                await_promise: true,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            },
        ))
        .await;
    let value_result = expect_script_value_result(
        value_result.expect("value evaluation should succeed"),
        "expected script value",
    );
    assert_eq!(value_result.value, json!("Z"));
}

#[tokio::test]
async fn devtools_command_executes_storage_cookie_commands_for_webdriver_context() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = DevToolsCommandContext {
        target_id: Some(create_result.target_id),
        ..context
    };
    let cookie_url = "https://example.com/path".to_owned();

    let (set_result, set_events) = conn
        .execute_devtools_command(DevToolsCommand::SetCookies(DevToolsSetCookiesCommand {
            context: target_context.clone(),
            browser_context_id: None,
            cookies: vec![DevToolsCookieParam {
                name: "sid".to_owned(),
                value: "abc".to_owned(),
                url: Some(cookie_url.clone()),
                domain: None,
                path: Some("/".to_owned()),
                secure: Some(true),
                http_only: true,
                same_site: Some("Lax".to_owned()),
                priority: None,
                source_scheme: None,
                source_port: None,
                partition_key: None,
                partition_key_opaque: None,
                expires: None,
            }],
        }))
        .await
        .into_parts();
    assert!(
        set_events.is_empty(),
        "direct storage setCookies must not emit CDP-shaped sidecar messages: {set_events:?}"
    );
    let DevToolsCommandResult::SetCookies(set_result) =
        set_result.expect("set cookies should succeed")
    else {
        panic!("expected set cookies result");
    };
    assert!(set_result.success);

    let (get_result, get_events) = conn
        .execute_devtools_command(DevToolsCommand::GetCookies(DevToolsGetCookiesCommand {
            context: target_context.clone(),
            browser_context_id: None,
            urls: Some(vec![cookie_url.clone()]),
            filter: None,
        }))
        .await
        .into_parts();
    assert!(
        get_events.is_empty(),
        "direct storage getCookies must not emit CDP-shaped sidecar messages: {get_events:?}"
    );
    let DevToolsCommandResult::GetCookies(get_result) =
        get_result.expect("get cookies should succeed")
    else {
        panic!("expected get cookies result");
    };
    assert_eq!(get_result.cookies.len(), 1);
    assert_eq!(get_result.cookies[0]["name"], json!("sid"));
    assert_eq!(get_result.cookies[0]["value"], json!("abc"));

    let (delete_result, delete_events) = conn
        .execute_devtools_command(DevToolsCommand::DeleteCookies(
            DevToolsDeleteCookiesCommand {
                context: target_context.clone(),
                browser_context_id: None,
                name: Some("sid".to_owned()),
                url: Some(cookie_url.clone()),
                domain: None,
                path: None,
                partition_key: None,
                filter: None,
            },
        ))
        .await
        .into_parts();
    assert!(
        delete_events.is_empty(),
        "direct storage deleteCookies must not emit CDP-shaped sidecar messages: {delete_events:?}"
    );
    assert!(matches!(
        delete_result.expect("delete cookie should succeed"),
        DevToolsCommandResult::DeleteCookies(_)
    ));

    let (after_delete, after_delete_events) = conn
        .execute_devtools_command(DevToolsCommand::GetCookies(DevToolsGetCookiesCommand {
            context: target_context,
            browser_context_id: None,
            urls: Some(vec![cookie_url]),
            filter: None,
        }))
        .await
        .into_parts();
    assert!(
        after_delete_events.is_empty(),
        "direct storage getCookies after delete must not emit CDP-shaped sidecar messages: {after_delete_events:?}"
    );
    let DevToolsCommandResult::GetCookies(after_delete) =
        after_delete.expect("get cookies after delete should succeed")
    else {
        panic!("expected get cookies result");
    };
    assert!(after_delete.cookies.is_empty());
}

#[tokio::test]
async fn devtools_storage_cookie_commands_scope_to_target_browser_context() {
    let mut conn = CdpConnection::new();
    let mut default_context = BrowserContext::new("BID-default".to_owned());
    default_context.set_active_target_id("TID-default".to_owned());
    let mut custom_context = BrowserContext::new("BID-custom".to_owned());
    custom_context.set_active_target_id("TID-custom".to_owned());
    conn.browser_context = Some(default_context);
    conn.inactive_browser_contexts.push(custom_context);

    let base_context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let default_target_context = DevToolsCommandContext {
        target_id: Some(DevToolsTargetId::from("TID-default")),
        ..base_context.clone()
    };
    let custom_target_context = DevToolsCommandContext {
        target_id: Some(DevToolsTargetId::from("TID-custom")),
        ..base_context
    };
    let cookie_url = "https://example.com/path".to_owned();

    let (set_result, _) = conn
        .execute_devtools_command(DevToolsCommand::SetCookies(DevToolsSetCookiesCommand {
            context: custom_target_context.clone(),
            browser_context_id: None,
            cookies: vec![DevToolsCookieParam {
                name: "targetScoped".to_owned(),
                value: "custom".to_owned(),
                url: Some(cookie_url.clone()),
                domain: None,
                path: Some("/".to_owned()),
                secure: Some(true),
                http_only: false,
                same_site: Some("Lax".to_owned()),
                priority: None,
                source_scheme: None,
                source_port: None,
                partition_key: None,
                partition_key_opaque: None,
                expires: None,
            }],
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::SetCookies(set_result) =
        set_result.expect("custom target setCookies should succeed")
    else {
        panic!("expected set cookies result");
    };
    assert!(set_result.success);

    let (custom_get, _) = conn
        .execute_devtools_command(DevToolsCommand::GetCookies(DevToolsGetCookiesCommand {
            context: custom_target_context,
            browser_context_id: None,
            urls: Some(vec![cookie_url.clone()]),
            filter: None,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::GetCookies(custom_get) =
        custom_get.expect("custom target getCookies should succeed")
    else {
        panic!("expected get cookies result");
    };
    assert_eq!(custom_get.cookies.len(), 1);
    assert_eq!(custom_get.cookies[0]["name"], json!("targetScoped"));
    assert_eq!(custom_get.cookies[0]["value"], json!("custom"));

    let (default_get, _) = conn
        .execute_devtools_command(DevToolsCommand::GetCookies(DevToolsGetCookiesCommand {
            context: default_target_context,
            browser_context_id: None,
            urls: Some(vec![cookie_url]),
            filter: None,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::GetCookies(default_get) =
        default_get.expect("default target getCookies should succeed")
    else {
        panic!("expected get cookies result");
    };
    assert!(default_get.cookies.is_empty());
}

#[tokio::test]
async fn devtools_command_executes_navigation_history_and_traverse() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id;
    let target_context = DevToolsCommandContext {
        target_id: Some(target_id.clone()),
        ..context
    };
    let first_url = "data:text/html,<title>A</title>classic-a".to_owned();
    let second_url = "data:text/html,<title>B</title>classic-b".to_owned();

    for url in [&first_url, &second_url] {
        let (navigate_result, _, _, _) = conn
            .execute_devtools_command(DevToolsCommand::Navigate(DevToolsNavigateCommand {
                context: target_context.clone(),
                url: (*url).clone(),
                referrer: None,
                wait: DevToolsNavigationWait::Load,
            }))
            .await
            .into_complete_parts();
        navigate_result.expect("navigate should succeed");
    }

    let (history_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetNavigationHistory(
            DevToolsGetNavigationHistoryCommand {
                context: target_context.clone(),
            },
        ))
        .await
        .into_parts();
    let DevToolsCommandResult::GetNavigationHistory(history) =
        history_result.expect("history should succeed")
    else {
        panic!("expected navigation history result");
    };
    assert!(history.current_index > 0);
    assert_eq!(history.entries[history.current_index].url, second_url);
    let previous = history.entries[history.current_index - 1].clone();

    let (traverse_result, _, _, _) = conn
        .execute_devtools_command(DevToolsCommand::TraverseHistory(
            DevToolsTraverseHistoryCommand {
                context: target_context.clone(),
                destination: DevToolsHistoryTraversalDestination::Entry {
                    entry_id: previous.id,
                    url: previous.url.clone(),
                },
                wait: DevToolsNavigationWait::Load,
            },
        ))
        .await
        .into_complete_parts();
    assert!(matches!(
        traverse_result.expect("traverse should succeed"),
        DevToolsCommandResult::TraverseHistory(DevToolsTraverseHistoryResult {
            same_document: false
        })
    ));

    let (history_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetNavigationHistory(
            DevToolsGetNavigationHistoryCommand {
                context: target_context.clone(),
            },
        ))
        .await
        .into_parts();
    let DevToolsCommandResult::GetNavigationHistory(history) =
        history_result.expect("history after traverse should succeed")
    else {
        panic!("expected navigation history result after traverse");
    };
    assert_eq!(history.entries[history.current_index].id, previous.id);

    let (delta_result, _, _, _) = conn
        .execute_devtools_command(DevToolsCommand::TraverseHistory(
            DevToolsTraverseHistoryCommand {
                context: target_context.clone(),
                destination: DevToolsHistoryTraversalDestination::Delta(1),
                wait: DevToolsNavigationWait::Load,
            },
        ))
        .await
        .into_complete_parts();
    assert!(matches!(
        delta_result.expect("delta traverse should succeed"),
        DevToolsCommandResult::TraverseHistory(DevToolsTraverseHistoryResult {
            same_document: false
        })
    ));

    let (history_result, _) = conn
        .execute_devtools_command(DevToolsCommand::GetNavigationHistory(
            DevToolsGetNavigationHistoryCommand {
                context: target_context.clone(),
            },
        ))
        .await
        .into_parts();
    let DevToolsCommandResult::GetNavigationHistory(history) =
        history_result.expect("history after delta traverse should succeed")
    else {
        panic!("expected navigation history result after delta traverse");
    };
    assert_eq!(history.entries[history.current_index].url, second_url);

    let (out_of_range_result, _) = conn
        .execute_devtools_command(DevToolsCommand::TraverseHistory(
            DevToolsTraverseHistoryCommand {
                context: target_context,
                destination: DevToolsHistoryTraversalDestination::Delta(1),
                wait: DevToolsNavigationWait::Load,
            },
        ))
        .await
        .into_parts();
    let error = out_of_range_result.expect_err("out-of-range delta should fail");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchHistoryEntry);
}

#[tokio::test]
async fn devtools_command_executes_context_preload_add_and_remove() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let create_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }),
    )
    .await;
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = DevToolsCommandContext {
        target_id: Some(create_result.target_id.clone()),
        ..context.clone()
    };

    let add_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::AddPreloadScript(DevToolsAddPreloadScriptCommand {
            context: target_context.clone(),
            source: DevToolsPreloadScriptSource::FunctionDeclaration {
                function_declaration: "() => { globalThis.__bidiPreload = 'from-preload'; }"
                    .to_owned(),
                arguments: Vec::new(),
            },
            world_name: None,
            target_ids: Some(vec![create_result.target_id.clone()]),
            browser_context_ids: Vec::new(),
            run_immediately: false,
            include_command_line_api: false,
        }),
    )
    .await;
    let DevToolsCommandResult::AddPreloadScript(add_result) =
        add_result.expect("addPreloadScript should succeed")
    else {
        panic!("expected add preload script result");
    };
    assert!(
        add_result
            .script_id
            .as_str()
            .starts_with(create_result.target_id.as_str()),
        "BiDi preload ids should be target-qualified"
    );

    let navigate_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: target_context.clone(),
            url: "data:text/html,bidi-preload".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await;
    navigate_result.expect("navigate should run preload script");

    let evaluate_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
            context: target_context.clone(),
            realm_id: None,
            world_name: None,
            expression: "globalThis.__bidiPreload".to_owned(),
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
    let evaluate_result = expect_script_value_result(
        evaluate_result.expect("preload value should evaluate"),
        "expected script value result",
    );
    assert_eq!(evaluate_result.value, json!("from-preload"));

    let remove_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::RemovePreloadScript(DevToolsRemovePreloadScriptCommand {
            context,
            script_id: add_result.script_id.clone(),
        }),
    )
    .await;
    assert_eq!(
        remove_result.expect("removePreloadScript should succeed"),
        DevToolsCommandResult::Empty
    );

    let navigate_after_remove_result =
        execute_direct_devtools_command_through_renderer_fence_for_test(
            &mut ctx,
            DevToolsCommand::Navigate(DevToolsNavigateCommand {
                context: target_context.clone(),
                url: "data:text/html,bidi-preload-removed".to_owned(),
                referrer: None,
                wait: DevToolsNavigationWait::Load,
            }),
        )
        .await;
    navigate_after_remove_result.expect("navigate after remove should succeed");

    let evaluate_after_remove_result =
        execute_direct_devtools_command_through_renderer_fence_for_test(
            &mut ctx,
            DevToolsCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
                context: target_context,
                realm_id: None,
                world_name: None,
                expression: "typeof globalThis.__bidiPreload".to_owned(),
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
    let after_remove_result = expect_script_value_result(
        evaluate_after_remove_result.expect("post-remove value should evaluate"),
        "expected script value result after remove",
    );
    assert_eq!(after_remove_result.value, json!("undefined"));

    let remove_again_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::RemovePreloadScript(DevToolsRemovePreloadScriptCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                target_id: None,
                browser_context_id: None,
            },
            script_id: add_result.script_id,
        }),
    )
    .await;
    assert_eq!(
        remove_again_result
            .expect_err("removing a BiDi preload twice should fail")
            .kind,
        DevToolsErrorKind::NoSuchScript
    );
}

#[tokio::test]
async fn devtools_command_executes_default_preload_add_and_remove_without_loaded_target() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (add_result, _) = conn
        .execute_devtools_command(DevToolsCommand::AddPreloadScript(
            DevToolsAddPreloadScriptCommand {
                context: context.clone(),
                source: DevToolsPreloadScriptSource::FunctionDeclaration {
                    function_declaration: "() => { globalThis.__bidiDefaultPreload = true; }"
                        .to_owned(),
                    arguments: Vec::new(),
                },
                world_name: None,
                target_ids: None,
                browser_context_ids: Vec::new(),
                run_immediately: false,
                include_command_line_api: false,
            },
        ))
        .await
        .into_parts();
    let DevToolsCommandResult::AddPreloadScript(add_result) =
        add_result.expect("default addPreloadScript should succeed")
    else {
        panic!("expected add preload script result");
    };
    assert!(
        !add_result.script_id.as_str().contains(':'),
        "default BiDi preload ids should not be target-qualified"
    );
    let browser_context = conn
        .browser_context
        .as_ref()
        .expect("default preload should materialize the default browser context");
    assert!(!browser_context.has_active_target());
    assert_eq!(browser_context.default_document_start_scripts.len(), 1);

    let (remove_result, _) = conn
        .execute_devtools_command(DevToolsCommand::RemovePreloadScript(
            DevToolsRemovePreloadScriptCommand {
                context: context.clone(),
                script_id: add_result.script_id.clone(),
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        remove_result.expect("default removePreloadScript should succeed"),
        DevToolsCommandResult::Empty
    );
    assert!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .default_document_start_scripts
            .is_empty(),
        "default preload removal should clear the browser-context registry"
    );

    let (remove_again_result, _) = conn
        .execute_devtools_command(DevToolsCommand::RemovePreloadScript(
            DevToolsRemovePreloadScriptCommand {
                context,
                script_id: add_result.script_id,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        remove_again_result
            .expect_err("removing a default BiDi preload twice should fail")
            .kind,
        DevToolsErrorKind::NoSuchScript
    );
}

#[tokio::test]
async fn devtools_command_executes_user_context_preload_without_default_leakage() {
    let mut ctx = crate::testing::TestContext::new();
    let default_context_id = ctx.conn.default_browser_context_id().to_owned();
    let default_browser_context = ctx.conn.new_browser_context(default_context_id.clone());
    ctx.conn.insert_browser_context(default_browser_context);
    let custom_browser_context = ctx
        .conn
        .new_browser_context("custom-user-context".to_owned());
    ctx.conn.insert_browser_context(custom_browser_context);
    let second_browser_context = ctx
        .conn
        .new_browser_context("second-user-context".to_owned());
    ctx.conn.insert_browser_context(second_browser_context);
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let add_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::AddPreloadScript(DevToolsAddPreloadScriptCommand {
            context: context.clone(),
            source: DevToolsPreloadScriptSource::FunctionDeclaration {
                function_declaration: "() => { globalThis.__bidiUserContextPreload = 'custom'; }"
                    .to_owned(),
                arguments: Vec::new(),
            },
            world_name: None,
            target_ids: None,
            browser_context_ids: vec![
                DevToolsBrowserContextId::from("custom-user-context"),
                DevToolsBrowserContextId::from("second-user-context"),
            ],
            run_immediately: false,
            include_command_line_api: false,
        }),
    )
    .await;
    let DevToolsCommandResult::AddPreloadScript(add_result) =
        add_result.expect("userContext addPreloadScript should succeed")
    else {
        panic!("expected add preload script result");
    };
    assert!(
        !add_result.script_id.as_str().contains(':'),
        "userContext preload ids are browser-context scoped"
    );
    let script_id = add_result.script_id.clone();

    let custom_target = create_target_in_browser_context_through_renderer_fence_for_test(
        &mut ctx,
        &context,
        "custom-user-context",
        "custom userContext target",
    )
    .await;
    let custom_context = DevToolsCommandContext {
        target_id: Some(custom_target.clone()),
        ..context.clone()
    };
    execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: custom_context.clone(),
            url: "data:text/html,user-context-preload".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await
    .expect("custom userContext navigation should succeed");
    let custom_value = evaluate_string_through_renderer_fence_for_test(
        &mut ctx,
        custom_context,
        "globalThis.__bidiUserContextPreload",
        "custom preload value",
    )
    .await;
    assert_eq!(custom_value, "custom");

    let second_target = create_target_in_browser_context_through_renderer_fence_for_test(
        &mut ctx,
        &context,
        "second-user-context",
        "second userContext target",
    )
    .await;
    let second_context = DevToolsCommandContext {
        target_id: Some(second_target.clone()),
        ..context.clone()
    };
    execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: second_context.clone(),
            url: "data:text/html,second-user-context-preload".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await
    .expect("second userContext navigation should succeed");
    let second_value = evaluate_string_through_renderer_fence_for_test(
        &mut ctx,
        second_context,
        "globalThis.__bidiUserContextPreload",
        "second preload value",
    )
    .await;
    assert_eq!(second_value, "custom");

    let default_target = create_target_in_browser_context_through_renderer_fence_for_test(
        &mut ctx,
        &context,
        &default_context_id,
        "default userContext target",
    )
    .await;
    let default_context = DevToolsCommandContext {
        target_id: Some(default_target),
        ..context.clone()
    };
    execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: default_context.clone(),
            url: "data:text/html,default-context-preload".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await
    .expect("default userContext navigation should succeed");
    let default_value = evaluate_string_through_renderer_fence_for_test(
        &mut ctx,
        default_context,
        "typeof globalThis.__bidiUserContextPreload",
        "default preload absence",
    )
    .await;
    assert_eq!(default_value, "undefined");

    let remove_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::RemovePreloadScript(DevToolsRemovePreloadScriptCommand {
            context,
            script_id: script_id.clone(),
        }),
    )
    .await;
    assert_eq!(
        remove_result.expect("userContext removePreloadScript should succeed"),
        DevToolsCommandResult::Empty
    );
    for browser_context_id in ["custom-user-context", "second-user-context"] {
        assert!(
            !ctx.conn
                .browser_context_by_id(browser_context_id)
                .expect("browser context should still exist")
                .has_default_document_start_script(script_id.as_str()),
            "removePreloadScript should clear browser-context scoped script from {browser_context_id}"
        );
    }
}

#[tokio::test]
async fn devtools_runtime_call_function_channel_does_not_emit_direct_script_message_sidecar() {
    let mut conn = CdpConnection::new();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_devtools_command(DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }))
        .await
        .into_parts();
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = DevToolsCommandContext {
        target_id: Some(create_result.target_id.clone()),
        ..context
    };
    conn.execute_devtools_command(DevToolsCommand::Navigate(DevToolsNavigateCommand {
        context: target_context.clone(),
        url: "data:text/html,bidi-script-channel".to_owned(),
        referrer: None,
        wait: DevToolsNavigationWait::Load,
    }))
    .await
    .into_complete_parts()
    .0
    .expect("navigate should succeed before script message channel call");

    let call = conn
        .execute_devtools_command(DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
            context: target_context.clone(),
            realm_id: None,
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration: "(channel) => channel('foo')".to_owned(),
            arguments: vec![json!({
                "type": "channel",
                "value": {
                    "channel": "channel_name"
                }
            })],
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::None,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        }))
        .await;
    let (call_result, _, protocol_events, _renderer_output_predecessor) =
        call.into_complete_parts();
    expect_script_value_result(
        call_result.expect("channel callFunction should succeed"),
        "expected channel callFunction script value result",
    );

    let script_message_events = protocol_events
        .into_iter()
        .filter_map(|event| event.into_parts().1)
        .filter_map(|event| match event {
            AutomationEvent::ScriptMessage(event) => Some(event),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        script_message_events.is_empty(),
        "direct protocol-neutral Runtime command should not surface BiDi script.message sidecar"
    );
}

#[tokio::test]
async fn devtools_command_executes_script_evaluate_and_call_function() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let create_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        }),
    )
    .await;
    let DevToolsCommandResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = DevToolsCommandContext {
        target_id: Some(create_result.target_id.clone()),
        ..context.clone()
    };
    let navigate_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::Navigate(DevToolsNavigateCommand {
            context: target_context.clone(),
            url: "data:text/html,bidi-script".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await;
    navigate_result.expect("navigate should succeed before script evaluation");

    let realms_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::GetRealms(DevToolsGetRealmsCommand {
            context: target_context.clone(),
            realm_type: Some("window".to_owned()),
        }),
    )
    .await;
    let DevToolsCommandResult::Realms(realms_result) =
        realms_result.expect("getRealms should succeed")
    else {
        panic!("expected realms result");
    };
    let default_realm_id = realms_result
        .realms
        .iter()
        .find(|realm| {
            realm.realm_id.is_some()
                && realm.frame_id.as_ref().map(|id| id.as_str())
                    == target_context.target_id.as_ref().map(|id| id.as_str())
                && realm.context_type.as_deref() == Some("default")
        })
        .and_then(|realm| realm.realm_id.clone())
        .expect("getRealms should expose the target default window realm");

    let evaluate_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
            context: target_context.clone(),
            realm_id: None,
            world_name: None,
            expression: "1 + 2".to_owned(),
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
    let evaluate_result = expect_script_value_result(
        evaluate_result.expect("evaluate should succeed"),
        "expected script value result",
    );
    assert_eq!(evaluate_result.value, json!(3));
    assert!(evaluate_result.handle.is_none());

    let realm_context = DevToolsCommandContext {
        target_id: None,
        ..context.clone()
    };
    let realm_evaluate_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
            context: realm_context.clone(),
            realm_id: Some(default_realm_id.clone()),
            world_name: None,
            expression: "2 + 3".to_owned(),
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
    let realm_evaluate_result = expect_script_value_result(
        realm_evaluate_result.expect("realm-target evaluate should succeed"),
        "expected realm-target script value result",
    );
    assert_eq!(realm_evaluate_result.value, json!(5));
    assert!(realm_evaluate_result.handle.is_none());

    let realm_call_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
            context: realm_context,
            realm_id: Some(default_realm_id),
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration: "(value) => value * 2".to_owned(),
            arguments: vec![json!({"type": "number", "value": 6})],
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::None,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        }),
    )
    .await;
    let realm_call_result = expect_script_value_result(
        realm_call_result.expect("realm-target callFunction should succeed"),
        "expected realm-target callFunction value result",
    );
    assert_eq!(realm_call_result.value, json!(12));
    assert!(realm_call_result.handle.is_none());

    let owned_evaluate_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
            context: target_context.clone(),
            realm_id: None,
            world_name: None,
            expression: "({value: 42})".to_owned(),
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
    let owned_evaluate_result = expect_script_value_result(
        owned_evaluate_result.expect("owned evaluate should succeed"),
        "expected owned script value result",
    );
    let handle = owned_evaluate_result
        .handle
        .expect("root-owned object result should return a handle");

    let unknown_release_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::ReleaseObjects(DevToolsReleaseObjectsCommand {
            context: target_context.clone(),
            realm_id: None,
            world_name: None,
            handles: vec!["unknown_handle".into()],
        }),
    )
    .await;
    assert_eq!(
        unknown_release_result.expect("unknown releaseObjects should be ignored"),
        DevToolsCommandResult::Empty
    );

    let still_owned_call_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
            context: target_context.clone(),
            realm_id: None,
            world_name: None,
            object_id: Some(handle.clone()),
            this_parameter: None,
            function_declaration: "function() { return this.value; }".to_owned(),
            arguments: Vec::new(),
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::None,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        }),
    )
    .await;
    let still_owned_result = expect_script_value_result(
        still_owned_call_result.expect("unknown release should not drop known handle"),
        "expected still-owned callFunction value result",
    );
    assert_eq!(still_owned_result.value, json!(42));

    let this_parameter_call_result =
        execute_direct_devtools_command_through_renderer_fence_for_test(
            &mut ctx,
            DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
                context: target_context.clone(),
                realm_id: None,
                world_name: None,
                object_id: None,
                this_parameter: Some(json!({"handle": handle.as_str()})),
                function_declaration: "function() { return this.value; }".to_owned(),
                arguments: Vec::new(),
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                object_group: None,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: false,
                serialization_options: None,
            }),
        )
        .await;
    let this_parameter_result = expect_script_value_result(
        this_parameter_call_result.expect("this-parameter callFunction should succeed"),
        "expected this-parameter callFunction value result",
    );
    assert_eq!(this_parameter_result.value, json!(42));

    let release_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::ReleaseObjects(DevToolsReleaseObjectsCommand {
            context: target_context.clone(),
            realm_id: None,
            world_name: None,
            handles: vec!["unknown_handle".into(), handle.clone()],
        }),
    )
    .await;
    assert_eq!(
        release_result.expect("releaseObjects should succeed"),
        DevToolsCommandResult::Empty
    );

    let released_call_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
            context: target_context.clone(),
            realm_id: None,
            world_name: None,
            object_id: Some(handle),
            this_parameter: None,
            function_declaration: "function() { return this.value; }".to_owned(),
            arguments: Vec::new(),
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::None,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        }),
    )
    .await;
    assert!(
        released_call_result.is_err(),
        "released handle should no longer be valid for shared runtime calls"
    );

    let call_result = execute_direct_devtools_command_through_renderer_fence_for_test(
        &mut ctx,
        DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
            context: target_context,
            realm_id: None,
            world_name: None,
            object_id: None,
            this_parameter: None,
            function_declaration: "(value) => value + 1".to_owned(),
            arguments: vec![json!({"type": "number", "value": 4})],
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::None,
            object_group: None,
            preserve_remote_metadata: false,
            materialize_bidi_script_result: false,
            serialization_options: None,
        }),
    )
    .await;
    let call_result = expect_script_value_result(
        call_result.expect("callFunction should succeed"),
        "expected callFunction value result",
    );
    assert_eq!(call_result.value, json!(5));
    assert!(call_result.handle.is_none());
}

#[test]
fn command_dispatch_completes_parse_errors_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let step = conn.start_command_dispatch("{");
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": null,
            "error": {"code": -32700, "message": "Parse error"}
        })]
    );
}

#[test]
fn command_dispatch_completes_invalid_methods_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let raw = serde_json::to_string(&json!({
        "id": 7,
        "method": "MalformedMethod",
        "sessionId": "SID-1"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 7,
            "error": {"code": -32600, "message": "Invalid method"},
            "sessionId": "SID-1"
        })]
    );
}

#[test]
fn command_dispatch_completes_startup_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let raw = serde_json::to_string(&json!({
        "id": 8,
        "method": "Page.getFrameTree",
        "sessionId": "STARTUP"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    let messages = complete_messages(step);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["id"], json!(8));
    assert_eq!(messages[0]["sessionId"], json!("STARTUP"));
    assert_eq!(
        messages[0]["result"]["frameTree"]["frame"]["id"],
        json!("TID-STARTUP")
    );
}

#[test]
fn command_dispatch_completes_unknown_domains_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    conn.browser_context = Some(bc);
    let raw = serde_json::to_string(&json!({
        "id": 9,
        "method": "Nope.command",
        "sessionId": "SID-1"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 9,
            "error": {"code": -32601, "message": "Unknown domain"},
            "sessionId": "SID-1"
        })]
    );
}

#[test]
fn command_dispatch_completes_console_log_and_inspector_owner_commands() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-dispatch".to_owned()));

    for (id, method) in [
        (10, "Console.enable"),
        (11, "Log.enable"),
        (12, "Inspector.enable"),
    ] {
        let raw = serde_json::to_string(&json!({ "id": id, "method": method })).unwrap();
        let step = conn.start_command_dispatch(&raw);
        let messages = complete_messages(step);
        assert_eq!(
            messages[0],
            json!({ "id": id, "result": {} }),
            "{method} should complete through the command dispatch entry"
        );
    }
}

#[test]
fn command_dispatch_completes_browser_sync_commands() {
    let mut conn = CdpConnection::new();
    for (id, method) in [
        (20, "Browser.getVersion"),
        (21, "Browser.getWindowForTarget"),
        (22, "Browser.setWindowBounds"),
        (23, "Browser.setDownloadBehavior"),
    ] {
        let params = match method {
            "Browser.setWindowBounds" => json!({
                "windowId": 1_923_710_101_i64,
                "bounds": { "windowState": "normal", "width": 800, "height": 600 }
            }),
            "Browser.setDownloadBehavior" => json!({
                "behavior": "allow",
                "downloadPath": "/tmp/moli-downloads"
            }),
            _ => json!({}),
        };
        let raw = serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
            .unwrap();
        let step = conn.start_command_dispatch(&raw);
        let messages = complete_messages(step);
        assert_eq!(messages.len(), 1, "{method} should emit one response");
        assert_eq!(messages[0]["id"], json!(id));
        assert!(
            messages[0].get("result").is_some(),
            "{method} should complete successfully: {:?}",
            messages[0]
        );
    }
}

#[test]
fn command_dispatch_completes_browser_owner_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    for (id, method, params, expects_result) in [
        (
            24,
            "Browser.openDownloadAsStream",
            json!({ "guid": "missing-download-guid" }),
            false,
        ),
        (
            25,
            "Browser.setPermission",
            json!({
                "permission": { "name": "geolocation" },
                "setting": "denied"
            }),
            true,
        ),
        (
            26,
            "Browser.grantPermissions",
            json!({
                "permissions": [{ "name": "notifications" }]
            }),
            true,
        ),
        (27, "Browser.resetPermissions", json!({}), true),
    ] {
        let raw = serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
            .unwrap();
        let step = conn.start_command_dispatch(&raw);
        let messages = complete_messages(step);
        assert_eq!(messages.len(), 1, "{method} should emit one response");
        assert_eq!(messages[0]["id"], json!(id));
        if expects_result {
            assert!(
                messages[0].get("result").is_some(),
                "{method} should complete successfully: {:?}",
                messages[0]
            );
        } else {
            assert!(
                messages[0].get("error").is_some(),
                "{method} should complete with a protocol error: {:?}",
                messages[0]
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_browser_permission_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-browser-permission-live".to_owned());
    browser_context.set_active_target_id("TID-browser-permission-live".to_owned());
    conn.browser_context = Some(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>browser permission</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 28,
        "method": "Browser.setPermission",
        "params": {
            "permission": { "name": "geolocation" },
            "setting": "denied"
        }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Browser.setPermission should update the live page")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 28, "result": {} })]
    );
    assert_eq!(conn.permission_overrides.len(), 1);
}

#[tokio::test]
async fn command_dispatch_completes_target_startup_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let create_raw = serde_json::to_string(&json!({
        "id": 29,
        "method": "Target.createTarget",
        "params": { "url": "about:blank" }
    }))
    .unwrap();
    let create_step = conn.start_command_dispatch(&create_raw);
    let create_messages = match create_step {
        CdpCommandTaskStep::Pending(pending) => {
            complete_command_task_for_test(&mut conn, *pending).await
        }
        CdpCommandTaskStep::Complete(outcome) => outcome.into_parts().0,
    };
    assert_eq!(create_messages.len(), 1);
    assert_eq!(create_messages[0]["id"], json!(29));
    let target_id = create_messages[0]["result"]["targetId"]
        .as_str()
        .expect("createTarget should return targetId")
        .to_owned();
    let browser_context = conn.browser_context.as_ref().expect("browser context");
    assert_eq!(browser_context.active_target_id(), Some(target_id.as_str()));
    assert!(
        browser_context.active_target.runtime_slot.has_loaded_page(),
        "Target.createTarget should complete target lifecycle initial document ensure"
    );

    let attach_raw = serde_json::to_string(&json!({
        "id": 30,
        "method": "Target.attachToTarget",
        "params": { "targetId": target_id }
    }))
    .unwrap();
    let attach_step = conn.start_command_dispatch(&attach_raw);
    let attach_messages = match attach_step {
        CdpCommandTaskStep::Pending(pending) => {
            complete_command_task_for_test(&mut conn, *pending).await
        }
        CdpCommandTaskStep::Complete(outcome) => outcome.into_parts().0,
    };
    assert_eq!(attach_messages.len(), 2);
    assert_eq!(
        attach_messages[0]["method"],
        json!("Target.attachedToTarget")
    );
    let attached_session_id = attach_messages[0]["params"]["sessionId"]
        .as_str()
        .expect("attachedToTarget should identify the new session");
    assert_eq!(attach_messages[1]["id"], json!(30));
    assert_eq!(
        attach_messages[1]["result"]["sessionId"],
        json!(attached_session_id)
    );
}

#[test]
fn command_dispatch_completes_network_sync_settings() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-network".to_owned()));

    for (id, method, params) in [
        (30, "Network.enable", json!({})),
        (31, "Network.disable", json!({})),
        (
            32,
            "Network.setCacheDisabled",
            json!({"cacheDisabled": true}),
        ),
        (
            33,
            "Network.setBypassServiceWorker",
            json!({"bypass": true}),
        ),
    ] {
        let raw = serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
            .unwrap();
        let step = conn.start_command_dispatch(&raw);
        assert_eq!(
            complete_messages(step),
            vec![json!({ "id": id, "result": {} })],
            "{method} should complete through the command dispatch entry"
        );
    }
}

#[test]
fn command_dispatch_completes_page_dialog_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let raw = serde_json::to_string(&json!({
        "id": 40,
        "method": "Page.handleJavaScriptDialog",
        "params": {"accept": true}
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 40,
            "error": {"code": -32602, "message": "No dialog is showing"}
        })]
    );
}

#[test]
fn command_dispatch_reports_no_document_for_default_page_screenshot() {
    let mut conn = crate::testing::real_layout_test_connection();
    conn.browser_context = Some(BrowserContext::new("BID-page-shot".to_owned()));
    let raw = serde_json::to_string(&json!({
        "id": 41,
        "method": "Page.captureScreenshot"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 41,
            "error": {
                "code": -32000,
                "message": "NoDocumentLoaded"
            }
        })]
    );
}

#[test]
fn command_dispatch_preserves_page_screenshot_unsupported_for_mock_layout() {
    let mut conn = CdpConnection::new_with_initial_storage_partition_and_runtime_config(
        crate::CdpInitialStoragePartition::memory(),
        moli_core::runtime::NavigationRuntimeConfig::new(
            moli_fetch::FetchConfig::default(),
            moli_core::OptionalResourceFetchMask::NONE,
            true,
            moli_core::LayoutPolicy::Mock,
        ),
    );
    conn.browser_context = Some(BrowserContext::new("BID-page-shot-mock".to_owned()));
    let raw = serde_json::to_string(&json!({
        "id": 42,
        "method": "Page.captureScreenshot"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 42,
            "error": {
                "code": -32000,
                "message": "Page.captureScreenshot is not supported: renderer screenshots are not implemented."
            }
        })]
    );
}

#[test]
fn command_dispatch_completes_additional_page_sync_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-page-sync".to_owned());
    browser_context.set_active_target_id("TID-page-sync");
    conn.browser_context = Some(browser_context);

    let download_raw = serde_json::to_string(&json!({
        "id": 411,
        "method": "Page.setDownloadBehavior",
        "params": { "behavior": "allow", "downloadPath": "/tmp/moli-downloads" }
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&download_raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 411, "result": {} })]
    );
    let settings = conn
        .download_behavior
        .effective_for_browser_context(Some("BID-page-sync"));
    assert_eq!(settings.behavior, "allow");

    let metrics_raw = serde_json::to_string(&json!({
        "id": 412,
        "method": "Page.getLayoutMetrics"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&metrics_raw);
    let messages = complete_messages(step);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["id"], json!(412));
    assert!(messages[0]["result"]["layoutViewport"].is_object());

    let print_raw = serde_json::to_string(&json!({
        "id": 413,
        "method": "Page.printToPDF"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&print_raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 413,
            "error": {
                "code": -32000,
                "message": "Page.printToPDF is not supported: renderer layout is disabled."
            }
        })]
    );
}

#[test]
fn command_dispatch_migrates_page_navigation_and_termination_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let navigate_raw = serde_json::to_string(&json!({
        "id": 413,
        "method": "Page.navigate",
        "params": { "url": "data:text/html,navigate" }
    }))
    .unwrap();
    let _navigate_step = conn.start_command_dispatch(&navigate_raw);

    for (method, params) in [
        ("Page.navigateToHistoryEntry", json!({"entryId": 1})),
        ("Page.reload", json!({})),
    ] {
        let raw = serde_json::to_string(&json!({
            "id": 414,
            "method": method,
            "params": params
        }))
        .unwrap();
        let _step = conn.start_command_dispatch(&raw);
    }

    for method in ["Page.crash", "Page.stopLoading", "Page.close"] {
        let raw = serde_json::to_string(&json!({ "id": 414, "method": method })).unwrap();
        let _step = conn.start_command_dispatch(&raw);
    }
}

#[test]
fn command_dispatch_completes_page_create_isolated_world_errors_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let raw = serde_json::to_string(&json!({
        "id": 415,
        "method": "Page.createIsolatedWorld",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility"
        }
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 415,
            "error": {
                "code": -31998,
                "message": "BrowserContextNotLoaded"
            }
        })]
    );
}

#[test]
fn command_dispatch_completes_dom_sync_and_error_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    let enable_raw = serde_json::to_string(&json!({
        "id": 421,
        "method": "DOM.enable"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&enable_raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 421, "result": {} })]
    );

    let get_document_raw = serde_json::to_string(&json!({
        "id": 422,
        "method": "DOM.getDocument"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&get_document_raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 422,
            "error": {
                "code": -31998,
                "message": "BrowserContextNotLoaded"
            }
        })]
    );

    let discard_raw = serde_json::to_string(&json!({
        "id": 423,
        "method": "DOM.discardSearchResults",
        "params": { "searchId": "missing" }
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&discard_raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 423, "result": {} })]
    );

    conn.browser_context = Some(BrowserContext::new("BID-dom-sync".to_owned()));
    let unknown_raw = serde_json::to_string(&json!({
        "id": 424,
        "method": "DOM.noSuchMethod"
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&unknown_raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 424,
            "error": {
                "code": -32601,
                "message": "UnknownMethod"
            }
        })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_page_preload_without_legacy_fallback() {
    let mut ctx = crate::testing::TestContext::new();
    let mut browser_context = BrowserContext::new("BID-page-preload-live".to_owned());
    browser_context.set_active_target_id("TID-page-preload-live".to_owned());
    browser_context.attach_active_session("SID-page-preload-live");
    ctx.conn.browser_context = Some(browser_context);
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<p>preload</p>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let add_raw = serde_json::to_string(&json!({
        "id": 42,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-page-preload-live",
        "params": {
            "source": "globalThis.__dispatchPreload = true;",
            "worldName": "__dispatch_world"
        }
    }))
    .unwrap();
    let add_pending = match ctx.conn.start_command_dispatch(&add_raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Page.addScriptToEvaluateOnNewDocument should update the live page")
        }
    };
    let add_step = ctx
        .conn
        .complete_pending_command_dispatch(add_pending.wait().await)
        .await;
    let (add_messages, _) = ctx.complete_command_task_step_for_test(add_step).await;
    assert_eq!(add_messages.len(), 1);
    assert_eq!(add_messages[0]["id"], json!(42));
    assert_eq!(add_messages[0]["sessionId"], json!("SID-page-preload-live"));
    let identifier = add_messages[0]["result"]["identifier"]
        .as_str()
        .expect("preload identifier")
        .to_owned();
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target
            .owner_state
            .document_start_scripts
            .iter()
            .any(|(stored_id, script)| {
                stored_id == &identifier
                    && script.source == "globalThis.__dispatchPreload = true;"
                    && script.world_name.as_deref() == Some("__dispatch_world")
            }),
        "preload should be persisted on the owner state"
    );

    let remove_raw = serde_json::to_string(&json!({
        "id": 43,
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "sessionId": "SID-page-preload-live",
        "params": { "identifier": identifier }
    }))
    .unwrap();
    let remove_pending = match ctx.conn.start_command_dispatch(&remove_raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Page.removeScriptToEvaluateOnNewDocument should update the live page")
        }
    };
    let remove_step = ctx
        .conn
        .complete_pending_command_dispatch(remove_pending.wait().await)
        .await;
    let (remove_messages, _) = ctx.complete_command_task_step_for_test(remove_step).await;
    assert_eq!(
        remove_messages,
        vec![json!({
            "id": 43,
            "sessionId": "SID-page-preload-live",
            "result": {}
        })]
    );
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target
            .owner_state
            .document_start_scripts
            .is_empty(),
        "preload removal should clear persisted owner state"
    );

    let create_world_raw = serde_json::to_string(&json!({
        "id": 44,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-page-preload-live",
        "params": {
            "frameId": "TID-page-preload-live",
            "worldName": "__dispatch_world"
        }
    }))
    .unwrap();
    let create_world_pending = match ctx.conn.start_command_dispatch(&create_world_raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Page.createIsolatedWorld should use explicit pending page dispatch")
        }
    };
    let (create_world_messages, _) = ctx
        .complete_command_task_step_for_test(CdpCommandTaskStep::Pending(create_world_pending))
        .await;
    assert_eq!(create_world_messages.len(), 1);
    assert_eq!(create_world_messages[0]["id"], json!(44));
    assert_eq!(
        create_world_messages[0]["sessionId"],
        json!("SID-page-preload-live")
    );
    assert!(
        create_world_messages[0]["result"]["executionContextId"]
            .as_i64()
            .is_some(),
        "createIsolatedWorld should return an execution context id"
    );
}

#[test]
fn command_dispatch_completes_target_sync_commands() {
    let mut conn = CdpConnection::new();

    for (id, method) in [
        (50, "Target.createBrowserContext"),
        (51, "Target.getBrowserContexts"),
        (52, "Target.getTargets"),
        (53, "Target.attachToBrowserTarget"),
        (54, "Target.getTargetInfo"),
        (55, "Target.setDiscoverTargets"),
    ] {
        let params = match method {
            "Target.setDiscoverTargets" => json!({"discover": true}),
            _ => json!({}),
        };
        let raw = serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
            .unwrap();
        let step = conn.start_command_dispatch(&raw);
        let messages = complete_messages(step);
        assert!(
            messages.iter().any(
                |message| message.get("id").and_then(Value::as_u64) == Some(id)
                    && message.get("result").is_some()
            ),
            "{method} should emit a successful command response: {messages:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_target_activate_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-target-activate".to_owned());
    browser_context.set_active_target_id("TID-target-activate".to_owned());
    conn.browser_context = Some(browser_context);

    let raw = serde_json::to_string(&json!({
        "id": 56,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-target-activate" }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("Target.activateTarget should use the Target pending dispatcher")
        }
    };
    assert_eq!(
        complete_command_task_for_test(&mut conn, *pending).await,
        vec![json!({ "id": 56, "result": {} })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_target_set_auto_attach_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-target-auto-attach".to_owned());
    browser_context.set_active_target_id("TID-target-auto-attach".to_owned());
    conn.browser_context = Some(browser_context);

    let raw = serde_json::to_string(&json!({
        "id": 57,
        "method": "Target.setAutoAttach",
        "params": {
            "autoAttach": true,
            "waitForDebuggerOnStart": false
        }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("Target.setAutoAttach should use the Target pending dispatcher")
        }
    };
    let messages = complete_command_task_for_test(&mut conn, *pending).await;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["method"], json!("Target.attachedToTarget"));
    assert_eq!(messages[0]["params"]["waitingForDebugger"], json!(false));
    assert!(
        messages[0]["params"]["sessionId"].as_str().is_some(),
        "auto attach should assign a session"
    );
    assert_eq!(messages[1], json!({ "id": 57, "result": {} }));
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_page_bring_to_front_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-page-bring".to_owned());
    browser_context.set_active_target_id("TID-page-bring".to_owned());
    browser_context.attach_active_session("SID-page-bring".to_owned());
    conn.browser_context = Some(browser_context);

    let raw = serde_json::to_string(&json!({
        "id": 5701,
        "method": "Page.bringToFront",
        "sessionId": "SID-page-bring"
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("Page.bringToFront should use the Page pending dispatcher")
        }
    };
    assert_eq!(
        complete_command_task_for_test(&mut conn, *pending).await,
        vec![json!({
            "id": 5701,
            "result": {},
            "sessionId": "SID-page-bring"
        })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_target_detach_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-target-detach".to_owned());
    browser_context.set_active_target_id("TID-target-detach".to_owned());
    browser_context.attach_active_session("SID-target-detach".to_owned());
    conn.browser_context = Some(browser_context);

    let raw = serde_json::to_string(&json!({
        "id": 58,
        "method": "Target.detachFromTarget",
        "params": {
            "targetId": "TID-target-detach",
            "sessionId": "SID-target-detach"
        }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("Target.detachFromTarget should use the Target pending dispatcher")
        }
    };
    let messages = complete_command_task_for_test(&mut conn, *pending).await;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0], json!({ "id": 58, "result": {} }));
    assert_eq!(messages[1]["method"], json!("Target.detachedFromTarget"));
    assert_eq!(
        messages[1]["params"],
        json!({
            "targetId": "TID-target-detach",
            "sessionId": "SID-target-detach"
        })
    );
    assert!(
        !conn
            .browser_context
            .as_ref()
            .expect("browser context should remain loaded")
            .has_active_session()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_target_close_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-target-close".to_owned());
    browser_context.set_active_target_id("TID-target-close".to_owned());
    browser_context.attach_active_session("SID-target-close".to_owned());
    conn.browser_context = Some(browser_context);

    let raw = serde_json::to_string(&json!({
        "id": 59,
        "method": "Target.closeTarget",
        "params": { "targetId": "TID-target-close" }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("Target.closeTarget should use the Target pending dispatcher")
        }
    };
    let messages = complete_command_task_for_test(&mut conn, *pending).await;
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[0],
        json!({ "id": 59, "result": { "success": true } })
    );
    assert_eq!(messages[1]["method"], json!("Inspector.detached"));
    assert_eq!(messages[1]["sessionId"], json!("SID-target-close"));
    assert_eq!(
        messages[1]["params"],
        json!({ "reason": "Render process gone." })
    );
    assert_eq!(messages[2]["method"], json!("Target.detachedFromTarget"));
    assert_eq!(
        messages[2]["params"],
        json!({
            "targetId": "TID-target-close",
            "sessionId": "SID-target-close"
        })
    );
    assert!(
        conn.browser_context
            .as_ref()
            .expect("browser context should remain loaded")
            .active_target_identity()
            .is_none(),
        "closing the active target should leave the active slot empty"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_target_dispose_browser_context_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-target-dispose".to_owned());
    browser_context.set_active_target_id("TID-target-dispose".to_owned());
    browser_context.attach_active_session("SID-target-dispose".to_owned());
    conn.browser_context = Some(browser_context);

    let raw = serde_json::to_string(&json!({
        "id": 60,
        "method": "Target.disposeBrowserContext",
        "params": { "browserContextId": "BID-target-dispose" }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("Target.disposeBrowserContext should use the Target pending dispatcher")
        }
    };
    let messages = complete_command_task_for_test(&mut conn, *pending).await;
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0], json!({ "id": 60, "result": {} }));
    assert_eq!(messages[1]["method"], json!("Inspector.detached"));
    assert_eq!(
        messages[1]["params"],
        json!({ "reason": "Render process gone." })
    );
    assert_eq!(messages[1]["sessionId"], json!("SID-target-dispose"));
    assert_eq!(messages[2]["method"], json!("Target.detachedFromTarget"));
    assert_eq!(
        messages[2]["params"],
        json!({
            "targetId": "TID-target-dispose",
            "sessionId": "SID-target-dispose"
        })
    );
    assert!(
        conn.browser_context.is_none(),
        "disposing the active browser context should remove it"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_target_send_message_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-target-send".to_owned());
    browser_context.set_active_target_id("TID-target-send".to_owned());
    browser_context.attach_active_session("SID-target-send".to_owned());
    conn.browser_context = Some(browser_context);

    let nested = serde_json::to_string(&json!({
        "id": 6101,
        "method": "Target.getBrowserContexts"
    }))
    .unwrap();
    let raw = serde_json::to_string(&json!({
        "id": 61,
        "method": "Target.sendMessageToTarget",
        "params": {
            "message": nested,
            "sessionId": "SID-target-send"
        }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("Target.sendMessageToTarget should use the Target pending dispatcher")
        }
    };
    let messages = complete_command_task_for_test(&mut conn, *pending).await;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0], json!({ "id": 61, "result": {} }));
    assert_eq!(
        messages[1]["method"],
        json!("Target.receivedMessageFromTarget")
    );
    assert_eq!(messages[1]["params"]["sessionId"], "SID-target-send");
    let nested: Value = serde_json::from_str(
        messages[1]["params"]["message"]
            .as_str()
            .expect("nested message should be stringified"),
    )
    .expect("nested message should be valid JSON");
    assert_eq!(nested["id"], 6101);
    assert_eq!(
        nested["result"]["browserContextIds"],
        json!(["BID-target-send"])
    );
}

#[test]
fn command_dispatch_completes_cookie_read_commands() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new("BID-cookie".to_owned()));

    for (id, method) in [
        (60, "Storage.getCookies"),
        (61, "Storage.clearCookies"),
        (62, "Storage.deleteCookies"),
        (63, "Storage.setCookies"),
        (64, "Network.getCookies"),
        (65, "Network.getAllCookies"),
        (66, "Network.clearBrowserCookies"),
        (67, "Network.setCookie"),
        (68, "Network.setCookies"),
    ] {
        let params = match method {
            "Storage.deleteCookies" | "Network.deleteCookies" => json!({"name": "missing"}),
            "Network.setCookie" => json!({
                "name": "network_sid",
                "value": "1",
                "url": "https://example.com/app"
            }),
            "Storage.setCookies" | "Network.setCookies" => json!({
                "cookies": [{
                    "name": "sid",
                    "value": "1",
                    "url": "https://example.com/app"
                }]
            }),
            _ => json!({}),
        };
        let raw = serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
            .unwrap();
        let step = conn.start_command_dispatch(&raw);
        let messages = complete_messages(step);
        assert_eq!(messages.len(), 1, "{method} should emit one response");
        assert_eq!(messages[0]["id"], json!(id));
        assert!(
            messages[0].get("result").is_some(),
            "{method} should complete successfully: {:?}",
            messages[0]
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_storage_set_cookies_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-storage-live".to_owned());
    browser_context.set_active_target_id("TID-storage-live".to_owned());
    conn.browser_context = Some(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>storage</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 67,
        "method": "Storage.setCookies",
        "params": {
            "browserContextId": "BID-storage-live",
            "cookies": [{
                "name": "sid",
                "value": "1",
                "url": "https://example.com/app"
            }]
        }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Storage.setCookies should snapshot the page cookie owner")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    let messages = complete_messages(step);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["id"], json!(67));
    assert_eq!(messages[0]["result"]["success"], json!(true));
    assert_eq!(
        messages[0]["result"]["cookieReports"][0]["status"]["kind"],
        json!("Accepted")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_network_extra_headers_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-network-live".to_owned());
    browser_context.set_active_target_id("TID-network-live".to_owned());
    conn.browser_context = Some(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>network</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 68,
        "method": "Network.setExtraHTTPHeaders",
        "params": { "headers": { "x-dispatch-test": "ok" } }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Network.setExtraHTTPHeaders should update the live page")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 68, "result": {} })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_network_blocked_urls_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-network-blocked-live".to_owned());
    browser_context.set_active_target_id("TID-network-blocked-live".to_owned());
    conn.browser_context = Some(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>network blocked</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 681,
        "method": "Network.setBlockedURLs",
        "params": { "urls": ["*://blocked-dispatch.test/*"] }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Network.setBlockedURLs should update the live page")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 681, "result": {} })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_network_set_cookie_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-network-cookie-live".to_owned());
    browser_context.set_active_target_id("TID-network-cookie-live".to_owned());
    conn.browser_context = Some(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>network cookie</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 69,
        "method": "Network.setCookie",
        "params": {
            "name": "network_sid",
            "value": "1",
            "url": "https://example.com/app"
        }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Network.setCookie should snapshot the page cookie owner")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    let messages = complete_messages(step);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["id"], json!(69));
    assert_eq!(messages[0]["result"]["success"], json!(true));
    assert_eq!(
        messages[0]["result"]["cookieReports"][0]["status"]["kind"],
        json!("Accepted")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_network_emulation_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-network-emulated-live".to_owned());
    browser_context.set_active_target_id("TID-network-emulated-live".to_owned());
    conn.browser_context = Some(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>network emulated</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 682,
        "method": "Network.emulateNetworkConditions",
        "params": {
            "offline": true,
            "latency": 20,
            "downloadThroughput": 1024,
            "uploadThroughput": 512,
            "connectionType": "cellular3g"
        }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Network.emulateNetworkConditions should update the live page")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 682, "result": {} })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_network_user_agent_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-network-ua-live".to_owned());
    browser_context.set_active_target_id("TID-network-ua-live".to_owned());
    conn.browser_context = Some(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>network ua</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 683,
        "method": "Network.setUserAgentOverride",
        "params": { "userAgent": "MoliDispatchNetworkUA/1.0" }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Network.setUserAgentOverride should update the live page loader")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 683, "result": {} })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_emulation_user_agent_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-emulation-ua-live".to_owned());
    browser_context.set_active_target_id("TID-emulation-ua-live".to_owned());
    conn.browser_context = Some(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>emulation ua</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 684,
        "method": "Emulation.setUserAgentOverride",
        "params": { "userAgent": "MoliDispatchEmulationUA/1.0" }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Emulation.setUserAgentOverride should update the live page loader")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 684, "result": {} })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_emulation_user_agent_loader_keeps_active_owner_route_across_completion() {
    let mut conn = CdpConnection::new();
    let active_page = conn
        .load_page_via_runtime_async("data:text/html,<title>active emulation ua</title>")
        .await
        .expect("active page should load");
    let background_page = conn
        .load_page_via_runtime_async("data:text/html,<title>background emulation ua</title>")
        .await
        .expect("background page should load");

    let mut browser_context = BrowserContext::new("BID-emulation-ua-owner-route".to_owned());
    browser_context.set_active_target_id("TID-emulation-ua-active".to_owned());
    browser_context
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(active_page);
    browser_context.stage_background_target(
        "TID-emulation-ua-background".to_owned(),
        None,
        "data:text/html,<title>background emulation ua</title>".to_owned(),
        None,
        None,
    );
    browser_context
        .background_target_mut("TID-emulation-ua-background")
        .expect("background target")
        .replace_loaded_page(Some(background_page));
    conn.browser_context = Some(browser_context);

    let raw = serde_json::to_string(&json!({
        "id": 688,
        "method": "Emulation.setUserAgentOverride",
        "params": { "userAgent": "Moli/Active-Emulation-UA" }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(outcome) => {
            panic!(
                "active Emulation.setUserAgentOverride should update the live page loader: {:?}",
                outcome.into_parts().0
            )
        }
    };

    let background_route = conn
        .target_session_route_for_target_id("TID-emulation-ua-background")
        .expect("background target route");
    let previous_route = conn.replace_none_session_owner_route_override(Some(background_route));
    let messages = complete_command_task_for_test(&mut conn, *pending).await;
    conn.replace_none_session_owner_route_override(previous_route);

    assert_eq!(messages, vec![json!({ "id": 688, "result": {} })]);
    let browser_context = conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        browser_context
            .loaded_page()
            .expect("active page should remain loaded")
            .document_title(),
        "active emulation ua",
        "Emulation user-agent loader completion must stay on the captured active owner"
    );
    assert_eq!(
        browser_context
            .background_target("TID-emulation-ua-background")
            .and_then(|target| target.loaded_page())
            .expect("background page should remain loaded")
            .document_title(),
        "background emulation ua",
        "ambient background owner must not consume the active loader completion"
    );
    assert_eq!(
        conn.navigation_load_inputs_for_session_owner(None)
            .browser_identity_override
            .as_ref()
            .map(|identity| identity.user_agent()),
        Some("Moli/Active-Emulation-UA")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_emulation_timezone_keeps_background_owner_route_across_completion() {
    let mut conn = CdpConnection::new();
    let active_page = conn
        .load_page_via_runtime_async("data:text/html,<title>active timezone</title>")
        .await
        .expect("active page should load");
    let background_page = conn
        .load_page_via_runtime_async("data:text/html,<title>background timezone</title>")
        .await
        .expect("background page should load");

    let mut browser_context = BrowserContext::new("BID-emulation-timezone-owner-route".to_owned());
    browser_context.set_active_target_id("TID-emulation-timezone-active".to_owned());
    browser_context
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(active_page);
    browser_context.stage_background_target(
        "TID-emulation-timezone-background".to_owned(),
        None,
        "data:text/html,<title>background timezone</title>".to_owned(),
        None,
        None,
    );
    browser_context
        .background_target_mut("TID-emulation-timezone-background")
        .expect("background target")
        .replace_loaded_page(Some(background_page));
    conn.browser_context = Some(browser_context);

    let background_route = conn
        .target_session_route_for_target_id("TID-emulation-timezone-background")
        .expect("background target route");
    let raw = serde_json::to_string(&json!({
        "id": 689,
        "method": "Emulation.setTimezoneOverride",
        "params": { "timezoneId": "UTC" }
    }))
    .unwrap();
    let pending = {
        let previous_route =
            conn.replace_none_session_owner_route_override(Some(background_route.clone()));
        let step = conn.start_command_dispatch(&raw);
        conn.replace_none_session_owner_route_override(previous_route);
        match step {
            CdpCommandTaskStep::Pending(pending) => pending,
            CdpCommandTaskStep::Complete(outcome) => {
                panic!(
                    "background Emulation.setTimezoneOverride should update the live background page: {:?}",
                    outcome.into_parts().0
                )
            }
        }
    };

    let active_route = conn
        .target_session_route_for_target_id("TID-emulation-timezone-active")
        .expect("active target route");
    let previous_route = conn.replace_none_session_owner_route_override(Some(active_route));
    let messages = complete_command_task_for_test(&mut conn, *pending).await;
    conn.replace_none_session_owner_route_override(previous_route);

    assert_eq!(messages, vec![json!({ "id": 689, "result": {} })]);
    let browser_context = conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        browser_context
            .loaded_page()
            .expect("active page should remain loaded")
            .document_title(),
        "active timezone",
        "ambient active owner must not consume the background timezone completion"
    );
    assert_eq!(
        browser_context
            .background_target("TID-emulation-timezone-background")
            .and_then(|target| target.loaded_page())
            .expect("background page should remain loaded")
            .document_title(),
        "background timezone",
        "background Emulation completion should preserve the captured owner"
    );
    let previous_route = conn.replace_none_session_owner_route_override(Some(background_route));
    let background_inputs = conn.navigation_load_inputs_for_session_owner(None);
    conn.replace_none_session_owner_route_override(previous_route);
    assert_eq!(background_inputs.timezone_override.as_deref(), Some("UTC"));
    assert!(
        conn.navigation_load_inputs_for_session_owner(None)
            .timezone_override
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_emulation_locale_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-emulation-locale-live".to_owned());
    browser_context.set_active_target_id("TID-emulation-locale-live".to_owned());
    conn.browser_context = Some(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>emulation locale</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 685,
        "method": "Emulation.setLocaleOverride",
        "params": { "locale": "fr-FR" }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Emulation.setLocaleOverride should update the live page")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 685, "result": {} })]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_security_tls_without_legacy_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-security-tls-live".to_owned());
    browser_context.set_active_target_id("TID-security-tls-live".to_owned());
    conn.browser_context = Some(browser_context);
    let page = conn
        .load_page_via_runtime_async("data:text/html,<p>security tls</p>")
        .await
        .expect("page should load");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 686,
        "method": "Security.setIgnoreCertificateErrors",
        "params": { "ignore": true }
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Security.setIgnoreCertificateErrors should update the live page loader")
        }
    };
    let completed = pending.wait().await;
    let step = conn.complete_pending_command_dispatch(completed).await;
    assert_eq!(
        complete_messages(step),
        vec![json!({ "id": 686, "result": {} })]
    );
    assert!(!conn.tls_verify_host());
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_security_tls_keeps_background_owner_route_across_completion() {
    let mut conn = CdpConnection::new();
    let active_page = conn
        .load_page_via_runtime_async("data:text/html,<title>active tls</title>")
        .await
        .expect("active page should load");
    let background_page = conn
        .load_page_via_runtime_async("data:text/html,<title>background tls</title>")
        .await
        .expect("background page should load");

    let mut browser_context = BrowserContext::new("BID-security-owner-route".to_owned());
    browser_context.set_active_target_id("TID-security-active".to_owned());
    browser_context
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(active_page);
    browser_context.stage_background_target(
        "TID-security-background".to_owned(),
        None,
        "data:text/html,<title>background tls</title>".to_owned(),
        None,
        None,
    );
    browser_context
        .background_target_mut("TID-security-background")
        .expect("background target")
        .replace_loaded_page(Some(background_page));
    conn.browser_context = Some(browser_context);

    let background_route = conn
        .target_session_route_for_target_id("TID-security-background")
        .expect("background target route");
    let raw = serde_json::to_string(&json!({
        "id": 687,
        "method": "Security.setIgnoreCertificateErrors",
        "params": { "ignore": true }
    }))
    .unwrap();
    let pending = {
        let previous_route =
            conn.replace_none_session_owner_route_override(Some(background_route.clone()));
        let step = conn.start_command_dispatch(&raw);
        conn.replace_none_session_owner_route_override(previous_route);
        match step {
            CdpCommandTaskStep::Pending(pending) => pending,
            CdpCommandTaskStep::Complete(outcome) => {
                panic!(
                    "background Security.setIgnoreCertificateErrors should update the live background page loader: {:?}",
                    outcome.into_parts().0
                )
            }
        }
    };

    let active_route = conn
        .target_session_route_for_target_id("TID-security-active")
        .expect("active target route");
    let previous_route = conn.replace_none_session_owner_route_override(Some(active_route));
    let messages = complete_command_task_for_test(&mut conn, *pending).await;
    conn.replace_none_session_owner_route_override(previous_route);

    assert_eq!(messages, vec![json!({ "id": 687, "result": {} })]);
    let browser_context = conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        browser_context
            .loaded_page()
            .expect("active page should remain loaded")
            .document_title(),
        "active tls",
        "background Security completion must not finish on the ambient active owner"
    );
    assert_eq!(
        browser_context
            .background_target("TID-security-background")
            .and_then(|target| target.loaded_page())
            .expect("background page should remain loaded")
            .document_title(),
        "background tls",
        "background Security completion should preserve the original background page snapshot"
    );
    assert!(!conn.tls_verify_host());
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_fetch_enable_without_legacy_fallback() {
    let mut ctx = crate::testing::TestContext::new();
    let mut browser_context = BrowserContext::new("BID-fetch-live".to_owned());
    browser_context.set_active_target_id("TID-fetch-live".to_owned());
    ctx.conn.browser_context = Some(browser_context);
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<p>fetch</p>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 69,
        "method": "Fetch.enable",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request" }]
        }
    }))
    .unwrap();
    let pending = match ctx.conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Fetch.enable should update live page interception state")
        }
    };
    let completed = pending.wait().await;
    let step = ctx.conn.complete_pending_command_dispatch(completed).await;
    let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
    assert_eq!(messages, vec![json!({ "id": 69, "result": {} })]);
}

#[tokio::test]
async fn devtools_network_intercept_commands_route_to_fetch_owner() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-bidi-intercept".to_owned());
    browser_context.set_active_target_id("TID-bidi-intercept".to_owned());
    conn.browser_context = Some(browser_context);
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("BIDI-SID")),
        target_id: None,
        browser_context_id: None,
    };

    let (unknown_target_result, _) = conn
        .execute_devtools_command(DevToolsCommand::AddNetworkIntercept(
            DevToolsAddNetworkInterceptCommand {
                context: DevToolsCommandContext {
                    target_id: Some(DevToolsTargetId::from("missing-target")),
                    ..context.clone()
                },
                intercept_id: DevToolsNetworkInterceptId::from("missing-target-intercept"),
                phases: vec![DevToolsNetworkInterceptPhase::BeforeRequestSent],
                url_patterns: Vec::new(),
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        unknown_target_result
            .expect_err("unknown context intercept should not fall back to active target")
            .kind,
        DevToolsErrorKind::NoSuchTarget
    );
    assert!(
        !conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target
            .fetch_owner
            .is_enabled(),
        "unknown context intercept should not mutate active fetch config"
    );

    let (result, _) = conn
        .execute_devtools_command(DevToolsCommand::AddNetworkIntercept(
            DevToolsAddNetworkInterceptCommand {
                context: context.clone(),
                intercept_id: DevToolsNetworkInterceptId::from("intercept-1"),
                phases: vec![
                    DevToolsNetworkInterceptPhase::ResponseStarted,
                    DevToolsNetworkInterceptPhase::BeforeRequestSent,
                    DevToolsNetworkInterceptPhase::AuthRequired,
                ],
                url_patterns: vec![DevToolsNetworkInterceptPattern {
                    url_pattern: "https://example.test/api".to_owned(),
                }],
            },
        ))
        .await
        .into_parts();
    let DevToolsCommandResult::AddNetworkIntercept(result) =
        result.expect("add intercept should succeed")
    else {
        panic!("expected AddNetworkIntercept result");
    };
    assert_eq!(result.intercept_id.as_str(), "intercept-1");
    let fetch_config = conn
        .browser_context
        .as_ref()
        .expect("browser context")
        .active_target
        .fetch_owner
        .config_snapshot();
    assert!(fetch_config.is_enabled());
    assert!(fetch_config.handle_auth_requests());
    assert_eq!(fetch_config.patterns().len(), 2);
    assert_eq!(
        fetch_config.patterns()[0].request_stage,
        FetchRequestStage::Request
    );
    assert_eq!(
        fetch_config.patterns()[1].request_stage,
        FetchRequestStage::Response
    );

    let (result, _) = conn
        .execute_devtools_command(DevToolsCommand::RemoveNetworkIntercept(
            DevToolsRemoveNetworkInterceptCommand {
                context: context.clone(),
                intercept_id: DevToolsNetworkInterceptId::from("intercept-1"),
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        result.expect("remove intercept should succeed"),
        DevToolsCommandResult::Empty
    );
    assert!(
        !conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target
            .fetch_owner
            .is_enabled()
    );

    conn.browser_context
        .as_mut()
        .expect("browser context")
        .background_targets
        .push(BackgroundTarget::with_url(
            "TID-bidi-intercept-background".to_owned(),
            None,
            "https://example.test/background".to_owned(),
        ));
    let (result, _) = conn
        .execute_devtools_command(DevToolsCommand::AddNetworkIntercept(
            DevToolsAddNetworkInterceptCommand {
                context: DevToolsCommandContext {
                    target_id: Some(DevToolsTargetId::from("TID-bidi-intercept-background")),
                    ..context.clone()
                },
                intercept_id: DevToolsNetworkInterceptId::from("intercept-background"),
                phases: vec![DevToolsNetworkInterceptPhase::BeforeRequestSent],
                url_patterns: Vec::new(),
            },
        ))
        .await
        .into_parts();
    result.expect("background add intercept should succeed");
    assert!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .parked_page_session_state("TID-bidi-intercept-background")
            .is_some_and(|state| state.fetch_config.is_enabled()),
        "background target should own the intercept"
    );

    let (result, _) = conn
        .execute_devtools_command(DevToolsCommand::RemoveNetworkIntercept(
            DevToolsRemoveNetworkInterceptCommand {
                context: context.clone(),
                intercept_id: DevToolsNetworkInterceptId::from("intercept-background"),
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        result.expect("target-less remove should find background intercept"),
        DevToolsCommandResult::Empty
    );
    assert!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .parked_page_session_state("TID-bidi-intercept-background")
            .is_none_or(|state| !state.fetch_config.is_enabled()),
        "target-less remove should clear the background intercept"
    );

    let (auth_only_result, _) = conn
        .execute_devtools_command(DevToolsCommand::AddNetworkIntercept(
            DevToolsAddNetworkInterceptCommand {
                context: DevToolsCommandContext {
                    target_id: Some(DevToolsTargetId::from("TID-bidi-intercept")),
                    ..context.clone()
                },
                intercept_id: DevToolsNetworkInterceptId::from("intercept-auth-only"),
                phases: vec![DevToolsNetworkInterceptPhase::AuthRequired],
                url_patterns: vec![DevToolsNetworkInterceptPattern {
                    url_pattern: "https://example.test/protected".to_owned(),
                }],
            },
        ))
        .await
        .into_parts();
    auth_only_result.expect("auth-only add intercept should succeed");
    let auth_url = url::Url::parse("https://example.test/protected").unwrap();
    let preflight = conn
        .prepare_navigation_request_for_session_owner(None, &auth_url, None, false)
        .expect("auth-only intercept should prepare navigation preflight");
    assert!(preflight.document_auth_required);
    assert_eq!(
        preflight
            .document_auth_required_blocked_intercepts
            .iter()
            .map(|intercept| intercept.as_str())
            .collect::<Vec<_>>(),
        vec!["intercept-auth-only"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn command_dispatch_completes_live_fetch_disable_without_legacy_fallback() {
    let mut ctx = crate::testing::TestContext::new();
    let mut browser_context = BrowserContext::new("BID-fetch-disable-live".to_owned());
    browser_context.set_active_target_id("TID-fetch-disable-live".to_owned());
    browser_context
        .active_target
        .fetch_owner
        .configure(None, true, Vec::new());
    ctx.conn.browser_context = Some(browser_context);
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<p>fetch disable</p>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = serde_json::to_string(&json!({
        "id": 6901,
        "method": "Fetch.disable"
    }))
    .unwrap();
    let pending = match ctx.conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(_) => {
            panic!("live Fetch.disable should clear live page interception state")
        }
    };
    let completed = pending.wait().await;
    let step = ctx.conn.complete_pending_command_dispatch(completed).await;
    let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
    assert_eq!(messages, vec![json!({ "id": 6901, "result": {} })]);
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should remain loaded")
            .active_target
            .fetch_owner
            .is_enabled()
    );
}

#[test]
fn command_dispatch_completes_fetch_fulfill_request_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    let raw = serde_json::to_string(&json!({
        "id": 6902,
        "method": "Fetch.fulfillRequest",
        "params": {
            "requestId": "INT-6902",
            "responseCode": 204
        }
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 6902,
            "error": {
                "code": -31998,
                "message": "BrowserContextNotLoaded"
            }
        })]
    );
}

#[test]
fn command_dispatch_completes_fetch_fail_request_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    let raw = serde_json::to_string(&json!({
        "id": 6903,
        "method": "Fetch.failRequest",
        "params": {
            "requestId": "INT-6903",
            "errorReason": "Aborted"
        }
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 6903,
            "error": {
                "code": -31998,
                "message": "BrowserContextNotLoaded"
            }
        })]
    );
}

#[test]
fn command_dispatch_completes_fetch_websocket_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    for (id, method, params) in [
        (
            6904,
            "Fetch.dispatchWebSocketMessage",
            json!({
                "requestId": "missing-websocket",
                "opcode": "text",
                "data": "hello"
            }),
        ),
        (
            6905,
            "Fetch.closeWebSocket",
            json!({
                "requestId": "missing-websocket",
                "code": 1000,
                "reason": "done"
            }),
        ),
    ] {
        let raw = serde_json::to_string(&json!({
            "id": id,
            "method": method,
            "params": params
        }))
        .unwrap();
        let step = conn.start_command_dispatch(&raw);
        assert_eq!(
            complete_messages(step),
            vec![json!({
                "id": id,
                "error": {
                    "code": -32000,
                    "message": "RequestNotFound"
                }
            })]
        );
    }
}

#[test]
fn command_dispatch_completes_fetch_body_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    for (id, method, request_id) in [
        (6906, "Fetch.getResponseBody", "INT-6906"),
        (6907, "Fetch.takeResponseBodyAsStream", "INT-6907"),
    ] {
        let raw = serde_json::to_string(&json!({
            "id": id,
            "method": method,
            "params": { "requestId": request_id }
        }))
        .unwrap();
        let step = conn.start_command_dispatch(&raw);
        assert_eq!(
            complete_messages(step),
            vec![json!({
                "id": id,
                "error": {
                    "code": -31998,
                    "message": "BrowserContextNotLoaded"
                }
            })]
        );
    }
}

#[test]
fn command_dispatch_completes_fetch_continue_commands_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    for (id, method, request_id, extra_params) in [
        (6908, "Fetch.continueRequest", "INT-6908", json!({})),
        (
            6909,
            "Fetch.continueWithAuth",
            "INT-6909",
            json!({
                "authChallengeResponse": { "response": "Default" }
            }),
        ),
        (6910, "Fetch.continueResponse", "INT-6910", json!({})),
    ] {
        let mut params = serde_json::Map::new();
        params.insert("requestId".to_owned(), json!(request_id));
        if let Some(extra) = extra_params.as_object() {
            params.extend(extra.clone());
        }
        let raw = serde_json::to_string(&json!({
            "id": id,
            "method": method,
            "params": params
        }))
        .unwrap();
        let step = conn.start_command_dispatch(&raw);
        assert_eq!(
            complete_messages(step),
            vec![json!({
                "id": id,
                "error": {
                    "code": -31998,
                    "message": "BrowserContextNotLoaded"
                }
            })]
        );
    }
}

#[test]
fn command_dispatch_completes_fetch_unknown_method_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    let raw = serde_json::to_string(&json!({
        "id": 6911,
        "method": "Fetch.noSuchMethod",
        "params": {}
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 6911,
            "error": {
                "code": -32601,
                "message": "UnknownMethod"
            }
        })]
    );
}

#[test]
fn command_dispatch_completes_shim_domains_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    for (id, method) in [
        (70, "Audits.enable"),
        (71, "Audits.disable"),
        (72, "SystemInfo.getInfo"),
        (73, "SystemInfo.getProcessInfo"),
        (74, "WebAuthn.enable"),
        (75, "WebAuthn.disable"),
        (76, "WebMCP.enable"),
        (77, "WebMCP.disable"),
    ] {
        let raw = serde_json::to_string(&json!({ "id": id, "method": method })).unwrap();
        let step = conn.start_command_dispatch(&raw);
        let messages = complete_messages(step);
        assert_eq!(messages.len(), 1, "{method} should emit one response");
        assert_eq!(messages[0]["id"], json!(id));
        assert!(
            messages[0].get("result").is_some(),
            "{method} should complete successfully: {:?}",
            messages[0]
        );
    }
}

#[test]
fn command_dispatch_completes_shim_domain_unknown_methods_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    for (id, method) in [
        (80, "Audits.noSuchMethod"),
        (81, "SystemInfo.noSuchMethod"),
        (82, "WebAuthn.noSuchMethod"),
        (83, "WebMCP.noSuchMethod"),
    ] {
        let raw = serde_json::to_string(&json!({ "id": id, "method": method })).unwrap();
        let step = conn.start_command_dispatch(&raw);
        assert_eq!(
            complete_messages(step),
            vec![json!({
                "id": id,
                "error": {"code": -32601, "message": "UnknownMethod"}
            })],
            "{method} should return UnknownMethod through the command dispatch entry"
        );
    }
}

#[test]
fn command_dispatch_completes_additional_sync_domains_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    for (id, method, expects_result) in [
        (90, "DOMSnapshot.enable", true),
        (91, "DOMSnapshot.disable", true),
        (92, "DOMSnapshot.captureSnapshot", false),
        (93, "Security.enable", true),
        (94, "Security.disable", true),
        (95, "Security.handleCertificateError", true),
        (96, "Security.setOverrideCertificateErrors", true),
        (97, "Network.clearBrowserCache", false),
        (98, "Network.getResponseBody", false),
        (981, "Network.getRequestPostData", false),
        (99, "IO.close", false),
        (100, "IO.read", false),
        (1003, "Accessibility.enable", true),
        (1004, "Accessibility.disable", true),
        (1005, "CSS.enable", true),
        (1006, "CSS.disable", true),
        (1007, "Runtime.disable", true),
        (1008, "Runtime.discardConsoleEntries", true),
    ] {
        let params = match method {
            "IO.close" | "IO.read" => json!({ "handle": "missing-stream" }),
            "Network.getResponseBody" | "Network.getRequestPostData" => {
                json!({ "requestId": "missing-request" })
            }
            _ => json!({}),
        };
        let raw = serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
            .unwrap();
        let step = conn.start_command_dispatch(&raw);
        let messages = complete_messages(step);
        assert_eq!(messages.len(), 1, "{method} should emit one response");
        assert_eq!(messages[0]["id"], json!(id));
        if expects_result {
            assert!(
                messages[0].get("result").is_some(),
                "{method} should complete successfully: {:?}",
                messages[0]
            );
        } else {
            assert!(
                messages[0].get("error").is_some(),
                "{method} should complete with a protocol error: {:?}",
                messages[0]
            );
        }
    }
}

#[test]
fn command_dispatch_completes_input_owner_commands_without_legacy_fallback() {
    let mut conn = crate::testing::real_layout_test_connection();
    conn.browser_context = Some(BrowserContext::new("BID-input".to_owned()));

    let raw = serde_json::to_string(&json!({
        "id": 1001,
        "method": "Input.setInterceptDrags",
        "params": { "enabled": true }
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 1001,
            "error": {
                "code": -32000,
                "message": crate::domains::input::SET_INTERCEPT_DRAGS_UNSUPPORTED_MESSAGE
            }
        })]
    );
    assert!(
        conn.browser_context
            .as_ref()
            .is_some_and(|context| !context.input_intercept_drags_enabled)
    );

    let raw = serde_json::to_string(&json!({
        "id": 1002,
        "method": "Input.dispatchDragEvent",
        "params": {
            "type": "drop",
            "x": 0,
            "y": 0,
            "data": { "items": [], "files": [], "dragOperationsMask": 0 }
        }
    }))
    .unwrap();
    let step = conn.start_command_dispatch(&raw);
    assert_eq!(
        complete_messages(step),
        vec![json!({
            "id": 1002,
            "error": {
                "code": -32000,
                "message": "NoDocumentLoaded"
            }
        })]
    );
}

#[test]
fn command_dispatch_completes_additional_sync_domain_unknown_methods_without_legacy_fallback() {
    let mut conn = CdpConnection::new();

    for (id, method) in [
        (101, "Browser.noSuchMethod"),
        (102, "Target.noSuchMethod"),
        (103, "DOMSnapshot.noSuchMethod"),
        (104, "Security.noSuchMethod"),
        (105, "IO.noSuchMethod"),
        (106, "Network.noSuchMethod"),
        (107, "Emulation.noSuchMethod"),
        (108, "Performance.noSuchMethod"),
        (109, "Input.noSuchMethod"),
        (110, "Accessibility.noSuchMethod"),
        (111, "CSS.noSuchMethod"),
        (112, "Runtime.noSuchMethod"),
        (113, "Page.noSuchMethod"),
    ] {
        let raw = serde_json::to_string(&json!({ "id": id, "method": method })).unwrap();
        let step = conn.start_command_dispatch(&raw);
        assert_eq!(
            complete_messages(step),
            vec![json!({
                "id": id,
                "error": {"code": -32601, "message": "UnknownMethod"}
            })],
            "{method} should return UnknownMethod through the command dispatch entry"
        );
    }
}
