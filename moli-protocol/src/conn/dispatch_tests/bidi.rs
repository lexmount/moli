use super::*;

#[tokio::test]
async fn bidi_node_arguments_release_temporary_handles_on_success_and_error() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let (context, shared_id, _) =
        materialize_bidi_target_node_for_test(&mut ctx, "<div id='target'>Hello</div>", "#target")
            .await;
    let owner = ctx.with_fixture(|conn| {
        conn.command_owner_scope_for_devtools_context(&context)
            .unwrap()
    });
    let roots = |ctx: &mut crate::testing::TestContext| {
        ctx.with_fixture(|conn| {
            conn.target_devtools_session_state_for_owner(&owner)
                .unwrap()
                .runtime_remote_object_ids
                .clone()
        })
    };
    let before = roots(&mut ctx);
    for (function, arguments, expected_error) in [
        (
            "function(node) { return node.textContent; }",
            vec![json!({"sharedId": shared_id.as_str()})],
            None,
        ),
        (
            "function(node) { throw null; }",
            vec![json!({"sharedId": shared_id.as_str()})],
            None,
        ),
        (
            "function(node, other) { return node; }",
            vec![
                json!({"sharedId": shared_id.as_str()}),
                json!({"sharedId": "moli:bidi-renderer-node:4294967295"}),
            ],
            Some(DevToolsErrorKind::NoSuchNode),
        ),
        (
            "function(node, other) { return node; }",
            vec![
                json!({"sharedId": shared_id.as_str()}),
                json!({"handle": "not-exported"}),
            ],
            Some(DevToolsErrorKind::NoSuchHandle),
        ),
    ] {
        let result = ctx
            .execute_automation_command_through_renderer_fence_for_test(
                AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
                    context: context.clone(),
                    realm_id: None,
                    world_name: None,
                    object_id: None,
                    this_parameter: None,
                    function_declaration: function.to_owned(),
                    arguments,
                    await_promise: false,
                    user_gesture: false,
                    webdriver_bidi_file_prompt_handler: None,
                    result_ownership: DevToolsResultOwnership::None,
                    object_group: None,
                    preserve_remote_metadata: false,
                    serialization_options: None,
                    materialize_bidi_script_result: true,
                }),
            )
            .await;
        if let Some(kind) = expected_error {
            assert_eq!(result.unwrap_err().kind, kind, "{function}");
        } else if function == "function(node) { throw null; }" {
            let AutomationResult::Script(result) = result.unwrap() else {
                panic!("expected the script exception result");
            };
            assert!(matches!(*result, DevToolsScriptResult::Exception(_)));
        } else {
            assert_eq!(
                expect_script_value_result(result.unwrap(), function).value,
                "Hello"
            );
        }
        ctx.wait_for_direct_command_work_completion_for_test("temporary node argument cleanup")
            .await;
        assert_eq!(
            roots(&mut ctx),
            before,
            "temporary argument handles escaped {function}"
        );
    }
    // Native interoperability lookup must also be independent of inspector
    // roots, including repeated first-use checks by Classic commands.
    for _ in 0..3 {
        let reference = ctx
            .state
            .with_state(|state| {
                state.webdriver_node_reference(
                    &context,
                    shared_id.as_str(),
                    crate::automation::WebDriverNodeKind::Element,
                )
            })
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            reference.kind,
            crate::automation::WebDriverNodeKind::Element
        );
    }
    assert_eq!(roots(&mut ctx), before);
    // Releasing the argument group must not disown the separately returned
    // client-owned root, even when both handles point at the same DOM node.
    let returned = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
                context: context.clone(),
                realm_id: None,
                world_name: None,
                object_id: None,
                this_parameter: None,
                function_declaration: "function(node) { return node; }".to_owned(),
                arguments: vec![json!({"sharedId": shared_id.as_str()})],
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::Root,
                object_group: None,
                preserve_remote_metadata: false,
                serialization_options: None,
                materialize_bidi_script_result: true,
            }),
        )
        .await
        .unwrap();
    let remote = expect_script_value_result(returned, "returning the node should succeed");
    let handle = remote.handle.unwrap();
    let mut realm_context = context.clone();
    realm_context.target_id = None;
    let reused = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
                context: realm_context,
                realm_id: Some(remote.realm.unwrap()),
                world_name: None,
                object_id: None,
                this_parameter: Some(json!({"handle": handle.as_str()})),
                function_declaration:
                    "function(node) { return this === node && node.textContent; }".to_owned(),
                arguments: vec![json!({"sharedId": shared_id.as_str()})],
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::None,
                object_group: None,
                preserve_remote_metadata: false,
                serialization_options: None,
                materialize_bidi_script_result: true,
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        expect_script_value_result(reused, "client root must remain live").value,
        "Hello"
    );
    ctx.execute_automation_command_through_renderer_fence_for_test(
        AutomationCommand::ReleaseObjects(DevToolsReleaseObjectsCommand {
            context,
            realm_id: None,
            world_name: None,
            handles: vec![handle],
        }),
    )
    .await
    .unwrap();
    ctx.wait_for_direct_command_work_completion_for_test("temporary node argument cleanup")
        .await;
    assert_eq!(roots(&mut ctx), before);
}

