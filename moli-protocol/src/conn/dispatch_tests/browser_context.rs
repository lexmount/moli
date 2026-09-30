use super::*;

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
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (create_result, create_events) = conn
        .execute_automation_command(AutomationCommand::CreateBrowserContext(
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
    let AutomationResult::CreateBrowserContext(create_result) =
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
        let context = AutomationContext {
            protocol: FrontendProtocol::WebDriverBidi,
            session_id: Some(DevToolsSessionId::from("bidi-session-1")),
            target_id: None,
            browser_context_id: None,
        };

        let (create_result, create_events) = conn
            .execute_automation_command(AutomationCommand::CreateBrowserContext(
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
async fn devtools_browser_context_commands_create_list_and_remove_user_context() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (create_result, create_events) = conn
        .execute_automation_command(AutomationCommand::CreateBrowserContext(
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
    let AutomationResult::CreateBrowserContext(create_result) =
        create_result.expect("create browser context should succeed")
    else {
        panic!("expected create browser context result");
    };
    assert_eq!(create_result.browser_context_id.as_str(), "user-context-1");
    let created_context = conn
        .browser_context_by_id("user-context-1")
        .expect("created browser context");
    assert_eq!(
        created_context.default_tls_verify_host_override,
        Some(false)
    );
    assert_eq!(
        created_context.default_http_proxy_override.as_deref(),
        Some("127.0.0.1:80")
    );
    assert_eq!(
        created_context.default_http_no_proxy_override.as_deref(),
        Some("localhost,127.0.0.1")
    );
    assert_eq!(created_context.proxy_autoconfig_url, None);
    assert_eq!(created_context.proxy_socks_version, None);

    let (get_contexts_result, _) = conn
        .execute_automation_command(AutomationCommand::GetBrowserContexts(
            DevToolsGetBrowserContextsCommand {
                context: context.clone(),
            },
        ))
        .await
        .into_parts();
    let AutomationResult::GetBrowserContexts(get_contexts_result) =
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
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: Some(create_result.browser_context_id.clone()),
                activate: true,
            },
        ))
        .await
        .into_parts();
    let AutomationResult::CreateTarget(create_target_result) =
        create_target_result.expect("create target in user context should succeed")
    else {
        panic!("expected create target result");
    };

    let (remove_result, _, remove_events) = conn
        .execute_automation_command_with_protocol_events(AutomationCommand::RemoveBrowserContext(
            DevToolsRemoveBrowserContextCommand {
                context,
                browser_context_id: create_result.browser_context_id,
            },
        ))
        .await
        .into_parts_with_protocol_events();
    assert_eq!(
        remove_result.expect("remove browser context should succeed"),
        AutomationResult::Empty
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
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let (create_result, create_events) = conn
        .execute_automation_command(AutomationCommand::CreateBrowserContext(
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
    let AutomationResult::CreateBrowserContext(create_result) =
        create_result.expect("create browser context should succeed")
    else {
        panic!("expected create browser context result");
    };
    let created_context = conn
        .browser_context_by_id(create_result.browser_context_id.as_str())
        .expect("created browser context");
    assert_eq!(
        created_context.default_http_proxy_override.as_deref(),
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
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (remove_result, _, remove_events) = conn
        .execute_automation_command_with_protocol_events(AutomationCommand::RemoveBrowserContext(
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
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                    target_id: None,
                    browser_context_id: Some(DevToolsBrowserContextId::from("BID-default")),
                },
                url: "about:blank".to_owned(),
                browser_context_id: Some(DevToolsBrowserContextId::from("BID-default")),
                activate: true,
            },
        ))
        .await
        .into_parts();
    let AutomationResult::CreateTarget(create_result) =
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
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                    target_id: Some(DevToolsTargetId::from("TID-reference")),
                    browser_context_id: None,
                },
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: true,
            },
        ))
        .await
        .into_parts();
    assert!(create_events.is_empty());
    let AutomationResult::CreateTarget(create_result) =
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
        "the reference context's previous active target should be deactivated inside the same browser context"
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
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: AutomationContext {
                    protocol: FrontendProtocol::WebDriverBidi,
                    session_id: Some(DevToolsSessionId::from("bidi-session-1")),
                    target_id: Some(DevToolsTargetId::from("TID-reference")),
                    browser_context_id: Some(DevToolsBrowserContextId::from("BID-explicit")),
                },
                url: "about:blank".to_owned(),
                browser_context_id: Some(DevToolsBrowserContextId::from("BID-explicit")),
                activate: true,
            },
        ))
        .await
        .into_parts();
    assert!(create_events.is_empty());
    let AutomationResult::CreateTarget(create_result) =
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
        "reference context should not be deactivated when explicit browser context is provided"
    );
}

