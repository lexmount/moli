use super::*;

#[tokio::test]
async fn automation_command_executes_target_create_and_get_targets() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let create = conn
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            },
        ))
        .await;
    let (create_result, create_events) = create.into_parts();
    assert!(create_events.is_empty());
    let AutomationResult::CreateTarget(create_result) =
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    assert_eq!(create_result.target_id.as_str(), "TID-1");

    let get_targets = conn
        .execute_automation_command(AutomationCommand::GetTargets(DevToolsGetTargetsCommand {
            context,
            root: Some(create_result.target_id.clone()),
            max_depth: None,
            filter: None,
        }))
        .await;
    let (get_targets_result, get_targets_events) = get_targets.into_parts();
    assert!(get_targets_events.is_empty());
    let AutomationResult::GetTargets(get_targets_result) =
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
async fn devtools_create_target_rejects_unknown_reference_target() {
    let mut conn = CdpConnection::new();
    let mut default_context = BrowserContext::new("BID-default".to_owned());
    default_context.set_active_target_id("TID-default".to_owned());
    conn.insert_browser_context(default_context);

    let (create_result, _) = conn
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                    target_id: Some(DevToolsTargetId::from("TID-missing")),
                    browser_context_id: None,
                },
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: true,
            },
        ))
        .await
        .into_parts();
    let error = create_result.expect_err("unknown reference context should fail");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchTarget);
}

