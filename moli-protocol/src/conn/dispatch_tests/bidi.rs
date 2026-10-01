use super::*;

#[tokio::test]
async fn bidi_fetch_control_resolves_background_request_owner() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-fetch-background".to_owned());
    browser_context.set_active_target_id("TID-active".to_owned());
    browser_context.attach_active_session("SID-active".to_owned());
    browser_context.insert_page_target_host(PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "https://example.test/background".to_owned(),
    ));
    conn.install_browser_context_fixture_for_test(browser_context);

    conn.register_pending_fetch_navigation_request_for_owner(
        &crate::conn::CommandOwnerScope::for_session("SID-background"),
        PendingFetchNavigation {
            fetch_request_id: "FETCH-background".to_owned(),
            interception_session_id: Some("bidi-session-1".to_owned()),
            document_navigation_token: None,
            navigation: NavigationDispatchState {
                auxiliary_document_response: None,
                redirect_chain: Vec::new(),
                redirect_headers: None,
                navigate_id: None,
                owner: CommandOwnerScope::for_session("SID-background"),
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
                request_headers: Vec::new().into(),
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
        Some(CdpSessionRoute::PageTarget { target_id, .. }) if target_id == "TID-background"
    ));

    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let outcome = conn
        .execute_automation_command(AutomationCommand::FailInterceptedRequest(
            DevToolsFailInterceptedRequestCommand {
                context,
                request_id: DevToolsRequestId::from("FETCH-background"),
                failure: crate::automation::DevToolsRequestFailure::Failed("Failed".to_owned()),
            },
        ))
        .await;
    let (result, _) = outcome.into_parts();

    assert_eq!(
        result.expect("BiDi failRequest should resolve background owner"),
        AutomationResult::Empty
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
        "resolving a BiDi request id must not activate the background target"
    );
}

#[tokio::test]
async fn bidi_fetch_request_actions_resolve_background_request_owner() {
    assert_bidi_fetch_action_consumes_background_request(
        AutomationCommand::ContinueInterceptedRequest(DevToolsContinueInterceptedRequestCommand {
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
        AutomationCommand::ContinueInterceptedResponse(
            DevToolsContinueInterceptedResponseCommand {
                context: bidi_fetch_command_context(),
                request_id: DevToolsRequestId::from("FETCH-background-continue-response"),
                response_code: None,
                response_headers: None,
                response_phrase: None,
                auth_credentials: None,
            },
        ),
        "FETCH-background-continue-response",
    )
    .await;

    assert_bidi_fetch_action_consumes_background_request(
        AutomationCommand::FulfillInterceptedRequest(DevToolsFulfillInterceptedRequestCommand {
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
async fn bidi_create_target_installs_initial_about_blank_page_without_default_preload() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-lifecycle-create")),
        target_id: None,
        browser_context_id: None,
    };

    let (create_result, _) = conn
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context,
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: true,
            },
        ))
        .await
        .into_parts();
    let AutomationResult::CreateTarget(_) =
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
async fn devtools_call_function_node_shared_id_failure_precedes_handle() {
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
        target_id: Some(create_result.target_id),
        ..context
    };

    let navigate_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::Navigate(DevToolsNavigateCommand {
            context: target_context.clone(),
            url: "data:text/html,<img id='target'>".to_owned(),
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
    let AutomationResult::Script(evaluate_result) =
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

    let call_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
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

    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    let renderer_binding = ctx
        .conn
        .document_bidi_node_binding_for_owner_async(&owner, shared_id.as_str())
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
    let target_id = create_result.target_id.as_str().to_owned();
    let target_context = AutomationContext {
        target_id: Some(create_result.target_id),
        ..context
    };

    let navigate_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::Navigate(DevToolsNavigateCommand {
            context: target_context.clone(),
            url: "data:text/html,<main id='target'>target</main>".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await;
    let AutomationResult::Navigate(navigation) = navigate_result.expect("navigate should succeed")
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

    let query_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::QuerySelector(DevToolsQuerySelectorCommand {
            context: target_context.clone(),
            root: None,
            selector: "#target".to_owned(),
            multiple: false,
        }),
    )
    .await;
    let AutomationResult::QuerySelector(query_result) =
        query_result.expect("query selector should succeed")
    else {
        panic!("expected query selector result");
    };
    let frontend_node_id = query_result.node_ids[0];

    let evaluate_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
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
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
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
            }),
        )
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

    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    let renderer_binding = ctx
        .conn
        .document_bidi_node_binding_for_owner_async(&owner, &child_shared_id)
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
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
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
            }),
        )
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
        crate::automation::webdriver_bidi_node_shared_id_for_backend_node_id(backend_node_id);
    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    ctx.conn
        .register_document_bidi_node_binding_for_owner_async(
            &owner,
            fake_shared_id.as_str(),
            backend_node_id,
        )
        .await
        .expect("renderer fake shared-node binding registration should run");

    let upload_bytes = b"from renderer registry";
    let set_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::SetFileInputFiles(DevToolsSetFileInputFilesCommand {
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
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::EvaluateScript(
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
        crate::automation::webdriver_bidi_node_shared_id_for_backend_node_id(backend_node_id);
    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    ctx.conn
        .register_document_bidi_node_binding_for_owner_async(
            &owner,
            fake_shared_id.as_str(),
            backend_node_id,
        )
        .await
        .expect("renderer fake shared-node binding registration should run");

    let locate_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::LocateNodes(
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
    let AutomationResult::LocateNodes(locate_result) =
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
        crate::automation::webdriver_bidi_node_shared_id_for_backend_node_id(backend_node_id);
    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    ctx.conn
        .register_document_bidi_node_binding_for_owner_async(
            &owner,
            fake_shared_id.as_str(),
            backend_node_id,
        )
        .await
        .expect("renderer fake shared-node binding registration should run");

    let call_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
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
            }),
        )
        .await;
    let remote_value = expect_script_value_result(
        call_result.expect("callFunction should use renderer shared-node binding"),
        "expected callFunction value result",
    );
    assert_eq!(remote_value.value, json!("target:inside"));
}
