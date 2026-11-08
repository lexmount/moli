use super::*;
use tokio_tungstenite::tungstenite::{Error, client::IntoClientRequest};

mod fetch;

#[tokio::test]
async fn request_guard_bidi_session_websocket_rejects_cross_origin() {
    let (addr, server) = spawn_test_protocol_server().await;
    let endpoint = format!("ws://{addr}/session");
    let mut request = endpoint.as_str().into_client_request().unwrap();
    request
        .headers_mut()
        .insert(header::ORIGIN, "https://attacker.example".parse().unwrap());
    match connect_async(request)
        .await
        .expect_err("cross-origin BiDi handshake must be rejected")
    {
        Error::Http(response) => assert_eq!(response.status(), StatusCode::FORBIDDEN),
        error => panic!("expected HTTP 403 for BiDi /session, got {error:?}"),
    }

    let (mut native, response) = connect_async(endpoint).await.unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    let messages = send_cdp_command(&mut native, 1, "session.status", None, json!({})).await;
    let status = messages.iter().find(|message| message["id"] == 1).unwrap();
    assert_eq!(status["type"], "success");
    assert_eq!(status["result"]["ready"], true);
    native.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn request_guard_webdriver_text_plain_session_post_rejects_cross_origin() {
    let router = build_router(test_state());
    let body = json!({"capabilities": {"alwaysMatch": {}}}).to_string();
    for content_type in ["text/plain", "text/plain;charset=UTF-8"] {
        for origin in [
            "https://attacker.example",
            "http://127.0.0.1:8000",
            "https://127.0.0.1:9222",
            "http://localhost:9222",
            "null",
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/session")
                        .header(header::HOST, "localhost")
                        .header(header::CONTENT_TYPE, content_type)
                        .header(header::ORIGIN, origin)
                        .body(Body::from(body.clone()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{content_type}, {origin}"
            );
            let status = request_json_with_router(router.clone(), Method::GET, "/status").await;
            assert_eq!(status["value"]["ready"], true, "no session may be created");
        }
    }

    // Native and same-origin POSTs both reach the session handler.
    for origin in [None, Some("http://127.0.0.1:9222")] {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/session")
            .header(header::HOST, "localhost")
            .header(header::CONTENT_TYPE, "text/plain");
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        let response = router
            .clone()
            .oneshot(request.body(Body::from(body.clone())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let session: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let session_id = session["value"]["sessionId"].as_str().unwrap();
        assert!(!session_id.is_empty());
        let status = request_json_with_router(router.clone(), Method::GET, "/status").await;
        assert_eq!(status["value"]["ready"], false);
        request_json_with_router(
            router.clone(),
            Method::DELETE,
            &format!("/session/{session_id}"),
        )
        .await;
    }
}

#[tokio::test]
async fn request_guard_json_endpoints_reject_rebinding_host_without_origin() {
    let router = build_router(test_state());
    let targets = request_json_with_router(router.clone(), Method::GET, "/json/list").await;
    let target_id = targets[0]["id"].as_str().unwrap();
    let activate = format!("/json/activate/{target_id}");
    let close = format!("/json/close/{target_id}");
    for (method, path) in [
        (Method::GET, "/json"),
        (Method::GET, "/json/list"),
        (Method::GET, "/json/version"),
        (Method::GET, "/json/protocol"),
        (Method::PUT, "/json/new?about:blank"),
        (Method::GET, activate.as_str()),
        (Method::GET, close.as_str()),
    ] {
        let rebinding = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method.clone())
                    .uri(path)
                    .header(header::HOST, "rebinding.attacker.example:9222")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rebinding.status(), StatusCode::FORBIDDEN, "{method} {path}");

        let native = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method.clone())
                    .uri(path)
                    .header(header::HOST, "127.0.0.1:9222")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(native.status(), StatusCode::OK, "{method} {path}");
    }
}

#[tokio::test]
async fn request_guard_allows_native_http_clients_and_blocks_untrusted_headers() {
    let router = build_router(test_state());
    for path in ["/json/version", "/json/version/", "/json/list", "/status"] {
        for (host, origin, expected) in [
            ("localhost", None, StatusCode::OK),
            ("localhost.", None, StatusCode::OK),
            ("APP.Dev.LocalHost:9222", None, StatusCode::OK),
            ("app.localhost.:9222", None, StatusCode::OK),
            ("localhost:", None, StatusCode::OK),
            ("127.0.0.1:9222", None, StatusCode::OK),
            ("127.0.0.1:", None, StatusCode::OK),
            ("192.168.1.20:9222", None, StatusCode::OK),
            ("[::1]:", None, StatusCode::OK),
            ("[2001:db8::1]:9222", None, StatusCode::OK),
            (
                "localhost",
                Some("https://attacker.example"),
                StatusCode::FORBIDDEN,
            ),
            (
                "app.localhost.:",
                Some("https://attacker.example"),
                StatusCode::FORBIDDEN,
            ),
            (
                "127.0.0.1:9222",
                Some("http://127.0.0.1:9222"),
                StatusCode::OK,
            ),
            ("localhost", Some("http://127.0.0.1:9222"), StatusCode::OK),
            (
                "localhost:9222",
                Some("http://localhost:9222"),
                StatusCode::FORBIDDEN,
            ),
            (
                "127.0.0.1:9222",
                Some("http://127.0.0.1:8000"),
                StatusCode::FORBIDDEN,
            ),
            (
                "127.0.0.1:9222",
                Some("https://127.0.0.1:9222"),
                StatusCode::FORBIDDEN,
            ),
            ("localhost", Some("null"), StatusCode::FORBIDDEN),
            (
                "rebinding.example:9222",
                Some("http://127.0.0.1:9222"),
                StatusCode::FORBIDDEN,
            ),
            (
                "localhost:bad",
                Some("http://127.0.0.1:9222"),
                StatusCode::FORBIDDEN,
            ),
            ("rebinding.example:9222", None, StatusCode::FORBIDDEN),
            (
                "app.localhost.attacker.example:",
                None,
                StatusCode::FORBIDDEN,
            ),
        ] {
            let mut request = Request::builder().uri(path).header(header::HOST, host);
            if let Some(origin) = origin {
                request = request.header(header::ORIGIN, origin);
            }
            let response = router
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{path}, {host}, {origin:?}");
        }
    }
}

#[tokio::test]
async fn request_guard_allowlist_and_wildcard_never_bypass_host_validation() {
    for allowed in ["https://trusted.example", "*"] {
        let mut state = test_state();
        state.remote_allow_origins = vec![allowed.into()];
        let router = build_router(state);
        for (host, origin, expected) in [
            ("localhost", "https://trusted.example", StatusCode::OK),
            (
                "localhost",
                "https://attacker.example",
                if allowed == "*" {
                    StatusCode::OK
                } else {
                    StatusCode::FORBIDDEN
                },
            ),
            (
                "rebinding.example:9222",
                "https://trusted.example",
                StatusCode::FORBIDDEN,
            ),
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/json/version")
                        .header(header::HOST, host)
                        .header(header::ORIGIN, origin)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{allowed}, {host}, {origin}");
        }
    }
}

#[tokio::test]
async fn request_guard_rejects_missing_and_duplicate_security_headers() {
    let router = build_router(test_state());
    for request in [
        Request::builder()
            .uri("/json/version")
            .header(header::ORIGIN, "http://127.0.0.1:9222"),
        Request::builder()
            .uri("/json/version")
            .header(header::HOST, "localhost")
            .header(header::HOST, "rebinding.example")
            .header(header::ORIGIN, "http://127.0.0.1:9222"),
        Request::builder()
            .uri("/json/version")
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://127.0.0.1:9222")
            .header(header::ORIGIN, "https://attacker.example"),
    ] {
        let response = router
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn request_guard_runs_before_route_matching_and_instance_admission() {
    let state = test_state();
    state.control.request(InstanceCommand::Shutdown);
    let router = build_router(state);
    for (path, method) in [
        ("/unknown/", Method::GET),
        ("//json/version//", Method::OPTIONS),
        ("/session", Method::POST),
        ("/json/new?about:blank", Method::PUT),
        ("/json/version", Method::DELETE),
    ] {
        for (host, origin) in [
            ("rebinding.example", None),
            ("localhost", Some("https://attacker.example")),
        ] {
            let mut request = Request::builder()
                .method(method.clone())
                .uri(path)
                .header(header::HOST, host);
            if let Some(origin) = origin {
                request = request.header(header::ORIGIN, origin);
            }
            let response = router
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
        }
    }
}

#[tokio::test]
async fn request_guard_rejects_websocket_handshakes_before_all_protocol_handlers() {
    let (addr, server) = spawn_test_protocol_server().await;
    let same_origin = format!("http://{addr}");
    let cross_port_origin = format!(
        "http://{}",
        SocketAddr::new(addr.ip(), addr.port().wrapping_add(1))
    );
    let cross_scheme_origin = format!("https://{addr}");
    let localhost_origin = format!("http://localhost:{}", addr.port());
    let (default_target_id, _) = discovered_startup_target_ids(addr).await;
    let browser = discover_browser_websocket_url(addr).await.unwrap();
    for endpoint in [
        browser,
        format!("ws://{addr}/devtools/page/{default_target_id}"),
        format!("ws://{addr}/session"),
        format!("ws://{addr}/session/unknown"),
        format!("ws://{addr}/unknown/"),
    ] {
        for (host, origin) in [
            (None, Some("https://attacker.example")),
            (None, Some(cross_port_origin.as_str())),
            (None, Some(cross_scheme_origin.as_str())),
            (None, Some(localhost_origin.as_str())),
            (None, Some("null")),
            (Some("rebinding.example"), None),
            (Some("rebinding.example"), Some(same_origin.as_str())),
        ] {
            let mut request = endpoint.as_str().into_client_request().unwrap();
            if let Some(host) = host {
                request
                    .headers_mut()
                    .insert(header::HOST, host.parse().unwrap());
            }
            if let Some(origin) = origin {
                request
                    .headers_mut()
                    .insert(header::ORIGIN, origin.parse().unwrap());
            }
            match connect_async(request).await.expect_err("unsafe WebSocket") {
                Error::Http(response) => {
                    assert_eq!(
                        response.status(),
                        StatusCode::FORBIDDEN,
                        "{endpoint}, {host:?}, {origin:?}"
                    );
                }
                error => panic!("expected HTTP 403 for {endpoint}, got {error:?}"),
            }
        }
    }
    server.abort();
}

#[tokio::test]
async fn request_guard_allows_native_and_same_origin_websocket_clients_for_all_protocols() {
    let (addr, server) = spawn_test_protocol_server().await;
    let same_origin = format!("http://{addr}");
    let (default_target_id, _) = discovered_startup_target_ids(addr).await;
    for endpoint in [
        discover_browser_websocket_url(addr).await.unwrap(),
        format!("ws://{addr}/devtools/page/{default_target_id}"),
        format!("ws://{addr}/session"),
    ] {
        for origin in [None, Some(same_origin.as_str())] {
            let mut request = endpoint.as_str().into_client_request().unwrap();
            if let Some(origin) = origin {
                request
                    .headers_mut()
                    .insert(header::ORIGIN, origin.parse().unwrap());
            }
            let (mut socket, response) = connect_async(request).await.unwrap();
            assert_eq!(
                response.status(),
                StatusCode::SWITCHING_PROTOCOLS,
                "{endpoint}, {origin:?}"
            );
            socket.close(None).await.unwrap();
        }
    }
    server.abort();
}
