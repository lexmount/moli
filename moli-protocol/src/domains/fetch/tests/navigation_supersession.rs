use super::*;

#[tokio::test]
async fn fulfill_document_protocol_tail_rejoins_after_wait_cancellation_and_ready_loss() {
    use crate::conn::{CdpCommandTaskStep, CommandDispatchContext, ParsedCdpCommand};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use futures_util::FutureExt as _;

    tokio::task::LocalSet::new().run_until(async {
        for cancel_phase in 0..3 {
            let mut ctx = TestContext::new();
            ctx.conn.install_browser_context_fixture_for_test(attached_browser_context());
            ctx.enable_page_events_for_test(Some("SID-1"));
            ctx.process_async(json!({"id":1,"sessionId":"SID-1","method":"Fetch.enable"})).await;
            ctx.expect_result(1, json!({}), Some("SID-1"));
            let request_id = start_request_pause(&mut ctx, 2, "SID-1", "http://example.test/owned-tail").await;
            let (sender, mut completions) = tokio::sync::mpsc::unbounded_channel();
            ctx.conn.set_background_navigation_completion_sender(sender);
            let mut requests = ctx.conn.take_protocol_state_requests().unwrap();
            let request = ParsedCdpCommand::from_serializable(json!({
                "id":3,"sessionId":"SID-1","method":"Fetch.fulfillRequest",
                "params":{"requestId":request_id,"responseCode":200,
                    "responseHeaders":[{"name":"Content-Type","value":"text/html"}],
                    "body": STANDARD.encode("<title>owned tail</title><p>fulfilled</p>")}
            })).unwrap();
            let mut context = CommandDispatchContext::default();
            let CdpCommandTaskStep::Pending(native) = ctx.conn.start_parsed_command_dispatch_with_context(&request, &mut context) else {
                panic!("fulfillment prepares and commits outside protocol state");
            };
            let mut native = Box::pin(native.wait());
            let native = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    tokio::select! {
                        completed = &mut native => break completed,
                        request = requests.recv() => request.unwrap().apply(&mut ctx.conn),
                    }
                }
            }).await.unwrap();
            let Some(CdpCommandTaskStep::Pending(tail)) = Box::pin(ctx.conn.complete_pending_command_dispatch_with_context(native, &mut context)).now_or_never() else {
                panic!("the protocol tail hands its waits back to the actor");
            };
            let mut wait = Box::pin(tail.wait());
            if cancel_phase == 1 {
                assert!(wait.as_mut().now_or_never().is_none());
                requests.recv().await.unwrap().apply(&mut ctx.conn);
                assert!(wait.as_mut().now_or_never().is_none());
            }
            if cancel_phase == 2 {
                let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    loop {
                        tokio::select! {
                            completed = &mut wait => break completed,
                            request = requests.recv() => request.unwrap().apply(&mut ctx.conn),
                        }
                    }
                }).await.unwrap();
                drop(completed);
            }
            drop(wait);
            let CdpCommandTaskStep::Pending(recovery) = ctx.conn.complete_failed_command_wait_with_context(
                Some(3), Some("SID-1"), "controlled fulfillment waiter cancellation", &mut context,
            ) else { panic!("the original command resumes its admitted protocol tail") };
            let mut recovery = Box::pin(recovery.wait());
            let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    tokio::select! {
                        completed = &mut recovery => break completed,
                        request = requests.recv() => request.unwrap().apply(&mut ctx.conn),
                    }
                }
            }).await.unwrap();
            let CdpCommandTaskStep::Complete(outcome) = ctx.conn.try_complete_ready_command_dispatch_with_context(completed, &mut context)
                .unwrap_or_else(|_| panic!("publication is finite")) else { panic!("no wait remains") };
            let (mut messages, scheduler_events) = ctx.route_completed_command_outcome_for_test(outcome).await;
            let sent_start = ctx.sent.len();
            ctx.route_direct_command_output_for_test(Vec::new(), scheduler_events).await;
            if !messages.iter().any(|message| message["method"] == "Page.frameStoppedLoading") {
                wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
            }
            messages.extend(ctx.sent.drain(sent_start..));
            for id in [2, 3] {
                assert_eq!(messages.iter().filter(|message| message["id"] == id && message.get("result").is_some()).count(), 1, "command {id}: {messages:?}");
            }
            for method in ["Page.domContentEventFired", "Page.loadEventFired"] {
                assert_eq!(messages.iter().filter(|message| message["method"] == method).count(), 1, "{method}: {messages:?}");
            }
            assert!(completions.try_recv().is_err(), "completed source navigation must not receive cancellation or a second terminal");
        }
    }).await;
}

