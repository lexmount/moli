use super::*;

#[tokio::test]
async fn element_screenshot_reports_unsupported_without_placeholder_payload() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("SID-element-shot")),
        target_id: None,
        browser_context_id: None,
    };

    let create_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
            context: context.clone(),
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: true,
        }),
    )
    .await;
    let AutomationResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = AutomationContext {
        target_id: Some(create_result.target_id.clone()),
        ..context
    };

    let navigate_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::Navigate(DevToolsNavigateCommand {
            context: target_context.clone(),
            url: "data:text/html,<div id='target' style='width:120px;height:80px'>shot</div>"
                .to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await;
    navigate_result.expect("navigate should succeed");

    let evaluate_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
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
    let AutomationResult::Script(evaluate_result) =
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

    let screenshot_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::CaptureScreenshot(DevToolsCaptureScreenshotCommand {
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
    conn.install_browser_context_fixture_for_test(browser_context);

    let (screenshot_result, _) = conn
        .execute_automation_command(AutomationCommand::CaptureScreenshot(
            DevToolsCaptureScreenshotCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
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
async fn automation_command_low_backend_node_refs_miss_without_backend_binding() {
    let mut ctx = crate::testing::TestContext::new_with_target_discovery(false);
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (create_result, _) = ctx
        .conn
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
    let target_context = AutomationContext {
        target_id: Some(target_id),
        ..context
    };
    let _ = ctx
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::Navigate(
            DevToolsNavigateCommand {
                context: target_context.clone(),
                url: "data:text/html,<!doctype html><html><body></body></html>".to_owned(),
                referrer: None,
                wait: DevToolsNavigationWait::Load,
            },
        ))
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
        let page = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .active_page_target_mut()
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

    let (attributes_result, _) = ctx
        .conn
        .execute_automation_command(AutomationCommand::GetAttributes(
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

    let (text_result, _) = ctx
        .conn
        .execute_automation_command(AutomationCommand::GetText(DevToolsGetTextCommand {
            context: target_context.clone(),
            reference: DevToolsDomNodeReference::BackendNodeId(backend_node_id),
        }))
        .await
        .into_parts();
    let error = text_result.expect_err("low backendNodeId get text should miss");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchNode);
    assert_eq!(error.message, "Could not find node with given id");

    let (property_result, _) = ctx
        .conn
        .execute_automation_command(AutomationCommand::GetProperty(DevToolsGetPropertyCommand {
            context: target_context.clone(),
            reference: DevToolsDomNodeReference::BackendNodeId(backend_node_id),
            name: "id".to_owned(),
        }))
        .await
        .into_parts();
    let error = property_result.expect_err("low backendNodeId get property should miss");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchNode);
    assert_eq!(error.message, "Could not find node with given id");

    let (outer_html_result, _) = ctx
        .conn
        .execute_automation_command(AutomationCommand::GetOuterHtml(
            DevToolsGetOuterHtmlCommand {
                context: target_context.clone(),
                reference: Some(DevToolsDomNodeReference::BackendNodeId(backend_node_id)),
                include_shadow_dom: false,
            },
        ))
        .await
        .into_parts();
    let error = outer_html_result.expect_err("low backendNodeId get outerHTML should miss");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchNode);
    assert_eq!(error.message, "Could not find node with given id");

    let (describe_result, _) = ctx
        .conn
        .execute_automation_command(AutomationCommand::DescribeNode(
            DevToolsDescribeNodeCommand {
                context: target_context.clone(),
                reference: Some(DevToolsDomNodeReference::BackendNodeId(backend_node_id)),
                depth: 0,
                pierce: false,
            },
        ))
        .await
        .into_parts();
    let error = describe_result.expect_err("low backendNodeId describe should miss");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchNode);
    assert_eq!(error.message, "Could not find node with given id");

    let (rooted_query_result, _) = ctx
        .conn
        .execute_automation_command(AutomationCommand::QuerySelector(
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

    let page = ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .loaded_page_mut()
        .expect("loaded page");
    let _ = page
        .finish_runtime_protocol_message(mutation_completion)
        .expect("runtime mutation completion should finish");
}
