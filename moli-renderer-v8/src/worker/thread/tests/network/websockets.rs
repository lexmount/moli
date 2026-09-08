// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

#[tokio::test]
async fn worker_classic_websocket_echo_round_trips_on_worker_loop() {
    ensure_v8();
    let (url, server) = spawn_text_echo_websocket_server().await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker websocket loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
            const events = [];
            const socket = new WebSocket({url:?});
            events.push(`construct:${{socket.readyState}}:${{socket instanceof WebSocket}}`);
            socket.onopen = () => {{
                events.push(`open:${{socket.readyState}}`);
                socket.send("hello-worker-ws");
            }};
            socket.onmessage = (event) => {{
                events.push(`message:${{event.data}}:${{event instanceof MessageEvent}}`);
                socket.close(1000, "done");
            }};
            socket.onclose = (event) => {{
                events.push(`close:${{event.code}}:${{event.wasClean}}:${{event instanceof CloseEvent}}`);
                postMessage(events.join("|"));
                close();
            }};
            socket.onerror = () => {{
                events.push("error");
            }};
            "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#""construct:0:true|open:1|message:hello-worker-ws:true|close:1000:true:true""#
    );
    server.await.expect("worker websocket server should finish");
}

#[tokio::test]
async fn worker_classic_websocket_report_only_csp_dispatches_without_blocking() {
    ensure_v8();
    let (url, server) = spawn_text_echo_websocket_server().await;
    let script_url = "http://127.0.0.1/worker/main.js";
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker websocket loader");
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
            const cspEvents = [];
            addEventListener("securitypolicyviolation", event => {{
                cspEvents.push({{
                    type: event.type,
                    effectiveDirective: event.effectiveDirective,
                    violatedDirective: event.violatedDirective,
                    blockedURI: event.blockedURI,
                    documentURI: event.documentURI,
                    originalPolicy: event.originalPolicy,
                    disposition: event.disposition,
                    instance: event instanceof SecurityPolicyViolationEvent
                }});
            }});
            const socket = new WebSocket({url:?});
            socket.onopen = () => socket.send("report-only-ws");
            socket.onmessage = (event) => {{
                postMessage({{ events: cspEvents, data: event.data }});
                socket.close(1000, "done");
                close();
            }};
            socket.onerror = () => {{
                postMessage({{ events: cspEvents, error: "ws-error" }});
                close();
            }};
            "#
            ),
            script_url.to_owned(),
        )
        .with_request_client(loader)
        .with_content_security_report_only_policies(vec!["connect-src 'none'".to_owned()]),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        format!(
            r#"{{"events":[{{"type":"securitypolicyviolation","effectiveDirective":"connect-src","violatedDirective":"connect-src","blockedURI":"{url}","documentURI":"{script_url}","originalPolicy":"connect-src 'none'","disposition":"report","instance":true}}],"data":"report-only-ws"}}"#
        )
    );
    server
        .await
        .expect("worker websocket report-only server should finish");
}

#[tokio::test]
async fn worker_websocket_csp_block_precedes_mixed_content_rejection() {
    ensure_v8();
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            r#"
            const events = [];
            addEventListener("securitypolicyviolation", event => {
                events.push({
                    blockedURI: event.blockedURI,
                    effectiveDirective: event.effectiveDirective,
                    disposition: event.disposition
                });
                postMessage({ outcome, events });
                close();
            });
            let outcome;
            try {
                const socket = new WebSocket("ws:/common/blank.html");
                outcome = `socket:${socket.readyState}:${socket.url}`;
            } catch (error) {
                outcome = `throw:${error.name}`;
            }
            postMessage({ outcome, events: events.length });
            "#
            .to_owned(),
            "http://localhost:8000/worker/main.js".to_owned(),
        )
        .with_content_security_policies(vec!["connect-src 'none'".to_owned()]),
    );

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"outcome":"socket:0:ws://common/blank.html","events":0}"#
    );
    assert_eq!(
        recv_post_json(&mut handle).await,
        r#"{"outcome":"socket:0:ws://common/blank.html","events":[{"blockedURI":"ws://common/blank.html","effectiveDirective":"connect-src","disposition":"enforce"}]}"#
    );
}