#[tokio::test]
async fn automation_command_preserves_target_create_typed_sidecar() {
    let mut conn = CdpConnection::new();
    conn.set_root_target_discovery_enabled(true);
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (result, scheduler_events, mut protocol_events) = conn
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            },
        ))
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
async fn automation_command_preserves_target_close_detached_typed_sidecar() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-close-sidecar".to_owned());
    browser_context.set_active_target_id("TID-close-sidecar".to_owned());
    browser_context.attach_active_session("SID-close-sidecar".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);

    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: Some(DevToolsTargetId::from("TID-close-sidecar")),
        browser_context_id: None,
    };
    let (result, scheduler_events, protocol_events) = conn
        .execute_automation_command_with_protocol_events(AutomationCommand::CloseTarget(
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
        AutomationResult::CloseTarget(crate::automation::DevToolsCloseTargetResult {
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
async fn automation_command_executes_target_pending_activate_and_close() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    for _ in 0..2 {
        let (result, _) = conn
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
        result.expect("create target should succeed");
    }

    let (activate_result, _) = conn
        .execute_automation_command(AutomationCommand::ActivateTarget(
            DevToolsActivateTargetCommand {
                context: context.clone(),
                target_id: DevToolsTargetId::from("TID-2"),
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        activate_result.expect("activate should succeed"),
        AutomationResult::Empty
    );

    let (close_result, _) = conn
        .execute_automation_command(AutomationCommand::CloseTarget(DevToolsCloseTargetCommand {
            context: context.clone(),
            target_id: DevToolsTargetId::from("TID-2"),
        }))
        .await
        .into_parts();
    let AutomationResult::CloseTarget(close_result) = close_result.expect("close should succeed")
    else {
        panic!("expected close target result");
    };
    assert!(close_result.success);

    let (remaining_result, _) = conn
        .execute_automation_command(AutomationCommand::GetTargets(DevToolsGetTargetsCommand {
            context,
            root: Some(DevToolsTargetId::from("TID-2")),
            max_depth: None,
            filter: None,
        }))
        .await
        .into_parts();
    let AutomationResult::GetTargets(remaining_result) =
        remaining_result.expect("get targets should succeed")
    else {
        panic!("expected get targets result");
    };
    assert!(remaining_result.targets.is_empty());
}

#[tokio::test]
async fn target_lifecycle_ensure_installs_initial_about_blank_page_for_active_target() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-lifecycle-active")),
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
    assert!(
        !conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .has_loaded_page(),
        "immediate target create path should still only stage owner metadata before lifecycle ensure"
    );

    let initial_document_owner = CommandOwnerScope::capture(&conn, None);
    let pending = conn
        .start_initial_document_page_ensure_for_owner(&initial_document_owner)
        .expect("target lifecycle ensure should start active initial page")
        .expect("fresh initial target should pend active initial document page build");
    let joined_pending = conn
        .start_initial_document_page_ensure_for_owner(&initial_document_owner)
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
        conn.start_initial_document_page_ensure_for_owner(&initial_document_owner)
            .expect("second target lifecycle ensure should succeed")
            .is_none(),
        "second ensure should be a no-op"
    );
}

#[tokio::test]
async fn target_lifecycle_ensure_installs_initial_about_blank_page_for_background_target() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-lifecycle-background")),
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
        .expect("target lifecycle ensure should start background initial page")
        .expect("fresh background initial target should pend initial document page build");

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
        !browser_context
            .active_page_target()
            .runtime_slot
            .has_loaded_page(),
        "background ensure must not install the materialized page on the active target"
    );
}

#[tokio::test]
async fn classic_create_target_ensures_fresh_initial_document_without_resolver_fallback() {
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
        .execute_automation_command(AutomationCommand::EvaluateScript(
            DevToolsEvaluateScriptCommand {
                context: AutomationContext {
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
async fn devtools_create_target_can_activate_created_target() {
    let mut ctx = crate::testing::TestContext::new_with_target_discovery(false);
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (first_result, _) = ctx
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
    let AutomationResult::CreateTarget(first_result) =
        first_result.expect("first create target should succeed")
    else {
        panic!("expected first create target result");
    };
    let first_target_id = first_result.target_id.clone();

    let first_navigate = ctx
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::Navigate(
            DevToolsNavigateCommand {
                context: AutomationContext {
                    target_id: Some(first_target_id.clone()),
                    ..context.clone()
                },
                url: "data:text/html,<title>first</title>first".to_owned(),
                referrer: None,
                wait: DevToolsNavigationWait::Load,
            },
        ))
        .await;
    first_navigate.expect("first navigate should succeed");

    let (second_result, _) = ctx
        .conn
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
    let AutomationResult::CreateTarget(second_result) =
        second_result.expect("second create target should succeed")
    else {
        panic!("expected second create target result");
    };
    let second_target_id = second_result.target_id.clone();

    let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        browser_context.active_target_id(),
        Some("TID-2"),
        "activate=true should activate the created target instead of staging it"
    );
    assert!(
        browser_context
            .background_targets()
            .any(|target| target.target_id() == "TID-1"),
        "the previous active target should be preserved as a background target"
    );
    assert!(
        browser_context
            .background_targets()
            .find(|target| target.target_id() == "TID-1")
            .is_some_and(|target| target.has_loaded_page()),
        "the previous target should retain its loaded page after deactivation"
    );

    let (first_realms_before_second_navigation, _) = ctx
        .conn
        .execute_automation_command(AutomationCommand::GetRealms(DevToolsGetRealmsCommand {
            context: AutomationContext {
                target_id: Some(first_target_id.clone()),
                ..context.clone()
            },
            realm_type: Some("window".to_owned()),
        }))
        .await
        .into_parts();
    first_realms_before_second_navigation
        .expect("deactivated first target realms should remain readable before second navigation");
    let second_navigate = ctx
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::Navigate(
            DevToolsNavigateCommand {
                context: AutomationContext {
                    target_id: Some(second_target_id.clone()),
                    ..context.clone()
                },
                url: "data:text/html,<title>second</title>second".to_owned(),
                referrer: None,
                wait: DevToolsNavigationWait::Load,
            },
        ))
        .await;
    second_navigate.expect("second navigate should succeed");

    let (first_realms, _) = ctx
        .conn
        .execute_automation_command(AutomationCommand::GetRealms(DevToolsGetRealmsCommand {
            context: AutomationContext {
                target_id: Some(first_target_id.clone()),
                ..context.clone()
            },
            realm_type: Some("window".to_owned()),
        }))
        .await
        .into_parts();
    let AutomationResult::Realms(first_realms) =
        first_realms.expect("deactivated first target realms should remain readable")
    else {
        panic!("expected realms result");
    };
    assert!(
        first_realms.realms.iter().any(|realm| {
            realm.frame_id.as_ref().map(|frame_id| frame_id.as_str()) == Some("TID-1")
        }),
        "deactivated target should keep its window realm"
    );

    let (all_realms, _) = ctx
        .conn
        .execute_automation_command(AutomationCommand::GetRealms(DevToolsGetRealmsCommand {
            context,
            realm_type: Some("window".to_owned()),
        }))
        .await
        .into_parts();
    let AutomationResult::Realms(all_realms) =
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
        .expect("all-context getRealms should include the deactivated target default realm");
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
async fn automation_command_dispatches_coordinate_mouse_input_for_target() {
    let mut ctx =
        crate::testing::TestContext::from_conn(crate::testing::real_layout_test_connection());
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
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
    let target_id = create_result.target_id.clone();
    let url = "data:text/html,<body style='margin:0'><button style='width:80px;height:80px' onclick='window.__clicked = true'>go</button></body>";
    execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::Navigate(DevToolsNavigateCommand {
            context: AutomationContext {
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

    execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::CaptureScreenshot(DevToolsCaptureScreenshotCommand {
            context: AutomationContext {
                target_id: Some(target_id.clone()),
                ..context.clone()
            },
            format: Some("png".to_owned()),
            quality: None,
            clip: None,
            capture_beyond_viewport: false,
            optimize_for_speed: false,
        }),
    )
    .await
    .expect("publish geometry before coordinate input");

    for (event_type, buttons) in [
        (DevToolsMouseEventType::Pressed, Some(1)),
        (DevToolsMouseEventType::Released, Some(0)),
    ] {
        let result = execute_direct_automation_command_through_renderer_fence_for_test(
            &mut ctx,
            AutomationCommand::DispatchMouseEvent(DevToolsDispatchMouseEventCommand {
                context: AutomationContext {
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
            matches!(result, Ok(AutomationResult::Empty)),
            "coordinate mouse dispatch should complete through the target renderer: {result:?}"
        );
    }

    let clicked_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
            context: AutomationContext {
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
async fn automation_command_executes_default_preload_add_and_remove_without_loaded_target() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (add_result, _) = conn
        .execute_automation_command(AutomationCommand::AddPreloadScript(
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
    let AutomationResult::AddPreloadScript(add_result) =
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
        .execute_automation_command(AutomationCommand::RemovePreloadScript(
            DevToolsRemovePreloadScriptCommand {
                context: context.clone(),
                script_id: add_result.script_id.clone(),
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        remove_result.expect("default removePreloadScript should succeed"),
        AutomationResult::Empty
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
        .execute_automation_command(AutomationCommand::RemovePreloadScript(
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

#[tokio::test(flavor = "multi_thread")]
async fn pending_emulation_viewport_keeps_original_page_when_active_target_changes() {
    let mut ctx = crate::testing::TestContext::new();
    let mut browser_context = BrowserContext::new("BID-emulation-viewport-owner".to_owned());
    browser_context.set_active_target_id("TID-emulation-viewport-original".to_owned());
    browser_context.stage_background_target(
        "TID-emulation-viewport-replacement".to_owned(),
        None,
        "data:text/html,<title>replacement active page</title>".to_owned(),
        None,
        None,
    );
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>original viewport owner</title>",
        None,
    )
    .await;
    ctx.conn
        .select_page_target_for_connection_async("TID-emulation-viewport-replacement")
        .await
        .unwrap();
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>replacement active page</title>",
        None,
    )
    .await;
    ctx.conn
        .select_page_target_for_connection_async("TID-emulation-viewport-original")
        .await
        .unwrap();
    ctx.take_all();

    let raw = serde_json::to_string(&json!({
        "id": 690,
        "method": "Emulation.setDeviceMetricsOverride",
        "params": {
            "width": 640,
            "height": 360,
            "deviceScaleFactor": 2,
            "mobile": false
        }
    }))
    .unwrap();
    let pending = match ctx.conn.start_command_dispatch(&raw) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(outcome) => {
            panic!(
                "live Emulation.setDeviceMetricsOverride should update the original Page: {:?}",
                outcome.into_parts().0
            )
        }
    };

    assert!(
        ctx.conn
            .select_page_target_for_connection_async("TID-emulation-viewport-replacement",)
            .await
            .expect("target activation should succeed")
            .is_some()
    );
    let (messages, _) = ctx
        .complete_command_task_step_for_test(CdpCommandTaskStep::Pending(pending))
        .await;

    assert_eq!(messages, vec![json!({ "id": 690, "result": {} })]);
    let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        browser_context.active_target_id(),
        Some("TID-emulation-viewport-replacement")
    );
    assert_eq!(
        browser_context
            .loaded_page()
            .expect("replacement page should remain loaded")
            .document_title(),
        "replacement active page",
        "the original Page completion must not overwrite the newly active Page state"
    );
    assert_eq!(
        browser_context
            .background_target("TID-emulation-viewport-original")
            .and_then(|target| target.loaded_page())
            .expect("original page should remain loaded in its stable target")
            .document_title(),
        "original viewport owner"
    );
}
