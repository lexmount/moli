use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn devtools_runtime_call_function_popup_activity_drains_from_protocol_neutral_command() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    ctx.conn.set_root_target_discovery_enabled(true);
    let mut browser_context = BrowserContext::new("BID-neutral-popup".to_owned());
    browser_context.set_active_target_id("TID-neutral-popup-opener".to_owned());
    browser_context.attach_active_session("SID-neutral-popup-opener");
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<p>neutral popup opener</p>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("SID-neutral-popup-opener")),
        target_id: Some(DevToolsTargetId::from("TID-neutral-popup-opener")),
        browser_context_id: None,
    };
    let (call_result, scheduler_events, protocol_events, renderer_output_predecessor) = ctx
        .conn
        .execute_automation_command_with_protocol_events(AutomationCommand::CallFunction(
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
    let popup_navigate = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::Navigate(DevToolsNavigateCommand {
            context: AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
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
    let popup_eval_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
            context: AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
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
async fn devtools_runtime_command_uses_background_initial_document_without_resolver_fallback() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
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
                url: "about:blank#background".to_owned(),
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
        conn.navigation_load_inputs_for_owner(&crate::conn::CommandOwnerScope::for_route(
            route.clone(),
        ))
    };
    assert!(
        background_load_inputs
            .document_start_scripts
            .iter()
            .all(|script| {
                !script
                    .source
                    .contains("defineGetter(document, 'hidden', () => true)")
            }),
        "background document surface must be applied through native state"
    );
    assert_eq!(
        background_load_inputs.document_activity,
        moli_page_types::DocumentActivity::new(false, false),
        "background initial document should receive native hidden/unfocused state"
    );

    let (name_result, _) = conn
        .execute_automation_command(AutomationCommand::CallFunction(
            DevToolsCallFunctionCommand {
                context: AutomationContext {
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
            },
        ))
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
        .execute_automation_command(AutomationCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: AutomationContext {
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
        "default background initial document should install background document surfaces"
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
async fn pending_runtime_binding_page_phase_keeps_background_owner_route_across_completion() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let conn = &mut ctx.conn;
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-binding-owner-route")),
        target_id: None,
        browser_context_id: None,
    };

    let (first_result, _) =
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            conn,
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
    ensure_initial_document_for_target_id_for_test(conn, &first_target_id).await;

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
    ensure_initial_document_for_target_id_for_test(conn, &second_target_id).await;

    let first_context = AutomationContext {
        target_id: Some(first_target_id.clone()),
        ..context.clone()
    };
    let second_context = AutomationContext {
        target_id: Some(second_target_id.clone()),
        ..context
    };
    assert_eq!(
        evaluate_string_for_test(
            conn,
            first_context,
            "globalThis.__bindingOwnerProbe = 'active'; globalThis.__bindingOwnerProbe",
            "active binding owner marker",
        )
        .await,
        "active"
    );
    assert_eq!(
        evaluate_string_for_test(
            conn,
            second_context,
            "globalThis.__bindingOwnerProbe = 'background'; globalThis.__bindingOwnerProbe",
            "background binding owner marker",
        )
        .await,
        "background"
    );

    let background_session =
        attach_page_session_for_test(&mut ctx.conn, second_target_id.as_str()).await;
    ctx.process_and_wait_for_response_async(json!({
        "id": 1269,
        "method": "Runtime.addBinding",
        "params": { "name": "backgroundRouteBinding" },
        "sessionId": background_session.clone()
    }))
    .await;
    let messages = ctx.take_all();
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == json!(1269) && message.get("error").is_none()),
        "Runtime.addBinding should complete successfully on the original background owner: {messages:?}"
    );
    assert!(
        ctx.conn
            .target_devtools_session_state_for_session(None)
            .is_none_or(|state| state
                .runtime_bindings
                .iter()
                .all(|binding| binding.name != "backgroundRouteBinding")),
        "binding state apply completion must not write the active owner"
    );
    assert!(
        ctx.conn
            .target_devtools_session_state_for_session(Some(&background_session))
            .is_some_and(|state| state
                .runtime_bindings
                .iter()
                .any(|binding| binding.name == "backgroundRouteBinding")),
        "binding state apply completion should persist on the original background owner"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_target_id(),
        Some(first_target_id.as_str()),
        "background Runtime.addBinding completion must not activate the background target"
    );
}

#[tokio::test]
async fn runtime_enable_uses_background_initial_document_through_attached_session() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-runtime-enable-normal-route")),
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
        "id": 1270,
        "method": "Runtime.enable",
        "sessionId": background_session
    }))
    .unwrap();
    let pending = match conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(outcome) => {
            panic!(
                "background Runtime.enable should replay the existing initial context through V8: {:?}",
                outcome.into_parts().0
            )
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
        !browser_context
            .active_page_target()
            .runtime_slot
            .has_loaded_page(),
        "Runtime.enable must not install a page on the active target"
    );
    assert!(
        conn.target_runtime_session_state_for_session(None)
            .is_none_or(|state| !state.runtime_frontend_enabled),
        "Runtime.enable completion must not enable Runtime on the active owner"
    );
    assert!(
        conn.target_runtime_session_state_for_session(Some(&background_session))
            .is_some_and(|state| state.runtime_frontend_enabled),
        "Runtime.enable completion should enable Runtime on the original background owner"
    );
}

#[tokio::test]
async fn devtools_get_realms_observes_create_target_initial_about_blank_page() {
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
                activate: true,
            },
        ))
        .await
        .into_parts();
    let AutomationResult::CreateTarget(create_result) =
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
        .execute_automation_command(AutomationCommand::GetRealms(DevToolsGetRealmsCommand {
            context: AutomationContext {
                target_id: Some(create_result.target_id.clone()),
                ..context
            },
            realm_type: Some("window".to_owned()),
        }))
        .await
        .into_parts();
    let AutomationResult::Realms(realms_result) =
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
    conn.install_browser_context_fixture_for_test(browser_context);

    let page = conn
        .load_page_via_runtime_async("data:text/html,<title>late-realms</title>")
        .await
        .expect("page should load before DevTools session attaches");
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);
    conn.browser_context
        .as_mut()
        .expect("browser context")
        .attach_active_session("SID-late-realms".to_owned());

    let (realms_result, _) = conn
        .execute_automation_command(AutomationCommand::GetRealms(DevToolsGetRealmsCommand {
            context: AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("SID-late-realms")),
                target_id: Some(DevToolsTargetId::from("TID-late-realms")),
                browser_context_id: None,
            },
            realm_type: Some("window".to_owned()),
        }))
        .await
        .into_parts();
    let AutomationResult::Realms(realms_result) =
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
                activate: true,
            },
        ))
        .await
        .into_parts();
    let AutomationResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };

    let (evaluate_result, _) = conn
        .execute_automation_command(AutomationCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: AutomationContext {
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
async fn devtools_runtime_evaluate_reports_no_document_without_resolver_fallback() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-no-page-runtime".to_owned());
    browser_context.set_active_target_id("TID-no-page-runtime".to_owned());
    browser_context.attach_active_session("SID-no-page-runtime".to_owned());
    browser_context.set_target_url("about:blank".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let (evaluate_result, _) = conn
        .execute_automation_command(AutomationCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
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
async fn devtools_runtime_call_function_channel_does_not_emit_direct_script_message_sidecar() {
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
        ..context
    };
    conn.execute_automation_command(AutomationCommand::Navigate(DevToolsNavigateCommand {
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
        .execute_automation_command(AutomationCommand::CallFunction(
            DevToolsCallFunctionCommand {
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
            },
        ))
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
