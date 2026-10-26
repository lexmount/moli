use super::*;

#[tokio::test]
async fn automation_command_executes_target_create_and_get_targets() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let create = ctx
        .execute_owned_automation_command_for_test(AutomationCommand::CreateTarget(
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

    let get_targets = ctx
        .execute_owned_automation_command_for_test(AutomationCommand::GetTargets(
            DevToolsGetTargetsCommand {
                context,
                root: Some(create_result.target_id.clone()),
                max_depth: None,
                filter: None,
            },
        ))
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
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let mut default_context = BrowserContext::new("BID-default".to_owned());
    default_context.set_active_target_id("TID-default".to_owned());
    ctx.with_fixture(|conn| conn.insert_browser_context(default_context));

    let create_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                    target_id: Some(DevToolsTargetId::from("TID-missing")),
                    browser_context_id: None,
                },
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: true,
            }),
        )
        .await;
    let error = create_result.expect_err("unknown reference context should fail");
    assert_eq!(error.kind, DevToolsErrorKind::NoSuchTarget);
}

#[tokio::test]
async fn automation_command_preserves_target_create_typed_sidecar() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    ctx.with_fixture(|conn| conn.set_root_target_discovery_enabled(true));
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (result, scheduler_events, mut protocol_events) = ctx
        .execute_owned_automation_command_for_test(AutomationCommand::CreateTarget(
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
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let mut browser_context = BrowserContext::new("BID-close-sidecar".to_owned());
    browser_context.set_active_target_id("TID-close-sidecar".to_owned());
    browser_context.attach_active_session("SID-close-sidecar".to_owned());
    ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser_context));

    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: Some(DevToolsTargetId::from("TID-close-sidecar")),
        browser_context_id: None,
    };
    let (result, scheduler_events, protocol_events) = ctx
        .execute_owned_automation_command_for_test(AutomationCommand::CloseTarget(
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
        if message.as_json().expect("scalar protocol fixture")["method"]
            != json!("Target.detachedFromTarget")
        {
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
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    for _ in 0..2 {
        let result = ctx
            .execute_automation_command_through_renderer_fence_for_test(
                AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                    context: context.clone(),
                    url: "about:blank".to_owned(),
                    browser_context_id: None,
                    activate: false,
                }),
            )
            .await;
        result.expect("create target should succeed");
    }

    let activate_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::ActivateTarget(DevToolsActivateTargetCommand {
                context: context.clone(),
                target_id: DevToolsTargetId::from("TID-2"),
            }),
        )
        .await;
    assert_eq!(
        activate_result.expect("activate should succeed"),
        AutomationResult::Empty
    );

    let close_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::CloseTarget(
            DevToolsCloseTargetCommand {
                context: context.clone(),
                target_id: DevToolsTargetId::from("TID-2"),
            },
        ))
        .await;
    let AutomationResult::CloseTarget(close_result) = close_result.expect("close should succeed")
    else {
        panic!("expected close target result");
    };
    assert!(close_result.success);

    let remaining_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::GetTargets(
            DevToolsGetTargetsCommand {
                context,
                root: Some(DevToolsTargetId::from("TID-2")),
                max_depth: None,
                filter: None,
            },
        ))
        .await;
    let AutomationResult::GetTargets(remaining_result) =
        remaining_result.expect("get targets should succeed")
    else {
        panic!("expected get targets result");
    };
    assert!(remaining_result.targets.is_empty());
}

