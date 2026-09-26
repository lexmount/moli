use super::*;

#[tokio::test]
async fn websocket_bidi_network_add_and_remove_intercept_return_wpt_shape() {
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
    assert_eq!(
        create["type"],
        json!("success"),
        "create response: {create:?}"
    );
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context")
        .to_owned();

    let add = send_bidi_command(
        &mut socket,
        3,
        "network.addIntercept",
        json!({
            "phases": ["beforeRequestSent"],
            "urlPatterns": [],
            "contexts": [context_id]
        }),
    )
    .await;
    assert_eq!(
        add["type"],
        json!("success"),
        "addIntercept response: {add:?}"
    );
    let intercept = add["result"]["intercept"]
        .as_str()
        .expect("addIntercept should return an intercept id")
        .to_owned();
    assert_eq!(
        intercept, "00000000-0000-4000-8000-000000000003",
        "intercept id should use the protocol-neutral id generated for the command"
    );

    let remove = send_bidi_command(
        &mut socket,
        4,
        "network.removeIntercept",
        json!({
            "intercept": intercept
        }),
    )
    .await;
    assert_eq!(
        remove,
        json!({
            "type": "success",
            "id": 4,
            "result": {}
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_network_get_data_reads_subresource_fetch_body() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>network getData fetch</body></html>",
        )
    }

    async fn api(body: String) -> impl IntoResponse {
        assert_eq!(body, "bidi request body");
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/plain")],
            "subresource network body",
        )
    }

    let fixture_app = Router::new().route("/", get(page)).route("/api", post(api));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi network getData fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi network getData fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let page_url = format!("http://{fixture_addr}/");
    let api_url = format!("http://{fixture_addr}/api");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let navigate = send_bidi_command_response(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": page_url,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let subscribe = send_bidi_command_response(
        &mut socket,
        4,
        "session.subscribe",
        json!({
            "events": ["network.responseCompleted"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let add_collector = send_bidi_command_response(
        &mut socket,
        5,
        "network.addDataCollector",
        json!({
            "dataTypes": ["request", "response"],
            "maxEncodedDataSize": 1000,
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(
        add_collector["type"],
        json!("success"),
        "add collector: {add_collector:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": format!("fetch({api_url:?}, {{ method: 'POST', body: 'bidi request body' }}).then(response => response.text())"),
                    "target": { "context": context_id.clone() },
                    "awaitPromise": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send BiDi fetch evaluate");
    let mut messages = recv_until_id(&mut socket, 6).await;
    let evaluate = bidi_message_by_id(&messages, 6).clone();
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("subresource network body")
    );
    if !messages.iter().any(|message| {
        message["method"] == json!("network.responseCompleted")
            && message["params"]["response"]["url"] == json!(api_url)
    }) {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == json!("network.responseCompleted")
                    && message["params"]["response"]["url"] == json!(api_url)
            })
            .await,
        );
    }
    let response_completed = messages
        .iter()
        .find(|message| {
            message["method"] == json!("network.responseCompleted")
                && message["params"]["response"]["url"] == json!(api_url)
        })
        .expect("fetch responseCompleted event");
    let request_id = response_completed["params"]["request"]["request"]
        .as_str()
        .expect("fetch request id")
        .to_owned();

    let request_data = send_bidi_command_response(
        &mut socket,
        7,
        "network.getData",
        json!({
            "request": request_id.clone(),
            "dataType": "request"
        }),
    )
    .await;
    assert_eq!(
        request_data["type"],
        json!("success"),
        "getData request response: {request_data:?}"
    );
    assert_eq!(
        request_data["result"]["bytes"],
        json!({
            "type": "string",
            "value": "bidi request body",
        })
    );

    let data = send_bidi_command_response(
        &mut socket,
        8,
        "network.getData",
        json!({
            "request": request_id,
            "dataType": "response"
        }),
    )
    .await;
    assert_eq!(data["type"], json!("success"), "getData response: {data:?}");
    assert_eq!(
        data["result"]["bytes"],
        json!({
            "type": "string",
            "value": "subresource network body",
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_network_before_request_intercept_blocks_fetch_until_continue() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main id=\"ready\">network-before-request-intercept</main></body></html>",
        )
    }
    async fn data() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/plain")],
            "blocked network body",
        )
    }

    let fixture_app = Router::new()
        .route("/", get(page))
        .route("/data", get(data));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi request-stage intercept fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi request-stage intercept fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

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

    let navigate = send_bidi_command_response(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": fixture_url,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let subscribe = send_bidi_command(
        &mut socket,
        4,
        "session.subscribe",
        json!({
            "events": [
                "network.beforeRequestSent",
                "network.responseStarted",
                "network.responseCompleted",
                "network.fetchError"
            ],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let fetch_url = format!("http://{fixture_addr}/data");
    let add = send_bidi_command(
        &mut socket,
        5,
        "network.addIntercept",
        json!({
            "phases": ["beforeRequestSent"],
            "urlPatterns": [{
                "type": "string",
                "pattern": fetch_url.clone()
            }],
            "contexts": [context_id.clone()]
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
                "method": "script.evaluate",
                "params": {
                    "expression": format!(
                        "globalThis.__fetchResult = undefined; fetch({fetch_url:?}).then(response => response.text()).then(text => {{ globalThis.__fetchResult = text; }}).catch(error => {{ globalThis.__fetchResult = 'ERROR:' + error.name; }}); 'started'"
                    ),
                    "target": {
                        "context": context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate fetch starter");
    let mut messages = recv_until_id(&mut socket, 6).await;
    let start_result = messages
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .expect("script.evaluate start response");
    assert_eq!(start_result["type"], json!("success"));
    assert_eq!(start_result["result"]["result"]["value"], json!("started"));
    if !messages.iter().any(|message| {
        message["method"] == json!("network.beforeRequestSent")
            && message["params"]["request"]["url"].as_str() == Some(fetch_url.as_str())
            && message["params"]["isBlocked"] == json!(true)
    }) {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == json!("network.beforeRequestSent")
                    && message["params"]["request"]["url"].as_str() == Some(fetch_url.as_str())
                    && message["params"]["isBlocked"] == json!(true)
            })
            .await,
        );
    }

    assert!(
        messages
            .iter()
            .all(|message| message["method"] != json!("network.responseCompleted")),
        "responseCompleted must not emit before continueRequest: {messages:?}"
    );
    let before_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("network.beforeRequestSent")
                && message["params"]["request"]["url"].as_str() == Some(fetch_url.as_str())
                && message["params"]["isBlocked"] == json!(true)
        })
        .expect("network.beforeRequestSent event");
    assert_eq!(before_request["type"], json!("event"));
    assert_eq!(before_request["params"]["context"], json!(context_id));
    assert_eq!(
        before_request["params"]["isBlocked"],
        json!(true),
        "request-stage intercept should block matching fetch: {before_request:?}; messages={messages:?}"
    );
    assert_eq!(before_request["params"]["intercepts"], json!([intercept]));
    assert_eq!(before_request["params"]["request"]["method"], json!("GET"));
    let request_id = before_request["params"]["request"]["request"]
        .as_str()
        .expect("blocked request id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "network.continueRequest",
                "params": {
                    "request": request_id
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send network.continueRequest");
    let mut continue_messages = recv_until_id(&mut socket, 7).await;
    let continue_request = continue_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("network.continueRequest response");
    assert_eq!(
        continue_request["type"],
        json!("success"),
        "continueRequest should release the request: {continue_messages:?}"
    );
    if !continue_messages
        .iter()
        .any(|message| message["method"] == json!("network.responseCompleted"))
    {
        continue_messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == json!("network.responseCompleted")
            })
            .await,
        );
    }

    let completed = continue_messages
        .iter()
        .find(|message| message["method"] == json!("network.responseCompleted"))
        .expect("network.responseCompleted after continue");
    assert_eq!(completed["params"]["context"], json!(context_id));
    assert_eq!(completed["params"]["request"]["url"], json!(fetch_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "globalThis.__fetchResult",
                    "target": {
                        "context": context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate fetch result");
    let result_messages = recv_until_id(&mut socket, 8).await;
    let fetch_result = result_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("script.evaluate fetch result response");
    assert_eq!(
        fetch_result["result"]["result"]["value"],
        json!("blocked network body"),
        "continued request should resolve the page fetch: {fetch_result:?}; messages={result_messages:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_network_response_started_intercept_blocks_fetch_until_continue() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main id=\"ready\">network-response-started-intercept</main></body></html>",
        )
    }
    async fn data() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/plain")],
            "response-stage network body",
        )
    }

    let fixture_app = Router::new()
        .route("/", get(page))
        .route("/data", get(data));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi response-stage intercept fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi response-stage intercept fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

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

    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": fixture_url,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let subscribe = send_bidi_command(
        &mut socket,
        4,
        "session.subscribe",
        json!({
            "events": [
                "network.beforeRequestSent",
                "network.responseStarted",
                "network.responseCompleted",
                "network.fetchError"
            ],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let fetch_url = format!("http://{fixture_addr}/data");
    let add = send_bidi_command(
        &mut socket,
        5,
        "network.addIntercept",
        json!({
            "phases": ["responseStarted"],
            "urlPatterns": [{
                "type": "string",
                "pattern": fetch_url.clone()
            }],
            "contexts": [context_id.clone()]
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
                "method": "script.evaluate",
                "params": {
                    "expression": format!(
                        "globalThis.__fetchResult = undefined; fetch({fetch_url:?}).then(response => response.text()).then(text => {{ globalThis.__fetchResult = text; }}).catch(error => {{ globalThis.__fetchResult = 'ERROR:' + error.name; }}); 'started'"
                    ),
                    "target": {
                        "context": context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate fetch starter");
    let mut messages = recv_until_id(&mut socket, 6).await;
    let start_result = messages
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .expect("script.evaluate start response");
    assert_eq!(start_result["type"], json!("success"));
    assert_eq!(start_result["result"]["result"]["value"], json!("started"));
    if !messages.iter().any(|message| {
        message["method"] == json!("network.responseStarted")
            && message["params"]["request"]["url"].as_str() == Some(fetch_url.as_str())
            && message["params"]["isBlocked"] == json!(true)
    }) {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == json!("network.responseStarted")
                    && message["params"]["request"]["url"].as_str() == Some(fetch_url.as_str())
                    && message["params"]["isBlocked"] == json!(true)
            })
            .await,
        );
    }

    assert!(
        messages
            .iter()
            .all(|message| message["method"] != json!("network.responseCompleted")),
        "responseCompleted must not emit before continueResponse: {messages:?}"
    );
    let response_started = messages
        .iter()
        .find(|message| {
            message["method"] == json!("network.responseStarted")
                && message["params"]["request"]["url"].as_str() == Some(fetch_url.as_str())
                && message["params"]["isBlocked"] == json!(true)
        })
        .expect("network.responseStarted event");
    assert_eq!(response_started["type"], json!("event"));
    assert_eq!(response_started["params"]["context"], json!(context_id));
    assert_eq!(response_started["params"]["intercepts"], json!([intercept]));
    assert_eq!(
        response_started["params"]["request"]["method"],
        json!("GET")
    );
    assert_eq!(response_started["params"]["response"]["status"], json!(200));
    let request_id = response_started["params"]["request"]["request"]
        .as_str()
        .expect("blocked response request id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "network.continueResponse",
                "params": {
                    "request": request_id
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send network.continueResponse");
    let mut continue_messages = recv_until_id(&mut socket, 7).await;
    let continue_response = continue_messages
        .iter()
        .find(|message| message["id"] == json!(7_u64))
        .expect("network.continueResponse response");
    assert_eq!(
        continue_response["type"],
        json!("success"),
        "continueResponse should release the response: {continue_messages:?}"
    );
    if !continue_messages
        .iter()
        .any(|message| message["method"] == json!("network.responseCompleted"))
    {
        continue_messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == json!("network.responseCompleted")
            })
            .await,
        );
    }

    let completed = continue_messages
        .iter()
        .find(|message| message["method"] == json!("network.responseCompleted"))
        .expect("network.responseCompleted after continue");
    assert_eq!(completed["params"]["context"], json!(context_id));
    assert_eq!(completed["params"]["request"]["url"], json!(fetch_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "globalThis.__fetchResult",
                    "target": {
                        "context": context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate fetch result");
    let result_messages = recv_until_id(&mut socket, 8).await;
    let fetch_result = result_messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("script.evaluate fetch result response");
    assert_eq!(
        fetch_result["result"]["result"]["value"],
        json!("response-stage network body"),
        "continued response should resolve the page fetch: {fetch_result:?}; messages={result_messages:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_network_auth_required_intercept_blocks_matching_request() {
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
                "Basic realm=\"testrealm\"",
            )],
            "auth required",
        )
            .into_response()
    }

    let fixture_app = Router::new().route("/auth", get(auth));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi authRequired fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi authRequired fixture addr");
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
                "network.beforeRequestSent",
                "network.responseStarted",
                "network.authRequired",
                "network.responseCompleted"
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
    let intercept = add["result"]["intercept"]
        .as_str()
        .expect("network.addIntercept should return intercept id")
        .to_owned();

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

    let before_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("network.beforeRequestSent")
                && message["params"]["request"]["url"].as_str() == Some(auth_url.as_str())
        })
        .expect("network.beforeRequestSent auth event");
    assert_eq!(before_request["type"], json!("event"));
    assert_eq!(before_request["params"]["context"], json!(context_id));
    assert_eq!(before_request["params"]["isBlocked"], json!(false));
    assert_eq!(before_request["params"]["request"]["url"], json!(auth_url));
    assert_eq!(before_request["params"]["request"]["method"], json!("GET"));

    let auth_required = messages
        .iter()
        .find(|message| message["method"] == json!("network.authRequired"))
        .expect("network.authRequired event");
    assert_eq!(auth_required["type"], json!("event"));
    assert_eq!(auth_required["params"]["context"], json!(context_id));
    assert_eq!(
        auth_required["params"]["isBlocked"],
        json!(true),
        "authRequired event should be blocked: {auth_required:?}; messages: {messages:?}"
    );
    assert_eq!(auth_required["params"]["intercepts"], json!([intercept]));
    assert_eq!(auth_required["params"]["request"]["url"], json!(auth_url));
    assert_eq!(auth_required["params"]["request"]["method"], json!("GET"));
    assert_eq!(auth_required["params"]["response"]["status"], json!(401));
    assert_eq!(
        auth_required["params"]["response"]["authChallenges"],
        json!([{
            "scheme": "Basic",
            "realm": "testrealm"
        }])
    );
    assert!(
        messages
            .iter()
            .all(|message| message["method"] != json!("network.responseCompleted")),
        "authRequired intercept should keep the request blocked: {messages:?}"
    );

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
                    "action": "cancel"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send network.continueWithAuth cancel");
    let continue_messages = recv_until_id(&mut socket, 6).await;
    let continue_auth = continue_messages
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .expect("network.continueWithAuth response");
    assert_eq!(continue_auth["type"], json!("success"));

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_network_continue_response_provides_auth_credentials() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/network/continue_response/credentials.py.
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
                "Basic realm=\"continue-response\"",
            )],
            "auth required",
        )
            .into_response()
    }

    let fixture_app = Router::new().route("/auth", get(auth));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi continueResponse auth fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi continueResponse auth fixture addr");
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
                "network.beforeRequestSent",
                "network.responseStarted",
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
    let auth_required = messages
        .iter()
        .find(|message| message["method"] == json!("network.authRequired"))
        .expect("network.authRequired event");
    assert_eq!(auth_required["params"]["context"], json!(context_id));
    assert_eq!(auth_required["params"]["isBlocked"], json!(true));
    assert_eq!(auth_required["params"]["request"]["url"], json!(auth_url));
    assert_eq!(auth_required["params"]["response"]["status"], json!(401));
    assert!(
        messages
            .iter()
            .all(|message| message["method"] != json!("network.responseCompleted")),
        "authRequired pause should block response completion before credentials: {messages:?}"
    );

    let request_id = auth_required["params"]["request"]["request"]
        .as_str()
        .expect("authRequired request id")
        .to_owned();
    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "network.continueResponse",
                "params": {
                    "request": request_id,
                    "credentials": {
                        "type": "password",
                        "username": "user",
                        "password": "secret"
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send network.continueResponse credentials");
    let mut continue_messages = recv_until_id(&mut socket, 6).await;
    let continue_response = continue_messages
        .iter()
        .find(|message| message["id"] == json!(6_u64))
        .expect("network.continueResponse response");
    assert_eq!(
        continue_response["type"],
        json!("success"),
        "continueResponse credentials should release the authRequired pause: {continue_messages:?}"
    );
    if !continue_messages
        .iter()
        .any(|message| message["method"] == json!("network.responseCompleted"))
    {
        continue_messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == json!("network.responseCompleted")
            })
            .await,
        );
    }

    let completed = continue_messages
        .iter()
        .find(|message| message["method"] == json!("network.responseCompleted"))
        .expect("network.responseCompleted after credentials");
    assert_eq!(completed["params"]["context"], json!(context_id));
    assert_eq!(completed["params"]["request"]["url"], json!(auth_url));
    assert_eq!(completed["params"]["response"]["status"], json!(200));

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_storage_cookie_commands_execute_through_devtools_runtime() {
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
    let session = recv_ws_json(&mut socket).await;
    assert_eq!(session["type"], json!("success"));

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
                "method": "storage.setCookie",
                "params": {
                    "cookie": {
                        "name": "sid",
                        "value": {
                            "type": "string",
                            "value": "abc"
                        },
                        "domain": "example.test",
                        "path": "/",
                        "httpOnly": true,
                        "secure": true,
                        "sameSite": "lax"
                    },
                    "partition": {
                        "type": "context",
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send storage.setCookie");
    let set_cookie = recv_ws_json(&mut socket).await;
    assert_eq!(set_cookie["type"], json!("success"), "{set_cookie}");
    assert_eq!(set_cookie["id"], json!(3_u64));
    assert_eq!(set_cookie["result"]["partitionKey"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "storage.getCookies",
                "params": {
                    "filter": {
                        "name": "sid",
                        "domain": "example.test",
                        "path": "/",
                        "sameSite": "lax"
                    },
                    "partition": {
                        "type": "context",
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send storage.getCookies");
    let cookies = recv_ws_json(&mut socket).await;
    assert_eq!(cookies["type"], json!("success"));
    assert_eq!(cookies["id"], json!(4_u64));
    assert_eq!(cookies["result"]["partitionKey"], json!({}));
    assert_eq!(
        cookies["result"]["cookies"],
        json!([{
            "name": "sid",
            "value": {
                "type": "string",
                "value": "abc"
            },
            "domain": "example.test",
            "path": "/",
            "size": 6,
            "httpOnly": true,
            "secure": true,
            "sameSite": "lax"
        }])
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "storage.deleteCookies",
                "params": {
                    "filter": {
                        "name": "sid",
                        "domain": "example.test"
                    },
                    "partition": {
                        "type": "context",
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send storage.deleteCookies");
    let deleted = recv_ws_json(&mut socket).await;
    assert_eq!(deleted["type"], json!("success"));
    assert_eq!(deleted["id"], json!(5_u64));
    assert_eq!(deleted["result"]["partitionKey"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "storage.getCookies",
                "params": {
                    "filter": {
                        "name": "sid",
                        "domain": "example.test"
                    },
                    "partition": {
                        "type": "context",
                        "context": context_id
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send storage.getCookies after delete");
    let after_delete = recv_ws_json(&mut socket).await;
    assert_eq!(after_delete["type"], json!("success"));
    assert_eq!(after_delete["id"], json!(6_u64));
    assert_eq!(after_delete["result"]["cookies"], json!([]));

    let _ = socket.close(None).await;
    protocol_server.abort();
}
