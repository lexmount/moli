use super::*;

#[tokio::test]
async fn protocol_neutral_await_promise_keeps_background_owner_route_across_pending_completion() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-owner-route")),
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
    ensure_initial_document_for_target_id_for_test(&mut conn, &first_target_id).await;

    let (second_result, _) = conn
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
    let AutomationResult::CreateTarget(second_result) =
        second_result.expect("background target create should succeed")
    else {
        panic!("expected create target result");
    };
    let second_target_id = second_result.target_id;
    ensure_initial_document_for_target_id_for_test(&mut conn, &second_target_id).await;

    let first_context = AutomationContext {
        target_id: Some(first_target_id.clone()),
        ..context.clone()
    };
    let second_context = AutomationContext {
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
        .start_devtools_runtime_command_dispatch(AutomationCommand::EvaluateScript(
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
    assert!(
        !conn
            .target_devtools_session_state_for_owner(&crate::conn::CommandOwnerScope::for_route(
                active_route.clone()
            ))
            .is_some_and(crate::conn::DevToolsSessionState::has_pending_inspector_awaits),
        "internal id 0 pending await must not be registered on the active owner"
    );

    let background_route = conn
        .target_session_route_for_target_id(second_target_id.as_str())
        .expect("background target route");
    assert!(
        conn.target_devtools_session_state_for_owner(&crate::conn::CommandOwnerScope::for_route(
            background_route.clone()
        ))
        .is_some_and(crate::conn::DevToolsSessionState::has_pending_inspector_awaits),
        "internal id 0 pending await must be registered on the targeted background owner"
    );

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
async fn automation_command_executes_preload_without_cdp_response_sidecar() {
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
        target_id: Some(create_result.target_id.clone()),
        ..context.clone()
    };

    let (add_result, _scheduler_events, add_protocol_events) = conn
        .execute_automation_command_with_protocol_events(AutomationCommand::AddPreloadScript(
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
    let AutomationResult::AddPreloadScript(add_result) =
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
        .execute_automation_command_with_protocol_events(AutomationCommand::RemovePreloadScript(
            DevToolsRemovePreloadScriptCommand {
                context,
                script_id: add_result.script_id,
            },
        ))
        .await
        .into_parts_with_protocol_events();
    assert_eq!(
        remove_result.expect("removePreloadScript should succeed"),
        AutomationResult::Empty
    );
    assert!(
        remove_protocol_events
            .iter()
            .all(|event| !is_command_response_sidecar_event(event)),
        "direct preload remove must not emit a CDP response sidecar: {remove_protocol_events:?}"
    );
}

#[tokio::test]
async fn automation_command_reports_invalid_preload_without_cdp_response_parser() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (add_result, _scheduler_events, protocol_events) = conn
        .execute_automation_command_with_protocol_events(AutomationCommand::AddPreloadScript(
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
async fn automation_command_applies_window_state_to_document_surface() {
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
    let target_context = AutomationContext {
        target_id: Some(target_id),
        ..context
    };

    let (minimize_result, _) = conn
        .execute_automation_command(AutomationCommand::SetWindowState(
            DevToolsSetWindowStateCommand {
                context: target_context.clone(),
                state: DevToolsWindowState::Minimized,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        minimize_result.expect("minimize surface state should succeed"),
        AutomationResult::Empty
    );
    assert!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
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
            "hasFullScreen": false,
            "hasWebkitIsFullScreen": false
        })
    );

    let (fullscreen_result, _) = conn
        .execute_automation_command(AutomationCommand::SetWindowState(
            DevToolsSetWindowStateCommand {
                context: target_context.clone(),
                state: DevToolsWindowState::Fullscreen,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        fullscreen_result.expect("fullscreen surface state should succeed"),
        AutomationResult::Empty
    );
    assert_eq!(
        conn.browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .owner_state
            .window_surface_state,
        TargetWindowSurfaceState::Fullscreen,
        "SetWindowState fullscreen must update the target owner before applying document surfaces",
    );
    assert_eq!(
        evaluate_document_surface_payload(&mut conn, target_context.clone()).await,
        json!({
            "hasFocus": true,
            "hidden": false,
            "visibilityState": "visible",
            "hasFullScreen": false,
            "hasWebkitIsFullScreen": false
        })
    );

    let (normal_result, _) = conn
        .execute_automation_command(AutomationCommand::SetWindowState(
            DevToolsSetWindowStateCommand {
                context: target_context.clone(),
                state: DevToolsWindowState::Normal,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        normal_result.expect("normal surface state should succeed"),
        AutomationResult::Empty
    );
    assert_eq!(
        evaluate_document_surface_payload(&mut conn, target_context).await,
        json!({
            "hasFocus": true,
            "hidden": false,
            "visibilityState": "visible",
            "hasFullScreen": false,
            "hasWebkitIsFullScreen": false
        })
    );
}

#[tokio::test]
async fn automation_command_executes_dom_outer_html_for_document_source() {
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
    let url = "data:text/html,<title>DOMSource</title><main>source-owner</main>".to_owned();
    let _ = ctx
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::Navigate(
            DevToolsNavigateCommand {
                context: AutomationContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                url,
                referrer: None,
                wait: DevToolsNavigationWait::Load,
            },
        ))
        .await;

    let (source_result, _) = ctx
        .conn
        .execute_automation_command(AutomationCommand::GetOuterHtml(
            DevToolsGetOuterHtmlCommand {
                context: AutomationContext {
                    target_id: Some(target_id),
                    ..context
                },
                reference: None,
                include_shadow_dom: false,
            },
        ))
        .await
        .into_parts();
    let AutomationResult::GetOuterHtml(source_result) =
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
async fn automation_command_executes_dom_query_selector_for_document_root() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
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
    let url = "data:text/html,<main id='target'>target</main><section id='root'><a id='child-link' href='child.html'>Child Link</a></section><a id='top-link' href='top.html'>Top Link</a><input id='field' value='initial'><script>document.getElementById('field').value='changed'</script><p class='item'></p><p class='item'></p>".to_owned();
    let navigation = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::Navigate(DevToolsNavigateCommand {
            context: AutomationContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            url,
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await;
    let AutomationResult::Navigate(navigation) = navigation.expect("navigate should succeed")
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
        .execute_automation_command(AutomationCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context: AutomationContext {
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
    let AutomationResult::QuerySelector(single_result) =
        single_result.expect("query selector should succeed")
    else {
        panic!("expected query selector result");
    };
    assert_eq!(single_result.node_ids.len(), 1);
    assert!(!single_result.multiple);
    let target_node_id = single_result.node_ids[0];

    let (attributes_result, _) = conn
        .execute_automation_command(AutomationCommand::GetAttributes(
            DevToolsGetAttributesCommand {
                context: AutomationContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                reference: DevToolsDomNodeReference::FrontendNodeId(target_node_id),
            },
        ))
        .await
        .into_parts();
    let AutomationResult::GetAttributes(attributes_result) =
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
        .execute_automation_command(AutomationCommand::GetText(DevToolsGetTextCommand {
            context: AutomationContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            reference: DevToolsDomNodeReference::FrontendNodeId(target_node_id),
        }))
        .await
        .into_parts();
    let AutomationResult::GetText(text_result) = text_result.expect("get text should succeed")
    else {
        panic!("expected get text result");
    };
    assert_eq!(text_result.text, "target");

    let (property_result, _) = conn
        .execute_automation_command(AutomationCommand::GetProperty(DevToolsGetPropertyCommand {
            context: AutomationContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            reference: DevToolsDomNodeReference::FrontendNodeId(target_node_id),
            name: "id".to_owned(),
        }))
        .await
        .into_parts();
    let AutomationResult::GetProperty(property_result) =
        property_result.expect("get property should succeed")
    else {
        panic!("expected get property result");
    };
    assert_eq!(property_result.value, json!("target"));

    let (field_result, _) = conn
        .execute_automation_command(AutomationCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context: AutomationContext {
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
    let AutomationResult::QuerySelector(field_result) =
        field_result.expect("field query selector should succeed")
    else {
        panic!("expected field query selector result");
    };
    let field_node_id = field_result.node_ids[0];

    let (field_value_result, _) = conn
        .execute_automation_command(AutomationCommand::GetProperty(DevToolsGetPropertyCommand {
            context: AutomationContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            reference: DevToolsDomNodeReference::FrontendNodeId(field_node_id),
            name: "value".to_owned(),
        }))
        .await
        .into_parts();
    let AutomationResult::GetProperty(field_value_result) =
        field_value_result.expect("field value property should succeed")
    else {
        panic!("expected field value property result");
    };
    assert_eq!(field_value_result.value, json!("changed"));

    let (resolve_result, _, resolve_events) = conn
        .execute_automation_command_with_protocol_events(AutomationCommand::ResolveNode(
            DevToolsResolveNodeCommand {
                context: AutomationContext {
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
    let AutomationResult::ResolveNode(resolve_result) =
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
        .execute_automation_command_with_protocol_events(AutomationCommand::DomGeometry(
            DevToolsDomGeometryCommand {
                context: AutomationContext {
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
    let AutomationResult::DomGeometry(content_quads_result) =
        content_quads_result.expect("DOM content quads should succeed")
    else {
        panic!("expected DOM geometry result");
    };
    assert_eq!(content_quads_result.quads.len(), 1);
    assert!(content_quads_result.width.is_none());
    assert!(content_quads_result.height.is_none());

    let (object_geometry_result, _, object_geometry_events) = conn
        .execute_automation_command_with_protocol_events(AutomationCommand::DomObjectReference(
            crate::automation::DevToolsDomObjectReferenceCommand {
                context: AutomationContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                object_id: DevToolsRemoteHandleId::from(resolved_object_id.clone()),
                operation: crate::automation::DevToolsDomObjectReferenceOperation::GetBoxModel,
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
    let AutomationResult::DomGeometry(object_geometry_result) =
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
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
                context: AutomationContext {
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
            }),
        )
        .await;
    let resolved_property = expect_script_value_result(
        resolved_property_result.expect("resolved call function should succeed"),
        "expected resolved call function value",
    );
    assert_eq!(resolved_property.value, json!("target"));

    let conn = &mut ctx.conn;
    let (scroll_result, _) = conn
        .execute_automation_command(AutomationCommand::ScrollIntoViewIfNeeded(
            DevToolsScrollIntoViewIfNeededCommand {
                context: AutomationContext {
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
        AutomationResult::Empty
    );

    let (geometry_result, _) = conn
        .execute_automation_command(AutomationCommand::DomGeometry(DevToolsDomGeometryCommand {
            context: AutomationContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            reference: DevToolsDomNodeReference::FrontendNodeId(target_node_id),
            operation: DevToolsDomGeometryOperation::GetBoxModel,
        }))
        .await
        .into_parts();
    let AutomationResult::DomGeometry(geometry_result) =
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
        .execute_automation_command(AutomationCommand::QuerySelector(
            DevToolsQuerySelectorCommand {
                context: AutomationContext {
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
    let AutomationResult::QuerySelector(multiple_result) =
        multiple_result.expect("query selector all should succeed")
    else {
        panic!("expected query selector result");
    };
    assert_eq!(multiple_result.node_ids.len(), 2);
    assert!(multiple_result.multiple);

    let _ = conn;
    let xpath_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::LocateNodes(
            DevToolsLocateNodesCommand {
                context: AutomationContext {
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
    let AutomationResult::LocateNodes(xpath_result) =
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
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::LocateNodes(
            DevToolsLocateNodesCommand {
                context: AutomationContext {
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
    let AutomationResult::LocateNodes(link_result) =
        link_result.expect("link text locate nodes should succeed")
    else {
        panic!("expected locate nodes result");
    };
    assert_eq!(link_result.node_ids.len(), 1);

    let root_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::QuerySelector(DevToolsQuerySelectorCommand {
                context: AutomationContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                root: None,
                selector: "#root".to_owned(),
                multiple: false,
            }),
        )
        .await;
    let AutomationResult::QuerySelector(root_result) =
        root_result.expect("root query selector should succeed")
    else {
        panic!("expected query selector result");
    };
    let root_node_id = root_result.node_ids[0];

    let sent_before_rooted_query = ctx.sent.len();
    let rooted_query_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::QuerySelector(DevToolsQuerySelectorCommand {
                context: AutomationContext {
                    target_id: Some(target_id.clone()),
                    ..context.clone()
                },
                root: Some(DevToolsDomNodeReference::FrontendNodeId(root_node_id)),
                selector: "a".to_owned(),
                multiple: false,
            }),
        )
        .await;
    assert!(
        ctx.sent[sent_before_rooted_query..]
            .iter()
            .all(|message| message.get("id").is_none()),
        "direct rooted DOM.querySelector must not route child-node sidecars as protocol events"
    );
    let AutomationResult::QuerySelector(rooted_query_result) =
        rooted_query_result.expect("rooted query selector should succeed")
    else {
        panic!("expected query selector result");
    };
    assert_eq!(rooted_query_result.node_ids.len(), 1);
    assert_ne!(rooted_query_result.node_ids[0], link_result.node_ids[0]);

    let root_link_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::LocateNodes(
            DevToolsLocateNodesCommand {
                context: AutomationContext {
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
    let AutomationResult::LocateNodes(root_link_result) =
        root_link_result.expect("rooted link text locate nodes should succeed")
    else {
        panic!("expected locate nodes result");
    };
    assert_eq!(root_link_result.node_ids.len(), 1);
    assert_ne!(root_link_result.node_ids[0], link_result.node_ids[0]);
}

#[tokio::test]
async fn automation_command_executes_input_key_command_without_cdp_sidecar() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
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
        target_id: Some(target_id.clone()),
        ..context.clone()
    };
    // Programmatic focus is part of parser execution. In contrast, `autofocus`
    // is a post-DOMContentLoaded rendering update and can race the first input
    // command sent to this intentionally inactive target.
    let url = "data:text/html,<input id='field'><script>document.getElementById('field').focus()</script>";
    let (navigate_result, scheduler_events, protocol_events) = ctx
        .conn
        .execute_automation_command(AutomationCommand::Navigate(DevToolsNavigateCommand {
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
        .execute_automation_command(AutomationCommand::DispatchKeyEvent(
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
        AutomationResult::Empty
    );
    assert!(
        protocol_events.is_empty(),
        "direct input key dispatch must not emit CDP-shaped sidecar messages: {protocol_events:?}"
    );

    let value_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
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
            }),
        )
        .await;
    let value_result = expect_script_value_result(
        value_result.expect("value evaluation should succeed"),
        "expected script value",
    );
    assert_eq!(value_result.value, json!("Z"));
}

#[tokio::test]
async fn automation_command_executes_context_preload_add_and_remove() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let create_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
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
        target_id: Some(create_result.target_id.clone()),
        ..context.clone()
    };

    let add_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::AddPreloadScript(DevToolsAddPreloadScriptCommand {
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
    let AutomationResult::AddPreloadScript(add_result) =
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

    let navigate_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::Navigate(DevToolsNavigateCommand {
            context: target_context.clone(),
            url: "data:text/html,bidi-preload".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await;
    navigate_result.expect("navigate should run preload script");

    let evaluate_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
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

    let remove_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::RemovePreloadScript(DevToolsRemovePreloadScriptCommand {
            context,
            script_id: add_result.script_id.clone(),
        }),
    )
    .await;
    assert_eq!(
        remove_result.expect("removePreloadScript should succeed"),
        AutomationResult::Empty
    );

    let navigate_after_remove_result =
        execute_direct_automation_command_through_renderer_fence_for_test(
            &mut ctx,
            AutomationCommand::Navigate(DevToolsNavigateCommand {
                context: target_context.clone(),
                url: "data:text/html,bidi-preload-removed".to_owned(),
                referrer: None,
                wait: DevToolsNavigationWait::Load,
            }),
        )
        .await;
    navigate_after_remove_result.expect("navigate after remove should succeed");

    let evaluate_after_remove_result =
        execute_direct_automation_command_through_renderer_fence_for_test(
            &mut ctx,
            AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
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

    let remove_again_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::RemovePreloadScript(DevToolsRemovePreloadScriptCommand {
            context: AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
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
async fn automation_command_executes_script_evaluate_and_call_function() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let create_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
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
        target_id: Some(create_result.target_id.clone()),
        ..context.clone()
    };
    let navigate_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::Navigate(DevToolsNavigateCommand {
            context: target_context.clone(),
            url: "data:text/html,bidi-script".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await;
    navigate_result.expect("navigate should succeed before script evaluation");

    let realms_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::GetRealms(DevToolsGetRealmsCommand {
            context: target_context.clone(),
            realm_type: Some("window".to_owned()),
        }),
    )
    .await;
    let AutomationResult::Realms(realms_result) = realms_result.expect("getRealms should succeed")
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

    let evaluate_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
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

    let realm_context = AutomationContext {
        target_id: None,
        ..context.clone()
    };
    let realm_evaluate_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
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

    let realm_call_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
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

    let owned_evaluate_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
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

    let unknown_release_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::ReleaseObjects(DevToolsReleaseObjectsCommand {
            context: target_context.clone(),
            realm_id: None,
            world_name: None,
            handles: vec!["unknown_handle".into()],
        }),
    )
    .await;
    assert_eq!(
        unknown_release_result.expect("unknown releaseObjects should be ignored"),
        AutomationResult::Empty
    );

    let still_owned_call_result =
        execute_direct_automation_command_through_renderer_fence_for_test(
            &mut ctx,
            AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
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
        execute_direct_automation_command_through_renderer_fence_for_test(
            &mut ctx,
            AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
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

    let release_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::ReleaseObjects(DevToolsReleaseObjectsCommand {
            context: target_context.clone(),
            realm_id: None,
            world_name: None,
            handles: vec!["unknown_handle".into(), handle.clone()],
        }),
    )
    .await;
    assert_eq!(
        release_result.expect("releaseObjects should succeed"),
        AutomationResult::Empty
    );

    let released_call_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
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

    let call_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
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
        .active_page_target_mut()
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
    conn.install_browser_context_fixture_for_test(browser_context);

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

    let messages = complete_command_task_for_test(&mut conn, *pending).await;

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
        .active_page_target_mut()
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
    conn.install_browser_context_fixture_for_test(browser_context);

    let background_session =
        attach_page_session_for_test(&mut conn, "TID-emulation-timezone-background").await;
    let raw = serde_json::to_string(&json!({
        "id": 689,
        "method": "Emulation.setTimezoneOverride",
        "params": { "timezoneId": "UTC" },
        "sessionId": background_session
    }))
    .unwrap();
    let messages = complete_messages(conn.start_command_dispatch(&raw));

    assert_eq!(
        messages,
        vec![json!({
            "id": 689,
            "result": {},
            "sessionId": background_session
        })]
    );
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
    assert_eq!(
        browser_context
            .background_target("TID-emulation-timezone-background")
            .unwrap()
            .devtools_sessions
            .effective_timezone_override()
            .as_deref(),
        Some("UTC")
    );
    assert_eq!(
        browser_context
            .active_page_target()
            .devtools_sessions
            .effective_timezone_override()
            .as_deref(),
        None
    );
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
        .active_page_target_mut()
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
    conn.install_browser_context_fixture_for_test(browser_context);

    let background_session =
        attach_page_session_for_test(&mut conn, "TID-security-background").await;
    let raw = serde_json::to_string(&json!({
        "id": 687,
        "method": "Security.setIgnoreCertificateErrors",
        "params": { "ignore": true },
        "sessionId": background_session
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(outcome) => {
            panic!(
                "background Security.setIgnoreCertificateErrors should update the live background page loader: {:?}",
                outcome.into_parts().0
            )
        }
    };

    let messages = complete_command_task_for_test(&mut conn, *pending).await;

    assert_eq!(
        messages,
        vec![json!({
            "id": 687,
            "result": {},
            "sessionId": background_session
        })]
    );
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
    assert!(
        conn.tls_verify_host(),
        "the background session override must not change the active target"
    );
    assert_eq!(
        conn.browser_context
            .as_ref()
            .and_then(|browser_context| {
                browser_context.background_target("TID-security-background")
            })
            .and_then(|target| target.tls_verify_host_override),
        Some(false),
        "the attached background session should update only its target"
    );
}
