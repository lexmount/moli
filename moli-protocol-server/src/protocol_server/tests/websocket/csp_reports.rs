use super::*;

#[tokio::test]
async fn websocket_cdp_csp_report_pause_preserves_one_request_through_decisions() {
    let (requested, mut request_seen) = tokio::sync::watch::channel(false);
    let (release, response_ready) = tokio::sync::watch::channel(false);
    let fixture = Router::new()
        .route(
            "/page",
            get(|| async {
                (
                    [
                        ("Content-Type", "text/html"),
                        (
                            "Content-Security-Policy",
                            "connect-src 'none'; report-uri /report",
                        ),
                    ],
                    "<!doctype html><title>CSP report</title>",
                )
            }),
        )
        .route(
            "/report",
            post(move |headers: axum::http::HeaderMap, body: String| {
                let requested = requested.clone();
                let mut response_ready = response_ready.clone();
                async move {
                    let report: serde_json::Value = serde_json::from_str(&body).unwrap();
                    assert_eq!(report["csp-report"]["effective-directive"], "connect-src");
                    if headers.get("x-hold").is_some() {
                        requested.send_replace(true);
                        response_ready.wait_for(|ready| *ready).await.unwrap();
                    }
                    if headers.get("x-challenge").is_some()
                        && headers.get("authorization").is_none()
                    {
                        (
                            StatusCode::UNAUTHORIZED,
                            [("WWW-Authenticate", "Basic realm=\"report\"")],
                            "challenge",
                        )
                            .into_response()
                    } else {
                        if headers.get("x-challenge").is_some() {
                            assert_eq!(headers["authorization"], "Basic dXNlcjpwYXNz");
                        }
                        ([("Content-Type", "text/plain")], "physical").into_response()
                    }
                }
            }),
        );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture, "csp-report-decisions");
    let origin = format!("http://{fixture_addr}");
    let report_url = format!("{origin}/report");
    let (cdp_addr, server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!(
        "ws://{cdp_addr}/devtools/browser/{DEFAULT_BROWSER_ID}"
    ))
    .await
    .unwrap();
    let session = cdp_create_session_and_navigate(&mut socket, &format!("{origin}/page")).await;
    for (id, method, params) in [
        (10, "Network.enable", json!({})),
        (
            11,
            "Fetch.enable",
            json!({"patterns": [{"urlPattern": report_url}], "handleAuthRequests": true}),
        ),
    ] {
        socket
            .send(WsMessage::Text(
                json!({"id": id, "method": method, "sessionId": session, "params": params})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        assert!(
            recv_until_id(&mut socket, id)
                .await
                .last()
                .unwrap()
                .get("error")
                .is_none()
        );
    }
    for decision in [
        "continue",
        "request_fulfill",
        "request_fail",
        "response_fulfill",
        "response_fail",
        "auth",
        "document_open",
    ] {
        socket
            .send(WsMessage::Text(
                json!({"id": 20, "method": "Runtime.evaluate", "sessionId": session,
            "params": {"expression": "fetch('/blocked').catch(()=>{});void 0"}})
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let mut messages = recv_until_match(&mut socket, |message| {
            message["method"] == "Fetch.requestPaused"
                && message["params"]["request"]["url"] == report_url
        })
        .await;
        let pause = messages.last().unwrap()["params"].clone();
        let request = &pause["requestId"];
        let network = &pause["networkId"];
        let starts: Vec<_> = messages
            .iter()
            .filter(|message| {
                message["method"] == "Network.requestWillBeSent"
                    && message["params"]["request"]["url"] == report_url
            })
            .collect();
        assert_eq!(
            starts.len(),
            1,
            "one native request before its pause: {messages:?}"
        );
        assert_eq!(&starts[0]["params"]["requestId"], network);
        let failed = decision.ends_with("fail");
        let fulfilled = decision.ends_with("fulfill");
        let response_paused = decision.starts_with("response");
        let (method, params) = if decision == "request_fail" {
            (
                "Fetch.failRequest",
                json!({"requestId": request, "errorReason": "BlockedByClient"}),
            )
        } else if decision == "request_fulfill" {
            (
                "Fetch.fulfillRequest",
                json!({"requestId": request, "responseCode": 201, "body": BASE64_STANDARD.encode("replaced")}),
            )
        } else {
            let mut params = json!({"requestId": request, "interceptResponse": response_paused});
            if decision == "auth" {
                params["headers"] = json!([{"name": "Content-Type", "value": "application/csp-report"}, {"name": "X-Challenge", "value": "1"}]);
            } else if decision == "document_open" {
                params["headers"] = json!([{"name": "Content-Type", "value": "application/csp-report"}, {"name": "X-Hold", "value": "1"}]);
            }
            ("Fetch.continueRequest", params)
        };
        socket
            .send(WsMessage::Text(
                json!({"id": 21, "method": method, "sessionId": session, "params": params})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        if decision == "document_open" {
            timeout(Duration::from_secs(5), request_seen.wait_for(|seen| *seen))
                .await
                .unwrap()
                .unwrap();
            socket.send(WsMessage::Text(json!({"id": 22, "method": "Runtime.evaluate", "sessionId": session,
                "params": {"expression": "document.open();document.write('<!doctype html><title>replacement</title>');document.close();void 0"}}).to_string().into())).await.unwrap();
            messages.extend(recv_until_id(&mut socket, 22).await);
            assert!(messages.last().unwrap().get("error").is_none());
            release.send_replace(true);
        }
        if decision == "auth" {
            messages.extend(
                recv_until_match(&mut socket, |message| {
                    message["method"] == "Fetch.authRequired"
                        && &message["params"]["requestId"] == request
                })
                .await,
            );
            assert_eq!(
                messages.last().unwrap()["params"]["authChallenge"]["realm"],
                "report"
            );
            socket.send(WsMessage::Text(json!({"id": 22, "method": "Fetch.continueWithAuth", "sessionId": session,
                "params": {"requestId": request, "authChallengeResponse": {"response": "ProvideCredentials", "username": "user", "password": "pass"}}}).to_string().into())).await.unwrap();
        }
        if response_paused {
            messages.extend(
                recv_until_match(&mut socket, |message| {
                    message["method"] == "Fetch.requestPaused"
                        && &message["params"]["requestId"] == request
                        && message["params"]["responseStatusCode"].is_number()
                })
                .await,
            );
            assert_eq!(
                messages.last().unwrap()["params"]["responseStatusCode"],
                200
            );
            assert!(
                !messages
                    .iter()
                    .any(|message| &message["params"]["requestId"] == network
                        && matches!(
                            message["method"].as_str(),
                            Some("Network.loadingFinished" | "Network.loadingFailed")
                        ))
            );
            let (method, params) = if failed {
                (
                    "Fetch.failRequest",
                    json!({"requestId": request, "errorReason": "BlockedByClient"}),
                )
            } else {
                (
                    "Fetch.fulfillRequest",
                    json!({"requestId": request, "responseCode": 201, "body": BASE64_STANDARD.encode("replaced")}),
                )
            };
            socket
                .send(WsMessage::Text(
                    json!({"id": 23, "method": method, "sessionId": session, "params": params})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        }
        let terminal = if failed {
            "Network.loadingFailed"
        } else {
            "Network.loadingFinished"
        };
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == terminal && &message["params"]["requestId"] == network
            })
            .await,
        );
        if !failed {
            socket.send(WsMessage::Text(json!({"id": 24, "method": "Network.getResponseBody", "sessionId": session, "params": {"requestId": network}}).to_string().into())).await.unwrap();
            messages.extend(recv_until_id(&mut socket, 24).await);
            let result = &messages.last().unwrap()["result"];
            let body = result["body"].as_str().unwrap();
            let body = if result["base64Encoded"] == true {
                BASE64_STANDARD.decode(body).unwrap()
            } else {
                body.as_bytes().to_vec()
            };
            assert_eq!(
                body,
                if fulfilled {
                    b"replaced".as_slice()
                } else {
                    b"physical".as_slice()
                }
            );
        }
        socket.send(WsMessage::Text(json!({"id": 25, "method": "Runtime.evaluate", "sessionId": session, "params": {"expression": "1"}}).to_string().into())).await.unwrap();
        messages.extend(recv_until_id(&mut socket, 25).await);
        let request_events: Vec<_> = messages
            .iter()
            .filter(|message| &message["params"]["requestId"] == network)
            .collect();
        assert_eq!(
            request_events
                .iter()
                .filter(|message| message["method"] == "Network.requestWillBeSent")
                .count(),
            1,
            "{decision}: {messages:?}"
        );
        assert_eq!(
            request_events
                .iter()
                .filter(|message| matches!(
                    message["method"].as_str(),
                    Some("Network.loadingFinished" | "Network.loadingFailed")
                ))
                .count(),
            1,
            "{decision}: {messages:?}"
        );
    }
    let _ = socket.close(None).await;
    abort_test_cdp_server(server).await;
}
