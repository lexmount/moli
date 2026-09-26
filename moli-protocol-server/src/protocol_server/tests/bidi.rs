use super::*;

fn bidi_window_realm<'a>(
    response: &'a serde_json::Value,
    context_id: &str,
) -> &'a serde_json::Value {
    assert_eq!(
        response["type"],
        json!("success"),
        "script.getRealms should succeed; response={response:?}"
    );
    response["result"]["realms"]
        .as_array()
        .expect("script.getRealms should return an array")
        .iter()
        .find(|realm| {
            realm["context"] == json!(context_id)
                && realm["type"] == json!("window")
                && realm["origin"].as_str().is_some()
                && realm
                    .as_object()
                    .map(|object| !object.contains_key("sandbox"))
                    .unwrap_or(true)
                && realm["realm"]
                    .as_str()
                    .is_some_and(|realm| !realm.is_empty())
        })
        .unwrap_or_else(|| {
            panic!(
                "script.getRealms should include a default window realm for context {context_id}; response={response:?}"
            )
        })
}

fn bidi_sandbox_window_realm<'a>(
    response: &'a serde_json::Value,
    context_id: &str,
    sandbox: &str,
) -> &'a serde_json::Value {
    assert_eq!(
        response["type"],
        json!("success"),
        "script.getRealms should succeed; response={response:?}"
    );
    response["result"]["realms"]
        .as_array()
        .expect("script.getRealms should return an array")
        .iter()
        .find(|realm| {
            realm["context"] == json!(context_id)
                && realm["type"] == json!("window")
                && realm["sandbox"] == json!(sandbox)
                && realm["origin"].as_str().is_some()
                && realm["realm"]
                    .as_str()
                    .is_some_and(|realm| !realm.is_empty())
        })
        .unwrap_or_else(|| {
            panic!(
                "script.getRealms should include sandbox `{sandbox}` for context {context_id}; response={response:?}"
            )
        })
}

fn bidi_default_window_realm_id(response: &serde_json::Value, context_id: &str) -> String {
    bidi_window_realm(response, context_id)["realm"]
        .as_str()
        .expect("window realm id")
        .to_owned()
}

fn bidi_remote_object_property<'a>(
    remote: &'a serde_json::Value,
    property_name: &str,
) -> &'a serde_json::Value {
    remote["value"]
        .as_array()
        .and_then(|properties| {
            properties.iter().find_map(|property| {
                let pair = property.as_array()?;
                (pair.len() == 2
                    && pair.first().and_then(serde_json::Value::as_str) == Some(property_name))
                .then(|| &pair[1])
            })
        })
        .unwrap_or_else(|| {
            panic!("remote object should include property {property_name}: {remote:?}")
        })
}

fn assert_bidi_script_exception_result(
    response: &serde_json::Value,
    id: u64,
    expected_value: &str,
) {
    assert_eq!(
        response["type"],
        json!("success"),
        "script command should succeed with an exception result: {response:?}"
    );
    assert_eq!(response["id"], json!(id));
    assert_eq!(response["result"]["type"], json!("exception"));
    assert!(
        response["result"]["realm"]
            .as_str()
            .is_some_and(|realm| !realm.is_empty()),
        "BiDi exception result should identify the realm: {response:?}"
    );
    let details = &response["result"]["exceptionDetails"];
    assert!(
        details["lineNumber"].as_u64().is_some(),
        "BiDi exceptionDetails should include lineNumber: {response:?}"
    );
    assert!(
        details["columnNumber"].as_u64().is_some(),
        "BiDi exceptionDetails should include columnNumber: {response:?}"
    );
    assert!(
        details["stackTrace"]["callFrames"].as_array().is_some(),
        "BiDi exceptionDetails should include stackTrace.callFrames: {response:?}"
    );
    assert!(
        details["text"]
            .as_str()
            .is_some_and(|text| !text.is_empty()),
        "BiDi exceptionDetails should include text: {response:?}"
    );
    assert_eq!(details["exception"]["type"], json!("string"));
    assert_eq!(details["exception"]["value"], json!(expected_value));
}

fn assert_bidi_script_exception_remote_handle(
    response: &serde_json::Value,
    id: u64,
    should_contain_handle: bool,
) {
    assert_eq!(
        response["type"],
        json!("success"),
        "script command should succeed with an exception result: {response:?}"
    );
    assert_eq!(response["id"], json!(id));
    assert_eq!(response["result"]["type"], json!("exception"));
    let exception = &response["result"]["exceptionDetails"]["exception"];
    assert_eq!(
        exception["type"],
        json!("object"),
        "exception remote value should preserve object type: {response:?}"
    );
    assert_eq!(
        exception
            .get("handle")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        should_contain_handle,
        "exception remote value handle presence should follow resultOwnership: {response:?}"
    );
}