#[tokio::test]
async fn disable_cleanup_reports_policy_failure_after_settling_its_navigation() {
    let mut ctx = TestContext::new();
    ctx.conn
        .install_browser_context_fixture_for_test(attached_browser_context());
    ctx.install_navigation_fixture_for_session_owner("about:blank", Some("SID-1"))
        .await;
    ctx.process_async(json!({"id": 1, "sessionId": "SID-1", "method": "Fetch.enable"}))
        .await;
    start_request_pause(&mut ctx, 2, "SID-1", "http://example.test/paused").await;
    let (sender, mut completions) = tokio::sync::mpsc::unbounded_channel();
    ctx.conn.set_background_navigation_completion_sender(sender);
    let (pending, policy) = ctx
        .conn
        .start_disable_fetch_for_session_owner(Some("SID-1"))
        .unwrap();
    drop(policy);
    let owner = crate::conn::CommandOwnerScope::capture(&ctx.conn, Some("SID-1"));
    // The handler and its requests have already been removed. A failed native
    // policy update must still drain the original source obligation.
    let cleanup = crate::domains::fetch::cleanup::PreparedFetchCleanup::new(
        &mut ctx.conn,
        &owner,
        pending,
        Some(Err(
            std::io::Error::other("controlled policy rejection").into()
        )),
        crate::domains::fetch::cleanup::FetchCleanupReason::Disabled,
        false,
    );
    let plan = cleanup.wait_inline(&mut ctx.conn).await.into_command_plan();
    let messages = plan
        .into_background_events(Some(3), Some("SID-1"))
        .into_iter()
        .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
        .collect::<Vec<_>>();
    assert_eq!(
        messages.iter().filter(|message| message["id"] == 2).count(),
        1
    );
    assert_eq!(
        messages.iter().find(|message| message["id"] == 2).unwrap()["error"]["message"],
        "Fetch interception disabled"
    );
    assert!(
        messages.iter().find(|message| message["id"] == 3).unwrap()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("controlled policy rejection")
    );
    assert!(
        completions.try_recv().is_err(),
        "consumed cleanup must disarm its cancellation reply"
    );
}

#[tokio::test]
async fn cancelled_disable_cleanup_settles_once_without_touching_a_successor_page() {
    tokio::task::LocalSet::new()
        .run_until(async {
            for before_wait in [false, true] {
                let mut ctx = TestContext::new();
                ctx.conn
                    .install_browser_context_fixture_for_test(attached_browser_context());
                ctx.install_navigation_fixture_for_session_owner("about:blank", Some("SID-1"))
                    .await;
                ctx.process_async(json!({"id": 1, "sessionId": "SID-1", "method": "Fetch.enable"}))
                    .await;
                start_request_pause(&mut ctx, 2, "SID-1", "http://example.test/paused").await;
                let (sender, mut completions) = tokio::sync::mpsc::unbounded_channel();
                ctx.conn.set_background_navigation_completion_sender(sender);
                let _requests = ctx.conn.take_protocol_state_requests().unwrap();
                let (pending, policy) = ctx
                    .conn
                    .start_disable_fetch_for_session_owner(Some("SID-1"))
                    .unwrap();
                let owner = crate::conn::CommandOwnerScope::capture(&ctx.conn, Some("SID-1"));
                let cleanup = crate::domains::fetch::cleanup::PreparedFetchCleanup::new(
                    &mut ctx.conn,
                    &owner,
                    pending,
                    policy,
                    crate::domains::fetch::cleanup::FetchCleanupReason::Disabled,
                    false,
                );
                if before_wait {
                    drop(cleanup);
                } else {
                    let completed = cleanup.wait().await;
                    assert!(matches!(
                        completed,
                        crate::domains::fetch::cleanup::CompletedFetchCleanup::Owned(_)
                    ));
                    drop(completed);
                }
                let completion =
                    tokio::time::timeout(std::time::Duration::from_secs(5), completions.recv())
                        .await
                        .unwrap()
                        .unwrap();
                ctx.install_navigation_fixture_for_session_owner("about:blank", Some("SID-1"))
                    .await;
                let successor = ctx
                    .conn
                    .renderer_page_residence_identity_for_owner(&owner)
                    .unwrap();
                let outcome = ctx
                    .conn
                    .drain_background_navigation_completion_turn_async(completion)
                    .await;
                let (messages, _) = ctx.route_completed_command_outcome_for_test(outcome).await;
                assert_eq!(
                    messages.iter().filter(|message| message["id"] == 2).count(),
                    1
                );
                assert_eq!(
                    ctx.conn.renderer_page_residence_identity_for_owner(&owner),
                    Some(successor)
                );
                assert!(
                    completions.try_recv().is_err(),
                    "late cleanup must not emit a second source result"
                );
            }
        })
        .await;
}

