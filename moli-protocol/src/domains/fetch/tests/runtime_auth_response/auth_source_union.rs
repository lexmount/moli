use super::*;
use crate::automation::DevToolsRemoveNetworkInterceptCommand;
use axum::extract::State;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[tokio::test(flavor = "multi_thread")]
async fn cdp_auth_survives_unrelated_network_request_intercept() {
    for network_first in [false, true] {
        run_auth_source_union(
            DevToolsNetworkInterceptPhase::BeforeRequestSent,
            network_first,
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cdp_auth_survives_nonmatching_network_auth_intercept() {
    for network_first in [false, true] {
        run_auth_source_union(DevToolsNetworkInterceptPhase::AuthRequired, network_first).await;
    }
}

async fn configure_cdp_auth(ctx: &mut TestContext, id: u64, enabled: bool) {
    ctx.process_async(json!({
        "id": id,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": {
            "handleAuthRequests": enabled,
            "patterns": [{"resourceType": "Image"}]
        }
    }))
    .await;
    ctx.expect_result(id, json!({}), Some("SID-1"));
}

async fn run_auth_source_union(phase: DevToolsNetworkInterceptPhase, network_first: bool) {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>auth union</title>",
        )
    }

    async fn protected(
        State(hits): State<Arc<AtomicUsize>>,
        uri: axum::http::Uri,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        hits.fetch_add(1, Ordering::SeqCst);
        // Different credentials per hop ensure that an earlier successful
        // authentication cannot satisfy a later request from an auth cache.
        let password = format!("pass:{}", uri.path());
        let expected = format!(
            "Basic {}",
            super::super::encode_basic_auth("user", &password)
        );
        if headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some(&expected)
        {
            (
                StatusCode::OK,
                [(CONTENT_TYPE.as_str(), "text/plain")],
                "secret xhr",
            )
                .into_response()
        } else {
            (
                StatusCode::UNAUTHORIZED,
                [(
                    WWW_AUTHENTICATE.as_str(),
                    format!("Basic realm=\"{}\"", uri.path()),
                )],
                "auth required",
            )
                .into_response()
        }
    }

    let hits = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/page", get(page))
        .route("/protected/{stage}", any(protected))
        .with_state(hits.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("http://{addr}/page"), "SID-1", "TID-1").await;
    ctx.process_async(json!({"id": 80_000, "method": "Network.enable", "sessionId": "SID-1"}))
        .await;
    ctx.expect_result(80_000, json!({}), Some("SID-1"));
    enable_runtime_async(&mut ctx, "SID-1", 80_001).await;
    let network_context = AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("BIDI-SID")),
        target_id: Some(DevToolsTargetId::from("TID-1")),
        browser_context_id: None,
    };
    if !network_first {
        configure_cdp_auth(&mut ctx, 80_002, true).await;
    }
    let added = ctx
        .execute_automation_command_through_renderer_fence_for_test(
            AutomationCommand::AddNetworkIntercept(DevToolsAddNetworkInterceptCommand {
                context: network_context.clone(),
                intercept_id: DevToolsNetworkInterceptId::from("unrelated-intercept"),
                phases: vec![phase],
                url_patterns: vec![DevToolsNetworkInterceptPattern {
                    url_pattern: format!("http://{addr}/unrelated*"),
                }],
            }),
        )
        .await;
    assert!(matches!(
        added.unwrap(),
        AutomationResult::AddNetworkIntercept(_)
    ));
    if network_first {
        configure_cdp_auth(&mut ctx, 80_002, true).await;
    }

    for stage in 0..3 {
        if stage == 1 {
            let removed = ctx
                .execute_automation_command_through_renderer_fence_for_test(
                    AutomationCommand::RemoveNetworkIntercept(
                        DevToolsRemoveNetworkInterceptCommand {
                            context: network_context.clone(),
                            intercept_id: DevToolsNetworkInterceptId::from("unrelated-intercept"),
                        },
                    ),
                )
                .await;
            assert_eq!(removed.unwrap(), AutomationResult::Empty);
        } else if stage == 2 {
            configure_cdp_auth(&mut ctx, 80_003, false).await;
        }
        ctx.sent.clear();
        let url = format!("http://{addr}/protected/{stage}");
        let eval_id = 80_010 + stage * 10;
        ctx.process_async(json!({
            "id": eval_id, "method": "Runtime.evaluate", "sessionId": "SID-1",
            "params": {"expression": format!(r#"(() => {{
                globalThis.authUnionResult = 'pending';
                const xhr = new XMLHttpRequest(); xhr.open('GET', {url:?});
                xhr.onload = () => {{ authUnionResult = xhr.status + ':' + xhr.responseText; }};
                xhr.onerror = () => {{ authUnionResult = 'xhr error'; }};
                xhr.send(); return 'scheduled';
            }})()"#)}
        }))
        .await;
        assert_eq!(
            take_response_by_id(&mut ctx, eval_id)["result"]["result"]["value"],
            "scheduled"
        );
        wait_until_message(&mut ctx, "SID-1", "auth union Network request", |message| {
            message["method"] == "Network.requestWillBeSent"
                && message["params"]["request"]["url"] == url
        })
        .await;
        let network_id = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == "Network.requestWillBeSent"
                    && message["params"]["request"]["url"] == url
            })
            .unwrap()["params"]["requestId"]
            .as_str()
            .unwrap()
            .to_owned();

        if stage < 2 {
            wait_until_message(
                &mut ctx,
                "SID-1",
                "CDP auth despite unrelated network rule",
                |message| {
                    message["method"] == "Fetch.authRequired"
                        && message["params"]["request"]["url"] == url
                },
            )
            .await;
            let auth = ctx
                .sent
                .iter()
                .find(|message| {
                    message["method"] == "Fetch.authRequired"
                        && message["params"]["request"]["url"] == url
                })
                .unwrap()
                .clone();
            assert_eq!(auth["sessionId"], "SID-1");
            assert_eq!(auth["params"]["resourceType"], "XHR");
            assert_eq!(
                auth["params"]["authChallenge"]["realm"],
                format!("/protected/{stage}")
            );
            ctx.process_async(json!({
                "id": eval_id + 1, "method": "Fetch.continueWithAuth", "sessionId": "SID-1",
                "params": {"requestId": auth["params"]["requestId"], "authChallengeResponse": {
                    "response": "ProvideCredentials", "username": "user", "password": format!("pass:/protected/{stage}")
                }}
            })).await;
            ctx.expect_result(eval_id + 1, json!({}), Some("SID-1"));
        }
        wait_for_network_loading_finished(&mut ctx, "SID-1", &network_id, "auth union completion")
            .await;
        let expected = if stage < 2 {
            json!("200:secret xhr")
        } else {
            json!("401:auth required")
        };
        let resolved = evaluate_until_value_async(
            &mut ctx,
            "SID-1",
            eval_id + 2,
            "globalThis.authUnionResult",
            &expected,
            "XHR auth result",
        )
        .await;
        assert_eq!(resolved["result"]["result"]["value"], expected);
        let challenges = ctx
            .sent
            .iter()
            .filter(|message| {
                message["method"] == "Fetch.authRequired"
                    && message["params"]["request"]["url"] == url
            })
            .collect::<Vec<_>>();
        assert_eq!(challenges.len(), usize::from(stage < 2));
        assert!(
            challenges
                .iter()
                .all(|message| message["sessionId"] == "SID-1")
        );
        assert!(
            ctx.sent.iter().all(|message| {
                message["method"] != "Fetch.requestPaused"
                    || message["params"]["request"]["url"] != url
            }),
            "auth-only XHR must not gain an Image request-stage pause"
        );
        assert_eq!(
            network_events_for_request(&ctx, "Network.loadingFinished", &network_id).len(),
            1
        );
        assert!(network_events_for_request(&ctx, "Network.loadingFailed", &network_id).is_empty());
        assert_eq!(
            hits.load(Ordering::SeqCst),
            if stage < 2 { (stage + 1) * 2 } else { 5 } as usize
        );
    }
    server.abort();
}
