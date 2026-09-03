use super::*;

#[tokio::test]
async fn process_message_invalid_json_and_invalid_method_cases_match_current_cdp_entrypoint() {
    let mut conn = CdpConnection::new();

    let parse_error = conn.process_message_messages_only_for_test("invalid").await;
    assert_eq!(
        parse_error,
        vec![json!({"id": null, "error": {"code": -32700, "message": "Parse error"}})]
    );

    let missing_method = conn.process_message_messages_only_for_test("{}").await;
    assert_eq!(
        missing_method,
        vec![json!({"id": null, "error": {"code": -32600, "message": "Invalid Request"}})]
    );

    let invalid_method = conn
        .process_message_messages_only_for_test(r#"{"id":1,"method":"Target"}"#)
        .await;
    assert_eq!(
        invalid_method,
        vec![json!({"id": 1, "error": {"code": -32600, "message": "Invalid method"}})]
    );

    let unknown_domain = conn
        .process_message_messages_only_for_test(r#"{"id":2,"method":"Unknown.domain"}"#)
        .await;
    assert_eq!(
        unknown_domain,
        vec![json!({"id": 2, "error": {"code": -32601, "message": "Unknown domain"}})]
    );

    let private_lp_domain = conn
        .process_message_messages_only_for_test(r#"{"id":4,"method":"LP.getMarkdown"}"#)
        .await;
    assert_eq!(
        private_lp_domain,
        vec![json!({"id": 4, "error": {"code": -32601, "message": "Unknown domain"}})]
    );

    let unknown_method = conn
        .process_message_messages_only_for_test(r#"{"id":3,"method":"Target.over9000"}"#)
        .await;
    assert_eq!(
        unknown_method,
        vec![json!({"id": 3, "error": {"code": -32601, "message": "UnknownMethod"}})]
    );
}

#[tokio::test]
async fn session_scoped_unknown_domain_error_keeps_session_id() {
    let mut conn = CdpConnection::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    conn.install_browser_context_fixture_for_test(bc);

    let unknown_domain = conn
        .process_message_messages_only_for_test(
            r#"{"id":13,"method":"Unknown.enable","sessionId":"SID-1"}"#,
        )
        .await;
    assert_eq!(
        unknown_domain,
        vec![json!({
            "id": 13,
            "error": {"code": -32601, "message": "Unknown domain"},
            "sessionId": "SID-1"
        })]
    );
}

#[tokio::test]
async fn session_scoped_unknown_method_error_keeps_session_id() {
    let mut conn = CdpConnection::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    conn.install_browser_context_fixture_for_test(bc);

    let unknown_method = conn
        .process_message_messages_only_for_test(
            r#"{"id":17,"method":"Page.noSuchMethod","sessionId":"SID-1"}"#,
        )
        .await;
    assert_eq!(
        unknown_method,
        vec![json!({
            "id": 17,
            "error": {"code": -32601, "message": "UnknownMethod"},
            "sessionId": "SID-1"
        })]
    );
}

#[tokio::test]
async fn browser_and_tab_sessions_do_not_fall_through_to_the_active_page() {
    let mut conn = CdpConnection::new();
    conn.install_default_browser_target();
    conn.register_browser_session("SID-browser".to_owned());
    let default_tab_target_id = conn.default_tab_target_id().to_owned();
    conn.attach_tab_target_session_event_plan(
        "SID-tab".to_owned(),
        None,
        &default_tab_target_id,
        false,
    )
    .expect("default tab target should accept a committed session");

    for session_id in ["SID-browser", "SID-tab"] {
        let response = conn
            .process_message_messages_only_for_test(&format!(
                r#"{{"id":19,"method":"Page.getFrameTree","sessionId":"{session_id}"}}"#
            ))
            .await;
        assert_eq!(
            response,
            vec![json!({
                "id": 19,
                "error": {
                    "code": -32601,
                    "message": "'Page.getFrameTree' wasn't found"
                },
                "sessionId": session_id
            })]
        );
    }
}

#[tokio::test]
async fn session_scoped_handler_error_keeps_session_id_even_when_domain_uses_plain_error_helper() {
    let mut conn = CdpConnection::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    conn.install_browser_context_fixture_for_test(bc);

    let invalid_dialog = conn
        .process_message_messages_only_for_test(
            r#"{"id":18,"method":"Page.handleJavaScriptDialog","params":null,"sessionId":"SID-1"}"#,
        )
        .await;
    assert_eq!(
        invalid_dialog,
        vec![json!({
            "id": 18,
            "error": {"code": -32602, "message": "InvalidParams"},
            "sessionId": "SID-1"
        })]
    );
}

#[tokio::test]
async fn puppeteer_bootstrap_domains_are_session_scoped_noops() {
    let mut conn = CdpConnection::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    conn.install_browser_context_fixture_for_test(bc);

    let audits = conn
        .process_message_messages_only_for_test(
            r#"{"id":14,"method":"Audits.enable","sessionId":"SID-1"}"#,
        )
        .await;
    assert_eq!(
        audits,
        vec![json!({"id": 14, "result": {}, "sessionId": "SID-1"})]
    );

    let web_mcp = conn
        .process_message_messages_only_for_test(
            r#"{"id":15,"method":"WebMCP.enable","sessionId":"SID-1"}"#,
        )
        .await;
    assert_eq!(
        web_mcp,
        vec![json!({"id": 15, "result": {}, "sessionId": "SID-1"})]
    );
}

#[tokio::test]
async fn startup_session_id_uses_startup_dispatch_without_browser_context() {
    let mut conn = CdpConnection::new();

    let generic = conn
        .process_message_messages_only_for_test(
            r#"{"id":2,"method":"Hi.there","sessionId":"STARTUP"}"#,
        )
        .await;
    assert_eq!(
        generic,
        vec![json!({"id": 2, "result": {}, "sessionId": "STARTUP"})]
    );

    let frame_tree = conn
        .process_message_messages_only_for_test(
            r#"{"id":3,"method":"Page.getFrameTree","sessionId":"STARTUP"}"#,
        )
        .await;
    assert_eq!(frame_tree.len(), 1);
    assert_eq!(frame_tree[0]["id"], 3);
    assert_eq!(frame_tree[0]["sessionId"], "STARTUP");
    assert_eq!(
        frame_tree[0]["result"]["frameTree"]["frame"]["id"],
        "TID-STARTUP"
    );
    assert_eq!(
        frame_tree[0]["result"]["frameTree"]["frame"]["url"],
        "about:blank"
    );

    conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    let still_startup = conn
        .process_message_messages_only_for_test(
            r#"{"id":4,"method":"Hi.there","sessionId":"STARTUP"}"#,
        )
        .await;
    assert_eq!(
        still_startup,
        vec![json!({"id": 4, "result": {}, "sessionId": "STARTUP"})]
    );
}
