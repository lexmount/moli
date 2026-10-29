const MAX_NAVIGATION_AUTH_RETRIES: u8 = moli_fetch::AuthRetryBudget::MAX_RESTARTS;
use super::*;

#[derive(Clone, Copy, Debug)]
enum AuthEndpoint {
    Server,
    HttpProxy,
    ConnectProxy,
}

async fn begin_navigation(ctx: &mut TestContext, url: &str, intercept_response: bool) -> String {
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 2000,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;
    wait_until_message(ctx, "SID-1", "main-document request pause", |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["params"]["request"]["url"] == json!(url)
            && message["params"].get("responseStatusCode").is_none()
    })
    .await;
    let paused = ctx.take_first_matching("main-document request pause", |message| {
        message["method"] == json!("Fetch.requestPaused")
            && message["params"]["request"]["url"] == json!(url)
            && message["params"].get("responseStatusCode").is_none()
    });
    let network_id = paused["params"]["networkId"].as_str().unwrap().to_owned();
    ctx.process_async(json!({
        "id": 2001,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": {
            "requestId": paused["params"]["requestId"],
            "interceptResponse": intercept_response
        }
    }))
    .await;
    ctx.expect_result(2001, json!({}), Some("SID-1"));
    network_id
}

async fn answer_auth(ctx: &mut TestContext, chained: bool, password: Option<&str>) -> String {
    let mut auth = ctx.take_first_matching("navigation auth challenge", |message| {
        message["method"] == json!("Fetch.authRequired")
    });
    assert_eq!(auth["sessionId"], "SID-1");
    if chained {
        ctx.process_async(json!({
            "id": 2002,
            "method": "Fetch.continueWithAuth",
            "sessionId": "SID-1",
            "params": {
                "requestId": auth["params"]["requestId"],
                "authChallengeResponse": { "response": "Default" }
            }
        }))
        .await;
        ctx.expect_result(2002, json!({}), Some("SID-1"));
        let next = ctx.take_first_matching("chained navigation auth challenge", |message| {
            message["method"] == json!("Fetch.authRequired")
        });
        assert_eq!(next["sessionId"], "SID-auth");
        assert_ne!(next["params"]["requestId"], auth["params"]["requestId"]);
        auth = next;
    }
    let request_id = auth["params"]["requestId"].as_str().unwrap().to_owned();
    let session_id = auth["sessionId"].as_str().unwrap().to_owned();
    let response = match password {
        Some(password) => json!({
            "response": "ProvideCredentials", "username": "user", "password": password
        }),
        None => json!({ "response": "CancelAuth" }),
    };
    ctx.process_async(json!({
        "id": 2003,
        "method": "Fetch.continueWithAuth",
        "sessionId": session_id,
        "params": { "requestId": request_id, "authChallengeResponse": response }
    }))
    .await;
    ctx.expect_result(2003, json!({}), Some(&session_id));
    request_id
}

