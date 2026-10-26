use super::*;

#[derive(Clone, Copy, Debug)]
enum Continuation {
    FulfillRequest,
    FulfillRequestWithTail,
    FulfillResponse,
    BufferedResponse,
    CapturedResponse,
    CancelAuthentication401,
    CancelAuthentication407,
    DigestAuthentication,
}

#[derive(Clone, Copy, Debug)]
enum Finish {
    Load,
    Replace,
    Disable,
}

#[tokio::test]
async fn websocket_cdp_captured_document_continuations_allow_peer_and_parser_continue() {
    for continuation in [
        Continuation::FulfillRequest,
        Continuation::FulfillResponse,
        Continuation::BufferedResponse,
        Continuation::CapturedResponse,
        Continuation::CancelAuthentication401,
        Continuation::CancelAuthentication407,
        Continuation::DigestAuthentication,
    ] {
        verify(continuation, Finish::Load, true).await;
    }
}

#[tokio::test]
async fn websocket_cdp_captured_document_wait_allows_replacement_before_script_release() {
    for continuation in [
        Continuation::FulfillRequest,
        Continuation::CapturedResponse,
        Continuation::CancelAuthentication401,
        Continuation::DigestAuthentication,
    ] {
        verify(continuation, Finish::Replace, false).await;
    }
}

#[tokio::test]
async fn websocket_cdp_captured_document_wait_is_revoked_by_fetch_disable() {
    for continuation in [
        Continuation::FulfillResponse,
        Continuation::BufferedResponse,
        Continuation::CancelAuthentication407,
        Continuation::DigestAuthentication,
    ] {
        verify(continuation, Finish::Disable, false).await;
    }
}

#[tokio::test]
async fn websocket_cdp_captured_document_network_prefix_survives_wait_and_adoption() {
    verify(Continuation::FulfillRequestWithTail, Finish::Load, false).await;
}

