use super::*;

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
    assert_eq!(
        bidi_string_script_value(
            &mut socket,
            91,
            &context_id,
            "Intl.DateTimeFormat().resolvedOptions().locale"
        )
        .await,
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