#[tokio::test]
async fn target_lifecycle_ensure_installs_initial_about_blank_page_for_active_target() {
    let ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-lifecycle-active")),
        target_id: None,
        browser_context_id: None,
    };

    let (create_result, _) = ctx.with_fixture(|conn| {
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            conn,
            AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                context,
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: true,
            }),
        )
    });
    let AutomationResult::CreateTarget(_) =
        create_result.expect("active target create should succeed")
    else {
        panic!("expected create target result");
    };
    ctx.with_fixture(|conn| { assert!(
        !conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .has_loaded_page(),
        "immediate target create path should still only stage owner metadata before lifecycle ensure"
    ) });

    let initial_document_owner = ctx.with_fixture(|conn| CommandOwnerScope::capture(conn, None));
    let pending = ctx.with_fixture(|conn| {
        conn.start_initial_document_page_ensure_for_owner(&initial_document_owner)
            .expect("target lifecycle ensure should start active initial page")
            .expect("fresh initial target should pend active initial document page build")
    });
    let joined_pending = ctx.with_fixture(|conn| {
        conn.start_initial_document_page_ensure_for_owner(&initial_document_owner)
            .expect("second ensure should join the active initial page build")
            .expect("second ensure should wait for the active initial page build")
    });
    let completed = pending
        .wait()
        .await
        .expect("active initial document page build should complete");
    (completed)
        .install_using(&mut crate::conn::ProtocolWorkflowDriver::new(ctx.handle()))
        .await
        .map(|_| ())
        .expect("active initial page should install");
    let joined_completed = joined_pending
        .wait()
        .await
        .expect("joined initial document page build should observe completion");
    (joined_completed)
        .install_using(&mut crate::conn::ProtocolWorkflowDriver::new(ctx.handle()))
        .await
        .map(|_| ())
        .expect("joined initial page completion should be a no-op");
    ctx.with_fixture(|conn| {
        assert!(
            conn.browser_context
                .as_ref()
                .expect("browser context")
                .has_loaded_page(),
            "ensure should install loaded page on active target"
        )
    });

    ctx.with_fixture(|conn| {
        assert!(
            conn.start_initial_document_page_ensure_for_owner(&initial_document_owner)
                .expect("second target lifecycle ensure should succeed")
                .is_none(),
            "second ensure should be a no-op"
        )
    });
}

#[tokio::test]
async fn target_lifecycle_ensure_installs_initial_about_blank_page_for_background_target() {
    let ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-lifecycle-background")),
        target_id: None,
        browser_context_id: None,
    };

    let (first_result, _) = ctx.with_fixture(|conn| {
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            conn,
            AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        )
    });
    let AutomationResult::CreateTarget(first_result) =
        first_result.expect("initial target create should succeed")
    else {
        panic!("expected create target result");
    };
    let first_target_id = first_result.target_id;

    let (second_result, _) = ctx.with_fixture(|conn| {
        crate::domains::target::execute_immediate_devtools_target_command_with_protocol_events(
            conn,
            AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                context,
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        )
    });
    let AutomationResult::CreateTarget(second_result) =
        second_result.expect("background target create should succeed")
    else {
        panic!("expected create target result");
    };
    let second_target_id = second_result.target_id;

    let background_route = ctx.with_fixture(|conn| {
        conn.target_session_route_for_target_id(second_target_id.as_str())
            .expect("background target route")
    });
    let background_owner = CommandOwnerScope::for_route(background_route);
    let pending = ctx.with_fixture(|conn| {
        conn.start_initial_document_page_ensure_for_owner(&background_owner)
            .expect("target lifecycle ensure should start background initial page")
            .expect("fresh background initial target should pend initial document page build")
    });

    let completed = pending
        .wait()
        .await
        .expect("background initial document page build should complete");
    (completed)
        .install_using(&mut crate::conn::ProtocolWorkflowDriver::new(ctx.handle()))
        .await
        .map(|_| ())
        .expect("background initial page should install on captured owner");

    ctx.with_fixture(|conn| {
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
    });
}

#[tokio::test]
async fn classic_create_target_ensures_fresh_initial_document_without_resolver_fallback() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverClassic,
        session_id: Some(DevToolsSessionId::from("classic-session-1")),
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
        create_result.expect("classic create target should succeed")
    else {
        panic!("expected create target result");
    };
    ctx.with_fixture(|conn| {
        assert!(
            conn.browser_context
                .as_ref()
                .expect("browser context")
                .has_loaded_page(),
            "classic create target should install its initial document during target lifecycle"
        )
    });

    let evaluate_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::EvaluateScript(DevToolsEvaluateScriptCommand {
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
            }),
        )
        .await;
    let evaluate_result = expect_script_value_result(
        evaluate_result.expect("classic evaluate should observe the target-lifecycle document"),
        "expected script value result",
    );
    assert_eq!(evaluate_result.value, json!("about:blank"));
}