#[tokio::test]
async fn dropping_a_completed_response_handoff_does_not_cancel_its_registered_pause() {
    use crate::domains::fetch::{FetchCommandTaskStep, try_start_fetch_command_dispatch};

    tokio::task::LocalSet::new().run_until(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new()
                .route("/auth", get(|| async {
                    (StatusCode::UNAUTHORIZED, [(WWW_AUTHENTICATE, "Basic realm=\"handoff\"")], "original challenge")
                }))
                .route("/response", get(|| async { "original response" })))
                .await.unwrap();
        });
        for response_stage in [false, true] {
            let mut ctx = TestContext::new();
            ctx.conn.install_browser_context_fixture_for_test(attached_browser_context());
            let (sender, mut completions) = tokio::sync::mpsc::unbounded_channel();
            ctx.conn.set_background_navigation_completion_sender(sender);
            ctx.process_async(json!({"id": 1, "sessionId": "SID-1", "method": "Fetch.enable",
                "params": {"handleAuthRequests": true, "patterns": [{"urlPattern": "*", "requestStage": "Request"}]}})).await;
            let path = if response_stage { "response" } else { "auth" };
            let request_id = start_request_pause(&mut ctx, 2, "SID-1", &format!("http://{addr}/{path}")).await;
            let mut requests = ctx.conn.take_protocol_state_requests().unwrap();
            let command = moli_protocol_cdp::ParsedCdpCommand::from_serializable(json!({
                "id": 3, "sessionId": "SID-1", "method": "Fetch.continueRequest",
                "params": {"requestId": request_id, "interceptResponse": response_stage},
            })).unwrap();
            let cmd = crate::conn::Cmd::from_parsed(&command).unwrap();
            let Some(FetchCommandTaskStep::Pending(pending)) = try_start_fetch_command_dispatch(&mut ctx.conn, &cmd)
            else { panic!("the request must leave protocol state while waiting for headers"); };
            let wait = tokio::task::spawn_local(pending.wait());
            let handoff = tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
                .await.unwrap().unwrap();
            handoff.apply(&mut ctx.conn);
            // This is the cancellation window between finite response admission
            // and the actor consuming a ready command result. The pause already
            // owns the original navigation's response obligation.
            drop(wait.await.unwrap());
            assert!(completions.try_recv().is_err(), "handoff drop must not synthesize a second source result");
            let host = ctx.conn.browser_context.as_ref().unwrap().active_page_target();
            assert!(host.fetch_owner.has_pending_fetch_request_id_for_test(&request_id));
            ctx.process_async(json!({"id": 4, "sessionId": "SID-1", "method": "Fetch.disable"})).await;
            let responses = ctx.sent.iter().filter(|message| message["id"] == 2).count();
            assert_eq!(responses, 1, "the registered pause must retain the sole navigation result");
            assert!(completions.try_recv().is_err());
        }
        server.abort();
    }).await;
}

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
            complete_pending_fetch_command(&mut ctx.conn, completed, &mut Default::default()).await
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