async fn verify_auth_retry_limit(endpoint: AuthEndpoint, intercept_response: bool, chained: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let server_requests = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            while !request.ends_with(b"\r\n\r\n") {
                let read = stream.read(&mut buffer).await.unwrap();
                assert_ne!(read, 0);
                request.extend_from_slice(&buffer[..read]);
                assert!(request.len() < 16384);
            }
            server_requests.fetch_add(1, Ordering::SeqCst);
            let request = String::from_utf8(request).unwrap();
            let (status, header, method) = match endpoint {
                AuthEndpoint::Server => ("401 Unauthorized", "WWW-Authenticate", "GET /"),
                AuthEndpoint::HttpProxy => (
                    "407 Proxy Authentication Required",
                    "Proxy-Authenticate",
                    "GET http://",
                ),
                AuthEndpoint::ConnectProxy => (
                    "407 Proxy Authentication Required",
                    "Proxy-Authenticate",
                    "CONNECT ",
                ),
            };
            assert!(request.starts_with(method), "{request}");
            let auth_header = match endpoint {
                AuthEndpoint::Server => "authorization",
                _ => "proxy-authorization",
            };
            let authorized = request.lines().any(|line| {
                line.split_once(':').is_some_and(|(name, value)| {
                    name.eq_ignore_ascii_case(auth_header) && value.trim() == "Basic dXNlcjpwYXNz"
                })
            });
            let response = if authorized && !matches!(endpoint, AuthEndpoint::ConnectProxy) {
                let body = "<!doctype html><title>authorized</title>authorized";
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                format!(
                    "HTTP/1.1 {status}\r\n{header}: Basic realm=\"retry-limit\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
            };
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.shutdown().await.unwrap();
        }
    });

    let mut conn = crate::testing::real_layout_test_connection();
    conn.set_root_target_discovery_enabled(true);
    let mut bc = attached_browser_context();
    if chained {
        assert!(bc.assign_attached_session_to_target("TID-1", "SID-auth".to_owned()));
    }
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    conn.install_browser_context_fixture_for_test(bc);
    let url = match endpoint {
        AuthEndpoint::Server => format!("http://{addr}/auth"),
        AuthEndpoint::HttpProxy => "http://example.test/auth".to_owned(),
        AuthEndpoint::ConnectProxy => "https://example.test/auth".to_owned(),
    };
    if !matches!(endpoint, AuthEndpoint::Server) {
        conn.set_http_proxy_override_async(Some(format!("http://{addr}")))
            .await;
    }
    let mut ctx = TestContext::from_conn(conn);
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.process_async(json!({
        "id": 1998, "method": "Fetch.enable", "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(1998, json!({}), Some("SID-1"));
    if chained {
        ctx.process_async(json!({
            "id": 1999, "method": "Fetch.enable", "sessionId": "SID-auth",
            "params": {
                "handleAuthRequests": true,
                "patterns": [{ "urlPattern": "*", "requestStage": "Response" }]
            }
        }))
        .await;
        ctx.expect_result(1999, json!({}), Some("SID-auth"));
    }

    let network_id = begin_navigation(&mut ctx, &url, intercept_response).await;
    let mut last_request_id = String::new();
    for _ in 0..MAX_NAVIGATION_AUTH_RETRIES {
        last_request_id = answer_auth(&mut ctx, chained, Some("wrong")).await;
    }
    let navigate = take_response_by_id(&mut ctx, 2000);
    assert_eq!(navigate["result"]["errorText"], "net::ERR_TOO_MANY_RETRIES");
    let failed = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(network_id)
        })
        .collect::<Vec<_>>();
    assert_eq!(failed.len(), 1);
    assert_eq!(
        failed[0]["params"]["errorText"],
        "net::ERR_TOO_MANY_RETRIES"
    );
    assert_eq!(
        requests.load(Ordering::SeqCst),
        usize::from(MAX_NAVIGATION_AUTH_RETRIES)
    );
    assert!(!ctx.sent.iter().any(|message| {
        matches!(
            message["method"].as_str(),
            Some("Fetch.authRequired" | "Network.loadingFinished")
        )
    }));
    ctx.with_fixture(|conn| {
        assert!(!{ conn.has_pending_document_navigation_for_session_owner(Some("SID-1")) })
    });
    ctx.with_fixture(|conn| {
        let fetch_owner = &conn
            .browser_context
            .as_ref()
            .unwrap()
            .active_page_target()
            .fetch_owner;
        assert!(!fetch_owner.has_pending_fetch_auth_navigation_for_test(&last_request_id));
        assert!(!fetch_owner.has_pending_fetch_auth_navigation_for_test("INT-1"));
    });

    // A fresh navigation gets its own budget and can correct an earlier mistake.
    begin_navigation(&mut ctx, &url, intercept_response).await;
    let wrong_attempts = if matches!(endpoint, AuthEndpoint::ConnectProxy) {
        1
    } else {
        MAX_NAVIGATION_AUTH_RETRIES - 2
    };
    for _ in 0..wrong_attempts {
        answer_auth(&mut ctx, chained, Some("wrong")).await;
    }
    if matches!(endpoint, AuthEndpoint::ConnectProxy) {
        answer_auth(&mut ctx, chained, None).await;
        let navigate = take_response_by_id(&mut ctx, 2000);
        assert_eq!(
            navigate["result"]["errorText"],
            "net::ERR_HTTP_RESPONSE_CODE_FAILURE"
        );
    } else {
        answer_auth(&mut ctx, chained, Some("pass")).await;
        if intercept_response {
            let paused = ctx.take_first_matching("authenticated response pause", |message| {
                message["method"] == json!("Fetch.requestPaused")
                    && message["params"]["responseStatusCode"] == json!(200)
            });
            ctx.process_async(json!({
                "id": 2004, "method": "Fetch.continueResponse", "sessionId": paused["sessionId"],
                "params": { "requestId": paused["params"]["requestId"] }
            }))
            .await;
            ctx.expect_result(2004, json!({}), paused["sessionId"].as_str());
        }
        assert!(
            take_response_by_id(&mut ctx, 2000)["result"]
                .get("errorText")
                .is_none()
        );
        wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    }
    ctx.with_fixture(|conn| {
        assert!(!{ conn.has_pending_document_navigation_for_session_owner(Some("SID-1")) })
    });
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn basic_server_auth_navigation_retries_are_bounded() {
    for streaming in [false, true] {
        verify_auth_retry_limit(AuthEndpoint::Server, streaming, false).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn basic_http_proxy_auth_navigation_retries_are_bounded() {
    for streaming in [false, true] {
        verify_auth_retry_limit(AuthEndpoint::HttpProxy, streaming, false).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn basic_connect_proxy_auth_navigation_retries_are_bounded() {
    for streaming in [false, true] {
        verify_auth_retry_limit(AuthEndpoint::ConnectProxy, streaming, false).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn basic_auth_navigation_retry_budget_survives_chained_session_request_ids() {
    verify_auth_retry_limit(AuthEndpoint::Server, true, true).await;
}
