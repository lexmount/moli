use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn get_response_body_success_queued_before_supersession_cannot_restore_the_old_request() {
    use crate::domains::fetch::{
        CompletedFetchCommandOperation, FetchCommandTaskStep, complete_pending_fetch_command,
        try_start_fetch_command_dispatch,
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/{page}", get(|| async { "response body" })),
        )
        .await
        .unwrap();
    });
    for cancel_replacement in [false, true] {
        let mut ctx = TestContext::new();
        ctx.conn
            .install_browser_context_fixture_for_test(attached_browser_context());
        ctx.process_async(
            json!({"id": 1, "sessionId": "SID-1", "method": "Fetch.enable",
            "params": {"patterns": [{"urlPattern": "*", "requestStage": "Response"}]}}),
        )
        .await;
        ctx.expect_result(1, json!({}), Some("SID-1"));
        ctx.process_async(
            json!({"id": 2, "sessionId": "SID-1", "method": "Page.navigate",
            "params": {"url": format!("http://{addr}/old")}}),
        )
        .await;
        let old = ctx
            .wait_for_scheduler_message("old response pause", |message| {
                message["method"] == "Fetch.requestPaused"
            })
            .await["params"]["requestId"]
            .as_str()
            .unwrap()
            .to_owned();
        let command = moli_protocol_cdp::ParsedCdpCommand::from_serializable(
            json!({"id": 3, "sessionId": "SID-1", "method": "Fetch.getResponseBody",
            "params": {"requestId": old}}),
        )
        .unwrap();
        let cmd = crate::conn::Cmd::from_parsed(&command).unwrap();
        let Some(FetchCommandTaskStep::Pending(read)) =
            try_start_fetch_command_dispatch(&mut ctx.conn, &cmd)
        else {
            panic!("getResponseBody must hand its body to the read task");
        };
        // Materialization succeeds, but its completion has not yet returned to
        // the owner. Supersession must reject this queued success too.
        let completed = read.wait().await;
        assert!(matches!(
            &completed.completed,
            CompletedFetchCommandOperation::MaterializeResponseBody { completed, .. }
                if matches!(completed.result(), Ok(Some(bytes)) if bytes == b"response body")
        ));
        ctx.process_async(
            json!({"id": 4, "sessionId": "SID-1", "method": "Page.navigate",
            "params": {"url": format!("http://{addr}/current")}}),
        )
        .await;
        let current = ctx
            .wait_for_scheduler_message("current response pause", |message| {
                message["method"] == "Fetch.requestPaused"
            })
            .await["params"]["requestId"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(old, current);
        assert_eq!(
            take_response_by_id(&mut ctx, 2)["result"]["errorText"],
            "net::ERR_ABORTED",
            "supersession must reply before the body read completion is processed"
        );
        if cancel_replacement {
            ctx.process_async(
                json!({"id": 5, "sessionId": "SID-1", "method": "Fetch.failRequest",
                "params": {"requestId": current, "errorReason": "Aborted"}}),
            )
            .await;
        }
        let FetchCommandTaskStep::Complete(plan) =
            complete_pending_fetch_command(&mut ctx.conn, completed).await
        else {
            panic!("body read completion must terminate");
        };
        plan.emit_into(&mut ctx.sent, Some(3), Some("SID-1"));
        assert_eq!(
            take_response_by_id(&mut ctx, 3)["error"]["message"],
            "RequestNotFound"
        );
        assert!(!ctx.sent.iter().any(|message| message["id"] == 2));
        assert_eq!(
            ctx.conn
                .renderer_document_navigation_is_suspended_for_session_owner(Some("SID-1")),
            !cancel_replacement
        );
        let fetch = &ctx
            .conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .fetch_owner;
        assert!(!fetch.has_pending_fetch_request_id_for_test(&old));
        assert!(!fetch.pending_fetch_response_transfer_is_pending_for_test(&old));
        assert_eq!(
            fetch.has_pending_fetch_request_id_for_test(&current),
            !cancel_replacement
        );
        ctx.process_async(
            json!({"id": 6, "sessionId": "SID-1", "method": "Fetch.continueResponse",
            "params": {"requestId": old}}),
        )
        .await;
        assert_eq!(
            take_response_by_id(&mut ctx, 6)["error"]["message"],
            "RequestNotFound"
        );
        assert!(
            !ctx.sent.iter().any(|message| message["id"] == 2),
            "late actions must not reply to Page.navigate twice"
        );
    }
    server.abort();
}