#[tokio::test]
async fn bidi_fetch_control_resolves_background_request_owner() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let mut browser_context = BrowserContext::new("BID-fetch-background".to_owned());
    browser_context.set_active_target_id("TID-active".to_owned());
    browser_context.attach_active_session("SID-active".to_owned());
    browser_context.insert_page_target_host(PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "https://example.test/background".to_owned(),
    ));
    ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser_context));

    ctx.with_fixture(|conn| {
        conn.register_pending_fetch_navigation_request_for_owner(
            &crate::conn::CommandOwnerScope::for_session("SID-background"),
            PendingFetchNavigation {
                reserved_service_worker_client: None,
                interception: None,
                remaining_request_stages: Vec::new(),
                redirected_request_id: None,
                auth_retry_budget: Default::default(),
                fetch_request_id: "FETCH-background".to_owned(),
                interception_session_id: Some("bidi-session-1".to_owned()),
                document_navigation_token: None,
                navigation: NavigationDispatchState {
                    web_mcp_invocation: None,
                    navigation_initiator: None,
                    initial_document_environment: None,
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
                    announced_redirects: 0,
                    requested_url: url::Url::parse("https://example.test/background").unwrap(),
                    request_method: "GET".to_owned(),
                    request_body: None,
                    request_body_bytes: None,
                    request_headers: Vec::new().into(),
                    request_load_policy:
                        crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
                    timestamp: 0.0,
                    source_document_security: Default::default(),
                },
                request_cookie_report: None,
                intercept_response: false,
                response_stage_url_match_policy: ResponseStageUrlMatchPolicy::AlreadyMatched,
                auth_required_blocked_intercepts: Vec::new(),
            },
        )
        .expect("background pending navigation should register")
    });

    ctx.with_fixture(|conn| {
        assert!(matches!(
            conn.pending_fetch_request_session_route("FETCH-background"),
            Some(CdpSessionRoute::PageTarget { target_id, .. }) if target_id == "TID-background"
        ))
    });

    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let outcome = ctx
        .execute_owned_automation_command_for_test(AutomationCommand::FailInterceptedRequest(
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
    ctx.with_fixture(|conn| {
        assert!(
            conn.pending_fetch_request_session_route("FETCH-background")
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
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-lifecycle-create")),
        target_id: None,
        browser_context_id: None,
    };

    let create_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                context,
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: true,
            }),
        )
        .await;
    let AutomationResult::CreateTarget(_) =
        create_result.expect("active target create should succeed")
    else {
        panic!("expected create target result");
    };
    ctx.with_fixture(|conn| { assert!(
        conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .has_loaded_page(),
        "BiDi Target.createTarget should install initial page even without default preload scripts"
    ) });
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

    ctx.with_fixture(|conn| conn.clear_runtime_remote_object_tracking_for_session_owner(None));

    let owner = ctx.with_fixture(|conn| CommandOwnerScope::capture(conn, None));
    let renderer_binding = {
        let prepared = ctx
            .state
            .with_state(|state| {
                state.prepare_runtime_bidi_binding_query(&owner, (shared_id.as_str()).to_owned())
            })
            .expect("live query owner")
            .expect("Native query admission");
        ctx.finish_fixture_query(prepared).await
    }
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

    ctx.with_fixture(|conn| conn.clear_runtime_remote_object_tracking_for_session_owner(None));

    let owner = ctx.with_fixture(|conn| CommandOwnerScope::capture(conn, None));
    let renderer_binding = {
        let prepared = ctx
            .state
            .with_state(|state| {
                state.prepare_runtime_bidi_binding_query(&owner, child_shared_id.to_owned())
            })
            .expect("live query owner")
            .expect("Native query admission");
        ctx.finish_fixture_query(prepared).await
    }
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
    let owner = ctx.with_fixture(|conn| CommandOwnerScope::capture(conn, None));
    {
        let prepared = ctx
            .state
            .with_state(|state| {
                state.prepare_runtime_bidi_binding_registration(
                    &owner,
                    (fake_shared_id.as_str()).to_owned(),
                    backend_node_id,
                )
            })
            .expect("live query owner")
            .expect("Native query admission");
        ctx.finish_fixture_query(prepared).await
    }
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
    let owner = ctx.with_fixture(|conn| CommandOwnerScope::capture(conn, None));
    {
        let prepared = ctx
            .state
            .with_state(|state| {
                state.prepare_runtime_bidi_binding_registration(
                    &owner,
                    (fake_shared_id.as_str()).to_owned(),
                    backend_node_id,
                )
            })
            .expect("live query owner")
            .expect("Native query admission");
        ctx.finish_fixture_query(prepared).await
    }
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
    let owner = ctx.with_fixture(|conn| CommandOwnerScope::capture(conn, None));
    {
        let prepared = ctx
            .state
            .with_state(|state| {
                state.prepare_runtime_bidi_binding_registration(
                    &owner,
                    (fake_shared_id.as_str()).to_owned(),
                    backend_node_id,
                )
            })
            .expect("live query owner")
            .expect("Native query admission");
        ctx.finish_fixture_query(prepared).await
    }
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

#[tokio::test]
async fn bidi_exports_reject_raw_cdp_ids_and_retire_after_disown_and_navigation() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let (context, _, _) =
        materialize_bidi_target_node_for_test(&mut ctx, "<div id='target'>Hello</div>", "#target")
            .await;
    let remote = expect_script_value_result(
        ctx.execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
                context: context.clone(),
                realm_id: None,
                world_name: None,
                expression: "({marker: 'owned'})".into(),
                await_promise: false,
                user_gesture: false,
                webdriver_bidi_file_prompt_handler: None,
                result_ownership: DevToolsResultOwnership::Root,
                preserve_remote_metadata: false,
                materialize_bidi_script_result: true,
                serialization_options: None,
            }),
        )
        .await
        .unwrap(),
        "export owned result",
    );
    let handle = remote.handle.unwrap();
    let realm = remote.realm.unwrap();
    let owner = ctx.with_fixture(|conn| {
        conn.command_owner_scope_for_devtools_context(&context)
            .unwrap()
    });
    let native = ctx.with_fixture(|conn| {
        conn.resolve_bidi_handle(&owner, handle.as_str(), Some(realm.as_str()))
            .unwrap()
    });
    assert_ne!(handle.as_str(), native);
    let call = |handle: &str| {
        AutomationCommand::CallFunction(DevToolsCallFunctionCommand {
            context: AutomationContext {
                target_id: None,
                ..context.clone()
            },
            realm_id: Some(realm.clone()),
            world_name: None,
            object_id: None,
            this_parameter: Some(json!({"handle": handle})),
            function_declaration: "function() { return this.marker; }".into(),
            arguments: vec![],
            await_promise: false,
            user_gesture: false,
            webdriver_bidi_file_prompt_handler: None,
            result_ownership: DevToolsResultOwnership::None,
            object_group: None,
            preserve_remote_metadata: false,
            serialization_options: None,
            materialize_bidi_script_result: true,
        })
    };
    assert_eq!(
        ctx.execute_automation_command_through_renderer_fence_for_test(call(&native))
            .await
            .unwrap_err()
            .kind,
        DevToolsErrorKind::NoSuchHandle
    );
    let value = ctx
        .execute_automation_command_through_renderer_fence_for_test(call(handle.as_str()))
        .await
        .unwrap();
    assert_eq!(
        expect_script_value_result(value, "resolve explicit export").value,
        "owned"
    );
    ctx.execute_automation_command_through_renderer_fence_for_test(
        AutomationCommand::ReleaseObjects(DevToolsReleaseObjectsCommand {
            context: context.clone(),
            realm_id: Some(realm.clone()),
            world_name: None,
            handles: vec![handle.clone()],
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        ctx.execute_automation_command_through_renderer_fence_for_test(call(handle.as_str()))
            .await
            .unwrap_err()
            .kind,
        DevToolsErrorKind::NoSuchHandle
    );
    let stale = ctx.with_fixture(|conn| {
        conn.export_bidi_handle(&owner, native.clone(), realm.as_str())
            .unwrap()
    });
    ctx.install_navigation_fixture_for_session_owner("about:blank", owner.session_id())
        .await;
    assert!(
        ctx.with_fixture(|conn| conn.resolve_bidi_handle(&owner, &stale, None))
            .is_none()
    );
}
