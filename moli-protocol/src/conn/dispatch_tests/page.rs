use super::*;

#[tokio::test]
async fn devtools_script_navigation_exact_cursor_rejects_replaced_page_owner_action() {
    let mut ctx = crate::testing::TestContext::new_with_target_discovery(false);
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let create_outcome = ctx
        .conn
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: true,
            },
        ))
        .await;
    let create_predecessor = create_outcome.renderer_output_predecessor();
    let (create_result, create_events) = create_outcome.into_parts();
    assert!(create_events.is_empty());
    if let Some(predecessor) = create_predecessor {
        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
            .await;
    }
    let AutomationResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id;
    ctx.install_navigation_fixture_for_session_owner("about:blank", None)
        .await;
    let target_context = AutomationContext {
        target_id: Some(target_id.clone()),
        ..context
    };

    let outcome = ctx
        .conn
        .execute_automation_command_with_protocol_events(AutomationCommand::EvaluateScript(
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
    ctx.conn
        .runtime_session_owner_slot_mut_for_owner(&crate::conn::CommandOwnerScope::for_route(
            route.clone(),
        ))
        .expect("created target runtime slot")
        .replace_page_attachment_id_for_test();

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
async fn page_enable_uses_background_initial_document_through_attached_session() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-page-normal-route")),
        target_id: None,
        browser_context_id: None,
    };

    let (first_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
    let AutomationResult::CreateTarget(first_result) =
        first_result.expect("initial target create should succeed")
    else {
        panic!("expected create target result");
    };
    let first_target_id = first_result.target_id;

    let (second_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                context,
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
    let AutomationResult::CreateTarget(second_result) =
        second_result.expect("background target create should succeed")
    else {
        panic!("expected create target result");
    };
    let second_target_id = second_result.target_id;

    let background_route = conn
        .target_session_route_for_target_id(second_target_id.as_str())
        .expect("background target route");
    let background_owner = CommandOwnerScope::for_route(background_route.clone());
    let pending_initial_document = conn
        .start_initial_document_page_ensure_for_owner(&background_owner)
        .expect("background target lifecycle ensure should start")
        .expect("fresh background target should need an initial document page build");
    let completed_initial_document = pending_initial_document
        .wait()
        .await
        .expect("background initial document page build should complete");
    conn.complete_initial_document_page_build_for_owner(completed_initial_document)
        .await
        .expect("background initial document should install on captured owner");

    let background_session =
        attach_page_session_for_test(&mut conn, second_target_id.as_str()).await;
    let raw = serde_json::to_string(&json!({
        "id": 1268,
        "method": "Page.enable",
        "sessionId": background_session
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(_) => {
            panic!("background Page.enable should not start a pending initial document page build")
        }
        CdpCommandTaskStep::Complete(outcome) => outcome,
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
        !browser_context
            .active_page_target()
            .runtime_slot
            .has_loaded_page(),
        "Page.enable must not install a page on the active target"
    );
}

#[tokio::test]
async fn initial_document_page_ensure_completion_uses_captured_owner() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-initial-owner")),
        target_id: None,
        browser_context_id: None,
    };

    let (first_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
    let AutomationResult::CreateTarget(first_result) =
        first_result.expect("initial target create should succeed")
    else {
        panic!("expected create target result");
    };
    let first_target_id = first_result.target_id;

    let (second_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                context,
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
    let AutomationResult::CreateTarget(second_result) =
        second_result.expect("background target create should succeed")
    else {
        panic!("expected create target result");
    };
    let second_target_id = second_result.target_id;

    let background_route = conn
        .target_session_route_for_target_id(second_target_id.as_str())
        .expect("background target route");
    let background_owner = CommandOwnerScope::for_route(background_route);
    let pending = conn
        .start_initial_document_page_ensure_for_owner(&background_owner)
        .expect("background initial document page ensure should start")
        .expect("background initial document page ensure should pend");

    let completed = pending
        .wait()
        .await
        .expect("initial document page build should complete");
    conn.complete_initial_document_page_build_for_owner(completed)
        .await
        .expect("completion should install on captured owner");

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
        !browser_context
            .active_page_target()
            .runtime_slot
            .has_loaded_page(),
        "completion must not install the materialized page on the ambient active target"
    );
}

#[tokio::test]
async fn stale_initial_document_page_build_does_not_overwrite_committed_page() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-lifecycle-stale-initial")),
        target_id: None,
        browser_context_id: None,
    };

    let (create_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            &mut conn,
            AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                context,
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: true,
            }),
        );
    let AutomationResult::CreateTarget(_) =
        create_result.expect("active target create should succeed")
    else {
        panic!("expected create target result");
    };

    let initial_document_owner = CommandOwnerScope::capture(&conn, None);
    let pending = conn
        .start_initial_document_page_ensure_for_owner(&initial_document_owner)
        .expect("target lifecycle ensure should start active initial page")
        .expect("fresh initial target should pend active initial document page build");
    let real_page_url = "data:text/html,<title>real-page</title>";
    let parsed_real_page_url = url::Url::parse(real_page_url).expect("data URL should parse");
    let owner = crate::conn::CommandOwnerScope::capture(&conn, None);
    let real_page = conn
        .load_page_via_runtime_async(real_page_url)
        .await
        .expect("real navigation page should build");
    conn.commit_loaded_navigation_page_for_owner_async(
        &owner,
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
        document_referrer: None,
        timestamp: 0.0,
        session_history_position: None,
        browsing_context_group: None,
    };
    conn.commit_loaded_navigation_target_identity_for_owner(
        &owner,
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
        .active_page_target()
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
async fn automation_command_executes_page_navigation_and_reload() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            },
        ))
        .await
        .into_parts();
    let AutomationResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let url = "data:text/html,bidi-nav".to_owned();
    let target_id = create_result.target_id.clone();

    let (navigate_result, _, _, _) = conn
        .execute_automation_command(AutomationCommand::Navigate(DevToolsNavigateCommand {
            context: AutomationContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            url: url.clone(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }))
        .await
        .into_complete_parts();
    let AutomationResult::Navigate(navigate_result) =
        navigate_result.expect("navigate should succeed")
    else {
        panic!("expected navigate result");
    };
    assert_eq!(navigate_result.url, url);

    let (reload_result, _, _, _) = conn
        .execute_automation_command(AutomationCommand::Reload(DevToolsReloadCommand {
            context: AutomationContext {
                target_id: Some(target_id),
                ..context
            },
            ignore_cache: false,
            script_to_evaluate_on_load: None,
            wait: DevToolsNavigationWait::Load,
        }))
        .await
        .into_complete_parts();
    let AutomationResult::Navigate(reload_result) = reload_result.expect("reload should succeed")
    else {
        panic!("expected reload navigation result");
    };
    assert_eq!(reload_result.url, url);
}

