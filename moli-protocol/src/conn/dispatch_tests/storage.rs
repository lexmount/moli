use super::*;

#[tokio::test]
async fn automation_command_executes_storage_cookie_commands_for_webdriver_context() {
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
        create_result.expect("create target should succeed")
    else {
        panic!("expected create target result");
    };
    let target_context = AutomationContext {
        target_id: Some(create_result.target_id),
        ..context
    };
    let cookie_url = "https://example.com/path".to_owned();

    let (set_result, set_events) = conn
        .execute_automation_command(AutomationCommand::SetCookies(DevToolsSetCookiesCommand {
            context: target_context.clone(),
            browser_context_id: None,
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
        }))
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

    let (get_result, get_events) = conn
        .execute_automation_command(AutomationCommand::GetCookies(DevToolsGetCookiesCommand {
            context: target_context.clone(),
            browser_context_id: None,
            urls: Some(vec![cookie_url.clone()]),
            filter: None,
        }))
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

    let (delete_result, delete_events) = conn
        .execute_automation_command(AutomationCommand::DeleteCookies(
            DevToolsDeleteCookiesCommand {
                context: target_context.clone(),
                browser_context_id: None,
                name: Some("sid".to_owned()),
                url: Some(cookie_url.clone()),
                domain: None,
                path: None,
                partition_key: None,
                filter: None,
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

    let (after_delete, after_delete_events) = conn
        .execute_automation_command(AutomationCommand::GetCookies(DevToolsGetCookiesCommand {
            context: target_context,
            browser_context_id: None,
            urls: Some(vec![cookie_url]),
            filter: None,
        }))
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
