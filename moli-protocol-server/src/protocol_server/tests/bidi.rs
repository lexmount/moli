use super::*;

#[tokio::test]
async fn websocket_bidi_session_route_handles_static_session_commands() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.status",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.status");
    let status = recv_ws_json(&mut socket).await;
    assert_eq!(status["type"], json!("success"));
    assert_eq!(status["id"], json!(1_u64));
    assert_eq!(status["result"]["ready"], json!(true));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "script.evaluate",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send unbound script.evaluate");
    let unbound = recv_ws_json(&mut socket).await;
    assert_eq!(unbound["type"], json!("error"));
    assert_eq!(unbound["id"], json!(2_u64));
    assert_eq!(unbound["error"], json!("invalid session id"));

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_new_reports_route_url_and_end_closes_socket() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session/"))
        .await
        .expect("connect to trailing-slash BiDi websocket");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 1_u64,
                "method": "session.new",
                "params": {
                    "capabilities": {}
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.new");
    let session = recv_ws_json(&mut socket).await;
    assert_eq!(session["type"], json!("success"));
    assert_eq!(session["id"], json!(1_u64));
    assert_eq!(session["result"]["sessionId"], json!("bidi-session-1"));
    assert_eq!(
        session["result"]["capabilities"]["webSocketUrl"],
        json!(format!("ws://{cdp_addr}/session"))
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "session.end",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.end");
    let end = recv_ws_json(&mut socket).await;
    assert_eq!(end["type"], json!("success"));
    assert_eq!(end["id"], json!(2_u64));
    assert_eq!(end["result"], json!({}));

    let closed = timeout(Duration::from_secs(1), socket.next())
        .await
        .expect("BiDi socket should close after session.end");
    assert!(matches!(
        closed,
        Some(Ok(WsMessage::Close(_))) | None | Some(Err(_))
    ));

    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_browser_close_closes_socket() {
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
                "method": "browser.close",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browser.close");
    let close = recv_ws_json(&mut socket).await;
    assert_eq!(close["type"], json!("success"));
    assert_eq!(close["id"], json!(2_u64));
    assert_eq!(close["result"], json!({}));

    let closed = timeout(Duration::from_secs(1), socket.next())
        .await
        .expect("BiDi socket should close after browser.close");
    assert!(matches!(
        closed,
        Some(Ok(WsMessage::Close(_))) | None | Some(Err(_))
    ));

    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_existing_classic_session_rejects_duplicate_upgrade() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let session_id = classic_new_session_on_server(cdp_addr).await;
    let mut first_socket = connect_classic_session_bidi_socket(cdp_addr, &session_id).await;

    let contexts =
        send_bidi_command(&mut first_socket, 1, "browser.getUserContexts", json!({})).await;
    assert_eq!(
        contexts["type"],
        json!("success"),
        "first attached socket should own the Classic session: {contexts:?}"
    );

    let duplicate_status =
        rejected_websocket_status(format!("ws://{cdp_addr}/session/{session_id}")).await;
    assert_eq!(duplicate_status, StatusCode::CONFLICT.as_u16());

    let _ = first_socket.close(None).await;
    protocol_server.abort();
}

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
async fn websocket_bidi_existing_classic_session_get_tree_includes_initial_context() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let session_id = classic_new_session_on_server(cdp_addr).await;
    let mut socket = connect_classic_session_bidi_socket(cdp_addr, &session_id).await;

    let tree = send_bidi_command(&mut socket, 1, "browsingContext.getTree", json!({})).await;
    assert_eq!(tree["type"], json!("success"));
    let contexts = tree["result"]["contexts"]
        .as_array()
        .expect("getTree contexts");
    assert_eq!(
        contexts.len(),
        1,
        "attached Classic session should expose the initial top-level context: {tree:?}"
    );
    assert_eq!(contexts[0]["url"], json!("about:blank"));
    assert_eq!(contexts[0]["parent"], json!(null));

    let _ = socket.close(None).await;
    protocol_server.abort();
}

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
async fn websocket_bidi_existing_classic_session_preload_channel_mutation_observer_emits_message() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let session_id = classic_new_session_on_server(cdp_addr).await;
    let mut socket = connect_classic_session_bidi_socket(cdp_addr, &session_id).await;

    let tree = send_bidi_command(&mut socket, 1, "browsingContext.getTree", json!({})).await;
    assert_eq!(tree["type"], json!("success"));
    let context_id = tree["result"]["contexts"][0]["context"]
        .as_str()
        .expect("attached Classic context id")
        .to_owned();

    let subscribe = send_bidi_command(
        &mut socket,
        2,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let preload_script = send_bidi_add_preload_script(
        &mut socket,
        3,
        "(channel) => {
            const onMutation = (mutationList) => mutationList.forEach((mutation) => {
                const attributeName = mutation.attributeName;
                const newValue = mutation.target.getAttribute(mutation.attributeName);
                channel({ attributeName, newValue });
            });
            const observer = new MutationObserver(onMutation);
            observer.observe(document, { attributes: true, subtree: true });
        }",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "classic_attached_channel"
            }
        })],
    )
    .await;

    let page_url =
        classic_data_url_for_bidi_test("<!doctype html><div class='old class name'>foo</div>");
    let navigated = classic_request_on_server_with_body(
        cdp_addr,
        "POST",
        &format!("/session/{session_id}/url"),
        json!({ "url": page_url }),
    )
    .await;
    assert_eq!(navigated, json!({ "value": null }));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "document.querySelector('div').setAttribute('class', 'mutated')",
                    "target": {
                        "context": context_id.clone()
                    },
                    "awaitPromise": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send attached Classic mutation script.evaluate");
    let messages = recv_until_id(&mut socket, 4).await;
    let messages =
        collect_bidi_messages_until_method_count(&mut socket, messages, "script.message", 1).await;
    let evaluate = bidi_message_by_id(&messages, 4);
    assert_eq!(evaluate["type"], json!("success"), "{messages:#?}");
    let realm = evaluate["result"]["realm"]
        .as_str()
        .expect("attached mutation evaluate realm");
    let event = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("attached mutation observer script.message event");
    assert_eq!(
        event["params"],
        json!({
            "channel": "classic_attached_channel",
            "data": {
                "type": "object",
                "value": [
                    ["attributeName", {"type": "string", "value": "class"}],
                    ["newValue", {"type": "string", "value": "mutated"}]
                ]
            },
            "source": {
                "realm": realm,
                "context": context_id
            }
        })
    );
    remove_bidi_preload_script(&mut socket, 5, &preload_script).await;

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_existing_classic_session_uses_file_prompt_capability() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let session_id = classic_new_session_on_server_with_body(
        cdp_addr,
        json!({
            "capabilities": {
                "alwaysMatch": {
                    "unhandledPromptBehavior": "accept"
                }
            }
        }),
    )
    .await;
    let mut socket = connect_classic_session_bidi_socket(cdp_addr, &session_id).await;

    let tree = send_bidi_command(&mut socket, 1, "browsingContext.getTree", json!({})).await;
    assert_eq!(tree["type"], json!("success"));
    let context_id = tree["result"]["contexts"][0]["context"]
        .as_str()
        .expect("attached Classic context id")
        .to_owned();
    let cancel = send_bidi_command(
        &mut socket,
        2,
        "script.evaluate",
        json!({
            "expression": r#"
new Promise(resolve => {
  const picker = document.createElement('input');
  picker.type = 'file';
  picker.addEventListener('cancel', event => {
    resolve(event.isTrusted);
  });
  picker.click();
})
"#,
            "awaitPromise": true,
            "target": {
                "context": context_id
            },
            "userActivation": true
        }),
    )
    .await;
    assert_eq!(cancel["type"], json!("success"), "{cancel:?}");
    assert_eq!(
        cancel["result"]["result"],
        json!({
            "type": "boolean",
            "value": true
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_classic_session_omits_synthetic_service_worker_runtime() {
    let fixture_app = Router::new()
        .route(
            "/page",
            get(|| async move {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><title>classic bidi service worker</title><main>ready</main>",
                )
            }),
        )
        .route(
            "/sw.js",
            get(|| async move {
                (
                    [(
                        axum::http::header::CONTENT_TYPE.as_str(),
                        "text/javascript; charset=utf-8",
                    )],
                    "console.log('classic-bidi-service-worker-log');\n\
                     self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));\n\
                     self.addEventListener('activate', event => event.waitUntil(self.clients.claim()));",
                )
            }),
        );
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "bidi-classic-service-worker-surface");
    let page_url = format!("http://{fixture_addr}/page");
    let worker_url = format!("http://{fixture_addr}/sw.js");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let session_id = classic_new_session_on_server(cdp_addr).await;
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
    assert_eq!(tree["type"], json!("success"), "{tree:?}");
    let context_id = tree["result"]["contexts"][0]["context"]
        .as_str()
        .expect("Classic-owned top-level context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 2_u64,
                "method": "session.subscribe",
                "params": {
                    "events": [
                        "browsingContext.contextCreated",
                        "script.realmCreated",
                        "log.entryAdded"
                    ]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send service worker surface subscription");
    let mut messages = recv_until_id(&mut socket, 2).await;
    let subscribe = bidi_message_by_id(&messages, 2);
    assert_eq!(subscribe["type"], json!("success"), "{subscribe:?}");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": r#"
(async () => {
  const registration = await navigator.serviceWorker.register('/sw.js', { scope: '/' });
  await navigator.serviceWorker.ready;
  const worker = registration.active || registration.waiting ||
      registration.installing || navigator.serviceWorker.controller;
  return worker ? worker.scriptURL : 'missing-service-worker';
})()
"#,
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
        .expect("send service worker registration script.evaluate");
    messages.extend(recv_until_id(&mut socket, 3).await);
    let evaluate = bidi_message_by_id(&messages, 3);
    assert_eq!(evaluate["type"], json!("success"), "{messages:#?}");
    assert_eq!(
        evaluate["result"]["result"],
        json!({
            "type": "string",
            "value": worker_url
        }),
        "service worker registration should resolve with the active script URL: {messages:#?}"
    );

    let service_worker_context = service_worker_context_created(&messages, &worker_url)
        .and_then(|message| message["params"]["context"].as_str())
        .expect("service worker browsing context should be exposed")
        .to_owned();

    assert!(
        messages.iter().all(|message| {
            message["method"] != json!("script.realmCreated")
                || message["params"]["origin"] != json!(worker_url)
                || message["params"]["type"] == json!("service-worker")
        }),
        "a real Service Worker Runtime context must not be exposed as a generic worker realm: {messages:#?}"
    );

    let service_worker_realm = service_worker_realm_created(&messages);
    if let Some(log) = service_worker_log_entry(&messages, &service_worker_context) {
        let realm = service_worker_realm.unwrap_or_else(|| {
            panic!("a Service Worker log must wait for its real Runtime realm: {messages:#?}")
        });
        assert_eq!(
            log["params"]["source"]["realm"], realm["params"]["realm"],
            "Service Worker logs must use the realm id from Runtime.executionContextCreated: {messages:#?}"
        );
        let realm_index = messages
            .iter()
            .position(|message| std::ptr::eq(message, realm))
            .expect("service worker realm position");
        let log_index = messages
            .iter()
            .position(|message| std::ptr::eq(message, log))
            .expect("service worker log position");
        assert!(
            realm_index < log_index,
            "Service Worker realmCreated must precede its log entry: {messages:#?}"
        );
    }

    let realms = send_bidi_command(
        &mut socket,
        4,
        "script.getRealms",
        json!({
            "context": service_worker_context,
            "type": "service-worker"
        }),
    )
    .await;
    assert_eq!(realms["type"], json!("success"), "{realms:?}");
    let returned_realms = realms["result"]["realms"]
        .as_array()
        .expect("script.getRealms result array");
    assert!(
        returned_realms
            .iter()
            .all(|realm| realm["type"] == json!("service-worker")),
        "script.getRealms must expose only real Service Worker-typed realms for the worker target: {realms:?}"
    );
    if let Some(service_worker_realm) = service_worker_realm {
        assert!(
            returned_realms
                .iter()
                .any(|realm| { realm["realm"] == service_worker_realm["params"]["realm"] }),
            "script.getRealms must retain a Service Worker realm that was already created: {realms:?}"
        );
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_shared_worker_subscription_projects_runtime_listener_predecessor() {
    let (fixture_addr, _fixture_server) = spawn_shared_worker_fixture_server("bidi-shared-worker");
    let page_url = format!("http://{fixture_addr}/");
    let worker_url = format!("http://{fixture_addr}/shared-worker.js");

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
    assert_eq!(navigate["type"], json!("success"), "{navigate:?}");

    let subscribe = send_bidi_command_response(
        &mut socket,
        4,
        "session.subscribe",
        json!({
            "events": [
                "browsingContext.contextCreated",
                "script.realmCreated"
            ]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"), "{subscribe:?}");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "globalThis.__sharedWorkerProbe('bidi').then(value => JSON.stringify(value))",
                    "target": { "context": context_id },
                    "awaitPromise": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send shared worker script.evaluate");
    let messages = recv_until_id(&mut socket, 5).await;
    let messages = collect_bidi_messages_until(
        &mut socket,
        messages,
        |messages| {
            let saw_context = messages.iter().any(|message| {
                message["method"] == json!("browsingContext.contextCreated")
                    && message["params"]["url"] == json!(worker_url)
            });
            let saw_realm = messages.iter().any(|message| {
                message["method"] == json!("script.realmCreated")
                    && message["params"]["type"] == json!("shared-worker")
            });
            saw_context && saw_realm
        },
        "shared worker context and realm events",
    )
    .await;

    let evaluate = bidi_message_by_id(&messages, 5);
    assert_eq!(evaluate["type"], json!("success"), "{messages:#?}");
    let probe: serde_json::Value = serde_json::from_str(
        evaluate["result"]["result"]["value"]
            .as_str()
            .expect("shared worker probe JSON string"),
    )
    .expect("parse shared worker probe JSON");
    assert_eq!(probe["echoed"], json!("bidi"));
    assert_eq!(probe["isSharedWorker"], json!(true));

    let end = send_bidi_command_response(&mut socket, 6, "session.end", json!({})).await;
    assert_eq!(end["type"], json!("success"), "{end:?}");
    let closed = timeout(Duration::from_secs(1), socket.next())
        .await
        .expect("BiDi socket should close after releasing shared worker event sources");
    assert!(matches!(
        closed,
        Some(Ok(WsMessage::Close(_))) | None | Some(Err(_))
    ));

    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_registry_allocates_unique_ids_across_connections() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut first_socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect first BiDi websocket");
    let (mut second_socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect second BiDi websocket");

    first_socket
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
        .expect("send first session.new");
    let first_session = recv_ws_json(&mut first_socket).await;

    second_socket
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
        .expect("send second session.new");
    let second_session = recv_ws_json(&mut second_socket).await;

    assert_eq!(
        first_session["result"]["sessionId"],
        json!("bidi-session-1")
    );
    assert_eq!(
        second_session["result"]["sessionId"],
        json!("bidi-session-2")
    );

    let _ = first_socket.close(None).await;
    let _ = second_socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_subscribe_rejects_unknown_context_and_user_context() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    let missing_context = send_bidi_command(
        &mut socket,
        2,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "contexts": ["missing-context"]
        }),
    )
    .await;
    assert_bidi_error(
        &missing_context,
        "no such frame",
        "session.subscribe contexts should reject unknown browsing contexts",
    );

    let missing_user_context = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "userContexts": ["missing-user-context"]
        }),
    )
    .await;
    assert_bidi_error(
        &missing_user_context,
        "no such user context",
        "session.subscribe userContexts should reject unknown user contexts",
    );

    let default_user_context = send_bidi_command(
        &mut socket,
        4,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "userContexts": ["default"]
        }),
    )
    .await;
    assert_eq!(default_user_context["type"], json!("success"));

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_subscribe_replays_existing_script_realm_created() {
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
                    "url": "data:text/html,<body>realm event</body>",
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
                "method": "session.subscribe",
                "params": {
                    "events": ["script.realmCreated"],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");

    let mut saw_realm_created = false;
    let mut saw_response = false;
    let mut observed = Vec::new();
    for _ in 0..8 {
        let message = timeout(Duration::from_secs(2), recv_ws_json(&mut socket))
            .await
            .expect("subscribe should replay existing realm event and response");
        observed.push(message.clone());
        if message["type"] == json!("event") {
            assert_eq!(message["method"], json!("script.realmCreated"));
            assert_eq!(message["params"]["type"], json!("window"));
            assert_eq!(message["params"]["context"], json!(context_id.as_str()));
            assert!(message["params"].get("sandbox").is_none());
            assert!(
                message["params"]["realm"].as_str().is_some(),
                "realmCreated should carry a realm id: {message:?}"
            );
            saw_realm_created = true;
        } else if message["id"] == json!(4_u64) {
            assert_eq!(message["type"], json!("success"));
            assert!(
                message["result"]["subscription"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("00000000-0000-4000-8000-"))
            );
            saw_response = true;
        }
        if saw_realm_created && saw_response {
            break;
        }
    }
    assert!(
        saw_realm_created,
        "expected script.realmCreated event; observed={observed:?}"
    );
    assert!(
        saw_response,
        "expected session.subscribe response; observed={observed:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_subscribe_replays_existing_script_realm_created_by_user_context() {
    // Derived from Chromium/WPT
    // webdriver/tests/bidi/session/subscribe/user_contexts.py userContext filtering semantics.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    let user_context =
        send_bidi_command(&mut socket, 2, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context id")
        .to_owned();

    let default_context = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": "default"
        }),
    )
    .await;
    assert_eq!(default_context["type"], json!("success"));
    let default_context_id = default_context["result"]["context"]
        .as_str()
        .expect("created default context id")
        .to_owned();

    let user_context_tab = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(user_context_tab["type"], json!("success"));
    let user_context_tab_id = user_context_tab["result"]["context"]
        .as_str()
        .expect("created user context tab id")
        .to_owned();

    let default_navigate = send_bidi_command(
        &mut socket,
        5,
        "browsingContext.navigate",
        json!({
            "context": default_context_id,
            "url": "data:text/html,<body>default realm</body>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(default_navigate["type"], json!("success"));

    let user_context_navigate = send_bidi_command(
        &mut socket,
        6,
        "browsingContext.navigate",
        json!({
            "context": user_context_tab_id,
            "url": "data:text/html,<body>user context realm</body>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(user_context_navigate["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["script.realmCreated"],
                    "userContexts": [user_context_id]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send userContext-scoped script.realmCreated subscribe");
    let subscribe_messages = recv_until_id(&mut socket, 7).await;
    assert_eq!(
        subscribe_messages.last().expect("subscribe response")["type"],
        json!("success")
    );
    let realm_events = subscribe_messages
        .iter()
        .filter(|message| message["method"] == json!("script.realmCreated"))
        .collect::<Vec<_>>();
    assert!(
        !realm_events.is_empty(),
        "userContext subscription should replay existing matching realms: {subscribe_messages:#?}"
    );
    assert!(
        realm_events
            .iter()
            .all(|event| event["params"]["context"] == json!(user_context_tab_id)),
        "userContext-scoped realm replay should not include other contexts: {subscribe_messages:#?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_subscribe_emits_browsing_context_lifecycle_events() {
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
                    "events": [
                        "browsingContext.domContentLoaded",
                        "browsingContext.load"
                    ],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");
    let subscribe = recv_ws_json(&mut socket).await;
    assert_eq!(subscribe["type"], json!("success"));

    let navigate_url = "data:text/html,<body>lifecycle-events</body>";
    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
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
    let messages = recv_until_id(&mut socket, 4).await;
    let navigate = messages
        .last()
        .expect("navigate response should be last message");
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(navigate["result"]["url"], json!(navigate_url));

    let lifecycle_events = messages
        .iter()
        .filter(|message| message["type"] == json!("event"))
        .collect::<Vec<_>>();
    assert_eq!(lifecycle_events.len(), 2, "messages: {messages:#?}");
    assert_eq!(
        lifecycle_events[0]["method"],
        json!("browsingContext.domContentLoaded")
    );
    assert_eq!(lifecycle_events[1]["method"], json!("browsingContext.load"));
    for event in lifecycle_events {
        assert_eq!(event["params"]["context"], json!(context_id));
        assert_eq!(
            event["params"]["navigation"],
            navigate["result"]["navigation"]
        );
        assert_eq!(event["params"]["url"], json!(navigate_url));
        assert!(
            event["params"]["timestamp"].as_u64().is_some(),
            "timestamp should be epoch milliseconds: {event:?}"
        );
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_subscribe_emits_context_created_before_create_response() {
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
                "method": "session.subscribe",
                "params": {
                    "events": ["browsingContext.contextCreated"]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
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
    let messages = recv_until_id(&mut socket, 3).await;
    let response_index = messages
        .iter()
        .position(|message| message["id"] == json!(3_u64))
        .expect("create response");
    let event_index = messages
        .iter()
        .position(|message| message["method"] == json!("browsingContext.contextCreated"))
        .expect("contextCreated event");
    assert!(
        event_index < response_index,
        "contextCreated should be emitted before browsingContext.create resolves: {messages:#?}"
    );

    let create = &messages[response_index];
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();
    let event = &messages[event_index];
    assert_eq!(event["type"], json!("event"));
    assert_eq!(event["params"]["context"], json!(context_id.clone()));
    assert_eq!(event["params"]["url"], json!("about:blank"));
    assert_eq!(event["params"]["children"], serde_json::Value::Null);
    assert_eq!(event["params"]["clientWindow"], json!(context_id));
    assert_eq!(event["params"]["originalOpener"], serde_json::Value::Null);
    assert_eq!(event["params"]["parent"], serde_json::Value::Null);
    assert_eq!(event["params"]["userContext"], json!("default"));

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_browsing_context_create_accepts_default_user_context() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/create/user_context.py.
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
            "type": "tab",
            "userContext": "default"
        }),
    )
    .await;
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    let tree = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.getTree",
        json!({ "root": context_id.clone() }),
    )
    .await;
    assert_eq!(tree["type"], json!("success"));
    assert_eq!(
        tree["result"]["contexts"][0]["userContext"],
        json!("default"),
        "default-created BiDi context should not expose Moli internal browserContextId"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_get_tree_root_reads_inactive_default_user_context() {
    // Derived from Chromium/WPT browsingContext.getTree root semantics combined with
    // webdriver/tests/bidi/browsing_context/create/user_context.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    let user_context =
        send_bidi_command(&mut socket, 2, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context id")
        .to_owned();

    let default_context = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": "default"
        }),
    )
    .await;
    assert_eq!(default_context["type"], json!("success"));
    let default_context_id = default_context["result"]["context"]
        .as_str()
        .expect("default context id")
        .to_owned();

    let default_navigate = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.navigate",
        json!({
            "context": default_context_id,
            "url": "data:text/html,<body>default context</body>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(default_navigate["type"], json!("success"));

    let custom_context = send_bidi_command(
        &mut socket,
        5,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(custom_context["type"], json!("success"));
    let custom_context_id = custom_context["result"]["context"]
        .as_str()
        .expect("custom context id")
        .to_owned();

    let custom_navigate = send_bidi_command(
        &mut socket,
        6,
        "browsingContext.navigate",
        json!({
            "context": custom_context_id,
            "url": "data:text/html,<body>custom context</body>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(custom_navigate["type"], json!("success"));

    let default_tree = send_bidi_command(
        &mut socket,
        7,
        "browsingContext.getTree",
        json!({ "root": default_context_id }),
    )
    .await;
    assert_eq!(
        default_tree["type"],
        json!("success"),
        "getTree(root) should read inactive default userContext: {default_tree:#?}"
    );
    assert_eq!(
        default_tree["result"]["contexts"][0]["userContext"],
        json!("default")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_browser_user_contexts_match_wpt_basics() {
    // Ported from Chromium/WPT webdriver/tests/bidi/browser/
    // create_user_context, get_user_contexts, and remove_user_context basics.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    let initial_contexts =
        send_bidi_command(&mut socket, 2, "browser.getUserContexts", json!({})).await;
    assert_eq!(initial_contexts["type"], json!("success"));
    assert!(
        bidi_user_context_ids(&initial_contexts).contains(&"default".to_owned()),
        "browser.getUserContexts should expose the default user context: {initial_contexts:?}"
    );

    let first = send_bidi_command(
        &mut socket,
        3,
        "browser.createUserContext",
        json!({
            "acceptInsecureCerts": true,
            "proxy": {
                "proxyType": "manual",
                "httpProxy": "127.0.0.1:80",
                "noProxy": ["localhost"]
            }
        }),
    )
    .await;
    assert_eq!(first["type"], json!("success"));
    let first_user_context = first["result"]["userContext"]
        .as_str()
        .expect("created user context")
        .to_owned();
    assert_ne!(first_user_context, "default");

    let second = send_bidi_command(&mut socket, 4, "browser.createUserContext", json!({})).await;
    assert_eq!(second["type"], json!("success"));
    let second_user_context = second["result"]["userContext"]
        .as_str()
        .expect("second user context")
        .to_owned();
    assert_ne!(first_user_context, second_user_context);

    let listed = send_bidi_command(&mut socket, 5, "browser.getUserContexts", json!({})).await;
    let listed_ids = bidi_user_context_ids(&listed);
    assert!(listed_ids.contains(&"default".to_owned()));
    assert!(listed_ids.contains(&first_user_context));
    assert!(listed_ids.contains(&second_user_context));

    let created_context = send_bidi_command(
        &mut socket,
        6,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": first_user_context
        }),
    )
    .await;
    assert_eq!(created_context["type"], json!("success"));
    let context_id = created_context["result"]["context"]
        .as_str()
        .expect("context in user context")
        .to_owned();

    let tree = send_bidi_command(
        &mut socket,
        7,
        "browsingContext.getTree",
        json!({ "root": context_id.clone() }),
    )
    .await;
    assert_eq!(tree["type"], json!("success"));
    assert_eq!(
        tree["result"]["contexts"][0]["userContext"],
        json!(first_user_context)
    );

    let subscribe = send_bidi_command(
        &mut socket,
        8,
        "session.subscribe",
        json!({
            "events": ["browsingContext.contextDestroyed"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "browser.removeUserContext",
                "params": {
                    "userContext": first_user_context
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browser.removeUserContext");
    let remove_messages = recv_until_id(&mut socket, 9).await;
    let remove = remove_messages
        .iter()
        .find(|message| message["id"] == json!(9_u64))
        .expect("remove response");
    assert_eq!(remove["type"], json!("success"));
    let destroyed = remove_messages
        .iter()
        .find(|message| message["method"] == json!("browsingContext.contextDestroyed"))
        .unwrap_or_else(|| {
            panic!("expected contextDestroyed before remove response: {remove_messages:#?}")
        });
    assert_eq!(destroyed["params"]["context"], json!(context_id));

    let listed_after_remove =
        send_bidi_command(&mut socket, 10, "browser.getUserContexts", json!({})).await;
    let listed_after_remove_ids = bidi_user_context_ids(&listed_after_remove);
    assert!(!listed_after_remove_ids.contains(&first_user_context));
    assert!(listed_after_remove_ids.contains(&second_user_context));
    assert!(listed_after_remove_ids.contains(&"default".to_owned()));

    let removed_context_create = send_bidi_command(
        &mut socket,
        11,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": first_user_context
        }),
    )
    .await;
    assert_bidi_error(
        &removed_context_create,
        "no such user context",
        "browsingContext.create should reject removed user context",
    );

    let remove_second = send_bidi_command(
        &mut socket,
        12,
        "browser.removeUserContext",
        json!({ "userContext": second_user_context }),
    )
    .await;
    assert_eq!(remove_second["type"], json!("success"));

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_browser_user_context_invalid_values_match_wpt() {
    // Ported from Chromium/WPT webdriver/tests/bidi/browser/
    // create_user_context/invalid.py and remove_user_context/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    let mut id = 2_u64;
    for params in [
        json!({"acceptInsecureCerts": "foo"}),
        json!({"proxy": false}),
        json!({"proxy": {}}),
        json!({"proxy": {"proxyType": "manual", "httpProxy": "http://foo"}}),
        json!({"proxy": {"proxyType": "manual", "socksProxy": "127.0.0.1:1080"}}),
        json!({"proxy": {"proxyType": "pac"}}),
        json!({"unhandledPromptBehavior": {"default": "invalid_value"}}),
    ] {
        let response =
            send_bidi_command(&mut socket, id, "browser.createUserContext", params.clone()).await;
        id += 1;
        assert_bidi_error(
            &response,
            "invalid argument",
            &format!("createUserContext params should be invalid: {params}"),
        );
    }

    let default_remove = send_bidi_command(
        &mut socket,
        id,
        "browser.removeUserContext",
        json!({ "userContext": "default" }),
    )
    .await;
    id += 1;
    assert_bidi_error(
        &default_remove,
        "invalid argument",
        "removeUserContext should reject default",
    );

    let unknown_remove = send_bidi_command(
        &mut socket,
        id,
        "browser.removeUserContext",
        json!({ "userContext": "missing-user-context" }),
    )
    .await;
    assert_bidi_error(
        &unknown_remove,
        "no such user context",
        "removeUserContext should reject unknown user contexts",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_browser_get_client_windows_matches_wpt_activation_state() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browser/get_client_windows/get_client_windows.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, initial_context_id) = bidi_session_with_context(cdp_addr).await;

    let initial = send_bidi_command(&mut socket, 3, "browser.getClientWindows", json!({})).await;
    assert_eq!(initial["type"], json!("success"));
    let initial_windows = initial["result"]["clientWindows"]
        .as_array()
        .expect("initial clientWindows array");
    assert_eq!(initial_windows.len(), 1);
    let initial_window_id = initial_windows[0]["clientWindow"]
        .as_str()
        .expect("initial clientWindow id")
        .to_owned();
    assert_eq!(initial_window_id, initial_context_id);
    assert_eq!(initial_windows[0]["active"], json!(true));

    let new_window = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.create",
        json!({ "type": "window" }),
    )
    .await;
    assert_eq!(new_window["type"], json!("success"));
    let new_context_id = new_window["result"]["context"]
        .as_str()
        .expect("new window context id")
        .to_owned();

    let updated = send_bidi_command(&mut socket, 5, "browser.getClientWindows", json!({})).await;
    assert_eq!(updated["type"], json!("success"));
    let updated_windows = updated["result"]["clientWindows"]
        .as_array()
        .expect("updated clientWindows array");
    assert_eq!(updated_windows.len(), 2);
    assert_ne!(
        updated_windows[0]["clientWindow"],
        updated_windows[1]["clientWindow"]
    );
    let first_window = updated_windows
        .iter()
        .find(|window| window["clientWindow"] == json!(initial_window_id))
        .expect("initial client window");
    let second_window = updated_windows
        .iter()
        .find(|window| window["clientWindow"] == json!(new_context_id))
        .expect("new client window");
    assert_eq!(first_window["active"], json!(false));
    assert_eq!(second_window["active"], json!(true));

    let activate_initial = send_bidi_command(
        &mut socket,
        6,
        "browsingContext.activate",
        json!({ "context": initial_context_id }),
    )
    .await;
    assert_eq!(activate_initial["type"], json!("success"));

    let activated = send_bidi_command(&mut socket, 7, "browser.getClientWindows", json!({})).await;
    assert_eq!(activated["type"], json!("success"));
    let activated_windows = activated["result"]["clientWindows"]
        .as_array()
        .expect("activated clientWindows array");
    let first_window = activated_windows
        .iter()
        .find(|window| window["clientWindow"] == json!(initial_window_id))
        .expect("activated initial client window");
    let second_window = activated_windows
        .iter()
        .find(|window| window["clientWindow"] == json!(new_context_id))
        .expect("activated new client window");
    assert_eq!(first_window["active"], json!(true));
    assert_eq!(second_window["active"], json!(false));

    let close = send_bidi_command(
        &mut socket,
        8,
        "browsingContext.close",
        json!({ "context": new_context_id }),
    )
    .await;
    assert_eq!(close["type"], json!("success"));

    let final_windows =
        send_bidi_command(&mut socket, 9, "browser.getClientWindows", json!({})).await;
    assert_eq!(final_windows["type"], json!("success"));
    assert_eq!(
        final_windows["result"]["clientWindows"],
        json!([{
            "clientWindow": initial_window_id,
            "active": true,
            "state": "normal",
            "width": 0,
            "height": 0,
            "x": 0,
            "y": 0
        }])
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_browser_set_client_window_state_updates_owner_info() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let minimized = send_bidi_command(
        &mut socket,
        3,
        "browser.setClientWindowState",
        json!({
            "clientWindow": context_id,
            "state": "minimized"
        }),
    )
    .await;
    assert_eq!(minimized["type"], json!("success"));
    assert_eq!(minimized["result"]["clientWindow"], json!(context_id));
    assert_eq!(minimized["result"]["state"], json!("minimized"));
    assert_eq!(minimized["result"]["active"], json!(true));

    let after_minimize =
        send_bidi_command(&mut socket, 4, "browser.getClientWindows", json!({})).await;
    assert_eq!(after_minimize["type"], json!("success"));
    assert_eq!(
        after_minimize["result"]["clientWindows"][0]["state"],
        json!("minimized")
    );

    let normal = send_bidi_command(
        &mut socket,
        5,
        "browser.setClientWindowState",
        json!({
            "clientWindow": context_id,
            "state": "normal",
            "width": 1024,
            "height": 768,
            "x": -12,
            "y": 34
        }),
    )
    .await;
    assert_eq!(normal["type"], json!("success"));
    assert_eq!(
        normal["result"],
        json!({
            "clientWindow": context_id,
            "active": true,
            "state": "normal",
            "width": 1024,
            "height": 768,
            "x": -12,
            "y": 34
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_browsing_context_create_background_preserves_focus_visibility() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/create/background.py.
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
    assert_eq!(
        bidi_focus_visibility_surface(&mut socket, 3, &foreground_context_id).await,
        json!({
            "hasFocus": true,
            "hidden": false,
            "visibilityState": "visible"
        })
    );

    let background = send_bidi_command(
        &mut socket,
        4,
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

    assert_eq!(
        bidi_focus_visibility_surface(&mut socket, 5, &foreground_context_id).await,
        json!({
            "hasFocus": true,
            "hidden": false,
            "visibilityState": "visible"
        }),
        "background=true should not activate the created context"
    );
    assert_eq!(
        bidi_focus_visibility_surface(&mut socket, 6, &background_context_id).await,
        json!({
            "hasFocus": false,
            "hidden": true,
            "visibilityState": "hidden"
        }),
        "background-created context should expose parked document surfaces"
    );

    let activated = send_bidi_command(
        &mut socket,
        7,
        "browsingContext.create",
        json!({
            "type": "tab",
            "background": false
        }),
    )
    .await;
    assert_eq!(activated["type"], json!("success"));
    let activated_context_id = activated["result"]["context"]
        .as_str()
        .expect("activated context id")
        .to_owned();

    assert_eq!(
        bidi_focus_visibility_surface(&mut socket, 8, &activated_context_id).await,
        json!({
            "hasFocus": true,
            "hidden": false,
            "visibilityState": "visible"
        }),
        "background=false should activate the created context"
    );
    assert_eq!(
        bidi_focus_visibility_surface(&mut socket, 9, &background_context_id).await,
        json!({
            "hasFocus": false,
            "hidden": true,
            "visibilityState": "hidden"
        }),
        "subsequent foreground creation should not promote the background context"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_subscribe_emits_context_destroyed_on_close() {
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
                    "events": ["browsingContext.contextDestroyed"],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "browsingContext.close",
                "params": {
                    "context": context_id.clone()
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.close");
    let messages = recv_until_id(&mut socket, 4).await;
    let close = messages
        .iter()
        .find(|message| message["id"] == json!(4_u64))
        .expect("close response");
    assert_eq!(close["type"], json!("success"));

    let event = messages
        .iter()
        .find(|message| message["method"] == json!("browsingContext.contextDestroyed"))
        .unwrap_or_else(|| {
            panic!("expected contextDestroyed before close response: {messages:#?}")
        });
    assert_eq!(event["type"], json!("event"));
    assert_eq!(event["params"]["context"], json!(context_id.clone()));
    assert_eq!(event["params"]["url"], json!("about:blank"));
    assert_eq!(event["params"]["children"], json!([]));
    assert_eq!(event["params"]["clientWindow"], json!(context_id));
    assert_eq!(event["params"]["originalOpener"], serde_json::Value::Null);
    assert_eq!(event["params"]["parent"], serde_json::Value::Null);
    assert!(
        event["params"]["userContext"].as_str().is_some(),
        "contextDestroyed should include userContext: {event:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

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
async fn websocket_bidi_session_subscribe_emits_main_document_network_events() {
    const MAIN_DOCUMENT_NETWORK_BODY: &str =
        "<!doctype html><html><body><main id=\"ready\">main-document-network</main></body></html>";

    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            MAIN_DOCUMENT_NETWORK_BODY,
        )
    }

    let fixture_app = Router::new().route("/", get(page));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi main document network event fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi main document network event fixture addr");
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
                    "events": [
                        "network.beforeRequestSent",
                        "network.responseStarted",
                        "network.responseCompleted"
                    ],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    let add_collector = send_bidi_command_response(
        &mut socket,
        4,
        "network.addDataCollector",
        json!({
            "dataTypes": ["response"],
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
                "id": 5_u64,
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
    let mut messages = recv_until_id(&mut socket, 5).await;
    if !messages
        .iter()
        .any(|message| message["method"] == json!("network.responseCompleted"))
    {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == json!("network.responseCompleted")
            })
            .await,
        );
    }

    let navigate = messages
        .iter()
        .find(|message| message["id"] == json!(5_u64))
        .expect("browsingContext.navigate response");
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(navigate["result"]["url"], json!(fixture_url));
    let navigation = navigate["result"]["navigation"].clone();
    assert!(
        navigation
            .as_str()
            .is_some_and(|id| id.starts_with("navigation-")),
        "navigate response should carry a WebDriver BiDi navigation id: {navigate:?}"
    );

    for method in [
        "network.beforeRequestSent",
        "network.responseStarted",
        "network.responseCompleted",
    ] {
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["method"] == json!(method))
                .count(),
            1,
            "{method} should be emitted once: {messages:?}"
        );
    }

    let before_request = messages
        .iter()
        .find(|message| message["method"] == json!("network.beforeRequestSent"))
        .expect("network.beforeRequestSent event");
    assert_eq!(before_request["type"], json!("event"));
    assert_eq!(before_request["params"]["context"], json!(context_id));
    assert_eq!(before_request["params"]["isBlocked"], json!(false));
    assert_eq!(before_request["params"]["navigation"], navigation);
    assert_eq!(before_request["params"]["redirectCount"], json!(0));
    assert_eq!(
        before_request["params"]["request"]["url"],
        json!(fixture_url)
    );
    assert_eq!(before_request["params"]["request"]["method"], json!("GET"));
    assert!(
        before_request["params"]["request"]["headers"].is_array(),
        "beforeRequestSent should carry BiDi request headers: {before_request:?}"
    );
    assert!(
        before_request["params"]["timestamp"].as_u64().is_some(),
        "beforeRequestSent should carry epoch milliseconds: {before_request:?}"
    );

    let response_started = messages
        .iter()
        .find(|message| message["method"] == json!("network.responseStarted"))
        .expect("network.responseStarted event");
    assert_eq!(response_started["type"], json!("event"));
    assert_eq!(response_started["params"]["context"], json!(context_id));
    assert_eq!(response_started["params"]["navigation"], navigation);
    assert_eq!(
        response_started["params"]["request"]["request"],
        before_request["params"]["request"]["request"]
    );
    assert_eq!(response_started["params"]["response"]["status"], json!(200));
    assert_eq!(
        response_started["params"]["response"]["url"],
        json!(fixture_url)
    );
    assert_eq!(
        response_started["params"]["response"]["mimeType"],
        json!("text/html")
    );
    assert_eq!(
        response_started["params"]["response"]["protocol"],
        json!("http/1.1")
    );

    let response_completed = messages
        .iter()
        .find(|message| message["method"] == json!("network.responseCompleted"))
        .expect("network.responseCompleted event");
    assert_eq!(response_completed["type"], json!("event"));
    assert_eq!(response_completed["params"]["context"], json!(context_id));
    assert_eq!(response_completed["params"]["navigation"], navigation);
    assert_eq!(
        response_completed["params"]["request"]["request"],
        before_request["params"]["request"]["request"]
    );
    assert_eq!(
        response_completed["params"]["response"]["status"],
        json!(200)
    );
    assert_eq!(
        response_completed["params"]["response"]["url"],
        json!(fixture_url)
    );
    assert_eq!(
        response_completed["params"]["response"]["protocol"],
        json!("http/1.1")
    );
    assert!(
        response_completed["params"]["response"]["bytesReceived"]
            .as_u64()
            .is_some(),
        "responseCompleted should carry bytesReceived: {response_completed:?}"
    );
    let request_id = response_completed["params"]["request"]["request"]
        .as_str()
        .expect("responseCompleted request id")
        .to_owned();

    let data = send_bidi_command_response(
        &mut socket,
        6,
        "network.getData",
        json!({
            "request": request_id,
            "dataType": "response"
        }),
    )
    .await;
    assert_eq!(data["type"], json!("success"));
    assert_eq!(
        data["result"]["bytes"],
        json!({
            "type": "string",
            "value": MAIN_DOCUMENT_NETWORK_BODY,
        })
    );

    let missing = send_bidi_command_response(
        &mut socket,
        7,
        "network.getData",
        json!({
            "request": "missing-request",
            "dataType": "response"
        }),
    )
    .await;
    assert_eq!(missing["type"], json!("error"));
    assert_eq!(missing["error"], json!("no such network data"));

    let request_body = send_bidi_command_response(
        &mut socket,
        8,
        "network.getData",
        json!({
            "request": request_id.clone(),
            "dataType": "request"
        }),
    )
    .await;
    assert_bidi_error(
        &request_body,
        "no such network data",
        "network.getData request data is not collected yet",
    );

    let missing_collector = send_bidi_command_response(
        &mut socket,
        9,
        "network.getData",
        json!({
            "request": request_id,
            "dataType": "response",
            "collector": "does-not-exist"
        }),
    )
    .await;
    assert_bidi_error(
        &missing_collector,
        "no such network collector",
        "network.getData should reject an unknown collector",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
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
async fn websocket_bidi_user_context_fetch_from_initial_about_blank_reports_cors_error() {
    async fn empty_text() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/plain")],
            "user-context fetch body",
        )
    }

    let fixture_app = Router::new().route("/empty.txt", get(empty_text));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi initial about:blank fetch fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi initial about:blank fetch fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
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

    let evaluate = send_bidi_command_response(
        &mut socket,
        5,
        "script.evaluate",
        json!({
            "expression": format!(
                "fetch({fetch_url:?}).then(response => response.text()).then(text => 'OK:' + text).catch(error => 'ERR:' + error.name + ':' + error.message)"
            ),
            "target": {
                "context": context_id
            },
            "awaitPromise": true
        }),
    )
    .await;
    assert_eq!(evaluate["type"], json!("success"), "evaluate: {evaluate:?}");
    assert!(
        evaluate["result"]["result"]["value"]
            .as_str()
            .is_some_and(|value| value.starts_with("ERR:TypeError:CORS check failed")),
        "initial about:blank cross-origin fetch should fail with CORS: {evaluate:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_subscribe_emits_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main id=\"ready\">network-events</main></body></html>",
        )
    }
    async fn data() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/plain")],
            "network body",
        )
    }

    let fixture_app = Router::new()
        .route("/", get(page))
        .route("/data", get(data));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi network event fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi network event fixture addr");
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
    let navigate = recv_ws_json(&mut socket).await;
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(navigate["result"]["url"], json!(fixture_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "session.subscribe",
                "params": {
                    "events": [
                        "network.beforeRequestSent",
                        "network.responseStarted",
                        "network.responseCompleted"
                    ],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    let fetch_url = format!("{fixture_url}data");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": format!("fetch({fetch_url:?}).then(response => response.text())"),
                    "target": {
                        "context": context_id.clone()
                    },
                    "awaitPromise": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate fetch");
    let mut messages = recv_until_id(&mut socket, 5).await;
    if !messages
        .iter()
        .any(|message| message["method"] == json!("network.responseCompleted"))
    {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == json!("network.responseCompleted")
            })
            .await,
        );
    }

    let evaluate = messages
        .iter()
        .find(|message| message["id"] == json!(5_u64))
        .expect("script.evaluate response");
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["result"]["result"]["value"], json!("network body"));

    let before_request = messages
        .iter()
        .find(|message| message["method"] == json!("network.beforeRequestSent"))
        .expect("network.beforeRequestSent event");
    assert_eq!(before_request["type"], json!("event"));
    assert_eq!(before_request["params"]["context"], json!(context_id));
    assert_eq!(before_request["params"]["isBlocked"], json!(false));
    assert_eq!(
        before_request["params"]["navigation"],
        serde_json::Value::Null
    );
    assert_eq!(before_request["params"]["request"]["url"], json!(fetch_url));
    assert_eq!(before_request["params"]["request"]["method"], json!("GET"));
    assert!(
        before_request["params"]["request"]["headers"].is_array(),
        "beforeRequestSent should carry BiDi request headers: {before_request:?}"
    );
    assert!(
        before_request["params"]["timestamp"].as_u64().is_some(),
        "beforeRequestSent should carry epoch milliseconds: {before_request:?}"
    );

    let response_started = messages
        .iter()
        .find(|message| message["method"] == json!("network.responseStarted"))
        .expect("network.responseStarted event");
    assert_eq!(response_started["type"], json!("event"));
    assert_eq!(response_started["params"]["context"], json!(context_id));
    assert_eq!(
        response_started["params"]["request"]["request"],
        before_request["params"]["request"]["request"]
    );
    assert_eq!(response_started["params"]["response"]["status"], json!(200));
    assert_eq!(
        response_started["params"]["response"]["url"],
        json!(fetch_url)
    );

    let response_completed = messages
        .iter()
        .find(|message| message["method"] == json!("network.responseCompleted"))
        .expect("network.responseCompleted event");
    assert_eq!(response_completed["type"], json!("event"));
    assert_eq!(response_completed["params"]["context"], json!(context_id));
    assert_eq!(
        response_completed["params"]["request"]["request"],
        before_request["params"]["request"]["request"]
    );
    assert_eq!(
        response_completed["params"]["response"]["status"],
        json!(200)
    );
    assert!(
        response_completed["params"]["response"]["bytesReceived"]
            .as_u64()
            .is_some(),
        "responseCompleted should carry bytesReceived: {response_completed:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_unsubscribe_network_stops_network_events() {
    async fn page() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main id=\"ready\">network-unsubscribe</main></body></html>",
        )
    }
    async fn data() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/plain")],
            "network after unsubscribe",
        )
    }

    let fixture_app = Router::new()
        .route("/", get(page))
        .route("/data", get(data));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi network unsubscribe fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi network unsubscribe fixture addr");
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
        json!({ "type": "tab" }),
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
            "events": ["network.beforeRequestSent"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let subscription_id = subscribe["result"]["subscription"]
        .as_str()
        .expect("network subscription id")
        .to_owned();

    let unsubscribe = send_bidi_command(
        &mut socket,
        5,
        "session.unsubscribe",
        json!({
            "subscriptions": [subscription_id]
        }),
    )
    .await;
    assert_eq!(unsubscribe["type"], json!("success"));

    let fetch_url = format!("{fixture_url}data");
    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": format!("fetch({fetch_url:?}).then(response => response.text())"),
                    "target": {
                        "context": context_id.clone()
                    },
                    "awaitPromise": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate fetch after network unsubscribe");
    let messages = recv_until_id(&mut socket, 6).await;
    let evaluate = bidi_message_by_id(&messages, 6);
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(
        evaluate["result"]["result"]["value"],
        json!("network after unsubscribe")
    );
    assert!(
        messages.iter().all(|message| !message["method"]
            .as_str()
            .is_some_and(|method| method.starts_with("network."))),
        "network events should not be emitted after unsubscribe: {messages:#?}"
    );

    let no_late_event = timeout(Duration::from_millis(300), recv_ws_json(&mut socket)).await;
    match no_late_event {
        Err(_) => {}
        Ok(message) => panic!("unexpected event after network unsubscribe: {message:#?}"),
    }

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
async fn websocket_bidi_session_subscribe_emits_log_entry_added() {
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
                    "url": "data:text/html,<body>log-events</body>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate before log subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["log.entryAdded"],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('bidi', 'log'); 'done'",
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
        .expect("send script.evaluate");
    let messages = recv_until_id(&mut socket, 5).await;
    let evaluate = messages
        .last()
        .expect("script.evaluate response should be last message");
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["id"], json!(5_u64));
    assert_eq!(evaluate["result"]["result"]["value"], json!("done"));

    let log_event = match messages
        .iter()
        .find(|message| message["method"] == json!("log.entryAdded"))
    {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
        })
        .await
        .pop()
        .expect("expected log.entryAdded after response"),
    };
    assert_eq!(log_event["type"], json!("event"));
    assert_eq!(log_event["params"]["type"], json!("console"));
    assert_eq!(log_event["params"]["method"], json!("log"));
    assert_eq!(log_event["params"]["level"], json!("info"));
    assert_eq!(log_event["params"]["text"], json!("bidi log"));
    assert_eq!(log_event["params"]["source"]["context"], json!(context_id));
    assert!(
        log_event["params"]["source"]["realm"].as_str().is_some(),
        "log event should carry a realm id: {log_event:?}"
    );
    assert!(
        log_event["params"]["timestamp"].as_u64().is_some(),
        "timestamp should be epoch milliseconds: {log_event:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_subscribe_google_channel_routes_log_events() {
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
            "context": context_id.clone(),
            "url": "data:text/html,<body>channel-log</body>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let subscribe = send_bidi_command_with_channel(
        &mut socket,
        4,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "contexts": [context_id.clone()]
        }),
        "alpha",
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    assert_eq!(subscribe["goog:channel"], json!("alpha"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('channel', 'event'); 'done'",
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
        .expect("send script.evaluate");
    let messages = recv_until_id(&mut socket, 5).await;
    let evaluate = bidi_message_by_id(&messages, 5);
    assert_eq!(evaluate["type"], json!("success"));

    let log_event = match messages
        .iter()
        .find(|message| message["method"] == json!("log.entryAdded"))
    {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
        })
        .await
        .pop()
        .expect("expected channel log.entryAdded"),
    };
    assert_eq!(log_event["type"], json!("event"));
    assert_eq!(log_event["goog:channel"], json!("alpha"));
    assert_eq!(log_event["params"]["text"], json!("channel event"));
    assert_eq!(log_event["params"]["source"]["context"], json!(context_id));

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_subscribe_global_then_context_keeps_log_source_enabled() {
    // Mirrors Chromium/WPT
    // webdriver/tests/bidi/session/subscribe/contexts.py::test_subscribe_to_all_context_and_then_to_one_again.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, top_context_id) = bidi_session_with_context(cdp_addr).await;

    let new_tab = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({ "type": "tab" }),
    )
    .await;
    assert_eq!(new_tab["type"], json!("success"));

    let subscribe_all = send_bidi_command(
        &mut socket,
        4,
        "session.subscribe",
        json!({ "events": ["log.entryAdded"] }),
    )
    .await;
    assert_eq!(subscribe_all["type"], json!("success"));

    let subscribe_top_context = send_bidi_command(
        &mut socket,
        5,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "contexts": [top_context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe_top_context["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('global then context'); 'done'",
                    "target": {
                        "context": top_context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate after repeated subscribe");
    let initial_messages = recv_until_id(&mut socket, 6).await;
    let messages = collect_bidi_messages_until_method_count(
        &mut socket,
        initial_messages,
        "log.entryAdded",
        1,
    )
    .await;
    let evaluate = bidi_message_by_id(&messages, 6);
    assert_eq!(evaluate["type"], json!("success"));
    let log_events = bidi_events_by_method(&messages, "log.entryAdded");
    assert_eq!(log_events.len(), 1, "expected one log event: {messages:#?}");
    assert_eq!(
        log_events[0]["params"]["source"]["context"],
        json!(top_context_id),
        "repeated subscription must retain the evaluated context source: {messages:#?}"
    );
    assert_eq!(
        log_events[0]["params"]["text"],
        json!("global then context")
    );

    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_global_log_subscription_covers_later_created_context() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, initial_context_id) = bidi_session_with_context(cdp_addr).await;

    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let late_context = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.create",
        json!({ "type": "tab" }),
    )
    .await;
    assert_eq!(late_context["type"], json!("success"));
    let late_context_id = late_context["result"]["context"]
        .as_str()
        .expect("late context id")
        .to_owned();
    assert_ne!(late_context_id, initial_context_id);

    let navigate = send_bidi_command(
        &mut socket,
        5,
        "browsingContext.navigate",
        json!({
            "context": late_context_id.clone(),
            "url": "data:text/html,<body>late-global-log</body>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('late global log'); 'done'",
                    "target": {
                        "context": late_context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate in late context");
    let messages = recv_until_id(&mut socket, 6).await;
    let evaluate = bidi_message_by_id(&messages, 6);
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["result"]["result"]["value"], json!("done"));

    let log_event = match messages.iter().find(|message| {
        message["method"] == json!("log.entryAdded")
            && message["params"]["text"] == json!("late global log")
    }) {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
                && message["params"]["text"] == json!("late global log")
        })
        .await
        .pop()
        .unwrap_or_else(|| {
            panic!("expected global log subscription to cover late context: {messages:#?}")
        }),
    };
    assert_eq!(log_event["type"], json!("event"));
    assert_eq!(
        log_event["params"]["source"]["context"],
        json!(late_context_id)
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_subscribe_emits_javascript_log_entry_added() {
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
                    "url": "data:text/html,<body>javascript-log-events</body>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate before log subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["log.entryAdded"],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "setTimeout(() => { throw new Error('bidi exception') }, 0); 'scheduled'",
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
        .expect("send script.evaluate");
    let messages = recv_until_id(&mut socket, 5).await;
    let evaluate = messages
        .last()
        .expect("script.evaluate response should be last message");
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["id"], json!(5_u64));
    assert_eq!(evaluate["result"]["result"]["value"], json!("scheduled"));

    let log_event = match messages
        .iter()
        .find(|message| message["method"] == json!("log.entryAdded"))
    {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
        })
        .await
        .pop()
        .expect("expected javascript log.entryAdded after response"),
    };
    assert_eq!(log_event["type"], json!("event"));
    assert_eq!(log_event["params"]["type"], json!("javascript"));
    assert_eq!(log_event["params"]["level"], json!("error"));
    assert!(
        log_event["params"]["text"]
            .as_str()
            .is_some_and(|text| text.contains("bidi exception")),
        "javascript log entry should include exception text: {log_event:?}"
    );
    assert_eq!(log_event["params"]["source"]["context"], json!(context_id));
    assert!(
        log_event["params"]["source"]["realm"].as_str().is_some(),
        "javascript log event should carry a realm id: {log_event:?}"
    );
    assert!(
        log_event["params"]["timestamp"].as_u64().is_some(),
        "timestamp should be epoch milliseconds: {log_event:?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_subscribe_replays_buffered_log_entry_added() {
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
                    "url": "data:text/html,<body>buffered-log-events</body>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate before buffered log subscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["log.entryAdded"],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send initial session.subscribe");
    let initial_subscribe = recv_ws_json(&mut socket).await;
    assert_eq!(initial_subscribe["type"], json!("success"));
    let initial_subscription_id = initial_subscribe["result"]["subscription"]
        .as_str()
        .expect("initial log subscription id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "session.unsubscribe",
                "params": {
                    "subscriptions": [initial_subscription_id]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.unsubscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.warn('cached-log'); 'done'",
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
        .expect("send script.evaluate while unsubscribed");
    let evaluate_messages = recv_until_id(&mut socket, 6).await;
    assert!(
        evaluate_messages
            .iter()
            .all(|message| message["method"] != json!("log.entryAdded")),
        "unsubscribed log event should be buffered, not sent: {evaluate_messages:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["log.entryAdded"],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send replay session.subscribe");
    let subscribe_messages = recv_until_id(&mut socket, 7).await;
    let replay_subscribe = subscribe_messages
        .last()
        .expect("replay subscribe response");
    let replay_subscription_id = replay_subscribe["result"]["subscription"]
        .as_str()
        .expect("replay log subscription id")
        .to_owned();
    let buffered = subscribe_messages
        .iter()
        .find(|message| message["method"] == json!("log.entryAdded"))
        .unwrap_or_else(|| {
            panic!("expected buffered log.entryAdded before subscribe response: {subscribe_messages:#?}")
        });
    assert_eq!(buffered["params"]["type"], json!("console"));
    assert_eq!(buffered["params"]["method"], json!("warn"));
    assert_eq!(buffered["params"]["level"], json!("warn"));
    assert_eq!(buffered["params"]["text"], json!("cached-log"));
    assert_eq!(buffered["params"]["source"]["context"], json!(context_id));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "session.unsubscribe",
                "params": {
                    "subscriptions": [replay_subscription_id]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send final session.unsubscribe");
    assert_eq!(recv_ws_json(&mut socket).await["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["log.entryAdded"],
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send second replay session.subscribe");
    let second_subscribe = recv_until_id(&mut socket, 9).await;
    assert!(
        second_subscribe
            .iter()
            .all(|message| message["method"] != json!("log.entryAdded")),
        "buffered log event should only replay once: {second_subscribe:#?}"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_subscribe_filters_log_events_by_user_context() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/session/subscribe/user_contexts.py::test_subscribe_one_user_context.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    let user_context =
        send_bidi_command(&mut socket, 2, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context id")
        .to_owned();

    let default_context = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": "default"
        }),
    )
    .await;
    assert_eq!(default_context["type"], json!("success"));
    let default_context_id = default_context["result"]["context"]
        .as_str()
        .expect("created default context id")
        .to_owned();

    let user_context_tab = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(user_context_tab["type"], json!("success"));
    let user_context_tab_id = user_context_tab["result"]["context"]
        .as_str()
        .expect("created user context tab id")
        .to_owned();

    let subscribe = send_bidi_command(
        &mut socket,
        5,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "userContexts": [user_context_id]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('default-user-context-log'); 'done'",
                    "target": {
                        "context": default_context_id
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send default user context script.evaluate");
    let default_messages = recv_until_id(&mut socket, 6).await;
    assert_eq!(
        default_messages.last().expect("default evaluate response")["type"],
        json!("success")
    );
    assert!(
        default_messages
            .iter()
            .all(|message| message["method"] != json!("log.entryAdded")),
        "default user context log should not match custom userContext subscription: {default_messages:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('custom-user-context-log'); 'done'",
                    "target": {
                        "context": user_context_tab_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send custom user context script.evaluate");
    let user_context_messages = recv_until_id(&mut socket, 7).await;
    assert_eq!(
        user_context_messages
            .last()
            .expect("user context evaluate response")["type"],
        json!("success")
    );
    let log_event = match user_context_messages
        .iter()
        .find(|message| message["method"] == json!("log.entryAdded"))
    {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
        })
        .await
        .pop()
        .expect("expected custom userContext log.entryAdded after response"),
    };
    assert_eq!(log_event["params"]["type"], json!("console"));
    assert_eq!(log_event["params"]["method"], json!("log"));
    assert_eq!(
        log_event["params"]["text"],
        json!("custom-user-context-log")
    );
    assert_eq!(
        log_event["params"]["source"]["context"],
        json!(user_context_tab_id)
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_subscribe_filters_log_events_by_default_and_multiple_user_contexts()
{
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/session/subscribe/user_contexts.py::test_subscribe_default_user_context
    // and test_subscribe_multiple_user_contexts.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));

    let user_context =
        send_bidi_command(&mut socket, 2, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context id")
        .to_owned();

    let default_context = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": "default"
        }),
    )
    .await;
    assert_eq!(default_context["type"], json!("success"));
    let default_context_id = default_context["result"]["context"]
        .as_str()
        .expect("created default context id")
        .to_owned();

    let user_context_tab = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(user_context_tab["type"], json!("success"));
    let user_context_tab_id = user_context_tab["result"]["context"]
        .as_str()
        .expect("created user context tab id")
        .to_owned();

    let subscribe_default = send_bidi_command(
        &mut socket,
        5,
        "session.subscribe",
        json!({
            "events": ["log.entryAdded"],
            "userContexts": ["default"]
        }),
    )
    .await;
    assert_eq!(subscribe_default["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('default-only-log'); 'done'",
                    "target": {
                        "context": default_context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send default userContext script.evaluate");
    let default_messages = recv_until_id(&mut socket, 6).await;
    assert_eq!(
        default_messages.last().expect("default evaluate response")["type"],
        json!("success")
    );
    let default_log = match default_messages
        .iter()
        .find(|message| message["method"] == json!("log.entryAdded"))
    {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
        })
        .await
        .pop()
        .unwrap_or_else(|| {
            panic!("expected default userContext log.entryAdded: {default_messages:#?}")
        }),
    };
    assert_eq!(default_log["params"]["text"], json!("default-only-log"));
    assert_eq!(
        default_log["params"]["source"]["context"],
        json!(default_context_id)
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('custom-ignored-log'); 'done'",
                    "target": {
                        "context": user_context_tab_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send custom userContext script.evaluate");
    let custom_ignored_messages = recv_until_id(&mut socket, 7).await;
    assert_eq!(
        custom_ignored_messages
            .last()
            .expect("custom ignored evaluate response")["type"],
        json!("success")
    );
    assert!(
        custom_ignored_messages
            .iter()
            .all(|message| message["method"] != json!("log.entryAdded")),
        "custom userContext log should not match default-only subscription: {custom_ignored_messages:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "session.subscribe",
                "params": {
                    "events": ["log.entryAdded"],
                    "userContexts": [user_context_id, "default"]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send multiple userContext session.subscribe");
    let subscribe_multiple_messages = recv_until_id(&mut socket, 8).await;
    let subscribe_multiple = subscribe_multiple_messages
        .last()
        .expect("multiple subscribe response");
    assert_eq!(subscribe_multiple["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('default-multiple-log'); 'done'",
                    "target": {
                        "context": default_context_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send default userContext script.evaluate after multi-subscribe");
    let default_multiple_messages = recv_until_id(&mut socket, 9).await;
    let default_multiple_log = match default_multiple_messages.iter().find(|message| {
        message["method"] == json!("log.entryAdded")
            && message["params"]["text"] == json!("default-multiple-log")
            && message["params"]["source"]["context"] == json!(default_context_id)
    }) {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
                && message["params"]["text"] == json!("default-multiple-log")
                && message["params"]["source"]["context"] == json!(default_context_id)
        })
        .await
        .pop()
        .unwrap_or_else(|| {
            panic!("multiple userContext subscription should include default context log: {default_multiple_messages:#?}")
        }),
    };
    assert_eq!(
        default_multiple_log["params"]["source"]["context"],
        json!(default_context_id)
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 10_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "console.log('custom-multiple-log'); 'done'",
                    "target": {
                        "context": user_context_tab_id.clone()
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send custom userContext script.evaluate after multi-subscribe");
    let custom_multiple_messages = recv_until_id(&mut socket, 10).await;
    let custom_multiple_log = match custom_multiple_messages.iter().find(|message| {
        message["method"] == json!("log.entryAdded")
            && message["params"]["text"] == json!("custom-multiple-log")
            && message["params"]["source"]["context"] == json!(user_context_tab_id)
    }) {
        Some(event) => (*event).clone(),
        None => recv_until_match(&mut socket, |message| {
            message["method"] == json!("log.entryAdded")
                && message["params"]["text"] == json!("custom-multiple-log")
                && message["params"]["source"]["context"] == json!(user_context_tab_id)
        })
        .await
        .pop()
        .unwrap_or_else(|| {
            panic!("multiple userContext subscription should include custom context log: {custom_multiple_messages:#?}")
        }),
    };
    assert_eq!(
        custom_multiple_log["params"]["source"]["context"],
        json!(user_context_tab_id)
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_unsubscribe_browsing_context_module_stops_navigation_events() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/session/unsubscribe/events.py::test_unsubscribe_from_module.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["browsingContext"]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let unsubscribe = send_bidi_command(
        &mut socket,
        4,
        "session.unsubscribe",
        json!({
            "events": ["browsingContext"]
        }),
    )
    .await;
    assert_eq!(unsubscribe["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id,
                    "url": "data:text/html,<main>unsubscribed</main>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate after unsubscribe");
    let navigate_messages = recv_until_id(&mut socket, 5).await;
    assert_eq!(
        navigate_messages.last().expect("navigate response")["type"],
        json!("success")
    );
    assert!(
        navigate_messages.iter().all(|message| {
            !matches!(
                message["method"].as_str(),
                Some("browsingContext.domContentLoaded" | "browsingContext.load")
            )
        }),
        "browsingContext module events should be unsubscribed before navigate response: {navigate_messages:#?}"
    );

    let no_late_event = timeout(Duration::from_millis(300), recv_ws_json(&mut socket)).await;
    match no_late_event {
        Err(_) => {}
        Ok(message) => panic!("unexpected BiDi event after module unsubscribe: {message:#?}"),
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_get_tree_root_includes_iframe_children_and_max_depth() {
    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>child-frame</main></body></html>",
        )
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi getTree frame fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi getTree frame fixture addr");
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
    let navigate = recv_ws_json(&mut socket).await;
    assert_eq!(navigate["type"], json!("success"));

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
    let contexts = tree["result"]["contexts"]
        .as_array()
        .expect("contexts array");
    assert_eq!(contexts.len(), 1);
    let root = &contexts[0];
    assert_eq!(root["context"], json!(context_id));
    assert_eq!(root["url"], json!(fixture_url));
    assert_eq!(root["clientWindow"], json!(context_id));
    assert_eq!(root["parent"], serde_json::Value::Null);
    let children = root["children"].as_array().expect("iframe children");
    assert_eq!(children.len(), 1, "getTree should expose iframe: {tree:?}");
    let child = &children[0];
    assert_ne!(child["context"], json!(context_id));
    assert_eq!(child["url"], json!(child_url));
    assert_eq!(child["clientWindow"], json!(context_id));
    assert_eq!(child["children"], json!([]));
    assert!(
        child.get("parent").is_none(),
        "inline getTree children should omit parent: {child:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.getTree",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.getTree without root");
    let all_trees = recv_ws_json(&mut socket).await;
    assert_eq!(all_trees["type"], json!("success"));
    let all_contexts = all_trees["result"]["contexts"]
        .as_array()
        .expect("all contexts array");
    let no_root = all_contexts
        .iter()
        .find(|context| context["context"] == json!(context_id.clone()))
        .expect("created context should appear in no-root getTree");
    let no_root_children = no_root["children"]
        .as_array()
        .expect("no-root iframe children");
    assert_eq!(
        no_root_children.len(),
        1,
        "no-root getTree should expose iframe: {all_trees:?}"
    );
    assert_eq!(no_root_children[0]["url"], json!(child_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "browsingContext.getTree",
                "params": {
                    "root": context_id,
                    "maxDepth": 0
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.getTree maxDepth 0");
    let depth_zero = recv_ws_json(&mut socket).await;
    assert_eq!(depth_zero["type"], json!("success"));
    assert_eq!(
        depth_zero["result"]["contexts"][0]["children"],
        serde_json::Value::Null
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_bidi_bound_devtools_command_executes_target_commands() {
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
    assert_eq!(create["id"], json!(2_u64));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();
    assert!(
        context_id.starts_with("TID-"),
        "created context should come from the DevTools target owner"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
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
    assert_eq!(tree["id"], json!(3_u64));
    assert_eq!(
        tree["result"]["contexts"],
        json!([{
            "context": context_id.clone(),
            "url": "about:blank",
            "children": [],
            "clientWindow": context_id.clone(),
            "originalOpener": null,
            "parent": null,
            "userContext": "default"
        }])
    );

    let navigate_url = "data:text/html,bidi-nav";
    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
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
    assert_eq!(navigate["id"], json!(4_u64));
    let navigation_id = navigate["result"]["navigation"]
        .as_str()
        .unwrap_or_else(|| panic!("navigate should return a navigation id: {navigate:?}"))
        .to_owned();
    assert_eq!(navigate["result"]["url"], json!(navigate_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.reload",
                "params": {
                    "context": context_id.clone(),
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.reload");
    let reload = recv_ws_json(&mut socket).await;
    assert_eq!(reload["type"], json!("success"));
    assert_eq!(reload["id"], json!(5_u64));
    let reload_navigation_id = reload["result"]["navigation"]
        .as_str()
        .unwrap_or_else(|| panic!("reload should return a navigation id: {reload:?}"));
    assert_ne!(
        reload_navigation_id, navigation_id,
        "reload should report a fresh navigation id"
    );
    assert_eq!(reload["result"]["url"], json!(navigate_url));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.getRealms",
                "params": {
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
    assert_eq!(realms["id"], json!(6_u64));
    let realms_list = realms["result"]["realms"]
        .as_array()
        .expect("script.getRealms result should include realms");
    let realm_id = realms_list
        .iter()
        .find_map(|realm| {
            let matches_realm = realm["context"] == json!(context_id)
                && realm["type"] == json!("window")
                && realm["origin"].as_str().is_some()
                && realm["realm"]
                    .as_str()
                    .is_some_and(|realm| !realm.is_empty());
            matches_realm.then(|| {
                realm["realm"]
                    .as_str()
                    .expect("non-empty realm id")
                    .to_owned()
            })
        })
        .expect("script.getRealms should expose the target window realm");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "1 + 2",
                    "target": {
                        "realm": realm_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate");
    let evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(evaluate["type"], json!("success"));
    assert_eq!(evaluate["id"], json!(7_u64));
    assert_eq!(evaluate["result"]["type"], json!("success"));
    assert_eq!(evaluate["result"]["result"]["type"], json!("number"));
    assert_eq!(evaluate["result"]["result"]["value"], json!(3));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "({value: 8})",
                    "target": {
                        "realm": realm_id.clone()
                    },
                    "resultOwnership": "root"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send root-owned script.evaluate");
    let owned_evaluate = recv_ws_json(&mut socket).await;
    assert_eq!(owned_evaluate["type"], json!("success"));
    assert_eq!(owned_evaluate["id"], json!(8_u64));
    let handle = owned_evaluate["result"]["result"]["handle"]
        .as_str()
        .expect("root-owned object should return a handle")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "(arg) => arg.value",
                    "arguments": [
                        {
                            "handle": handle.clone()
                        }
                    ],
                    "target": {
                        "realm": realm_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send handle script.callFunction");
    let handle_call = recv_ws_json(&mut socket).await;
    assert_eq!(handle_call["type"], json!("success"));
    assert_eq!(handle_call["id"], json!(9_u64));
    assert_eq!(handle_call["result"]["type"], json!("success"));
    assert_eq!(handle_call["result"]["result"]["type"], json!("number"));
    assert_eq!(handle_call["result"]["result"]["value"], json!(8));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 10_u64,
                "method": "script.disown",
                "params": {
                    "handles": ["unknown_handle"],
                    "target": {
                        "realm": realm_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send unknown script.disown");
    let unknown_disown = recv_ws_json(&mut socket).await;
    assert_eq!(unknown_disown["type"], json!("success"));
    assert_eq!(unknown_disown["id"], json!(10_u64));
    assert_eq!(unknown_disown["result"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 11_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "(arg) => arg.value",
                    "arguments": [
                        {
                            "handle": handle.clone()
                        }
                    ],
                    "target": {
                        "realm": realm_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send handle script.callFunction after unknown disown");
    let handle_call_after_unknown = recv_ws_json(&mut socket).await;
    assert_eq!(handle_call_after_unknown["type"], json!("success"));
    assert_eq!(handle_call_after_unknown["id"], json!(11_u64));
    assert_eq!(
        handle_call_after_unknown["result"]["result"]["value"],
        json!(8)
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 12_u64,
                "method": "script.disown",
                "params": {
                    "handles": ["unknown_handle", handle.clone()],
                    "target": {
                        "realm": realm_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.disown");
    let disown = recv_ws_json(&mut socket).await;
    assert_eq!(disown["type"], json!("success"));
    assert_eq!(disown["id"], json!(12_u64));
    assert_eq!(disown["result"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 13_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "(arg) => arg.value",
                    "arguments": [
                        {
                            "handle": handle
                        }
                    ],
                    "target": {
                        "realm": realm_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send released handle script.callFunction");
    let released_handle_call = recv_ws_json(&mut socket).await;
    assert_eq!(released_handle_call["type"], json!("error"));
    assert_eq!(released_handle_call["id"], json!(13_u64));
    assert_eq!(released_handle_call["error"], json!("no such handle"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 14_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "(value) => value + 1",
                    "arguments": [
                        {
                            "type": "number",
                            "value": 4
                        }
                    ],
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.callFunction");
    let call_function = recv_ws_json(&mut socket).await;
    assert_eq!(call_function["type"], json!("success"));
    assert_eq!(call_function["id"], json!(14_u64));
    assert_eq!(call_function["result"]["type"], json!("success"));
    assert_eq!(call_function["result"]["result"]["type"], json!("number"));
    assert_eq!(call_function["result"]["result"]["value"], json!(5));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 15_u64,
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
    assert_eq!(second_create["id"], json!(15_u64));
    let second_context_id = second_create["result"]["context"]
        .as_str()
        .expect("second created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 16_u64,
                "method": "browsingContext.activate",
                "params": {
                    "context": second_context_id.clone()
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.activate");
    let activate = recv_ws_json(&mut socket).await;
    assert_eq!(activate["type"], json!("success"));
    assert_eq!(activate["id"], json!(16_u64));
    assert_eq!(activate["result"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 17_u64,
                "method": "browsingContext.close",
                "params": {
                    "context": second_context_id.clone()
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.close");
    let close = recv_ws_json(&mut socket).await;
    assert_eq!(close["type"], json!("success"));
    assert_eq!(close["id"], json!(17_u64));
    assert_eq!(close["result"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 18_u64,
                "method": "browsingContext.getTree",
                "params": {
                    "root": second_context_id.clone()
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send closed-context browsingContext.getTree");
    let closed_tree = recv_ws_json(&mut socket).await;
    assert_eq!(closed_tree["type"], json!("error"));
    assert_eq!(closed_tree["id"], json!(18_u64));
    assert_eq!(closed_tree["error"], json!("no such frame"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 19_u64,
                "method": "browsingContext.setViewport",
                "params": {
                    "context": context_id.clone(),
                    "viewport": {
                        "width": 40,
                        "height": 30
                    },
                    "devicePixelRatio": 2.0
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.setViewport");
    let viewport = recv_ws_json(&mut socket).await;
    assert_eq!(viewport["type"], json!("success"));
    assert_eq!(viewport["id"], json!(19_u64));
    assert_eq!(viewport["result"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 20_u64,
                "method": "browsingContext.captureScreenshot",
                "params": {
                    "context": context_id.clone(),
                    "format": {
                        "type": "image/png"
                    },
                    "clip": {
                        "type": "box",
                        "x": 0,
                        "y": 0,
                        "width": 20,
                        "height": 10
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.captureScreenshot");
    let screenshot = recv_ws_json(&mut socket).await;
    assert_eq!(screenshot["type"], json!("error"));
    assert_eq!(screenshot["id"], json!(20_u64));
    assert_eq!(screenshot["error"], json!("unsupported operation"));
    assert_eq!(
        screenshot["message"],
        json!("Page.captureScreenshot is not supported: renderer screenshots are not implemented.")
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 21_u64,
                "method": "browsingContext.print",
                "params": {
                    "context": context_id,
                    "orientation": "portrait",
                    "page": {
                        "width": 21.59,
                        "height": 27.94
                    },
                    "margin": {
                        "top": 1.0,
                        "bottom": 1.0,
                        "left": 1.0,
                        "right": 1.0
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.print");
    let print = recv_ws_json(&mut socket).await;
    assert_eq!(print["type"], json!("error"));
    assert_eq!(print["id"], json!(21_u64));
    assert_eq!(print["error"], json!("unsupported operation"));
    assert_eq!(
        print["message"],
        json!("Page.printToPDF is not supported: PDF generation is not implemented.")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_script_message_channel_matches_wpt_shape() {
    // Mirrors webdriver/tests/bidi/script/call_function/channel.py::test_channel
    // for the default channel ownership/serialization case.
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

    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": "(channel) => channel({'foo': 'bar', 'baz': {'1': 2}})",
                    "arguments": [{
                        "type": "channel",
                        "value": {
                            "channel": "channel_name"
                        }
                    }],
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
        .expect("send channel script.callFunction");
    let mut messages = recv_until_id(&mut socket, 4).await;
    if !messages
        .iter()
        .any(|message| message["method"] == json!("script.message"))
    {
        messages.push(
            timeout(Duration::from_secs(1), recv_ws_json(&mut socket))
                .await
                .expect("script.message should arrive after callFunction response"),
        );
    }
    let call_function = messages
        .iter()
        .find(|message| message["id"] == json!(4_u64))
        .unwrap_or_else(|| panic!("expected callFunction response: {messages:#?}"));
    assert_eq!(call_function["type"], json!("success"));
    assert_eq!(call_function["result"]["type"], json!("success"));
    let realm_id = call_function["result"]["realm"]
        .as_str()
        .expect("callFunction response realm");

    let script_message = messages
        .iter()
        .find(|message| message["method"] == json!("script.message"))
        .unwrap_or_else(|| panic!("expected script.message event: {messages:#?}"));
    assert_eq!(
        script_message["params"],
        json!({
            "channel": "channel_name",
            "data": {
                "type": "object",
                "value": [
                    ["foo", {"type": "string", "value": "bar"}],
                    [
                        "baz",
                        {
                            "type": "object",
                            "value": [["1", {"type": "number", "value": 2}]]
                        }
                    ]
                ]
            },
            "source": {
                "realm": realm_id,
                "context": context_id
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_script_message_channel_observes_payload_mutation_before_serialization() {
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        4,
        &context_id,
        "(channel) => {
            const payload = { foo: 'before', nested: { value: 1 }, list: ['a'] };
            channel(payload);
            payload.foo = 'after';
            payload.nested.value = 2;
            payload.list.push('b');
        }",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "mutation_channel"
            }
        })],
        1,
    )
    .await;
    let response = bidi_message_by_id(&messages, 4);
    assert_eq!(response["type"], json!("success"));
    let realm = response["result"]["realm"]
        .as_str()
        .expect("callFunction response realm");
    let event = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("script.message event");
    assert_eq!(
        event["params"],
        json!({
            "channel": "mutation_channel",
            "data": {
                "type": "object",
                "value": [
                    ["foo", {"type": "string", "value": "after"}],
                    [
                        "nested",
                        {
                            "type": "object",
                            "value": [["value", {"type": "number", "value": 2}]]
                        }
                    ],
                    [
                        "list",
                        {
                            "type": "array",
                            "value": [
                                {"type": "string", "value": "a"},
                                {"type": "string", "value": "b"}
                            ]
                        }
                    ]
                ]
            },
            "source": {
                "realm": realm,
                "context": context_id
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_script_message_channel_ignores_spoofed_to_string_tag() {
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        4,
        &context_id,
        "(channel) => {
            const payload = { foo: 'bar' };
            Object.defineProperty(payload, Symbol.toStringTag, { value: 'Map' });
            channel(payload);
            return 'sent';
        }",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "tag_spoof_channel"
            }
        })],
        1,
    )
    .await;
    let response = bidi_message_by_id(&messages, 4);
    assert_eq!(response["type"], json!("success"));
    assert_eq!(
        response["result"]["result"],
        json!({"type": "string", "value": "sent"})
    );
    let realm = response["result"]["realm"]
        .as_str()
        .expect("callFunction response realm");
    let event = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("script.message event");
    assert_eq!(
        event["params"],
        json!({
            "channel": "tag_spoof_channel",
            "data": {
                "type": "object",
                "value": [
                    ["foo", {"type": "string", "value": "bar"}]
                ]
            },
            "source": {
                "realm": realm,
                "context": context_id
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_script_message_channel_variants_match_wpt_shape() {
    // Mirrors the remaining Chromium/WPT
    // webdriver/tests/bidi/script/call_function/channel.py channel cases.
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let shallow_messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        4,
        &context_id,
        "(channel) => channel({'foo': 'bar', 'baz': {'1': 2}})",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name",
                "serializationOptions": {
                    "maxObjectDepth": 0
                }
            }
        })],
        1,
    )
    .await;
    let shallow_response = bidi_message_by_id(&shallow_messages, 4);
    let shallow_realm = shallow_response["result"]["realm"]
        .as_str()
        .expect("shallow callFunction response realm");
    let shallow_event = bidi_events_by_method(&shallow_messages, "script.message")
        .pop()
        .expect("shallow script.message event");
    assert_eq!(
        shallow_event["params"],
        json!({
            "channel": "channel_name",
            "data": {
                "type": "object"
            },
            "source": {
                "realm": shallow_realm,
                "context": context_id
            }
        }),
        "channel serializationOptions should apply to script.message data"
    );

    let root_messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        5,
        &context_id,
        "(channel) => channel({'foo': 'bar', 'baz': {'1': 2}})",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name",
                "ownership": "root"
            }
        })],
        1,
    )
    .await;
    let root_response = bidi_message_by_id(&root_messages, 5);
    let root_realm = root_response["result"]["realm"]
        .as_str()
        .expect("root callFunction response realm");
    let root_event = bidi_events_by_method(&root_messages, "script.message")
        .pop()
        .expect("root script.message event");
    assert!(
        root_event["params"]["data"]["handle"]
            .as_str()
            .is_some_and(|handle| !handle.is_empty()),
        "root channel data should include a handle: {root_event:?}"
    );
    assert_eq!(
        root_event["params"]["data"]["type"],
        json!("object"),
        "root channel data should keep object type"
    );
    assert_eq!(
        root_event["params"]["data"]["value"],
        json!([
            ["foo", {"type": "string", "value": "bar"}],
            [
                "baz",
                {
                    "type": "object",
                    "value": [["1", {"type": "number", "value": 2}]]
                }
            ]
        ]),
        "root channel data should still include the serialized object value"
    );
    assert_eq!(
        root_event["params"]["source"],
        json!({
            "realm": root_realm,
            "context": context_id
        })
    );

    let multiple_arg_messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        6,
        &context_id,
        "(channel) => channel('will_be_send', 'will_be_ignored')",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name"
            }
        })],
        1,
    )
    .await;
    let multiple_arg_response = bidi_message_by_id(&multiple_arg_messages, 6);
    let multiple_arg_realm = multiple_arg_response["result"]["realm"]
        .as_str()
        .expect("multiple-argument callFunction response realm");
    let multiple_arg_event = bidi_events_by_method(&multiple_arg_messages, "script.message")
        .pop()
        .expect("multiple-argument script.message event");
    assert_eq!(
        multiple_arg_event["params"],
        json!({
            "channel": "channel_name",
            "data": {"type": "string", "value": "will_be_send"},
            "source": {
                "realm": multiple_arg_realm,
                "context": context_id
            }
        })
    );

    let two_channel_messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        7,
        &context_id,
        "(channel_1, channel_2) => { channel_1('message_from_channel_1'); channel_2('message_from_channel_2'); }",
        vec![
            json!({
                "type": "channel",
                "value": {
                    "channel": "channel_name_1"
                }
            }),
            json!({
                "type": "channel",
                "value": {
                    "channel": "channel_name_2"
                }
            }),
        ],
        2,
    )
    .await;
    let two_channel_response = bidi_message_by_id(&two_channel_messages, 7);
    let two_channel_realm = two_channel_response["result"]["realm"]
        .as_str()
        .expect("two-channel callFunction response realm");
    let two_channel_events = bidi_events_by_method(&two_channel_messages, "script.message");
    assert_eq!(two_channel_events.len(), 2);
    assert_eq!(
        two_channel_events[0]["params"],
        json!({
            "channel": "channel_name_1",
            "data": {"type": "string", "value": "message_from_channel_1"},
            "source": {
                "realm": two_channel_realm,
                "context": context_id
            }
        })
    );
    assert_eq!(
        two_channel_events[1]["params"],
        json!({
            "channel": "channel_name_2",
            "data": {"type": "string", "value": "message_from_channel_2"},
            "source": {
                "realm": two_channel_realm,
                "context": context_id
            }
        })
    );

    let mixed_arg_messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        8,
        &context_id,
        "(string, channel) => channel(string)",
        vec![
            json!({"type": "string", "value": "foo"}),
            json!({
                "type": "channel",
                "value": {
                    "channel": "channel_name"
                }
            }),
        ],
        1,
    )
    .await;
    let mixed_arg_response = bidi_message_by_id(&mixed_arg_messages, 8);
    let mixed_arg_realm = mixed_arg_response["result"]["realm"]
        .as_str()
        .expect("mixed-argument callFunction response realm");
    let mixed_arg_event = bidi_events_by_method(&mixed_arg_messages, "script.message")
        .pop()
        .expect("mixed-argument script.message event");
    assert_eq!(
        mixed_arg_event["params"],
        json!({
            "channel": "channel_name",
            "data": {"type": "string", "value": "foo"},
            "source": {
                "realm": mixed_arg_realm,
                "context": context_id
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_script_message_unsubscribe_stops_events() {
    // Mirrors webdriver/tests/bidi/script/message/message.py::test_unsubscribe.
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let unsubscribe = send_bidi_command(
        &mut socket,
        4,
        "session.unsubscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(unsubscribe["type"], json!("success"));

    let messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        5,
        &context_id,
        "(channel) => channel('foo')",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name"
            }
        })],
        0,
    )
    .await;
    assert_eq!(bidi_message_by_id(&messages, 5)["type"], json!("success"));
    assert!(
        bidi_events_by_method(&messages, "script.message").is_empty(),
        "script.message should not be emitted after unsubscribe: {messages:#?}"
    );
    let no_late_event = timeout(Duration::from_millis(300), recv_ws_json(&mut socket)).await;
    match no_late_event {
        Err(_) => {}
        Ok(message) => panic!("unexpected script.message after unsubscribe: {message:#?}"),
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_script_message_subscription_filters_context() {
    // Mirrors webdriver/tests/bidi/script/message/message.py::test_subscribe_to_one_context.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));
    let first = send_bidi_command(
        &mut socket,
        2,
        "browsingContext.create",
        json!({ "type": "tab" }),
    )
    .await;
    assert_eq!(first["type"], json!("success"));
    let first_context = first["result"]["context"]
        .as_str()
        .expect("first context id")
        .to_owned();
    let second = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.create",
        json!({ "type": "tab" }),
    )
    .await;
    assert_eq!(second["type"], json!("success"));
    let second_context = second["result"]["context"]
        .as_str()
        .expect("second context id")
        .to_owned();
    let subscribe = send_bidi_command(
        &mut socket,
        4,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [first_context.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let second_messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        5,
        &second_context,
        "(channel) => channel('foo')",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name"
            }
        })],
        0,
    )
    .await;
    assert_eq!(
        bidi_message_by_id(&second_messages, 5)["type"],
        json!("success")
    );
    assert!(
        bidi_events_by_method(&second_messages, "script.message").is_empty(),
        "script.message should be filtered for unsubscribed context: {second_messages:#?}"
    );
    let no_second_context_event =
        timeout(Duration::from_millis(300), recv_ws_json(&mut socket)).await;
    match no_second_context_event {
        Err(_) => {}
        Ok(message) => panic!("unexpected script.message for unsubscribed context: {message:#?}"),
    }

    let first_messages = send_bidi_script_call_function_and_collect_messages(
        &mut socket,
        6,
        &first_context,
        "(channel) => channel('foo')",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name"
            }
        })],
        1,
    )
    .await;
    let first_response = bidi_message_by_id(&first_messages, 6);
    assert_eq!(first_response["type"], json!("success"));
    let first_realm = first_response["result"]["realm"]
        .as_str()
        .expect("first context callFunction realm");
    let first_event = bidi_events_by_method(&first_messages, "script.message")
        .pop()
        .expect("script.message for subscribed context");
    assert_eq!(
        first_event["params"],
        json!({
            "channel": "channel_name",
            "data": {"type": "string", "value": "foo"},
            "source": {
                "realm": first_realm,
                "context": first_context
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
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
async fn websocket_bidi_pending_await_promise_does_not_block_later_session_command() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command_response(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"), "{session:?}");

    let create = send_bidi_command_response(
        &mut socket,
        2,
        "browsingContext.create",
        json!({"type": "tab"}),
    )
    .await;
    assert_eq!(create["type"], json!("success"), "{create:?}");
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();

    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "new Promise(() => {})",
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
        .expect("send permanently pending script.evaluate");

    let immediate = timeout(Duration::from_millis(200), recv_ws_json(&mut socket)).await;
    if let Ok(collected) = immediate {
        assert_eq!(collected["id"], json!(3_u64), "{collected:?}");
        assert_eq!(collected["type"], json!("error"), "{collected:?}");
        assert_eq!(
            collected["message"],
            json!("Promise was collected"),
            "V8 Inspector may collect an otherwise unreachable pending promise, matching Chromium: {collected:?}"
        );
    }

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "session.status",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send session.status while awaitPromise is pending");

    let status_messages = timeout(Duration::from_secs(1), recv_until_id(&mut socket, 4))
        .await
        .expect("session.status should not be blocked by an earlier pending awaitPromise");
    let status = bidi_message_by_id(&status_messages, 4);
    assert_eq!(status["type"], json!("success"), "{status:?}");

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browser.close",
                "params": {}
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browser.close while awaitPromise is pending");

    let close_messages = timeout(Duration::from_secs(1), recv_until_id(&mut socket, 5))
        .await
        .expect("browser.close should not be blocked by an earlier pending awaitPromise");
    let close_response = bidi_message_by_id(&close_messages, 5);
    assert_eq!(
        close_response["type"],
        json!("success"),
        "{close_response:?}"
    );

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
            "functionDeclaration": "() => navigator.userActivation.isActive || navigator.userActivation.hasBeenActive",
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
            "value": false
        }),
        "BiDi userActivation should be scoped to the wrapped call: {restored:?}"
    );

    let spoofed_global = send_bidi_command(
        &mut socket,
        6,
        "script.evaluate",
        json!({
            "expression": "globalThis.__moliWebDriverBidiUserActivation = true; navigator.userActivation.isActive || navigator.userActivation.hasBeenActive",
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
            "value": false
        }),
        "page globals must not spoof BiDi userActivation: {spoofed_global:?}"
    );

    for (id, user_activation, expected) in [(7_u64, false, false), (8, true, true)] {
        let response = send_bidi_command(
            &mut socket,
            id,
            "script.callFunction",
            json!({
                "functionDeclaration": "() => document.body.appendChild(document.createTextNode('test')) && document.execCommand('selectAll') && document.execCommand('copy')",
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
            "execCommand('copy') should follow BiDi userActivation={user_activation}: {response:?}"
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
async fn websocket_bidi_browsing_context_locate_nodes_matches_wpt_locator_basics() {
    // Mirrors Chromium/WPT webdriver/tests/bidi/browsing_context/locate_nodes/locator.py
    // for the core locator families.
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

    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": "data:text/html,<div data-class='one' role='banner' aria-label='foo'>foobarBARbaz</div><div data-class='two' role='banner' aria-label='foo'>foobarBARbaz</div>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    for (offset, locator) in [
        json!({ "type": "css", "value": "div" }),
        json!({ "type": "xpath", "value": "//div" }),
        json!({ "type": "innerText", "value": "foobarBARbaz" }),
        json!({ "type": "accessibility", "value": { "role": "banner" } }),
        json!({ "type": "accessibility", "value": { "name": "foo" } }),
        json!({ "type": "accessibility", "value": { "role": "banner", "name": "foo" } }),
    ]
    .into_iter()
    .enumerate()
    {
        let response = send_bidi_command(
            &mut socket,
            4 + offset as u64,
            "browsingContext.locateNodes",
            json!({
                "context": context_id.clone(),
                "locator": locator,
                "maxNodeCount": 10
            }),
        )
        .await;
        assert_eq!(
            response["type"],
            json!("success"),
            "locateNodes should succeed: {response:?}"
        );
        assert_locate_nodes_two_divs(&response);
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_browsing_context_locate_nodes_accepts_start_nodes_and_max_count() {
    // Mirrors the Chromium/WPT locate_nodes start_nodes and max_node_count coverage
    // with the shared node remote values returned by locateNodes itself.
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

    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": "data:text/html,<section data-scope='one'><span data-hit='one-a'></span><span data-hit='one-b'></span></section><section data-scope='two'><span data-hit='two-a'></span></section>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let sections = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.locateNodes",
        json!({
            "context": context_id.clone(),
            "locator": { "type": "css", "value": "section" }
        }),
    )
    .await;
    assert_eq!(sections["type"], json!("success"));
    let first_section = sections["result"]["nodes"][0].clone();

    let spans = send_bidi_command(
        &mut socket,
        5,
        "browsingContext.locateNodes",
        json!({
            "context": context_id,
            "locator": { "type": "css", "value": "span" },
            "startNodes": [first_section],
            "maxNodeCount": 1
        }),
    )
    .await;
    assert_eq!(
        spans["type"],
        json!("success"),
        "locateNodes should accept returned node remote values as startNodes: {spans:?}"
    );
    let nodes = spans["result"]["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("startNodes locateNodes result should contain nodes: {spans:?}"));
    assert_eq!(nodes.len(), 1, "maxNodeCount should limit scoped results");
    assert_eq!(nodes[0]["value"]["localName"], json!("span"));
    assert_eq!(nodes[0]["value"]["attributes"]["data-hit"], json!("one-a"));

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_browsing_context_locate_nodes_context_locator_returns_iframe_owner() {
    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><main>child-frame</main></body></html>",
        )
    }

    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi locateNodes context fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi locateNodes context fixture addr");
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
                            "<!doctype html><html><body><main>parent</main><iframe id=\"target\" src=\"{child_url}\"></iframe></body></html>"
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

    let tree = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.getTree",
        json!({
            "root": context_id.clone()
        }),
    )
    .await;
    assert_eq!(tree["type"], json!("success"));
    let child_context_id = tree["result"]["contexts"][0]["children"][0]["context"]
        .as_str()
        .unwrap_or_else(|| panic!("getTree should expose iframe context: {tree:?}"))
        .to_owned();

    let located = send_bidi_command(
        &mut socket,
        5,
        "browsingContext.locateNodes",
        json!({
            "context": context_id,
            "locator": {
                "type": "context",
                "value": {
                    "context": child_context_id
                }
            }
        }),
    )
    .await;
    assert_eq!(
        located["type"],
        json!("success"),
        "context locator should resolve the iframe owner node: {located:?}"
    );
    let nodes = located["result"]["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("context locator should return nodes: {located:?}"));
    assert_eq!(nodes.len(), 1);
    let iframe = &nodes[0];
    assert_eq!(iframe["type"], json!("node"));
    assert!(
        iframe["sharedId"]
            .as_str()
            .is_some_and(|shared_id| !shared_id.is_empty()),
        "iframe owner should include sharedId: {iframe:?}"
    );
    assert_eq!(iframe["value"]["nodeType"], json!(1));
    assert_eq!(iframe["value"]["localName"], json!("iframe"));
    assert_eq!(
        iframe["value"]["namespaceURI"],
        json!("http://www.w3.org/1999/xhtml")
    );
    assert_eq!(iframe["value"]["childNodeCount"], json!(0));
    assert_eq!(iframe["value"]["attributes"]["id"], json!("target"));
    assert_eq!(iframe["value"]["attributes"]["src"], json!(child_url));

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
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
async fn websocket_bidi_set_viewport_rejects_iframe_context() {
    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Viewport Child</title><main>viewport-child</main>",
        )
    }

    let child_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi viewport child fixture listener");
    let child_addr = child_listener
        .local_addr()
        .expect("BiDi viewport child fixture addr");
    let child_url = format!("http://{child_addr}/child");
    let child_app = Router::new().route("/child", get(child));
    let child_server = tokio::spawn(async move { axum::serve(child_listener, child_app).await });

    let parent_html = format!(
        r#"<!doctype html>
<html>
<head><title>Viewport Parent</title></head>
<body><main>viewport-parent</main><iframe src="{child_url}"></iframe></body>
</html>"#
    );
    let parent_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi viewport parent fixture listener");
    let parent_addr = parent_listener
        .local_addr()
        .expect("BiDi viewport parent fixture addr");
    let parent_url = format!("http://{parent_addr}/");
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
                "method": "browsingContext.setViewport",
                "params": {
                    "context": child_context_id
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send iframe-context browsingContext.setViewport");
    let iframe_context = recv_ws_json(&mut socket).await;
    assert_eq!(iframe_context["type"], json!("error"));
    assert_eq!(iframe_context["id"], json!(5_u64));
    assert_eq!(iframe_context["error"], json!("invalid argument"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "browsingContext.setViewport",
                "params": {
                    "context": "missing-frame-context"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send unknown-context browsingContext.setViewport");
    let unknown_context = recv_ws_json(&mut socket).await;
    assert_eq!(unknown_context["type"], json!("error"));
    assert_eq!(unknown_context["id"], json!(6_u64));
    assert_eq!(unknown_context["error"], json!("no such frame"));

    let _ = socket.close(None).await;
    protocol_server.abort();
    parent_server.abort();
    child_server.abort();
}

#[tokio::test]
async fn websocket_bidi_iframe_context_print_and_capture_screenshot() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/capture_screenshot/context.py and
    // webdriver/tests/bidi/browsing_context/print/context.py.
    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Capture Child</title><main>capture-child</main>",
        )
    }

    let child_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi capture child fixture listener");
    let child_addr = child_listener
        .local_addr()
        .expect("BiDi capture child fixture addr");
    let child_url = format!("http://{child_addr}/child");
    let child_app = Router::new().route("/child", get(child));
    let child_server = tokio::spawn(async move { axum::serve(child_listener, child_app).await });

    let parent_html = format!(
        r#"<!doctype html>
<html>
<head><title>Capture Parent</title></head>
<body><main>capture-parent</main><iframe src="{child_url}"></iframe></body>
</html>"#
    );
    let parent_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi capture parent fixture listener");
    let parent_addr = parent_listener
        .local_addr()
        .expect("BiDi capture parent fixture addr");
    let parent_url = format!("http://{parent_addr}/");
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
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id,
            "url": parent_url,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let all_realms = send_bidi_command(
        &mut socket,
        4,
        "script.getRealms",
        json!({"type": "window"}),
    )
    .await;
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

    let screenshot = send_bidi_command(
        &mut socket,
        5,
        "browsingContext.captureScreenshot",
        json!({
            "context": child_context_id.clone(),
            "format": {
                "type": "image/png"
            }
        }),
    )
    .await;
    assert_eq!(screenshot["type"], json!("error"));
    assert_eq!(screenshot["error"], json!("unsupported operation"));
    assert_eq!(
        screenshot["message"],
        json!("Page.captureScreenshot is not supported: renderer screenshots are not implemented.")
    );

    let print = send_bidi_command(
        &mut socket,
        6,
        "browsingContext.print",
        json!({
            "context": child_context_id,
            "orientation": "portrait",
            "page": {
                "width": 21.59,
                "height": 27.94
            },
            "margin": {
                "top": 1.0,
                "bottom": 1.0,
                "left": 1.0,
                "right": 1.0
            }
        }),
    )
    .await;
    assert_eq!(print["type"], json!("error"));
    assert_eq!(print["error"], json!("unsupported operation"));
    assert_eq!(
        print["message"],
        json!("Page.printToPDF is not supported: PDF generation is not implemented.")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    parent_server.abort();
    child_server.abort();
}

#[tokio::test]
async fn websocket_bidi_capture_screenshot_reports_unsupported_without_placeholder_payload() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/capture_screenshot/capture_screenshot.py and
    // webdriver/tests/bidi/browsing_context/capture_screenshot/clip.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let viewport = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.setViewport",
        json!({
            "context": context_id,
            "viewport": {
                "width": 120,
                "height": 80
            },
            "devicePixelRatio": 1.5
        }),
    )
    .await;
    assert_eq!(viewport["type"], json!("success"));

    let navigate = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.navigate",
        json!({
            "context": context_id,
            "url": "data:text/html,<div style='width:1000px;height:1000px'>capture</div>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let full = send_bidi_command(
        &mut socket,
        5,
        "browsingContext.captureScreenshot",
        json!({
            "context": context_id,
            "format": {
                "type": "image/png"
            }
        }),
    )
    .await;
    assert_eq!(full["type"], json!("error"));
    assert_eq!(full["error"], json!("unsupported operation"));
    assert_eq!(
        full["message"],
        json!("Page.captureScreenshot is not supported: renderer screenshots are not implemented.")
    );

    let clip = send_bidi_command(
        &mut socket,
        6,
        "browsingContext.captureScreenshot",
        json!({
            "context": context_id,
            "clip": {
                "type": "box",
                "x": 5,
                "y": 10,
                "width": 33,
                "height": 17
            }
        }),
    )
    .await;
    assert_eq!(clip["type"], json!("error"));
    assert_eq!(clip["error"], json!("unsupported operation"));
    assert_eq!(
        clip["message"],
        json!("Page.captureScreenshot is not supported: renderer screenshots are not implemented.")
    );

    let navigate_element = send_bidi_command(
        &mut socket,
        7,
        "browsingContext.navigate",
        json!({
            "context": context_id,
            "url": "data:text/html,<input id='clip-target'>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate_element["type"], json!("success"));

    let element = send_bidi_command(
        &mut socket,
        8,
        "script.evaluate",
        json!({
            "expression": "document.querySelector('#clip-target')",
            "awaitPromise": false,
            "target": {
                "context": context_id
            }
        }),
    )
    .await;
    assert_eq!(element["type"], json!("success"));
    assert_eq!(element["result"]["type"], json!("success"));
    let shared_id = element["result"]["result"]["sharedId"]
        .as_str()
        .expect("element remote value should include sharedId")
        .to_owned();

    let element_clip = send_bidi_command(
        &mut socket,
        9,
        "browsingContext.captureScreenshot",
        json!({
            "context": context_id,
            "clip": {
                "type": "element",
                "element": {
                    "sharedId": shared_id
                }
            }
        }),
    )
    .await;
    assert_eq!(
        element_clip["type"],
        json!("error"),
        "element clip screenshot should fail explicitly: {element_clip:?}"
    );
    assert_eq!(element_clip["error"], json!("unsupported operation"));
    assert_eq!(
        element_clip["message"],
        json!("Page.captureScreenshot is not supported: renderer screenshots are not implemented.")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_print_reports_unsupported_without_placeholder_pdf() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/print/page.py and
    // webdriver/tests/bidi/browsing_context/print/orientation.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id,
            "url": "data:text/html,<main>print</main>",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let portrait = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.print",
        json!({
            "context": context_id,
            "orientation": "portrait",
            "page": {
                "width": 10.0,
                "height": 20.0
            },
            "margin": {
                "top": 0,
                "bottom": 0,
                "left": 0,
                "right": 0
            }
        }),
    )
    .await;
    assert_eq!(portrait["type"], json!("error"));
    assert_eq!(portrait["error"], json!("unsupported operation"));
    assert_eq!(
        portrait["message"],
        json!("Page.printToPDF is not supported: PDF generation is not implemented.")
    );

    let landscape = send_bidi_command(
        &mut socket,
        5,
        "browsingContext.print",
        json!({
            "context": context_id,
            "orientation": "landscape",
            "page": {
                "width": 10.0,
                "height": 20.0
            },
            "margin": {
                "top": 0,
                "bottom": 0,
                "left": 0,
                "right": 0
            }
        }),
    )
    .await;
    assert_eq!(landscape["type"], json!("error"));
    assert_eq!(landscape["error"], json!("unsupported operation"));
    assert_eq!(
        landscape["message"],
        json!("Page.printToPDF is not supported: PDF generation is not implemented.")
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_set_viewport_resets_and_persists_across_navigation() {
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

    let initial_url = "data:text/html,<title>Viewport Initial</title><main>viewport-initial</main>";
    socket
        .send(WsMessage::Text(
            json!({
                "id": 3_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": initial_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send initial viewport browsingContext.navigate");
    let navigate = recv_ws_json(&mut socket).await;
    assert_eq!(navigate["type"], json!("success"));

    let original_surface = bidi_viewport_surface(&mut socket, 4, &context_id).await;
    let override_surface = json!({
        "width": 499_u64,
        "height": 599_u64,
        "dpr": 2_u64
    });
    assert_ne!(original_surface, override_surface);

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.setViewport",
                "params": {
                    "context": context_id.clone(),
                    "viewport": {
                        "width": 499,
                        "height": 599
                    },
                    "devicePixelRatio": 2.0
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.setViewport override");
    let viewport = recv_ws_json(&mut socket).await;
    assert_eq!(viewport["type"], json!("success"));
    assert_eq!(
        bidi_viewport_surface(&mut socket, 6, &context_id).await,
        override_surface
    );

    let first_url = "data:text/html,<title>Viewport A</title><main>viewport-a</main>";
    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": first_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send first viewport browsingContext.navigate");
    let navigate = recv_ws_json(&mut socket).await;
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(
        bidi_viewport_surface(&mut socket, 8, &context_id).await,
        override_surface
    );

    let second_url = "data:text/html,<title>Viewport B</title><main>viewport-b</main>";
    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": second_url,
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send second viewport browsingContext.navigate");
    let navigate = recv_ws_json(&mut socket).await;
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(
        bidi_viewport_surface(&mut socket, 10, &context_id).await,
        override_surface
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 11_u64,
                "method": "browsingContext.reload",
                "params": {
                    "context": context_id.clone(),
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send viewport browsingContext.reload");
    let reload = recv_ws_json(&mut socket).await;
    assert_eq!(reload["type"], json!("success"));
    assert_eq!(
        bidi_viewport_surface(&mut socket, 12, &context_id).await,
        override_surface
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 13_u64,
                "method": "browsingContext.setViewport",
                "params": {
                    "context": context_id.clone(),
                    "viewport": null,
                    "devicePixelRatio": null
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.setViewport reset");
    let reset = recv_ws_json(&mut socket).await;
    assert_eq!(reset["type"], json!("success"));
    assert_eq!(
        bidi_viewport_surface(&mut socket, 14, &context_id).await,
        original_surface
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_emulation_set_user_agent_override_matches_wpt_precedence() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_user_agent_override/user_agent.py,
    // contexts.py, global.py, and user_contexts.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, default_context_id) = bidi_session_with_context(cdp_addr).await;

    let default_user_agent =
        bidi_string_script_value(&mut socket, 3, &default_context_id, "navigator.userAgent").await;
    let global_user_agent = "Moli-BiDi-Global-UA/1.0";
    let user_context_user_agent = "Moli-BiDi-UserContext-UA/1.0";
    let context_user_agent = "Moli-BiDi-Context-UA/1.0";
    assert_ne!(default_user_agent, global_user_agent);

    let set_global = send_bidi_command(
        &mut socket,
        4,
        "emulation.setUserAgentOverride",
        json!({
            "userAgent": global_user_agent
        }),
    )
    .await;
    assert_eq!(set_global["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(&mut socket, 5, &default_context_id, "navigator.userAgent").await,
        global_user_agent
    );

    let user_context =
        send_bidi_command(&mut socket, 6, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context")
        .to_owned();

    let user_context_tab = send_bidi_command(
        &mut socket,
        7,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(user_context_tab["type"], json!("success"));
    let user_context_tab_id = user_context_tab["result"]["context"]
        .as_str()
        .expect("created userContext tab")
        .to_owned();
    assert_eq!(
        bidi_string_script_value(&mut socket, 8, &user_context_tab_id, "navigator.userAgent").await,
        global_user_agent,
        "new userContext tab should inherit the global userAgent override"
    );

    let set_user_context = send_bidi_command(
        &mut socket,
        9,
        "emulation.setUserAgentOverride",
        json!({
            "userContexts": [user_context_id],
            "userAgent": user_context_user_agent
        }),
    )
    .await;
    assert_eq!(set_user_context["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(&mut socket, 10, &user_context_tab_id, "navigator.userAgent")
            .await,
        user_context_user_agent
    );
    assert_eq!(
        bidi_string_script_value(&mut socket, 11, &default_context_id, "navigator.userAgent").await,
        global_user_agent,
        "non-default userContext override should not affect default contexts"
    );

    let set_context = send_bidi_command(
        &mut socket,
        12,
        "emulation.setUserAgentOverride",
        json!({
            "contexts": [user_context_tab_id],
            "userAgent": context_user_agent
        }),
    )
    .await;
    assert_eq!(set_context["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(&mut socket, 13, &user_context_tab_id, "navigator.userAgent")
            .await,
        context_user_agent
    );

    let reset_context = send_bidi_command(
        &mut socket,
        14,
        "emulation.setUserAgentOverride",
        json!({
            "contexts": [user_context_tab_id],
            "userAgent": null
        }),
    )
    .await;
    assert_eq!(reset_context["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(&mut socket, 15, &user_context_tab_id, "navigator.userAgent")
            .await,
        user_context_user_agent,
        "context reset should reveal userContext userAgent override"
    );

    let reset_user_context = send_bidi_command(
        &mut socket,
        16,
        "emulation.setUserAgentOverride",
        json!({
            "userContexts": [user_context_id],
            "userAgent": null
        }),
    )
    .await;
    assert_eq!(reset_user_context["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(&mut socket, 17, &user_context_tab_id, "navigator.userAgent")
            .await,
        global_user_agent,
        "userContext reset should reveal global userAgent override"
    );

    let reset_global = send_bidi_command(
        &mut socket,
        18,
        "emulation.setUserAgentOverride",
        json!({
            "userAgent": null
        }),
    )
    .await;
    assert_eq!(reset_global["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(&mut socket, 19, &default_context_id, "navigator.userAgent").await,
        default_user_agent
    );
    assert_eq!(
        bidi_string_script_value(&mut socket, 20, &user_context_tab_id, "navigator.userAgent")
            .await,
        default_user_agent
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_emulation_set_network_conditions_matches_wpt_precedence() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_network_conditions/{contexts,global,user_contexts}.py
    // plus Selenium's set_network_conditions(offline=True/False) facade smoke.
    async fn index() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>network conditions</title>",
        )
    }
    async fn ping() -> &'static str {
        "pong"
    }

    let fixture_app = Router::new()
        .route("/", get(index))
        .route("/ping", get(ping));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi network conditions fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi network conditions fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, default_context_id) = bidi_session_with_context(cdp_addr).await;

    let navigate = send_bidi_command_response(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": default_context_id.clone(),
            "url": fixture_url,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            4,
            &default_context_id,
            "String(navigator.onLine)"
        )
        .await,
        "true"
    );
    assert_eq!(
        bidi_awaited_string_script_value(
            &mut socket,
            5,
            &default_context_id,
            "fetch('/ping').then(response => response.text()).catch(() => 'offline')",
        )
        .await,
        "pong"
    );

    let set_global_offline = send_bidi_command(
        &mut socket,
        6,
        "emulation.setNetworkConditions",
        json!({
            "networkConditions": {
                "type": "offline"
            }
        }),
    )
    .await;
    assert_eq!(set_global_offline["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            7,
            &default_context_id,
            "String(navigator.onLine)"
        )
        .await,
        "false"
    );
    assert_eq!(
        bidi_awaited_string_script_value(
            &mut socket,
            8,
            &default_context_id,
            "fetch('/ping').then(response => response.text()).catch(() => 'offline')",
        )
        .await,
        "offline"
    );

    let user_context =
        send_bidi_command(&mut socket, 9, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context")
        .to_owned();
    let user_context_tab = send_bidi_command(
        &mut socket,
        10,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(user_context_tab["type"], json!("success"));
    let user_context_tab_id = user_context_tab["result"]["context"]
        .as_str()
        .expect("created userContext tab")
        .to_owned();
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            11,
            &user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "false",
        "later userContext tab should inherit the global network conditions"
    );

    let reset_user_context_under_global = send_bidi_command(
        &mut socket,
        12,
        "emulation.setNetworkConditions",
        json!({
            "userContexts": [user_context_id],
            "networkConditions": null
        }),
    )
    .await;
    assert_eq!(reset_user_context_under_global["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            13,
            &user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "false",
        "userContext reset should reveal global network conditions"
    );

    let reset_global = send_bidi_command(
        &mut socket,
        14,
        "emulation.setNetworkConditions",
        json!({
            "networkConditions": null
        }),
    )
    .await;
    assert_eq!(reset_global["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            15,
            &default_context_id,
            "String(navigator.onLine)"
        )
        .await,
        "true"
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            16,
            &user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "true"
    );

    let set_user_context_offline = send_bidi_command(
        &mut socket,
        17,
        "emulation.setNetworkConditions",
        json!({
            "userContexts": [user_context_id],
            "networkConditions": {
                "type": "offline"
            }
        }),
    )
    .await;
    assert_eq!(set_user_context_offline["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            18,
            &user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "false"
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            19,
            &default_context_id,
            "String(navigator.onLine)"
        )
        .await,
        "true",
        "non-default userContext network conditions should not affect default contexts"
    );

    let later_user_context_tab = send_bidi_command(
        &mut socket,
        20,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(later_user_context_tab["type"], json!("success"));
    let later_user_context_tab_id = later_user_context_tab["result"]["context"]
        .as_str()
        .expect("later created userContext tab")
        .to_owned();
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            21,
            &later_user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "false",
        "later context should inherit userContext network conditions"
    );

    let set_context_offline = send_bidi_command(
        &mut socket,
        22,
        "emulation.setNetworkConditions",
        json!({
            "contexts": [user_context_tab_id],
            "networkConditions": {
                "type": "offline"
            }
        }),
    )
    .await;
    assert_eq!(set_context_offline["type"], json!("success"));
    let reset_user_context = send_bidi_command(
        &mut socket,
        23,
        "emulation.setNetworkConditions",
        json!({
            "userContexts": [user_context_id],
            "networkConditions": null
        }),
    )
    .await;
    assert_eq!(reset_user_context["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            24,
            &user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "false",
        "context override should survive userContext reset"
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            25,
            &later_user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "true",
        "userContext reset should restore sibling contexts without context override"
    );

    let reset_context = send_bidi_command(
        &mut socket,
        26,
        "emulation.setNetworkConditions",
        json!({
            "contexts": [user_context_tab_id],
            "networkConditions": null
        }),
    )
    .await;
    assert_eq!(reset_context["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            27,
            &user_context_tab_id,
            "String(navigator.onLine)"
        )
        .await,
        "true"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_bidi_emulation_user_context_overrides_apply_to_later_created_context() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _default_context_id) = bidi_session_with_context(cdp_addr).await;

    let user_context =
        send_bidi_command(&mut socket, 3, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context")
        .to_owned();
    let user_agent = "Moli-BiDi-Later-Context-UA/1.0";
    let locale = "fr-FR";
    let timezone = "Asia/Tokyo";

    let set_user_context_user_agent = send_bidi_command(
        &mut socket,
        4,
        "emulation.setUserAgentOverride",
        json!({
            "userContexts": [user_context_id.clone()],
            "userAgent": user_agent
        }),
    )
    .await;
    assert_eq!(set_user_context_user_agent["type"], json!("success"));
    let set_user_context_locale = send_bidi_command(
        &mut socket,
        5,
        "emulation.setLocaleOverride",
        json!({
            "userContexts": [user_context_id.clone()],
            "locale": locale
        }),
    )
    .await;
    assert_eq!(set_user_context_locale["type"], json!("success"));
    let set_user_context_timezone = send_bidi_command(
        &mut socket,
        6,
        "emulation.setTimezoneOverride",
        json!({
            "userContexts": [user_context_id.clone()],
            "timezone": timezone
        }),
    )
    .await;
    assert_eq!(set_user_context_timezone["type"], json!("success"));

    let tab = send_bidi_command(
        &mut socket,
        7,
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
    assert_eq!(
        bidi_string_script_value(&mut socket, 8, &context_id, "navigator.userAgent").await,
        user_agent
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            9,
            &context_id,
            "Intl.DateTimeFormat().resolvedOptions().locale"
        )
        .await,
        locale
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            10,
            &context_id,
            "Intl.DateTimeFormat().resolvedOptions().timeZone"
        )
        .await,
        timezone
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_emulation_user_context_overrides_apply_to_later_http_navigation() {
    async fn profile(headers: axum::http::HeaderMap) -> impl IntoResponse {
        let user_agent = headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let accept_language = headers
            .get(axum::http::header::ACCEPT_LANGUAGE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let body = serde_json::json!({
            "userAgent": user_agent,
            "acceptLanguage": accept_language
        });
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            format!("<!doctype html><main id='profile'>{}</main>", body),
        )
    }

    let fixture_app = Router::new().route("/", get(profile));
    let fixture_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind BiDi userContext profile fixture listener");
    let fixture_addr = fixture_listener
        .local_addr()
        .expect("BiDi userContext profile fixture addr");
    let fixture_server =
        tokio::spawn(async move { axum::serve(fixture_listener, fixture_app).await });
    let fixture_url = format!("http://{fixture_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _default_context_id) = bidi_session_with_context(cdp_addr).await;

    let user_context =
        send_bidi_command(&mut socket, 3, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context")
        .to_owned();
    let user_agent = "Moli-BiDi-HTTP-UA/1.0";
    let locale = "fr-FR";

    assert_eq!(
        send_bidi_command(
            &mut socket,
            4,
            "emulation.setUserAgentOverride",
            json!({
                "userContexts": [user_context_id.clone()],
                "userAgent": user_agent
            }),
        )
        .await["type"],
        json!("success")
    );
    assert_eq!(
        send_bidi_command(
            &mut socket,
            5,
            "emulation.setLocaleOverride",
            json!({
                "userContexts": [user_context_id.clone()],
                "locale": locale
            }),
        )
        .await["type"],
        json!("success")
    );

    let tab = send_bidi_command(
        &mut socket,
        6,
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
        7,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": fixture_url,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    assert_eq!(
        bidi_string_script_value(&mut socket, 8, &context_id, "navigator.userAgent").await,
        user_agent
    );
    assert_eq!(
        bidi_string_script_value(&mut socket, 9, &context_id, "navigator.language").await,
        locale
    );
    let profile = bidi_string_script_value(
        &mut socket,
        10,
        &context_id,
        "document.getElementById('profile').textContent",
    )
    .await;
    let profile: serde_json::Value =
        serde_json::from_str(&profile).expect("profile echo should be JSON");
    assert_eq!(profile["userAgent"], json!(user_agent));
    assert_eq!(profile["acceptLanguage"], json!(locale));

    let _ = socket.close(None).await;
    protocol_server.abort();
    fixture_server.abort();
}

#[tokio::test]
async fn websocket_bidi_emulation_set_locale_and_timezone_override_match_wpt_precedence() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_locale_override/{locale,contexts,user_contexts}.py
    // and set_timezone_override/{timezone,contexts,user_contexts}.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, default_context_id) = bidi_session_with_context(cdp_addr).await;

    let default_locale = bidi_string_script_value(
        &mut socket,
        3,
        &default_context_id,
        "Intl.DateTimeFormat().resolvedOptions().locale",
    )
    .await;
    let default_timezone = bidi_string_script_value(
        &mut socket,
        4,
        &default_context_id,
        "Intl.DateTimeFormat().resolvedOptions().timeZone",
    )
    .await;
    let user_context_locale = if default_locale == "fr-FR" {
        "de-DE"
    } else {
        "fr-FR"
    };
    let context_locale = if user_context_locale == "fr-FR" {
        "de-DE"
    } else {
        "fr-FR"
    };
    let user_context_timezone = if default_timezone == "Asia/Tokyo" {
        "Europe/Berlin"
    } else {
        "Asia/Tokyo"
    };
    let context_timezone = if user_context_timezone == "Asia/Tokyo" {
        "Europe/Berlin"
    } else {
        "Asia/Tokyo"
    };

    let user_context =
        send_bidi_command(&mut socket, 5, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context")
        .to_owned();

    let user_context_tab = send_bidi_command(
        &mut socket,
        6,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(user_context_tab["type"], json!("success"));
    let user_context_tab_id = user_context_tab["result"]["context"]
        .as_str()
        .expect("created userContext tab")
        .to_owned();

    let set_user_context_locale = send_bidi_command(
        &mut socket,
        7,
        "emulation.setLocaleOverride",
        json!({
            "userContexts": [user_context_id],
            "locale": user_context_locale
        }),
    )
    .await;
    assert_eq!(set_user_context_locale["type"], json!("success"));
    let set_user_context_timezone = send_bidi_command(
        &mut socket,
        8,
        "emulation.setTimezoneOverride",
        json!({
            "userContexts": [user_context_id],
            "timezone": user_context_timezone
        }),
    )
    .await;
    assert_eq!(set_user_context_timezone["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            9,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().locale"
        )
        .await,
        user_context_locale
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            10,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().timeZone"
        )
        .await,
        user_context_timezone
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            11,
            &default_context_id,
            "Intl.DateTimeFormat().resolvedOptions().locale"
        )
        .await,
        default_locale,
        "non-default userContext locale should not affect default contexts"
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            12,
            &default_context_id,
            "Intl.DateTimeFormat().resolvedOptions().timeZone"
        )
        .await,
        default_timezone,
        "non-default userContext timezone should not affect default contexts"
    );

    let set_context_locale = send_bidi_command(
        &mut socket,
        13,
        "emulation.setLocaleOverride",
        json!({
            "contexts": [user_context_tab_id],
            "locale": context_locale
        }),
    )
    .await;
    assert_eq!(set_context_locale["type"], json!("success"));
    let set_context_timezone = send_bidi_command(
        &mut socket,
        14,
        "emulation.setTimezoneOverride",
        json!({
            "contexts": [user_context_tab_id],
            "timezone": context_timezone
        }),
    )
    .await;
    assert_eq!(set_context_timezone["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            15,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().locale"
        )
        .await,
        context_locale
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            16,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().timeZone"
        )
        .await,
        context_timezone
    );

    let reset_context_locale = send_bidi_command(
        &mut socket,
        17,
        "emulation.setLocaleOverride",
        json!({
            "contexts": [user_context_tab_id],
            "locale": null
        }),
    )
    .await;
    assert_eq!(reset_context_locale["type"], json!("success"));
    let reset_context_timezone = send_bidi_command(
        &mut socket,
        18,
        "emulation.setTimezoneOverride",
        json!({
            "contexts": [user_context_tab_id],
            "timezone": null
        }),
    )
    .await;
    assert_eq!(reset_context_timezone["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            19,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().locale"
        )
        .await,
        user_context_locale,
        "context locale reset should reveal userContext locale override"
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            20,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().timeZone"
        )
        .await,
        user_context_timezone,
        "context timezone reset should reveal userContext timezone override"
    );

    let reset_user_context_locale = send_bidi_command(
        &mut socket,
        21,
        "emulation.setLocaleOverride",
        json!({
            "userContexts": [user_context_id],
            "locale": null
        }),
    )
    .await;
    assert_eq!(reset_user_context_locale["type"], json!("success"));
    let reset_user_context_timezone = send_bidi_command(
        &mut socket,
        22,
        "emulation.setTimezoneOverride",
        json!({
            "userContexts": [user_context_id],
            "timezone": null
        }),
    )
    .await;
    assert_eq!(reset_user_context_timezone["type"], json!("success"));
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            23,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().locale"
        )
        .await,
        default_locale
    );
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            24,
            &user_context_tab_id,
            "Intl.DateTimeFormat().resolvedOptions().timeZone"
        )
        .await,
        default_timezone
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_set_viewport_user_contexts_apply_and_inherit() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/set_viewport/user_contexts.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _default_context_id) = bidi_session_with_context(cdp_addr).await;

    let user_context =
        send_bidi_command(&mut socket, 3, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context")
        .to_owned();

    let user_context_tab = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(user_context_tab["type"], json!("success"));
    let user_context_tab_id = user_context_tab["result"]["context"]
        .as_str()
        .expect("created userContext tab")
        .to_owned();

    let user_override = json!({
        "width": 377_u64,
        "height": 523_u64,
        "dpr": 1_u64
    });
    let set_user_context_viewport = send_bidi_command(
        &mut socket,
        6,
        "browsingContext.setViewport",
        json!({
            "userContexts": [user_context_id],
            "viewport": {
                "width": 377,
                "height": 523
            }
        }),
    )
    .await;
    assert_eq!(set_user_context_viewport["type"], json!("success"));
    assert_eq!(
        bidi_viewport_surface(&mut socket, 7, &user_context_tab_id).await,
        user_override
    );

    let user_dpr_override = json!({
        "width": 377_u64,
        "height": 523_u64,
        "dpr": 2.5
    });
    let set_user_context_dpr = send_bidi_command(
        &mut socket,
        8,
        "browsingContext.setViewport",
        json!({
            "userContexts": [user_context_id],
            "devicePixelRatio": 2.5
        }),
    )
    .await;
    assert_eq!(set_user_context_dpr["type"], json!("success"));
    assert_eq!(
        bidi_viewport_surface(&mut socket, 9, &user_context_tab_id).await,
        user_dpr_override
    );

    let inherited_tab = send_bidi_command(
        &mut socket,
        10,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(inherited_tab["type"], json!("success"));
    let inherited_tab_id = inherited_tab["result"]["context"]
        .as_str()
        .expect("created inherited userContext tab")
        .to_owned();
    assert_eq!(
        bidi_viewport_surface(&mut socket, 11, &inherited_tab_id).await,
        user_dpr_override,
        "new contexts in the userContext should inherit viewport and devicePixelRatio defaults"
    );

    let default_tab = send_bidi_command(
        &mut socket,
        12,
        "browsingContext.create",
        json!({ "type": "tab" }),
    )
    .await;
    assert_eq!(default_tab["type"], json!("success"));
    let default_tab_id = default_tab["result"]["context"]
        .as_str()
        .expect("created default tab")
        .to_owned();
    assert_ne!(
        bidi_viewport_surface(&mut socket, 13, &default_tab_id).await,
        user_dpr_override,
        "new default contexts should not inherit non-default userContext viewport defaults"
    );

    let default_override = json!({
        "width": 333_u64,
        "height": 444_u64,
        "dpr": 1_u64
    });
    let set_default_viewport = send_bidi_command(
        &mut socket,
        14,
        "browsingContext.setViewport",
        json!({
            "userContexts": ["default"],
            "viewport": {
                "width": 333,
                "height": 444
            }
        }),
    )
    .await;
    assert_eq!(set_default_viewport["type"], json!("success"));
    let default_inherited_tab = send_bidi_command(
        &mut socket,
        15,
        "browsingContext.create",
        json!({ "type": "tab" }),
    )
    .await;
    assert_eq!(default_inherited_tab["type"], json!("success"));
    let default_inherited_tab_id = default_inherited_tab["result"]["context"]
        .as_str()
        .expect("created default inherited tab")
        .to_owned();
    assert_eq!(
        bidi_viewport_surface(&mut socket, 16, &default_inherited_tab_id).await,
        default_override,
        "new default contexts should inherit default userContext viewport defaults"
    );
    let inherited_after_default_tab = send_bidi_command(
        &mut socket,
        17,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(inherited_after_default_tab["type"], json!("success"));
    let inherited_after_default_tab_id = inherited_after_default_tab["result"]["context"]
        .as_str()
        .expect("created userContext tab after default viewport update")
        .to_owned();
    assert_eq!(
        bidi_viewport_surface(&mut socket, 18, &inherited_after_default_tab_id).await,
        user_dpr_override,
        "default userContext viewport should not affect non-default userContexts"
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_set_viewport_user_context_inherits_through_window_open() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/set_viewport/window_open.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _default_context_id) = bidi_session_with_context(cdp_addr).await;
    const POPUP_URL: &str = "data:text/html,<title>Popup</title><main>popup</main>";

    let user_context =
        send_bidi_command(&mut socket, 3, "browser.createUserContext", json!({})).await;
    assert_eq!(user_context["type"], json!("success"));
    let user_context_id = user_context["result"]["userContext"]
        .as_str()
        .expect("created user context")
        .to_owned();

    let opener = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.create",
        json!({
            "type": "tab",
            "userContext": user_context_id
        }),
    )
    .await;
    assert_eq!(opener["type"], json!("success"));
    let opener_context_id = opener["result"]["context"]
        .as_str()
        .expect("created opener context")
        .to_owned();

    let expected_surface = json!({
        "width": 250_u64,
        "height": 300_u64,
        "dpr": 1_u64
    });
    let set_viewport = send_bidi_command(
        &mut socket,
        5,
        "browsingContext.setViewport",
        json!({
            "userContexts": [user_context_id],
            "viewport": {
                "width": 250,
                "height": 300
            }
        }),
    )
    .await;
    assert_eq!(set_viewport["type"], json!("success"));
    assert_eq!(
        bidi_viewport_surface(&mut socket, 6, &opener_context_id).await,
        expected_surface,
        "opener should observe its userContext viewport default"
    );

    let subscribe = send_bidi_command(
        &mut socket,
        7,
        "session.subscribe",
        json!({
            "events": [
                "browsingContext.contextCreated",
                "browsingContext.load"
            ]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "script.callFunction",
                "params": {
                    "functionDeclaration": format!(
                        "() => {{ const win = window.open({}); return JSON.stringify({{ opened: win !== null, width: win && win.innerWidth, height: win && win.innerHeight }}); }}",
                        serde_json::to_string(POPUP_URL).expect("serialize popup URL")
                    ),
                    "awaitPromise": false,
                    "target": {
                        "context": opener_context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send window.open script.callFunction");
    let mut messages = recv_until_id(&mut socket, 8).await;
    let call_response = messages
        .iter()
        .find(|message| message["id"] == json!(8_u64))
        .expect("window.open callFunction response");
    assert_eq!(call_response["type"], json!("success"));
    assert_eq!(call_response["result"]["type"], json!("success"));
    let popup_surface_payload = call_response["result"]["result"]["value"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("window.open surface should return a JSON string: {call_response:?}")
        });
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(popup_surface_payload).unwrap_or_else(|error| {
            panic!("window.open surface JSON should parse: {error}; {popup_surface_payload}")
        }),
        json!({
            "opened": true,
            "width": 250_u64,
            "height": 300_u64
        }),
        "popup WindowProxy should synchronously inherit the userContext viewport"
    );

    let popup_context_event = if let Some(message) = messages.iter().find(|message| {
        message["method"] == json!("browsingContext.contextCreated")
            && message["params"]["originalOpener"] == json!(opener_context_id)
    }) {
        message.clone()
    } else {
        let mut created_messages = recv_until_match(&mut socket, |message| {
            message["method"] == json!("browsingContext.contextCreated")
                && message["params"]["context"] != json!(opener_context_id)
                && message["params"]["userContext"] == json!(user_context_id)
        })
        .await;
        let event = created_messages
            .iter()
            .find(|message| {
                message["method"] == json!("browsingContext.contextCreated")
                    && message["params"]["context"] != json!(opener_context_id)
                    && message["params"]["userContext"] == json!(user_context_id)
            })
            .unwrap_or_else(|| panic!("expected popup contextCreated event: {created_messages:#?}"))
            .clone();
        messages.append(&mut created_messages);
        event
    };
    let popup_context_id = {
        let message = &popup_context_event;
        assert_eq!(message["type"], json!("event"));
        assert_eq!(
            message["params"]["originalOpener"],
            json!(opener_context_id),
            "popup contextCreated should retain opener: {message:?}"
        );
        assert_eq!(
            message["params"]["userContext"],
            json!(user_context_id),
            "popup contextCreated should retain userContext: {message:?}"
        );
        message["params"]["context"]
            .as_str()
            .expect("popup context id")
            .to_owned()
    };

    if !messages.iter().any(|message| {
        message["method"] == json!("browsingContext.load")
            && message["params"]["context"] == json!(popup_context_id)
            && message["params"]["url"] == json!(POPUP_URL)
    }) {
        let load_messages = recv_until_match(&mut socket, |message| {
            message["method"] == json!("browsingContext.load")
                && message["params"]["context"] == json!(popup_context_id)
                && message["params"]["url"] == json!(POPUP_URL)
        })
        .await;
        assert!(
            load_messages.iter().any(|message| {
                message["method"] == json!("browsingContext.load")
                    && message["params"]["context"] == json!(popup_context_id)
                    && message["params"]["url"] == json!(POPUP_URL)
            }),
            "expected popup load event: {load_messages:#?}"
        );
    }

    assert_eq!(
        bidi_viewport_surface(&mut socket, 9, &popup_context_id).await,
        expected_surface,
        "loaded popup context should inherit the userContext viewport"
    );

    let tree = send_bidi_command_response(
        &mut socket,
        10,
        "browsingContext.getTree",
        json!({
            "root": popup_context_id
        }),
    )
    .await;
    assert_eq!(tree["type"], json!("success"));
    assert_eq!(
        tree["result"]["contexts"][0]["userContext"],
        json!(user_context_id)
    );
    assert_eq!(
        tree["result"]["contexts"][0]["originalOpener"],
        json!(opener_context_id)
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
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

#[tokio::test]
async fn websocket_bidi_preload_script_runs_after_navigation_and_remove() {
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
                    "url": "data:text/html,bidi-preload-before-add",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send initial browsingContext.navigate");
    let initial_navigate = recv_ws_json(&mut socket).await;
    assert_eq!(initial_navigate["type"], json!("success"));
    assert_eq!(initial_navigate["id"], json!(3_u64));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.addPreloadScript",
                "params": {
                    "functionDeclaration": "() => { globalThis.__bidiPreload = 'from-preload'; }",
                    "contexts": [context_id.clone()]
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.addPreloadScript");
    let add_preload = recv_ws_json(&mut socket).await;
    assert_eq!(
        add_preload["type"],
        json!("success"),
        "addPreloadScript should succeed: {add_preload:?}"
    );
    assert_eq!(add_preload["id"], json!(4_u64));
    let preload_script = add_preload["result"]["script"]
        .as_str()
        .expect("preload script id")
        .to_owned();
    assert!(
        preload_script.starts_with(&context_id),
        "BiDi preload script id should be target-qualified"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "typeof globalThis.__bidiPreload",
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send pre-navigation script.evaluate");
    let pre_navigation_value = recv_ws_json(&mut socket).await;
    assert_eq!(pre_navigation_value["type"], json!("success"));
    assert_eq!(pre_navigation_value["id"], json!(5_u64));
    assert_eq!(
        pre_navigation_value["result"]["result"],
        json!({
            "type": "string",
            "value": "undefined"
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": "data:text/html,bidi-preload",
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
    assert_eq!(navigate["id"], json!(6_u64));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "globalThis.__bidiPreload",
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send post-navigation script.evaluate");
    let preload_value = recv_ws_json(&mut socket).await;
    assert_eq!(preload_value["type"], json!("success"));
    assert_eq!(preload_value["id"], json!(7_u64));
    assert_eq!(
        preload_value["result"]["result"],
        json!({
            "type": "string",
            "value": "from-preload"
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 8_u64,
                "method": "script.removePreloadScript",
                "params": {
                    "script": preload_script.clone()
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.removePreloadScript");
    let remove_preload = recv_ws_json(&mut socket).await;
    assert_eq!(remove_preload["type"], json!("success"));
    assert_eq!(remove_preload["id"], json!(8_u64));
    assert_eq!(remove_preload["result"], json!({}));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 9_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": "data:text/html,bidi-preload-removed",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send navigation after remove");
    let navigate_after_remove = recv_ws_json(&mut socket).await;
    assert_eq!(navigate_after_remove["type"], json!("success"));
    assert_eq!(navigate_after_remove["id"], json!(9_u64));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 10_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "typeof globalThis.__bidiPreload",
                    "target": {
                        "context": context_id.clone()
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send post-remove script.evaluate");
    let post_remove_value = recv_ws_json(&mut socket).await;
    assert_eq!(post_remove_value["type"], json!("success"));
    assert_eq!(post_remove_value["id"], json!(10_u64));
    assert_eq!(
        post_remove_value["result"]["result"],
        json!({
            "type": "string",
            "value": "undefined"
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 11_u64,
                "method": "script.removePreloadScript",
                "params": {
                    "script": preload_script
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send second script.removePreloadScript");
    let second_remove = recv_ws_json(&mut socket).await;
    assert_eq!(second_remove["type"], json!("error"));
    assert_eq!(second_remove["id"], json!(11_u64));
    assert_eq!(second_remove["error"], json!("no such script"));

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_add_preload_script_rejects_iframe_context() {
    // Reduced from Chromium/WPT
    // webdriver/tests/bidi/script/add_preload_script/invalid.py.
    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Child Preload Context</title><main>child</main>",
        )
    }

    let child_app = Router::new().route("/child", get(child));
    let (child_addr, _child_server) =
        spawn_dedicated_fixture_server(child_app, "bidi-preload-invalid-child-context");
    let child_url = format!("http://{child_addr}/child");
    let parent_html = format!(
        "<!doctype html><title>Parent Preload Context</title><iframe src=\"{child_url}\"></iframe>"
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
        spawn_dedicated_fixture_server(parent_app, "bidi-preload-invalid-parent-context");
    let parent_url = format!("http://{parent_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": parent_url,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"), "{navigate:?}");

    let tree = send_bidi_command(
        &mut socket,
        4,
        "browsingContext.getTree",
        json!({"root": context_id.clone()}),
    )
    .await;
    assert_eq!(tree["type"], json!("success"), "{tree:?}");
    let child_context_id = tree["result"]["contexts"][0]["children"][0]["context"]
        .as_str()
        .expect("iframe context id")
        .to_owned();

    let add_preload = send_bidi_command(
        &mut socket,
        5,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "() => {}",
            "contexts": [child_context_id]
        }),
    )
    .await;
    assert_bidi_error(
        &add_preload,
        "invalid argument",
        "script.addPreloadScript should reject iframe contexts",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_preload_script_channel_argument_emits_script_message() {
    // Mirrors webdriver/tests/bidi/script/add_preload_script/arguments.py::test_channel
    // for the default channel serialization case.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));
    let subscribe = send_bidi_command(
        &mut socket,
        2,
        "session.subscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let add_preload = send_bidi_command(
        &mut socket,
        3,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "(channel) => channel({'foo': 'bar', 'baz': {'1': 2}})",
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "channel_name"
                }
            }]
        }),
    )
    .await;
    assert_eq!(
        add_preload["type"],
        json!("success"),
        "addPreloadScript should succeed: {add_preload:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
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
    let create_messages = recv_until_id(&mut socket, 4).await;
    let messages =
        collect_bidi_messages_until_method_count(&mut socket, create_messages, "script.message", 1)
            .await;
    let create = bidi_message_by_id(&messages, 4);
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id");
    let script_message = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("preload script.message event");
    let realm = script_message["params"]["source"]["realm"]
        .as_str()
        .expect("preload script.message realm");
    assert!(!realm.is_empty());
    assert_eq!(
        script_message["params"],
        json!({
            "channel": "channel_name",
            "data": {
                "type": "object",
                "value": [
                    ["foo", {"type": "string", "value": "bar"}],
                    [
                        "baz",
                        {
                            "type": "object",
                            "value": [["1", {"type": "number", "value": 2}]]
                        }
                    ]
                ]
            },
            "source": {
                "realm": realm,
                "context": context_id
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_preload_channel_variants_match_wpt_shape() {
    // Mirrors the remaining Chromium/WPT
    // webdriver/tests/bidi/script/add_preload_script/arguments.py channel cases.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));
    let subscribe = send_bidi_command(
        &mut socket,
        2,
        "session.subscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    let shallow_script = send_bidi_add_preload_script(
        &mut socket,
        3,
        "(channel) => channel({'foo': 'bar', 'baz': {'1': 2}})",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name",
                "serializationOptions": {
                    "maxObjectDepth": 0
                }
            }
        })],
    )
    .await;
    let shallow_messages = create_bidi_context_and_collect_script_messages(&mut socket, 4, 1).await;
    let shallow_create = bidi_message_by_id(&shallow_messages, 4);
    assert_eq!(shallow_create["type"], json!("success"));
    let shallow_context = shallow_create["result"]["context"]
        .as_str()
        .expect("shallow context id")
        .to_owned();
    let shallow_event = bidi_events_by_method(&shallow_messages, "script.message")
        .pop()
        .expect("shallow preload script.message event");
    let shallow_realm = shallow_event["params"]["source"]["realm"]
        .as_str()
        .expect("shallow preload script.message realm");
    assert_eq!(
        shallow_event["params"],
        json!({
            "channel": "channel_name",
            "data": {
                "type": "object"
            },
            "source": {
                "realm": shallow_realm,
                "context": shallow_context
            }
        }),
        "preload channel serializationOptions should apply to script.message data"
    );
    remove_bidi_preload_script(&mut socket, 5, &shallow_script).await;

    let root_script = send_bidi_add_preload_script(
        &mut socket,
        6,
        "(channel) => channel({'foo': 'bar', 'baz': {'1': 2}})",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name",
                "ownership": "root"
            }
        })],
    )
    .await;
    let root_messages = create_bidi_context_and_collect_script_messages(&mut socket, 7, 1).await;
    let root_create = bidi_message_by_id(&root_messages, 7);
    assert_eq!(root_create["type"], json!("success"));
    let root_context = root_create["result"]["context"]
        .as_str()
        .expect("root context id")
        .to_owned();
    let root_event = bidi_events_by_method(&root_messages, "script.message")
        .pop()
        .expect("root preload script.message event");
    let root_realm = root_event["params"]["source"]["realm"]
        .as_str()
        .expect("root preload script.message realm");
    assert!(
        root_event["params"]["data"]["handle"]
            .as_str()
            .is_some_and(|handle| !handle.is_empty()),
        "root preload channel data should include a handle: {root_event:?}"
    );
    assert_eq!(root_event["params"]["channel"], json!("channel_name"));
    assert_eq!(root_event["params"]["data"]["type"], json!("object"));
    assert_eq!(
        root_event["params"]["data"]["value"],
        json!([
            ["foo", {"type": "string", "value": "bar"}],
            [
                "baz",
                {
                    "type": "object",
                    "value": [["1", {"type": "number", "value": 2}]]
                }
            ]
        ])
    );
    assert_eq!(
        root_event["params"]["source"],
        json!({
            "realm": root_realm,
            "context": root_context
        })
    );
    remove_bidi_preload_script(&mut socket, 8, &root_script).await;

    let multiple_arg_script = send_bidi_add_preload_script(
        &mut socket,
        9,
        "(channel) => channel('will_be_send', 'will_be_ignored')",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name"
            }
        })],
    )
    .await;
    let multiple_arg_messages =
        create_bidi_context_and_collect_script_messages(&mut socket, 10, 1).await;
    let multiple_arg_create = bidi_message_by_id(&multiple_arg_messages, 10);
    assert_eq!(multiple_arg_create["type"], json!("success"));
    let multiple_arg_context = multiple_arg_create["result"]["context"]
        .as_str()
        .expect("multiple-argument context id")
        .to_owned();
    let multiple_arg_event = bidi_events_by_method(&multiple_arg_messages, "script.message")
        .pop()
        .expect("multiple-argument preload script.message event");
    let multiple_arg_realm = multiple_arg_event["params"]["source"]["realm"]
        .as_str()
        .expect("multiple-argument preload script.message realm");
    assert_eq!(
        multiple_arg_event["params"],
        json!({
            "channel": "channel_name",
            "data": {"type": "string", "value": "will_be_send"},
            "source": {
                "realm": multiple_arg_realm,
                "context": multiple_arg_context
            }
        })
    );
    remove_bidi_preload_script(&mut socket, 11, &multiple_arg_script).await;

    let two_channel_script = send_bidi_add_preload_script(
        &mut socket,
        12,
        "(channel_1, channel_2) => { channel_1('message_from_channel_1'); channel_2('message_from_channel_2'); }",
        vec![
            json!({
                "type": "channel",
                "value": {
                    "channel": "channel_name_1"
                }
            }),
            json!({
                "type": "channel",
                "value": {
                    "channel": "channel_name_2"
                }
            }),
        ],
    )
    .await;
    let two_channel_messages =
        create_bidi_context_and_collect_script_messages(&mut socket, 13, 2).await;
    let two_channel_create = bidi_message_by_id(&two_channel_messages, 13);
    assert_eq!(two_channel_create["type"], json!("success"));
    let two_channel_context = two_channel_create["result"]["context"]
        .as_str()
        .expect("two-channel context id")
        .to_owned();
    let two_channel_events = bidi_events_by_method(&two_channel_messages, "script.message");
    assert_eq!(two_channel_events.len(), 2);
    let first_realm = two_channel_events[0]["params"]["source"]["realm"]
        .as_str()
        .expect("first two-channel preload script.message realm");
    let second_realm = two_channel_events[1]["params"]["source"]["realm"]
        .as_str()
        .expect("second two-channel preload script.message realm");
    assert_eq!(
        two_channel_events[0]["params"],
        json!({
            "channel": "channel_name_1",
            "data": {"type": "string", "value": "message_from_channel_1"},
            "source": {
                "realm": first_realm,
                "context": two_channel_context
            }
        })
    );
    assert_eq!(
        two_channel_events[1]["params"],
        json!({
            "channel": "channel_name_2",
            "data": {"type": "string", "value": "message_from_channel_2"},
            "source": {
                "realm": second_realm,
                "context": two_channel_context
            }
        })
    );
    remove_bidi_preload_script(&mut socket, 14, &two_channel_script).await;

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_preload_channel_mutation_observer_matches_wpt_shape() {
    // Mirrors webdriver/tests/bidi/script/add_preload_script/arguments.py::test_mutation_observer.
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let preload_script = send_bidi_add_preload_script(
        &mut socket,
        4,
        "(channel) => {
            const onMutation = (mutationList) => mutationList.forEach((mutation) => {
                const attributeName = mutation.attributeName;
                const newValue = mutation.target.getAttribute(mutation.attributeName);
                channel({ attributeName, newValue });
            });
            const observer = new MutationObserver(onMutation);
            observer.observe(document, { attributes: true, subtree: true });
        }",
        vec![json!({
            "type": "channel",
            "value": {
                "channel": "channel_name"
            }
        })],
    )
    .await;

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": "data:text/html,<div class='old class name'>foo</div>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate");
    let mut messages = recv_until_id(&mut socket, 5).await;
    let navigate = bidi_message_by_id(&messages, 5);
    assert_eq!(
        navigate["type"],
        json!("success"),
        "navigation should succeed before mutation observer event: {messages:#?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "document.querySelector('div').setAttribute('class', 'mutated')",
                    "target": {
                        "context": context_id.clone()
                    },
                    "awaitPromise": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send mutation script.evaluate");
    messages.extend(recv_until_id(&mut socket, 6).await);
    let messages =
        collect_bidi_messages_until_method_count(&mut socket, messages, "script.message", 1).await;
    let evaluate = bidi_message_by_id(&messages, 6);
    assert_eq!(evaluate["type"], json!("success"));
    let realm = evaluate["result"]["realm"]
        .as_str()
        .expect("mutation evaluate realm");
    let event = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("mutation observer script.message event");
    assert_eq!(
        event["params"],
        json!({
            "channel": "channel_name",
            "data": {
                "type": "object",
                "value": [
                    ["attributeName", {"type": "string", "value": "class"}],
                    ["newValue", {"type": "string", "value": "mutated"}]
                ]
            },
            "source": {
                "realm": realm,
                "context": context_id
            }
        })
    );
    remove_bidi_preload_script(&mut socket, 7, &preload_script).await;

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_preload_channel_observes_payload_mutation_before_serialization() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));
    let subscribe = send_bidi_command(
        &mut socket,
        2,
        "session.subscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let add_preload = send_bidi_command(
        &mut socket,
        3,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "(channel) => {
                const payload = { foo: 'before', nested: { value: 1 }, list: ['a'] };
                channel(payload);
                payload.foo = 'after';
                payload.nested.value = 2;
                payload.list.push('b');
            }",
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "preload_mutation_channel"
                }
            }]
        }),
    )
    .await;
    assert_eq!(
        add_preload["type"],
        json!("success"),
        "addPreloadScript should succeed: {add_preload:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
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
    let create_messages = recv_until_id(&mut socket, 4).await;
    let messages =
        collect_bidi_messages_until_method_count(&mut socket, create_messages, "script.message", 1)
            .await;
    let create = bidi_message_by_id(&messages, 4);
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id");
    let event = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("preload mutation script.message event");
    let realm = event["params"]["source"]["realm"]
        .as_str()
        .expect("preload mutation script.message realm");
    assert_eq!(
        event["params"],
        json!({
            "channel": "preload_mutation_channel",
            "data": {
                "type": "object",
                "value": [
                    ["foo", {"type": "string", "value": "after"}],
                    [
                        "nested",
                        {
                            "type": "object",
                            "value": [["value", {"type": "number", "value": 2}]]
                        }
                    ],
                    [
                        "list",
                        {
                            "type": "array",
                            "value": [
                                {"type": "string", "value": "a"},
                                {"type": "string", "value": "b"}
                            ]
                        }
                    ]
                ]
            },
            "source": {
                "realm": realm,
                "context": context_id
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_preload_channel_handoff_proxy_is_not_page_forgeable_after_start() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    let session = send_bidi_command(&mut socket, 1, "session.new", json!({})).await;
    assert_eq!(session["type"], json!("success"));
    let subscribe = send_bidi_command(
        &mut socket,
        2,
        "session.subscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let add_preload = send_bidi_command(
        &mut socket,
        3,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "(channel) => channel('legit')",
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "preload_cleanup_channel"
                }
            }]
        }),
    )
    .await;
    assert_eq!(add_preload["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
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
    let create_messages = recv_until_id(&mut socket, 4).await;
    let messages =
        collect_bidi_messages_until_method_count(&mut socket, create_messages, "script.message", 1)
            .await;
    let create = bidi_message_by_id(&messages, 4);
    assert_eq!(create["type"], json!("success"));
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();
    let event = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("legit preload script.message event");
    assert_eq!(
        event["params"]["data"],
        json!({"type": "string", "value": "legit"})
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
	                "method": "script.evaluate",
	                "params": {
	                    "expression": "(() => {
	                        const handoffs = Object.getOwnPropertyNames(globalThis)
	                            .filter((name) => name.indexOf('__lmBidiPreloadChannel_') === 0);
	                        for (const name of handoffs) {
	                            const take = globalThis[name];
	                            if (typeof take === 'function') {
	                                const proxy = take('wrong-token');
	                                if (proxy && typeof proxy.sendMessage === 'function') {
	                                    proxy.sendMessage('forged');
	                                }
	                            }
	                        }
	                        return JSON.stringify({handoffs, legacyRegistry: typeof globalThis.__moliBidiPreloadChannelRegistry});
	                    })()",
                    "target": {
                        "context": context_id
                    },
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send registry forge probe");
    let probe_messages = recv_until_id(&mut socket, 5).await;
    assert!(
        bidi_events_by_method(&probe_messages, "script.message").is_empty(),
        "fixed preload registry probe should not produce forged script.message: {probe_messages:#?}"
    );
    let probe = bidi_message_by_id(&probe_messages, 5);
    assert_eq!(probe["type"], json!("success"));
    let probe_json: serde_json::Value = serde_json::from_str(
        probe["result"]["result"]["value"]
            .as_str()
            .expect("probe JSON string"),
    )
    .expect("probe JSON should decode");
    assert_eq!(probe_json["legacyRegistry"], json!("undefined"));
    assert_eq!(
        probe_json["handoffs"],
        json!([]),
        "preload handoff globals should be deleted after listener start"
    );
    match timeout(Duration::from_millis(200), recv_ws_json(&mut socket)).await {
        Ok(message) if message["method"] == json!("script.message") => {
            panic!("unexpected forged script.message after registry probe: {message:#?}")
        }
        _ => {}
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_preload_channel_handoff_is_token_gated_during_page_script() {
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let add_preload = send_bidi_command(
        &mut socket,
        4,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "(channel) => channel('legit-during-navigation')",
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "preload_token_gate_channel"
                }
            }]
        }),
    )
    .await;
    assert_eq!(add_preload["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id,
	                    "url": "data:text/html,<script>window.__handoffProbe=[];for(const name of Object.getOwnPropertyNames(globalThis)){if(name.indexOf('__lmBidiPreloadChannel_')===0){const take=globalThis[name];let wrongTokenValue;let directSendValue;try{wrongTokenValue=take('wrong-token');}catch(error){wrongTokenValue=String(error);}try{if(take&&typeof take.sendMessage==='function'){directSendValue=take.sendMessage('forged');}}catch(error){directSendValue=String(error);}window.__handoffProbe.push([typeof take,wrongTokenValue,directSendValue]);}}</script><main>preload-token-gate</main>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send browsingContext.navigate");
    let navigate_messages = recv_until_id(&mut socket, 5).await;
    let messages = collect_bidi_messages_until_method_count(
        &mut socket,
        navigate_messages,
        "script.message",
        1,
    )
    .await;
    let script_messages = bidi_events_by_method(&messages, "script.message");
    assert_eq!(
        script_messages.len(),
        1,
        "wrong-token page probes should not forge script.message events: {messages:#?}"
    );
    assert_eq!(
        script_messages[0]["params"],
        json!({
            "channel": "preload_token_gate_channel",
            "data": {
                "type": "string",
                "value": "legit-during-navigation"
            },
            "source": {
                "context": context_id,
                "realm": script_messages[0]["params"]["source"]["realm"]
                    .as_str()
                    .expect("script.message source realm")
            }
        })
    );
    match timeout(Duration::from_millis(200), recv_ws_json(&mut socket)).await {
        Ok(message) if message["method"] == json!("script.message") => {
            panic!("unexpected forged script.message after wrong-token probe: {message:#?}")
        }
        _ => {}
    }

    let probe = send_bidi_command(
        &mut socket,
        6,
        "script.evaluate",
        json!({
	            "expression": "JSON.stringify({legacyRegistry: typeof globalThis.__moliBidiPreloadChannelRegistry, handoffGlobals: Object.getOwnPropertyNames(globalThis).filter((name) => name.indexOf('__lmBidiPreloadChannel_') === 0), probe: window.__handoffProbe, forged: window.__forgedMessage})",
            "target": {
                "context": context_id
            },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(probe["type"], json!("success"));
    let probe_json: serde_json::Value = serde_json::from_str(
        probe["result"]["result"]["value"]
            .as_str()
            .expect("probe JSON string"),
    )
    .expect("probe JSON should decode");
    assert_eq!(probe_json["legacyRegistry"], json!("undefined"));
    assert_eq!(
        probe_json["handoffGlobals"],
        json!([]),
        "transient preload handoff should be deleted after listener starts: {probe_json:#?}"
    );
    assert_eq!(probe_json["forged"], serde_json::Value::Null);
    let probe_items = probe_json["probe"]
        .as_array()
        .expect("handoff probe should serialize an array");
    assert!(
        !probe_items.is_empty(),
        "page should see only token-gated handoff functions, not the old proxy registry: {probe_json:#?}"
    );
    for item in probe_items {
        assert_eq!(item[0], json!("function"));
        assert_eq!(item[1], serde_json::Value::Null);
        assert_eq!(item[2], serde_json::Value::Null);
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_preload_channel_emits_after_wait_none_navigation() {
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let add_preload = send_bidi_command(
        &mut socket,
        4,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "(channel) => channel('wait-none-preload')",
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "wait_none_channel"
                }
            }],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(
        add_preload["type"],
        json!("success"),
        "addPreloadScript should succeed: {add_preload:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": "data:text/html,<body>wait-none-preload</body>",
                    "wait": "none"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send wait=none browsingContext.navigate");
    let navigate_messages = recv_until_id(&mut socket, 5).await;
    let messages = collect_bidi_messages_until_method_count(
        &mut socket,
        navigate_messages,
        "script.message",
        1,
    )
    .await;
    let navigate = bidi_message_by_id(&messages, 5);
    assert_eq!(
        navigate["type"],
        json!("success"),
        "wait=none navigation should return successfully: {messages:#?}"
    );
    let script_message = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("wait=none preload script.message event");
    let realm = script_message["params"]["source"]["realm"]
        .as_str()
        .expect("wait=none preload script.message realm");
    assert_eq!(
        script_message["params"],
        json!({
            "channel": "wait_none_channel",
            "data": {
                "type": "string",
                "value": "wait-none-preload"
            },
            "source": {
                "realm": realm,
                "context": context_id
            }
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_context_preload_channel_survives_frame_navigation() {
    // Reduced from Selenium Python bidi_script_tests.py preload channel cases,
    // with a child frame to cover default-world preload replay.
    async fn parent() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html>
<head><title>Preload Frame Parent</title></head>
<body><main>parent</main><iframe src="/child"></iframe></body>
</html>"#,
        )
    }

    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Preload Frame Child</title><main>child</main>",
        )
    }

    let fixture_app = Router::new()
        .route("/parent", get(parent))
        .route("/child", get(child));
    let (fixture_addr, _fixture_server) =
        spawn_dedicated_fixture_server(fixture_app, "bidi-preload-channel-frame");
    let parent_url = format!("http://{fixture_addr}/parent");

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
    assert_eq!(
        create["type"],
        json!("success"),
        "browsingContext.create should succeed: {create:?}"
    );
    let context_id = create["result"]["context"]
        .as_str()
        .expect("created context id")
        .to_owned();
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({ "events": ["script.message"] }),
    )
    .await;
    assert_eq!(
        subscribe["type"],
        json!("success"),
        "session.subscribe should succeed: {subscribe:?}"
    );
    let add_preload = send_bidi_command(
        &mut socket,
        4,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "(channel) => { channel('preload:' + location.pathname); globalThis.__bidiPreloadChannel = 'received'; }",
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "frame_channel",
                    "ownership": "none"
                }
            }],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(
        add_preload["type"],
        json!("success"),
        "addPreloadScript should succeed: {add_preload:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
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
        .expect("send frame browsingContext.navigate");
    let navigation_messages = recv_until_id(&mut socket, 5).await;
    let navigation_messages = collect_bidi_messages_until(
        &mut socket,
        navigation_messages,
        |messages| {
            let script_messages = bidi_events_by_method(messages, "script.message");
            script_messages.iter().any(|message| {
                message["params"]["channel"] == json!("frame_channel")
                    && message["params"]["data"]
                        == json!({
                            "type": "string",
                            "value": "preload:/parent"
                        })
            }) && script_messages.iter().any(|message| {
                message["params"]["channel"] == json!("frame_channel")
                    && message["params"]["data"]
                        == json!({
                            "type": "string",
                            "value": "preload:/child"
                        })
            })
        },
        "top-frame and child-frame preload script.message events",
    )
    .await;
    let navigate = bidi_message_by_id(&navigation_messages, 5);
    assert_eq!(
        navigate["type"],
        json!("success"),
        "frame navigation should succeed without renderer abort: {navigation_messages:#?}"
    );
    let top_script_message = bidi_events_by_method(&navigation_messages, "script.message")
        .into_iter()
        .find(|message| {
            message["params"]["channel"] == json!("frame_channel")
                && message["params"]["data"]
                    == json!({
                        "type": "string",
                        "value": "preload:/parent"
                    })
        })
        .unwrap_or_else(|| {
            panic!("expected top-frame preload script.message: {navigation_messages:#?}")
        });
    assert_eq!(
        top_script_message["params"]["source"]["context"],
        json!(context_id)
    );
    assert!(
        top_script_message["params"]["source"]["realm"]
            .as_str()
            .is_some_and(|realm| !realm.is_empty()),
        "top script.message should include a realm: {top_script_message:?}"
    );
    let child_script_message = bidi_events_by_method(&navigation_messages, "script.message")
        .into_iter()
        .find(|message| {
            message["params"]["channel"] == json!("frame_channel")
                && message["params"]["data"]
                    == json!({
                        "type": "string",
                        "value": "preload:/child"
                    })
        })
        .unwrap_or_else(|| {
            panic!("expected child-frame preload script.message: {navigation_messages:#?}")
        });
    let child_context = child_script_message["params"]["source"]["context"]
        .as_str()
        .expect("child script.message context");
    assert_ne!(
        child_context, context_id,
        "child script.message should report the iframe browsing context"
    );
    assert!(
        child_script_message["params"]["source"]["realm"]
            .as_str()
            .is_some_and(|realm| !realm.is_empty()),
        "child script.message should include a realm: {child_script_message:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "globalThis.__bidiPreloadChannel",
                    "target": {
                        "context": context_id
                    }
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send preload channel marker script.evaluate");
    let evaluate_messages = recv_until_id(&mut socket, 6).await;
    let evaluate = bidi_message_by_id(&evaluate_messages, 6);
    assert_eq!(
        evaluate["type"],
        json!("success"),
        "preload marker evaluate should succeed: {evaluate_messages:#?}"
    );
    assert_eq!(
        evaluate["result"]["result"],
        json!({
            "type": "string",
            "value": "received"
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_sandbox_preload_channel_reports_sandbox_realm() {
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
    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["script.message"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));
    let add_preload = send_bidi_command(
        &mut socket,
        4,
        "script.addPreloadScript",
        json!({
            "functionDeclaration": "(channel) => { globalThis.__bidiSandboxPreload = 'sandbox'; channel('sandbox-preload'); }",
            "arguments": [{
                "type": "channel",
                "value": {
                    "channel": "sandbox_channel"
                }
            }],
            "contexts": [context_id.clone()],
            "sandbox": "sandbox-channel"
        }),
    )
    .await;
    assert_eq!(
        add_preload["type"],
        json!("success"),
        "sandbox addPreloadScript should succeed: {add_preload:?}"
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.navigate",
                "params": {
                    "context": context_id.clone(),
                    "url": "data:text/html,<body>sandbox-preload</body>",
                    "wait": "complete"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send sandbox preload browsingContext.navigate");
    let navigation_messages = recv_until_id(&mut socket, 5).await;
    let messages = collect_bidi_messages_until_method_count(
        &mut socket,
        navigation_messages,
        "script.message",
        1,
    )
    .await;
    let navigate = bidi_message_by_id(&messages, 5);
    assert_eq!(
        navigate["type"],
        json!("success"),
        "sandbox preload navigation should succeed: {messages:#?}"
    );
    let script_message = bidi_events_by_method(&messages, "script.message")
        .pop()
        .expect("sandbox preload script.message event");
    let script_message_realm = script_message["params"]["source"]["realm"]
        .as_str()
        .expect("sandbox script.message realm")
        .to_owned();
    assert_eq!(
        script_message["params"],
        json!({
            "channel": "sandbox_channel",
            "data": {
                "type": "string",
                "value": "sandbox-preload"
            },
            "source": {
                "realm": script_message_realm,
                "context": context_id
            }
        })
    );

    let realms = send_bidi_command(
        &mut socket,
        6,
        "script.getRealms",
        json!({ "context": context_id }),
    )
    .await;
    assert_eq!(
        realms["type"],
        json!("success"),
        "script.getRealms should succeed: {realms:?}"
    );
    let default_realm = bidi_window_realm(&realms, &context_id)["realm"]
        .as_str()
        .expect("default realm id")
        .to_owned();
    let sandbox_realm = bidi_sandbox_window_realm(&realms, &context_id, "sandbox-channel")["realm"]
        .as_str()
        .expect("sandbox realm id")
        .to_owned();
    assert_ne!(
        default_realm, sandbox_realm,
        "sandbox preload should use a distinct realm"
    );
    assert_eq!(
        script_message_realm, sandbox_realm,
        "script.message source realm should identify the sandbox realm"
    );

    let default_probe = send_bidi_command(
        &mut socket,
        7,
        "script.evaluate",
        json!({
            "expression": "globalThis.__bidiSandboxPreload",
            "target": {
                "context": context_id
            }
        }),
    )
    .await;
    assert_eq!(default_probe["type"], json!("success"));
    assert_eq!(
        default_probe["result"]["result"],
        json!({ "type": "undefined" }),
        "sandbox preload globals must not leak into the default realm"
    );
    let sandbox_probe = send_bidi_command(
        &mut socket,
        8,
        "script.evaluate",
        json!({
            "expression": "globalThis.__bidiSandboxPreload",
            "target": {
                "context": context_id,
                "sandbox": "sandbox-channel"
            }
        }),
    )
    .await;
    assert_eq!(sandbox_probe["type"], json!("success"));
    assert_eq!(
        sandbox_probe["result"]["result"],
        json!({
            "type": "string",
            "value": "sandbox"
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_session_route_reports_invalid_json() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
        .await
        .expect("connect to BiDi websocket");

    socket
        .send(WsMessage::Text("{".into()))
        .await
        .expect("send invalid JSON");
    let invalid = recv_ws_json(&mut socket).await;
    assert_eq!(invalid["type"], json!("error"));
    assert_eq!(invalid["error"], json!("invalid argument"));
    assert!(invalid.get("id").is_none());

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_print_invalid_parameters_match_wpt_error_shape() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/print/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 10_u64;

    for params in [
        json!({"context": false}),
        json!({"context": 42}),
        json!({"context": {}}),
        json!({"context": []}),
        json!({"context": context_id, "background": "foo"}),
        json!({"context": context_id, "background": 42}),
        json!({"context": context_id, "margin": false}),
        json!({"context": context_id, "margin": {"top": "foo"}}),
        json!({"context": context_id, "margin": {"bottom": -0.1}}),
        json!({"context": context_id, "orientation": false}),
        json!({"context": context_id, "orientation": "foo"}),
        json!({"context": context_id, "page": "foo"}),
        json!({"context": context_id, "page": {"height": false}}),
        json!({"context": context_id, "page": {"width": 0.03}}),
        json!({"context": context_id, "pageRanges": false}),
        json!({"context": context_id, "pageRanges": [null]}),
        json!({"context": context_id, "pageRanges": ["3-2"]}),
        json!({"context": context_id, "pageRanges": ["1-2-3"]}),
        json!({"context": context_id, "scale": false}),
        json!({"context": context_id, "scale": 0.09}),
        json!({"context": context_id, "scale": 2.01}),
        json!({"context": context_id, "shrinkToFit": "foo"}),
    ] {
        id += 1;
        let response =
            send_bidi_command(&mut socket, id, "browsingContext.print", params.clone()).await;
        assert_eq!(
            response["type"],
            json!("error"),
            "params should fail: {params}"
        );
        assert_eq!(
            response["error"],
            json!("invalid argument"),
            "params should be invalid argument: {params}; response={response:?}"
        );
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_capture_screenshot_invalid_parameters_and_unsupported_boundary() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/capture_screenshot/invalid.py.
    // Unknown element clips would be `no such node` once real screenshots exist, but the
    // current product boundary is to fail all valid screenshot requests as unsupported.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 100_u64;

    for params in [
        json!({"context": null}),
        json!({"context": false}),
        json!({"context": 42}),
        json!({"context": {}}),
        json!({"context": []}),
        json!({"context": context_id, "clip": false}),
        json!({"context": context_id, "clip": {"type": null}}),
        json!({"context": context_id, "clip": {"type": "foo"}}),
        json!({"context": context_id, "clip": {"type": "box", "x": "foo", "y": 0, "width": 1, "height": 1}}),
        json!({"context": context_id, "clip": {"type": "box", "x": 0, "y": false, "width": 1, "height": 1}}),
        json!({"context": context_id, "clip": {"type": "box", "x": 0, "y": 0, "width": [], "height": 1}}),
        json!({"context": context_id, "clip": {"type": "box", "x": 0, "y": 0, "width": 1, "height": {}}}),
        json!({"context": context_id, "origin": 42}),
        json!({"context": context_id, "origin": "page"}),
        json!({"context": context_id, "format": "foo"}),
        json!({"context": context_id, "format": {}}),
        json!({"context": context_id, "format": {"type": null}}),
        json!({"context": context_id, "format": {"type": "image/jpeg", "quality": "foo"}}),
        json!({"context": context_id, "format": {"type": "image/jpeg", "quality": -0.1}}),
        json!({"context": context_id, "format": {"type": "image/jpeg", "quality": 1.1}}),
    ] {
        id += 1;
        let response = send_bidi_command(
            &mut socket,
            id,
            "browsingContext.captureScreenshot",
            params.clone(),
        )
        .await;
        assert_eq!(
            response["type"],
            json!("error"),
            "params should fail: {params}"
        );
        assert_eq!(
            response["error"],
            json!("invalid argument"),
            "params should be invalid argument: {params}; response={response:?}"
        );
    }

    let unknown_element_clip = send_bidi_command(
        &mut socket,
        id + 1,
        "browsingContext.captureScreenshot",
        json!({
            "context": context_id,
            "clip": {
                "type": "element",
                "element": {
                    "sharedId": "foo"
                }
            }
        }),
    )
    .await;
    assert_eq!(unknown_element_clip["type"], json!("error"));
    assert_eq!(
        unknown_element_clip["error"],
        json!("unsupported operation"),
        "element clip should fail at the unsupported screenshot boundary: {unknown_element_clip:?}"
    );
    assert_eq!(
        unknown_element_clip["message"],
        json!("Page.captureScreenshot is not supported: renderer screenshots are not implemented.")
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

#[tokio::test]
async fn websocket_bidi_user_prompt_opened_handler_capabilities_match_wpt() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/user_prompt_opened/handler.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;

    for (offset, (capability, expected_handler)) in [
        ("accept", "accept"),
        ("accept and notify", "accept"),
        ("dismiss", "dismiss"),
        ("dismiss and notify", "dismiss"),
        ("ignore", "ignore"),
    ]
    .into_iter()
    .enumerate()
    {
        let base_id = (offset as u64) * 10;
        let (mut socket, _) = connect_async(format!("ws://{cdp_addr}/session"))
            .await
            .expect("connect to BiDi websocket");
        let session = send_bidi_command(
            &mut socket,
            base_id + 1,
            "session.new",
            json!({
                "capabilities": {
                    "unhandledPromptBehavior": capability
                }
            }),
        )
        .await;
        assert_eq!(session["type"], json!("success"));

        let create = send_bidi_command(
            &mut socket,
            base_id + 2,
            "browsingContext.create",
            json!({ "type": "tab" }),
        )
        .await;
        assert_eq!(create["type"], json!("success"));
        let context_id = create["result"]["context"]
            .as_str()
            .expect("created context id")
            .to_owned();

        let subscribe = send_bidi_command(
            &mut socket,
            base_id + 3,
            "session.subscribe",
            json!({
                "events": ["browsingContext.userPromptOpened"],
                "contexts": [context_id]
            }),
        )
        .await;
        assert_eq!(subscribe["type"], json!("success"));

        socket
            .send(WsMessage::Text(
                json!({
                    "id": base_id + 4,
                    "method": "script.evaluate",
                    "params": {
                        "expression": "window.alert('handler check')",
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
            .expect("send alert script.evaluate");
        let initial_messages = recv_until_id(&mut socket, base_id + 4).await;
        let messages = collect_bidi_messages_until_method_count(
            &mut socket,
            initial_messages,
            "browsingContext.userPromptOpened",
            1,
        )
        .await;
        assert_eq!(
            bidi_message_by_id(&messages, base_id + 4)["type"],
            json!("success")
        );
        let event = bidi_events_by_method(&messages, "browsingContext.userPromptOpened")[0];
        assert_eq!(
            event["params"],
            json!({
                "context": context_id,
                "type": "alert",
                "message": "handler check",
                "handler": expected_handler
            }),
            "unhandledPromptBehavior={capability:?} should surface handler={expected_handler:?}"
        );

        let close_prompt = send_bidi_command(
            &mut socket,
            base_id + 5,
            "browsingContext.handleUserPrompt",
            json!({ "context": context_id }),
        )
        .await;
        assert_eq!(close_prompt["type"], json!("success"));
        let _ = socket.close(None).await;
    }

    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_handle_user_prompt_emits_wpt_prompt_events() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/handle_user_prompt/handle_user_prompt.py
    // and browsing_context/user_prompt_{opened,closed}/user_prompt_*.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let subscribe = send_bidi_command(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": [
                "browsingContext.userPromptOpened",
                "browsingContext.userPromptClosed"
            ]
        }),
    )
    .await;
    assert_eq!(subscribe["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 4_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "window.alert('bidi alert')",
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
        .expect("send alert script.evaluate");
    let initial_alert_messages = recv_until_id(&mut socket, 4).await;
    let alert_messages = collect_bidi_messages_until_method_count(
        &mut socket,
        initial_alert_messages,
        "browsingContext.userPromptOpened",
        1,
    )
    .await;
    assert_eq!(
        bidi_message_by_id(&alert_messages, 4)["type"],
        json!("success")
    );
    let alert_opened =
        bidi_events_by_method(&alert_messages, "browsingContext.userPromptOpened")[0];
    assert_eq!(
        alert_opened["params"],
        json!({
            "context": context_id,
            "type": "alert",
            "message": "bidi alert",
            "handler": "dismiss"
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "browsingContext.handleUserPrompt",
                "params": {
                    "context": context_id
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send alert handleUserPrompt");
    let initial_alert_closed_messages = recv_until_id(&mut socket, 5).await;
    let alert_closed_messages = collect_bidi_messages_until_method_count(
        &mut socket,
        initial_alert_closed_messages,
        "browsingContext.userPromptClosed",
        1,
    )
    .await;
    assert_eq!(
        bidi_message_by_id(&alert_closed_messages, 5)["result"],
        json!({})
    );
    let alert_closed =
        bidi_events_by_method(&alert_closed_messages, "browsingContext.userPromptClosed")[0];
    assert_eq!(
        alert_closed["params"],
        json!({
            "context": context_id,
            "accepted": true,
            "type": "alert"
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 6_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "window.prompt('Enter Your Name: ')",
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
        .expect("send prompt script.evaluate");
    let initial_prompt_messages = recv_until_id(&mut socket, 6).await;
    let prompt_messages = collect_bidi_messages_until_method_count(
        &mut socket,
        initial_prompt_messages,
        "browsingContext.userPromptOpened",
        1,
    )
    .await;
    assert_eq!(
        bidi_message_by_id(&prompt_messages, 6)["type"],
        json!("success")
    );
    let prompt_opened =
        bidi_events_by_method(&prompt_messages, "browsingContext.userPromptOpened")[0];
    assert_eq!(
        prompt_opened["params"],
        json!({
            "context": context_id,
            "type": "prompt",
            "message": "Enter Your Name: ",
            "handler": "dismiss",
            "defaultValue": ""
        })
    );

    socket
        .send(WsMessage::Text(
            json!({
                "id": 7_u64,
                "method": "browsingContext.handleUserPrompt",
                "params": {
                    "context": context_id,
                    "accept": true,
                    "userText": "Test"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send prompt handleUserPrompt");
    let initial_prompt_closed_messages = recv_until_id(&mut socket, 7).await;
    let prompt_closed_messages = collect_bidi_messages_until_method_count(
        &mut socket,
        initial_prompt_closed_messages,
        "browsingContext.userPromptClosed",
        1,
    )
    .await;
    assert_eq!(
        bidi_message_by_id(&prompt_closed_messages, 7)["type"],
        json!("success")
    );
    let prompt_closed =
        bidi_events_by_method(&prompt_closed_messages, "browsingContext.userPromptClosed")[0];
    assert_eq!(
        prompt_closed["params"],
        json!({
            "context": context_id,
            "accepted": true,
            "type": "prompt",
            "userText": "Test"
        })
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_handle_user_prompt_invalid_parameters_match_wpt_error_shape() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/handle_user_prompt/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 100_u64;

    for params in [
        json!({"context": null}),
        json!({"context": false}),
        json!({"context": 42}),
        json!({"context": {}}),
        json!({"context": []}),
        json!({"context": context_id, "accept": "foo"}),
        json!({"context": context_id, "accept": 42}),
        json!({"context": context_id, "accept": {}}),
        json!({"context": context_id, "accept": []}),
        json!({"context": context_id, "userText": false}),
        json!({"context": context_id, "userText": 42}),
        json!({"context": context_id, "userText": {}}),
        json!({"context": context_id, "userText": []}),
    ] {
        id += 1;
        let response = send_bidi_command(
            &mut socket,
            id,
            "browsingContext.handleUserPrompt",
            params.clone(),
        )
        .await;
        assert_bidi_error(
            &response,
            "invalid argument",
            &format!("handleUserPrompt params should be invalid argument: {params}"),
        );
    }

    for params in [json!({"context": ""}), json!({"context": "somestring"})] {
        id += 1;
        let response = send_bidi_command(
            &mut socket,
            id,
            "browsingContext.handleUserPrompt",
            params.clone(),
        )
        .await;
        assert_bidi_error(
            &response,
            "no such frame",
            &format!("handleUserPrompt context should be missing: {params}"),
        );
    }

    let no_alert = send_bidi_command(
        &mut socket,
        id + 1,
        "browsingContext.handleUserPrompt",
        json!({"context": context_id}),
    )
    .await;
    assert_bidi_error(
        &no_alert,
        "no such alert",
        "handleUserPrompt should reject when no dialog is showing",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_browsing_context_context_invalid_values_match_wpt_error_shape() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/create/invalid.py,
    // close/invalid.py, activate/invalid.py, and traverse_history/invalid.py.
    async fn child() -> impl IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Child Context</title><main>child-context</main>",
        )
    }

    let child_app = Router::new().route("/child", get(child));
    let (child_addr, _child_server) =
        spawn_dedicated_fixture_server(child_app, "bidi-invalid-context-child");
    let child_url = format!("http://{child_addr}/child");

    let parent_html = format!(
        r#"<!doctype html>
<html>
<head><title>Parent Context</title></head>
<body><main>parent-context</main><iframe src="{child_url}"></iframe></body>
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
        spawn_dedicated_fixture_server(parent_app, "bidi-invalid-context-parent");
    let parent_url = format!("http://{parent_addr}/");

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 500_u64;

    let missing_reference = send_bidi_command(
        &mut socket,
        id,
        "browsingContext.create",
        json!({"type": "tab", "referenceContext": "missing-frame-context"}),
    )
    .await;
    assert_bidi_error(
        &missing_reference,
        "no such frame",
        "create referenceContext should reject unknown context",
    );
    id += 1;

    for method in ["browsingContext.close", "browsingContext.activate"] {
        let missing = send_bidi_command(
            &mut socket,
            id,
            method,
            json!({"context": "missing-frame-context"}),
        )
        .await;
        assert_bidi_error(
            &missing,
            "no such frame",
            &format!("{method} should reject unknown context"),
        );
        id += 1;
    }

    let missing_traverse = send_bidi_command(
        &mut socket,
        id,
        "browsingContext.traverseHistory",
        json!({"context": "missing-frame-context", "delta": 1}),
    )
    .await;
    assert_bidi_error(
        &missing_traverse,
        "no such frame",
        "traverseHistory should reject unknown context",
    );
    id += 1;

    let navigate = send_bidi_command(
        &mut socket,
        id,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": parent_url,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"), "{navigate:?}");
    id += 1;

    let tree = send_bidi_command(
        &mut socket,
        id,
        "browsingContext.getTree",
        json!({"root": context_id.clone()}),
    )
    .await;
    assert_eq!(tree["type"], json!("success"), "{tree:?}");
    let child_context_id = tree["result"]["contexts"][0]["children"][0]["context"]
        .as_str()
        .expect("iframe context id")
        .to_owned();
    id += 1;

    let iframe_reference = send_bidi_command(
        &mut socket,
        id,
        "browsingContext.create",
        json!({"type": "tab", "referenceContext": child_context_id.clone()}),
    )
    .await;
    assert_bidi_error(
        &iframe_reference,
        "invalid argument",
        "create referenceContext should reject iframe context",
    );
    id += 1;

    for method in ["browsingContext.close", "browsingContext.activate"] {
        let iframe_context = send_bidi_command(
            &mut socket,
            id,
            method,
            json!({"context": child_context_id.clone()}),
        )
        .await;
        assert_bidi_error(
            &iframe_context,
            "invalid argument",
            &format!("{method} should reject iframe context"),
        );
        id += 1;
    }

    let iframe_traverse = send_bidi_command(
        &mut socket,
        id,
        "browsingContext.traverseHistory",
        json!({"context": child_context_id, "delta": -1}),
    )
    .await;
    assert_bidi_error(
        &iframe_traverse,
        "invalid argument",
        "traverseHistory should reject iframe context",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_get_tree_invalid_parameters_match_wpt_error_shape() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/get_tree/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, _context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 300_u64;

    for params in [
        json!({"maxDepth": false}),
        json!({"maxDepth": "foo"}),
        json!({"maxDepth": {}}),
        json!({"maxDepth": []}),
        json!({"maxDepth": -1}),
        json!({"maxDepth": 1.1}),
        json!({"maxDepth": 9_007_199_254_740_992_u64}),
        json!({"root": false}),
        json!({"root": 42}),
        json!({"root": {}}),
        json!({"root": []}),
    ] {
        id += 1;
        let response =
            send_bidi_command(&mut socket, id, "browsingContext.getTree", params.clone()).await;
        assert_bidi_error(
            &response,
            "invalid argument",
            &format!("getTree params should be invalid argument: {params}"),
        );
    }

    let response = send_bidi_command(
        &mut socket,
        id + 1,
        "browsingContext.getTree",
        json!({"root": "foo"}),
    )
    .await;
    assert_bidi_error(&response, "no such frame", "getTree root should be missing");

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_set_viewport_invalid_parameters_match_wpt_error_shape() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/browsing_context/set_viewport/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 400_u64;

    for params in [
        json!({"context": false, "viewport": {"width": 100, "height": 200}}),
        json!({"context": 42, "viewport": {"width": 100, "height": 200}}),
        json!({"context": {}, "viewport": {"width": 100, "height": 200}}),
        json!({"context": [], "viewport": {"width": 100, "height": 200}}),
        json!({"context": context_id, "viewport": false}),
        json!({"context": context_id, "viewport": 42}),
        json!({"context": context_id, "viewport": ""}),
        json!({"context": context_id, "viewport": {}}),
        json!({"context": context_id, "viewport": []}),
        json!({"context": context_id, "viewport": {"width": 100}}),
        json!({"context": context_id, "viewport": {"height": 100}}),
        json!({"context": context_id, "viewport": {"width": null, "height": 100}}),
        json!({"context": context_id, "viewport": {"width": false, "height": 100}}),
        json!({"context": context_id, "viewport": {"width": "", "height": 100}}),
        json!({"context": context_id, "viewport": {"width": 42.1, "height": 100}}),
        json!({"context": context_id, "viewport": {"width": {}, "height": 100}}),
        json!({"context": context_id, "viewport": {"width": [], "height": 100}}),
        json!({"context": context_id, "viewport": {"width": 100, "height": null}}),
        json!({"context": context_id, "viewport": {"width": 100, "height": false}}),
        json!({"context": context_id, "viewport": {"width": 100, "height": ""}}),
        json!({"context": context_id, "viewport": {"width": 100, "height": 42.1}}),
        json!({"context": context_id, "viewport": {"width": 100, "height": {}}}),
        json!({"context": context_id, "viewport": {"width": 100, "height": []}}),
        json!({"context": context_id, "viewport": {"width": -1, "height": 100}}),
        json!({"context": context_id, "viewport": {"width": 100, "height": -1}}),
        json!({"context": context_id, "viewport": {"width": -1, "height": -1}}),
        json!({"context": context_id, "viewport": null, "devicePixelRatio": false}),
        json!({"context": context_id, "viewport": null, "devicePixelRatio": ""}),
        json!({"context": context_id, "viewport": null, "devicePixelRatio": {}}),
        json!({"context": context_id, "viewport": null, "devicePixelRatio": []}),
        json!({"context": context_id, "viewport": null, "devicePixelRatio": 0}),
        json!({"context": context_id, "viewport": null, "devicePixelRatio": -1}),
        json!({"userContexts": true, "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": "foo", "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": 42, "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": {}, "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": [], "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": [null], "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": [false], "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": [42], "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": [{}], "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": [[]], "viewport": {"width": 100, "height": 200}}),
        json!({
            "context": context_id,
            "userContexts": ["default"],
            "viewport": {"width": 100, "height": 200}
        }),
        json!({"viewport": {"width": 100, "height": 200}}),
    ] {
        id += 1;
        let response = send_bidi_command(
            &mut socket,
            id,
            "browsingContext.setViewport",
            params.clone(),
        )
        .await;
        assert_bidi_error(
            &response,
            "invalid argument",
            &format!("setViewport params should be invalid argument: {params}"),
        );
    }

    let response = send_bidi_command(
        &mut socket,
        id + 1,
        "browsingContext.setViewport",
        json!({"context": "_invalid_"}),
    )
    .await;
    assert_bidi_error(
        &response,
        "no such frame",
        "setViewport context should be missing",
    );

    for params in [
        json!({"userContexts": [""], "viewport": {"width": 100, "height": 200}}),
        json!({"userContexts": ["somestring"], "viewport": {"width": 100, "height": 200}}),
    ] {
        id += 1;
        let response = send_bidi_command(
            &mut socket,
            id,
            "browsingContext.setViewport",
            params.clone(),
        )
        .await;
        assert_bidi_error(
            &response,
            "no such user context",
            &format!("setViewport userContexts should be missing: {params}"),
        );
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_emulation_set_user_agent_override_invalid_parameters_match_wpt_error_shape()
{
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_user_agent_override/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 500_u64;

    for params in [
        json!({}),
        json!({"userAgent": false}),
        json!({"userAgent": 42}),
        json!({"userAgent": {}}),
        json!({"userAgent": []}),
        json!({"contexts": [], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [false], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [42], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [{}], "userAgent": "Moli-UA/1.0"}),
        json!({"contexts": [[]], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [false], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [42], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [{}], "userAgent": "Moli-UA/1.0"}),
        json!({"userContexts": [[]], "userAgent": "Moli-UA/1.0"}),
        json!({
            "contexts": [context_id.clone()],
            "userContexts": ["default"],
            "userAgent": "Moli-UA/1.0"
        }),
    ] {
        id += 1;
        let response = send_bidi_command(
            &mut socket,
            id,
            "emulation.setUserAgentOverride",
            params.clone(),
        )
        .await;
        assert_bidi_error(
            &response,
            "invalid argument",
            &format!("setUserAgentOverride params should be invalid argument: {params}"),
        );
    }

    let empty_user_agent = send_bidi_command(
        &mut socket,
        id + 1,
        "emulation.setUserAgentOverride",
        json!({
            "userAgent": ""
        }),
    )
    .await;
    assert_bidi_error(
        &empty_user_agent,
        "unsupported operation",
        "setUserAgentOverride empty userAgent should be unsupported",
    );

    let missing_context = send_bidi_command(
        &mut socket,
        id + 2,
        "emulation.setUserAgentOverride",
        json!({
            "contexts": ["_invalid_"],
            "userAgent": "Moli-UA/1.0"
        }),
    )
    .await;
    assert_bidi_error(
        &missing_context,
        "no such frame",
        "setUserAgentOverride context should be missing",
    );

    let missing_user_context = send_bidi_command(
        &mut socket,
        id + 3,
        "emulation.setUserAgentOverride",
        json!({
            "userContexts": ["somestring"],
            "userAgent": "Moli-UA/1.0"
        }),
    )
    .await;
    assert_bidi_error(
        &missing_user_context,
        "no such user context",
        "setUserAgentOverride userContext should be missing",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_emulation_locale_timezone_invalid_parameters_match_wpt_error_shape() {
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_locale_override/invalid.py and
    // webdriver/tests/bidi/emulation/set_timezone_override/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 600_u64;

    for (method, params) in [
        ("emulation.setLocaleOverride", json!({})),
        ("emulation.setLocaleOverride", json!({"locale": "fr-FR"})),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [context_id.clone()]}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [context_id.clone()], "locale": false}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [context_id.clone()], "locale": 42}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [context_id.clone()], "locale": {}}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [context_id.clone()], "locale": []}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [], "locale": "fr-FR"}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"userContexts": [], "locale": "fr-FR"}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({
                "contexts": [context_id.clone()],
                "userContexts": ["default"],
                "locale": "fr-FR"
            }),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [context_id.clone()], "locale": ""}),
        ),
        (
            "emulation.setLocaleOverride",
            json!({"contexts": [context_id.clone()], "locale": "en_US"}),
        ),
        ("emulation.setTimezoneOverride", json!({})),
        (
            "emulation.setTimezoneOverride",
            json!({"timezone": "Asia/Tokyo"}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()]}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": false}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": 42}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": {}}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": []}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [], "timezone": "Asia/Tokyo"}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"userContexts": [], "timezone": "Asia/Tokyo"}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({
                "contexts": [context_id.clone()],
                "userContexts": ["default"],
                "timezone": "Asia/Tokyo"
            }),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": ""}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": "Europe/Bielefeld"}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": "+1:00"}),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({"contexts": [context_id.clone()], "timezone": "GMT+05:00"}),
        ),
    ] {
        id += 1;
        let response = send_bidi_command(&mut socket, id, method, params.clone()).await;
        assert_bidi_error(
            &response,
            "invalid argument",
            &format!("{method} params should be invalid argument: {params}"),
        );
    }

    for (method, params) in [
        (
            "emulation.setLocaleOverride",
            json!({
                "contexts": ["_invalid_"],
                "locale": "fr-FR"
            }),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({
                "contexts": ["_invalid_"],
                "timezone": "Asia/Tokyo"
            }),
        ),
    ] {
        id += 1;
        let response = send_bidi_command(&mut socket, id, method, params).await;
        assert_bidi_error(
            &response,
            "no such frame",
            &format!("{method} context should be missing"),
        );
    }

    for (method, params) in [
        (
            "emulation.setLocaleOverride",
            json!({
                "userContexts": ["somestring"],
                "locale": "fr-FR"
            }),
        ),
        (
            "emulation.setTimezoneOverride",
            json!({
                "userContexts": ["somestring"],
                "timezone": "Asia/Tokyo"
            }),
        ),
    ] {
        id += 1;
        let response = send_bidi_command(&mut socket, id, method, params).await;
        assert_bidi_error(
            &response,
            "no such user context",
            &format!("{method} userContext should be missing"),
        );
    }

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_emulation_set_network_conditions_invalid_parameters_match_wpt_error_shape()
{
    // Ported from Chromium/WPT
    // webdriver/tests/bidi/emulation/set_network_conditions/invalid.py.
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;
    let mut id = 650_u64;

    for params in [
        json!({}),
        json!({"contexts": [context_id.clone()]}),
        json!({"contexts": [], "networkConditions": null}),
        json!({"contexts": [false], "networkConditions": null}),
        json!({"contexts": [42], "networkConditions": null}),
        json!({"contexts": [{}], "networkConditions": null}),
        json!({"contexts": [[]], "networkConditions": null}),
        json!({"userContexts": [], "networkConditions": null}),
        json!({"userContexts": [false], "networkConditions": null}),
        json!({"userContexts": [42], "networkConditions": null}),
        json!({"userContexts": [{}], "networkConditions": null}),
        json!({"userContexts": [[]], "networkConditions": null}),
        json!({
            "contexts": [context_id.clone()],
            "userContexts": ["default"],
            "networkConditions": null
        }),
        json!({"contexts": [context_id.clone()], "networkConditions": false}),
        json!({"contexts": [context_id.clone()], "networkConditions": 42}),
        json!({"contexts": [context_id.clone()], "networkConditions": "offline"}),
        json!({"contexts": [context_id.clone()], "networkConditions": []}),
        json!({"contexts": [context_id.clone()], "networkConditions": {}}),
        json!({
            "contexts": [context_id.clone()],
            "networkConditions": {
                "type": "SOME_INVALID_TYPE"
            }
        }),
        json!({
            "contexts": [context_id.clone()],
            "networkConditions": {
                "type": false
            }
        }),
        json!({
            "contexts": [context_id.clone()],
            "networkConditions": {
                "type": "offline",
                "extra": true
            }
        }),
    ] {
        id += 1;
        let response = send_bidi_command(
            &mut socket,
            id,
            "emulation.setNetworkConditions",
            params.clone(),
        )
        .await;
        assert_bidi_error(
            &response,
            "invalid argument",
            &format!("setNetworkConditions params should be invalid argument: {params}"),
        );
    }

    let missing_context = send_bidi_command(
        &mut socket,
        id + 1,
        "emulation.setNetworkConditions",
        json!({
            "contexts": ["_invalid_"],
            "networkConditions": null
        }),
    )
    .await;
    assert_bidi_error(
        &missing_context,
        "no such frame",
        "setNetworkConditions context should be missing",
    );

    let missing_user_context = send_bidi_command(
        &mut socket,
        id + 2,
        "emulation.setNetworkConditions",
        json!({
            "userContexts": ["somestring"],
            "networkConditions": null
        }),
    )
    .await;
    assert_bidi_error(
        &missing_user_context,
        "no such user context",
        "setNetworkConditions userContext should be missing",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_input_perform_and_release_actions_use_shared_input() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let page = "data:text/html,<input id='field'><script>window.__events=[];const field=document.getElementById('field');field.focus();field.addEventListener('keydown',event=>window.__events.push(event.type+':'+event.key));field.addEventListener('keyup',event=>window.__events.push(event.type+':'+event.key));</script>";
    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": page,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let type_a = send_bidi_command_response(
        &mut socket,
        4,
        "input.performActions",
        json!({
            "context": context_id.clone(),
            "actions": [{
                "type": "key",
                "id": "keyboard",
                "actions": [
                    { "type": "keyDown", "value": "a" },
                    { "type": "keyUp", "value": "a" }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(type_a["type"], json!("success"));
    assert_eq!(type_a["result"], json!({}));

    let events_after_a = send_bidi_command_response(
        &mut socket,
        5,
        "script.evaluate",
        json!({
            "expression": "window.__events.join(',')",
            "target": {
                "context": context_id.clone()
            },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(events_after_a["type"], json!("success"));
    assert_eq!(
        events_after_a["result"]["result"]["value"],
        json!("keydown:a,keyup:a")
    );

    let hold_b = send_bidi_command_response(
        &mut socket,
        6,
        "input.performActions",
        json!({
            "context": context_id.clone(),
            "actions": [{
                "type": "key",
                "id": "keyboard",
                "actions": [
                    { "type": "keyDown", "value": "b" }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(hold_b["type"], json!("success"));
    assert_eq!(hold_b["result"], json!({}));

    let release = send_bidi_command_response(
        &mut socket,
        7,
        "input.releaseActions",
        json!({
            "context": context_id.clone()
        }),
    )
    .await;
    assert_eq!(release["type"], json!("success"));
    assert_eq!(release["result"], json!({}));

    let events = send_bidi_command_response(
        &mut socket,
        8,
        "script.evaluate",
        json!({
            "expression": "window.__events.join(',')",
            "target": {
                "context": context_id.clone()
            },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(events["type"], json!("success"));
    assert_eq!(
        events["result"]["result"]["value"],
        json!("keydown:a,keyup:a,keydown:b,keyup:b")
    );

    let invalid_context = send_bidi_command_response(
        &mut socket,
        9,
        "input.performActions",
        json!({
            "context": "missing-context",
            "actions": [{
                "type": "key",
                "id": "keyboard",
                "actions": [{ "type": "keyDown", "value": "x" }]
            }]
        }),
    )
    .await;
    assert_bidi_error(
        &invalid_context,
        "no such frame",
        "input.performActions should reject missing context",
    );

    let invalid_release_context = send_bidi_command_response(
        &mut socket,
        10,
        "input.releaseActions",
        json!({
            "context": "missing-context"
        }),
    )
    .await;
    assert_bidi_error(
        &invalid_release_context,
        "no such frame",
        "input.releaseActions should reject missing context",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_input_set_files_updates_file_input() {
    let first_file = TempPath::new("bidi-set-files-first");
    let second_file = TempPath::new("bidi-set-files-second");
    fs::write(&first_file.path, b"alpha").expect("write first upload file");
    fs::write(&second_file.path, b"bravo!").expect("write second upload file");
    let first_name = first_file
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("first file should have a filename")
        .to_owned();
    let second_name = second_file
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("second file should have a filename")
        .to_owned();

    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let page = "data:text/html,<input id='file' type='file' multiple><input id='single' type='file'><script>window.__events=[];for(const id of ['file','single']){const el=document.getElementById(id);for(const type of ['input','change','cancel']){el.addEventListener(type,()=>window.__events.push(id+':'+type+':'+el.files.length));}}</script>";
    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": page,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let file_input = send_bidi_command_response(
        &mut socket,
        4,
        "script.evaluate",
        json!({
            "expression": "document.getElementById('file')",
            "target": { "context": context_id.clone() },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(file_input["type"], json!("success"));
    let file_shared_id = file_input["result"]["result"]["sharedId"]
        .as_str()
        .expect("file input remote value should include sharedId")
        .to_owned();

    for (id, params, expected_error, context) in [
        (
            40,
            json!({
                "context": context_id.clone(),
                "element": null,
                "files": []
            }),
            "invalid argument",
            "input.setFiles should reject non-object element",
        ),
        (
            41,
            json!({
                "context": context_id.clone(),
                "element": { "sharedId": file_shared_id.clone() },
                "files": false
            }),
            "invalid argument",
            "input.setFiles should reject non-array files",
        ),
        (
            42,
            json!({
                "context": context_id.clone(),
                "element": { "sharedId": file_shared_id.clone() },
                "files": [false]
            }),
            "invalid argument",
            "input.setFiles should reject non-string file entries",
        ),
    ] {
        let invalid = send_bidi_command_response(&mut socket, id, "input.setFiles", params).await;
        assert_bidi_error(&invalid, expected_error, context);
    }

    let set_files = send_bidi_command_response(
        &mut socket,
        5,
        "input.setFiles",
        json!({
            "context": context_id.clone(),
            "element": { "sharedId": file_shared_id },
            "files": [
                first_file.path.to_string_lossy().to_string(),
                second_file.path.to_string_lossy().to_string()
            ]
        }),
    )
    .await;
    assert_eq!(
        set_files["type"],
        json!("success"),
        "input.setFiles response: {set_files:?}"
    );
    assert_eq!(set_files["result"], json!({}));

    let summary = send_bidi_command_response(
        &mut socket,
        6,
        "script.evaluate",
        json!({
            "expression": "(()=>{const input=document.getElementById('file');return JSON.stringify({length:input.files.length,names:Array.from(input.files).map(file=>file.name).join('|'),sizes:Array.from(input.files).map(file=>file.size).join('|'),value:input.value,events:window.__events.join(',')});})()",
            "target": { "context": context_id.clone() },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(summary["type"], json!("success"));
    let summary: serde_json::Value = serde_json::from_str(
        summary["result"]["result"]["value"]
            .as_str()
            .expect("summary should be a JSON string"),
    )
    .expect("summary should parse");
    assert_eq!(summary["length"], json!(2));
    assert_eq!(
        summary["names"],
        json!(format!("{first_name}|{second_name}"))
    );
    assert_eq!(summary["sizes"], json!("5|6"));
    assert_eq!(
        summary["value"],
        json!(format!("C:\\fakepath\\{first_name}"))
    );
    assert_eq!(summary["events"], json!("file:input:2,file:change:2"));

    let clear = send_bidi_command_response(
        &mut socket,
        7,
        "input.setFiles",
        json!({
            "context": context_id.clone(),
            "element": { "sharedId": file_shared_id },
            "files": []
        }),
    )
    .await;
    assert_eq!(clear["type"], json!("success"));

    let after_clear = send_bidi_command_response(
        &mut socket,
        8,
        "script.evaluate",
        json!({
            "expression": "(()=>{const input=document.getElementById('file');return JSON.stringify({length:input.files.length,value:input.value,events:window.__events.join(',')});})()",
            "target": { "context": context_id.clone() },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(after_clear["type"], json!("success"));
    let after_clear: serde_json::Value = serde_json::from_str(
        after_clear["result"]["result"]["value"]
            .as_str()
            .expect("clear summary should be a JSON string"),
    )
    .expect("clear summary should parse");
    assert_eq!(after_clear["length"], json!(0));
    assert_eq!(after_clear["value"], json!(""));
    assert_eq!(
        after_clear["events"],
        json!("file:input:2,file:change:2,file:input:0,file:change:0")
    );

    let single_input = send_bidi_command_response(
        &mut socket,
        9,
        "script.evaluate",
        json!({
            "expression": "document.getElementById('single')",
            "target": { "context": context_id.clone() },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(single_input["type"], json!("success"));
    let single_shared_id = single_input["result"]["result"]["sharedId"]
        .as_str()
        .expect("single file input remote value should include sharedId")
        .to_owned();
    let non_multiple = send_bidi_command_response(
        &mut socket,
        10,
        "input.setFiles",
        json!({
            "context": context_id.clone(),
            "element": { "sharedId": single_shared_id },
            "files": [
                first_file.path.to_string_lossy().to_string(),
                second_file.path.to_string_lossy().to_string()
            ]
        }),
    )
    .await;
    assert_bidi_error(
        &non_multiple,
        "unable to set file input",
        "input.setFiles should reject multiple files for non-multiple input",
    );

    let set_single = send_bidi_command_response(
        &mut socket,
        11,
        "input.setFiles",
        json!({
            "context": context_id.clone(),
            "element": { "sharedId": single_shared_id },
            "files": [
                first_file.path.to_string_lossy().to_string()
            ]
        }),
    )
    .await;
    assert_eq!(
        set_single["type"],
        json!("success"),
        "single input.setFiles response: {set_single:?}"
    );

    let set_single_again = send_bidi_command_response(
        &mut socket,
        12,
        "input.setFiles",
        json!({
            "context": context_id.clone(),
            "element": { "sharedId": single_shared_id },
            "files": [
                first_file.path.to_string_lossy().to_string()
            ]
        }),
    )
    .await;
    assert_eq!(
        set_single_again["type"],
        json!("success"),
        "same single input.setFiles response: {set_single_again:?}"
    );

    let after_same_single = send_bidi_command_response(
        &mut socket,
        13,
        "script.evaluate",
        json!({
            "expression": "(()=>{const input=document.getElementById('single');return JSON.stringify({length:input.files.length,names:Array.from(input.files).map(file=>file.name).join('|'),value:input.value,events:window.__events.join(',')});})()",
            "target": { "context": context_id.clone() },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(after_same_single["type"], json!("success"));
    let after_same_single: serde_json::Value = serde_json::from_str(
        after_same_single["result"]["result"]["value"]
            .as_str()
            .expect("same-file summary should be a JSON string"),
    )
    .expect("same-file summary should parse");
    assert_eq!(after_same_single["length"], json!(1));
    assert_eq!(after_same_single["names"], json!(first_name));
    assert_eq!(
        after_same_single["value"],
        json!(format!("C:\\fakepath\\{first_name}"))
    );
    assert_eq!(
        after_same_single["events"],
        json!(
            "file:input:2,file:change:2,file:input:0,file:change:0,single:input:1,single:change:1,single:cancel:1"
        )
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_input_file_dialog_opened_event_matches_wpt_shape() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let subscribe = send_bidi_command_response(
        &mut socket,
        3,
        "session.subscribe",
        json!({
            "events": ["input.fileDialogOpened"],
            "contexts": [context_id.clone()]
        }),
    )
    .await;
    assert_eq!(
        subscribe["type"],
        json!("success"),
        "session.subscribe should accept input.fileDialogOpened: {subscribe:?}"
    );

    let navigate = send_bidi_command_response(
        &mut socket,
        4,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": "data:text/html,<input id=input type=file multiple />",
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    socket
        .send(WsMessage::Text(
            json!({
                "id": 5_u64,
                "method": "script.evaluate",
                "params": {
                    "expression": "input.click(); input",
                    "target": { "context": context_id.clone() },
                    "awaitPromise": false,
                    "userActivation": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send script.evaluate that opens file dialog");
    let mut messages = recv_until_id(&mut socket, 5).await;
    if bidi_events_by_method(&messages, "input.fileDialogOpened").is_empty() {
        messages.extend(
            recv_until_match(&mut socket, |message| {
                message["method"] == json!("input.fileDialogOpened")
            })
            .await,
        );
    }
    let evaluate = bidi_message_by_id(&messages, 5);
    assert_eq!(
        evaluate["type"],
        json!("success"),
        "script.evaluate should return the clicked input: {evaluate:?}"
    );
    let returned_shared_id = evaluate["result"]["result"]["sharedId"]
        .as_str()
        .expect("returned input should include a sharedId");
    let event = bidi_events_by_method(&messages, "input.fileDialogOpened")
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("input.fileDialogOpened event should arrive: {messages:#?}"));
    assert_eq!(event["type"], json!("event"));
    assert_eq!(event["params"]["context"], json!(context_id));
    assert_eq!(event["params"]["multiple"], json!(true));
    assert_eq!(
        event["params"]["element"]["sharedId"],
        json!(returned_shared_id)
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
}

#[tokio::test]
async fn websocket_bidi_input_element_origin_uses_real_geometry() {
    let (cdp_addr, protocol_server) = spawn_test_protocol_server().await;
    let (mut socket, context_id) = bidi_session_with_context(cdp_addr).await;

    let page = "data:text/html,<script>window.__clicked=false;window.__wheel=null;document.addEventListener('wheel',event=>{window.__wheel={type:event.type,deltaX:event.deltaX,deltaY:event.deltaY,clientX:event.clientX,clientY:event.clientY};});</script><button id='target' onclick='window.__clicked=true' style='width:80px;height:40px'>go</button><div id='wheel' style='width:200px;height:200px'>wheel-target</div>";
    let navigate = send_bidi_command(
        &mut socket,
        3,
        "browsingContext.navigate",
        json!({
            "context": context_id.clone(),
            "url": page,
            "wait": "complete"
        }),
    )
    .await;
    assert_eq!(navigate["type"], json!("success"));

    let button = send_bidi_command_response(
        &mut socket,
        4,
        "script.evaluate",
        json!({
            "expression": "document.getElementById('target')",
            "target": {
                "context": context_id.clone()
            },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(button["type"], json!("success"));
    let button_shared_id = button["result"]["result"]["sharedId"]
        .as_str()
        .expect("button remote value should include sharedId")
        .to_owned();

    let wheel_target = send_bidi_command_response(
        &mut socket,
        5,
        "script.evaluate",
        json!({
            "expression": "document.getElementById('wheel')",
            "target": {
                "context": context_id.clone()
            },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(wheel_target["type"], json!("success"));
    let wheel_shared_id = wheel_target["result"]["result"]["sharedId"]
        .as_str()
        .expect("wheel target remote value should include sharedId")
        .to_owned();

    let click = send_bidi_command_response(
        &mut socket,
        6,
        "input.performActions",
        json!({
            "context": context_id.clone(),
            "actions": [{
                "type": "pointer",
                "id": "mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [
                    {
                        "type": "pointerMove",
                        "origin": { "type": "element", "sharedId": button_shared_id },
                        "x": 0,
                        "y": 0
                    },
                    { "type": "pointerDown", "button": 0 },
                    { "type": "pointerUp", "button": 0 }
                ]
            }]
        }),
    )
    .await;
    assert_eq!(click["type"], json!("success"), "click response: {click:?}");

    let clicked = send_bidi_command_response(
        &mut socket,
        7,
        "script.evaluate",
        json!({
            "expression": "Boolean(window.__clicked)",
            "target": {
                "context": context_id.clone()
            },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(clicked["type"], json!("success"));
    assert_eq!(clicked["result"]["result"]["value"], json!(true));

    let scroll = send_bidi_command_response(
        &mut socket,
        8,
        "input.performActions",
        json!({
            "context": context_id.clone(),
            "actions": [{
                "type": "wheel",
                "id": "wheel",
                "actions": [{
                    "type": "scroll",
                    "origin": { "type": "element", "sharedId": wheel_shared_id },
                    "x": 1,
                    "y": 2,
                    "deltaX": 7,
                    "deltaY": 13
                }]
            }]
        }),
    )
    .await;
    assert_eq!(
        scroll["type"],
        json!("success"),
        "scroll response: {scroll:?}"
    );

    let wheel = send_bidi_command_response(
        &mut socket,
        9,
        "script.evaluate",
        json!({
            "expression": "window.__wheel ? [window.__wheel.type, window.__wheel.deltaX, window.__wheel.deltaY].join(':') : 'null'",
            "target": {
                "context": context_id.clone()
            },
            "awaitPromise": false
        }),
    )
    .await;
    assert_eq!(wheel["type"], json!("success"));
    assert_eq!(
        wheel["result"]["result"]["value"],
        json!("wheel:7:13"),
        "element-origin wheel should dispatch at real geometry: {wheel:?}"
    );

    let missing_origin = send_bidi_command_response(
        &mut socket,
        10,
        "input.performActions",
        json!({
            "context": context_id.clone(),
            "actions": [{
                "type": "pointer",
                "id": "missing-origin-mouse",
                "parameters": { "pointerType": "mouse" },
                "actions": [{
                    "type": "pointerMove",
                    "origin": { "type": "element", "sharedId": "missing-shared-id" },
                    "x": 0,
                    "y": 0
                }]
            }]
        }),
    )
    .await;
    assert_bidi_error(
        &missing_origin,
        "no such node",
        "input.performActions should reject missing element-origin sharedId",
    );

    let _ = socket.close(None).await;
    protocol_server.abort();
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