#[tokio::test]
async fn automation_command_executes_page_navigation_without_cdp_response_sidecar() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            },
        ))
        .await
        .into_parts();
    let AutomationResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = AutomationContext {
        target_id: Some(create_result.target_id),
        ..context
    };
    let url = "data:text/html,direct-nav-no-sidecar".to_owned();

    let (navigate_result, _scheduler_events, protocol_events, _renderer_output_predecessor) = conn
        .execute_automation_command_with_protocol_events(AutomationCommand::Navigate(
            DevToolsNavigateCommand {
                context: target_context,
                url: url.clone(),
                referrer: None,
                wait: DevToolsNavigationWait::Load,
            },
        ))
        .await
        .into_complete_parts();
    let AutomationResult::Navigate(navigate_result) =
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
async fn automation_command_executes_child_frame_navigation_without_cdp_response_sidecar() {
    let mut ctx = crate::testing::TestContext::new_with_target_discovery(false);
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let create_outcome = ctx
        .conn
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            },
        ))
        .await;
    let (create_result, _, _, create_predecessor) = create_outcome.into_complete_parts();
    if let Some(predecessor) = create_predecessor {
        ctx.route_direct_command_renderer_predecessor_for_test(predecessor)
            .await;
    }
    let AutomationResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = AutomationContext {
        target_id: Some(create_result.target_id.clone()),
        ..context.clone()
    };

    let parent_url =
        "data:text/html,<iframe srcdoc='<p id=\"child\">initial</p>'></iframe>".to_owned();
    let parent_outcome = ctx
        .conn
        .execute_automation_command(AutomationCommand::Navigate(DevToolsNavigateCommand {
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
        .execute_automation_command(AutomationCommand::GetFrameTree(
            DevToolsGetFrameTreeCommand {
                context: target_context,
                max_depth: None,
            },
        ))
        .await
        .into_parts();
    let AutomationResult::GetFrameTree(frame_tree_result) =
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
        .execute_automation_command_with_protocol_events(AutomationCommand::Navigate(
            DevToolsNavigateCommand {
                context: AutomationContext {
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
    let AutomationResult::Navigate(navigate_result) =
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
async fn automation_command_reports_invalid_navigation_without_cdp_response_parser() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            },
        ))
        .await
        .into_parts();
    let AutomationResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = AutomationContext {
        target_id: Some(create_result.target_id),
        ..context
    };

    let (navigate_result, _scheduler_events, protocol_events) = conn
        .execute_automation_command_with_protocol_events(AutomationCommand::Navigate(
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
async fn automation_command_rejects_page_print_to_pdf_without_placeholder_payload() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            },
        ))
        .await
        .into_parts();
    let AutomationResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id.clone();
    let url = "data:text/html,bidi-print".to_owned();
    let (navigate_result, _, _, _) = conn
        .execute_automation_command(AutomationCommand::Navigate(DevToolsNavigateCommand {
            context: AutomationContext {
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
        .execute_automation_command(AutomationCommand::PrintToPdf(DevToolsPrintToPdfCommand {
            context: AutomationContext {
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
async fn automation_command_executes_context_viewport_override() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            },
        ))
        .await
        .into_parts();
    let AutomationResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id.clone();

    let (viewport_result, _) = conn
        .execute_automation_command(AutomationCommand::SetViewport(DevToolsSetViewportCommand {
            context: AutomationContext {
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
        AutomationResult::Empty
    );

    let owner = CommandOwnerScope::capture(&conn, None);
    let metrics = conn
        .target_session_owner_emulated_device_metrics_for_owner(&owner)
        .expect("active target should hold emulated device metrics");
    assert_eq!(metrics.width, 800);
    assert_eq!(metrics.height, 600);
    assert_eq!(metrics.device_scale_factor, 2.0);

    let (layout_result, _) = conn
        .execute_automation_command(AutomationCommand::GetLayoutMetrics(
            DevToolsGetLayoutMetricsCommand {
                context: AutomationContext {
                    target_id: Some(target_id),
                    ..context
                },
                publish_layout: false,
            },
        ))
        .await
        .into_parts();
    let AutomationResult::LayoutMetrics(layout_metrics) =
        layout_result.expect("target layout metrics should succeed")
    else {
        panic!("expected layout metrics result");
    };
    assert_eq!(layout_metrics.layout_viewport_width, 800);
    assert_eq!(layout_metrics.layout_viewport_height, 600);
    assert_eq!(layout_metrics.device_pixel_ratio, 2.0);
}

#[tokio::test]
async fn automation_command_executes_navigation_history_and_traverse() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = conn
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            },
        ))
        .await
        .into_parts();
    let AutomationResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_id = create_result.target_id;
    let target_context = AutomationContext {
        target_id: Some(target_id.clone()),
        ..context
    };
    let first_url = "data:text/html,<title>A</title>classic-a".to_owned();
    let second_url = "data:text/html,<title>B</title>classic-b".to_owned();

    for url in [&first_url, &second_url] {
        let (navigate_result, _, _, _) = conn
            .execute_automation_command(AutomationCommand::Navigate(DevToolsNavigateCommand {
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
        .execute_automation_command(AutomationCommand::GetNavigationHistory(
            DevToolsGetNavigationHistoryCommand {
                context: target_context.clone(),
            },
        ))
        .await
        .into_parts();
    let AutomationResult::GetNavigationHistory(history) =
        history_result.expect("history should succeed")
    else {
        panic!("expected navigation history result");
    };
    assert!(history.current_index > 0);
    assert_eq!(history.entries[history.current_index].url, second_url);
    let previous = history.entries[history.current_index - 1].clone();

    let (traverse_result, _, _, _) = conn
        .execute_automation_command(AutomationCommand::TraverseHistory(
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
        AutomationResult::TraverseHistory(DevToolsTraverseHistoryResult {
            same_document: false
        })
    ));

    let (history_result, _) = conn
        .execute_automation_command(AutomationCommand::GetNavigationHistory(
            DevToolsGetNavigationHistoryCommand {
                context: target_context.clone(),
            },
        ))
        .await
        .into_parts();
    let AutomationResult::GetNavigationHistory(history) =
        history_result.expect("history after traverse should succeed")
    else {
        panic!("expected navigation history result after traverse");
    };
    assert_eq!(history.entries[history.current_index].id, previous.id);

    let (delta_result, _, _, _) = conn
        .execute_automation_command(AutomationCommand::TraverseHistory(
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
        AutomationResult::TraverseHistory(DevToolsTraverseHistoryResult {
            same_document: false
        })
    ));

    let (history_result, _) = conn
        .execute_automation_command(AutomationCommand::GetNavigationHistory(
            DevToolsGetNavigationHistoryCommand {
                context: target_context.clone(),
            },
        ))
        .await
        .into_parts();
    let AutomationResult::GetNavigationHistory(history) =
        history_result.expect("history after delta traverse should succeed")
    else {
        panic!("expected navigation history result after delta traverse");
    };
    assert_eq!(history.entries[history.current_index].url, second_url);

    let (out_of_range_result, _) = conn
        .execute_automation_command(AutomationCommand::TraverseHistory(
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
