use super::*;

#[tokio::test]
async fn websocket_bidi_existing_classic_session_shares_classic_runtime_context() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let session_id = classic_new_session_on_server(cdp_addr).await;
    let page_url = classic_data_url_for_bidi_test(
        "<!doctype html><title>shared classic bidi</title><main id='marker'>same document</main>",
    );
    let navigated = classic_request_on_server_with_body(
        cdp_addr,
        "POST",
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let mut socket = connect_classic_session_bidi_socket(cdp_addr, &session_id).await;
    let tree = send_bidi_command(&mut socket, 1, "browsingContext.getTree", json!({})).await;
    assert_eq!(tree["type"], json!("success"));
    let contexts = tree["result"]["contexts"]
        .as_array()
        .expect("getTree contexts");
    assert_eq!(
        contexts.len(),
        1,
        "attached BiDi should see the Classic-owned top-level context: {tree:?}"
    );
    assert_eq!(contexts[0]["url"], json!(page_url));
    let context_id = contexts[0]["context"]
        .as_str()
        .expect("attached Classic context id")
        .to_owned();

    let title = send_bidi_command(
        &mut socket,
        2,
        "script.evaluate",
        json!({
            "expression": "document.title",
            "awaitPromise": true,
            "target": {
                "context": context_id
            }
        }),
    )
    .await;
    assert_eq!(
        title["result"]["result"],
        json!({
            "type": "string",
            "value": "shared classic bidi"
        }),
        "attached BiDi script command should execute in the Classic-owned document: {title:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_attached_classic_session_title_waits_for_script_triggered_form_navigation()
{
    let fixture_app = Router::new()
        .route(
            "/form",
            get(|| async move {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><html><head><title>Form Source</title></head>\
                     <body><form action='/submitted'><input name='login' value='moli'></form></body></html>",
                )
            }),
        )
        .route(
            "/submitted",
            get(|| async move {
                sleep(Duration::from_millis(250)).await;
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><html><head><title>Submitted Target</title></head>\
                     <body><main>submitted</main></body></html>",
                )
            }),
        );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "bidi-attached-classic-form-navigation");
    let form_url = format!("http://{fixture_addr}/form");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let session_id = classic_new_session_on_server(cdp_addr).await;
    let mut socket = connect_classic_session_bidi_socket(cdp_addr, &session_id).await;

    let tree = send_bidi_command(&mut socket, 1, "browsingContext.getTree", json!({})).await;
    assert_eq!(tree["type"], json!("success"));

    let navigated = classic_request_on_server_with_body(
        cdp_addr,
        "POST",
        &format!("/session/{session_id}/url"),
        json!({ "url": form_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    let submitted = classic_request_on_server_with_body(
        cdp_addr,
        "POST",
        &format!("/session/{session_id}/execute/sync"),
        json!({
            "script": "document.querySelector('form').submit(); return 'submitted';",
            "args": []
        }),
    )
    .await;
    assert_eq!(submitted, json!({ "value": "submitted" }));

    let title = timeout(
        Duration::from_secs(5),
        classic_request_on_server_with_body(
            cdp_addr,
            "GET",
            &format!("/session/{session_id}/title"),
            json!({}),
        ),
    )
    .await
    .expect("attached Classic session title should not hang while form navigation completes");
    assert_eq!(title, json!({ "value": "Submitted Target" }));

    let _ = socket.close(None).await;
    let _ = classic_request_on_server_with_body(
        cdp_addr,
        "DELETE",
        &format!("/session/{session_id}"),
        json!({}),
    )
    .await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_empty_html_user_context_call_function_can_append_iframe() {
    async fn empty_html() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "",
        )
    }

    async fn child_html() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>iframe child</body></html>",
        )
    }

    let fixture_app = Router::new()
        .route("/empty.html", get(empty_html))
        .route("/", get(child_html));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi empty html iframe fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi empty html iframe fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let empty_url = format!("http://{fixture_addr}/empty.html");
    let child_url = format!("http://{fixture_addr}/");

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
            "url": empty_url,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"), "navigate: {navigate:?}");

    let shell = send_bidi_command_response(
        &mut socket,
        6,
        "script.evaluate",
        json!({
            "expression": r#"JSON.stringify({
                documentElement: document.documentElement && document.documentElement.localName,
                body: document.body && document.body.localName,
                lastElementChild: document.documentElement && document.documentElement.lastElementChild && document.documentElement.lastElementChild.localName,
                lastElementChildAppend: typeof (document.documentElement && document.documentElement.lastElementChild && document.documentElement.lastElementChild.append),
                childElementCount: document.documentElement ? document.documentElement.childElementCount : -1
            })"#,
            "target": {
                "context": context_id.clone()
            }
        }),
    )
    .await;
    assert_eq!(shell["type"], json!("success"), "shell probe: {shell:?}");
    assert_eq!(
        shell["result"]["result"]["value"],
        json!(
            r#"{"documentElement":"html","body":"body","lastElementChild":"body","lastElementChildAppend":"function","childElementCount":2}"#
        ),
        "empty HTML navigation should expose a normal html/head/body shell"
    );

    let create_iframe = send_bidi_command_response(
        &mut socket,
        7,
        "script.callFunction",
        json!({
            "functionDeclaration": r#"(url) => {
                const iframe = document.createElement("iframe");
                iframe.src = url;
                document.documentElement.lastElementChild.append(iframe);
                return new Promise(resolve => iframe.onload = () => resolve(iframe.contentWindow));
            }"#,
            "arguments": [{"type": "string", "value": child_url}],
            "target": {
                "context": context_id
            },
            "awaitPromise": true
        }),
    )
    .await;
    assert_eq!(
        create_iframe["type"],
        json!("success"),
        "WPT-style create_iframe helper should succeed: {create_iframe:?}"
    );
    assert_eq!(create_iframe["result"]["result"]["type"], json!("window"));

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_script_await_promise_and_exception_details_match_wpt_shape() {
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

    let navigate_url = "data:text/html,bidi-script-await-promise";
    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": navigate_url,
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

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "new Promise(resolve => setTimeout(() => resolve('EVAL_DELAYED'), 0))",
                    "awaitPromise": true,
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send awaited script.evaluate");
    let awaited_evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(
        awaited_evaluate["type"],
        json!("success"),
        "awaited script.evaluate should resolve delayed promise: {awaited_evaluate:?}"
    );
    assert_eq!(awaited_evaluate["id"], json!(4_u64));
    assert_eq!(awaited_evaluate["result"]["type"], json!("success"));
    assert_eq!(
        awaited_evaluate["result"]["result"],
        json!({
            "type": "string",
            "value": "EVAL_DELAYED"
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "Promise.resolve('EVAL_UNAWAITED')",
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
        .expect("send unawaited script.evaluate");
    let unawaited_evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(unawaited_evaluate["type"], json!("success"));
    assert_eq!(unawaited_evaluate["id"], json!(5_u64));
    assert_eq!(unawaited_evaluate["result"]["type"], json!("success"));
    assert_eq!(
        unawaited_evaluate["result"]["result"]["type"],
        json!("promise"),
        "unawaited script.evaluate should return a promise remote value: {unawaited_evaluate:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "Promise.reject('EVAL_REJECTED')",
                    "awaitPromise": true,
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send rejected script.evaluate");
    let rejected_evaluate = recv_ws_json(&mut socket).await;
    assert_bidi_script_exception_result(&rejected_evaluate, 6, "EVAL_REJECTED");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "async function() { await new Promise(resolve => setTimeout(resolve, 0)); return 'CALL_DELAYED'; }",
                    "awaitPromise": true,
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send awaited script.callFunction");
    let awaited_call = recv_ws_json(&mut socket).await;
    assert_eq!(
        awaited_call["type"],
        json!("success"),
        "awaited script.callFunction should resolve delayed promise: {awaited_call:?}"
    );
    assert_eq!(awaited_call["id"], json!(7_u64));
    assert_eq!(awaited_call["result"]["type"], json!("success"));
    assert_eq!(
        awaited_call["result"]["result"],
        json!({
            "type": "string",
            "value": "CALL_DELAYED"
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "async () => 'CALL_UNAWAITED'",
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
        .expect("send unawaited script.callFunction");
    let unawaited_call = recv_ws_json(&mut socket).await;
    assert_eq!(unawaited_call["type"], json!("success"));
    assert_eq!(unawaited_call["id"], json!(8_u64));
    assert_eq!(unawaited_call["result"]["type"], json!("success"));
    assert_eq!(
        unawaited_call["result"]["result"]["type"],
        json!("promise"),
        "unawaited script.callFunction should return a promise remote value: {unawaited_call:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "() => Promise.reject('CALL_REJECTED')",
                    "awaitPromise": true,
                    "target": {
                        "context": context_id
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send rejected script.callFunction");
    let rejected_call = recv_ws_json(&mut socket).await;
    assert_bidi_script_exception_result(&rejected_call, 9, "CALL_REJECTED");

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_script_result_ownership_applies_to_exception_remote_value() {
    // Mirrors webdriver/tests/bidi/script/{evaluate,call_function}/result_ownership.py.
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
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    let root_evaluate = send_bidi_command(
        &mut socket,
        3,
        "script.evaluate",
        json!({
            "expression": "throw {a: 1}",
            "awaitPromise": false,
            "resultOwnership": "root",
            "target": {
                "context": context_id.clone()
            }
        }),
    )
    .await;
    assert_bidi_script_exception_remote_handle(&root_evaluate, 3, true);

    let root_call = send_bidi_command(
        &mut socket,
        4,
        "script.callFunction",
        json!({
            "functionDeclaration": "() => { throw {a: 1}; }",
            "awaitPromise": false,
            "resultOwnership": "root",
            "target": {
                "context": context_id.clone()
            }
        }),
    )
    .await;
    assert_bidi_script_exception_remote_handle(&root_call, 4, true);

    let none_evaluate = send_bidi_command(
        &mut socket,
        5,
        "script.evaluate",
        json!({
            "expression": "Promise.reject({a: 1})",
            "awaitPromise": true,
            "resultOwnership": "none",
            "target": {
                "context": context_id
            }
        }),
    )
    .await;
    assert_bidi_script_exception_remote_handle(&none_evaluate, 5, false);

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_call_function_user_activation_controls_navigator_and_copy() {
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
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    for (id, user_activation, expected) in [(3_u64, false, false), (4, true, true)] {
        let response = send_bidi_command(
            &mut socket,
            id,
            "script.callFunction",
            json!({
                "functionDeclaration": "() => navigator.userActivation.isActive && navigator.userActivation.hasBeenActive",
                "awaitPromise": true,
                "userActivation": user_activation,
                "target": {
                    "context": context_id.clone()
                }
            }),
        )
        .await;
        assert_eq!(response["type"], json!("success"));
        assert_eq!(
            response["result"]["result"],
            json!({
                "type": "boolean",
                "value": expected
            }),
            "navigator.userActivation should follow BiDi userActivation={user_activation}: {response:?}"
        );
    }

    let restored = send_bidi_command(
        &mut socket,
        5,
        "script.callFunction",
        json!({
            "functionDeclaration": "() => navigator.userActivation.isActive && navigator.userActivation.hasBeenActive",
            "awaitPromise": true,
            "target": {
                "context": context_id.clone()
            }
        }),
    )
    .await;
    assert_eq!(
        restored["result"]["result"],
        json!({
            "type": "boolean",
            "value": true
        }),
        "BiDi activation notification should survive the activating call: {restored:?}"
    );

    let spoofed_global = send_bidi_command(
        &mut socket,
        6,
        "script.evaluate",
        json!({
            "expression": "globalThis.__moliWebDriverBidiUserActivation = false; navigator.userActivation.isActive && navigator.userActivation.hasBeenActive",
            "awaitPromise": true,
            "target": {
                "context": context_id.clone()
            }
        }),
    )
    .await;
    assert_eq!(
        spoofed_global["result"]["result"],
        json!({
            "type": "boolean",
            "value": true
        }),
        "page globals must not clear native BiDi activation: {spoofed_global:?}"
    );

    let copy_context = send_bidi_command(
        &mut socket,
        7,
        "browsingContext.create",
        json!({ "type": "tab" }),
    )
    .await;
    let copy_context_id = copy_context["result"]["context"]
        .as_str()
        .expect("fresh copy context");
    let spoofed_fresh = send_bidi_command(&mut socket, 8, "script.evaluate", json!({
        "expression": "globalThis.__moliWebDriverBidiUserActivation = true; navigator.userActivation.isActive || navigator.userActivation.hasBeenActive",
        "awaitPromise": true,
        "target": {"context": copy_context_id},
    })).await;
    assert_eq!(
        spoofed_fresh["result"]["result"],
        json!({"type":"boolean","value":false}),
        "page globals must not grant activation in a fresh document"
    );

    for (id, user_activation, expected) in
        [(9_u64, false, false), (10, true, true), (11, false, true)]
    {
        let response = send_bidi_command(
            &mut socket,
            id,
            "script.callFunction",
            json!({
                "functionDeclaration": "() => document.body.appendChild(document.createTextNode('test')) && document.execCommand('selectAll') && document.execCommand('copy')",
                "awaitPromise": true,
                "userActivation": user_activation,
                "target": {
                    "context": copy_context_id
                }
            }),
        )
        .await;
        assert_eq!(response["type"], json!("success"));
        assert_eq!(
            response["result"]["result"],
            json!({
                "type": "boolean",
                "value": expected
            }),
            "execCommand('copy') should use the document's activation state for userActivation={user_activation}: {response:?}"
        );
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_script_context_sandbox_uses_isolated_world_and_get_realms() {
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
                    "url": "data:text/html,<body>initial</body>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "window.foo = 1",
                    "awaitPromise": true,
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send default script.evaluate");
    let default_set = recv_ws_json(&mut socket).await;
    assert_eq!(default_set["type"], json!("success"));
    assert_eq!(default_set["result"]["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "window.foo",
                    "awaitPromise": true,
                    "target": {
                        "context": context_id.clone(),
                        "sandbox": "sandbox"
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send sandbox window.foo script.evaluate");
    let sandbox_probe = recv_ws_json(&mut socket).await;
    assert_eq!(sandbox_probe["type"], json!("success"));
    assert_eq!(
        sandbox_probe["result"]["result"],
        json!({"type": "undefined"})
    );
    let sandbox_realm_from_evaluate = sandbox_probe["result"]["realm"]
        .as_str()
        .expect("sandbox evaluate should report a realm")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "window.bar = 2",
                    "awaitPromise": true,
                    "target": {
                        "context": context_id.clone(),
                        "sandbox": "sandbox"
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send sandbox window.bar script.evaluate");
    let sandbox_set = recv_ws_json(&mut socket).await;
    assert_eq!(sandbox_set["type"], json!("success"));
    assert_eq!(
        sandbox_set["result"]["result"],
        json!({"type": "number", "value": 2})
    );
    assert_eq!(
        sandbox_set["result"]["realm"],
        json!(sandbox_realm_from_evaluate)
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "() => window.bar",
                    "awaitPromise": true,
                    "target": {
                        "context": context_id.clone(),
                        "sandbox": "sandbox"
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send sandbox script.callFunction");
    let sandbox_call = recv_ws_json(&mut socket).await;
    assert_eq!(sandbox_call["type"], json!("success"));
    assert_eq!(
        sandbox_call["result"]["result"],
        json!({"type": "number", "value": 2})
    );
    assert_eq!(
        sandbox_call["result"]["realm"],
        json!(sandbox_realm_from_evaluate)
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "window.bar",
                    "awaitPromise": true,
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send default window.bar script.evaluate");
    let default_probe = recv_ws_json(&mut socket).await;
    assert_eq!(default_probe["type"], json!("success"));
    assert_eq!(
        default_probe["result"]["result"],
        json!({"type": "undefined"})
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "document.body.textContent = 'from sandbox'",
                    "awaitPromise": true,
                    "target": {
                        "context": context_id.clone(),
                        "sandbox": "sandbox"
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send sandbox DOM side-effect script.evaluate");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 10_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "document.body.textContent",
                    "awaitPromise": true,
                    "target": {
                        "context": context_id.clone(),
                        "sandbox": "another_sandbox"
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send second sandbox DOM side-effect script.evaluate");
    let second_sandbox_dom = recv_ws_json(&mut socket).await;
    assert_eq!(second_sandbox_dom["type"], json!("success"));
    assert_eq!(
        second_sandbox_dom["result"]["result"],
        json!({"type": "string", "value": "from sandbox"})
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 11_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "() => document.querySelector('body')",
                    "awaitPromise": true,
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send default body node script.callFunction");
    let default_body_node = recv_ws_json(&mut socket).await;
    assert_eq!(default_body_node["type"], json!("success"));
    let default_body = default_body_node["result"]["result"].clone();
    assert_eq!(default_body["type"], json!("node"));
    assert!(
        default_body["sharedId"]
            .as_str()
            .is_some_and(|shared_id| !shared_id.is_empty()),
        "default body node should include sharedId: {default_body_node:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 12_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "() => document.querySelector('body')",
                    "awaitPromise": true,
                    "target": {
                        "context": context_id.clone(),
                        "sandbox": "sandbox"
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send sandbox body node script.callFunction");
    let sandbox_body_node = recv_ws_json(&mut socket).await;
    assert_eq!(sandbox_body_node["type"], json!("success"));
    assert_eq!(
        sandbox_body_node["result"]["result"], default_body,
        "sandbox should return the same BiDi node sharedId as default realm: {sandbox_body_node:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 13_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "(node) => node.localName",
                    "arguments": [default_body],
                    "awaitPromise": false,
                    "target": {
                        "context": context_id.clone(),
                        "sandbox": "sandbox"
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send sandbox node argument script.callFunction");
    let sandbox_node_argument = recv_ws_json(&mut socket).await;
    assert_eq!(sandbox_node_argument["type"], json!("success"));
    assert_eq!(
        sandbox_node_argument["result"]["result"],
        json!({"type": "string", "value": "body"}),
        "sandbox should accept a default-realm node sharedId argument: {sandbox_node_argument:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 14_u64,
                "method": "script.getRealms",
                "params": {
                    "context": context_id.clone(),
                    "type": "window"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.getRealms");
    let realms = recv_ws_json(&mut socket).await;
    assert_eq!(realms["type"], json!("success"));
    let default_realm = bidi_window_realm(&realms, &context_id);
    let sandbox_realm = bidi_sandbox_window_realm(&realms, &context_id, "sandbox");
    assert_ne!(
        default_realm["realm"], sandbox_realm["realm"],
        "sandbox should expose a distinct non-default realm: {realms:?}"
    );
    assert_eq!(sandbox_realm["realm"], json!(sandbox_realm_from_evaluate));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 15_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "window.bar",
                    "awaitPromise": true,
                    "target": {
                        "realm": sandbox_realm_from_evaluate
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send sandbox realm-target script.evaluate");
    let realm_target = recv_ws_json(&mut socket).await;
    assert_eq!(realm_target["type"], json!("success"));
    assert_eq!(
        realm_target["result"]["result"],
        json!({"type": "number", "value": 2})
    );
    assert_eq!(
        realm_target["result"]["realm"], sandbox_realm["realm"],
        "realm-target evaluation should round-trip through the sandbox realm"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_script_disown_respects_sandbox_handle_owner() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    macro_rules! bidi_command {
        ($id:expr, $method:literal, $params:expr) => {{
            socket
                .send(WsMessage::Text(
                    json!({
                        "id": $id,
                        "method": $method,
                        "params": $params
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect(concat!("send ", $method));
            recv_ws_json(&mut socket).await
        }};
    }

    let session = bidi_command!(1_u64, "session.new", json!({}));
    assert_eq!(session["type"], json!("success"));

    let create = bidi_command!(
        2_u64,
        "browsingContext.create",
        json!({
            "type": "tab"
        })
    );
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    let navigate = bidi_command!(
        3_u64,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": "data:text/html,<body>bidi-disown-sandbox</body>",
            "wait": "complete"
        })
    );
    assert_eq!(navigate["type"], json!("success"));

    let default_evaluate = bidi_command!(
        4_u64,
        "script.evaluate",
        json!({
            "expression": "({a: 'without sandbox'})",
            "awaitPromise": false,
            "resultOwnership": "root",
            "target": {
                "context": context_id.clone()
            }
        })
    );
    assert_eq!(default_evaluate["type"], json!("success"));
    let default_handle = default_evaluate["result"]["result"]["handle"]
        .as_str()
        .expect("default realm value should return a handle")
        .to_owned();

    let sandbox_evaluate = bidi_command!(
        5_u64,
        "script.evaluate",
        json!({
            "expression": "({a: 'with sandbox'})",
            "awaitPromise": false,
            "resultOwnership": "root",
            "target": {
                "context": context_id.clone(),
                "sandbox": "basic_sandbox"
            }
        })
    );
    assert_eq!(sandbox_evaluate["type"], json!("success"));
    let sandbox_handle = sandbox_evaluate["result"]["result"]["handle"]
        .as_str()
        .expect("sandbox realm value should return a handle")
        .to_owned();

    let wrong_sandbox_disown = bidi_command!(
        6_u64,
        "script.disown",
        json!({
            "handles": [default_handle.clone()],
            "target": {
                "context": context_id.clone(),
                "sandbox": "basic_sandbox"
            }
        })
    );
    assert_eq!(wrong_sandbox_disown["type"], json!("success"));
    assert_eq!(wrong_sandbox_disown["result"], json!({}));

    let default_after_wrong_disown = bidi_command!(
        7_u64,
        "script.callFunction",
        json!({
            "functionDeclaration": "(arg) => arg.a",
            "arguments": [
                {
                    "handle": default_handle.clone()
                }
            ],
            "target": {
                "context": context_id.clone()
            }
        })
    );
    assert_eq!(default_after_wrong_disown["type"], json!("success"));
    assert_eq!(
        default_after_wrong_disown["result"]["result"],
        json!({"type": "string", "value": "without sandbox"})
    );

    let default_disown_sandbox_handle = bidi_command!(
        8_u64,
        "script.disown",
        json!({
            "handles": [sandbox_handle.clone()],
            "target": {
                "context": context_id.clone()
            }
        })
    );
    assert_eq!(default_disown_sandbox_handle["type"], json!("success"));
    assert_eq!(default_disown_sandbox_handle["result"], json!({}));

    let other_sandbox_disown = bidi_command!(
        9_u64,
        "script.disown",
        json!({
            "handles": [sandbox_handle.clone()],
            "target": {
                "context": context_id.clone(),
                "sandbox": "another_sandbox"
            }
        })
    );
    assert_eq!(other_sandbox_disown["type"], json!("success"));
    assert_eq!(other_sandbox_disown["result"], json!({}));

    let sandbox_after_wrong_disown = bidi_command!(
        10_u64,
        "script.callFunction",
        json!({
            "functionDeclaration": "(arg) => arg.a",
            "arguments": [
                {
                    "handle": sandbox_handle.clone()
                }
            ],
            "target": {
                "context": context_id.clone(),
                "sandbox": "basic_sandbox"
            }
        })
    );
    assert_eq!(sandbox_after_wrong_disown["type"], json!("success"));
    assert_eq!(
        sandbox_after_wrong_disown["result"]["result"],
        json!({"type": "string", "value": "with sandbox"})
    );

    let correct_sandbox_disown = bidi_command!(
        11_u64,
        "script.disown",
        json!({
            "handles": [sandbox_handle.clone()],
            "target": {
                "context": context_id.clone(),
                "sandbox": "basic_sandbox"
            }
        })
    );
    assert_eq!(correct_sandbox_disown["type"], json!("success"));
    assert_eq!(correct_sandbox_disown["result"], json!({}));

    let sandbox_after_correct_disown = bidi_command!(
        12_u64,
        "script.callFunction",
        json!({
            "functionDeclaration": "(arg) => arg.a",
            "arguments": [
                {
                    "handle": sandbox_handle
                }
            ],
            "target": {
                "context": context_id.clone(),
                "sandbox": "basic_sandbox"
            }
        })
    );
    assert_eq!(sandbox_after_correct_disown["type"], json!("error"));
    assert_eq!(
        sandbox_after_correct_disown["error"],
        json!("no such handle")
    );

    let default_disown = bidi_command!(
        13_u64,
        "script.disown",
        json!({
            "handles": [default_handle.clone()],
            "target": {
                "context": context_id.clone()
            }
        })
    );
    assert_eq!(default_disown["type"], json!("success"));
    assert_eq!(default_disown["result"], json!({}));

    let default_after_correct_disown = bidi_command!(
        14_u64,
        "script.callFunction",
        json!({
            "functionDeclaration": "(arg) => arg.a",
            "arguments": [
                {
                    "handle": default_handle
                }
            ],
            "target": {
                "context": context_id,
                "sandbox": "basic_sandbox"
            }
        })
    );
    assert_eq!(default_after_correct_disown["type"], json!("error"));
    assert_eq!(
        default_after_correct_disown["error"],
        json!("no such handle")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_call_function_unknown_target_without_context_matches_wpt_error_shape() {
    // Reduced from Chromium/WPT
    // webdriver/tests/bidi/script/call_function/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    for (id, target) in [
        (2_u64, json!({"context": "_UNKNOWN_"})),
        (3_u64, json!({"realm": "_UNKNOWN_"})),
    ] {
        let response = send_bidi_command(
            &mut socket,
            id,
            "script.callFunction",
            json!({
                "functionDeclaration": "(arg) => arg",
                "awaitPromise": false,
                "target": target,
            }),
        )
        .await;
        assert_bidi_error(
            &response,
            "no such frame",
            "script.callFunction should reject unknown context or realm",
        );
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_script_node_shared_id_round_trips_as_argument() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    macro_rules! bidi_command {
        ($id:expr, $method:literal, $params:expr) => {{
            socket
                .send(WsMessage::Text(
                    json!({
                        "id": $id,
                        "method": $method,
                        "params": $params
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .expect(concat!("send ", $method));
            recv_ws_json(&mut socket).await
        }};
    }

    let session = bidi_command!(1_u64, "session.new", json!({}));
    assert_eq!(session["type"], json!("success"));

    let create = bidi_command!(
        2_u64,
        "browsingContext.create",
        json!({
            "type": "tab"
        })
    );
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    let navigate = bidi_command!(
        3_u64,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": "data:text/html,<main><div id='parent'>Hello<span id='target' data-x='1'></span></div></main>",
            "wait": "complete"
        })
    );
    assert_eq!(navigate["type"], json!("success"));

    let evaluate = bidi_command!(
        4_u64,
        "script.evaluate",
        json!({
            "expression": "document.querySelector('#target')",
            "awaitPromise": false,
            "target": {
                "context": context_id.clone()
            },
            "serializationOptions": {
                "maxDomDepth": 1
            }
        })
    );
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["result"]["type"], json!("success"));
    let node = evaluate["result"]["result"].clone();
    assert_eq!(node["type"], json!("node"), "node result: {evaluate:?}");
    let shared_id = node["sharedId"]
        .as_str()
        .expect("node remote value should include sharedId")
        .to_owned();
    assert!(
        node.get("handle").is_none(),
        "non-root node should not carry handle: {node:?}"
    );
    assert_eq!(node["value"]["nodeType"], json!(1));
    assert_eq!(node["value"]["localName"], json!("span"));
    assert_eq!(
        node["value"]["namespaceURI"],
        json!("http://www.w3.org/1999/xhtml")
    );
    assert_eq!(
        node["value"]["attributes"],
        json!({
            "id": "target",
            "data-x": "1"
        })
    );
    assert_eq!(node["value"]["childNodeCount"], json!(0));
    assert_eq!(node["value"]["children"], json!([]));

    let call = bidi_command!(
        5_u64,
        "script.callFunction",
        json!({
            "functionDeclaration": "(node) => `${node.localName}:${node.getAttribute('data-x')}`",
            "arguments": [
                {
                    "sharedId": shared_id
                }
            ],
            "awaitPromise": false,
            "target": {
                "context": context_id.clone()
            }
        })
    );
    assert_eq!(call["type"], json!("success"));
    assert_eq!(
        call["result"]["result"],
        json!({"type": "string", "value": "span:1"})
    );

    let evaluate_parent = bidi_command!(
        6_u64,
        "script.evaluate",
        json!({
            "expression": "document.querySelector('#parent')",
            "awaitPromise": false,
            "target": {
                "context": context_id.clone()
            },
            "serializationOptions": {
                "maxDomDepth": 1
            }
        })
    );
    assert_eq!(evaluate_parent["type"], json!("success"));
    let parent = evaluate_parent["result"]["result"].clone();
    assert_eq!(
        parent["type"],
        json!("node"),
        "parent result: {evaluate_parent:?}"
    );
    assert_eq!(parent["value"]["nodeType"], json!(1));
    assert_eq!(parent["value"]["localName"], json!("div"));
    assert_eq!(parent["value"]["childNodeCount"], json!(2));
    let children = parent["value"]["children"]
        .as_array()
        .expect("parent children should be serialized");
    assert_eq!(children.len(), 2, "parent children: {children:?}");
    assert_eq!(children[0]["type"], json!("node"));
    assert_eq!(children[0]["value"]["nodeType"], json!(3));
    assert_eq!(children[0]["value"]["nodeValue"], json!("Hello"));
    assert!(
        children[0]["value"].get("children").is_none(),
        "depth-limited text child should not serialize children: {:?}",
        children[0]
    );
    assert_eq!(children[1]["type"], json!("node"));
    assert_eq!(children[1]["value"]["nodeType"], json!(1));
    assert_eq!(children[1]["value"]["localName"], json!("span"));
    assert_eq!(children[1]["value"]["childNodeCount"], json!(0));
    assert!(
        children[1]["value"].get("children").is_none(),
        "depth-limited element child should not serialize children: {:?}",
        children[1]
    );
    let text_shared_id = children[0]["sharedId"]
        .as_str()
        .expect("text child should include sharedId")
        .to_owned();
    let child_span_shared_id = children[1]["sharedId"]
        .as_str()
        .expect("span child should include sharedId")
        .to_owned();

    let call_children = bidi_command!(
        7_u64,
        "script.callFunction",
        json!({
            "functionDeclaration": "(text, span) => `${text.nodeValue}:${span.getAttribute('data-x')}`",
            "arguments": [
                {
                    "sharedId": text_shared_id
                },
                {
                    "sharedId": child_span_shared_id
                }
            ],
            "awaitPromise": false,
            "target": {
                "context": context_id
            }
        })
    );
    assert_eq!(call_children["type"], json!("success"));
    assert_eq!(
        call_children["result"]["result"],
        json!({"type": "string", "value": "Hello:1"})
    );

    let evaluate_attribute = bidi_command!(
        8_u64,
        "script.evaluate",
        json!({
            "expression": "document.querySelector('#target').attributes[1]",
            "awaitPromise": false,
            "target": {
                "context": context_id.clone()
            }
        })
    );
    assert_eq!(evaluate_attribute["type"], json!("success"));
    let attribute = evaluate_attribute["result"]["result"].clone();
    assert_eq!(
        attribute["type"],
        json!("node"),
        "attribute result: {evaluate_attribute:?}"
    );
    let attribute_shared_id = attribute["sharedId"]
        .as_str()
        .expect("attribute node remote value should include sharedId")
        .to_owned();
    assert_eq!(
        attribute["value"],
        json!({
            "childNodeCount": 0,
            "localName": "data-x",
            "namespaceURI": null,
            "nodeType": 2,
            "nodeValue": "1"
        }),
        "attribute node should serialize with WPT node value shape: {evaluate_attribute:?}"
    );

    let call_attribute = bidi_command!(
        9_u64,
        "script.callFunction",
        json!({
            "functionDeclaration": "(attr) => `${attr.nodeType}:${attr.localName}:${attr.nodeValue}`",
            "arguments": [
                {
                    "sharedId": attribute_shared_id
                }
            ],
            "awaitPromise": false,
            "target": {
                "context": context_id.clone()
            }
        })
    );
    assert_eq!(call_attribute["type"], json!("success"));
    assert_eq!(
        call_attribute["result"]["result"],
        json!({"type": "string", "value": "2:data-x:1"})
    );

    let evaluate_namespaced_attribute = bidi_command!(
        10_u64,
        "script.evaluate",
        json!({
            "expression": "(() => { const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); svg.setAttributeNS('http://www.w3.org/2000/svg', 'svg:foo', 'bar'); document.body.appendChild(svg); return svg.attributes[0]; })()",
            "awaitPromise": false,
            "target": {
                "context": context_id
            }
        })
    );
    assert_eq!(evaluate_namespaced_attribute["type"], json!("success"));
    assert_eq!(
        evaluate_namespaced_attribute["result"]["result"]["type"],
        json!("node"),
        "namespaced attribute result: {evaluate_namespaced_attribute:?}"
    );
    assert!(
        evaluate_namespaced_attribute["result"]["result"]["sharedId"]
            .as_str()
            .is_some_and(|shared_id| !shared_id.is_empty()),
        "namespaced attribute node should include sharedId: {evaluate_namespaced_attribute:?}"
    );
    assert_eq!(
        evaluate_namespaced_attribute["result"]["result"]["value"],
        json!({
            "childNodeCount": 0,
            "localName": "foo",
            "namespaceURI": "http://www.w3.org/2000/svg",
            "nodeType": 2,
            "nodeValue": "bar"
        }),
        "namespaced attribute node should serialize with WPT node value shape: {evaluate_namespaced_attribute:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_script_serialization_options_deep_serialize_object_properties() {
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
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": "data:text/html,bidi-script-deep-serialization",
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

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "({'foo': {'bar': 'baz'}, 'qux': 'quux', 1: 'fred', '2': 'thud'})",
                    "awaitPromise": false,
                    "target": {
                        "context": context_id.clone()
                    },
                    "serializationOptions": {
                        "maxObjectDepth": 1
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send deep-serialized script.evaluate");
    let evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["id"], json!(4_u64));
    assert_eq!(evaluate["result"]["type"], json!("success"));
    assert_eq!(
        evaluate["result"]["result"],
        json!({
            "type": "object",
            "value": [
                ["1", {"type": "string", "value": "fred"}],
                ["2", {"type": "string", "value": "thud"}],
                ["foo", {"type": "object"}],
                ["qux", {"type": "string", "value": "quux"}]
            ]
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "[1, 'foo', true, new RegExp(/foo/g), [1]]",
                    "awaitPromise": false,
                    "target": {
                        "context": context_id.clone()
                    },
                    "serializationOptions": {
                        "maxObjectDepth": 1
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send deep-serialized array script.evaluate");
    let evaluate_array = recv_ws_json(&mut socket).await;
    assert_eq!(evaluate_array["type"], json!("success"));
    assert_eq!(
        evaluate_array["result"]["result"],
        json!({
            "type": "array",
            "value": [
                {"type": "number", "value": 1},
                {"type": "string", "value": "foo"},
                {"type": "boolean", "value": true},
                {
                    "type": "regexp",
                    "value": {
                        "pattern": "foo",
                        "flags": "g"
                    }
                },
                {"type": "array"}
            ]
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "() => ({'outer': {'inner': 1}, 'leaf': 'ok'})",
                    "awaitPromise": false,
                    "target": {
                        "context": context_id.clone()
                    },
                    "serializationOptions": {
                        "maxObjectDepth": 1
                    },
                    "resultOwnership": "root"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send root-owned deep-serialized script.callFunction");
    let call_function = recv_ws_json(&mut socket).await;
    assert_eq!(call_function["type"], json!("success"));
    assert_eq!(call_function["id"], json!(6_u64));
    assert_eq!(call_function["result"]["type"], json!("success"));
    let root_result = &call_function["result"]["result"];
    assert_eq!(root_result["type"], json!("object"));
    assert!(
        root_result["handle"]
            .as_str()
            .is_some_and(|handle| !handle.is_empty()),
        "root-owned deep serialization should retain the root handle: {call_function:?}"
    );
    assert_eq!(
        root_result["value"],
        json!([
            ["outer", {"type": "object"}],
            ["leaf", {"type": "string", "value": "ok"}]
        ])
    );
    let handle = root_result["handle"]
        .as_str()
        .expect("root-owned deep serialized object handle")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "(arg) => arg.leaf",
                    "arguments": [
                        {
                            "handle": handle
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
        .expect("send handle script.callFunction after deep serialization");
    let handle_call = recv_ws_json(&mut socket).await;
    assert_eq!(handle_call["type"], json!("success"));
    assert_eq!(
        handle_call["result"]["result"],
        json!({
            "type": "string",
            "value": "ok"
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "{const data = { baz: 'qux' }; [data, data]}",
                    "awaitPromise": false,
                    "target": {
                        "context": context_id.clone()
                    },
                    "serializationOptions": {
                        "maxObjectDepth": 2
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send duplicate-object script.evaluate");
    let duplicate_array = recv_ws_json(&mut socket).await;
    assert_eq!(duplicate_array["type"], json!("success"));
    assert_eq!(duplicate_array["id"], json!(8_u64));
    let duplicate_array_values = duplicate_array["result"]["result"]["value"]
        .as_array()
        .expect("duplicate object array should serialize array values");
    assert_eq!(duplicate_array_values.len(), 2);
    let first_duplicate_id = duplicate_array_values[0]["internalId"]
        .as_str()
        .expect("first duplicate object should include internalId");
    let second_duplicate_id = duplicate_array_values[1]["internalId"]
        .as_str()
        .expect("second duplicate object should include internalId");
    assert_eq!(
        first_duplicate_id, second_duplicate_id,
        "same JS object should keep the same BiDi internalId: {duplicate_array:?}"
    );
    assert_eq!(duplicate_array_values[0]["type"], json!("object"));
    assert_eq!(duplicate_array_values[1]["type"], json!("object"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "{const obj1 = {a: 1}; const obj2 = [2]; ({key1: obj1, key2: obj2, nested: {key3: obj1, key4: obj2}})}",
                    "awaitPromise": false,
                    "target": {
                        "context": context_id.clone()
                    },
                    "serializationOptions": {
                        "maxObjectDepth": 3
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send nested duplicate-object script.evaluate");
    let nested_duplicates = recv_ws_json(&mut socket).await;
    assert_eq!(nested_duplicates["type"], json!("success"));
    assert_eq!(nested_duplicates["id"], json!(9_u64));
    let nested_result = &nested_duplicates["result"]["result"];
    let key1 = bidi_remote_object_property(nested_result, "key1");
    let key2 = bidi_remote_object_property(nested_result, "key2");
    let nested = bidi_remote_object_property(nested_result, "nested");
    let key3 = bidi_remote_object_property(nested, "key3");
    let key4 = bidi_remote_object_property(nested, "key4");
    let key1_internal_id = key1["internalId"]
        .as_str()
        .expect("key1 object should include internalId");
    let key2_internal_id = key2["internalId"]
        .as_str()
        .expect("key2 array should include internalId");
    let key3_internal_id = key3["internalId"]
        .as_str()
        .expect("key3 duplicate object should include internalId");
    let key4_internal_id = key4["internalId"]
        .as_str()
        .expect("key4 duplicate array should include internalId");
    assert_ne!(
        key1_internal_id, key2_internal_id,
        "different objects should not share BiDi internalId: {nested_duplicates:?}"
    );
    assert_eq!(key1_internal_id, key3_internal_id);
    assert_eq!(key2_internal_id, key4_internal_id);

    socket
        .send(WsMessage::Text(
            json!({
                "id": 100_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "{const obj1 = document; const obj2 = {}; ({key1: obj1, key2: obj2, nested: {key3: obj1, key4: obj2}})}",
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
        .expect("send nested duplicate node script.evaluate");
    let nested_node_duplicates = recv_ws_json(&mut socket).await;
    assert_eq!(nested_node_duplicates["type"], json!("success"));
    assert_eq!(nested_node_duplicates["id"], json!(100_u64));
    let nested_node_result = &nested_node_duplicates["result"]["result"];
    let key1_node = bidi_remote_object_property(nested_node_result, "key1");
    let key2_object = bidi_remote_object_property(nested_node_result, "key2");
    let nested_node = bidi_remote_object_property(nested_node_result, "nested");
    let key3_node = bidi_remote_object_property(nested_node, "key3");
    let key4_object = bidi_remote_object_property(nested_node, "key4");
    assert_eq!(key1_node["type"], json!("node"));
    let key1_node_internal_id = key1_node["internalId"]
        .as_str()
        .expect("top-level duplicate node should include internalId");
    let key2_object_internal_id = key2_object["internalId"]
        .as_str()
        .expect("top-level duplicate object should include internalId");
    let key3_node_internal_id = key3_node["internalId"]
        .as_str()
        .expect("nested duplicate node should include internalId");
    let key4_object_internal_id = key4_object["internalId"]
        .as_str()
        .expect("nested duplicate object should include internalId");
    assert_ne!(
        key1_node_internal_id, key2_object_internal_id,
        "node and plain object should not share BiDi internalId: {nested_node_duplicates:?}"
    );
    assert_eq!(key1_node_internal_id, key3_node_internal_id);
    assert_eq!(key2_object_internal_id, key4_object_internal_id);

    let remote_value_cases = [
        (
            10_u64,
            "new RegExp(/foo/g)",
            json!({
                "type": "regexp",
                "value": {
                    "pattern": "foo",
                    "flags": "g"
                }
            }),
        ),
        (
            11_u64,
            "new Date(1654004849000)",
            json!({
                "type": "date",
                "value": "2022-05-31T13:47:29.000Z"
            }),
        ),
        (
            12_u64,
            "new Map([[1, 2], ['foo', 'bar'], [true, false], ['baz', [1]]])",
            json!({
                "type": "map",
                "value": [
                    [
                        {"type": "number", "value": 1},
                        {"type": "number", "value": 2}
                    ],
                    [
                        "foo",
                        {"type": "string", "value": "bar"}
                    ],
                    [
                        {"type": "boolean", "value": true},
                        {"type": "boolean", "value": false}
                    ],
                    [
                        "baz",
                        {"type": "array"}
                    ]
                ]
            }),
        ),
        (
            13_u64,
            "new Set([1, 'foo', true, [1], new Map([[1,2]])])",
            json!({
                "type": "set",
                "value": [
                    {"type": "number", "value": 1},
                    {"type": "string", "value": "foo"},
                    {"type": "boolean", "value": true},
                    {"type": "array"},
                    {"type": "map"}
                ]
            }),
        ),
        (14_u64, "new WeakMap()", json!({"type": "weakmap"})),
        (15_u64, "new WeakSet()", json!({"type": "weakset"})),
        (
            16_u64,
            "new Error('SOME_ERROR_TEXT')",
            json!({"type": "error"}),
        ),
        (
            17_u64,
            "window",
            json!({
                "type": "window",
                "value": {
                    "context": context_id.clone()
                }
            }),
        ),
    ];
    for (id, expression, expected) in remote_value_cases {
        socket
            .send(WsMessage::Text(
                json!({
                    "id": id,
                    "method": "script.evaluate",
                    "params": {
                        "expression": expression,
                        "awaitPromise": false,
                        "target": {
                            "context": context_id.clone()
                        },
                        "serializationOptions": {
                            "maxObjectDepth": 1
                        }
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send remote value script.evaluate");
        let response = recv_ws_json(&mut socket).await;
        assert_eq!(response["type"], json!("success"));
        assert_eq!(response["id"], json!(id));
        assert_eq!(
            response["result"]["result"], expected,
            "BiDi remote value should match WPT shape for {expression}: {response:?}"
        );
    }

    let local_value_cases = [
        (
            18_u64,
            json!({
                "type": "array",
                "value": [
                    {"type": "string", "value": "foobar"}
                ]
            }),
        ),
        (
            19_u64,
            json!({
                "type": "date",
                "value": "2022-05-31T13:47:29.000Z"
            }),
        ),
        (
            20_u64,
            json!({
                "type": "map",
                "value": [
                    ["foobar", {"type": "string", "value": "foobar"}]
                ]
            }),
        ),
        (
            21_u64,
            json!({
                "type": "object",
                "value": [
                    ["foobar", {"type": "string", "value": "foobar"}]
                ]
            }),
        ),
        (
            22_u64,
            json!({
                "type": "regexp",
                "value": {
                    "pattern": "foo",
                    "flags": "g"
                }
            }),
        ),
        (
            23_u64,
            json!({
                "type": "set",
                "value": [
                    {"type": "string", "value": "foobar"}
                ]
            }),
        ),
    ];
    for (id, argument) in local_value_cases {
        socket
            .send(WsMessage::Text(
                json!({
                    "id": id,
                    "method": "script.callFunction",
                    "params": {
                        "functionDeclaration": "(arg) => arg",
                        "arguments": [argument.clone()],
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
            .expect("send local value round-trip script.callFunction");
        let response = recv_ws_json(&mut socket).await;
        assert_eq!(response["type"], json!("success"));
        assert_eq!(response["id"], json!(id));
        assert_eq!(
            response["result"]["result"], argument,
            "BiDi LocalValue should round-trip with default deep serialization: {response:?}"
        );
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 24_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "(() => { const elem = document.createElement('img'); document.body.appendChild(elem); return {elem}; })()",
                    "awaitPromise": false,
                    "target": {
                        "context": context_id.clone()
                    },
                    "serializationOptions": {
                        "maxObjectDepth": 1,
                        "maxDomDepth": 0
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send embedded node script.evaluate");
    let embedded_node_response = recv_ws_json(&mut socket).await;
    assert_eq!(embedded_node_response["type"], json!("success"));
    assert_eq!(embedded_node_response["id"], json!(24_u64));
    let embedded_node =
        bidi_remote_object_property(&embedded_node_response["result"]["result"], "elem");
    assert_eq!(
        embedded_node["type"],
        json!("node"),
        "embedded DOM nodes should use BiDi node remote values: {embedded_node_response:?}"
    );
    let embedded_node_shared_id = embedded_node["sharedId"]
        .as_str()
        .expect("embedded DOM node should include sharedId")
        .to_owned();
    assert_eq!(
        embedded_node["value"],
        json!({
            "attributes": {},
            "childNodeCount": 0,
            "localName": "img",
            "namespaceURI": "http://www.w3.org/1999/xhtml",
            "nodeType": 1,
            "shadowRoot": null
        }),
        "embedded DOM nodes should serialize with WPT node value shape: {embedded_node_response:?}"
    );

    let embedded_node_container_cases = [
        (
            19_u64,
            "array",
            "(() => { const elem = document.createElement('img'); document.body.appendChild(elem); return [elem]; })()",
            "/result/result/value/0",
        ),
        (
            20_u64,
            "map-key",
            "(() => { const elem = document.createElement('img'); document.body.appendChild(elem); return new Map([[elem, 'elem']]); })()",
            "/result/result/value/0/0",
        ),
        (
            21_u64,
            "map-value",
            "(() => { const elem = document.createElement('img'); document.body.appendChild(elem); return new Map([['elem', elem]]); })()",
            "/result/result/value/0/1",
        ),
        (
            22_u64,
            "set",
            "(() => { const elem = document.createElement('img'); document.body.appendChild(elem); return new Set([elem]); })()",
            "/result/result/value/0",
        ),
    ];
    for (id, label, expression, node_pointer) in embedded_node_container_cases {
        socket
            .send(WsMessage::Text(
                json!({
                    "id": id,
                    "method": "script.evaluate",
                    "params": {
                        "expression": expression,
                        "awaitPromise": false,
                        "target": {
                            "context": context_id.clone()
                        },
                        "serializationOptions": {
                            "maxObjectDepth": 1,
                            "maxDomDepth": 0
                        }
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send embedded container node script.evaluate");
        let response = recv_ws_json(&mut socket).await;
        assert_eq!(response["type"], json!("success"));
        assert_eq!(response["id"], json!(id));
        let node = response
            .pointer(node_pointer)
            .unwrap_or_else(|| panic!("{label} should include embedded node: {response:?}"));
        assert_eq!(
            node["type"],
            json!("node"),
            "{label} embedded DOM node should use BiDi node remote value: {response:?}"
        );
        assert!(
            node["sharedId"]
                .as_str()
                .is_some_and(|shared_id| !shared_id.is_empty()),
            "{label} embedded DOM node should include sharedId: {response:?}"
        );
        assert_eq!(
            node["value"],
            json!({
                "attributes": {},
                "childNodeCount": 0,
                "localName": "img",
                "namespaceURI": "http://www.w3.org/1999/xhtml",
                "nodeType": 1,
                "shadowRoot": null
            }),
            "{label} embedded DOM node should serialize with WPT node value shape: {response:?}"
        );
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 23_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "(node) => node.localName",
                    "arguments": [
                        {
                            "sharedId": embedded_node_shared_id
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
        .expect("send embedded node sharedId script.callFunction");
    let embedded_node_call = recv_ws_json(&mut socket).await;
    assert_eq!(
        embedded_node_call["type"],
        json!("success"),
        "embedded node sharedId should be accepted as a callFunction argument: {embedded_node_call:?}"
    );
    assert_eq!(embedded_node_call["id"], json!(23_u64));
    assert_eq!(
        embedded_node_call["result"]["result"],
        json!({
            "type": "string",
            "value": "img"
        }),
        "embedded node sharedId should round-trip as a callFunction argument: {embedded_node_call:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 24_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "() => window",
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
        .expect("send window script.callFunction");
    let call_window = recv_ws_json(&mut socket).await;
    assert_eq!(call_window["type"], json!("success"));
    assert_eq!(call_window["id"], json!(24_u64));
    assert_eq!(
        call_window["result"]["result"],
        json!({
            "type": "window",
            "value": {
                "context": context_id
            }
        }),
        "BiDi callFunction should return WPT window remote value: {call_window:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_evaluate_iframe_window_await_promise_reports_child_context() {
    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Child Window</title><main>child-window</main>",
        )
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi evaluate iframe window fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi evaluate iframe window fixture addr");
    let child_url = format!("http://{fixture_addr}/child");
    let parent_child_url = child_url.clone();
    let fixture_app = Router::new()
        .route(
            "/",
            get(move || {
                let child_url = parent_child_url.clone();
                async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        format!(
                            "<!doctype html><html><body><main>parent</main><iframe src=\"{child_url}\"></iframe></body></html>"
                        ),
                    )
                }
            }),
        )
        .route("/child", get(child));
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
        .expect("send browsingContext.navigate");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "browsingContext.getTree",
                "params": {
                    "root": context_id.clone()
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.getTree");
    let tree = recv_ws_json(&mut socket).await;
    assert_eq!(tree["type"], json!("success"));
    let child_context_id = tree["result"]["contexts"][0]["children"][0]["context"]
        .as_str()
        .unwrap_or_else(|| panic!("getTree should expose child context: {tree:?}"))
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "window",
                    "awaitPromise": true,
                    "target": {
                        "context": child_context_id.clone()
                    },
                    "serializationOptions": {
                        "maxObjectDepth": 1
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send iframe-context window script.evaluate");
    let evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(
        evaluate["result"]["result"],
        json!({
            "type": "window",
            "value": {
                "context": child_context_id
            }
        }),
        "awaitPromise iframe window evaluate should return child window context: {evaluate:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}
#[tokio::test]
async fn websocket_bidi_script_serializes_dom_collections_with_node_entries() {
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
        json!({
            "type": "tab"
        }),
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
            "context": context_id,
            "url": "data:text/html,<!doctype html><img id='target'>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let cases = [
        (4_u64, "() => document.images", "htmlcollection"),
        (5_u64, "() => document.querySelectorAll('img')", "nodelist"),
    ];
    for (id, function_declaration, expected_type) in cases {
        let response = send_bidi_command(
            &mut socket,
            id,
            "script.callFunction",
            json!({
                "functionDeclaration": function_declaration,
                "awaitPromise": false,
                "target": {
                    "context": context_id
                },
                "serializationOptions": {
                    "maxDomDepth": 1
                }
            }),
        )
        .await;
        assert_eq!(response["type"], json!("success"));
        assert_eq!(response["result"]["type"], json!("success"));
        let remote = &response["result"]["result"];
        assert_eq!(
            remote["type"],
            json!(expected_type),
            "{function_declaration} should serialize as {expected_type}: {response:?}"
        );
        let entries = remote["value"]
            .as_array()
            .unwrap_or_else(|| panic!("{expected_type} should include value array: {response:?}"));
        assert_eq!(
            entries.len(),
            1,
            "{expected_type} should contain the single img node: {response:?}"
        );
        let node = &entries[0];
        assert_eq!(node["type"], json!("node"));
        assert!(
            node["sharedId"]
                .as_str()
                .is_some_and(|shared_id| !shared_id.is_empty()),
            "collection node should include sharedId: {response:?}"
        );
        assert_eq!(node["value"]["nodeType"], json!(1));
        assert_eq!(node["value"]["localName"], json!("img"));
        assert_eq!(
            node["value"]["namespaceURI"],
            json!("http://www.w3.org/1999/xhtml")
        );
        assert_eq!(node["value"]["attributes"], json!({"id": "target"}));
        assert_eq!(node["value"]["childNodeCount"], json!(0));
    }

    let detached_node = send_bidi_command(
        &mut socket,
        6,
        "script.callFunction",
        json!({
            "functionDeclaration": "() => document.createElement('div')",
            "awaitPromise": false,
            "target": {
                "context": context_id
            },
            "serializationOptions": {
                "maxDomDepth": 1
            }
        }),
    )
    .await;
    assert_eq!(detached_node["type"], json!("success"));
    assert_eq!(
        detached_node["result"]["result"]["type"],
        json!("node"),
        "detached element should remain a BiDi node remote value: {detached_node:?}"
    );
    assert!(
        detached_node["result"]["result"]["sharedId"]
            .as_str()
            .is_some_and(|shared_id| !shared_id.is_empty()),
        "detached element should include sharedId: {detached_node:?}"
    );
    assert_eq!(
        detached_node["result"]["result"]["value"]["attributes"],
        json!({})
    );
    assert_eq!(
        detached_node["result"]["result"]["value"]["childNodeCount"],
        json!(0)
    );
    assert_eq!(
        detached_node["result"]["result"]["value"]["children"],
        json!([])
    );
    assert_eq!(
        detached_node["result"]["result"]["value"]["localName"],
        json!("div")
    );
    assert_eq!(
        detached_node["result"]["result"]["value"]["namespaceURI"],
        json!("http://www.w3.org/1999/xhtml")
    );
    assert_eq!(
        detached_node["result"]["result"]["value"]["nodeType"],
        json!(1)
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_get_realms_invalid_parameters_match_wpt_error_shape() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/script/get_realms/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 180_u64;

    for params in [
        json!({"context": false}),
        json!({"context": 42}),
        json!({"context": {}}),
        json!({"context": []}),
        json!({"type": false}),
        json!({"type": 42}),
        json!({"type": {}}),
        json!({"type": []}),
        json!({"type": "foo"}),
    ] {
        id += 1;
        let response = send_bidi_command(&mut socket, id, "script.getRealms", params.clone()).await;
        assert_bidi_error(
            &response,
            "invalid argument",
            &format!("script.getRealms params should be invalid argument: {params}"),
        );
    }

    let missing_context = send_bidi_command(
        &mut socket,
        id + 1,
        "script.getRealms",
        json!({"context": "foo"}),
    )
    .await;
    assert_bidi_error(
        &missing_context,
        "no such frame",
        "script.getRealms context should be missing",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_get_realms_materializes_initial_about_blank_realms() {
    // Mirrors webdriver/tests/bidi/script/call_function/realm.py: newly-created
    // about:blank tabs must expose default realms before any explicit navigate.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    let first_create = send_bidi_command(
        &mut socket,
        2,
        "browsingContext.create",
        json!({"type": "tab"}),
    )
    .await;
    assert_eq!(first_create["type"], json!("success"));
    let first_context_id = first_create["result"]["context"]
        .as_str()
        .expect("first context id")
        .to_owned();

    let second_create = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({"type": "tab"}),
    )
    .await;
    assert_eq!(second_create["type"], json!("success"));
    let second_context_id = second_create["result"]["context"]
        .as_str()
        .expect("second context id")
        .to_owned();

    let realms = send_bidi_command(&mut socket, 4, "script.getRealms", json!({})).await;
    assert_eq!(
        realms["type"],
        json!("success"),
        "script.getRealms should materialize initial about:blank realms: {realms:?}"
    );
    let first_realm_id = bidi_default_window_realm_id(&realms, &first_context_id);
    let second_realm_id = bidi_default_window_realm_id(&realms, &second_context_id);
    assert_ne!(
        first_realm_id, second_realm_id,
        "distinct tabs should expose distinct default realms"
    );

    let first_set = send_bidi_command(
        &mut socket,
        5,
        "script.callFunction",
        json!({
            "functionDeclaration": "() => { window.foo = 3; }",
            "target": {
                "realm": first_realm_id.clone()
            },
            "awaitPromise": true
        }),
    )
    .await;
    assert_eq!(first_set["type"], json!("success"));
    assert_eq!(first_set["result"]["realm"], json!(first_realm_id));
    assert_eq!(first_set["result"]["result"], json!({"type": "undefined"}));

    let second_set = send_bidi_command(
        &mut socket,
        6,
        "script.callFunction",
        json!({
            "functionDeclaration": "() => { window.foo = 5; }",
            "target": {
                "realm": second_realm_id.clone()
            },
            "awaitPromise": true
        }),
    )
    .await;
    assert_eq!(second_set["type"], json!("success"));
    assert_eq!(second_set["result"]["realm"], json!(second_realm_id));
    assert_eq!(second_set["result"]["result"], json!({"type": "undefined"}));

    let first_get = send_bidi_command(
        &mut socket,
        7,
        "script.callFunction",
        json!({
            "functionDeclaration": "() => window.foo",
            "target": {
                "realm": first_realm_id.clone()
            },
            "awaitPromise": true
        }),
    )
    .await;
    assert_eq!(first_get["type"], json!("success"));
    assert_eq!(first_get["result"]["realm"], json!(first_realm_id));
    assert_eq!(
        first_get["result"]["result"],
        json!({"type": "number", "value": 3})
    );

    let second_get = send_bidi_command(
        &mut socket,
        8,
        "script.callFunction",
        json!({
            "functionDeclaration": "() => window.foo",
            "target": {
                "realm": second_realm_id.clone()
            },
            "awaitPromise": true
        }),
    )
    .await;
    assert_eq!(second_get["type"], json!("success"));
    assert_eq!(second_get["result"]["realm"], json!(second_realm_id));
    assert_eq!(
        second_get["result"]["result"],
        json!({"type": "number", "value": 5})
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_get_realms_tracks_reload_and_multiple_contexts() {
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
        .expect("send first browsingContext.create");
    let first_create = recv_ws_json(&mut socket).await;
    assert_eq!(first_create["type"], json!("success"));
    let first_context_id = first_create["result"]["context"]
        .as_str()
        .expect("first context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": first_context_id.clone(),
                    "url": "data:text/html,<title>Realm A1</title>realm-a1",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send first context navigate");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.getRealms",
                "params": {
                    "context": first_context_id.clone(),
                    "type": "window"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send first script.getRealms");
    let first_realms = recv_ws_json(&mut socket).await;
    let first_realm_id = bidi_default_window_realm_id(&first_realms, &first_context_id);

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.getRealms",
                "params": {
                    "context": first_context_id.clone(),
                    "type": "window"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send repeated script.getRealms");
    let repeated_realms = recv_ws_json(&mut socket).await;
    assert_eq!(
        bidi_default_window_realm_id(&repeated_realms, &first_context_id),
        first_realm_id,
        "script.getRealms should keep the default window realm stable before navigation"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": first_context_id.clone(),
                    "url": "data:text/html,<title>Realm A2</title>realm-a2",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send first context reload-like navigate");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "script.getRealms",
                "params": {
                    "context": first_context_id.clone(),
                    "type": "window"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send post-navigation script.getRealms");
    let reloaded_realms = recv_ws_json(&mut socket).await;
    let reloaded_realm_id = bidi_default_window_realm_id(&reloaded_realms, &first_context_id);
    assert_ne!(
        reloaded_realm_id, first_realm_id,
        "script.getRealms should expose a new default window realm after navigation"
    );

    for (id, method, params) in [
        (
            70_u64,
            "script.evaluate",
            json!({
                "expression": "1 + 2",
                "target": {
                    "realm": first_realm_id.clone()
                }
            }),
        ),
        (
            71_u64,
            "script.callFunction",
            json!({
                "functionDeclaration": "() => 3",
                "target": {
                    "realm": first_realm_id.clone()
                }
            }),
        ),
        (
            72_u64,
            "script.disown",
            json!({
                "handles": [],
                "target": {
                    "realm": first_realm_id.clone()
                }
            }),
        ),
    ] {
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
            .expect("send stale realm script command");
        let stale_realm = recv_ws_json(&mut socket).await;
        assert_eq!(
            stale_realm["type"],
            json!("error"),
            "stale realm target should fail for {method}: {stale_realm:?}"
        );
        assert_eq!(stale_realm["id"], json!(id));
        assert_eq!(stale_realm["error"], json!("no such frame"));
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "browsingContext.create",
                "params": {
                    "type": "tab"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send second browsingContext.create");
    let second_create = recv_ws_json(&mut socket).await;
    assert_eq!(second_create["type"], json!("success"));
    let second_context_id = second_create["result"]["context"]
        .as_str()
        .expect("second context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": second_context_id.clone(),
                    "url": "data:text/html,<title>Realm B</title>realm-b",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send second context navigate");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 10_u64,
                "method": "script.getRealms",
                "params": {
                    "context": second_context_id.clone(),
                    "type": "window"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send second context script.getRealms");
    let second_realms = recv_ws_json(&mut socket).await;
    let second_realm_id = bidi_default_window_realm_id(&second_realms, &second_context_id);

    socket
        .send(WsMessage::Text(
            json!({
                "id": 11_u64,
                "method": "script.getRealms",
                "params": {
                    "context": first_context_id.clone(),
                    "type": "window"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send first context script.getRealms after second navigate");
    let first_after_second_realms = recv_ws_json(&mut socket).await;
    assert_eq!(
        bidi_default_window_realm_id(&first_after_second_realms, &first_context_id),
        reloaded_realm_id
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 12_u64,
                "method": "script.getRealms",
                "params": {
                    "type": "window"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send all-context script.getRealms");
    let all_realms = recv_ws_json(&mut socket).await;
    assert_eq!(
        bidi_default_window_realm_id(&all_realms, &first_context_id),
        reloaded_realm_id
    );
    assert_eq!(
        bidi_default_window_realm_id(&all_realms, &second_context_id),
        second_realm_id
    );
    assert_ne!(
        second_realm_id, reloaded_realm_id,
        "distinct top-level contexts should expose distinct window realms"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}
#[tokio::test]
async fn websocket_bidi_get_realms_exposes_iframe_context_origin() {
    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Child Realm</title><main>child-realm</main>",
        )
    }

    let child_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi child realm fixture listener");
    let child_addr = child_listener
        .local_addr()
        .expect("BiDi child realm fixture addr");
    let child_origin = format!("http://{child_addr}");
    let child_url = format!("{child_origin}/child");
    let child_app = Router::new().route("/child", get(child));
    let child_server = tokio::spawn(async move { axum::serve(child_listener, child_app).await });

    let parent_html = format!(
        r#"<!doctype html>
<html>
<head><title>Top Realm</title></head>
<body><main>top-realm</main><iframe src="{child_url}"></iframe></body>
</html>"#
    );
    let parent_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi parent realm fixture listener");
    let parent_addr = parent_listener
        .local_addr()
        .expect("BiDi parent realm fixture addr");
    let parent_origin = format!("http://{parent_addr}");
    let parent_url = format!("{parent_origin}/");
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
    let parent_server = tokio::spawn(async move { axum::serve(parent_listener, parent_app).await });

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
    let top_realm = bidi_window_realm(&all_realms, &context_id);
    assert_eq!(top_realm["origin"], json!(parent_origin));

    let window_realms = all_realms["result"]["realms"]
        .as_array()
        .expect("script.getRealms should return realm array");
    assert_eq!(
        window_realms.len(),
        2,
        "all-context getRealms should expose top and iframe realms exactly once: {all_realms:?}"
    );
    let child_realm = window_realms
        .iter()
        .find(|realm| {
            realm["type"] == json!("window")
                && realm["context"].as_str().is_some_and(|id| id != context_id)
        })
        .unwrap_or_else(|| {
            panic!("script.getRealms should include iframe window realm: {all_realms:?}")
        });
    assert_eq!(child_realm["origin"], json!(child_origin));
    let child_context_id = child_realm["context"]
        .as_str()
        .expect("child realm context id")
        .to_owned();
    let child_realm_id = child_realm["realm"]
        .as_str()
        .expect("child realm id")
        .to_owned();
    assert_ne!(
        child_realm_id,
        top_realm["realm"].as_str().expect("top realm id"),
        "iframe should expose a distinct window realm"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "window.__iframeSandboxValue = 17",
                    "awaitPromise": true,
                    "target": {
                        "context": child_context_id.clone(),
                        "sandbox": "sandbox"
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send iframe sandbox script.evaluate");
    let iframe_sandbox_evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(
        iframe_sandbox_evaluate["type"],
        json!("success"),
        "iframe sandbox evaluate should succeed: {iframe_sandbox_evaluate:?}"
    );
    assert_eq!(
        iframe_sandbox_evaluate["result"]["result"],
        json!({"type": "number", "value": 17})
    );
    let iframe_sandbox_realm_id = iframe_sandbox_evaluate["result"]["realm"]
        .as_str()
        .expect("iframe sandbox evaluate should report realm")
        .to_owned();
    assert_ne!(
        iframe_sandbox_realm_id, child_realm_id,
        "iframe sandbox should use a non-default realm"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "window.__iframeSandboxValue",
                    "awaitPromise": true,
                    "target": {
                        "context": child_context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send iframe default script.evaluate");
    let iframe_default_probe = recv_ws_json(&mut socket).await;
    assert_eq!(
        iframe_default_probe["type"],
        json!("success"),
        "iframe default evaluate should succeed: {iframe_default_probe:?}"
    );
    assert_eq!(
        iframe_default_probe["result"]["result"],
        json!({"type": "undefined"}),
        "iframe sandbox globals must not leak to default realm"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "script.getRealms",
                "params": {
                    "context": context_id.clone(),
                    "type": "window"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send top-context script.getRealms");
    let top_context_realms = recv_ws_json(&mut socket).await;
    let top_context_window_realms = top_context_realms["result"]["realms"]
        .as_array()
        .expect("top context getRealms should return realm array");
    assert_eq!(
        top_context_window_realms.len(),
        1,
        "top context getRealms should not include iframe realms: {top_context_realms:?}"
    );
    assert_eq!(top_context_window_realms[0]["context"], json!(context_id));
    assert_eq!(top_context_window_realms[0]["origin"], json!(parent_origin));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "script.getRealms",
                "params": {
                    "context": child_context_id.clone(),
                    "type": "window"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send iframe-context script.getRealms");
    let child_context_realms = recv_ws_json(&mut socket).await;
    assert_eq!(
        child_context_realms["type"],
        json!("success"),
        "iframe context getRealms should succeed: {child_context_realms:?}"
    );
    let child_context_window_realms = child_context_realms["result"]["realms"]
        .as_array()
        .expect("child context getRealms should return realm array");
    assert_eq!(
        child_context_window_realms.len(),
        2,
        "iframe context getRealms should include default and sandbox realms: {child_context_realms:?}"
    );
    assert_eq!(
        child_context_window_realms[0]["realm"],
        json!(child_realm_id),
        "iframe context getRealms should report the default window realm before sandbox realms: {child_context_realms:?}"
    );
    assert_eq!(
        child_context_window_realms[0]["sandbox"],
        json!(null),
        "default iframe window realm should not carry a sandbox name: {child_context_realms:?}"
    );
    assert_eq!(
        child_context_window_realms[1]["realm"],
        json!(iframe_sandbox_realm_id),
        "iframe context getRealms should report sandbox realms after the default realm: {child_context_realms:?}"
    );
    let child_default_realm = bidi_window_realm(&child_context_realms, &child_context_id);
    assert_eq!(child_default_realm["origin"], json!(child_origin));
    assert_eq!(child_default_realm["realm"], json!(child_realm_id));
    let child_sandbox_realm =
        bidi_sandbox_window_realm(&child_context_realms, &child_context_id, "sandbox");
    assert_eq!(child_sandbox_realm["origin"], json!(child_origin));
    assert_eq!(child_sandbox_realm["realm"], json!(iframe_sandbox_realm_id));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "window.__iframeSandboxValue",
                    "awaitPromise": true,
                    "target": {
                        "realm": iframe_sandbox_realm_id
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send iframe sandbox realm-target script.evaluate");
    let iframe_sandbox_realm_probe = recv_ws_json(&mut socket).await;
    assert_eq!(
        iframe_sandbox_realm_probe["type"],
        json!("success"),
        "iframe sandbox realm target should succeed: {iframe_sandbox_realm_probe:?}"
    );
    assert_eq!(
        iframe_sandbox_realm_probe["result"]["result"],
        json!({"type": "number", "value": 17})
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 10_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "window",
                    "awaitPromise": false,
                    "target": {
                        "context": child_context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send iframe context window script.evaluate");
    let iframe_window_probe = recv_ws_json(&mut socket).await;
    assert_eq!(
        iframe_window_probe["type"],
        json!("success"),
        "iframe context window evaluate should succeed: {iframe_window_probe:?}"
    );
    assert_eq!(
        iframe_window_probe["result"]["result"],
        json!({
            "type": "window",
            "value": {
                "context": child_context_id.clone()
            }
        }),
        "iframe context window should serialize as its own BiDi window remote value: {iframe_window_probe:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 11_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "window.frames[0]",
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
        .expect("send parent context child window script.evaluate");
    let parent_child_window_probe = recv_ws_json(&mut socket).await;
    assert_eq!(
        parent_child_window_probe["type"],
        json!("success"),
        "parent context child window evaluate should succeed: {parent_child_window_probe:?}"
    );
    assert_eq!(
        parent_child_window_probe["result"]["result"],
        json!({
            "type": "window",
            "value": {
                "context": child_context_id.clone()
            }
        }),
        "parent-evaluated window.frames[0] should serialize as the iframe BiDi window remote value: {parent_child_window_probe:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 12_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "() => document.querySelector('iframe').contentWindow",
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
        .expect("send parent context iframe contentWindow script.callFunction");
    let parent_content_window_probe = recv_ws_json(&mut socket).await;
    assert_eq!(
        parent_content_window_probe["type"],
        json!("success"),
        "parent context iframe contentWindow callFunction should succeed: {parent_content_window_probe:?}"
    );
    assert_eq!(
        parent_content_window_probe["result"]["result"],
        json!({
            "type": "window",
            "value": {
                "context": child_context_id.clone()
            }
        }),
        "parent-evaluated iframe.contentWindow should serialize as the iframe BiDi window remote value: {parent_content_window_probe:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 13_u64,
                "method": "script.getRealms",
                "params": {
                    "context": "missing-frame-context",
                    "type": "window"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send unknown-context script.getRealms");
    let unknown_context_realms = recv_ws_json(&mut socket).await;
    assert_eq!(unknown_context_realms["type"], json!("error"));
    assert_eq!(unknown_context_realms["id"], json!(13_u64));
    assert_eq!(unknown_context_realms["error"], json!("no such frame"));

    let _ = socket.close(None).await;
    protocol_server.abort();
    parent_server.abort();
    child_server.abort();
}
