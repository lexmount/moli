use super::*;

#[tokio::test]
async fn devtools_fetch_control_command_routes_through_fetch_owner() {
    let mut conn = CdpConnection::new();
    conn.browser_context = Some(BrowserContext::new_with_page_for_test(
        "BID-fetch-control",
        "TID-fetch-control",
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
async fn devtools_network_intercept_commands_route_to_fetch_owner() {
    let mut conn = CdpConnection::new();
    let mut browser_context = BrowserContext::new("BID-bidi-intercept".to_owned());
    browser_context.set_active_target_id("TID-bidi-intercept".to_owned());
    conn.install_browser_context_fixture_for_test(browser_context);
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
            .active_page_target()
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
        .active_page_target()
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
            .active_page_target()
            .fetch_owner
            .is_enabled()
    );

    conn.browser_context
        .as_mut()
        .expect("browser context")
        .insert_page_target_host(PageTargetHost::with_url(
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
            .background_target("TID-bidi-intercept-background")
            .filter(|target| target.has_non_default_session_state())
            .is_some_and(|state| state.fetch_owner.is_enabled()),
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
            .background_target("TID-bidi-intercept-background")
            .filter(|target| target.has_non_default_session_state())
            .is_none_or(|state| !state.fetch_owner.is_enabled()),
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
    let owner = CommandOwnerScope::capture(&conn, None);
    let preflight = conn
        .prepare_navigation_request_for_owner(&owner, &auth_url, None, false)
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