async fn bidi_location_href(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    context_id: &str,
) -> String {
    socket
        .send(WsMessage::Text(
            json!({
                "id": id,
                "method": "script.evaluate",
                "params": {
                    "expression": "location.href",
                    "target": {
                        "context": context_id
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send location.href script.evaluate");
    let response = recv_ws_json(socket).await;
    assert_eq!(
        response["type"],
        json!("success"),
        "location.href evaluation should succeed: {response:?}"
    );
    assert_eq!(
        response["result"]["type"],
        json!("success"),
        "location.href evaluation should return success result: {response:?}"
    );
    response["result"]["result"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("location.href should return a string: {response:?}"))
        .to_owned()
}

async fn bidi_string_script_value(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    context_id: &str,
    expression: &str,
) -> String {
    socket
        .send(WsMessage::Text(
            json!({
                "id": id,
                "method": "script.evaluate",
                "params": {
                    "expression": expression,
                    "target": {
                        "context": context_id
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send string script.evaluate");
    let response = recv_ws_json(socket).await;
    assert_eq!(
        response["type"],
        json!("success"),
        "string script evaluation should succeed: {response:?}"
    );
    assert_eq!(
        response["result"]["type"],
        json!("success"),
        "string script evaluation should return success result: {response:?}"
    );
    response["result"]["result"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("string script should return a string: {response:?}"))
        .to_owned()
}

async fn bidi_awaited_string_script_value(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    context_id: &str,
    expression: &str,
) -> String {
    socket
        .send(WsMessage::Text(
            json!({
                "id": id,
                "method": "script.evaluate",
                "params": {
                    "expression": expression,
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
        .expect("send awaited string script.evaluate");
    let response = recv_ws_json(socket).await;
    assert_eq!(
        response["type"],
        json!("success"),
        "awaited string script evaluation should succeed: {response:?}"
    );
    assert_eq!(
        response["result"]["type"],
        json!("success"),
        "awaited string script evaluation should return success result: {response:?}"
    );
    response["result"]["result"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("awaited string script should return a string: {response:?}"))
        .to_owned()
}

async fn bidi_viewport_surface(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    context_id: &str,
) -> serde_json::Value {
    // BiDi events share the websocket with command responses and may precede
    // them. Chromium's test client likewise routes WaitForResponse by command
    // id instead of treating the next websocket frame as the response.
    let response = send_bidi_command_response(
        socket,
        id,
        "script.evaluate",
        json!({
            "expression": "JSON.stringify({ width: innerWidth, height: innerHeight, dpr: devicePixelRatio })",
            "target": {
                "context": context_id
            }
        }),
    )
    .await;
    assert_eq!(
        response["type"],
        json!("success"),
        "viewport surface evaluation should succeed: {response:?}"
    );
    assert_eq!(
        response["result"]["type"],
        json!("success"),
        "viewport surface evaluation should return success result: {response:?}"
    );
    let payload = response["result"]["result"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("viewport surface should return a JSON string: {response:?}"));
    serde_json::from_str(payload)
        .unwrap_or_else(|error| panic!("viewport surface JSON should parse: {error}; {payload}"))
}

async fn bidi_focus_visibility_surface(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    context_id: &str,
) -> serde_json::Value {
    socket
        .send(WsMessage::Text(
            json!({
                "id": id,
                "method": "script.evaluate",
                "params": {
                    "expression": "JSON.stringify({ hasFocus: document.hasFocus(), hidden: document.hidden, visibilityState: document.visibilityState })",
                    "target": {
                        "context": context_id
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send focus/visibility surface script.evaluate");
    let messages = recv_until_id(socket, id).await;
    let response = bidi_message_by_id(&messages, id);
    assert_eq!(
        response["type"],
        json!("success"),
        "focus/visibility surface evaluation should succeed: {response:?}"
    );
    assert_eq!(
        response["result"]["type"],
        json!("success"),
        "focus/visibility surface evaluation should return success result: {response:?}"
    );
    let payload = response["result"]["result"]["value"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("focus/visibility surface should return a JSON string: {response:?}")
        });
    serde_json::from_str(payload).unwrap_or_else(|error| {
        panic!("focus/visibility surface JSON should parse: {error}; {payload}")
    })
}

async fn bidi_session_with_context(
    cdp_addr: std::net::SocketAddr,
) -> (
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    String,
) {
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
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();
    (socket, context_id)
}

async fn classic_new_session_on_server(addr: std::net::SocketAddr) -> String {
    classic_new_session_on_server_with_body(addr, json!({})).await
}

async fn classic_new_session_on_server_with_body(
    addr: std::net::SocketAddr,
    body: serde_json::Value,
) -> String {
    let session = classic_request_on_server_with_body(addr, "POST", "/session", body).await;
    session["value"]["sessionId"]
        .as_str()
        .expect("Classic session id")
        .to_owned()
}

async fn classic_request_on_server_with_body(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to Classic HTTP server");
    let body = body.to_string();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write Classic new session request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read Classic new session response");
    let response = String::from_utf8(response).expect("Classic new session utf-8 response");
    assert!(
        response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200"),
        "Classic {method} {path} returned unexpected response: {response:?}"
    );
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .expect("Classic response body");
    serde_json::from_str(body).expect("Classic response json")
}

async fn classic_request_status_on_server_with_body(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: serde_json::Value,
) -> (u16, serde_json::Value) {
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to Classic HTTP server");
    let body = body.to_string();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write Classic request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read Classic response");
    let response = String::from_utf8(response).expect("Classic response utf-8");
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .expect("Classic HTTP status");
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .expect("Classic response body");
    (
        status,
        serde_json::from_str(body).expect("Classic response json"),
    )
}

fn classic_data_url_for_bidi_test(html: &str) -> String {
    fn push_hex(encoded: &mut String, byte: u8) {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        encoded.push('%');
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }

    let mut encoded = String::with_capacity(html.len());
    for byte in html.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => push_hex(&mut encoded, byte),
        }
    }
    format!("data:text/html;charset=utf-8,{encoded}")
}

fn service_worker_context_created<'a>(
    messages: &'a [serde_json::Value],
    worker_url: &str,
) -> Option<&'a serde_json::Value> {
    messages.iter().find(|message| {
        message["method"] == json!("browsingContext.contextCreated")
            && message["params"]["url"] == json!(worker_url)
    })
}

fn service_worker_realm_created(messages: &[serde_json::Value]) -> Option<&serde_json::Value> {
    messages.iter().find(|message| {
        message["method"] == json!("script.realmCreated")
            && message["params"]["type"] == json!("service-worker")
    })
}

fn service_worker_log_entry<'a>(
    messages: &'a [serde_json::Value],
    service_worker_context: &str,
) -> Option<&'a serde_json::Value> {
    messages.iter().find(|message| {
        message["method"] == json!("log.entryAdded")
            && message["params"]["text"] == json!("classic-bidi-service-worker-log")
            && message["params"]["source"]["context"] == json!(service_worker_context)
    })
}

async fn connect_classic_session_bidi_socket(
    addr: std::net::SocketAddr,
    session_id: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    connect_async(format!("ws://{addr}/session/{session_id}"))
        .await
        .expect("connect Classic-session BiDi websocket")
        .0
}

async fn send_bidi_command(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    socket
        .send(WsMessage::Text(
            json!({
                "id": id,
                "method": method,
                "params": params
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send BiDi command");
    recv_ws_json(socket).await
}

async fn send_bidi_command_with_channel(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    method: &str,
    params: serde_json::Value,
    channel: &str,
) -> serde_json::Value {
    socket
        .send(WsMessage::Text(
            json!({
                "id": id,
                "method": method,
                "params": params,
                "goog:channel": channel
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send BiDi command");
    recv_ws_json(socket).await
}

async fn send_bidi_command_response(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    socket
        .send(WsMessage::Text(
            json!({
                "id": id,
                "method": method,
                "params": params
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send BiDi command");
    let messages = recv_until_id(socket, id).await;
    bidi_message_by_id(&messages, id).clone()
}

async fn send_bidi_script_call_function_and_collect_messages(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    context_id: &str,
    function_declaration: &str,
    arguments: Vec<serde_json::Value>,
    expected_script_message_count: usize,
) -> Vec<serde_json::Value> {
    socket
        .send(WsMessage::Text(
            json!({
                "id": id,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": function_declaration,
                    "arguments": arguments,
                    "awaitPromise": false,
                    "target": {
                        "context": context_id
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.callFunction");
    let messages = recv_until_id(socket, id).await;
    collect_bidi_messages_until_method_count(
        socket,
        messages,
        "script.message",
        expected_script_message_count,
    )
    .await
}

async fn send_bidi_add_preload_script(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    function_declaration: &str,
    arguments: Vec<serde_json::Value>,
) -> String {
    let response = send_bidi_command(
        socket,
        id,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": function_declaration,
            "arguments": arguments
        }),
    )
    .await;
    assert_eq!(
        response["type"],
        json!("success"),
        "script.addPreloadScript should succeed: {response:?}"
    );
    response["result"]["script"]
        .as_str()
        .expect("preload script id")
        .to_owned()
}

async fn remove_bidi_preload_script(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    script: &str,
) {
    let response = send_bidi_command(
        socket,
        id,
        "script.removePreloadScript",
        json!({ "script": script }),
    )
    .await;
    assert_eq!(
        response["type"],
        json!("success"),
        "script.removePreloadScript should succeed: {response:?}"
    );
    assert_eq!(response["result"], json!({}));
}

async fn create_bidi_context_and_collect_script_messages(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    expected_script_message_count: usize,
) -> Vec<serde_json::Value> {
    socket
        .send(WsMessage::Text(
            json!({
                "id": id,
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
    let messages = recv_until_id(socket, id).await;
    collect_bidi_messages_until_method_count(
        socket,
        messages,
        "script.message",
        expected_script_message_count,
    )
    .await
}

fn assert_locate_nodes_two_divs(response: &serde_json::Value) {
    let nodes = response["result"]["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("locateNodes result should contain nodes: {response:?}"));
    assert_eq!(nodes.len(), 2, "locateNodes should find both divs");
    for (node, data_class) in nodes.iter().zip(["one", "two"]) {
        assert_eq!(node["type"], json!("node"), "node remote type: {node:?}");
        assert!(
            node["sharedId"]
                .as_str()
                .is_some_and(|shared_id| !shared_id.is_empty()),
            "node should include sharedId: {node:?}"
        );
        assert_eq!(node["value"]["nodeType"], json!(1));
        assert_eq!(node["value"]["localName"], json!("div"));
        assert_eq!(
            node["value"]["namespaceURI"],
            json!("http://www.w3.org/1999/xhtml")
        );
        assert_eq!(node["value"]["childNodeCount"], json!(1));
        assert_eq!(node["value"]["attributes"]["data-class"], json!(data_class));
    }
}

async fn collect_bidi_messages_until_method_count(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    mut messages: Vec<serde_json::Value>,
    method: &str,
    expected_count: usize,
) -> Vec<serde_json::Value> {
    while bidi_events_by_method(&messages, method).len() < expected_count {
        messages.push(
            timeout(Duration::from_secs(1), recv_ws_json(socket))
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "{method} event should arrive; expected_count={expected_count}; messages={messages:#?}"
                    )
                }),
        );
    }
    messages
}

async fn collect_bidi_messages_until(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    mut messages: Vec<serde_json::Value>,
    mut predicate: impl FnMut(&[serde_json::Value]) -> bool,
    description: &str,
) -> Vec<serde_json::Value> {
    while !predicate(&messages) {
        messages.push(
            timeout(Duration::from_secs(1), recv_ws_json(socket))
                .await
                .unwrap_or_else(|_| panic!("{description} should arrive; messages={messages:#?}")),
        );
    }
    messages
}

fn bidi_message_by_id(messages: &[serde_json::Value], id: u64) -> &serde_json::Value {
    messages
        .iter()
        .find(|message| message["id"] == json!(id))
        .unwrap_or_else(|| panic!("expected BiDi response id {id}: {messages:#?}"))
}

fn bidi_events_by_method<'a>(
    messages: &'a [serde_json::Value],
    method: &str,
) -> Vec<&'a serde_json::Value> {
    messages
        .iter()
        .filter(|message| message["method"] == json!(method))
        .collect()
}

fn bidi_user_context_ids(response: &serde_json::Value) -> Vec<String> {
    response["result"]["userContexts"]
        .as_array()
        .expect("userContexts array")
        .iter()
        .map(|context| {
            context["userContext"]
                .as_str()
                .expect("userContext string")
                .to_owned()
        })
        .collect()
}

fn assert_bidi_error(response: &serde_json::Value, expected_error: &str, context: &str) {
    assert_eq!(
        response["type"],
        json!("error"),
        "{context}; response={response:?}"
    );
    assert_eq!(
        response["error"],
        json!(expected_error),
        "{context}; response={response:?}"
    );
}
mod extracted;
