use super::*;

#[tokio::test]
async fn scoped_cookie_write_uses_one_partition_identity_for_storage_and_response() {
    use crate::automation::{DevToolsCookieScope, StoredCookiePartitionFilter};

    let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
    {
        let fixture_value = Some(BrowserContext::new("BID-scoped-write".into()));
        ctx.with_fixture(|conn| {
            conn.browser_context = fixture_value;
        });
    }
    let context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: None,
        target_id: None,
        browser_context_id: None,
    };
    let scope = DevToolsCookieScope {
        browser_context_id: Some(DevToolsBrowserContextId::from("BID-scoped-write")),
        partition_filter: StoredCookiePartitionFilter::Serialized {
            top_level_site: Some("https://site-a.test".into()),
            has_cross_site_ancestor: Some(true),
        },
    };
    let cookie = DevToolsCookieParam {
        name: "scoped".into(),
        value: "one".into(),
        url: Some("https://example.com/".into()),
        domain: None,
        path: Some("/".into()),
        secure: Some(true),
        http_only: false,
        same_site: Some("None".into()),
        priority: None,
        source_scheme: None,
        source_port: None,
        expires: None,
        partition_key: Some(
            json!({"topLevelSite":"https://site-b.test", "hasCrossSiteAncestor":false}),
        ),
        partition_key_opaque: Some(true),
    };
    let result = ctx
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::SetCookies(
            DevToolsSetCookiesCommand {
                context: context.clone(),
                scope: scope.clone(),
                cookies: vec![cookie],
            },
        ))
        .await;
    let AutomationResult::SetCookies(result) = result.expect("scoped write") else {
        panic!("set cookies");
    };
    assert!(result.success);
    assert_eq!(result.scope, scope);
    let result = ctx
        .execute_automation_command_through_renderer_fence_for_test(AutomationCommand::GetCookies(
            DevToolsGetCookiesCommand {
                context,
                scope: DevToolsCookieScope {
                    partition_filter: StoredCookiePartitionFilter::All,
                    ..scope
                },
                urls: None,
                filter: None,
            },
        ))
        .await;
    let AutomationResult::GetCookies(result) = result.expect("stored cookies") else {
        panic!("get cookies");
    };
    assert_eq!(result.cookies.len(), 1);
    assert_eq!(
        result.cookies[0]["partitionKey"],
        json!({"topLevelSite":"https://site-a.test", "hasCrossSiteAncestor":true})
    );
}

#[tokio::test]
async fn automation_command_executes_storage_cookie_commands_for_webdriver_context() {
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
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = AutomationContext {
        target_id: Some(create_result.target_id),
        ..context
    };
    let cookie_url = "https://example.com/path".to_owned();

    let (set_result, set_events) = ctx
        .execute_owned_automation_command_for_test(AutomationCommand::SetCookies(
            DevToolsSetCookiesCommand {
                context: target_context.clone(),
                scope: Default::default(),
                cookies: vec![DevToolsCookieParam {
                    name: "sid".to_owned(),
                    value: "abc".to_owned(),
                    url: Some(cookie_url.clone()),
                    domain: None,
                    path: Some("/".to_owned()),
                    secure: Some(true),
                    http_only: true,
                    same_site: Some("Lax".to_owned()),
                    priority: None,
                    source_scheme: None,
                    source_port: None,
                    partition_key: None,
                    partition_key_opaque: None,
                    expires: None,
                }],
            },
        ))
        .await
        .into_parts();
    assert!(
        set_events.is_empty(),
        "direct storage setCookies must not emit CDP-shaped sidecar messages: {set_events:?}"
    );
    let AutomationResult::SetCookies(set_result) = set_result.expect("set cookies should succeed")
    else {
        panic!("expected set cookies result");
    };
    assert!(set_result.success);

    let (get_result, get_events) = ctx
        .execute_owned_automation_command_for_test(AutomationCommand::GetCookies(
            DevToolsGetCookiesCommand {
                context: target_context.clone(),
                scope: Default::default(),
                urls: Some(vec![cookie_url.clone()]),
                filter: None,
            },
        ))
        .await
        .into_parts();
    assert!(
        get_events.is_empty(),
        "direct storage getCookies must not emit CDP-shaped sidecar messages: {get_events:?}"
    );
    let AutomationResult::GetCookies(get_result) = get_result.expect("get cookies should succeed")
    else {
        panic!("expected get cookies result");
    };
    assert_eq!(get_result.cookies.len(), 1);
    assert_eq!(get_result.cookies[0]["name"], json!("sid"));
    assert_eq!(get_result.cookies[0]["value"], json!("abc"));

    let (delete_result, delete_events) = ctx
        .execute_owned_automation_command_for_test(AutomationCommand::DeleteCookies(
            DevToolsDeleteCookiesCommand {
                context: target_context.clone(),
                scope: crate::automation::DevToolsCookieScope {
                    browser_context_id: None,
                    partition_filter: moli_cookie_jar::StoredCookiePartitionFilter::All,
                },
                filter: crate::automation::DevToolsCookieDeleteFilter::Scope {
                    name: Some("sid".to_owned()),
                    url: Some(cookie_url.clone()),
                    domain: None,
                    domain_match: moli_cookie_jar::StoredCookieDomainMatch::Canonical,
                    path: None,
                },
            },
        ))
        .await
        .into_parts();
    assert!(
        delete_events.is_empty(),
        "direct storage deleteCookies must not emit CDP-shaped sidecar messages: {delete_events:?}"
    );
    assert!(matches!(
        delete_result.expect("delete cookie should succeed"),
        AutomationResult::DeleteCookies(_)
    ));

    let (after_delete, after_delete_events) = ctx
        .execute_owned_automation_command_for_test(AutomationCommand::GetCookies(
            DevToolsGetCookiesCommand {
                context: target_context,
                scope: Default::default(),
                urls: Some(vec![cookie_url]),
                filter: None,
            },
        ))
        .await
        .into_parts();
    assert!(
        after_delete_events.is_empty(),
        "direct storage getCookies after delete must not emit CDP-shaped sidecar messages: {after_delete_events:?}"
    );
    let AutomationResult::GetCookies(after_delete) =
        after_delete.expect("get cookies after delete should succeed")
    else {
        panic!("expected get cookies result");
    };
    assert!(after_delete.cookies.is_empty());
}
