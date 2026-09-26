use super::*;

#[tokio::test]
async fn websocket_bidi_file_navigation_returns_unknown_error_without_lifecycle_or_replacement() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));
    let create = send_bidi_command(
        &mut socket,
        2,
        "browsingContext.create",
        json!({ "type": "tab" }),
    )
    .await;
    let context = create["result"]["context"]
        .as_str()
        .expect("created BiDi context")
        .to_owned();
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": [
                "browsingContext.navigationStarted",
                "browsingContext.fragmentNavigated",
                "browsingContext.domContentLoaded",
                "browsingContext.load"
            ],
            "contexts": [&context]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": &context,
                    "url": "file:///moli-policy-must-not-open",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send rejected BiDi file navigation");
    let rejected_messages = recv_until_id(&mut socket, 4).await;
    let rejected = bidi_message_by_id(&rejected_messages, 4);
    assert_eq!(
        rejected,
        &json!({
            "type": "error",
            "id": 4,
            "error": "unknown error",
            "message": "Navigation to a local file URL requires an explicitly granted browser capability.",
            "stacktrace": "",
        })
    );
    assert!(
        rejected_messages.iter().all(|message| {
            !matches!(
                message["method"].as_str(),
                Some("browsingContext.navigationStarted")
                    | Some("browsingContext.fragmentNavigated")
                    | Some("browsingContext.domContentLoaded")
                    | Some("browsingContext.load")
            )
        }),
        "rejected file navigation must not emit BiDi lifecycle events: {rejected_messages:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.getTree",
                "params": { "root": &context }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send BiDi getTree after rejected navigation");
    let tree_messages = recv_until_id(&mut socket, 5).await;
    assert!(
        tree_messages.iter().all(|message| {
            !matches!(
                message["method"].as_str(),
                Some("browsingContext.navigationStarted")
                    | Some("browsingContext.fragmentNavigated")
                    | Some("browsingContext.domContentLoaded")
                    | Some("browsingContext.load")
            )
        }),
        "rejected file navigation must not leak delayed lifecycle events: {tree_messages:?}"
    );
    let tree = bidi_message_by_id(&tree_messages, 5);
    assert_eq!(tree["type"], json!("success"));
    assert_eq!(tree["result"]["contexts"][0]["url"], json!("about:blank"));

    let end = send_bidi_command(&mut socket, 6, "session.end", json!({})).await;
    assert_eq!(end["type"], json!("success"));
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_navigation_stales_classic_element_from_replaced_page() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let session_id = classic_new_session_on_server(cdp_addr).await;
    let first_url =
        classic_data_url_for_bidi_test("<!doctype html><main id='target'>first Page</main>");
    let second_url =
        classic_data_url_for_bidi_test("<!doctype html><main id='target'>second Page</main>");
    let navigated = classic_request_on_server_with_body(
        cdp_addr,
        "POST",
        &format!("/session/{session_id}/url"),
        json!({ "url": first_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));
    let element = classic_request_on_server_with_body(
        cdp_addr,
        "POST",
        &format!("/session/{session_id}/element"),
        json!({ "using": "css selector", "value": "#target" }),
    )
    .await;
    let element_id = element["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .expect("Classic element id")
        .to_owned();

    let mut socket = connect_classic_session_bidi_socket(cdp_addr, &session_id).await;
    let tree = send_bidi_command(&mut socket, 1, "browsingContext.getTree", json!({})).await;
    let context_id = tree["result"]["contexts"][0]["context"]
        .as_str()
        .expect("attached Classic context id")
        .to_owned();
    let bidi_navigation = send_bidi_command(
        &mut socket,
        2,
        "browsingContext.navigate",
        json!({
            "context": context_id,
            "url": second_url,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(
        bidi_navigation["type"],
        json!("success"),
        "attached BiDi navigation should replace the Classic-owned Page: {bidi_navigation:?}"
    );

    let (status, stale) = classic_request_status_on_server_with_body(
        cdp_addr,
        "GET",
        &format!("/session/{session_id}/element/{element_id}/text"),
        json!({}),
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(stale["value"]["error"], json!("stale element reference"));

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_meta_refresh_navigation_emits_second_before_request() {
    async fn redirect_http_equiv() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><head><meta http-equiv="refresh" content="0;redirected.html"></head>"#,
        )
    }
    async fn redirected() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>redirected</body></html>",
        )
    }

    let fixture_app = Router::new()
        .route("/redirect_http_equiv.html", get(redirect_http_equiv))
        .route("/redirected.html", get(redirected));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi meta refresh fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi meta refresh fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let redirect_url = format!("http://{fixture_addr}/redirect_http_equiv.html");
    let redirected_url = format!("http://{fixture_addr}/redirected.html");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let subscribe = send_bidi_command_response(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["network.beforeRequestSent"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": redirect_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate meta refresh");
    let messages = recv_until_id(&mut socket, 4).await;
    let messages = collect_bidi_messages_until_method_count(
        &mut socket,
        messages,
        "network.beforeRequestSent",
        2,
    )
    .await;

    let navigate = bidi_message_by_id(&messages, 4);
    assert_eq!(
        navigate["type"],
        json!("success"),
        "meta refresh navigate response: {navigate:?}"
    );

    let before_requests = bidi_events_by_method(&messages, "network.beforeRequestSent");
    assert_eq!(
        before_requests.len(),
        2,
        "meta refresh should produce two document beforeRequestSent events: {messages:#?}"
    );
    assert_eq!(
        before_requests[0]["params"]["request"]["url"],
        json!(redirect_url)
    );
    assert_eq!(
        before_requests[1]["params"]["request"]["url"],
        json!(redirected_url)
    );
    assert_eq!(before_requests[0]["params"]["context"], json!(context_id));
    assert_eq!(before_requests[1]["params"]["context"], json!(context_id));
    assert_eq!(before_requests[0]["params"]["redirectCount"], json!(0));
    assert_eq!(before_requests[1]["params"]["redirectCount"], json!(0));
    assert_ne!(
        before_requests[0]["params"]["request"]["request"],
        before_requests[1]["params"]["request"]["request"],
        "meta refresh navigation should be a new document request"
    );
    assert!(
        before_requests[0]["params"]["navigation"]
            .as_str()
            .is_some(),
        "first document request should carry a navigation id: {before_requests:#?}"
    );
    assert!(
        before_requests[1]["params"]["navigation"]
            .as_str()
            .is_some(),
        "meta refresh document request should carry a navigation id: {before_requests:#?}"
    );
    assert_ne!(
        before_requests[0]["params"]["navigation"], before_requests[1]["params"]["navigation"],
        "meta refresh navigation should have a distinct navigation id"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_user_context_fetch_after_navigation_collects_response_body() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>userContext network page</body></html>",
        )
    }

    async fn empty_text() -> impl IntoResponse {
        // Keep the fetch asynchronous so script.evaluate(awaitPromise=true)
        // must resume through the non-default userContext target owner.
        sleep(Duration::from_millis(150)).await;
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/plain")],
            "user-context fetch body",
        )
    }

    let fixture_app = Router::new()
        .route("/empty.html", get(page))
        .route("/empty.txt", get(empty_text));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi userContext fetch fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi userContext fetch fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let page_url = format!("http://{fixture_addr}/empty.html");
    let fetch_url = format!("http://{fixture_addr}/empty.txt");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _default_context_id) = bidi_session_with_context(cdp_addr).await;

    let user_context =
        send_bidi_command(&mut socket, 3, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context")
        .to_owned();

    let tab = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(tab["type"], json!("success"));
    let context_id = tab["result"]["context"]
        .as_str()
        .expect("created userContext tab")
        .to_owned();

    let navigate = send_bidi_command_response(
        &mut socket,
        5,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": page_url,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"), "navigate: {navigate:?}");

    let baseline_fetch = send_bidi_command_response(
        &mut socket,
        6,
        "script.evaluate",
        json!({
            "expression": format!(
                "fetch({fetch_url:?}).then(response => 'OK:' + response.status).catch(error => 'ERR:' + error.name + ':' + error.message)"
            ),
            "target": {
                "context": context_id.clone()
            },
            "awaitPromise": true
        }),
    )
    .await;
    assert_eq!(
        baseline_fetch["type"],
        json!("success"),
        "baseline fetch: {baseline_fetch:?}"
    );
    assert_eq!(
        baseline_fetch["result"]["result"]["value"],
        json!("OK:200"),
        "fetch should resolve after navigating the non-default userContext tab before a collector is added: {baseline_fetch:?}"
    );

    let subscribe = send_bidi_command_response(
        &mut socket,
        7,
        "session.subscribe",
        json!({
            "events": ["network.responseCompleted"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(
        subscribe["type"],
        json!("success"),
        "subscribe: {subscribe:?}"
    );

    let add_collector = send_bidi_command_response(
        &mut socket,
        8,
        "network.addDataCollector",
        json!({
            "dataTypes": ["response"],
            "maxEncodedDataSize": 1000,
            "userContexts": [user_context_id]
        }),
    )
    .await;
    assert_eq!(
        add_collector["type"],
        json!("success"),
        "add collector: {add_collector:?}"
    );
    let collector = add_collector["result"]["collector"]
        .as_str()
        .expect("collector id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": format!(
                        "fetch({fetch_url:?}).then(response => response.text()).then(text => 'OK:' + text).catch(error => 'ERR:' + error.name + ':' + error.message)"
                    ),
                    "target": {
                        "context": context_id
                    },
                    "awaitPromise": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send BiDi userContext fetch evaluate");
    let mut messages = recv_until_id(&mut socket, 9).await;
    let evaluate = bidi_message_by_id(&messages, 9).clone();
    assert_eq!(evaluate["type"], json!("success"), "evaluate: {evaluate:?}");
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("OK:user-context fetch body"),
        "fetch should resolve after navigating the non-default userContext tab: {evaluate:?}"
    );
    if !messages.iter().any(|message| {
        message["method"] == json!("network.responseCompleted")
            && message["params"]["response"]["url"] == json!(fetch_url)
    }) {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == json!("network.responseCompleted")
                    && message["params"]["response"]["url"] == json!(fetch_url)
            })
            .await,
        );
    }
    let response_completed = messages
        .iter()
        .find(|message| {
            message["method"] == json!("network.responseCompleted")
                && message["params"]["response"]["url"] == json!(fetch_url)
        })
        .expect("fetch responseCompleted event");
    let request_id = response_completed["params"]["request"]["request"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    let data = send_bidi_command_response(
        &mut socket,
        10,
        "network.getData",
        json!({
            "request": request_id,
            "dataType": "response",
            "collector": collector
        }),
    )
    .await;
    assert_eq!(data["type"], json!("success"), "getData response: {data:?}");
    assert_eq!(
        data["result"]["bytes"],
        json!({
            "type": "string",
            "value": "user-context fetch body",
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_network_continue_with_auth_completes_waiting_navigation() {
    async fn auth(headers: axum::http::HeaderMap) -> axum::response::Response {
        let expected = "Basic cG9zdG1hbjpwYXNzd29yZA==";
        let authorized = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value == expected);
        if authorized {
            return (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><main>authenticated</main>",
            )
                .into_response();
        }
        (
            StatusCode::UNAUTHORIZED,
            [(
                axum::http::header::WWW_AUTHENTICATE.as_str(),
                "Basic realm=\"webdriver-smoke\"",
            )],
            "auth required",
        )
            .into_response()
    }

    let fixture_app = Router::new().route("/auth", get(auth));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi continueWithAuth fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi continueWithAuth fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let auth_url = format!("http://{fixture_addr}/auth");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    let create = send_bidi_command(
        &mut socket,
        2,
        "browsingContext.create",
        json!({"type": "tab"}),
    )
    .await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": [
                "network.authRequired",
                "network.responseCompleted",
                "browsingContext.load"
            ],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let add = send_bidi_command(
        &mut socket,
        4,
        "network.addIntercept",
        json!({
            "phases": ["authRequired"],
            "urlPatterns": [{
                "type": "string",
                "pattern": auth_url.clone()
            }],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(add["type"], json!("success"), "addIntercept: {add:?}");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": auth_url.clone(),
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate auth fixture");

    let messages = recv_until_match(&mut socket, |message| {
        message["method"] == json!("network.authRequired")
    })
    .await;
    assert!(
        messages.iter().all(|message| message["id"] != json!(5_u64)),
        "navigate response should stay pending until credentials continue auth: {messages:?}"
    );
    let auth_required = messages
        .iter()
        .find(|message| message["method"] == json!("network.authRequired"))
        .expect("network.authRequired event");
    assert_eq!(auth_required["params"]["context"], json!(context_id));
    assert_eq!(auth_required["params"]["isBlocked"], json!(true));
    assert_eq!(auth_required["params"]["request"]["url"], json!(auth_url));
    let request_id = auth_required["params"]["request"]["request"]
        .as_str()
        .expect("authRequired request id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "network.continueWithAuth",
                "params": {
                    "request": request_id,
                    "action": "provideCredentials",
                    "credentials": {
                        "type": "password",
                        "username": "postman",
                        "password": "password"
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send network.continueWithAuth credentials");

    let mut continue_messages = recv_until_id(&mut socket, 6).await;
    if !continue_messages
        .iter()
        .any(|message| message["id"] == json!(5_u64))
    {
        continue_messages.extend(recv_until_id(&mut socket, 5).await);
    }
    let continue_auth = continue_messages
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .expect("network.continueWithAuth response");
    assert_eq!(continue_auth["type"], json!("success"));
    let navigate = continue_messages
        .iter()
        .find(|message| message["id"] == json!(5_u64))
        .expect("waiting navigation response after continueWithAuth");
    assert_eq!(
        navigate["type"],
        json!("success"),
        "continueWithAuth should complete the original navigate: {continue_messages:?}"
    );
    assert!(
        navigate["result"]["navigation"].as_str().is_some(),
        "delayed navigate response should carry a navigation id: {navigate:?}"
    );
    assert_eq!(navigate["result"]["url"], json!(auth_url));

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_network_continue_with_auth_resolves_background_context_request() {
    async fn auth(headers: axum::http::HeaderMap) -> axum::response::Response {
        let expected = "Basic dXNlcjpzZWNyZXQ=";
        let authorized = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value == expected);
        if authorized {
            return (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/plain")],
                "authenticated",
            )
                .into_response();
        }
        (
            StatusCode::UNAUTHORIZED,
            [(
                axum::http::header::WWW_AUTHENTICATE.as_str(),
                "Basic realm=\"background\"",
            )],
            "auth required",
        )
            .into_response()
    }

    let fixture_app = Router::new().route("/auth", get(auth));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi background authRequired fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi background authRequired fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");
    let auth_url = format!("{fixture_url}auth");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    let foreground = send_bidi_command(
        &mut socket,
        2,
        "browsingContext.create",
        json!({ "type": "tab" }),
    )
    .await;
    assert_eq!(foreground["type"], json!("success"));
    let foreground_context_id = foreground["result"]["context"]
        .as_str()
        .expect("foreground context id")
        .to_owned();

    let background = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({
            "type": "tab",
            "background": true
        }),
    )
    .await;
    assert_eq!(background["type"], json!("success"));
    let background_context_id = background["result"]["context"]
        .as_str()
        .expect("background context id")
        .to_owned();

    let subscribe = send_bidi_command(
        &mut socket,
        4,
        "session.subscribe",
        json!({
            "events": [
                "network.beforeRequestSent",
                "network.authRequired",
                "network.responseCompleted"
            ],
            "contexts": [background_context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let add = send_bidi_command(
        &mut socket,
        5,
        "network.addIntercept",
        json!({
            "phases": ["authRequired"],
            "urlPatterns": [{
                "type": "string",
                "pattern": auth_url.clone()
            }],
            "contexts": [background_context_id.clone()]
        }),
    )
    .await;
    assert_eq!(add["type"], json!("success"), "addIntercept: {add:?}");
    let intercept = add["result"]["intercept"]
        .as_str()
        .expect("network.addIntercept should return intercept id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": background_context_id.clone(),
                    "url": auth_url.clone(),
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send background browsingContext.navigate auth fixture");
    let messages = recv_until_match(&mut socket, |message| {
        message["method"] == json!("network.authRequired")
    })
    .await;

    let before_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("network.beforeRequestSent")
                && message["params"]["request"]["url"].as_str() == Some(auth_url.as_str())
        })
        .expect("background network.beforeRequestSent auth event");
    assert_eq!(before_request["type"], json!("event"));
    assert_eq!(
        before_request["params"]["context"],
        json!(background_context_id.clone())
    );
    assert_eq!(before_request["params"]["isBlocked"], json!(false));

    let auth_required = messages
        .iter()
        .find(|message| message["method"] == json!("network.authRequired"))
        .expect("background network.authRequired event");
    assert_eq!(auth_required["type"], json!("event"));
    assert_eq!(
        auth_required["params"]["context"],
        json!(background_context_id.clone())
    );
    assert_eq!(auth_required["params"]["isBlocked"], json!(true));
    assert_eq!(auth_required["params"]["intercepts"], json!([intercept]));
    assert_eq!(auth_required["params"]["request"]["url"], json!(auth_url));
    assert_eq!(auth_required["params"]["response"]["status"], json!(401));
    assert_eq!(
        auth_required["params"]["response"]["authChallenges"],
        json!([{
            "scheme": "Basic",
            "realm": "background"
        }])
    );
    assert!(
        messages
            .iter()
            .all(|message| message["method"] != json!("network.responseCompleted")),
        "background authRequired intercept should keep the request blocked: {messages:?}"
    );
    assert_eq!(
        bidi_focus_visibility_surface(&mut socket, 7, &foreground_context_id).await,
        json!({
            "hasFocus": true,
            "hidden": false,
            "visibilityState": "visible"
        }),
        "blocked background navigation must not promote the background context"
    );

    let request_id = auth_required["params"]["request"]["request"]
        .as_str()
        .expect("background authRequired request id")
        .to_owned();
    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "network.continueWithAuth",
                "params": {
                    "request": request_id,
                    "action": "cancel"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send background network.continueWithAuth cancel");
    let continue_messages = recv_until_id(&mut socket, 8).await;
    let continue_auth = continue_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("background network.continueWithAuth response");
    assert_eq!(
        continue_auth["type"],
        json!("success"),
        "continueWithAuth should resolve the background request owner: {continue_messages:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_wait_none_navigation_drains_before_next_command() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    let navigate_url = "data:text/html,<body>wait-none-ready</body>";
    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": navigate_url,
                    "wait": "none"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send wait=none browsingContext.navigate");
    let navigate = recv_ws_json(&mut socket).await;
    assert_eq!(navigate["type"], json!("success"));
    assert!(
        navigate["result"]["navigation"].as_str().is_some(),
        "wait=none navigate should return a navigation id: {navigate:?}"
    );
    assert_eq!(navigate["result"]["url"], json!(navigate_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "document.body.textContent",
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate after wait=none navigation");
    let evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["id"], json!(4_u64));
    assert_eq!(evaluate["result"]["type"], json!("success"));
    assert_eq!(evaluate["result"]["result"]["type"], json!("string"));
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("wait-none-ready")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_wait_none_navigation_returns_before_parser_blocking_script() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
window.__bidiDclFired = false;
document.addEventListener('DOMContentLoaded', () => { window.__bidiDclFired = true; });
</script>
<script src="/slow.js"></script>
</head>
<body><main id="ready">wait-none-script-ready</main></body>
</html>"#,
        )
    }
    let script_requested = Arc::new(tokio::sync::Notify::new());
    let release_script = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&script_requested);
    let release_for_route = Arc::clone(&release_script);
    let fixture_app = Router::new().route("/", get(page)).route(
        "/slow.js",
        get(move || {
            let requested_for_route = Arc::clone(&requested_for_route);
            let release_for_route = Arc::clone(&release_for_route);
            async move {
                requested_for_route.notify_one();
                release_for_route.notified().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                    "window.__bidiSlowScriptExecuted = true;",
                )
            }
        }),
    );
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi wait=none script fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi wait=none script fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["browsingContext.domContentLoaded"],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("subscribe to the exact navigation DOMContentLoaded event");
    let subscribe = recv_ws_json(&mut socket).await;
    assert_eq!(subscribe["type"], json!("success"));
    assert_eq!(subscribe["id"], json!(3_u64));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": fixture_url,
                    "wait": "none"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send wait=none browsingContext.navigate");
    let navigate = timeout(Duration::from_secs(1), recv_ws_json(&mut socket))
        .await
        .expect("wait=none navigate should return before parser-blocking script completes");
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(navigate["id"], json!(4_u64));
    assert_eq!(navigate["result"]["url"], json!(fixture_url));
    let navigation_id = navigate["result"]["navigation"]
        .as_str()
        .expect("wait=none navigate should return its navigation id")
        .to_owned();

    timeout(Duration::from_secs(1), script_requested.notified())
        .await
        .expect("parser-blocking script request should start after wait=none navigate");
    release_script.notify_one();

    let dom_content_loaded = recv_ws_json(&mut socket).await;
    assert_eq!(dom_content_loaded["type"], json!("event"));
    assert_eq!(
        dom_content_loaded["method"],
        json!("browsingContext.domContentLoaded")
    );
    assert_eq!(
        dom_content_loaded["params"]["context"],
        json!(context_id.as_str())
    );
    assert_eq!(
        dom_content_loaded["params"]["navigation"],
        json!(navigation_id)
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "JSON.stringify({text: document.querySelector('#ready')?.textContent, script: window.__bidiSlowScriptExecuted === true, dcl: window.__bidiDclFired === true})",
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate after wait=none script navigation");
    let evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["id"], json!(5_u64));
    assert_eq!(evaluate["result"]["type"], json!("success"));
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("{\"text\":\"wait-none-script-ready\",\"script\":true,\"dcl\":true}")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_interactive_navigation_returns_before_pending_load_stylesheet() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
window.__bidiLoadFired = false;
window.addEventListener('load', () => { window.__bidiLoadFired = true; });
</script>
<link rel="stylesheet" href="/slow.css">
</head>
<body><main id="ready">interactive-ready</main></body>
</html>"#,
        )
    }
    let stylesheet_requested = Arc::new(tokio::sync::Notify::new());
    let release_stylesheet = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&stylesheet_requested);
    let release_for_route = Arc::clone(&release_stylesheet);
    let fixture_app = Router::new().route("/", get(page)).route(
        "/slow.css",
        get(move || {
            let requested_for_route = Arc::clone(&requested_for_route);
            let release_for_route = Arc::clone(&release_for_route);
            async move {
                requested_for_route.notify_one();
                release_for_route.notified().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                    "body { color: black; }",
                )
            }
        }),
    );
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi wait=interactive fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi wait=interactive fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": fixture_url,
                    "wait": "interactive"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send wait=interactive browsingContext.navigate");
    timeout(Duration::from_secs(1), stylesheet_requested.notified())
        .await
        .expect("stylesheet request should start before load");
    let navigate = timeout(Duration::from_secs(1), recv_ws_json(&mut socket))
        .await
        .expect("wait=interactive navigate should return before stylesheet load completes");
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(navigate["id"], json!(3_u64));
    assert_eq!(navigate["result"]["url"], json!(fixture_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "JSON.stringify({text: document.querySelector('#ready')?.textContent, loaded: window.__bidiLoadFired === true})",
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate after wait=interactive navigation");
    let evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["id"], json!(4_u64));
    assert_eq!(evaluate["result"]["type"], json!("success"));
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("{\"text\":\"interactive-ready\",\"loaded\":false}")
    );

    release_stylesheet.notify_one();
    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_interactive_navigation_waits_for_parser_blocking_script() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
window.__bidiDclFired = false;
document.addEventListener('DOMContentLoaded', () => { window.__bidiDclFired = true; });
</script>
<script src="/slow.js"></script>
</head>
<body><main id="ready">interactive-script-ready</main></body>
</html>"#,
        )
    }
    let script_requested = Arc::new(tokio::sync::Notify::new());
    let release_script = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&script_requested);
    let release_for_route = Arc::clone(&release_script);
    let fixture_app = Router::new().route("/", get(page)).route(
        "/slow.js",
        get(move || {
            let requested_for_route = Arc::clone(&requested_for_route);
            let release_for_route = Arc::clone(&release_for_route);
            async move {
                requested_for_route.notify_one();
                release_for_route.notified().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                    "window.__bidiSlowScriptExecuted = true;",
                )
            }
        }),
    );
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi wait=interactive script fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi wait=interactive script fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": fixture_url,
                    "wait": "interactive"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send wait=interactive browsingContext.navigate");
    timeout(Duration::from_secs(1), script_requested.notified())
        .await
        .expect("parser-blocking script request should start before DCL");

    let early_navigate = timeout(Duration::from_millis(100), recv_ws_json(&mut socket)).await;
    assert!(
        early_navigate.is_err(),
        "wait=interactive navigate must not return before parser-blocking script completes: {early_navigate:?}"
    );

    release_script.notify_one();
    let navigate = timeout(Duration::from_secs(1), recv_ws_json(&mut socket))
        .await
        .expect("wait=interactive navigate should return after parser-blocking script completes");
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(navigate["id"], json!(3_u64));
    assert_eq!(navigate["result"]["url"], json!(fixture_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "JSON.stringify({text: document.querySelector('#ready')?.textContent, script: window.__bidiSlowScriptExecuted === true, dcl: window.__bidiDclFired === true})",
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate after wait=interactive script navigation");
    let evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["id"], json!(4_u64));
    assert_eq!(evaluate["result"]["type"], json!("success"));
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("{\"text\":\"interactive-script-ready\",\"script\":true,\"dcl\":true}")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_complete_navigation_waits_for_pending_load_stylesheet() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
window.__bidiLoadFired = false;
window.addEventListener('load', () => { window.__bidiLoadFired = true; });
</script>
<link rel="stylesheet" href="/slow.css">
</head>
<body><main id="ready">complete-ready</main></body>
</html>"#,
        )
    }
    let stylesheet_requested = Arc::new(tokio::sync::Notify::new());
    let release_stylesheet = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&stylesheet_requested);
    let release_for_route = Arc::clone(&release_stylesheet);
    let fixture_app = Router::new().route("/", get(page)).route(
        "/slow.css",
        get(move || {
            let requested_for_route = Arc::clone(&requested_for_route);
            let release_for_route = Arc::clone(&release_for_route);
            async move {
                requested_for_route.notify_one();
                release_for_route.notified().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                    "body { color: black; }",
                )
            }
        }),
    );
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi wait=complete fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi wait=complete fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": fixture_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send wait=complete browsingContext.navigate");
    timeout(Duration::from_secs(1), stylesheet_requested.notified())
        .await
        .expect("stylesheet request should start before load");

    let early_navigate = timeout(Duration::from_millis(100), recv_ws_json(&mut socket)).await;
    assert!(
        early_navigate.is_err(),
        "wait=complete navigate must not return before stylesheet load completes: {early_navigate:?}"
    );

    release_stylesheet.notify_one();
    let navigate = timeout(Duration::from_secs(1), recv_ws_json(&mut socket))
        .await
        .expect("wait=complete navigate should return after stylesheet load completes");
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(navigate["id"], json!(3_u64));
    assert_eq!(navigate["result"]["url"], json!(fixture_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "JSON.stringify({text: document.querySelector('#ready')?.textContent, loaded: window.__bidiLoadFired === true})",
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate after wait=complete navigation");
    let evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["id"], json!(4_u64));
    assert_eq!(evaluate["result"]["type"], json!("success"));
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("{\"text\":\"complete-ready\",\"loaded\":true}")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_complete_navigation_waits_for_parser_blocking_script() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
window.__bidiLoadFired = false;
window.addEventListener('load', () => { window.__bidiLoadFired = true; });
</script>
<script src="/slow.js"></script>
</head>
<body><main id="ready">script-ready</main></body>
</html>"#,
        )
    }
    let script_requested = Arc::new(tokio::sync::Notify::new());
    let release_script = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&script_requested);
    let release_for_route = Arc::clone(&release_script);
    let fixture_app = Router::new().route("/", get(page)).route(
        "/slow.js",
        get(move || {
            let requested_for_route = Arc::clone(&requested_for_route);
            let release_for_route = Arc::clone(&release_for_route);
            async move {
                requested_for_route.notify_one();
                release_for_route.notified().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/javascript")],
                    "window.__bidiSlowScriptExecuted = true;",
                )
            }
        }),
    );
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi wait=complete script fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi wait=complete script fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": fixture_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send wait=complete browsingContext.navigate");
    timeout(Duration::from_secs(1), script_requested.notified())
        .await
        .expect("parser-blocking script request should start before DCL/load");

    let early_navigate = timeout(Duration::from_millis(100), recv_ws_json(&mut socket)).await;
    assert!(
        early_navigate.is_err(),
        "wait=complete navigate must not return before parser-blocking script completes: {early_navigate:?}"
    );

    release_script.notify_one();
    let navigate = timeout(Duration::from_secs(1), recv_ws_json(&mut socket))
        .await
        .expect("wait=complete navigate should return after parser-blocking script completes");
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(navigate["id"], json!(3_u64));
    assert_eq!(navigate["result"]["url"], json!(fixture_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "JSON.stringify({text: document.querySelector('#ready')?.textContent, script: window.__bidiSlowScriptExecuted === true, loaded: window.__bidiLoadFired === true})",
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate after wait=complete script navigation");
    let evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["id"], json!(4_u64));
    assert_eq!(evaluate["result"]["type"], json!("success"));
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("{\"text\":\"script-ready\",\"script\":true,\"loaded\":true}")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_complete_navigation_waits_for_slow_image_load() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head>
<script>
window.__bidiLoadFired = false;
window.addEventListener('load', () => { window.__bidiLoadFired = true; });
</script>
</head>
<body>
<main id="ready">complete-image-ready</main>
<img id="tail" src="/slow.svg">
</body>
</html>"#,
        )
    }
    let image_requested = Arc::new(tokio::sync::Notify::new());
    let release_image = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&image_requested);
    let release_for_route = Arc::clone(&release_image);
    let fixture_app = Router::new().route("/", get(page)).route(
        "/slow.svg",
        get(move || {
            let requested_for_route = Arc::clone(&requested_for_route);
            let release_for_route = Arc::clone(&release_for_route);
            async move {
                requested_for_route.notify_one();
                release_for_route.notified().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "image/svg+xml")],
                    r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"></svg>"#,
                )
            }
        }),
    );
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi wait=complete image fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi wait=complete image fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) =
        spawn_test_protocol_server_with_image_fetch_enabled(true).await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": fixture_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send wait=complete browsingContext.navigate");
    timeout(Duration::from_secs(1), image_requested.notified())
        .await
        .expect("image request should start before load");

    let early_navigate = timeout(Duration::from_millis(100), recv_ws_json(&mut socket)).await;
    assert!(
        early_navigate.is_err(),
        "wait=complete navigate must not return before image load completes: {early_navigate:?}"
    );

    release_image.notify_one();
    let navigate = timeout(Duration::from_secs(1), recv_ws_json(&mut socket))
        .await
        .expect("wait=complete navigate should return after image load completes");
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(navigate["id"], json!(3_u64));
    assert_eq!(navigate["result"]["url"], json!(fixture_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "JSON.stringify({text: document.querySelector('#ready')?.textContent, loaded: window.__bidiLoadFired === true})",
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate after wait=complete image navigation");
    let evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["id"], json!(4_u64));
    assert_eq!(evaluate["result"]["type"], json!("success"));
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("{\"text\":\"complete-image-ready\",\"loaded\":true}")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_complete_navigation_waits_for_slow_main_document_response() {
    let page_requested = Arc::new(tokio::sync::Notify::new());
    let release_page = Arc::new(tokio::sync::Notify::new());
    let requested_for_route = Arc::clone(&page_requested);
    let release_for_route = Arc::clone(&release_page);
    let fixture_app = Router::new().route(
        "/slow-page",
        get(move || {
            let requested_for_route = Arc::clone(&requested_for_route);
            let release_for_route = Arc::clone(&release_for_route);
            async move {
                requested_for_route.notify_one();
                release_for_route.notified().await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    r#"<!doctype html>
<html>
<head>
<script>
window.__bidiLoadFired = false;
window.addEventListener('load', () => { window.__bidiLoadFired = true; });
</script>
</head>
<body><main id="ready">complete-slow-page-ready</main></body>
</html>"#,
                )
            }
        }),
    );
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi wait=complete slow-page fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi wait=complete slow-page fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/slow-page");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": fixture_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send wait=complete browsingContext.navigate");
    timeout(Duration::from_secs(1), page_requested.notified())
        .await
        .expect("main document request should start");

    let early_navigate = timeout(Duration::from_millis(100), recv_ws_json(&mut socket)).await;
    assert!(
        early_navigate.is_err(),
        "wait=complete navigate must not return before main document response completes: {early_navigate:?}"
    );

    release_page.notify_one();
    let navigate = timeout(Duration::from_secs(1), recv_ws_json(&mut socket))
        .await
        .expect("wait=complete navigate should return after main document response completes");
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(navigate["id"], json!(3_u64));
    assert_eq!(navigate["result"]["url"], json!(fixture_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "JSON.stringify({text: document.querySelector('#ready')?.textContent, loaded: window.__bidiLoadFired === true})",
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate after wait=complete slow-page navigation");
    let evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["id"], json!(4_u64));
    assert_eq!(evaluate["result"]["type"], json!("success"));
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("{\"text\":\"complete-slow-page-ready\",\"loaded\":true}")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_traverse_history_executes_shared_history_delta() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    let first_url = "data:text/html,<title>First</title>bidi-history-first";
    let second_url = "data:text/html,<title>Second</title>bidi-history-second";
    for (id, url) in [(3_u64, first_url), (4_u64, second_url)] {
        socket
            .send(WsMessage::Text(
                json!({
                    "id": id,
                    "method": "browsingContext.navigate",
                    "params": {
                        "context": context_id.clone(),
                        "url": url,
                        "wait": "complete"
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send browsingContext.navigate");
        let navigate = recv_ws_json(&mut socket).await;
        assert_eq!(navigate["type"], json!("success"));
        assert_eq!(navigate["result"]["url"], json!(url));
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.traverseHistory",
                "params": {
                    "context": context_id.clone(),
                    "delta": -1
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send back browsingContext.traverseHistory");
    let back = recv_ws_json(&mut socket).await;
    assert_eq!(back["type"], json!("success"));
    assert_eq!(back["id"], json!(5_u64));
    assert_eq!(back["result"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "browsingContext.traverseHistory",
                "params": {
                    "context": context_id.clone(),
                    "delta": 0
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send no-op browsingContext.traverseHistory");
    let noop = recv_ws_json(&mut socket).await;
    assert_eq!(noop["type"], json!("success"));
    assert_eq!(noop["id"], json!(6_u64));
    assert_eq!(noop["result"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "browsingContext.getTree",
                "params": {
                    "root": context_id.clone()
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.getTree after back");
    let tree = recv_ws_json(&mut socket).await;
    assert_eq!(tree["type"], json!("success"));
    assert_eq!(tree["result"]["contexts"][0]["url"], json!(first_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "browsingContext.traverseHistory",
                "params": {
                    "context": context_id.clone(),
                    "delta": 1
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send forward browsingContext.traverseHistory");
    let forward = recv_ws_json(&mut socket).await;
    assert_eq!(forward["type"], json!("success"));
    assert_eq!(forward["id"], json!(8_u64));
    assert_eq!(forward["result"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "browsingContext.getTree",
                "params": {
                    "root": context_id.clone()
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.getTree after forward");
    let tree = recv_ws_json(&mut socket).await;
    assert_eq!(tree["type"], json!("success"));
    assert_eq!(tree["result"]["contexts"][0]["url"], json!(second_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 10_u64,
                "method": "browsingContext.traverseHistory",
                "params": {
                    "context": context_id,
                    "delta": 1
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send out-of-range browsingContext.traverseHistory");
    let out_of_range = recv_ws_json(&mut socket).await;
    assert_eq!(out_of_range["type"], json!("error"));
    assert_eq!(out_of_range["id"], json!(10_u64));
    assert_eq!(out_of_range["error"], json!("no such history entry"));

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_traverse_history_covers_same_document_hash_and_push_state() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Same Document History</title><main>same-document</main>",
        )
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi same-document history fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi same-document history fixture addr");
    let base_url = format!("http://{fixture_addr}/page");
    let fixture_app = Router::new().route("/page", get(page));
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    let hash_pages = [
        base_url.clone(),
        format!("{base_url}#foo"),
        format!("{base_url}#bar"),
    ];
    for (id, url) in [
        (3_u64, &hash_pages[0]),
        (4_u64, &hash_pages[1]),
        (5_u64, &hash_pages[2]),
    ] {
        socket
            .send(WsMessage::Text(
                json!({
                    "id": id,
                    "method": "browsingContext.navigate",
                    "params": {
                        "context": context_id.clone(),
                        "url": url,
                        "wait": "complete"
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send hash browsingContext.navigate");
        let navigate = recv_ws_json(&mut socket).await;
        assert_eq!(navigate["type"], json!("success"));
        assert_eq!(
            bidi_location_href(&mut socket, id + 100, &context_id).await,
            url.as_str()
        );
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "browsingContext.traverseHistory",
                "params": {
                    "context": context_id.clone(),
                    "delta": -1
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send hash back browsingContext.traverseHistory");
    let hash_back = recv_ws_json(&mut socket).await;
    assert_eq!(hash_back["type"], json!("success"));
    assert_eq!(
        bidi_location_href(&mut socket, 7, &context_id).await,
        hash_pages[1]
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "browsingContext.traverseHistory",
                "params": {
                    "context": context_id.clone(),
                    "delta": 1
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send hash forward browsingContext.traverseHistory");
    let hash_forward = recv_ws_json(&mut socket).await;
    assert_eq!(hash_forward["type"], json!("success"));
    assert_eq!(
        bidi_location_href(&mut socket, 9, &context_id).await,
        hash_pages[2]
    );

    let pushed_pages = [format!("{base_url}#push-a"), format!("{base_url}#push-b")];
    for (id, url) in [(10_u64, &pushed_pages[0]), (11_u64, &pushed_pages[1])] {
        socket
            .send(WsMessage::Text(
                json!({
                    "id": id,
                    "method": "script.callFunction",
                    "params": {
                        "functionDeclaration": "(url) => { history.pushState(null, '', url); return location.href; }",
                        "arguments": [
                            {
                                "type": "string",
                                "value": url
                            }
                        ],
                        "awaitPromise": false,
                        "target": {
                            "context": context_id.clone()
                        }
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send pushState script.callFunction");
        let push_state = recv_ws_json(&mut socket).await;
        assert_eq!(push_state["type"], json!("success"));
        assert_eq!(push_state["result"]["type"], json!("success"));
        assert_eq!(push_state["result"]["result"]["value"], json!(url));
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 12_u64,
                "method": "browsingContext.traverseHistory",
                "params": {
                    "context": context_id.clone(),
                    "delta": -1
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send pushState back browsingContext.traverseHistory");
    let push_back = recv_ws_json(&mut socket).await;
    assert_eq!(push_back["type"], json!("success"));
    assert_eq!(
        bidi_location_href(&mut socket, 13, &context_id).await,
        pushed_pages[0]
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 14_u64,
                "method": "browsingContext.traverseHistory",
                "params": {
                    "context": context_id.clone(),
                    "delta": 1
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send pushState forward browsingContext.traverseHistory");
    let push_forward = recv_ws_json(&mut socket).await;
    assert_eq!(push_forward["type"], json!("success"));
    assert_eq!(
        bidi_location_href(&mut socket, 15, &context_id).await,
        pushed_pages[1]
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_traverse_history_rejects_iframe_context() {
    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Child Frame</title><main>child-frame</main>",
        )
    }

    let child_app = Router::new().route("/child", get(child));
    let (child_addr, _child_server) =
        spawn_dedicated_fixture_server(child_app, "bidi-traverse-history-child");
    let child_url = format!("http://{child_addr}/child");

    let parent_html = format!(
        r#"<!doctype html>
<html>
<head><title>Top Frame</title></head>
<body><main>top-frame</main><iframe src="{child_url}"></iframe></body>
</html>"#
    );
    let parent_app = Router::new().route(
        "/",
        get(move || {
            let parent_html = parent_html.clone();
            async move {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    parent_html,
                )
            }
        }),
    );
    let (parent_addr, _parent_server) =
        spawn_dedicated_fixture_server(parent_app, "bidi-traverse-history-parent");
    let parent_url = format!("http://{parent_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.create");
    let create = recv_ws_json(&mut socket).await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": parent_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send parent browsingContext.navigate");
    let navigate = recv_ws_json(&mut socket).await;
    assert_eq!(navigate["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.getRealms",
                "params": {
                    "type": "window"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send all script.getRealms");
    let all_realms = recv_ws_json(&mut socket).await;
    let window_realms = all_realms["result"]["realms"]
        .as_array()
        .expect("script.getRealms should return realm array");
    let child_context_id = window_realms
        .iter()
        .find_map(|realm| {
            (realm["type"] == json!("window")
                && realm["context"].as_str().is_some_and(|id| id != context_id))
            .then(|| {
                realm["context"]
                    .as_str()
                    .expect("iframe context id")
                    .to_owned()
            })
        })
        .unwrap_or_else(|| {
            panic!("script.getRealms should include iframe window realm: {all_realms:?}")
        });

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.traverseHistory",
                "params": {
                    "context": child_context_id,
                    "delta": -1
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send iframe-context browsingContext.traverseHistory");
    let iframe_context = recv_ws_json(&mut socket).await;
    assert_eq!(iframe_context["type"], json!("error"));
    assert_eq!(iframe_context["id"], json!(5_u64));
    assert_eq!(iframe_context["error"], json!("invalid argument"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "browsingContext.traverseHistory",
                "params": {
                    "context": "missing-frame-context",
                    "delta": -1
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send unknown-context browsingContext.traverseHistory");
    let unknown_context = recv_ws_json(&mut socket).await;
    assert_eq!(unknown_context["type"], json!("error"));
    assert_eq!(unknown_context["id"], json!(6_u64));
    assert_eq!(unknown_context["error"], json!("no such frame"));

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_navigate_explicit_about_blank_uses_synthetic_document() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let data_navigate = send_bidi_command(
        &mut socket,
        190,
        "browsingContext.navigate",
        json!({
            "context": context_id,
            "url": "data:text/html,<script>window.marker='old'</script><p>old</p>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(
        data_navigate["type"],
        json!("success"),
        "data navigation should succeed before about:blank cleanup: {data_navigate:?}"
    );

    let blank_navigate = send_bidi_command(
        &mut socket,
        191,
        "browsingContext.navigate",
        json!({
            "context": context_id,
            "url": "about:blank",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(
        blank_navigate["type"],
        json!("success"),
        "explicit about:blank navigation should not fetch via curl: {blank_navigate:?}"
    );
    assert_eq!(blank_navigate["result"]["url"], json!("about:blank"));
    assert!(
        blank_navigate["result"]["navigation"].is_string(),
        "explicit about:blank navigation should keep a navigation id: {blank_navigate:?}"
    );

    let page_state = send_bidi_command(
        &mut socket,
        192,
        "script.evaluate",
        json!({
            "expression": "location.href + '|' + document.body.childNodes.length + '|' + document.title + '|' + (window.marker === undefined)",
            "target": {
                "context": context_id
            },
            "awaitPromise": true
        }),
    )
    .await;
    assert_eq!(page_state["type"], json!("success"));
    assert_eq!(
        page_state["result"]["result"],
        json!({
            "type": "string",
            "value": "about:blank|0||true"
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_browsing_context_navigation_invalid_parameters_match_wpt_error_shape() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/navigate/invalid.py and
    // webdriver/tests/bidi/browsing_context/reload/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 200_u64;

    for params in [
        json!({"context": null, "url": "data:text/html,<p>foo</p>"}),
        json!({"context": false, "url": "data:text/html,<p>foo</p>"}),
        json!({"context": 42, "url": "data:text/html,<p>foo</p>"}),
        json!({"context": {}, "url": "data:text/html,<p>foo</p>"}),
        json!({"context": [], "url": "data:text/html,<p>foo</p>"}),
        json!({"context": context_id, "url": null}),
        json!({"context": context_id, "url": false}),
        json!({"context": context_id, "url": 42}),
        json!({"context": context_id, "url": {}}),
        json!({"context": context_id, "url": []}),
        json!({"context": context_id, "url": "http://:invalid"}),
        json!({"context": context_id, "url": "https://#invalid"}),
        json!({"context": context_id, "url": "data:text/html,<p>bar</p>", "wait": false}),
        json!({"context": context_id, "url": "data:text/html,<p>bar</p>", "wait": 42}),
        json!({"context": context_id, "url": "data:text/html,<p>bar</p>", "wait": {}}),
        json!({"context": context_id, "url": "data:text/html,<p>bar</p>", "wait": []}),
        json!({"context": context_id, "url": "data:text/html,<p>bar</p>", "wait": ""}),
        json!({"context": context_id, "url": "data:text/html,<p>bar</p>", "wait": "somestring"}),
    ] {
        id += 1;
        let response =
            send_bidi_command(&mut socket, id, "browsingContext.navigate", params.clone()).await;
        assert_bidi_error(
            &response,
            "invalid argument",
            &format!("navigate params should be invalid argument: {params}"),
        );
    }

    for params in [
        json!({"context": "", "url": "data:text/html,<p>foo</p>"}),
        json!({"context": "somestring", "url": "data:text/html,<p>foo</p>"}),
    ] {
        id += 1;
        let response =
            send_bidi_command(&mut socket, id, "browsingContext.navigate", params.clone()).await;
        assert_bidi_error(
            &response,
            "no such frame",
            &format!("navigate params should be no such frame: {params}"),
        );
    }

    for params in [
        json!({"context": null}),
        json!({"context": false}),
        json!({"context": 42}),
        json!({"context": {}}),
        json!({"context": []}),
        json!({"context": context_id, "ignoreCache": ""}),
        json!({"context": context_id, "ignoreCache": 42}),
        json!({"context": context_id, "ignoreCache": {}}),
        json!({"context": context_id, "ignoreCache": []}),
        json!({"context": context_id, "wait": false}),
        json!({"context": context_id, "wait": 42}),
        json!({"context": context_id, "wait": {}}),
        json!({"context": context_id, "wait": []}),
        json!({"context": context_id, "wait": ""}),
        json!({"context": context_id, "wait": "somestring"}),
    ] {
        id += 1;
        let response =
            send_bidi_command(&mut socket, id, "browsingContext.reload", params.clone()).await;
        assert_bidi_error(
            &response,
            "invalid argument",
            &format!("reload params should be invalid argument: {params}"),
        );
    }

    for params in [json!({"context": ""}), json!({"context": "somestring"})] {
        id += 1;
        let response =
            send_bidi_command(&mut socket, id, "browsingContext.reload", params.clone()).await;
        assert_bidi_error(
            &response,
            "no such frame",
            &format!("reload params should be no such frame: {params}"),
        );
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}