#[tokio::test]
async fn worker_classic_websocket_connect_src_self_allows_same_host_ws() {
    ensure_v8();
    let (url, server) = spawn_text_echo_websocket_server().await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker websocket loader");
    let ws_url = url::Url::parse(&url).expect("websocket url should parse");
    let worker_script_url = format!(
        "http://{}:{}/worker/main.js",
        ws_url.host_str().expect("websocket host"),
        ws_url.port_or_known_default().expect("websocket port")
    );
    let mut handle = spawn_test_worker_with_options(
        WorkerSpawnOptions::new(
            format!(
                r#"
            const socket = new WebSocket({url:?});
            socket.onopen = () => {{
                socket.send("self-csp");
            }};
            socket.onmessage = (event) => {{
                postMessage(event.data);
                socket.close(1000, "done");
                close();
            }};
            socket.onerror = () => {{
                postMessage("error");
                close();
            }};
            "#
            ),
            worker_script_url,
        )
        .with_request_client(loader)
        .with_content_security_policies(vec!["connect-src 'self'".to_owned()]),
    );

    assert_eq!(recv_post_json(&mut handle).await, r#""self-csp""#);
    server
        .await
        .expect("worker websocket self CSP server should finish");
}

#[tokio::test]
async fn worker_classic_websocket_applies_network_policy_extra_http_headers() {
    ensure_v8();
    let (url, headers_rx, server) = spawn_header_capture_websocket_server().await;
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker websocket loader");
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        format!(
            r#"
            const socket = new WebSocket({url:?});
            socket.onopen = () => {{
                postMessage("opened");
                socket.close();
                close();
            }};
            socket.onerror = () => {{
                postMessage("error");
                close();
            }};
            "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
        WorkerNetworkPolicy {
            extra_http_headers: vec![("x-cdp-test".to_owned(), "worker-ws".to_owned())].into(),
            ..WorkerNetworkPolicy::default()
        },
    );

    assert_eq!(recv_post_json(&mut handle).await, r#""opened""#);
    let headers = timeout(TIMEOUT, headers_rx)
        .await
        .expect("timed out waiting for websocket headers")
        .expect("websocket header sender dropped");
    let received = headers
        .iter()
        .find_map(|(name, value)| {
            name.eq_ignore_ascii_case("x-cdp-test")
                .then_some(value.as_str())
        })
        .unwrap_or_default();
    assert_eq!(received, "worker-ws");
    server
        .await
        .expect("worker websocket header server should finish");
}

#[tokio::test]
async fn worker_classic_websocket_handshake_set_cookie_updates_cookie_store() {
    ensure_v8();
    let (url, server) = spawn_set_cookie_websocket_server().await;
    let ws_url = url::Url::parse(&url).expect("websocket url");
    let cookie_url = moli_websocket::websocket_cookie_url(&ws_url);
    let cookie_store = moli_cookie_jar::new_shared_browser_cookie_store();
    let loader =
        ResourceRequestClient::new_with_cookie_store(&FetchConfig::default(), cookie_store.clone())
            .expect("worker websocket cookie loader");
    let mut handle = spawn_worker_with_request_client(
        format!(
            r#"
            const socket = new WebSocket({url:?});
            socket.onopen = () => {{
                postMessage("opened");
                socket.close();
                close();
            }};
            socket.onerror = () => {{
                postMessage("error");
                close();
            }};
            "#
        ),
        "http://127.0.0.1/worker/main.js".into(),
        loader,
    );

    assert_eq!(recv_post_json(&mut handle).await, r#""opened""#);
    server
        .await
        .expect("worker websocket set-cookie server should finish");

    let cookie_header = moli_fetch::cookie_header_for_request(
        &cookie_store,
        &cookie_url,
        moli_cookie_jar::NetworkCookieRequestContext::subresource("GET"),
    )
    .expect("cookie header lookup should succeed");
    assert_eq!(cookie_header.as_deref(), Some("ws_response_cookie=ok"));
}

#[tokio::test]
async fn worker_classic_websocket_blocked_url_reports_network_failure() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_blocked_url_patterns(
        r#"
        const events = [];
        const socket = new WebSocket("ws://127.0.0.1/blocked/worker-ws");
        socket.onerror = () => {
            events.push(`error:${socket.readyState}`);
        };
        socket.onclose = (event) => {
            events.push(`close:${event.code}:${event.wasClean}:${socket.readyState}`);
            postMessage(events.join("|"));
            close();
        };
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
        vec!["ws://127.0.0.1/blocked/*".to_owned()],
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let record = expect_subresource_network_record(network);
    assert_eq!(record.url().as_str(), "ws://127.0.0.1/blocked/worker-ws");
    assert_eq!(record.resource_type(), SubresourceResourceType::WebSocket);
    assert!(record.websocket_socket_id().is_some());
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text == "net::ERR_BLOCKED_BY_CLIENT"
    ));

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#""error:3|close:1006:false:3""#
    );
}