async fn verify(continuation: Continuation, finish: Finish, intercept_script: bool) {
    eprintln!("captured workflow {continuation:?}/{finish:?}: start");
    let buffered = matches!(continuation, Continuation::BufferedResponse);
    let digest = matches!(continuation, Continuation::DigestAuthentication);
    let auth_requested = Arc::new(tokio::sync::Notify::new());
    let release_auth = Arc::new(tokio::sync::Notify::new());
    let auth_requested_for_route = Arc::clone(&auth_requested);
    let release_auth_for_route = Arc::clone(&release_auth);
    let held_tail = matches!(continuation, Continuation::FulfillRequestWithTail);
    let tail_requested = Arc::new(tokio::sync::Notify::new());
    let release_tail = Arc::new(tokio::sync::Notify::new());
    let tail_requested_for_route = Arc::clone(&tail_requested);
    let release_tail_for_route = Arc::clone(&release_tail);
    let cancel_auth = matches!(
        continuation,
        Continuation::CancelAuthentication401 | Continuation::CancelAuthentication407
    );
    let challenge_continuation = cancel_auth || digest;
    let auth = buffered || challenge_continuation;
    let proxy_auth = matches!(continuation, Continuation::CancelAuthentication407);
    let requested = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&requested);
    let release_for_route = Arc::clone(&release);
    let fixture = Router::new()
        .route(
            "/page",
            get(move |headers: axum::http::HeaderMap| {
                let requested = Arc::clone(&auth_requested_for_route);
                let release = Arc::clone(&release_auth_for_route);
                async move {
                let authorized = digest && headers.get(axum::http::header::AUTHORIZATION)
                    .is_some_and(|value| value.as_bytes().starts_with(b"Digest "));
                if authorized {
                    requested.notify_one();
                    release.notified().await;
                }
                let mut response = (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><script src=/app.js></script><p>continued</p>",
                )
                    .into_response();
                if auth && !authorized {
                    *response.status_mut() = if proxy_auth {
                        axum::http::StatusCode::PROXY_AUTHENTICATION_REQUIRED
                    } else {
                        axum::http::StatusCode::UNAUTHORIZED
                    };
                    response.headers_mut().insert(
                        if proxy_auth {
                            axum::http::header::PROXY_AUTHENTICATE
                        } else {
                            axum::http::header::WWW_AUTHENTICATE
                        },
                        axum::http::HeaderValue::from_static(if digest {
                            "Digest realm=\"captured-response\", nonce=\"deadbeef\", qop=\"auth\", algorithm=MD5"
                        } else { "Basic realm=\"captured-response\"" }),
                    );
                }
                response
                }
            }),
        )
        .route(
            "/app.js",
            get(move || {
                let requested = Arc::clone(&requested_for_route);
                let release = Arc::clone(&release_for_route);
                async move {
                    requested.notify_one();
                    release.notified().await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                        "globalThis.capturedScript = true;",
                    )
                }
            }),
        )
        .route(
            "/tail.js",
            get(move || {
                let requested = Arc::clone(&tail_requested_for_route);
                let release = Arc::clone(&release_tail_for_route);
                async move {
                    requested.notify_one();
                    release.notified().await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                        "globalThis.capturedTail = true;",
                    )
                }
            }),
        )
        .route(
            "/replacement",
            get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><p>replacement</p>",
                )
            }),
        );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture, "captured-document-navigation");
    let tail = if held_tail {
        format!("<script src=http://{fixture_addr}/tail.js></script>")
    } else {
        String::new()
    };
    let html = format!(
        "<!doctype html><script src=http://{fixture_addr}/app.js></script>{tail}<p>continued</p>"
    );
    let url = format!("http://{fixture_addr}/page");
    let (addr, server) = spawn_test_protocol_server().await;
    let endpoint = format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}");
    let (mut socket, _) = connect_async(&endpoint).await.unwrap();
    let (mut peer, _) = connect_async(&endpoint).await.unwrap();
    let context = cdp_create_browser_context(&mut socket, 1).await;
    let session = cdp_create_attached_target(&mut socket, 2, &context).await;
    for (id, method) in [(23, "Runtime.enable"), (24, "Network.enable")] {
        send_cdp_command(
            &mut socket,
            id,
            method,
            Some(&session.session_id),
            json!({}),
        )
        .await;
    }
    send_cdp_command(
        &mut socket,
        4,
        "Page.enable",
        Some(&session.session_id),
        json!({}),
    )
    .await;
    let mut patterns = vec![json!({
        "urlPattern": "*page",
        "resourceType": "Document",
        "requestStage": if matches!(continuation, Continuation::FulfillRequest | Continuation::FulfillRequestWithTail) || challenge_continuation {
            "Request"
        } else {
            "Response"
        },
    })];
    if intercept_script {
        patterns.push(
            json!({"urlPattern": "*app.js", "resourceType": "Script", "requestStage": "Request"}),
        );
    }
    send_cdp_command(
        &mut socket,
        5,
        "Fetch.enable",
        Some(&session.session_id),
        json!({"patterns": patterns, "handleAuthRequests": auth}),
    )
    .await;
    send_cdp_command_without_wait(
        &mut socket,
        6,
        "Page.navigate",
        Some(&session.session_id),
        json!({"url": url}),
    )
    .await;
    let mut observed = recv_until_match(&mut socket, |message| {
        if buffered {
            message["method"] == "Fetch.authRequired"
        } else {
            message["method"] == "Fetch.requestPaused"
                && message["params"]["resourceType"] == "Document"
        }
    })
    .await;
    if challenge_continuation {
        let request_id = observed.last().unwrap()["params"]["requestId"].clone();
        observed.extend(
            send_cdp_command(
                &mut socket,
                22,
                "Fetch.continueRequest",
                Some(&session.session_id),
                json!({"requestId": request_id}),
            )
            .await,
        );
        if !observed
            .iter()
            .any(|message| message["method"] == "Fetch.authRequired")
        {
            observed.extend(
                recv_until_match(&mut socket, |message| {
                    message["method"] == "Fetch.authRequired"
                })
                .await,
            );
        }
    }
    if buffered {
        let auth_request_id = observed.last().unwrap()["params"]["requestId"].clone();
        observed.extend(send_cdp_command(
            &mut socket, 21, "Fetch.continueWithAuth", Some(&session.session_id),
            json!({"requestId": auth_request_id, "authChallengeResponse": {"response": "CancelAuth"}}),
        ).await);
        if !observed
            .iter()
            .any(|message| message["method"] == "Fetch.requestPaused")
        {
            observed.extend(
                recv_until_match(&mut socket, |message| {
                    message["method"] == "Fetch.requestPaused"
                })
                .await,
            );
        }
    }
    eprintln!("captured workflow {continuation:?}/{finish:?}: Document paused");
    let request_id = observed
        .iter()
        .find(|message| {
            message["method"]
                == if challenge_continuation {
                    "Fetch.authRequired"
                } else {
                    "Fetch.requestPaused"
                }
        })
        .unwrap()["params"]["requestId"]
        .clone();
    if matches!(continuation, Continuation::CapturedResponse) {
        let body = send_cdp_command(
            &mut socket,
            7,
            "Fetch.getResponseBody",
            Some(&session.session_id),
            json!({"requestId": request_id}),
        )
        .await;
        assert!(
            body.iter()
                .any(|message| message["id"] == 7 && message["result"]["body"].is_string()),
            "{body:?}"
        );
        observed.extend(body);
    }
    let (method, params) = match continuation {
        Continuation::FulfillRequest
        | Continuation::FulfillRequestWithTail
        | Continuation::FulfillResponse => (
            "Fetch.fulfillRequest",
            json!({
                "requestId": request_id,
                "responseCode": 200,
                "responseHeaders": [{"name": "Content-Type", "value": "text/html"}],
                "body": BASE64_STANDARD.encode(&html),
            }),
        ),
        Continuation::BufferedResponse | Continuation::CapturedResponse => {
            ("Fetch.continueResponse", json!({"requestId": request_id}))
        }
        Continuation::CancelAuthentication401 | Continuation::CancelAuthentication407 => (
            "Fetch.continueWithAuth",
            json!({"requestId": request_id, "authChallengeResponse": {"response": "CancelAuth"}}),
        ),
        Continuation::DigestAuthentication => (
            "Fetch.continueWithAuth",
            json!({"requestId": request_id, "authChallengeResponse": {
                "response": "ProvideCredentials", "username": "digest-user", "password": "digest-pass",
            }}),
        ),
    };
    send_cdp_command_without_wait(&mut socket, 8, method, Some(&session.session_id), params).await;
    if digest {
        timeout(Duration::from_secs(10), auth_requested.notified())
            .await
            .expect("Digest retry must reach the controlled network wait");
        let progress = timeout(
            Duration::from_secs(5),
            send_cdp_command(&mut peer, 30, "Target.getTargets", None, json!({})),
        )
        .await
        .expect("an independent frontend must progress while Digest's response is withheld");
        assert!(
            progress
                .iter()
                .any(|message| message["id"] == 30 && message["result"]["targetInfos"].is_array())
        );
        observed.extend(
            timeout(
                Duration::from_secs(5),
                send_cdp_command(&mut socket, 31, "Browser.getVersion", None, json!({})),
            )
            .await
            .expect("the source frontend must progress during Digest transport"),
        );
        assert!(
            !observed.iter().any(|message| message["id"] == 8),
            "Digest's captured continuation must not ACK before the response: {observed:?}"
        );
        release_auth.notify_one();
    }
    if intercept_script {
        let paused = recv_until_match(&mut socket, |message| {
            message["method"] == "Fetch.requestPaused"
                && message["params"]["resourceType"] == "Script"
        })
        .await;
        eprintln!("captured workflow {continuation:?}/{finish:?}: Script paused");
        let script_request_id = paused.last().unwrap()["params"]["requestId"].clone();
        let script_network_id = paused.last().unwrap()["params"]["networkId"].clone();
        observed.extend(paused);
        let request = observed
            .iter()
            .find(|message| {
                message["method"] == "Network.requestWillBeSent"
                    && message["params"]["requestId"] == script_network_id
            })
            .expect("Network must announce the parser request before its Fetch pause");
        let commit = observed
            .iter()
            .find(|message| message["method"] == "Page.frameNavigated")
            .expect("the admitted native Document must be visible before parser interception");
        assert_eq!(
            request["params"]["loaderId"],
            commit["params"]["frame"]["loaderId"]
        );
        observed.extend(
            send_cdp_command(
                &mut socket,
                9,
                "Fetch.continueRequest",
                Some(&session.session_id),
                json!({"requestId": script_request_id}),
            )
            .await,
        );
        assert!(
            observed
                .iter()
                .any(|message| message["id"] == 9 && message["result"] == json!({})),
            "{observed:?}"
        );
    }
    timeout(Duration::from_secs(10), requested.notified())
        .await
        .expect("native parser must reach the controlled script wait");
    eprintln!("captured workflow {continuation:?}/{finish:?}: Script waiting");
    let progress = timeout(
        Duration::from_secs(5),
        send_cdp_command(&mut peer, 1, "Target.getTargets", None, json!({})),
    )
    .await
    .expect("an independent frontend must progress before script release");
    assert!(
        progress
            .iter()
            .any(|message| message["id"] == 1 && message["result"]["targetInfos"].is_array())
    );
    observed.extend(
        timeout(
            Duration::from_secs(5),
            send_cdp_command(&mut socket, 20, "Browser.getVersion", None, json!({})),
        )
        .await
        .expect("the source frontend must also accept independent commands during native commit"),
    );
    match finish {
        Finish::Load => {
            // This captured/synthetic ACK keeps its existing commit boundary.
            // The streaming ACK-before-EOF contract has a separate regression.
            assert!(
                !observed.iter().any(|message| message["id"] == 8),
                "continuation ACK preceded its commit: {observed:?}"
            );
            release.notify_one();
            if held_tail {
                timeout(Duration::from_secs(10), tail_requested.notified())
                    .await
                    .expect("the second parser script must hold native commit");
                observed.extend(timeout(Duration::from_secs(5), recv_until_match(&mut socket, |message| {
                    message["method"] == "Network.responseReceived" && message["params"]["response"]["url"].as_str().is_some_and(|url| url.ends_with("/app.js"))
                })).await.expect("the first request's native output must publish while the second script still blocks adoption"));
                assert!(
                    !observed.iter().any(|message| message["id"] == 8),
                    "the controlled native commit must still be waiting"
                );
                release_tail.notify_one();
            }
        }
        Finish::Replace => {
            send_cdp_command_without_wait(
                &mut socket,
                10,
                "Page.navigate",
                Some(&session.session_id),
                json!({"url": format!("http://{fixture_addr}/replacement")}),
            )
            .await;
        }
        Finish::Disable => {
            observed.extend(
                timeout(
                    Duration::from_secs(5),
                    send_cdp_command(
                        &mut socket,
                        10,
                        "Fetch.disable",
                        Some(&session.session_id),
                        json!({}),
                    ),
                )
                .await
                .expect("Fetch.disable must revoke the captured workflow before script release"),
            );
        }
    }
    let mut expected_ids = vec![6, 8];
    if matches!(finish, Finish::Replace) {
        expected_ids.push(10);
    }
    while !expected_ids
        .iter()
        .all(|id| observed.iter().any(|message| message["id"] == *id))
    {
        let next = recv_until_match(&mut socket, |message| message["id"].is_number()).await;
        observed.extend(next);
    }
    for id in expected_ids {
        assert_eq!(
            observed
                .iter()
                .filter(|message| message["id"] == id)
                .count(),
            1,
            "{continuation:?}/{finish:?}: {observed:?}"
        );
    }
    match finish {
        Finish::Load => {
            assert!(
                observed
                    .iter()
                    .any(|message| message["id"] == 8 && message["result"] == json!({})),
                "{observed:?}"
            );
            assert_eq!(
                cdp_runtime_evaluate_string(
                    &mut socket,
                    &session.session_id,
                    11,
                    "JSON.stringify([capturedScript, document.querySelector('p').textContent])",
                )
                .await,
                r#"[true,"continued"]"#
            );
            if !observed.iter().any(|message| {
                message["method"] == "Network.responseReceived"
                    && message["params"]["response"]["url"]
                        .as_str()
                        .is_some_and(|url| url.ends_with("/app.js"))
            }) {
                observed.extend(
                    recv_until_match(&mut socket, |message| {
                        message["method"] == "Network.responseReceived"
                            && message["params"]["response"]["url"]
                                .as_str()
                                .is_some_and(|url| url.ends_with("/app.js"))
                    })
                    .await,
                );
            }
            let network_id = observed
                .iter()
                .find(|message| {
                    message["method"] == "Network.responseReceived"
                        && message["params"]["response"]["url"]
                            .as_str()
                            .is_some_and(|url| url.ends_with("/app.js"))
                })
                .unwrap()["params"]["requestId"]
                .clone();
            if !observed.iter().any(|message| {
                message["method"] == "Network.loadingFinished"
                    && message["params"]["requestId"] == network_id
            }) {
                observed.extend(
                    recv_until_match(&mut socket, |message| {
                        message["method"] == "Network.loadingFinished"
                            && message["params"]["requestId"] == network_id
                    })
                    .await,
                );
            }
            let body = send_cdp_command(
                &mut socket,
                25,
                "Network.getResponseBody",
                Some(&session.session_id),
                json!({"requestId": network_id}),
            )
            .await;
            assert!(
                body.iter().any(|message| message["id"] == 25
                    && message["result"]["body"]
                        .as_str()
                        .is_some_and(|body| body.contains("capturedScript"))),
                "{body:?}"
            );
            for method in [
                "Network.requestWillBeSent",
                "Network.responseReceived",
                "Network.loadingFinished",
            ] {
                assert_eq!(
                    observed
                        .iter()
                        .filter(|message| message["method"] == method
                            && message["params"]["requestId"] == network_id)
                        .count(),
                    1,
                    "{method} must publish once: {observed:?}"
                );
            }
        }
        Finish::Replace => {
            assert_eq!(
                cdp_runtime_evaluate_string(
                    &mut socket,
                    &session.session_id,
                    11,
                    "document.querySelector('p').textContent",
                )
                .await,
                "replacement"
            );
            release.notify_one();
            assert_eq!(
                cdp_runtime_evaluate_string(
                    &mut socket,
                    &session.session_id,
                    12,
                    "document.querySelector('p').textContent",
                )
                .await,
                "replacement"
            );
        }
        Finish::Disable => {
            release.notify_one();
        }
    }
    let _ = peer.close(None).await;
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}