#[tokio::test]
async fn devtools_create_target_can_activate_created_target() {
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let first_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        )
        .await;
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

    let second_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: true,
            }),
        )
        .await;
    let AutomationResult::CreateTarget(second_result) =
        second_result.expect("second create target should succeed")
    else {
        panic!("expected second create target result");
    };
    let second_target_id = second_result.target_id.clone();

    ctx.with_fixture(|conn| {
        let browser_context = conn.browser_context.as_ref().expect("browser context");
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
    });

    let first_realms_before_second_navigation = ctx
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::GetRealms(
            DevToolsGetRealmsCommand {
                context: AutomationContext {
                    target_id: Some(first_target_id.clone()),
                    ..context.clone()
                },
                realm_type: Some("window".to_owned()),
            },
        ))
        .await;
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

    let first_realms = ctx
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::GetRealms(
            DevToolsGetRealmsCommand {
                context: AutomationContext {
                    target_id: Some(first_target_id.clone()),
                    ..context.clone()
                },
                realm_type: Some("window".to_owned()),
            },
        ))
        .await;
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

    let all_realms = ctx
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::GetRealms(
            DevToolsGetRealmsCommand {
                context,
                realm_type: Some("window".to_owned()),
            },
        ))
        .await;
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
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let add_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::AddPreloadScript(DevToolsAddPreloadScriptCommand {
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
            }),
        )
        .await;
    let AutomationResult::AddPreloadScript(add_result) =
        add_result.expect("default addPreloadScript should succeed")
    else {
        panic!("expected add preload script result");
    };
    assert!(
        !add_result.script_id.as_str().contains(':'),
        "default BiDi preload ids should not be target-qualified"
    );
    ctx.with_fixture(|conn| {
        let browser_context = {
            conn.browser_context
                .as_ref()
                .expect("default preload should materialize the default browser context")
        };
        assert!(!browser_context.has_active_target());
        assert_eq!(browser_context.default_document_start_scripts.len(), 1);
    });

    let remove_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::RemovePreloadScript(DevToolsRemovePreloadScriptCommand {
                context: context.clone(),
                script_id: add_result.script_id.clone(),
            }),
        )
        .await;
    assert_eq!(
        remove_result.expect("default removePreloadScript should succeed"),
        AutomationResult::Empty
    );
    ctx.with_fixture(|conn| {
        assert!(
            conn.browser_context
                .as_ref()
                .expect("browser context")
                .default_document_start_scripts
                .is_empty(),
            "default preload removal should clear the browser-context registry"
        )
    });

    let remove_again_result = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::RemovePreloadScript(DevToolsRemovePreloadScriptCommand {
                context,
                script_id: add_result.script_id,
            }),
        )
        .await;
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
    ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser_context));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>original viewport owner</title>",
        None,
    )
    .await;
    ctx.activate_fixture_target("TID-emulation-viewport-replacement")
        .await
        .unwrap();
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>replacement active page</title>",
        None,
    )
    .await;
    ctx.activate_fixture_target("TID-emulation-viewport-original")
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
    let pending = match ctx.with_fixture(|conn| conn.start_command_dispatch(&raw)) {
        CdpCommandTaskStep::Pending(pending) => pending,
        CdpCommandTaskStep::Complete(outcome) => {
            panic!(
                "live Emulation.setDeviceMetricsOverride should update the original Page: {:?}",
                outcome.into_parts().0
            )
        }
    };

    ctx.activate_fixture_target("TID-emulation-viewport-replacement")
        .await
        .expect("target activation should succeed");
    ctx.with_fixture(|conn| {
        assert_eq!(
            conn.browser_context.as_ref().unwrap().active_target_id(),
            Some("TID-emulation-viewport-replacement")
        )
    });
    // The interleaved activation can already project the original reply.
    let mut messages = ctx.take_all();
    let (completion_messages, _) = ctx
        .complete_command_task_step_for_test(CdpCommandTaskStep::Pending(pending))
        .await;
    messages.extend(completion_messages);
    let replies = messages
        .into_iter()
        .filter(|message| message["id"] == json!(690))
        .collect::<Vec<_>>();

    assert_eq!(replies, vec![json!({ "id": 690, "result": {} })]);
    ctx.with_fixture(|conn| {
        let browser_context = conn.browser_context.as_ref().expect("browser context");
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
    });
}