async fn start_request_pause(
    ctx: &mut TestContext,
    id: u64,
    session_id: &str,
    url: &str,
) -> String {
    ctx.process_async(json!({
        "id": id, "sessionId": session_id, "method": "Page.navigate", "params": {"url": url}
    }))
    .await;
    let pause = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == "Fetch.requestPaused"
                && message["sessionId"] == session_id
                && message["params"]["request"]["url"] == url
        })
        .expect("request-stage pause");
    pause["params"]["requestId"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn supersession_only_retires_interceptions_owned_by_the_same_target() {
    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.insert_page_target_host(PageTargetHost::with_url(
        "TID-2".into(),
        Some("SID-2".into()),
        "about:blank".into(),
    ));
    ctx.conn.install_browser_context_fixture_for_test(bc);
    for (id, session) in [(1, "SID-1"), (2, "SID-2")] {
        ctx.process_async(json!({"id": id, "sessionId": session, "method": "Fetch.enable"}))
            .await;
        ctx.expect_result(id, json!({}), Some(session));
    }
    let old = start_request_pause(&mut ctx, 3, "SID-1", "http://example.test/old").await;
    let other = start_request_pause(&mut ctx, 4, "SID-2", "http://example.test/other").await;
    let current = start_request_pause(&mut ctx, 5, "SID-1", "http://example.test/current").await;
    let retired = take_response_by_id(&mut ctx, 3);
    assert_eq!(retired["sessionId"], "SID-1");
    assert_eq!(retired["result"]["errorText"], "net::ERR_ABORTED");
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["id"] == 4 || message["id"] == 5)
    );
    for (id, method, extra) in [
        (6, "Fetch.continueRequest", json!({})),
        (7, "Fetch.failRequest", json!({"errorReason": "Aborted"})),
        (8, "Fetch.fulfillRequest", json!({"responseCode": 200})),
    ] {
        let mut params = extra;
        params["requestId"] = json!(old);
        ctx.process_async(
            json!({"id": id, "sessionId": "SID-1", "method": method, "params": params}),
        )
        .await;
        let reply = take_response_by_id(&mut ctx, id);
        assert_eq!(reply["error"]["message"], "RequestNotFound");
        assert!(
            !ctx.sent.iter().any(|message| message["id"] == 3),
            "late actions cannot reply to the retired navigation twice"
        );
    }
    for (id, session, request, navigate_id) in [(9, "SID-1", current, 5), (10, "SID-2", other, 4)] {
        ctx.process_async(
            json!({"id": id, "sessionId": session, "method": "Fetch.failRequest",
            "params": {"requestId": request, "errorReason": "Aborted"}}),
        )
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
        assert_eq!(
            take_response_by_id(&mut ctx, navigate_id)["error"]["message"],
            "Aborted"
        );
        assert!(
            !ctx.conn
                .renderer_document_navigation_is_suspended_for_session_owner(Some(session))
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn supersession_retires_an_auth_pause_without_client_credentials() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/",
                get(|| async {
                    (
                        StatusCode::UNAUTHORIZED,
                        [(WWW_AUTHENTICATE, "Basic realm=\"supersession\"")],
                        "auth required",
                    )
                }),
            ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    ctx.conn
        .install_browser_context_fixture_for_test(attached_browser_context());
    ctx.process_async(json!({"id": 1, "sessionId": "SID-1", "method": "Fetch.enable", "params": {"handleAuthRequests": true}})).await;
    let old = start_request_pause(&mut ctx, 2, "SID-1", &format!("http://{addr}/")).await;
    ctx.process_async(json!({"id": 3, "sessionId": "SID-1", "method": "Fetch.continueRequest", "params": {"requestId": old}})).await;
    assert!(
        ctx.sent
            .iter()
            .any(|message| message["method"] == "Fetch.authRequired")
    );
    let current = start_request_pause(&mut ctx, 4, "SID-1", "http://example.test/current").await;
    assert_eq!(
        take_response_by_id(&mut ctx, 2)["result"]["errorText"],
        "net::ERR_ABORTED"
    );
    ctx.process_async(
        json!({"id": 5, "sessionId": "SID-1", "method": "Fetch.continueWithAuth",
        "params": {"requestId": old, "authChallengeResponse": {"response": "CancelAuth"}}}),
    )
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 5)["error"]["message"],
        "RequestNotFound"
    );
    ctx.process_async(
        json!({"id": 6, "sessionId": "SID-1", "method": "Fetch.failRequest",
        "params": {"requestId": current, "errorReason": "Aborted"}}),
    )
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 4)["error"]["message"],
        "Aborted"
    );
    assert!(
        !ctx.conn
            .renderer_document_navigation_is_suspended_for_session_owner(Some("SID-1"))
    );
    assert!(!ctx.sent.iter().any(|message| message["id"] == 2));
    server.abort();
}