#[tokio::test]
async fn worker_classic_websocket_offline_reports_network_failure() {
    ensure_v8();
    let mut handle = spawn_worker_with_request_client_and_network_policy(
        r#"
        const events = [];
        const socket = new WebSocket("ws://127.0.0.1/offline/worker-ws");
        socket.onerror = () => {
            events.push(`error:${socket.readyState}`);
        };
        socket.onclose = (event) => {
            events.push(`close:${event.code}:${event.wasClean}:${socket.readyState}`);
            postMessage(events.join("|"));
            close();
        };
        "#
        .into(),
        "http://127.0.0.1/worker/main.js".into(),
        worker_test_request_client(),
        WorkerNetworkPolicy {
            network_offline: true,
            ..WorkerNetworkPolicy::default()
        },
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let record = expect_subresource_network_record(network);
    assert_eq!(record.url().as_str(), "ws://127.0.0.1/offline/worker-ws");
    assert_eq!(record.resource_type(), SubresourceResourceType::WebSocket);
    assert!(record.websocket_socket_id().is_some());
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text == "Network emulation offline"
    ));

    assert_eq!(
        recv_post_json(&mut handle).await,
        r#""error:3|close:1006:false:3""#
    );
}

#[tokio::test]
async fn worker_importscripts_websocket_resolves_against_worker_settings_url() {
    ensure_v8();
    let imported_script = r#"
        const events = [];
        const socket = new WebSocket("./blocked/imported-ws");
        socket.onerror = () => {
            events.push(`error:${socket.readyState}`);
        };
        socket.onclose = (event) => {
            events.push(`close:${event.code}:${event.wasClean}:${socket.readyState}`);
            postMessage({ url: socket.url, events: events.join("|") });
            close();
        };
    "#;
    let (base_url, server) = spawn_path_response_http_server(vec![(
        "/worker/imported/ws.js",
        "HTTP/1.1 200 OK",
        "application/javascript",
        imported_script.to_owned(),
        Duration::ZERO,
    )])
    .await;
    let websocket_base_url = base_url.replacen("http://", "ws://", 1);
    let expected_url = format!("{websocket_base_url}/worker/blocked/imported-ws");
    let loader =
        ResourceRequestClient::new(&FetchConfig::default()).expect("worker importScripts loader");
    let mut handle = spawn_worker_with_request_client_and_blocked_url_patterns(
        r#"
        importScripts("./imported/ws.js");
        "#
        .into(),
        format!("{base_url}/worker/main.js"),
        loader,
        vec![format!("{websocket_base_url}/worker/blocked/*")],
    );

    let network = timeout(TIMEOUT, handle.recv())
        .await
        .expect("timed out")
        .expect("channel closed");
    let record = expect_subresource_network_record(network);
    assert_eq!(record.url().as_str(), expected_url);
    assert_eq!(record.resource_type(), SubresourceResourceType::WebSocket);
    assert!(record.websocket_socket_id().is_some());
    assert!(matches!(
        record.outcome(),
        SubresourceNetworkOutcome::Failure { error_text }
            if error_text == "net::ERR_BLOCKED_BY_CLIENT"
    ));

    assert_eq!(
        recv_post_json(&mut handle).await,
        format!(r#"{{"url":"{expected_url}","events":"error:3|close:1006:false:3"}}"#)
    );
    server
        .await
        .expect("importScripts websocket server should finish");
}