#[tokio::test]
async fn tab_auto_attach_waits_outside_completion_for_the_original_page_install() {
    use futures_util::FutureExt as _;
    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    ctx.with_fixture(|conn| conn.publish_default_browser_target());
    let mut page_context = CommandDispatchContext::default();
    let page_command = ParsedCdpCommand::parse_str(
        json!({"id":1,"method":"Target.attachToTarget",
        "params":{"targetId":crate::DEFAULT_CDP_PAGE_TARGET_ID,"flatten":true}})
        .to_string(),
    )
    .unwrap();
    let CdpCommandTaskStep::Pending(page_attach) = ctx.with_fixture(|conn| {
        conn.start_parsed_command_dispatch_with_context(&page_command, &mut page_context)
    }) else {
        panic!("the first Page attachment must own its initial document build");
    };
    let tab_command = ParsedCdpCommand::parse_str(
        json!({"id":2,"method":"Target.attachToTarget",
        "params":{"targetId":crate::DEFAULT_CDP_TAB_TARGET_ID,"flatten":true}})
        .to_string(),
    )
    .unwrap();
    let CdpCommandTaskStep::Complete(tab) = ctx.with_fixture(|conn| {
        conn.start_parsed_command_dispatch_with_context(
            &tab_command,
            &mut CommandDispatchContext::default(),
        )
    }) else {
        panic!("attaching the Tab host does not wait for its Page");
    };
    let (events, _, _) = tab.into_protocol_event_parts();
    let tab_session = events
        .into_iter()
        .map(BackgroundProtocolEvent::into_protocol_message)
        .find(|message| message["id"] == 2)
        .unwrap()["result"]["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut auto_context = CommandDispatchContext::default();
    let auto_command = ParsedCdpCommand::parse_str(
        json!({"id":3,"sessionId":tab_session,"method":"Target.setAutoAttach",
        "params":{"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true}})
        .to_string(),
    )
    .unwrap();
    let CdpCommandTaskStep::Pending(auto_attach) = ctx.with_fixture(|conn| {
        conn.start_parsed_command_dispatch_with_context(&auto_command, &mut auto_context)
    }) else {
        panic!("auto-attach retains its pending dependency");
    };
    // Hold the original Page's ready result until after the dependent command
    // is polled. The actor must remain able to install that original result.
    let completed_page = page_attach.wait().await;
    let mut auto_wait = Box::pin(auto_attach.wait());
    assert!(
        auto_wait.as_mut().now_or_never().is_none(),
        "auto-attach cannot join installation while borrowing the actor"
    );
    ctx.with_fixture(|conn| {
        assert!(matches!(
            conn.start_command_dispatch(r#"{"id":4,"method":"Browser.getVersion"}"#),
            CdpCommandTaskStep::Complete(_)
        ))
    });
    let page_step = {
        let completed = completed_page;
        ctx.state
            .with_state(|state| {
                state.complete_command_dispatch_with_context(completed, &mut page_context)
            })
            .expect("live test owner")
    };
    let (page_messages, page_events) = ctx
        .complete_command_task_step_with_context_for_test(page_step, &mut page_context)
        .await;
    assert!(
        page_messages
            .iter()
            .any(|message| message["id"] == 1 && message.get("result").is_some()),
        "the original Page attach must finish installation: {page_messages:?}"
    );
    ctx.sent.extend(page_messages);
    ctx.route_direct_command_output_for_test(Vec::new(), page_events)
        .await;
    let completed_auto = auto_wait.await;
    let CdpCommandTaskStep::Complete(outcome) = ({
        let completed = completed_auto;
        ctx.state
            .with_state(|state| {
                state.complete_command_dispatch_with_context(completed, &mut auto_context)
            })
            .expect("live test owner")
    }) else {
        panic!("auto-attach can now publish its child session");
    };
    let (events, _, _) = outcome.into_protocol_event_parts();
    let messages: Vec<_> = events
        .into_iter()
        .map(BackgroundProtocolEvent::into_protocol_message)
        .collect();
    let response = messages
        .iter()
        .position(|message| message["id"] == 3)
        .expect("auto-attach response");
    let attached = messages
        .iter()
        .position(|message| {
            message["method"] == "Target.attachedToTarget"
                && message["params"]["targetInfo"]["targetId"] == crate::DEFAULT_CDP_PAGE_TARGET_ID
        })
        .expect("original Page child attachment");
    assert!(
        attached < response,
        "the child session must precede the auto-attach ACK"
    );
    assert_eq!(messages[response]["result"], json!({}));
}
