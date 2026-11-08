use super::*;

const ELEMENT_KEY: &str = "element-6066-11e4-a52e-4f735466cecf";

#[tokio::test]
async fn websocket_bidi_and_classic_share_node_identity_when_classic_finds_first() {
    let (addr, server) = spawn_test_protocol_server().await;
    let session = classic_new_session_on_server(addr).await;
    let mut socket = connect_classic_session_bidi_socket(addr, &session).await;
    let tree = send_bidi_command(&mut socket, 1, "browsingContext.getTree", json!({})).await;
    let context = tree["result"]["contexts"][0]["context"].as_str().unwrap();
    let navigated = send_bidi_command(
        &mut socket,
        2,
        "browsingContext.navigate",
        json!({
            "context": context,
            "url": classic_data_url_for_bidi_test("<!doctype html><h1>Classic first</h1>"),
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigated["type"], "success", "{navigated}");
    let first = classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/element"),
        json!({"using": "css selector", "value": "h1"}),
    )
    .await;
    let classic_id = first["value"][ELEMENT_KEY].as_str().unwrap();
    // Use the Classic reference before BiDi has exported or located this node.
    let direct = send_bidi_command(
        &mut socket,
        3,
        "script.callFunction",
        json!({
            "target": {"context": context},
            "functionDeclaration": "function(node) { return node; }",
            "arguments": [{"sharedId": classic_id}], "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(direct["type"], "success", "{direct}");
    assert_eq!(
        direct["result"]["result"]["sharedId"], classic_id,
        "{direct}"
    );
    let located = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.locateNodes",
        json!({"context": context, "locator": {"type": "css", "value": "h1"}}),
    )
    .await;
    let shared_id = located["result"]["nodes"][0]["sharedId"].as_str().unwrap();
    assert_eq!(shared_id, classic_id, "{located}");
    let imported = classic_request_on_server_with_body(
        addr,
        "GET",
        &format!("/session/{session}/element/{shared_id}/text"),
        json!({}),
    )
    .await;
    assert_eq!(imported["value"], "Classic first", "{imported}");
    let repeated = classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/element"),
        json!({"using": "css selector", "value": "h1"}),
    )
    .await;
    assert_eq!(repeated["value"][ELEMENT_KEY], classic_id, "{repeated}");
    // Both aliases still resolve, and returning either one reuses the first ID.
    for element_id in [classic_id, shared_id] {
        let echoed = classic_request_on_server_with_body(
            addr,
            "POST",
            &format!("/session/{session}/execute/sync"),
            json!({"script": "return arguments[0];", "args": [{ELEMENT_KEY: element_id}]}),
        )
        .await;
        assert_eq!(echoed["value"][ELEMENT_KEY], classic_id, "{echoed}");
    }
    let reused = send_bidi_command(
        &mut socket,
        5,
        "script.callFunction",
        json!({
            "target": {"context": context},
            "functionDeclaration": "function(node) { return node.textContent; }",
            "arguments": [{"sharedId": shared_id}], "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(
        reused["result"]["result"]["value"], "Classic first",
        "{reused}"
    );
    socket.close(None).await.unwrap();
    classic_request_on_server_with_body(addr, "DELETE", &format!("/session/{session}"), json!({}))
        .await;
    server.abort();
}

#[tokio::test]
async fn websocket_bidi_classic_element_round_trip_preserves_shared_id() {
    let (addr, server) = spawn_test_protocol_server().await;
    let session = classic_new_session_on_server(addr).await;
    let mut socket = connect_classic_session_bidi_socket(addr, &session).await;
    let tree = send_bidi_command(&mut socket, 1, "browsingContext.getTree", json!({})).await;
    let context = tree["result"]["contexts"][0]["context"].as_str().unwrap();
    let navigated = send_bidi_command(
        &mut socket,
        2,
        "browsingContext.navigate",
        json!({
            "context": context,
        "url": classic_data_url_for_bidi_test("<!doctype html><h1>Shared</h1><input type=file>"),
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigated["type"], "success", "{navigated}");
    let exported = send_bidi_command(
        &mut socket,
        3,
        "script.evaluate",
        json!({
            "target": {"context": context}, "expression": "document.querySelector('h1')", "awaitPromise": false
        }),
    )
    .await;
    let shared = exported["result"]["result"]["sharedId"].as_str().unwrap();
    let found = classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/element"),
        json!({"using": "css selector", "value": "h1"}),
    )
    .await;
    assert_eq!(found["value"][ELEMENT_KEY], shared, "{found}");
    let echoed = classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/execute/sync"),
        json!({
            "script": "return arguments[0];", "args": [{ELEMENT_KEY: shared}]
        }),
    )
    .await;
    assert_eq!(echoed["value"][ELEMENT_KEY], shared, "{echoed}");
    let reused = send_bidi_command(&mut socket, 4, "script.callFunction", json!({
        "target": {"context": context}, "functionDeclaration": "function(node) { return node.textContent; }",
        "arguments": [{"sharedId": echoed["value"][ELEMENT_KEY]}], "awaitPromise": false
    })).await;
    assert_eq!(reused["result"]["result"]["value"], "Shared", "{reused}");
    let subscribed = send_bidi_command(
        &mut socket,
        5,
        "session.subscribe",
        json!({
            "events": ["input.fileDialogOpened"], "contexts": [context]
        }),
    )
    .await;
    assert_eq!(subscribed["type"], "success", "{subscribed}");
    socket.send(WsMessage::Text(json!({
        "id": 6, "method": "script.evaluate", "params": {
            "target": {"context": context}, "expression": "document.querySelector('input').click(); undefined",
            "awaitPromise": false, "userActivation": true
        }
    }).to_string().into())).await.unwrap();
    let messages = recv_until_id(&mut socket, 6).await;
    let messages = collect_bidi_messages_until(
        &mut socket,
        messages,
        |messages| {
            messages
                .iter()
                .any(|m| m["method"] == "input.fileDialogOpened")
        },
        "the file chooser's shared node",
    )
    .await;
    let event = bidi_events_by_method(&messages, "input.fileDialogOpened")[0];
    let input = event["params"]["element"]["sharedId"].as_str().unwrap();
    let tag = classic_request_on_server_with_body(
        addr,
        "GET",
        &format!("/session/{session}/element/{input}/name"),
        json!({}),
    )
    .await;
    assert_eq!(tag["value"], "input", "{tag}");
    socket.close(None).await.unwrap();
    classic_request_on_server_with_body(addr, "DELETE", &format!("/session/{session}"), json!({}))
        .await;
    server.abort();
}

#[tokio::test]
async fn websocket_bidi_accepts_classic_script_active_and_shadow_references() {
    const SHADOW_KEY: &str = "shadow-6066-11e4-a52e-4f735466cecf";
    let (addr, server) = spawn_test_protocol_server().await;
    let session = classic_new_session_on_server(addr).await;
    let mut socket = connect_classic_session_bidi_socket(addr, &session).await;
    let tree = send_bidi_command(&mut socket, 1, "browsingContext.getTree", json!({})).await;
    let context = tree["result"]["contexts"][0]["context"].as_str().unwrap();
    let navigated = send_bidi_command(&mut socket, 2, "browsingContext.navigate", json!({
        "context": context,
        "url": classic_data_url_for_bidi_test("<!doctype html><h1>Script</h1><input><div id=host></div><script>document.querySelector('input').focus();document.querySelector('#host').attachShadow({mode:'open'}).innerHTML='<span>Shadow</span>';</script>"),
        "wait": "complete"
    })).await;
    assert_eq!(navigated["type"], "success", "{navigated}");
    let script = classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/execute/sync"),
        json!({"script": "return document.querySelector('h1');", "args": []}),
    )
    .await;
    let active = classic_request_on_server_with_body(
        addr,
        "GET",
        &format!("/session/{session}/element/active"),
        json!({}),
    )
    .await;
    let host = classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/element"),
        json!({"using": "css selector", "value": "#host"}),
    )
    .await;
    let host_id = host["value"][ELEMENT_KEY].as_str().unwrap();
    let shadow = classic_request_on_server_with_body(
        addr,
        "GET",
        &format!("/session/{session}/element/{host_id}/shadow"),
        json!({}),
    )
    .await;
    let all = classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/elements"),
        json!({"using": "css selector", "value": "input"}),
    )
    .await;
    assert_eq!(all["value"][0], active["value"]);
    // No BiDi node lookup has occurred: every reference originated in Classic.
    for (index, (value, key, expression)) in [
        (&script, ELEMENT_KEY, "node.textContent"),
        (&active, ELEMENT_KEY, "node.localName"),
        (&shadow, SHADOW_KEY, "node.firstElementChild.textContent"),
    ]
    .into_iter()
    .enumerate()
    {
        let id = value["value"][key].as_str().unwrap();
        let reused = send_bidi_command(&mut socket, 3+index as u64, "script.callFunction", json!({
            "target": {"context": context}, "functionDeclaration": format!("function(node) {{ return [node, {expression}]; }}"),
            "arguments": [{"sharedId": id}], "awaitPromise": false
        })).await;
        assert_eq!(reused["type"], "success", "{reused}");
        assert_eq!(
            reused["result"]["result"]["value"][0]["sharedId"], id,
            "{reused}"
        );
        assert_eq!(
            reused["result"]["result"]["value"][1]["value"],
            ["Script", "input", "Shadow"][index],
            "{reused}"
        );
        let echoed = classic_request_on_server_with_body(
            addr,
            "POST",
            &format!("/session/{session}/execute/sync"),
            json!({"script": "return arguments[0];", "args": [{key: id}]}),
        )
        .await;
        assert_eq!(echoed["value"][key], id, "{echoed}");
    }
    let shadow_id = shadow["value"][SHADOW_KEY].as_str().unwrap();
    let (status, wrong_element) = classic_request_status_on_server_with_body(
        addr,
        "GET",
        &format!("/session/{session}/element/{shadow_id}/text"),
        json!({}),
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(
        wrong_element["value"]["error"], "no such element",
        "{wrong_element}"
    );
    let (status, wrong_shadow) = classic_request_status_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/shadow/{host_id}/element"),
        json!({"using":"css selector","value":"span"}),
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(
        wrong_shadow["value"]["error"], "no such shadow root",
        "{wrong_shadow}"
    );
    let other_session = classic_new_session_on_server(addr).await;
    let (status, foreign) = classic_request_status_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{other_session}/shadow/{shadow_id}/element"),
        json!({"using":"css selector","value":"span"}),
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(
        foreign["value"]["error"], "no such shadow root",
        "{foreign}"
    );
    let replaced = send_bidi_command(&mut socket, 6, "browsingContext.navigate", json!({
        "context":context,"url":classic_data_url_for_bidi_test("<!doctype html><h1>Replacement</h1>"),"wait":"complete"
    })).await;
    assert_eq!(replaced["type"], "success", "{replaced}");
    let (status, detached) = classic_request_status_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/shadow/{shadow_id}/element"),
        json!({"using":"css selector","value":"span"}),
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(
        detached["value"]["error"], "detached shadow root",
        "{detached}"
    );
    socket.close(None).await.unwrap();
    for id in [&session, &other_session] {
        classic_request_on_server_with_body(addr, "DELETE", &format!("/session/{id}"), json!({}))
            .await;
    }
    server.abort();
}

#[tokio::test]
async fn websocket_bidi_classic_element_references_respect_current_frame() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture_addr = listener.local_addr().unwrap();
    let fixture = tokio::spawn(async move {
        axum::serve(listener, Router::new()
            .route("/", get(|| async { ([("content-type", "text/html")],
                "<!doctype html><h1>TOP</h1><iframe src='/a'></iframe><iframe src='/b'></iframe>") }))
            .route("/a", get(|| async { ([("content-type", "text/html")], "<!doctype html><h1>A</h1>") }))
            .route("/b", get(|| async { ([("content-type", "text/html")], "<!doctype html><h1>B</h1>") }))
        ).await
    });
    let (addr, server) = spawn_test_protocol_server().await;
    let session = classic_new_session_on_server(addr).await;
    let mut socket = connect_classic_session_bidi_socket(addr, &session).await;
    let tree = send_bidi_command(&mut socket, 1, "browsingContext.getTree", json!({})).await;
    let context = tree["result"]["contexts"][0]["context"].as_str().unwrap();
    let navigated = send_bidi_command(
        &mut socket,
        2,
        "browsingContext.navigate",
        json!({
            "context": context, "url": format!("http://{fixture_addr}/"), "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigated["type"], "success", "{navigated}");
    let tree = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.getTree",
        json!({"root": context}),
    )
    .await;
    let children = tree["result"]["contexts"][0]["children"]
        .as_array()
        .unwrap();
    assert_eq!(children.len(), 2, "{tree}");
    classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/frame"),
        json!({"id": 0}),
    )
    .await;
    let classic_child = classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/element"),
        json!({"using": "css selector", "value": "h1"}),
    )
    .await;
    let classic_child_id = classic_child["value"][ELEMENT_KEY].as_str().unwrap();
    let child_a = children
        .iter()
        .find(|child| child["url"].as_str().unwrap().ends_with("/a"))
        .unwrap();
    let direct_child = send_bidi_command(
        &mut socket,
        40,
        "script.callFunction",
        json!({
            "target": {"context": child_a["context"]},
            "functionDeclaration": "function(node) { return node.textContent; }",
            "arguments": [{"sharedId": classic_child_id}], "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(
        direct_child["result"]["result"]["value"], "A",
        "{direct_child}"
    );
    classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/frame"),
        json!({"id": null}),
    )
    .await;
    // Export from another same-origin realm: authority follows the node's
    // Document, not the realm in which the script happened to run.
    let from_top = send_bidi_command(
        &mut socket,
        6,
        "script.evaluate",
        json!({
            "target": {"context": context}, "expression": "frames[0].document.querySelector('h1')",
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(from_top["type"], "success", "{from_top}");
    let mut shared = Vec::new();
    for (index, path) in ["/a", "/b"].into_iter().enumerate() {
        let child = children
            .iter()
            .find(|child| child["url"].as_str().unwrap().ends_with(path))
            .unwrap();
        let located = send_bidi_command(
            &mut socket,
            4 + index as u64,
            "browsingContext.locateNodes",
            json!({
                "context": child["context"], "locator": {"type": "css", "value": "h1"}
            }),
        )
        .await;
        shared.push(
            located["result"]["nodes"][0]["sharedId"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    assert_eq!(
        from_top["result"]["result"]["sharedId"], shared[0],
        "{from_top}"
    );
    assert_eq!(shared[0], classic_child_id);
    // First use must not import an iframe node as belonging to the top frame.
    let (_, wrong_frame) = classic_request_status_on_server_with_body(
        addr,
        "GET",
        &format!("/session/{session}/element/{}/text", shared[0]),
        json!({}),
    )
    .await;
    assert_eq!(
        wrong_frame["value"]["error"], "no such element",
        "{wrong_frame}"
    );
    let switched = classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/frame"),
        json!({"id": 0}),
    )
    .await;
    assert_eq!(switched, json!({"value": null}));
    let text = classic_request_on_server_with_body(
        addr,
        "GET",
        &format!("/session/{session}/element/{}/text", shared[0]),
        json!({}),
    )
    .await;
    assert_eq!(text["value"], "A", "{text}");
    // An unimported sibling reference is also outside the current navigable.
    let (_, sibling) = classic_request_status_on_server_with_body(
        addr,
        "GET",
        &format!("/session/{session}/element/{}/text", shared[1]),
        json!({}),
    )
    .await;
    assert_eq!(sibling["value"]["error"], "no such element", "{sibling}");
    classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/frame"),
        json!({"id": null}),
    )
    .await;
    for frame in [None, Some(1)] {
        if let Some(index) = frame {
            classic_request_on_server_with_body(
                addr,
                "POST",
                &format!("/session/{session}/frame"),
                json!({"id": index}),
            )
            .await;
        }
        // A successful import must never allow a subsequent frame mismatch.
        let (_, wrong_frame) = classic_request_status_on_server_with_body(
            addr,
            "GET",
            &format!("/session/{session}/element/{}/text", shared[0]),
            json!({}),
        )
        .await;
        assert_eq!(
            wrong_frame["value"]["error"], "no such element",
            "{wrong_frame}"
        );
    }
    classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/frame"),
        json!({"id": null}),
    )
    .await;
    classic_request_on_server_with_body(
        addr,
        "POST",
        &format!("/session/{session}/frame"),
        json!({"id": 0}),
    )
    .await;
    let a_context = children
        .iter()
        .find(|child| child["url"].as_str().unwrap().ends_with("/a"))
        .unwrap()["context"]
        .as_str()
        .unwrap();
    let replaced = send_bidi_command(
        &mut socket,
        7,
        "browsingContext.navigate",
        json!({
            "context": a_context, "url": format!("http://{fixture_addr}/b"), "wait": "complete"
        }),
    )
    .await;
    assert_eq!(replaced["type"], "success", "{replaced}");
    let (_, stale) = classic_request_status_on_server_with_body(
        addr,
        "GET",
        &format!("/session/{session}/element/{}/text", shared[0]),
        json!({}),
    )
    .await;
    assert_eq!(
        stale["value"]["error"], "stale element reference",
        "{stale}"
    );
    socket.close(None).await.unwrap();
    classic_request_on_server_with_body(addr, "DELETE", &format!("/session/{session}"), json!({}))
        .await;
    server.abort();
    fixture.abort();
}