#[tokio::test]
async fn automation_command_preserves_remove_browser_context_detached_typed_sidecar() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("user-context-detach-sidecar".to_owned());
    browser_context.set_active_target_id("TID-dispose-sidecar".to_owned());
    browser_context.attach_active_session("SID-dispose-sidecar".to_owned());
    conn.insert_browser_context(browser_context);

    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (result, scheduler_events, protocol_events) = conn
        .execute_automation_command_with_protocol_events(AutomationCommand::RemoveBrowserContext(
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
        AutomationResult::Empty
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
async fn automation_command_rejects_missing_user_context_viewport_override() {
    let mut conn = CdpConnection::new();
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (viewport_result, _) = conn
        .execute_automation_command(AutomationCommand::SetViewport(DevToolsSetViewportCommand {
            context: context.clone(),
            browser_context_ids: vec![crate::automation::DevToolsBrowserContextId::from(
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
async fn automation_command_applies_known_user_context_viewport_default() {
    let mut conn = CdpConnection::new();
    let browser_context = conn.new_browser_context("custom-user-context".to_owned());
    conn.insert_browser_context(browser_context);
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let (viewport_result, _) = conn
        .execute_automation_command(AutomationCommand::SetViewport(DevToolsSetViewportCommand {
            context: context.clone(),
            browser_context_ids: vec![crate::automation::DevToolsBrowserContextId::from(
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
        AutomationResult::Empty
    );
    let browser_context = conn
        .browser_context_by_id("custom-user-context")
        .expect("custom user context should exist");
    assert!(
        browser_context.active_target_id().is_none(),
        "userContext viewport should not create a page target"
    );
    let default_metrics = browser_context
        .default_emulated_device_metrics
        .as_ref()
        .expect("userContext should hold default emulated device metrics");
    assert_eq!(default_metrics.width, 800);
    assert_eq!(default_metrics.height, 600);
    assert_eq!(default_metrics.device_scale_factor, 2.0);

    let (create_result, _) = conn
        .execute_automation_command(AutomationCommand::CreateTarget(
            DevToolsCreateTargetCommand {
                context: context.clone(),
                url: "about:blank".to_owned(),
                browser_context_id: Some(DevToolsBrowserContextId::from("custom-user-context")),
                activate: true,
            },
        ))
        .await
        .into_parts();
    let AutomationResult::CreateTarget(create_result) =
        create_result.expect("create target in userContext should succeed")
    else {
        panic!("expected create target result");
    };
    let route = conn
        .target_session_route_for_target_id(create_result.target_id.as_str())
        .expect("created target route");
    let inherited_metrics = conn
        .target_session_owner_emulated_device_metrics_for_owner(
            &crate::conn::CommandOwnerScope::for_route(route.clone()),
        )
        .expect("new target should inherit userContext default metrics");
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
async fn devtools_storage_cookie_commands_scope_to_target_browser_context() {
    let mut conn = CdpConnection::new();
    let mut default_context = BrowserContext::new("BID-default".to_owned());
    default_context.set_active_target_id("TID-default".to_owned());
    let mut custom_context = BrowserContext::new("BID-custom".to_owned());
    custom_context.set_active_target_id("TID-custom".to_owned());
    conn.install_browser_context_fixture_for_test(default_context);
    conn.push_inactive_browser_context_fixture_for_test(custom_context);

    let base_context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };
    let default_target_context = AutomationContext {
        target_id: Some(DevToolsTargetId::from("TID-default")),
        ..base_context.clone()
    };
    let custom_target_context = AutomationContext {
        target_id: Some(DevToolsTargetId::from("TID-custom")),
        ..base_context
    };
    let cookie_url = "https://example.com/path".to_owned();

    let (set_result, _) = conn
        .execute_automation_command(AutomationCommand::SetCookies(DevToolsSetCookiesCommand {
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
    let AutomationResult::SetCookies(set_result) =
        set_result.expect("custom target setCookies should succeed")
    else {
        panic!("expected set cookies result");
    };
    assert!(set_result.success);

    let (custom_get, _) = conn
        .execute_automation_command(AutomationCommand::GetCookies(DevToolsGetCookiesCommand {
            context: custom_target_context,
            browser_context_id: None,
            urls: Some(vec![cookie_url.clone()]),
            filter: None,
        }))
        .await
        .into_parts();
    let AutomationResult::GetCookies(custom_get) =
        custom_get.expect("custom target getCookies should succeed")
    else {
        panic!("expected get cookies result");
    };
    assert_eq!(custom_get.cookies.len(), 1);
    assert_eq!(custom_get.cookies[0]["name"], json!("targetScoped"));
    assert_eq!(custom_get.cookies[0]["value"], json!("custom"));

    let (default_get, _) = conn
        .execute_automation_command(AutomationCommand::GetCookies(DevToolsGetCookiesCommand {
            context: default_target_context,
            browser_context_id: None,
            urls: Some(vec![cookie_url]),
            filter: None,
        }))
        .await
        .into_parts();
    let AutomationResult::GetCookies(default_get) =
        default_get.expect("default target getCookies should succeed")
    else {
        panic!("expected get cookies result");
    };
    assert!(default_get.cookies.is_empty());
}

#[tokio::test]
async fn automation_command_executes_user_context_preload_without_default_leakage() {
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
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-session-1")),
        target_id: None,
        browser_context_id: None,
    };

    let add_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::AddPreloadScript(DevToolsAddPreloadScriptCommand {
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
    let AutomationResult::AddPreloadScript(add_result) =
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
    let custom_route = ctx
        .conn
        .target_session_route_for_target_id(custom_target.as_str())
        .expect("custom userContext target route");
    assert!(
        ctx.conn
            .navigation_load_inputs_for_owner(&crate::conn::CommandOwnerScope::for_route(
                custom_route.clone()
            ))
            .document_start_scripts
            .iter()
            .any(|script| script.source.contains("__bidiUserContextPreload")),
        "custom userContext target navigation inputs should retain its context preload script"
    );
    let custom_context = AutomationContext {
        target_id: Some(custom_target.clone()),
        ..context.clone()
    };
    execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::Navigate(DevToolsNavigateCommand {
            context: custom_context.clone(),
            url: "data:text/html,user-context-preload".to_owned(),
            referrer: None,
            wait: DevToolsNavigationWait::Load,
        }),
    )
    .await
    .expect("custom userContext navigation should succeed");
    let custom_location = evaluate_string_through_renderer_fence_for_test(
        &mut ctx,
        custom_context.clone(),
        "location.href",
        "custom preload location",
    )
    .await;
    assert_eq!(custom_location, "data:text/html,user-context-preload");
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
    let second_context = AutomationContext {
        target_id: Some(second_target.clone()),
        ..context.clone()
    };
    execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::Navigate(DevToolsNavigateCommand {
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
    let default_context = AutomationContext {
        target_id: Some(default_target),
        ..context.clone()
    };
    execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::Navigate(DevToolsNavigateCommand {
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

    let remove_result = execute_direct_automation_command_through_renderer_fence_for_test(
        &mut ctx,
        AutomationCommand::RemovePreloadScript(DevToolsRemovePreloadScriptCommand {
            context,
            script_id: script_id.clone(),
        }),
    )
    .await;
    assert_eq!(
        remove_result.expect("userContext removePreloadScript should succeed"),
        AutomationResult::Empty
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
