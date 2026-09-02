use super::*;

fn find_cdp_node_by_local_name<'a>(
    node: &'a serde_json::Value,
    local_name: &str,
) -> Option<&'a serde_json::Value> {
    if node["localName"] == json!(local_name) {
        return Some(node);
    }
    node["children"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|child| find_cdp_node_by_local_name(child, local_name))
}

fn assert_runtime_navigation_context_reset(
    sent: &[serde_json::Value],
    session_id: &str,
    frame_id: &str,
) {
    let session_context_events = sent
        .iter()
        .enumerate()
        .filter(|(_, message)| message["sessionId"] == json!(session_id))
        .filter(|(_, message)| {
            matches!(
                message["method"].as_str(),
                Some("Runtime.executionContextsCleared") | Some("Runtime.executionContextCreated")
            )
        })
        .collect::<Vec<_>>();
    let last_clear_index = session_context_events
        .iter()
        .filter(|(_, message)| message["method"] == json!("Runtime.executionContextsCleared"))
        .map(|(index, _)| *index)
        .max()
        .unwrap_or_else(|| {
            panic!("navigation should clear old Runtime contexts for {session_id}: {sent:?}")
        });
    let default_context_index = session_context_events
        .iter()
        .filter(|(_, message)| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(frame_id)
        })
        .map(|(index, _)| *index)
        .next()
        .unwrap_or_else(|| {
            panic!(
                "navigation should create the new default Runtime context for {session_id}: {sent:?}"
            )
        });
    assert!(
        last_clear_index < default_context_index,
        "all old-context clears should precede the new default context for {session_id}: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn data_url_commit_applies_preloads_worlds_and_bindings_before_author_script() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 90_100,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    take_response_by_id(&mut ctx, 90_100);

    ctx.process_async(json!({
        "id": 90_101,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": { "source": "globalThis.__dataPreload = 'ready';" }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_101)["result"]["identifier"],
        json!("1")
    );

    ctx.process_async(json!({
        "id": 90_102,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__dataNamed = 'ready'; dataBinding('named-preload');",
            "worldName": "data-world"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_102)["result"]["identifier"],
        json!("2")
    );

    ctx.process_async(json!({
        "id": 90_103,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": { "name": "dataBinding" }
    }))
    .await;
    take_response_by_id(&mut ctx, 90_103);
    ctx.sent.clear();

    let url = concat!(
        "data:text/html,<!doctype html><script>",
        "globalThis.__dataCommitOrdering=JSON.stringify([globalThis.__dataPreload,typeof dataBinding]);",
        "dataBinding('author-script');",
        "</script>"
    );
    ctx.process_async(json!({
        "id": 90_104,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_104)["result"]["frameId"],
        json!("TID-1")
    );

    ctx.process_async(json!({
        "id": 90_105,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.__dataCommitOrdering",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_105)["result"]["result"]["value"],
        json!(r#"["ready","function"]"#)
    );

    let named_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("data-world")
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("data URL commit should publish the named-world execution context");
    ctx.process_async(json!({
        "id": 90_106,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": named_context_id,
            "expression": "globalThis.__dataNamed",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_106)["result"]["result"]["value"],
        json!("ready")
    );

    let binding_payloads = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("dataBinding")
        })
        .map(|message| message["params"]["payload"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        binding_payloads,
        vec![json!("named-preload"), json!("author-script")],
        "data URL named-world preload must run before its first author script"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn page_navigate_file_url_fails_before_navigation_events_or_document_replacement() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_107,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "file:///moli-policy-must-not-open" }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 90_107);
    assert_eq!(response["error"]["code"], json!(-32000));
    assert_eq!(
        response["error"]["message"],
        json!("Navigation to a local file URL requires an explicitly granted browser capability.")
    );
    assert!(
        ctx.sent.iter().all(|message| {
            !matches!(
                message["method"].as_str(),
                Some("Page.frameStartedNavigating")
                    | Some("Page.frameStartedLoading")
                    | Some("Network.requestWillBeSent")
            )
        }),
        "rejected file navigation must not start a browser load: {:?}",
        ctx.sent
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .target_url(),
        "about:blank"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_navigation_background_events_keep_typed_sidecars() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-typed",
        "TID-typed",
        "SID-typed",
        "about:blank",
    );
    let mut events = Vec::new();

    let owner = crate::conn::CommandOwnerScope::for_session("SID-typed");
    crate::domains::page::navigate_command_owner_from_renderer_background_events_async(
        &mut ctx.conn,
        &mut events,
        &owner,
        "data:text/html,<body>typed</body>",
    )
    .await;

    let parts = events
        .into_iter()
        .map(|event| event.into_parts())
        .collect::<Vec<_>>();
    assert!(
        parts.iter().all(|(message, _)| message.get("id").is_none()),
        "renderer-owned navigation must not synthesize a command response"
    );
    let frame_methods = parts
        .iter()
        .filter_map(|(message, _)| {
            message["method"]
                .as_str()
                .filter(|method| method.starts_with("Page.frame"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        &frame_methods[..5],
        &[
            "Page.frameScheduledNavigation",
            "Page.frameRequestedNavigation",
            "Page.frameClearedScheduledNavigation",
            "Page.frameStartedNavigating",
            "Page.frameStartedLoading",
        ],
        "renderer navigation probes must precede browser-side load start: {frame_methods:?}"
    );
    let (message, automation_event) = parts
        .iter()
        .find(|(message, _)| message["method"] == json!("Page.frameStartedNavigating"))
        .expect("renderer navigation should emit frameStartedNavigating");

    assert_eq!(message["sessionId"], "SID-typed");
    assert_eq!(message["params"]["frameId"], "TID-typed");
    assert_eq!(message["params"]["loaderId"], LOADER_ID);
    assert_eq!(
        message["params"]["url"],
        "data:text/html,<body>typed</body>"
    );
    assert!(matches!(
        automation_event,
        Some(AutomationEvent::NavigationFrame(event))
            if event.kind == NavigationFrameEventKind::StartedNavigating
                && event.frame_id.as_str() == "TID-typed"
                && event.loader_id.as_ref().map(|id| id.as_str()) == Some(LOADER_ID)
                && event.url == "data:text/html,<body>typed</body>"
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_fragment_navigation_preserves_initial_document_residence() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-renderer-fragment",
        "TID-renderer-fragment",
        "SID-renderer-fragment",
        "about:blank",
    );
    ensure_initial_document_for_session(&mut ctx, Some("SID-renderer-fragment")).await;
    ctx.process_async(json!({
        "id": 90_120,
        "method": "Page.enable",
        "sessionId": "SID-renderer-fragment"
    }))
    .await;
    ctx.expect_result(90_120, json!({}), Some("SID-renderer-fragment"));
    ctx.sent.clear();
    let before = ctx
        .conn
        .renderer_page_residence_identity_for_session_owner(Some("SID-renderer-fragment"))
        .expect("initial renderer Page residence");
    let mut events = Vec::new();

    let owner = crate::conn::CommandOwnerScope::for_session("SID-renderer-fragment");
    crate::domains::page::navigate_command_owner_from_renderer_background_events_async(
        &mut ctx.conn,
        &mut events,
        &owner,
        "about:blank#popup",
    )
    .await;

    assert_eq!(
        ctx.conn
            .renderer_page_residence_identity_for_session_owner(Some("SID-renderer-fragment")),
        Some(before),
        "a renderer-owned fragment navigation must retain the current Document's Page residence"
    );
    assert!(
        events.is_empty(),
        "the browser-owner helper must not synthesize output already owned by the renderer stream: {events:?}"
    );
    wait_until_message(
        &mut ctx,
        Some("SID-renderer-fragment"),
        "renderer-owned same-document navigation",
        |message| {
            message["method"] == json!("Page.navigatedWithinDocument")
                && message["params"]["url"] == json!("about:blank#popup")
        },
    )
    .await;
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Network.loadingFailed")),
        "about:blank#fragment must not be sent through the network loader: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_history_supports_playwright_back_forward_commands() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let first_url = "data:text/html,<title>A</title><main>history-a</main>";
    let second_url = "data:text/html,<title>B</title><main>history-b</main>";

    ctx.process_async(json!({
        "id": 1,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": first_url }
    }))
    .await;
    take_response_by_id(&mut ctx, 1);
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "first history document DOMContentLoaded",
        |message| message["method"] == json!("Page.domContentEventFired"),
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 2,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": second_url }
    }))
    .await;
    take_response_by_id(&mut ctx, 2);
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "second history document DOMContentLoaded",
        |message| message["method"] == json!("Page.domContentEventFired"),
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 3,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-1"
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 3);
    // A newly created Chromium target starts with a real `about:blank`
    // session-history entry. The first Page.navigate appends to that entry;
    // it does not replace it.
    assert_eq!(history["result"]["currentIndex"], json!(2));
    assert_eq!(history["result"]["entries"][0]["url"], "about:blank");
    assert_eq!(history["result"]["entries"][1]["url"], first_url);
    assert_eq!(history["result"]["entries"][1]["title"], "A");
    assert_eq!(history["result"]["entries"][2]["url"], second_url);
    assert_eq!(history["result"]["entries"][2]["title"], "B");
    let first_entry_id = history["result"]["entries"][1]["id"]
        .as_i64()
        .expect("first history entry id");
    let second_entry_id = history["result"]["entries"][2]["id"]
        .as_i64()
        .expect("second history entry id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 4,
        "method": "Page.navigateToHistoryEntry",
        "sessionId": "SID-1",
        "params": { "entryId": first_entry_id }
    }))
    .await;
    take_response_by_id(&mut ctx, 4);
    assert_eq!(
        ctx.conn.browser_context.as_ref().unwrap().target_url(),
        first_url
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 5,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-1"
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 5);
    assert_eq!(history["result"]["currentIndex"], json!(1));
    assert_eq!(history["result"]["entries"][1]["id"], json!(first_entry_id));
    assert_eq!(
        history["result"]["entries"][2]["id"],
        json!(second_entry_id)
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 6,
        "method": "Page.navigateToHistoryEntry",
        "sessionId": "SID-1",
        "params": { "entryId": second_entry_id }
    }))
    .await;
    take_response_by_id(&mut ctx, 6);
    assert_eq!(
        ctx.conn.browser_context.as_ref().unwrap().target_url(),
        second_url
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 7,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-1"
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 7);
    assert_eq!(history["result"]["currentIndex"], json!(2));
    assert_eq!(history["result"]["entries"][1]["id"], json!(first_entry_id));
    assert_eq!(
        history["result"]["entries"][2]["id"],
        json!(second_entry_id)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_history_back_uses_browser_owned_navigation_history() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-RENDERER-HISTORY",
        "TID-RENDERER-HISTORY",
        "SID-RENDERER-HISTORY",
        "about:blank",
    );
    let first_url = "data:text/html,<title>First</title><main>first</main>";
    let second_url = "data:text/html,<title>Second</title><main>second</main>";

    for (id, url) in [(10, first_url), (11, second_url)] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": "SID-RENDERER-HISTORY",
            "params": { "url": url }
        }))
        .await;
        take_response_by_id(&mut ctx, id);
        ctx.sent.clear();
    }

    ctx.process_async(json!({
        "id": 12,
        "method": "Runtime.evaluate",
        "sessionId": "SID-RENDERER-HISTORY",
        "params": {
            "expression": "history.back(); 'queued'",
            "returnByValue": true
        }
    }))
    .await;

    let response = take_response_by_id(&mut ctx, 12);
    assert_eq!(response["result"]["result"]["value"], json!("queued"));
    assert_eq!(
        ctx.conn.browser_context.as_ref().unwrap().target_url(),
        first_url
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 13,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-RENDERER-HISTORY"
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 13);
    assert_eq!(history["result"]["currentIndex"], json!(1));
    assert_eq!(history["result"]["entries"][0]["url"], "about:blank");
    assert_eq!(history["result"]["entries"][1]["url"], first_url);
    assert_eq!(history["result"]["entries"][2]["url"], second_url);

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 14,
        "method": "Runtime.evaluate",
        "sessionId": "SID-RENDERER-HISTORY",
        "params": {
            "expression": "history.back(); 'to-initial-empty-document'",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 14);
    assert_eq!(
        response["result"]["result"]["value"],
        json!("to-initial-empty-document")
    );
    assert_eq!(
        ctx.conn.browser_context.as_ref().unwrap().target_url(),
        "about:blank"
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 15,
        "method": "Runtime.evaluate",
        "sessionId": "SID-RENDERER-HISTORY",
        "params": {
            "expression": "history.back(); 'at-start'",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 15);
    assert_eq!(response["result"]["result"]["value"], json!("at-start"));
    assert_eq!(
        ctx.conn.browser_context.as_ref().unwrap().target_url(),
        "about:blank"
    );
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("id").is_none_or(|id| !id.is_null())),
        "an out-of-range page traversal must not emit an id:null command response: {:?}",
        ctx.sent
    );

    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 16,
        "method": "Runtime.evaluate",
        "sessionId": "SID-RENDERER-HISTORY",
        "params": {
            "expression": "history.forward(); 'queued'",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 16);
    assert_eq!(response["result"]["result"]["value"], json!("queued"));
    assert_eq!(
        ctx.conn.browser_context.as_ref().unwrap().target_url(),
        first_url
    );
}

fn take_navigated_within_document_event(
    ctx: &mut TestContext,
    expected_url: &str,
    expected_navigation_type: &str,
) {
    let position = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.navigatedWithinDocument"))
        .unwrap_or_else(|| {
            panic!(
                "expected Page.navigatedWithinDocument for {expected_url}; messages={:?}",
                ctx.sent
            )
        });
    let event = ctx.sent.remove(position);
    assert_eq!(event["params"]["frameId"], json!("TID-SAME-DOCUMENT"));
    assert_eq!(event["params"]["url"], json!(expected_url));
    assert_eq!(
        event["params"]["navigationType"],
        json!(expected_navigation_type)
    );
}

// Ported from Chromium's
// third_party/blink/web_tests/http/tests/inspector-protocol/page/
// page-navigatedWithinDocument.js. Keep the whole sequence together: the
// back/forward assertions depend on the mixed fragment + History API list.
#[tokio::test(flavor = "multi_thread")]
async fn navigated_within_document_matches_chromium_mixed_history_sequence() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/inspector-protocol-page.html",
            axum::routing::get(|| async move {
                (
                    [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                    "<!doctype html><title>same-document</title><main>page</main>",
                )
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-SAME-DOCUMENT",
        "TID-SAME-DOCUMENT",
        "SID-SAME-DOCUMENT",
        "about:blank",
    );
    ctx.enable_page_events_for_test(Some("SID-SAME-DOCUMENT"));
    let base_url = format!("http://{addr}/inspector-protocol-page.html");
    let foo_url = format!("{base_url}#foo");
    let bar_url = format!("{base_url}#bar");
    let wow_url = format!("http://{addr}/wow.html");
    let replaced_url = format!("http://{addr}/replaced.html");

    ctx.process_and_wait_for_response_async(json!({
        "id": 20,
        "method": "Page.navigate",
        "sessionId": "SID-SAME-DOCUMENT",
        "params": { "url": base_url }
    }))
    .await;
    take_response_by_id(&mut ctx, 20);
    wait_until_frame_stopped_loading(&mut ctx, "TID-SAME-DOCUMENT").await;
    ctx.sent.clear();

    for (id, url) in [
        (21, foo_url.as_str()),
        // Chromium's navigate-same-fragment.js requires a repeated
        // Page.navigate to remain same-document and emit the event again.
        (28, foo_url.as_str()),
        (22, bar_url.as_str()),
    ] {
        let hashchange_completion_ids = match id {
            21 => Some((121, 221)),
            22 => Some((122, 222)),
            _ => None,
        };
        if let Some((arm_id, _)) = hashchange_completion_ids {
            ctx.process_async(json!({
                "id": arm_id,
                "method": "Runtime.evaluate",
                "sessionId": "SID-SAME-DOCUMENT",
                "params": {
                    "expression": r#"
                        globalThis.__fragmentNavigationDone = new Promise(resolve => {
                            addEventListener('hashchange', () => resolve(location.href), {
                                once: true,
                            });
                        });
                        'armed'
                    "#,
                    "returnByValue": true
                }
            }))
            .await;
            let armed = take_response_by_id(&mut ctx, arm_id);
            assert_eq!(armed["result"]["result"]["value"], json!("armed"));
            ctx.sent.clear();
        }
        if id == 28 {
            ctx.process_async(json!({
                "id": 128,
                "method": "Runtime.evaluate",
                "sessionId": "SID-SAME-DOCUMENT",
                "params": {
                    "expression": r#"
                        globalThis.__repeatFragmentBefore = {
                            historyLength: history.length,
                            navigationIndex: navigation.currentEntry.index,
                        };
                        globalThis.__repeatFragmentEvents = [];
                        navigation.addEventListener('navigate', event => {
                            __repeatFragmentEvents.push(`navigate:${event.navigationType}`);
                        }, { once: true });
                        navigation.addEventListener('currententrychange', event => {
                            __repeatFragmentEvents.push(`currententrychange:${event.navigationType}`);
                        }, { once: true });
                        addEventListener('popstate', () => {
                            __repeatFragmentEvents.push('popstate');
                        }, { once: true });
                        addEventListener('hashchange', () => {
                            __repeatFragmentEvents.push('hashchange');
                        }, { once: true });
                    "#
                }
            }))
            .await;
            take_response_by_id(&mut ctx, 128);
            ctx.sent.clear();
        }
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": "SID-SAME-DOCUMENT",
            "params": { "url": url }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        assert!(
            response["result"]["loaderId"].is_null(),
            "same-document Page.navigate must not report a loader id: {response:?}"
        );
        take_navigated_within_document_event(&mut ctx, url, "fragment");
        ctx.sent.clear();
        if let Some((_, completion_id)) = hashchange_completion_ids {
            ctx.process_async(json!({
                "id": completion_id,
                "method": "Runtime.evaluate",
                "sessionId": "SID-SAME-DOCUMENT",
                "params": {
                    "expression": "globalThis.__fragmentNavigationDone",
                    "awaitPromise": true,
                    "returnByValue": true
                }
            }))
            .await;
            wait_until_message(
                &mut ctx,
                "SID-SAME-DOCUMENT",
                "fragment navigation hashchange completion",
                |message| message["id"] == json!(completion_id),
            )
            .await;
            let completed = take_response_by_id(&mut ctx, completion_id);
            assert_eq!(completed["result"]["result"]["value"], json!(url));
            ctx.sent.clear();
        }
        if id == 28 {
            ctx.process_async(json!({
                "id": 129,
                "method": "Runtime.evaluate",
                "sessionId": "SID-SAME-DOCUMENT",
                "params": {
                    "expression": r#"({
                        historyDelta: history.length - __repeatFragmentBefore.historyLength,
                        navigationIndexDelta:
                            navigation.currentEntry.index - __repeatFragmentBefore.navigationIndex,
                        events: __repeatFragmentEvents,
                    })"#,
                    "returnByValue": true
                }
            }))
            .await;
            let repeat_observation = take_response_by_id(&mut ctx, 129);
            assert_eq!(
                repeat_observation["result"]["result"]["value"],
                json!({
                    "historyDelta": 1,
                    "navigationIndexDelta": 1,
                    "events": [
                        "navigate:push",
                        "currententrychange:push",
                        "popstate",
                    ],
                }),
                "repeated same-fragment Page.navigate must match Chromium's renderer-visible surfaces"
            );
            ctx.sent.clear();
        }
    }

    for (id, expression, expected_url) in [
        (
            23,
            "history.pushState({}, '', 'wow.html')",
            wow_url.as_str(),
        ),
        (
            24,
            "history.replaceState({}, '', '/replaced.html')",
            replaced_url.as_str(),
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-SAME-DOCUMENT",
            "params": { "expression": expression }
        }))
        .await;
        take_response_by_id(&mut ctx, id);
        take_navigated_within_document_event(&mut ctx, expected_url, "historyApi");
        ctx.sent.clear();
    }

    for (id, expression, expected_url) in [
        (25, "history.back()", bar_url.as_str()),
        (26, "history.forward()", replaced_url.as_str()),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-SAME-DOCUMENT",
            "params": { "expression": expression }
        }))
        .await;
        take_response_by_id(&mut ctx, id);
        // Chromium's inspector test deliberately starts `history.back()` /
        // `history.forward()` without awaiting their Runtime reply, then
        // independently awaits Page.navigatedWithinDocument. Traversal is a
        // later history task, so the Runtime response is not its completion
        // boundary.
        wait_until_message(
            &mut ctx,
            "SID-SAME-DOCUMENT",
            "history traversal Page.navigatedWithinDocument",
            |message| {
                message["method"] == json!("Page.navigatedWithinDocument")
                    && message["params"]["url"] == json!(expected_url)
            },
        )
        .await;
        take_navigated_within_document_event(&mut ctx, expected_url, "fragment");
        ctx.sent.clear();
    }

    ctx.process_async(json!({
        "id": 27,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-SAME-DOCUMENT"
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 27);
    assert_eq!(history["result"]["currentIndex"], json!(5));
    let urls = history["result"]["entries"]
        .as_array()
        .expect("navigation history entries")
        .iter()
        .map(|entry| entry["url"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(
        urls,
        vec![
            "about:blank",
            base_url.as_str(),
            foo_url.as_str(),
            foo_url.as_str(),
            bar_url.as_str(),
            replaced_url.as_str(),
        ],
        "a repeated same-fragment Page.navigate must append, while back/forward only move the cursor"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn get_navigation_history_completes_through_command_dispatch() {
    let mut ctx = TestContext::new();
    let page_url = "data:text/html,<title>History</title><main>start</main>";
    load_bc_with_session(
        &mut ctx,
        "BID-HISTORY-COMPLETE",
        "TID-HISTORY-COMPLETE",
        "SID-HISTORY-COMPLETE",
        page_url,
    );
    let page = ctx
        .conn
        .load_page_via_runtime_async(page_url)
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = json!({
        "id": 1209,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-HISTORY-COMPLETE"
    })
    .to_string();
    let CdpCommandTaskStep::Complete(outcome) = ctx.conn.start_command_dispatch(&raw) else {
        panic!("Page.getNavigationHistory should complete without renderer wait");
    };
    let (messages, scheduler_events) = outcome.into_parts();
    assert!(
        scheduler_events.is_empty(),
        "Page.getNavigationHistory should not enqueue scheduler events: {scheduler_events:?}"
    );
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["id"], json!(1209));
    assert_eq!(messages[0]["sessionId"], json!("SID-HISTORY-COMPLETE"));
    assert_eq!(messages[0]["result"]["currentIndex"], json!(0));
    assert_eq!(messages[0]["result"]["entries"][0]["url"], json!(page_url));
}

#[tokio::test(flavor = "multi_thread")]
async fn reset_navigation_history_prunes_browser_and_renderer_history() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Reset History</title><main>start</main>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind reset history server");
    let addr = listener.local_addr().expect("reset history server address");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let page_url = format!("http://{addr}/page");
    load_bc_with_session(
        &mut ctx,
        "BID-RESET-HISTORY",
        "TID-RESET-HISTORY",
        "SID-RESET-HISTORY",
        &page_url,
    );
    let page = ctx
        .conn
        .load_page_via_runtime_async(&page_url)
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    ctx.process_async(json!({
        "id": 1210,
        "method": "Runtime.evaluate",
        "sessionId": "SID-RESET-HISTORY",
        "params": {
            "expression": r##"
(() => {
  history.pushState({ step: 1 }, "", "#one");
  history.pushState({ step: 2 }, "", "#two");
  globalThis.__resetEntries = navigation.entries();
  globalThis.__resetCurrent = navigation.currentEntry;
  globalThis.__resetDisposed = [];
  __resetEntries.forEach((entry, index) => {
    entry.addEventListener("dispose", () => __resetDisposed.push(index));
  });
})()
"##
        }
    }))
    .await;
    let setup_response = take_response_by_id(&mut ctx, 1210);
    assert!(
        setup_response["result"]["exceptionDetails"].is_null(),
        "pushState setup should succeed: {setup_response}"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1211,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-RESET-HISTORY"
    }))
    .await;
    let history_before = take_response_by_id(&mut ctx, 1211);
    assert_eq!(history_before["result"]["currentIndex"], json!(2));
    assert_eq!(
        history_before["result"]["entries"]
            .as_array()
            .expect("history entries")
            .len(),
        3
    );
    let current_entry_id = history_before["result"]["entries"][2]["id"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1212,
        "method": "Page.resetNavigationHistory",
        "sessionId": "SID-RESET-HISTORY"
    }))
    .await;
    let reset_response = take_response_by_id(&mut ctx, 1212);
    assert_eq!(reset_response["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1213,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-RESET-HISTORY"
    }))
    .await;
    let history_after = take_response_by_id(&mut ctx, 1213);
    assert_eq!(history_after["result"]["currentIndex"], json!(0));
    assert_eq!(
        history_after["result"]["entries"]
            .as_array()
            .expect("history entries")
            .len(),
        1
    );
    assert_eq!(
        history_after["result"]["entries"][0]["id"],
        current_entry_id
    );
    assert_eq!(
        history_after["result"]["entries"][0]["url"],
        format!("{page_url}#two")
    );
    assert_eq!(
        history_after["result"]["entries"][0]["userTypedURL"],
        page_url
    );
    assert_eq!(
        history_after["result"]["entries"][0]["transitionType"],
        "link"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1214,
        "method": "Runtime.evaluate",
        "sessionId": "SID-RESET-HISTORY",
        "params": {
            "expression": r##"
({
  historyLength: history.length,
  navigationLength: navigation.entries().length,
  sameCurrent: navigation.currentEntry === __resetCurrent,
  sameArrayEntry: navigation.entries()[0] === __resetCurrent,
  currentIndex: navigation.currentEntry.index,
  currentUrl: navigation.currentEntry.url,
  historyState: history.state.step,
  disposed: __resetDisposed
})
"##,
            "returnByValue": true
        }
    }))
    .await;
    let renderer_state = take_response_by_id(&mut ctx, 1214);
    assert_eq!(
        renderer_state["result"]["result"]["value"],
        json!({
            "historyLength": 1,
            "navigationLength": 1,
            "sameCurrent": true,
            "sameArrayEntry": true,
            "currentIndex": 0,
            "currentUrl": format!("{page_url}#two"),
            "historyState": 2,
            "disposed": [1, 0],
        })
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1215,
        "method": "Runtime.evaluate",
        "sessionId": "SID-RESET-HISTORY",
        "params": {
            "expression": r##"
(() => {
  history.pushState({ step: 3 }, "", "#three");
  history.pushState({ step: 4 }, "", "#four");
  navigation.entries()[0].addEventListener("dispose", () => {
    history.pushState({ step: 5 }, "", "#during-dispose");
  });
})()
"##
        }
    }))
    .await;
    let reentrant_setup_response = take_response_by_id(&mut ctx, 1215);
    assert!(
        reentrant_setup_response["result"]["exceptionDetails"].is_null(),
        "reentrant pushState setup should succeed: {reentrant_setup_response}"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1216,
        "method": "Page.resetNavigationHistory",
        "sessionId": "SID-RESET-HISTORY"
    }))
    .await;
    let reentrant_reset_response = take_response_by_id(&mut ctx, 1216);
    assert_eq!(reentrant_reset_response["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1217,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-RESET-HISTORY"
    }))
    .await;
    let reentrant_history = take_response_by_id(&mut ctx, 1217);
    assert_eq!(reentrant_history["result"]["currentIndex"], json!(1));
    assert_eq!(
        reentrant_history["result"]["entries"]
            .as_array()
            .expect("reentrant history entries")
            .iter()
            .map(|entry| entry["url"].as_str().expect("history entry URL"))
            .collect::<Vec<_>>(),
        vec![
            format!("{page_url}#four"),
            format!("{page_url}#during-dispose")
        ]
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1218,
        "method": "Runtime.evaluate",
        "sessionId": "SID-RESET-HISTORY",
        "params": {
            "expression": r##"
({
  historyLength: history.length,
  navigationLength: navigation.entries().length,
  currentIndex: navigation.currentEntry.index,
  currentUrl: navigation.currentEntry.url,
  historyState: history.state.step
})
"##,
            "returnByValue": true
        }
    }))
    .await;
    let reentrant_renderer_state = take_response_by_id(&mut ctx, 1218);
    assert_eq!(
        reentrant_renderer_state["result"]["result"]["value"],
        json!({
            "historyLength": 1,
            "navigationLength": 2,
            "currentIndex": 1,
            "currentUrl": format!("{page_url}#during-dispose"),
            "historyState": 5,
        })
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_history_is_preserved_per_background_target() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-A", "SID-A", "about:blank");
    let a1_url = "data:text/html,<title>A1</title><main>a1</main>";
    let a2_url = "data:text/html,<title>A2</title><main>a2</main>";
    let b1_url = "data:text/html,<title>B1</title><main>b1</main>";

    for (id, url) in [(10, a1_url), (11, a2_url)] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": "SID-A",
            "params": { "url": url }
        }))
        .await;
        take_response_by_id(&mut ctx, id);
        ctx.sent.clear();
    }

    {
        let browser_context = ctx.conn.browser_context.as_mut().unwrap();
        browser_context.insert_page_target_host(PageTargetHost::new(
            "TID-B".to_owned(),
            Some("SID-B".to_owned()),
            crate::conn::TargetIdentityState::new(
                "about:blank".to_owned(),
                URL_BASE.to_owned(),
                "Secure".to_owned(),
            ),
            crate::conn::TargetPageSlot::empty_for_test_fixture(),
        ));
    }
    assert!(
        ctx.conn
            .select_page_target_for_connection_async("TID-B")
            .await
            .unwrap()
            .is_some()
    );

    ctx.process_async(json!({
        "id": 12,
        "method": "Page.navigate",
        "sessionId": "SID-B",
        "params": { "url": b1_url }
    }))
    .await;
    take_response_by_id(&mut ctx, 12);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 13,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-B"
    }))
    .await;
    let b_history = take_response_by_id(&mut ctx, 13);
    assert_eq!(b_history["result"]["currentIndex"], json!(0));
    assert_eq!(b_history["result"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(b_history["result"]["entries"][0]["url"], b1_url);
    ctx.sent.clear();

    {
        assert!(
            ctx.conn
                .select_page_target_for_connection_async("TID-A")
                .await
                .unwrap()
                .is_some()
        );
    }

    ctx.process_async(json!({
        "id": 14,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-A"
    }))
    .await;
    let a_history = take_response_by_id(&mut ctx, 14);
    assert_eq!(a_history["result"]["currentIndex"], json!(2));
    assert_eq!(a_history["result"]["entries"].as_array().unwrap().len(), 3);
    assert_eq!(a_history["result"]["entries"][0]["url"], "about:blank");
    assert_eq!(a_history["result"]["entries"][1]["url"], a1_url);
    assert_eq!(a_history["result"]["entries"][2]["url"], a2_url);
    assert!(
        a_history["result"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["url"] != b1_url)
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn get_navigation_history_targets_loaded_background_owner_without_activation() {
    let mut ctx = TestContext::new();
    let background_url = "data:text/html,<title>Background History</title><main>background</main>";
    let background = PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );

    let mut bc = BrowserContext::new("BID-1".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.set_target_url("data:text/html,<title>Active</title><main>active</main>".to_owned());
    bc.insert_page_target_host(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(background_url, Some("SID-background"))
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 15,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-background"
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 15);
    assert_eq!(history["result"]["currentIndex"], json!(0));
    assert_eq!(history["result"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(history["result"]["entries"][0]["url"], background_url);
    assert_eq!(
        history["result"]["entries"][0]["title"],
        "Background History"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Page.getNavigationHistory should not activate the target"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reset_navigation_history_targets_loaded_background_owner_without_activation() {
    let mut ctx = TestContext::new();
    let background_url =
        "data:text/html,<title>Background Reset History</title><main>background</main>";
    let background = PageTargetHost::with_url(
        "TID-background-reset".to_owned(),
        Some("SID-background-reset".to_owned()),
        "about:blank".to_owned(),
    );

    let mut browser_context = BrowserContext::new("BID-reset-background".to_owned());
    browser_context.set_active_target_id("TID-active".to_owned());
    browser_context.attach_active_session("SID-active".to_owned());
    browser_context
        .set_target_url("data:text/html,<title>Active</title><main>active</main>".to_owned());
    browser_context.insert_page_target_host(background);
    ctx.conn.browser_context = Some(browser_context);
    ctx.install_navigation_fixture_for_session_owner(background_url, Some("SID-background-reset"))
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1215,
        "method": "Page.resetNavigationHistory",
        "sessionId": "SID-background-reset"
    }))
    .await;
    let reset_response = take_response_by_id(&mut ctx, 1215);
    assert_eq!(reset_response["sessionId"], json!("SID-background-reset"));
    assert_eq!(reset_response["result"], json!({}));
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Page.resetNavigationHistory should not activate the target"
    );

    ctx.process_async(json!({
        "id": 1216,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-background-reset"
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 1216);
    assert_eq!(history["result"]["currentIndex"], json!(0));
    assert_eq!(
        history["result"]["entries"]
            .as_array()
            .expect("background history entries")
            .len(),
        1
    );
    assert_eq!(history["result"]["entries"][0]["url"], background_url);
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_to_history_entry_targets_background_owner_without_activation() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.set_target_url("data:text/html,<title>Active</title><main>active</main>".to_owned());
    bc.insert_page_target_host(PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    ));
    ctx.conn.browser_context = Some(bc);
    let first_url = "data:text/html,<title>Background A</title><main>a</main>";
    let second_url = "data:text/html,<title>Background B</title><main>b</main>";

    for (id, url) in [(17, first_url), (18, second_url)] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": "SID-background",
            "params": { "url": url }
        }))
        .await;
        take_response_by_id(&mut ctx, id);
        ctx.sent.clear();
    }

    ctx.process_async(json!({
        "id": 19,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-background"
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 19);
    let first_entry_id = history["result"]["entries"][0]["id"]
        .as_i64()
        .expect("first background history entry id");
    assert_eq!(history["result"]["currentIndex"], json!(1));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 20,
        "method": "Page.navigateToHistoryEntry",
        "sessionId": "SID-background",
        "params": { "entryId": first_entry_id }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 20);
    assert_eq!(response["sessionId"], json!("SID-background"));

    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(
        browser_context.active_target_id(),
        Some("TID-active"),
        "background Page.navigateToHistoryEntry should not activate the target"
    );
    let background = browser_context
        .background_target("TID-background")
        .expect("background target should remain background");
    assert_eq!(background.target_url(), first_url);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 21,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-background"
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 21);
    assert_eq!(history["result"]["currentIndex"], json!(0));
}
#[tokio::test(flavor = "multi_thread")]
async fn get_navigation_history_targets_inactive_loaded_owner_without_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-active",
        "TID-active",
        "SID-active",
        "about:blank",
    );
    let inactive_url = "data:text/html,<title>Inactive History</title><main>inactive</main>";
    let page = ctx
        .conn
        .load_page_via_runtime_async(inactive_url)
        .await
        .expect("inactive page should load");
    let mut inactive = BrowserContext::new("BID-inactive".to_owned());
    inactive.set_active_target_id("TID-inactive".to_owned());
    inactive.attach_active_session("SID-inactive".to_owned());
    inactive.set_target_url(page.final_url().as_str().to_owned());
    inactive
        .active_page_target_mut()
        .runtime_slot
        .replace_loaded_page(Some(page));
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 16,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-inactive"
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 16);
    assert_eq!(history["result"]["currentIndex"], json!(0));
    assert_eq!(history["result"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(history["result"]["entries"][0]["url"], inactive_url);
    assert_eq!(history["result"]["entries"][0]["title"], "Inactive History");
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .map(|browser_context| browser_context.id.as_str()),
        Some("BID-active"),
        "inactive Page.getNavigationHistory should not activate its browser context"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn navigation_history_marks_reload_as_reload_transition() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let url = "data:text/html,<title>Reload</title><main>reload</main>";

    ctx.process_async(json!({
        "id": 20,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;
    take_response_by_id(&mut ctx, 20);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 21,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-1"
    }))
    .await;
    let before_reload = take_response_by_id(&mut ctx, 21);
    assert_eq!(before_reload["result"]["currentIndex"], json!(1));
    assert_eq!(
        before_reload["result"]["entries"].as_array().unwrap().len(),
        2
    );
    assert_eq!(before_reload["result"]["entries"][0]["url"], "about:blank");
    let entry_id = before_reload["result"]["entries"][1]["id"]
        .as_i64()
        .expect("history entry id before reload");
    assert_eq!(
        before_reload["result"]["entries"][1]["transitionType"],
        "typed"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 22,
        "method": "Page.reload",
        "sessionId": "SID-1"
    }))
    .await;
    take_response_by_id(&mut ctx, 22);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 23,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-1"
    }))
    .await;
    let after_reload = take_response_by_id(&mut ctx, 23);
    assert_eq!(after_reload["result"]["currentIndex"], json!(1));
    assert_eq!(
        after_reload["result"]["entries"].as_array().unwrap().len(),
        2
    );
    assert_eq!(after_reload["result"]["entries"][0]["url"], "about:blank");
    assert_eq!(after_reload["result"]["entries"][1]["id"], json!(entry_id));
    assert_eq!(after_reload["result"]["entries"][1]["url"], url);
    assert_eq!(
        after_reload["result"]["entries"][1]["transitionType"],
        "reload"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_targets_background_owner_without_activation() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.set_target_url("data:text/html,<body>active</body>".to_owned());
    bc.insert_page_target_host(PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    ));
    ctx.conn.browser_context = Some(bc);

    let background_url = "data:text/html,<title>Background</title><main>background</main>";
    ctx.process_async(json!({
        "id": 1204,
        "method": "Page.navigate",
        "sessionId": "SID-background",
        "params": { "url": background_url }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1204);
    assert_eq!(response["sessionId"], json!("SID-background"));
    assert_eq!(response["result"]["frameId"], json!("TID-background"));

    let browser_context = ctx.conn.browser_context.as_ref().unwrap();
    assert_eq!(
        browser_context.active_target_id(),
        Some("TID-active"),
        "background Page.navigate should not activate the target"
    );
    assert_eq!(
        browser_context.target_url(),
        "data:text/html,<body>active</body>",
        "background Page.navigate should not rewrite the active target identity"
    );
    let background = browser_context
        .background_target("TID-background")
        .expect("background target should remain background");
    assert!(background.has_loaded_page());
    assert_eq!(background.target_url(), background_url);
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_targets_inactive_owner_without_activation() {
    let mut ctx = TestContext::new();
    let mut inactive = BrowserContext::new("BID-inactive".to_owned());
    inactive.set_active_target_id("TID-inactive".to_owned());
    inactive.attach_active_session("SID-inactive".to_owned());
    inactive.set_target_url("about:blank".to_owned());
    ctx.conn.inactive_browser_contexts.push(inactive);

    let inactive_url = "data:text/html,<title>Inactive</title><main>inactive</main>";
    ctx.process_async(json!({
        "id": 1205,
        "method": "Page.navigate",
        "sessionId": "SID-inactive",
        "params": { "url": inactive_url }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1205);
    assert_eq!(response["sessionId"], json!("SID-inactive"));
    assert_eq!(response["result"]["frameId"], json!("TID-inactive"));
    assert!(
        ctx.conn.browser_context.is_none(),
        "inactive Page.navigate should not activate its browser context"
    );
    let inactive = ctx
        .conn
        .inactive_browser_contexts
        .iter()
        .find(|browser_context| browser_context.id == "BID-inactive")
        .expect("inactive browser context should stay background");
    assert!(inactive.has_loaded_page());
    assert_eq!(inactive.target_url(), inactive_url);
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_post_parse_location_download_keeps_loaded_document_and_emits_download() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}/page");
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/page",
                axum::routing::get(|| async move {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><html><body><main id=\"source\">source</main><script>setTimeout(() => location.assign('/download'), 0);</script></body></html>",
                    )
                }),
            )
            .route(
                "/download",
                axum::routing::get(|| async move {
                    (
                        [
                            (axum::http::header::CONTENT_TYPE.as_str(), "text/plain"),
                            (
                                axum::http::header::CONTENT_DISPOSITION.as_str(),
                                "attachment; filename=\"saved.txt\"",
                            ),
                        ],
                        "download-body",
                    )
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let download_root = std::env::temp_dir().join(format!(
        "moli-cdp-post-parse-download-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-POST-PARSE-DOWNLOAD",
        "TID-POST-PARSE-DOWNLOAD",
        "SID-POST-PARSE-DOWNLOAD",
        "about:blank",
    );

    ctx.process_async(json!({
        "id": 200,
        "method": "Browser.setDownloadBehavior",
        "params": {
            "behavior": "allowAndName",
            "downloadPath": download_root.to_string_lossy(),
            "eventsEnabled": true
        }
    }))
    .await;
    ctx.expect_result(200, json!({}), None);

    ctx.process_async(json!({
        "id": 201,
        "method": "Page.navigate",
        "sessionId": "SID-POST-PARSE-DOWNLOAD",
        "params": { "url": url }
    }))
    .await;

    consume_main_document_navigation_start(&mut ctx);
    let navigation = take_response_by_id(&mut ctx, 201);
    assert_eq!(
        navigation["result"]["frameId"],
        json!("TID-POST-PARSE-DOWNLOAD")
    );
    assert!(
        navigation["result"].get("isDownload").is_none(),
        "post-parse location download should keep the source document navigation result loaded: {navigation:?}"
    );
    assert!(
        navigation["result"].get("loaderId").is_some(),
        "loaded navigation should still expose loaderId: {navigation:?}"
    );

    wait_until_message(
        &mut ctx,
        None,
        "post-parse location download will begin",
        |message| message["method"] == json!("Browser.downloadWillBegin"),
    )
    .await;
    wait_until_message(
        &mut ctx,
        None,
        "post-parse location download completion",
        |message| {
            message["method"] == json!("Browser.downloadProgress")
                && message["params"]["state"] == json!("completed")
        },
    )
    .await;

    let sent = ctx.take_all();
    let will_begin = sent
        .iter()
        .find(|message| message["method"] == json!("Browser.downloadWillBegin"))
        .expect("post-parse location download should emit Browser.downloadWillBegin");
    assert_eq!(
        will_begin["params"]["suggestedFilename"],
        json!("saved.txt")
    );
    let completed = sent
        .iter()
        .find(|message| {
            message["method"] == json!("Browser.downloadProgress")
                && message["params"]["state"] == json!("completed")
        })
        .expect("post-parse location download should emit completed progress");
    let guid = completed["params"]["guid"]
        .as_str()
        .expect("completed download should include guid");
    let artifact_path = download_root.join(guid);
    let body = std::fs::read_to_string(&artifact_path).expect("download artifact should exist");
    assert_eq!(body, "download-body");

    ctx.process_async(json!({
        "id": 202,
        "method": "Runtime.evaluate",
        "sessionId": "SID-POST-PARSE-DOWNLOAD",
        "params": {
            "expression": "location.pathname + '|' + document.getElementById('source').id"
        }
    }))
    .await;
    let evaluate = take_response_by_id(&mut ctx, 202);
    assert_eq!(evaluate["result"]["result"]["value"], json!("/page|source"));

    let _ = std::fs::remove_dir_all(&download_root);
    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn reload_requires_browser_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 13, "method": "Page.reload"}))
        .await;
    ctx.expect_error(13, -31998, "BrowserContextNotLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn reload_without_target_loaded_errors() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));
    ctx.process_async(json!({"id": 14, "method": "Page.reload"}))
        .await;
    ctx.expect_error(14, -31998, "TargetNotLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_runtime_frontend_enabled_network_child_playwright_style_utility_script_uses_child_scope()
 {
    async fn parent() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><iframe src=\"/child\"></iframe></body></html>",
        )
    }

    async fn child() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>child-navigate-playwright-eval</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/parent", axum::routing::get(parent))
                .route("/child", axum::routing::get(child)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    ctx.enable_background_navigation_scheduler_for_test();

    tokio::task::LocalSet::new()
        .run_until(async {
            ctx.process_and_wait_for_response_async(json!({
                "id": 4100,
                "method": "Page.navigate",
                "sessionId": "SID-1",
                "params": { "url": format!("http://{addr}/parent") }
            }))
            .await;
            let _ = take_response_by_id(&mut ctx, 4100);

            wait_until_message(
                &mut ctx,
                "SID-1",
                "navigate child frame attached",
                |message| {
                    message["method"] == json!("Page.frameAttached")
                        && message["params"]["parentFrameId"] == json!("TID-1")
                },
            )
            .await;
            let child_frame_id = ctx
                .sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Page.frameAttached")
                        && message["params"]["parentFrameId"] == json!("TID-1")
                })
                .and_then(|message| message["params"]["frameId"].as_str())
                .map(str::to_owned)
                .expect("child frame should emit Page.frameAttached");
            wait_until_message(
                &mut ctx,
                "SID-1",
                "navigate child default execution context",
                |message| {
                    message["method"] == json!("Runtime.executionContextCreated")
                        && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                        && message["params"]["context"]["auxData"]["frameId"]
                            == json!(child_frame_id)
                },
            )
            .await;
            let child_default_context_id = ctx
                .sent
                .iter()
                .find(|message| {
                    message["method"] == json!("Runtime.executionContextCreated")
                        && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                        && message["params"]["context"]["auxData"]["frameId"]
                            == json!(child_frame_id)
                })
                .and_then(|message| message["params"]["context"]["id"].as_i64())
                .expect("child default execution context id");

            ctx.sent.clear();

            ctx.process_async(json!({
                "id": 4101,
                "method": "Runtime.evaluate",
                "sessionId": "SID-1",
                "params": {
                    "contextId": child_default_context_id,
                    "expression": "(() => { const module = { exports: {} }; class UtilityScript { constructor(global, isUnderTest) { this.global = global; this.isUnderTest = isUnderTest; } evaluate(isFunction, returnByValue, expression, argCount, ...argsAndHandles) { const args = argsAndHandles.slice(0, argCount); let result = this.global.eval(expression); if (isFunction === true) { result = result(...args); } else if (isFunction === false) { result = result; } else if (typeof result === 'function') { result = result(...args); } return returnByValue ? result : result; } } module.exports.UtilityScript = () => UtilityScript; return new (module.exports.UtilityScript())(globalThis, false); })()"
                }
            }))
            .await;
            let utility_response = take_response_by_id(&mut ctx, 4101);
            let object_id = utility_response["result"]["result"]["objectId"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    panic!("playwright-style utility object id: {utility_response:?}")
                });

            ctx.process_async(json!({
                "id": 4102,
                "method": "Runtime.callFunctionOn",
                "sessionId": "SID-1",
                "params": {
                    "objectId": object_id.clone(),
                    "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
                    "arguments": [
                        { "objectId": object_id },
                        { "value": {} },
                        { "value": true },
                        { "value": "document.body.textContent.trim()" },
                        { "value": 1 },
                        { "value": null }
                    ],
                    "returnByValue": true,
                    "awaitPromise": true
                }
            }))
            .await;
            let result = take_response_by_id(&mut ctx, 4102);
            assert_eq!(
                result["result"]["result"]["value"],
                json!("child-navigate-playwright-eval")
            );
        })
        .await;

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_emits_frame_and_load_events_in_order() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_dom_events_for_test(Some("SID-1"));

    ctx.process_and_wait_for_response_async(json!({
        "id": 20,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>hi</body>" }
    }))
    .await;
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;

    let started_navigating = ctx.take_one();
    assert_eq!(started_navigating["method"], "Page.frameStartedNavigating");
    assert_eq!(started_navigating["sessionId"], "SID-1");
    assert_eq!(started_navigating["params"]["frameId"], "TID-1");
    assert_eq!(started_navigating["params"]["loaderId"], LOADER_ID);
    assert_eq!(
        started_navigating["params"]["url"],
        "data:text/html,<body>hi</body>"
    );
    assert_eq!(
        started_navigating["params"]["navigationType"],
        "differentDocument"
    );

    let started_loading = ctx.take_one();
    assert_eq!(started_loading["method"], "Page.frameStartedLoading");
    assert_eq!(started_loading["sessionId"], "SID-1");
    assert_eq!(started_loading["params"]["frameId"], "TID-1");

    let result = ctx.take_one();
    assert_eq!(result["id"], 20);
    assert_eq!(result["sessionId"], "SID-1");
    assert_eq!(
        result["result"],
        json!({
            "frameId": "TID-1",
            "loaderId": LOADER_ID,
        })
    );

    let frame_navigated = ctx.take_one();
    assert_eq!(frame_navigated["method"], "Page.frameNavigated");
    assert_eq!(frame_navigated["sessionId"], "SID-1");
    assert_eq!(frame_navigated["params"]["type"], "Navigation");
    assert_eq!(frame_navigated["params"]["frame"]["id"], "TID-1");
    assert_eq!(frame_navigated["params"]["frame"]["loaderId"], LOADER_ID);
    assert_eq!(
        frame_navigated["params"]["frame"]["url"],
        "data:text/html,<body>hi</body>"
    );

    let document_updated = ctx.take_one();
    assert_eq!(document_updated["method"], "DOM.documentUpdated");
    assert_eq!(document_updated["sessionId"], "SID-1");

    let parse_completed_document_updated = ctx.take_one();
    assert_eq!(
        parse_completed_document_updated["method"],
        "DOM.documentUpdated"
    );
    assert_eq!(parse_completed_document_updated["sessionId"], "SID-1");

    let dom_content_loaded = ctx.take_one();
    assert_eq!(dom_content_loaded["method"], "Page.domContentEventFired");
    assert_eq!(dom_content_loaded["sessionId"], "SID-1");
    assert!(dom_content_loaded["params"]["timestamp"].as_f64().is_some());

    let load_event = ctx.take_one();
    assert_eq!(load_event["method"], "Page.loadEventFired");
    assert_eq!(load_event["sessionId"], "SID-1");
    assert!(load_event["params"]["timestamp"].as_f64().is_some());

    let stopped_loading = ctx.take_one();
    assert_eq!(stopped_loading["method"], "Page.frameStoppedLoading");
    assert_eq!(stopped_loading["sessionId"], "SID-1");
    assert_eq!(stopped_loading["params"]["frameId"], "TID-1");
    assert!(ctx.sent.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_tail_dom_mutations_precede_the_dcl_binding_refresh() {
    let script_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_script = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_requested = script_requested.clone();
    let handler_release = release_script.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_html = format!(
        "<!doctype html><html><head><script src='http://{addr}/held.js'></script></head>\
         <body id='late-body'><main>ready</main></body></html>"
    );
    let server = tokio::spawn(async move {
        let page = page_html.clone();
        let app = axum::Router::new()
            .route(
                "/page",
                axum::routing::get(move || {
                    let html = page.clone();
                    async move {
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                            html,
                        )
                    }
                }),
            )
            .route(
                "/held.js",
                axum::routing::get(move || {
                    let requested = handler_requested.clone();
                    let release = handler_release.clone();
                    async move {
                        requested.notify_one();
                        release.notified().await;
                        (
                            [(
                                axum::http::header::CONTENT_TYPE.as_str(),
                                "application/javascript",
                            )],
                            "globalThis.__heldParserScriptRan = true;",
                        )
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-parser-tail", "TID-1", "SID-1", "about:blank");
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>initial</body>",
        Some("SID-1"),
    )
    .await;
    wait_until_renderer_document_load(&mut ctx, Some("SID-1"), "TID-1", LOADER_ID).await;
    for (id, method) in [(30, "Page.enable"), (31, "DOM.enable")] {
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "sessionId": "SID-1",
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
    }
    ctx.sent.clear();
    ctx.enable_background_navigation_scheduler_for_test();

    tokio::task::LocalSet::new()
        .run_until(async {
            ctx.process_and_wait_for_response_async(json!({
                "id": 32,
                "method": "Page.navigate",
                "sessionId": "SID-1",
                "params": { "url": format!("http://{addr}/page") }
            }))
            .await;
            let navigation = take_response_by_id(&mut ctx, 32);
            assert_eq!(navigation["result"]["frameId"], json!("TID-1"));

            wait_until_scheduler_message(&mut ctx, "held parser document commit", |message| {
                message["method"] == json!("Page.frameNavigated")
            })
            .await;

            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                script_requested.notified(),
            )
            .await
            .expect("the parser-blocking script request should reach the fixture");

            ctx.process_and_wait_for_response_async(json!({
                "id": 33,
                "method": "DOM.getDocument",
                "sessionId": "SID-1",
                "params": { "depth": -1 }
            }))
            .await;
            let early_root = take_response_by_id(&mut ctx, 33)["result"]["root"].clone();
            assert!(
                find_cdp_node_by_local_name(&early_root, "body").is_none(),
                "the held parser must expose the same incomplete pre-BODY snapshot as Chromium: \
                 {early_root:?}"
            );
            let early_root_node_id = early_root["nodeId"]
                .as_u64()
                .expect("early document frontend node id");
            let before_release = ctx.take_all();
            assert!(
                before_release
                    .iter()
                    .any(|message| message["method"] == json!("Page.frameNavigated")),
                "the new Document must commit before its parser tail resumes: {before_release:?}"
            );
            assert_eq!(
                before_release
                    .iter()
                    .filter(|message| message["method"] == json!("DOM.documentUpdated"))
                    .count(),
                1,
                "Chromium publishes one pre-parser commit binding barrier: {before_release:?}"
            );
            assert!(
                before_release
                    .iter()
                    .all(|message| message["method"] != json!("Page.domContentEventFired")),
                "DOMContentLoaded must remain behind the parser: {before_release:?}"
            );

            release_script.notify_one();
            wait_until_scheduler_message(
                &mut ctx,
                "main document DCL DOM binding refresh",
                |message| message["method"] == json!("DOM.documentUpdated"),
            )
            .await;
            wait_until_scheduler_message(&mut ctx, "main document DOMContentLoaded", |message| {
                message["method"] == json!("Page.domContentEventFired")
            })
            .await;
            let completed = ctx.take_all();
            let body_inserted_index = completed
                .iter()
                .position(|message| {
                    message["method"] == json!("DOM.childNodeInserted")
                        && message["params"]["node"]["localName"] == json!("body")
                })
                .unwrap_or_else(|| panic!("missing parser-tail BODY insertion: {completed:?}"));
            let document_updated_indices = completed
                .iter()
                .enumerate()
                .filter_map(|(index, message)| {
                    (message["method"] == json!("DOM.documentUpdated")).then_some(index)
                })
                .collect::<Vec<_>>();
            assert_eq!(
                document_updated_indices.len(),
                1,
                "DCL must refresh frontend bindings exactly once after parser resumption: \
                 {completed:?}"
            );
            let document_updated_index = document_updated_indices[0];
            let dom_content_loaded_index = completed
                .iter()
                .position(|message| message["method"] == json!("Page.domContentEventFired"))
                .unwrap_or_else(|| panic!("missing Page.domContentEventFired: {completed:?}"));
            assert!(
                body_inserted_index < document_updated_index
                    && document_updated_index < dom_content_loaded_index,
                "parser mutations must precede the DCL binding barrier and Page lifecycle: \
                 {completed:?}"
            );

            ctx.process_and_wait_for_response_async(json!({
                "id": 34,
                "method": "DOM.describeNode",
                "sessionId": "SID-1",
                "params": { "nodeId": early_root_node_id }
            }))
            .await;
            let stale_node = take_response_by_id(&mut ctx, 34);
            assert_eq!(stale_node["error"]["code"], json!(-32000));
            assert_eq!(
                stale_node["error"]["message"],
                json!("Could not find node with given id")
            );

            ctx.process_and_wait_for_response_async(json!({
                "id": 35,
                "method": "DOM.getDocument",
                "sessionId": "SID-1",
                "params": { "depth": -1 }
            }))
            .await;
            let refreshed_root = take_response_by_id(&mut ctx, 35)["result"]["root"].clone();
            let body = find_cdp_node_by_local_name(&refreshed_root, "body").unwrap_or_else(|| {
                panic!("refreshed document must contain BODY: {refreshed_root:?}")
            });
            assert_eq!(body["attributes"], json!(["id", "late-body"]));
            assert!(
                find_cdp_node_by_local_name(body, "main").is_some(),
                "the refreshed BODY snapshot must include the parser tail: {body:?}"
            );
        })
        .await;

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn page_navigate_does_not_mask_later_renderer_requested_navigation() {
    async fn source() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>source</title>",
        )
    }

    async fn child() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>child</title>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/source", axum::routing::get(source))
                .route("/child", axum::routing::get(child)),
        )
        .await
        .unwrap();
    });
    let source_url = format!("http://{addr}/source");
    let child_url = format!("http://{addr}/child");
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));

    ctx.process_and_wait_for_response_async(json!({
        "id": 20_100,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": source_url }
    }))
    .await;
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let _ = take_response_by_id(&mut ctx, 20_100);
    assert!(
        !ctx.sent.iter().any(|message| matches!(
            message["method"].as_str(),
            Some("Page.frameScheduledNavigation" | "Page.frameRequestedNavigation")
        )),
        "browser-initiated Page.navigate must not emit renderer navigation probes: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 20_101,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "location.assign('/child'); 'ok'",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 20_101);
    assert_eq!(response["result"]["result"]["value"], json!("ok"));
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "renderer-requested child navigation",
        |message| message["method"] == json!("Page.frameRequestedNavigation"),
    )
    .await;
    let requested_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.frameRequestedNavigation"))
        .expect("renderer navigation should emit frameRequestedNavigation");
    assert_eq!(ctx.sent[requested_index]["params"]["url"], json!(child_url));
    let cleared_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.frameClearedScheduledNavigation"))
        .expect("renderer navigation should clear its scheduled probe before load start");
    let started_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.frameStartedNavigating"))
        .expect("renderer navigation should start loading");
    assert!(requested_index < cleared_index && cleared_index < started_index);

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_location_reload_keeps_browser_navigation_headers() {
    let (request_tx, mut request_rx) = tokio::sync::mpsc::unbounded_channel();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/page",
                axum::routing::get(move |headers: axum::http::HeaderMap| {
                    let request_tx = request_tx.clone();
                    async move {
                        request_tx.send(headers).unwrap();
                        (
                            [
                                (axum::http::header::CONTENT_TYPE.as_str(), "text/html"),
                                ("set-cookie", "sid=1; Path=/; SameSite=Lax"),
                            ],
                            "<!doctype html><title>reload headers</title>",
                        )
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.process_and_wait_for_response_async(json!({
        "id": 20_109,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let url = format!("http://{addr}/page");

    ctx.process_and_wait_for_response_async(json!({
        "id": 20_110,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let _ = request_rx.recv().await.expect("initial navigation request");
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 20_111,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "location.reload(); 'reload requested'",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 20_111)["result"]["result"]["value"],
        json!("reload requested")
    );
    let headers = tokio::time::timeout(std::time::Duration::from_secs(5), request_rx.recv())
        .await
        .expect("renderer reload should reach the server")
        .expect("renderer reload request headers");
    let value = |name: &'static str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };

    assert_eq!(
        value(axum::http::header::ACCEPT.as_str()).as_deref(),
        Some(
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"
        )
    );
    assert_eq!(
        value(axum::http::header::ACCEPT_LANGUAGE.as_str()).as_deref(),
        Some("en-US,en;q=0.9")
    );
    assert_eq!(value("sec-fetch-mode").as_deref(), Some("navigate"));
    assert_eq!(value("sec-fetch-dest").as_deref(), Some("document"));
    assert_eq!(value("sec-fetch-site").as_deref(), Some("same-origin"));
    assert_eq!(value("referer").as_deref(), Some(url.as_str()));
    assert_eq!(value("cache-control").as_deref(), Some("max-age=0"));
    assert!(value("sec-ch-ua").is_some());
    assert!(value("sec-ch-ua-mobile").is_some());
    assert!(value("sec-ch-ua-platform").is_some());

    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let messages = ctx.take_all();
    let request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"] == json!(url)
        })
        .expect("reload should emit requestWillBeSent");
    let request_id = request["params"]["requestId"].clone();
    let extra_info = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSentExtraInfo")
                && message["params"]["requestId"] == request_id
        })
        .expect("reload should emit requestWillBeSentExtraInfo");
    assert_eq!(
        extra_info["params"]["headers"]["Cache-Control"],
        json!("max-age=0")
    );
    assert!(extra_info["params"]["headers"]["Accept"].is_string());
    assert_eq!(
        extra_info["params"]["headers"]["Sec-Fetch-Mode"],
        json!("navigate")
    );
    assert_eq!(extra_info["params"]["headers"]["Cookie"], json!("sid=1"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn renderer_top_level_form_post_preserves_request_through_document_commit() {
    let (request_tx, mut request_rx) = tokio::sync::mpsc::unbounded_channel();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let submit_tx = request_tx.clone();
        let app = axum::Router::new()
            .route(
                "/source",
                axum::routing::get(|| async {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><title>source</title><main>source</main>",
                    )
                }),
            )
            .route(
                "/submit",
                axum::routing::post(
                    move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                        let submit_tx = submit_tx.clone();
                        async move {
                            let content_type = headers
                                .get(axum::http::header::CONTENT_TYPE)
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned);
                            let _ = submit_tx.send((content_type, body.to_vec()));
                            (
                                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                                "<!doctype html><title>posted</title><main id=posted>committed POST response</main>",
                            )
                        }
                    },
                ),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-POST", "TID-POST", "SID-POST", "about:blank");
    ctx.process_async(json!({
        "id": 20_200,
        "method": "Network.enable",
        "sessionId": "SID-POST"
    }))
    .await;
    take_response_by_id(&mut ctx, 20_200);
    ctx.process_and_wait_for_response_async(json!({
        "id": 20_201,
        "method": "Page.navigate",
        "sessionId": "SID-POST",
        "params": { "url": format!("http://{addr}/source") }
    }))
    .await;
    let initial_navigation = take_response_by_id(&mut ctx, 20_201);
    let initial_loader_id = initial_navigation["result"]["loaderId"]
        .as_str()
        .expect("initial navigation should return a loader id")
        .to_owned();
    wait_until_renderer_document_load(&mut ctx, Some("SID-POST"), "TID-POST", &initial_loader_id)
        .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 20_202,
        "method": "Runtime.evaluate",
        "sessionId": "SID-POST",
        "params": {
            "expression": r#"
(() => {
  const form = document.createElement('form');
  form.method = 'post';
  form.action = '/submit?existing=1';
  const input = document.createElement('input');
  input.name = 'a b';
  input.value = 'c+d';
  form.appendChild(input);
  document.body.appendChild(form);
  form.submit();
  return 'submitted';
})()
"#,
            "returnByValue": true
        }
    }))
    .await;
    let evaluation = take_response_by_id(&mut ctx, 20_202);
    assert_eq!(evaluation["result"]["result"]["value"], json!("submitted"));

    let (content_type, request_body) =
        tokio::time::timeout(std::time::Duration::from_secs(5), request_rx.recv())
            .await
            .expect("top-level POST should reach the loopback server")
            .expect("top-level POST request channel should remain open");
    assert_eq!(
        content_type.as_deref(),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(request_body, b"a+b=c%2Bd");

    wait_until_message(
        &mut ctx,
        "SID-POST",
        "top-level POST Network request",
        |message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"]
                    == json!(format!("http://{addr}/submit?existing=1"))
        },
    )
    .await;
    let post_request = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["request"]["url"]
                    == json!(format!("http://{addr}/submit?existing=1"))
        })
        .unwrap_or_else(|| {
            panic!(
                "POST navigation should publish a Network request: {:?}",
                ctx.sent
            )
        });
    let post_loader_id = post_request["params"]["loaderId"]
        .as_str()
        .expect("top-level POST request should have a loader id")
        .to_owned();
    assert_eq!(post_request["params"]["request"]["method"], json!("POST"));
    assert_eq!(
        post_request["params"]["request"]["postData"],
        json!("a+b=c%2Bd")
    );
    assert_eq!(
        post_request["params"]["request"]["headers"]["Content-Type"],
        json!("application/x-www-form-urlencoded")
    );
    wait_until_renderer_document_load(&mut ctx, Some("SID-POST"), "TID-POST", &post_loader_id)
        .await;
    assert!(
        loaded_page_html_for_test(&mut ctx)
            .await
            .contains("committed POST response"),
        "the POST response should replace the source Document"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_lifecycle_events_enabled_emits_lifecycle_markers() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_dom_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;

    ctx.process_and_wait_for_response_async(json!({
        "id": 21,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>hi</body>" }
    }))
    .await;
    wait_until_scheduler_message(&mut ctx, "networkIdle lifecycle marker", |message| {
        message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["frameId"] == json!("TID-1")
            && message["params"]["name"] == json!("networkIdle")
    })
    .await;
    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;

    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let started_navigating = ctx.take_one();
    assert_eq!(started_navigating["method"], "Page.frameStartedNavigating");
    assert_eq!(ctx.take_one()["method"], "Page.frameStartedLoading");
    let navigate_response = ctx.take_one();
    assert_eq!(navigate_response["id"], 21);
    let loader_id = navigate_response["result"]["loaderId"]
        .as_str()
        .expect("Page.navigate loaderId")
        .to_owned();
    assert_eq!(
        started_navigating["params"]["loaderId"].as_str(),
        Some(loader_id.as_str())
    );

    let init = ctx.take_one();
    assert_eq!(init["method"], "Page.lifecycleEvent");
    assert_eq!(init["sessionId"], "SID-1");
    assert_eq!(init["params"]["name"], "init");
    assert_eq!(init["params"]["frameId"], "TID-1");
    assert_eq!(
        init["params"]["loaderId"].as_str(),
        Some(loader_id.as_str())
    );
    assert!(init["params"]["timestamp"].as_f64().is_some());

    let frame_navigated = ctx.take_one();
    assert_eq!(frame_navigated["method"], "Page.frameNavigated");
    assert_eq!(
        frame_navigated["params"]["frame"]["loaderId"].as_str(),
        Some(loader_id.as_str())
    );
    assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
    assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
    assert_eq!(ctx.take_one()["method"], "Page.domContentEventFired");

    let dom_lifecycle = ctx.take_one();
    assert_eq!(dom_lifecycle["method"], "Page.lifecycleEvent");
    assert_eq!(dom_lifecycle["params"]["name"], "DOMContentLoaded");
    assert_eq!(dom_lifecycle["params"]["frameId"], "TID-1");
    assert_eq!(
        dom_lifecycle["params"]["loaderId"].as_str(),
        Some(loader_id.as_str())
    );
    assert!(dom_lifecycle["params"]["timestamp"].as_f64().is_some());

    assert_eq!(ctx.take_one()["method"], "Page.loadEventFired");

    let load_lifecycle = ctx.take_one();
    assert_eq!(load_lifecycle["method"], "Page.lifecycleEvent");
    assert_eq!(load_lifecycle["params"]["name"], "load");
    assert_eq!(load_lifecycle["params"]["frameId"], "TID-1");
    assert_eq!(
        load_lifecycle["params"]["loaderId"].as_str(),
        Some(loader_id.as_str())
    );
    assert!(load_lifecycle["params"]["timestamp"].as_f64().is_some());

    let almost_idle = ctx.take_one();
    assert_eq!(almost_idle["method"], "Page.lifecycleEvent");
    assert_eq!(almost_idle["params"]["name"], "networkAlmostIdle");
    assert_eq!(almost_idle["params"]["frameId"], "TID-1");
    assert_eq!(
        almost_idle["params"]["loaderId"].as_str(),
        Some(loader_id.as_str())
    );
    assert!(almost_idle["params"]["timestamp"].as_f64().is_some());

    let idle = ctx.take_one();
    assert_eq!(idle["method"], "Page.lifecycleEvent");
    assert_eq!(idle["params"]["name"], "networkIdle");
    assert_eq!(idle["params"]["frameId"], "TID-1");
    assert_eq!(
        idle["params"]["loaderId"].as_str(),
        Some(loader_id.as_str())
    );
    assert!(idle["params"]["timestamp"].as_f64().is_some());

    assert_eq!(ctx.take_one()["method"], "Page.frameStoppedLoading");
    assert!(ctx.sent.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_target_discovery_emits_target_info_changed() {
    let mut ctx = TestContext::new_with_target_discovery(false);
    load_bc_with_session(
        &mut ctx,
        "BID-target-info-navigation",
        "TID-target-info-navigation",
        "SID-target-info-navigation",
        "about:blank",
    );

    ctx.process_async(json!({
        "id": 22,
        "method": "Target.setDiscoverTargets",
        "params": { "discover": true }
    }))
    .await;
    ctx.expect_result(22, json!({}), None);
    ctx.expect_event("Target.targetCreated", None);

    let url = "data:text/html,<title>Target Info Title</title><main>target info navigation</main>";
    ctx.process_async(json!({
        "id": 23,
        "method": "Page.navigate",
        "sessionId": "SID-target-info-navigation",
        "params": { "url": url }
    }))
    .await;

    wait_until_message(
        &mut ctx,
        None,
        "parsed document title targetInfoChanged",
        |message| {
            message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"]
                    == json!("TID-target-info-navigation")
                && message["params"]["targetInfo"]["title"] == json!("Target Info Title")
        },
    )
    .await;
    let changed = ctx.take_first_matching("Target.targetInfoChanged", |message| {
        message["method"] == json!("Target.targetInfoChanged")
            && message["params"]["targetInfo"]["targetId"] == json!("TID-target-info-navigation")
            && message["params"]["targetInfo"]["title"] == json!("Target Info Title")
    });
    assert_eq!(
        changed["params"]["targetInfo"]["targetId"],
        json!("TID-target-info-navigation")
    );
    assert_eq!(changed["params"]["targetInfo"]["url"], json!(url));
    assert_eq!(
        changed["params"]["targetInfo"]["title"],
        json!("Target Info Title")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_child_iframe_emits_child_frame_navigation_and_lifecycle_events() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;

    ctx.process_async(json!({
        "id": 521,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe name='child-frame' srcdoc=\"<body>child</body>\"></iframe>"
        }
    })).await;

    let _ = take_response_by_id(&mut ctx, 521);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "child frame navigation after Page.navigate response",
        |message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["name"] == json!("child-frame")
        },
    )
    .await;
    let child_navigated = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["name"] == json!("child-frame")
        })
        .cloned()
        .expect("child frame should emit Page.frameNavigated");
    let child_frame_id = child_navigated["params"]["frame"]["id"]
        .as_str()
        .expect("child frame id")
        .to_owned();
    assert_ne!(child_frame_id, "TID-1");
    assert_eq!(
        child_navigated["params"]["frame"]["parentId"],
        json!("TID-1")
    );
    assert_child_frame_attached(&ctx, &child_frame_id, "TID-1");
    assert_child_frame_navigation_completion(&mut ctx, &child_frame_id, Some("child-frame"), None)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_removing_child_iframe_emits_frame_detached_and_forgets_owner_state() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));

    ctx.process_async(json!({
        "id": 5242,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe id='removable' srcdoc=\"<body>child</body>\"></iframe>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 5242);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "removable child frame attachment after Page.navigate response",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame should emit Page.frameAttached");
    assert!(ctx.conn.has_attached_child_frame_id(&child_frame_id));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 5243,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.getElementById('removable').remove(); true",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 5243);
    assert_eq!(response["result"]["result"]["value"], json!(true));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "removed child frameDetached",
        |message| {
            message["method"] == json!("Page.frameDetached")
                && message["params"]["frameId"] == json!(child_frame_id)
        },
    )
    .await;

    let detached = ctx
        .sent
        .iter()
        .filter(|message| {
            message["method"] == json!("Page.frameDetached")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        detached.len(),
        1,
        "frame detach must be emitted exactly once"
    );
    assert_eq!(detached[0]["params"]["reason"], json!("remove"));
    assert_eq!(detached[0]["sessionId"], json!("SID-1"));
    assert!(
        !ctx.conn.has_attached_child_frame_id(&child_frame_id),
        "detached frame must be removed from protocol owner state"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_insert_then_remove_iframe_preserves_attach_before_detach() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 5244,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { const frame = document.createElement('iframe'); document.body.appendChild(frame); frame.remove(); return true; })()",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 5244);
    assert_eq!(response["result"]["result"]["value"], json!(true));
    wait_until_messages(
        &mut ctx,
        "SID-1",
        "same-command child frameAttached followed by frameDetached",
        |messages| {
            messages
                .iter()
                .enumerate()
                .any(|(attached_index, attached)| {
                    if attached["method"] != json!("Page.frameAttached") {
                        return false;
                    }
                    messages.iter().skip(attached_index + 1).any(|detached| {
                        detached["method"] == json!("Page.frameDetached")
                            && detached["params"]["frameId"] == attached["params"]["frameId"]
                    })
                })
        },
    )
    .await;

    let attached_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.frameAttached"))
        .expect("same-command insertion should emit Page.frameAttached");
    let child_frame_id = ctx.sent[attached_index]["params"]["frameId"]
        .as_str()
        .expect("attached child frame id")
        .to_owned();
    let detached_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameDetached")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .expect("same-command removal should emit Page.frameDetached");
    assert!(attached_index < detached_index);
    assert_eq!(
        ctx.sent[detached_index]["params"]["reason"],
        json!("remove")
    );
    assert!(!ctx.conn.has_attached_child_frame_id(&child_frame_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_removing_nested_iframe_detaches_descendant_before_parent() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));

    ctx.process_async(json!({
        "id": 5245,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe id='outer' srcdoc=\"<iframe srcdoc='<body>inner</body>'></iframe>\"></iframe>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 5245);
    wait_until_messages(
        &mut ctx,
        "SID-1",
        "nested child frame attachments",
        |messages| {
            messages
                .iter()
                .find(|message| {
                    message["method"] == json!("Page.frameAttached")
                        && message["params"]["parentFrameId"] == json!("TID-1")
                })
                .and_then(|message| message["params"]["frameId"].as_str())
                .is_some_and(|outer_frame_id| {
                    messages.iter().any(|message| {
                        message["method"] == json!("Page.frameAttached")
                            && message["params"]["parentFrameId"] == json!(outer_frame_id)
                    })
                })
        },
    )
    .await;
    let outer_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("outer child frame should attach");
    let inner_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!(outer_frame_id)
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("inner child frame should attach");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 5246,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.getElementById('outer').remove(); true",
            "returnByValue": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 5246);
    wait_until_messages(
        &mut ctx,
        "SID-1",
        "nested child frameDetached events",
        |messages| {
            [inner_frame_id.as_str(), outer_frame_id.as_str()]
                .into_iter()
                .all(|frame_id| {
                    messages.iter().any(|message| {
                        message["method"] == json!("Page.frameDetached")
                            && message["params"]["frameId"] == json!(frame_id)
                    })
                })
        },
    )
    .await;

    let inner_detached_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameDetached")
                && message["params"]["frameId"] == json!(inner_frame_id)
        })
        .expect("inner frame should detach");
    let outer_detached_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameDetached")
                && message["params"]["frameId"] == json!(outer_frame_id)
        })
        .expect("outer frame should detach");
    assert!(
        inner_detached_index < outer_detached_index,
        "Chromium detaches descendants before their parent: {:?}",
        ctx.sent
    );
    assert!(!ctx.conn.has_attached_child_frame_id(&inner_frame_id));
    assert!(!ctx.conn.has_attached_child_frame_id(&outer_frame_id));
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_nested_child_frame_reports_outer_parent_frame_id() {
    // This regression targets Page.frameAttached parentFrameId for nested
    // frames discovered through the current frame tree. Dynamic insertion and
    // removal are covered separately because they exercise the activity pump.
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;
    ctx.process_async(json!({
        "id": 5241,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe id='outer' name='outer-frame' srcdoc=\"<iframe name='inner-frame' srcdoc='<body>inner</body>'></iframe>\"></iframe>"
        }
    })).await;
    let _ = take_response_by_id(&mut ctx, 5241);
    wait_until_messages(
        &mut ctx,
        "SID-1",
        "nested child frame navigations after Page.navigate response",
        |messages| {
            ["outer-frame", "inner-frame"].iter().all(|name| {
                messages.iter().any(|message| {
                    message["method"] == json!("Page.frameNavigated")
                        && message["params"]["frame"]["name"] == json!(name)
                })
            })
        },
    )
    .await;
    let outer_navigated = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["name"] == json!("outer-frame")
        })
        .cloned()
        .expect("outer frame should emit Page.frameNavigated");
    let outer_frame_id = outer_navigated["params"]["frame"]["id"]
        .as_str()
        .expect("outer frame id")
        .to_owned();
    assert_eq!(
        outer_navigated["params"]["frame"]["parentId"],
        json!("TID-1")
    );
    assert_child_frame_attached(&ctx, &outer_frame_id, "TID-1");
    let inner_navigated = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["name"] == json!("inner-frame")
        })
        .cloned()
        .expect("inner frame should emit Page.frameNavigated");
    let inner_frame_id = inner_navigated["params"]["frame"]["id"]
        .as_str()
        .expect("inner frame id")
        .to_owned();
    assert_eq!(
        inner_navigated["params"]["frame"]["parentId"],
        json!(outer_frame_id)
    );
    let inner_attached = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(inner_frame_id)
                && message["params"]["parentFrameId"] == json!(outer_frame_id)
        })
        .cloned()
        .expect("inner frame should emit Page.frameAttached with outer parent");
    assert_eq!(inner_attached["params"]["frameId"], json!(inner_frame_id));
    assert_ne!(inner_frame_id, outer_frame_id);
    assert_ne!(inner_frame_id, "TID-1");
    assert!(
        inner_frame_id.starts_with("child-browsing-context-"),
        "live child frame ids should use the renderer browsing-context id; got {inner_frame_id}"
    );
    assert_child_frame_attached(&ctx, &inner_frame_id, &outer_frame_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_runtime_frontend_enabled_emits_nested_child_default_context() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    let browser_context = ctx.conn.browser_context.as_mut().unwrap();
    browser_context.active_page_target_mut().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;
    browser_context.active_page_target_mut().devtools_sessions
        [moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;

    ctx.process_async(json!({
        "id": 5242,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe id='outer' name='outer-frame' srcdoc=\"<iframe name='inner-frame' srcdoc='<body>inner</body>'></iframe>\"></iframe>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 5242);
    wait_until_messages(
        &mut ctx,
        "SID-1",
        "nested child attachments and Runtime contexts",
        |messages| {
            let child_attachments = messages
                .iter()
                .filter(|message| message["method"] == json!("Page.frameAttached"))
                .count();
            let child_default_contexts = messages
                .iter()
                .filter(|message| {
                    message["method"] == json!("Runtime.executionContextCreated")
                        && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                        && message["params"]["context"]["auxData"]["frameId"] != json!("TID-1")
                })
                .count();
            child_attachments >= 2 && child_default_contexts >= 2
        },
    )
    .await;

    let outer_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("outer frame should emit Page.frameAttached");
    let inner_context_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"]
                    .as_str()
                    .is_some_and(|frame_id| {
                        frame_id != "TID-1" && frame_id != outer_frame_id.as_str()
                    })
        })
        .and_then(|message| message["params"]["context"]["auxData"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("navigation should emit nested child default execution context");
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(inner_context_frame_id)
                && message["params"]["parentFrameId"] == json!(outer_frame_id)
        }),
        "nested child default context should belong to an attached inner frame; sent={:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_legacy_runtime_frontend_projection_emits_context_creation_without_synthetic_clear()
 {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_dom_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    ctx.enable_background_navigation_scheduler_for_test();

    tokio::task::LocalSet::new()
        .run_until(async {
            ctx.process_and_wait_for_response_async(json!({
                "id": 22,
                "method": "Page.navigate",
                "sessionId": "SID-1",
                "params": { "url": "data:text/html,<body>hi</body>" }
            }))
            .await;
            wait_until_scheduler_message(
                &mut ctx,
                "legacy default Runtime execution context",
                |message| {
                    message["method"] == json!("Runtime.executionContextCreated")
                        && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                        && message["params"]["context"]["auxData"]["frameId"] == json!("TID-1")
                },
            )
            .await;
            wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;

            assert_eq!(ctx.take_one()["method"], "Page.frameStartedNavigating");
            assert_eq!(ctx.take_one()["method"], "Page.frameStartedLoading");

            let result = ctx.take_one();
            assert_eq!(result["id"], 22);
            assert_eq!(result["sessionId"], "SID-1");

            assert_eq!(ctx.take_one()["method"], "Page.frameNavigated");
            assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");

            let created = ctx.take_one();
            assert_eq!(created["method"], "Runtime.executionContextCreated");
            assert_eq!(created["sessionId"], "SID-1");
            assert_eq!(
                created["params"]["context"]["name"],
                json!("data:text/html,<body>hi</body>")
            );
            assert!(created["params"]["context"]["id"].as_i64().is_some());
            assert_eq!(
                created["params"]["context"]["auxData"]["isDefault"],
                json!(true)
            );
            assert!(
                !ctx.sent
                    .iter()
                    .any(|message| message["method"]
                        == json!("Runtime.executionContextsCleared")),
                "legacy protocol projection must not synthesize a Runtime.executionContextsCleared event: {:?}",
                ctx.sent
            );
            assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
            assert_eq!(ctx.take_one()["method"], "Page.domContentEventFired");

            assert_eq!(ctx.take_one()["method"], "Page.loadEventFired");
            assert_eq!(ctx.take_one()["method"], "Page.frameStoppedLoading");
            assert!(ctx.sent.is_empty());
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_real_runtime_enable_resets_before_creating_default_context() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 21,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!("SID-1")
        }),
        "real Runtime.enable should connect the renderer V8 Runtime agent: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 22,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>hi</body>" }
    }))
    .await;

    let sent = ctx.take_all();
    assert_runtime_navigation_context_reset(&sent, "SID-1", "TID-1");
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_real_runtime_enable_fans_out_context_reset_to_attached_session() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    for (id, session_id) in [(21, "SID-1"), (22, "SID-attached")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.enable",
            "sessionId": session_id
        }))
        .await;
        assert!(
            ctx.sent.iter().any(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    && message["sessionId"] == json!(session_id)
            }),
            "Runtime.enable should connect renderer V8 Runtime agent for {session_id}: {:?}",
            ctx.sent
        );
        ctx.sent.clear();
    }

    ctx.process_async(json!({
        "id": 23,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>multi-session</body>" }
    }))
    .await;

    let sent = ctx.take_all();
    for session_id in ["SID-1", "SID-attached"] {
        assert_runtime_navigation_context_reset(&sent, session_id, "TID-1");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_from_attached_session_keeps_primary_and_attached_runtime_events_separate() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    for (id, session_id) in [(21, "SID-1"), (22, "SID-attached")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.enable",
            "sessionId": session_id
        }))
        .await;
        ctx.sent.clear();
    }

    ctx.process_async(json!({
        "id": 23,
        "method": "Page.navigate",
        "sessionId": "SID-attached",
        "params": { "url": "data:text/html,<body>aux-session-nav</body>" }
    }))
    .await;

    let sent = ctx.take_all();
    for session_id in ["SID-1", "SID-attached"] {
        assert_runtime_navigation_context_reset(&sent, session_id, "TID-1");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_attached_runtime_disable_keeps_primary_runtime_enabled() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    for (id, session_id) in [(21, "SID-1"), (22, "SID-attached")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.enable",
            "sessionId": session_id
        }))
        .await;
        ctx.sent.clear();
    }
    ctx.process_async(json!({
        "id": 23,
        "method": "Runtime.disable",
        "sessionId": "SID-attached"
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 23)["result"],
        json!({}),
        "attached Runtime.disable should succeed through its own V8 session"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24,
        "method": "Page.navigate",
        "sessionId": "SID-attached",
        "params": { "url": "data:text/html,<body>aux-runtime-disabled</body>" }
    }))
    .await;

    let sent = ctx.take_all();
    assert_runtime_navigation_context_reset(&sent, "SID-1", "TID-1");
    assert!(
        sent.iter().all(|message| {
            message["sessionId"] != json!("SID-attached")
                || !matches!(
                    message["method"].as_str(),
                    Some("Runtime.executionContextsCleared")
                        | Some("Runtime.executionContextCreated")
                )
        }),
        "Runtime-disabled attached session must stay off context lifecycle surfaces after navigation: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_staged_attached_runtime_enable_emits_context_created_for_attached_session() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );
    ctx.conn
        .with_target_devtools_session_state_for_session_mut(Some("SID-1"), |state| {
            state.runtime_session_state.runtime_frontend_enabled = true;
        });
    ctx.conn
        .with_target_devtools_session_state_for_session_mut(Some("SID-attached"), |state| {
            state.runtime_session_state.runtime_frontend_enabled = true;
        });

    ctx.process_async(json!({
        "id": 24,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>staged-multi-session</body>" }
    }))
    .await;

    let sent = ctx.take_all();
    for session_id in ["SID-1", "SID-attached"] {
        assert!(
            sent.iter().any(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    && message["sessionId"] == json!(session_id)
            }),
            "staged Runtime-enabled session {session_id} should receive new default context on navigation: {sent:?}"
        );
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_runtime_frontend_enabled_emits_initial_console_before_dcl() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;

    ctx.process_async(json!({
        "id": 25,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<script>console.warn('boot warning')</script><body>hi</body>"
        }
    }))
    .await;

    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let sent = ctx.take_all();
    let context_created_index = sent
        .iter()
        .position(|message| message["method"] == json!("Runtime.executionContextCreated"))
        .unwrap_or_else(|| panic!("navigation should emit context creation: {sent:?}"));
    let console_index = sent
        .iter()
        .position(|message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["type"] == json!("warning")
                && message["params"]["args"][0]["value"] == json!("boot warning")
        })
        .unwrap_or_else(|| panic!("navigation should emit initial console output: {sent:?}"));
    let dcl_index = sent
        .iter()
        .position(|message| message["method"] == json!("Page.domContentEventFired"))
        .unwrap_or_else(|| panic!("navigation should emit DOMContentLoaded: {sent:?}"));

    assert!(
        context_created_index < console_index,
        "initial console must follow context creation so clients can resolve the context: {sent:?}"
    );
    assert!(
        console_index < dcl_index,
        "parser-time console output should be visible before DOMContentLoaded: {sent:?}"
    );
    assert_eq!(
        sent.iter()
            .filter(|message| message["method"] == json!("Runtime.consoleAPICalled"))
            .count(),
        1,
        "initial console output should not be duplicated by post-navigation capture: {sent:?}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_console_enabled_emits_initial_console_without_runtime_enable() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.process_async(json!({
        "id": 26,
        "method": "Console.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(26, json!({}), Some("SID-1"));
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .inspector_session_state
            .v8_state
            .is_none(),
        "pre-document Console.enable should remain a first-attach bootstrap without inventing a V8 cookie"
    );

    ctx.process_async(json!({
        "id": 27,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<script>console.warn('console only boot warning')</script><body>hi</body>"
        }
    }))
    .await;

    wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
    let sent = ctx.take_all();
    let console_index = sent
        .iter()
        .position(|message| {
            message["method"] == json!("Console.messageAdded")
                && message["params"]["message"]["text"] == json!("console only boot warning")
        })
        .unwrap_or_else(|| {
            panic!("Console-only navigation should emit parser-time console output: {sent:?}")
        });
    let dcl_index = sent
        .iter()
        .position(|message| message["method"] == json!("Page.domContentEventFired"))
        .unwrap_or_else(|| panic!("navigation should emit DOMContentLoaded: {sent:?}"));

    assert!(
        console_index < dcl_index,
        "Console-only parser-time output should be visible before DOMContentLoaded: {sent:?}"
    );
    assert!(
        sent.iter()
            .all(|message| message["method"] != json!("Runtime.consoleAPICalled")),
        "Console-only navigation should not emit Runtime.consoleAPICalled without Runtime.enable: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn attached_console_enable_survives_navigation_without_enabling_primary_or_runtime() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .unwrap()
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    ctx.process_async(json!({
        "id": 28,
        "method": "Console.enable",
        "sessionId": "SID-attached"
    }))
    .await;
    ctx.expect_result(28, json!({}), Some("SID-attached"));

    ctx.process_async(json!({
        "id": 29,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<body>hi</body>"
        }
    }))
    .await;

    let navigation = ctx.take_all();
    assert!(
        navigation.iter().all(|message| {
            !matches!(
                message["method"].as_str(),
                Some("Runtime.executionContextsCleared")
                    | Some("Runtime.executionContextCreated")
                    | Some("Runtime.consoleAPICalled")
            )
        }),
        "Console-only subscription must not implicitly enable Runtime surfaces: {navigation:?}"
    );

    ctx.process_async(json!({
        "id": 30,
        "method": "Runtime.evaluate",
        "sessionId": "SID-attached",
        "params": {
            "expression": "console.warn('aux console after navigation')"
        }
    }))
    .await;
    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Console.messageAdded")
                && message["sessionId"] == json!("SID-attached")
                && message["params"]["message"]["text"] == json!("aux console after navigation")
        }),
        "Console-enabled attached session should retain its own V8 Console subscription after navigation: {sent:?}"
    );
    assert!(
        sent.iter().all(|message| {
            message["sessionId"] != json!("SID-1")
                || message["method"] != json!("Console.messageAdded")
        }),
        "Console-disabled primary session must not receive the attached subscription's events: {sent:?}"
    );
    assert!(
        sent.iter()
            .all(|message| message["method"] != json!("Runtime.consoleAPICalled")),
        "Console-only evaluation must not implicitly enable Runtime console events: {sent:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_without_runtime_frontend_enabled_emits_no_runtime_context_events() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 23,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>hi</body>" }
    }))
    .await;

    let messages = ctx.take_all();
    assert!(
        !messages.iter().any(|message| {
            matches!(
                message["method"].as_str(),
                Some("Runtime.executionContextsCleared") | Some("Runtime.executionContextCreated")
            )
        }),
        "runtime context events should not be emitted when Runtime is disabled"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_runtime_frontend_enabled_emits_child_default_execution_context_created() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;

    ctx.process_async(json!({
        "id": 231,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe name='child-frame' srcdoc=\"<body>child</body>\"></iframe>"
        }
    }))
    .await;

    let _ = take_response_by_id(&mut ctx, 231);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "child frame attachment after Page.navigate response",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame should emit Page.frameAttached");
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_message(
        &mut ctx,
        "SID-1",
        "child default Runtime context after Page.navigate response",
        move |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"]
                    == json!(expected_child_frame_id)
        },
    )
    .await;

    let child_context_created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .cloned()
        .expect("navigation should emit child default execution context created");
    assert_eq!(child_context_created["sessionId"], json!("SID-1"));
    assert!(
        child_context_created["params"]["context"]["id"]
            .as_i64()
            .is_some()
    );
    assert_eq!(
        child_context_created["params"]["context"]["auxData"]["type"],
        json!("default")
    );

    let frame_attached_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .expect("child frame should emit Page.frameAttached");
    let child_context_created_index = ctx
        .sent
        .iter()
        .position(|message| message == &child_context_created)
        .expect("child default execution context created event should remain buffered");
    assert!(
        frame_attached_index < child_context_created_index,
        "child Page.frameAttached must precede child default Runtime.executionContextCreated; sent={:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_real_runtime_enable_emits_native_child_default_execution_context_created() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 21,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["context"]["uniqueId"].as_str().is_some()
        }),
        "real Runtime.enable should connect the renderer V8 Runtime agent: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 231,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe name='child-frame' srcdoc=\"<body>child</body>\"></iframe>"
        }
    }))
    .await;

    let _ = take_response_by_id(&mut ctx, 231);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "native child frame attachment after Page.navigate response",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame should emit Page.frameAttached");
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_message(
        &mut ctx,
        "SID-1",
        "native child default Runtime context after Page.navigate response",
        move |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"]
                    == json!(expected_child_frame_id)
        },
    )
    .await;

    let child_context_created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .cloned()
        .expect("navigation should emit child default execution context created");
    assert_eq!(child_context_created["sessionId"], json!("SID-1"));
    assert!(
        child_context_created["params"]["context"]["id"]
            .as_i64()
            .is_some()
    );
    assert!(
        child_context_created["params"]["context"]["uniqueId"]
            .as_str()
            .is_some(),
        "child default context should come from V8 native Runtime event with uniqueId: {child_context_created:?}"
    );
    assert_eq!(
        child_context_created["params"]["context"]["auxData"]["type"],
        json!("default")
    );

    let frame_attached_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .expect("child frame should emit Page.frameAttached");
    let child_context_created_index = ctx
        .sent
        .iter()
        .position(|message| message == &child_context_created)
        .expect("child default execution context created event should remain buffered");
    assert!(
        frame_attached_index < child_context_created_index,
        "child Page.frameAttached must precede child default Runtime.executionContextCreated; sent={:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_emits_each_child_default_context_identity_once_per_runtime_session() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-primary", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-primary"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-primary")).await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    for (command_id, session_id) in [(25_301, "SID-primary"), (25_302, "SID-attached")] {
        ctx.process_async(json!({
            "id": command_id,
            "method": "Runtime.enable",
            "sessionId": session_id,
        }))
        .await;
        assert_eq!(
            take_response_by_id(&mut ctx, command_id)["result"],
            json!({})
        );
    }
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 25_303,
        "method": "Page.navigate",
        "sessionId": "SID-primary",
        "params": {
            "url": "data:text/html,<iframe srcdoc=\"<body>child</body>\"></iframe>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 25_303);
    wait_until_message(
        &mut ctx,
        "SID-primary",
        "child frame attachment for multi-session Runtime navigation",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame should be attached");
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_messages(
        &mut ctx,
        "SID-primary",
        "child default Runtime context fan-out to primary and attached sessions",
        move |messages| {
            ["SID-primary", "SID-attached"]
                .into_iter()
                .all(|session_id| {
                    messages.iter().any(|message| {
                        message["method"] == json!("Runtime.executionContextCreated")
                            && message["sessionId"] == json!(session_id)
                            && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                            && message["params"]["context"]["auxData"]["frameId"]
                                == json!(expected_child_frame_id)
                    })
                })
        },
    )
    .await;

    let child_context_identities = |session_id: &str| {
        ctx.sent
            .iter()
            .filter(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    && message["sessionId"] == json!(session_id)
                    && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                    && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
            })
            .map(|message| {
                format!(
                    "{}:{}",
                    message["params"]["context"]["id"], message["params"]["context"]["uniqueId"]
                )
            })
            .collect::<Vec<_>>()
    };
    let primary_identities = child_context_identities("SID-primary");
    let attached_identities = child_context_identities("SID-attached");
    assert!(
        !primary_identities.is_empty() && !attached_identities.is_empty(),
        "both Runtime frontends must observe a child default context: {:?}",
        ctx.sent
    );
    // Chromium can expose more than one legitimate child context generation
    // here (the initial about:blank realm followed by the srcdoc realm). The
    // invariant is one delivery of each identity per frontend, not a fixed
    // total number of child contexts.
    let mut unique_primary_identities = primary_identities.clone();
    unique_primary_identities.sort();
    unique_primary_identities.dedup();
    assert_eq!(
        primary_identities.len(),
        unique_primary_identities.len(),
        "the primary Runtime frontend must receive each child context once: {:?}",
        ctx.sent
    );
    let mut unique_attached_identities = attached_identities.clone();
    unique_attached_identities.sort();
    unique_attached_identities.dedup();
    assert_eq!(
        attached_identities.len(),
        unique_attached_identities.len(),
        "the attached Runtime frontend must receive each child context once: {:?}",
        ctx.sent
    );
    assert_eq!(
        unique_primary_identities, unique_attached_identities,
        "both Runtime frontends should observe the same child context identities"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_real_runtime_enable_auto_creates_native_named_child_world_before_frame_navigated()
 {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 231,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 231);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 232,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_nav_child_world = 'ready';",
            "worldName": "utility-child"
        }
    }))
    .await;
    ctx.expect_result(232, json!({ "identifier": "1" }), Some("SID-1"));

    ctx.process_async(json!({
        "id": 233,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "childNavigationBinding",
            "executionContextName": "utility-child"
        }
    }))
    .await;
    ctx.expect_result(233, json!({}), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 234,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe name='child-frame' srcdoc=\"<body>nav-child</body>\"></iframe>"
        }
    }))
    .await;

    let _ = take_response_by_id(&mut ctx, 234);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "named child frame attachment after Page.navigate response",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame should emit Page.frameAttached");
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_message(
        &mut ctx,
        "SID-1",
        "named child Runtime context after Page.navigate response",
        move |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-child")
                && message["params"]["context"]["auxData"]["frameId"]
                    == json!(expected_child_frame_id)
        },
    )
    .await;
    let named_world_created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-child")
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .cloned()
        .expect("navigation should auto-create child named world");
    assert!(
        named_world_created["params"]["context"]["uniqueId"]
            .as_str()
            .is_some(),
        "child named-world context should come from V8 native Runtime event with uniqueId: {named_world_created:?}"
    );
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_message(
        &mut ctx,
        "SID-1",
        "child frame navigation after its native named Runtime context",
        move |message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(expected_child_frame_id)
        },
    )
    .await;
    let named_world_created_index = ctx
        .sent
        .iter()
        .position(|message| message == &named_world_created)
        .expect("child named-world event index");
    let child_frame_navigated_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["id"] == json!(child_frame_id)
        })
        .expect("child frame should emit Page.frameNavigated");
    assert!(
        named_world_created_index < child_frame_navigated_index,
        "child named-world context should be emitted before child frameNavigated; sent={:?}",
        ctx.sent
    );
    let child_context_id = named_world_created["params"]["context"]["id"]
        .as_i64()
        .expect("child named-world execution context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 235,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_context_id,
            "expression": "JSON.stringify([globalThis.__lm_nav_child_world, typeof childNavigationBinding, document.body.textContent.trim()])"
        }
    }))
    .await;
    let child_state = take_response_by_id(&mut ctx, 235);
    assert_eq!(
        child_state["result"]["result"]["value"],
        json!("[\"ready\",\"function\",\"nav-child\"]")
    );

    ctx.process_async(json!({
        "id": 236,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_context_id,
            "expression": "childNavigationBinding('nav-child-payload'); 13"
        }
    }))
    .await;
    let binding_call = take_response_by_id(&mut ctx, 236);
    assert_eq!(binding_call["result"]["result"]["value"], json!(13));
    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("childNavigationBinding")
        })
        .cloned()
        .expect("navigation child named-world binding should emit Runtime.bindingCalled");
    assert_eq!(
        binding_called["params"]["payload"],
        json!("nav-child-payload")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_child_runtime_contexts_fan_out_to_each_runtime_enabled_session() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .assign_attached_session_to_target("TID-1", "SID-attached".to_owned())
    );

    for (id, session_id) in [(90_240, "SID-1"), (90_241, "SID-attached")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.enable",
            "sessionId": session_id
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
    }
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_242,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__fanoutChildWorld = true;",
            "worldName": "fanout-child-world"
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 90_242)["result"]["identifier"],
        json!("1")
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 90_243,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe srcdoc=\"<body>fanout-child</body>\"></iframe>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 90_243);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "multi-session child frame attachment",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame id");
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_messages(
        &mut ctx,
        "SID-1",
        "default and named child contexts for both Runtime sessions",
        move |messages| {
            ["SID-1", "SID-attached"].into_iter().all(|session_id| {
                let has_default = messages.iter().any(|message| {
                    message["method"] == json!("Runtime.executionContextCreated")
                        && message["sessionId"] == json!(session_id)
                        && message["params"]["context"]["auxData"]["frameId"]
                            == json!(expected_child_frame_id)
                        && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                });
                let has_named = messages.iter().any(|message| {
                    message["method"] == json!("Runtime.executionContextCreated")
                        && message["sessionId"] == json!(session_id)
                        && message["params"]["context"]["auxData"]["frameId"]
                            == json!(expected_child_frame_id)
                        && message["params"]["context"]["name"] == json!("fanout-child-world")
                });
                has_default && has_named
            })
        },
    )
    .await;

    for (is_default, name) in [(true, None), (false, Some("fanout-child-world"))] {
        let contexts = ["SID-1", "SID-attached"]
            .into_iter()
            .map(|session_id| {
                ctx.sent
                    .iter()
                    .filter(|message| {
                        message["method"] == json!("Runtime.executionContextCreated")
                            && message["sessionId"] == json!(session_id)
                            && message["params"]["context"]["auxData"]["frameId"]
                                == json!(child_frame_id)
                            && message["params"]["context"]["auxData"]["isDefault"]
                                == json!(is_default)
                            && name.is_none_or(|name| {
                                message["params"]["context"]["name"] == json!(name)
                            })
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            contexts[0].len(),
            1,
            "primary context should emit once: {contexts:?}"
        );
        assert_eq!(
            contexts[1].len(),
            1,
            "attached context should emit once: {contexts:?}"
        );
        assert_eq!(
            contexts[0][0]["params"]["context"]["id"], contexts[1][0]["params"]["context"]["id"],
            "both sessions must observe the same V8 context id"
        );
        assert_eq!(
            contexts[0][0]["params"]["context"]["uniqueId"],
            contexts[1][0]["params"]["context"]["uniqueId"],
            "both sessions must observe the same V8 context unique id"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_child_named_world_reports_context_when_document_start_script_throws() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 237,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 237);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 238,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "if (window.top !== window) { throw new Error('child utility bootstrap failed'); }",
            "worldName": "utility-child"
        }
    }))
    .await;
    ctx.expect_result(238, json!({ "identifier": "1" }), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 239,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe srcdoc=\"<body>nav-child</body>\"></iframe>"
        }
    }))
    .await;

    let _ = take_response_by_id(&mut ctx, 239);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "throwing preload child frame attachment after Page.navigate response",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("child frame should emit Page.frameAttached: {:?}", ctx.sent));
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_message(
        &mut ctx,
        "SID-1",
        "throwing preload named child Runtime context",
        move |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-child")
                && message["params"]["context"]["auxData"]["frameId"]
                    == json!(expected_child_frame_id)
        },
    )
    .await;
    let named_world_created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-child")
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .cloned()
        .expect("throwing document-start script must not orphan the child named world");
    assert!(
        named_world_created["params"]["context"]["uniqueId"]
            .as_str()
            .is_some(),
        "child named-world context should still come from V8 native Runtime event: {named_world_created:?}"
    );
    let child_context_id = named_world_created["params"]["context"]["id"]
        .as_i64()
        .expect("child named-world execution context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 240,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_context_id,
            "expression": "document.body.textContent.trim()"
        }
    }))
    .await;
    let child_state = take_response_by_id(&mut ctx, 240);
    assert_eq!(child_state["result"]["result"]["value"], json!("nav-child"));
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_real_runtime_enable_emits_native_child_named_world_context_created() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 10231,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 10231);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10232,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_native_child_world = 'ready';",
            "worldName": "utility-native-child"
        }
    }))
    .await;
    ctx.expect_result(10232, json!({ "identifier": "1" }), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 10233,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe name='child-frame' srcdoc=\"<body>native-child</body>\"></iframe>"
        }
    }))
    .await;

    let _ = take_response_by_id(&mut ctx, 10233);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "native named child frame attachment after Page.navigate response",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame should emit Page.frameAttached");
    let expected_child_frame_id = child_frame_id.clone();
    wait_until_message(
        &mut ctx,
        "SID-1",
        "native named child Runtime context after Page.navigate response",
        move |message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-native-child")
                && message["params"]["context"]["auxData"]["frameId"]
                    == json!(expected_child_frame_id)
        },
    )
    .await;
    let named_world_created = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-native-child")
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .cloned()
        .expect("navigation should auto-create child named world from native Runtime agent");
    assert_eq!(named_world_created["sessionId"], json!("SID-1"));
    assert!(
        named_world_created["params"]["context"]["uniqueId"]
            .as_str()
            .is_some(),
        "child named-world context should come from V8 native Runtime event with uniqueId: {named_world_created:?}"
    );
    assert_eq!(
        named_world_created["params"]["context"]["auxData"]["type"],
        json!("isolated")
    );

    let child_context_id = named_world_created["params"]["context"]["id"]
        .as_i64()
        .expect("child named-world execution context id");
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 10234,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_context_id,
            "expression": "JSON.stringify([globalThis.__lm_native_child_world, document.body.textContent.trim()])"
        }
    }))
    .await;
    let child_state = take_response_by_id(&mut ctx, 10234);
    assert_eq!(
        child_state["result"]["result"]["value"],
        json!("[\"ready\",\"native-child\"]")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_child_iframe_delays_main_load_until_child_frame_and_contexts_complete() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 236,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 236);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 237,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_child_world_before_load = true;",
            "worldName": "utility-child"
        }
    }))
    .await;
    ctx.expect_result(237, json!({ "identifier": "1" }), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 238,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<iframe name='child-frame' srcdoc=\"<body>child</body>\"></iframe>"
        }
    }))
    .await;

    let _ = take_response_by_id(&mut ctx, 238);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "main load after child frame completion",
        |message| message["method"] == json!("Page.loadEventFired"),
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("child frame should emit Page.frameAttached");
    let child_default_context_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .expect("child frame should emit default execution context before main load");
    let child_named_world_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-child")
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .expect("child frame should emit named isolated world before main load");
    let child_stopped_loading_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameStoppedLoading")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .expect("child frame should emit Page.frameStoppedLoading");
    let main_load_event_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.loadEventFired"))
        .expect("main frame should emit Page.loadEventFired");

    assert!(
        child_default_context_index < main_load_event_index,
        "child default execution context must precede main loadEventFired; sent={:?}",
        ctx.sent
    );
    assert!(
        child_named_world_index < main_load_event_index,
        "child named-world execution context must precede main loadEventFired; sent={:?}",
        ctx.sent
    );
    assert!(
        child_stopped_loading_index < main_load_event_index,
        "child frame must finish loading before main loadEventFired; sent={:?}",
        ctx.sent
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_network_child_iframe_delays_main_load_until_child_frame_and_contexts_complete()
 {
    async fn parent() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body><iframe name=\"child-frame\" src=\"/child\"></iframe></body></html>",
        )
    }

    async fn child() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>child-network-load-boundary</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/parent", axum::routing::get(parent))
                .route("/child", axum::routing::get(child)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    // This assertion depends on native V8 Runtime events. Exercise the public
    // command so the Inspector frontend is retained across the async network
    // navigation; toggling only the protocol projection does not establish
    // that renderer-side lifetime.
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 23_09,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 23_09);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 239,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_network_child_world_before_load = true;",
            "worldName": "utility-child"
        }
    }))
    .await;
    ctx.expect_result(239, json!({ "identifier": "1" }), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 23_10,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": format!("http://{addr}/parent")
        }
    }))
    .await;

    let _ = take_response_by_id(&mut ctx, 23_10);
    wait_until_message(
        &mut ctx,
        "SID-1",
        "network child frameAttached",
        |message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        },
    )
    .await;
    let child_frame_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Page.frameAttached")
                && message["params"]["parentFrameId"] == json!("TID-1")
        })
        .and_then(|message| message["params"]["frameId"].as_str())
        .map(str::to_owned)
        .expect("network child frame should emit Page.frameAttached");
    crate::testing::wait_until_messages(
        &mut ctx,
        "SID-1",
        "network child frame completion and main load",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                    && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
            }) && messages.iter().any(|message| {
                message["method"] == json!("Runtime.executionContextCreated")
                    && message["params"]["context"]["name"] == json!("utility-child")
                    && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
            }) && messages.iter().any(|message| {
                message["method"] == json!("Page.frameStoppedLoading")
                    && message["params"]["frameId"] == json!(child_frame_id)
            }) && messages
                .iter()
                .any(|message| message["method"] == json!("Page.loadEventFired"))
        },
    )
    .await;
    let child_default_context_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .expect("network child frame should expose its default execution context before main load");
    let child_named_world_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility-child")
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .unwrap_or_else(|| {
            panic!(
                "network child frame should emit named isolated world before main load; sent={:?}",
                ctx.sent
            )
        });
    let child_stopped_loading_index = ctx
        .sent
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameStoppedLoading")
                && message["params"]["frameId"] == json!(child_frame_id)
        })
        .expect("network child frame should emit Page.frameStoppedLoading");
    let main_load_event_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.loadEventFired"))
        .expect("main frame should emit Page.loadEventFired");
    let child_default_context_id = ctx.sent[child_default_context_index]["params"]["context"]["id"]
        .as_i64()
        .expect("network child default execution context id");
    let child_named_world_context_id = ctx.sent[child_named_world_index]["params"]["context"]["id"]
        .as_i64()
        .expect("network child named-world execution context id");

    assert!(
        child_default_context_index < main_load_event_index,
        "network child default execution context must precede main loadEventFired; sent={:?}",
        ctx.sent
    );
    assert!(
        child_named_world_index < main_load_event_index,
        "network child named-world execution context must precede main loadEventFired; sent={:?}",
        ctx.sent
    );
    assert!(
        child_stopped_loading_index < main_load_event_index,
        "network child frame must finish loading before main loadEventFired; sent={:?}",
        ctx.sent
    );

    // The first eligible same-origin commit reuses the initial-empty
    // LocalWindow and its V8 contexts. The context-created event can therefore
    // retain its about:blank name; the stable context must project the newly
    // committed Document before main load completes.
    ctx.process_async(json!({
        "id": 23_12,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_default_context_id,
            "expression": "JSON.stringify([location.pathname, document.body.textContent.trim()])"
        }
    }))
    .await;
    let default_world_state = take_response_by_id(&mut ctx, 23_12);
    assert_eq!(
        default_world_state["result"]["result"]["value"],
        json!("[\"/child\",\"child-network-load-boundary\"]")
    );

    ctx.process_async(json!({
        "id": 23_13,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": child_named_world_context_id,
            "expression": "JSON.stringify([globalThis.__lm_network_child_world_before_load, location.pathname, document.body.textContent.trim()])"
        }
    }))
    .await;
    let named_world_state = take_response_by_id(&mut ctx, 23_13);
    assert_eq!(
        named_world_state["result"]["result"]["value"],
        json!("[true,\"/child\",\"child-network-load-boundary\"]")
    );

    ctx.process_async(json!({
        "id": 23_11,
        "method": "Page.getFrameTree",
        "sessionId": "SID-1"
    }))
    .await;
    let frame_tree = take_response_by_id(&mut ctx, 23_11);
    let child_frames = frame_tree["result"]["frameTree"]["childFrames"]
        .as_array()
        .expect("frame tree childFrames array");
    assert_eq!(
        child_frames.len(),
        1,
        "network child frame should be visible in frame tree immediately after navigate response"
    );
    assert_eq!(
        child_frames[0]["frame"]["id"],
        json!(child_frame_id),
        "frame tree child should match attached child frame id"
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_failure_commits_error_document_with_visible_unreachable_url() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    let committed_document_token = ctx
        .conn
        .browser_context
        .as_mut()
        .unwrap()
        .start_document_navigation_for_active_target("LOADER-committed".to_owned())
        .expect("active target should start committed document navigation");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .commit_document_navigation_if_matches(&committed_document_token);

    let unreachable_url = format!("http://{addr}/missing");
    ctx.process_async(json!({
        "id": 231,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": unreachable_url }
    }))
    .await;
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "network error Document stopped loading",
        |message| message["method"] == json!("Page.frameStoppedLoading"),
    )
    .await;

    let response = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(231))
        .expect("Page.navigate response");
    assert_eq!(response["id"], 231);
    assert_eq!(response["result"]["frameId"], "TID-1");
    assert!(response["result"]["loaderId"].is_string());
    assert_eq!(response["result"]["isDownload"], false);
    assert!(response["result"]["errorText"].is_string());
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .accepts_document_body_completion_event(&committed_document_token),
        "ordinary navigation load failures must invalidate the previously committed document"
    );

    let messages = ctx.take_all();
    let frame_navigated = messages
        .iter()
        .find(|message| message["method"] == json!("Page.frameNavigated"))
        .unwrap_or_else(|| panic!("error Document should commit a frame: {messages:?}"));
    assert_eq!(
        frame_navigated["params"]["frame"]["url"],
        NETWORK_ERROR_PAGE_URL
    );
    assert_eq!(
        frame_navigated["params"]["frame"]["unreachableUrl"],
        unreachable_url
    );
    assert_eq!(frame_navigated["params"]["frame"]["securityOrigin"], "://");
    assert_eq!(
        frame_navigated["params"]["frame"]["secureContextType"],
        "InsecureScheme"
    );
    for method in [
        "Page.domContentEventFired",
        "Page.loadEventFired",
        "Page.frameStoppedLoading",
    ] {
        assert!(
            messages
                .iter()
                .any(|message| message["method"] == json!(method)),
            "error Document should complete {method}: {messages:?}"
        );
    }
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .target_url(),
        unreachable_url
    );

    ctx.process_async(json!({
        "id": 233,
        "method": "Page.getNavigationHistory",
        "sessionId": "SID-1"
    }))
    .await;
    let history = take_response_by_id(&mut ctx, 233);
    let current_index = history["result"]["currentIndex"]
        .as_u64()
        .expect("current history index") as usize;
    assert_eq!(
        history["result"]["entries"][current_index]["url"],
        unreachable_url
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_failure_creates_runtime_context_and_completes_lifecycle() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .runtime_frontend_enabled = true;
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .page_session_state
        .page_lifecycle_events = true;

    ctx.process_async(json!({
        "id": 232,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/missing") }
    }))
    .await;
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "network error Document stopped loading",
        |message| message["method"] == json!("Page.frameStoppedLoading"),
    )
    .await;

    let response = ctx
        .sent
        .iter()
        .find(|message| message["id"] == json!(232))
        .expect("Page.navigate response");
    assert_eq!(response["id"], 232);
    assert_eq!(response["result"]["frameId"], "TID-1");
    assert!(response["result"]["loaderId"].is_string());
    assert_eq!(response["result"]["isDownload"], false);
    assert!(response["result"]["errorText"].is_string());

    let messages = ctx.take_all();
    assert!(
        messages.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
        }),
        "error Document should create a default runtime context: {messages:?}"
    );
    assert!(
        messages.iter().any(|message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("DOMContentLoaded")
        }) && messages.iter().any(|message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["name"] == json!("load")
        }),
        "error Document should complete lifecycle: {messages:?}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_with_runtime_and_lifecycle_enabled_replays_contexts_before_load() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.enable_dom_events_for_test(Some("SID-1"));
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 2401,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 2401);
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!("SID-1")
        }),
        "real Runtime.enable should connect the renderer V8 Runtime agent: {:?}",
        ctx.sent
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 2402,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-1",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(2402, json!({}), Some("SID-1"));
    ctx.sent.clear();
    ctx.enable_background_navigation_scheduler_for_test();

    tokio::task::LocalSet::new()
        .run_until(async {
            ctx.process_async(json!({
                "id": 24,
                "method": "Page.navigate",
                "sessionId": "SID-1",
                "params": { "url": "data:text/html,<body>hi</body>" }
            }))
            .await;

            wait_until_frame_stopped_loading(&mut ctx, "TID-1").await;
            assert_eq!(ctx.take_one()["method"], "Page.frameStartedNavigating");
            assert_eq!(ctx.take_one()["method"], "Page.frameStartedLoading");
            assert_eq!(ctx.take_one()["id"], 24);
            assert_eq!(ctx.take_one()["method"], "Runtime.executionContextsCleared");
            let init = ctx.take_one();
            assert_eq!(init["method"], "Page.lifecycleEvent");
            assert_eq!(init["params"]["name"], "init");
            assert_eq!(ctx.take_one()["method"], "Page.frameNavigated");
            assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
            assert_eq!(ctx.take_one()["method"], "Runtime.executionContextCreated");
            assert_eq!(ctx.take_one()["method"], "DOM.documentUpdated");
            assert_eq!(ctx.take_one()["method"], "Page.domContentEventFired");
            let dcl = ctx.take_one();
            assert_eq!(dcl["method"], "Page.lifecycleEvent");
            assert_eq!(dcl["params"]["name"], "DOMContentLoaded");
            assert_eq!(ctx.take_one()["method"], "Page.loadEventFired");
            assert_eq!(ctx.take_one()["method"], "Page.lifecycleEvent");
            assert_eq!(ctx.take_one()["method"], "Page.lifecycleEvent");
            assert_eq!(ctx.take_one()["method"], "Page.lifecycleEvent");
            assert_eq!(ctx.take_one()["method"], "Page.frameStoppedLoading");
            assert!(ctx.sent.is_empty());
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_request_completes_paused_navigation_before_commit_events() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>continued</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();

    ctx.process_async(json!({
        "id": 241,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(241, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 242,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 243,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;

    let continue_index = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(243))
        .expect("Fetch.continueRequest response");
    let navigate_index = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(242))
        .expect("Page.navigate response");
    let commit_index = ctx
        .sent
        .iter()
        .position(|message| message["method"] == json!("Page.frameNavigated"))
        .expect("navigation commit event");
    assert!(
        continue_index < navigate_index,
        "Fetch command response should precede the completed Page.navigate response: {:?}",
        ctx.sent
    );
    assert!(
        navigate_index < commit_index,
        "Page.navigate response should precede commit events: {:?}",
        ctx.sent
    );

    let continue_response = ctx.sent.remove(continue_index);
    assert_eq!(continue_response["result"], json!({}));
    assert_eq!(continue_response["sessionId"], json!("SID-1"));
    let navigate_index = ctx
        .sent
        .iter()
        .position(|message| message["id"] == json!(242))
        .expect("Page.navigate response after removing continue response");
    let navigate_response = ctx.sent.remove(navigate_index);
    assert_eq!(navigate_response["sessionId"], json!("SID-1"));
    assert_eq!(navigate_response["result"]["frameId"], json!("TID-1"));
    assert!(
        navigate_response["result"].get("loaderId").is_some(),
        "continued main-document navigation should return a loader id: {navigate_response:?}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_aborts_paused_request_stage_navigation() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();
    let committed_document_token = bc
        .start_document_navigation_for_active_target("LOADER-committed-stop".to_owned())
        .expect("active target should start committed document navigation");
    bc.commit_document_navigation_if_matches(&committed_document_token);

    ctx.process_async(json!({
        "id": 233,
        "method": "Fetch.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(233, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 234,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "http://example.test/stop-loading" }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    assert_eq!(paused["sessionId"], "SID-1");
    assert_eq!(paused["params"]["resourceType"], "Document");
    let network_id = paused["params"]["networkId"].clone();

    ctx.process_async(json!({
        "id": 235,
        "method": "Page.stopLoading",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(235, json!({}), Some("SID-1"));

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["type"], "Document");
    assert_eq!(failed["params"]["errorText"], "Navigation stopped");

    let error = ctx.take_one();
    assert_eq!(error["id"], 234);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Navigation stopped");
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .unwrap()
            .accepts_document_body_completion_event(&committed_document_token),
        "Page.stopLoading should preserve the previously committed document"
    );

    let messages = ctx.take_all();
    for method in [
        "Page.frameClearedScheduledNavigation",
        "Page.frameNavigated",
        "DOM.documentUpdated",
        "Page.domContentEventFired",
        "Page.loadEventFired",
        "Page.lifecycleEvent",
        "Page.frameStoppedLoading",
        "Network.responseReceived",
        "Network.loadingFinished",
    ] {
        assert!(
            !messages
                .iter()
                .any(|message| message["method"] == json!(method)),
            "unexpected completion event after stopLoading: {method}"
        );
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_without_browser_context_returns_empty_result() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 160,
        "method": "Page.stopLoading"
    }))
    .await;

    ctx.expect_result(160, json!({}), None);
    assert!(ctx.take_all().is_empty());
}
#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_aborts_paused_response_stage_navigation() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [
                (axum::http::header::CONTENT_TYPE.as_str(), "text/html"),
                ("x-stage", "response"),
            ],
            "<!doctype html><html><body>stop-loading</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();

    ctx.process_async(json!({
            "id": 236,
            "method": "Fetch.enable",
            "sessionId": "SID-1",
            "params": {
                "patterns": [{ "urlPattern": "*", "requestStage": "Response", "resourceType": "Document" }]
            }
        })).await;
    ctx.expect_result(236, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 237,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;

    consume_main_document_navigation_start(&mut ctx);
    let request = ctx.take_one();
    assert_eq!(request["method"], "Network.requestWillBeSent");
    let network_id = request["params"]["requestId"].clone();

    let paused = take_main_document_response_pause_after_extra_info(&mut ctx, &network_id, 200);
    assert_eq!(paused["sessionId"], "SID-1");
    assert_eq!(paused["params"]["resourceType"], "Document");
    assert_eq!(paused["params"]["networkId"], network_id);
    assert_eq!(paused["params"]["responseStatusCode"], 200);

    ctx.process_async(json!({
        "id": 238,
        "method": "Page.stopLoading",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(238, json!({}), Some("SID-1"));

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["sessionId"], "SID-1");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["type"], "Document");
    assert_eq!(failed["params"]["errorText"], "Navigation stopped");

    let error = ctx.take_one();
    assert_eq!(error["id"], 237);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Navigation stopped");

    let messages = ctx.take_all();
    for method in [
        "Page.frameClearedScheduledNavigation",
        "Page.frameNavigated",
        "DOM.documentUpdated",
        "Page.domContentEventFired",
        "Page.loadEventFired",
        "Page.lifecycleEvent",
        "Page.frameStoppedLoading",
        "Network.responseReceived",
        "Network.loadingFinished",
    ] {
        assert!(
            !messages
                .iter()
                .any(|message| message["method"] == json!(method)),
            "unexpected completion event after stopLoading: {method}"
        );
    }

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn stop_loading_aborts_paused_auth_navigation() {
    async fn auth(headers: axum::http::HeaderMap) -> impl axum::response::IntoResponse {
        let expected = "Basic YWxhZGRpbjpvcGVuc2VzYW1l";
        let authorization = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok());
        if authorization != Some(expected) {
            return axum::response::IntoResponse::into_response((
                axum::http::StatusCode::UNAUTHORIZED,
                [
                    (
                        axum::http::header::WWW_AUTHENTICATE.as_str(),
                        r#"Basic realm="stop-area""#,
                    ),
                    (axum::http::header::CONTENT_TYPE.as_str(), "text/plain"),
                ],
                "auth required",
            ));
        }

        axum::response::IntoResponse::into_response((
            axum::http::StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>authorized</body></html>",
        ))
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/auth", axum::routing::get(auth)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_target_mut()
        .runtime_slot
        .enable_primary_network_events();

    ctx.process_async(json!({
        "id": 254,
        "method": "Fetch.enable",
        "sessionId": "SID-1",
        "params": { "handleAuthRequests": true }
    }))
    .await;
    ctx.expect_result(254, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 255,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/auth") }
    }))
    .await;

    let paused = take_main_document_request_pause(&mut ctx);
    assert_eq!(paused["method"], "Fetch.requestPaused");
    let request_id = paused["params"]["requestId"]
        .as_str()
        .expect("request id")
        .to_owned();
    let network_id = paused["params"]["networkId"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 256,
        "method": "Fetch.continueRequest",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(256, json!({}), Some("SID-1"));

    let auth_required = ctx.take_one();
    assert_eq!(auth_required["method"], "Fetch.authRequired");
    assert_eq!(auth_required["params"]["requestId"], json!(request_id));
    assert!(auth_required["params"].get("networkId").is_none());
    assert_eq!(auth_required["params"]["resourceType"], "Document");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 257,
        "method": "Page.stopLoading",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(257, json!({}), Some("SID-1"));

    let failed = ctx.take_one();
    assert_eq!(failed["method"], "Network.loadingFailed");
    assert_eq!(failed["params"]["requestId"], network_id);
    assert_eq!(failed["params"]["type"], "Document");
    assert_eq!(failed["params"]["errorText"], "Navigation stopped");

    let error = ctx.take_one();
    assert_eq!(error["id"], 255);
    assert_eq!(error["error"]["code"], -32000);
    assert_eq!(error["error"]["message"], "Navigation stopped");

    ctx.process_async(json!({
        "id": 258,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": request_id,
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "aladdin",
                "password": "opensesame"
            }
        }
    }))
    .await;
    ctx.expect_error(258, -32000, "RequestNotFound");

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn reload_reloads_current_url_and_returns_empty_result() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    async fn page(
        axum::extract::State(counter): axum::extract::State<Arc<AtomicUsize>>,
    ) -> impl axum::response::IntoResponse {
        let next = counter.fetch_add(1, Ordering::SeqCst) + 1;
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            format!("<!doctype html><html><body>{next}</body></html>"),
        )
    }

    let counter = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_counter = counter.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/page", axum::routing::get(page))
                .with_state(server_counter),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 239,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/page") }
    }))
    .await;
    let navigate = take_response_by_id(&mut ctx, 239);
    assert_eq!(
        navigate["result"],
        json!({ "frameId": "TID-1", "loaderId": LOADER_ID })
    );

    let first_html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        first_html.contains(">1<"),
        "expected first navigation html to contain counter 1, got {first_html}"
    );

    ctx.process_async(json!({
        "id": 241,
        "method": "Page.reload",
        "sessionId": "SID-1"
    }))
    .await;
    let reload = take_response_by_id(&mut ctx, 241);
    assert_eq!(reload["result"], json!({}));

    let second_html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        second_html.contains(">2<"),
        "expected reloaded html to contain counter 2, got {second_html}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_bypass_csp_controls_response_policy_across_reload() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [
                (axum::http::header::CONTENT_TYPE.as_str(), "text/html"),
                ("content-security-policy", "script-src 'none'"),
            ],
            r#"<!doctype html>
<title>CSP Locked</title>
<script>
globalThis.__cspRan = true;
document.title = "CSP Bypassed";
</script>"#,
        )
    }

    async fn evaluate_csp_state(ctx: &mut TestContext, id: u64) -> serde_json::Value {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-CSP",
            "params": {
                "expression": "[globalThis.__cspRan === true, document.title]",
                "returnByValue": true
            }
        }))
        .await;
        take_response_by_id(ctx, id)["result"]["result"]["value"].clone()
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/csp", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-CSP", "TID-CSP", "SID-CSP", "about:blank");
    ctx.process_async(json!({
        "id": 24_000,
        "method": "Page.enable",
        "sessionId": "SID-CSP"
    }))
    .await;
    take_response_by_id(&mut ctx, 24_000);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_001,
        "method": "Page.navigate",
        "sessionId": "SID-CSP",
        "params": { "url": format!("http://{addr}/csp") }
    }))
    .await;
    take_response_by_id(&mut ctx, 24_001);
    wait_until_frame_stopped_loading(&mut ctx, "TID-CSP").await;
    assert_eq!(
        evaluate_csp_state(&mut ctx, 24_002).await,
        json!([false, "CSP Locked"])
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_003,
        "method": "Page.setBypassCSP",
        "sessionId": "SID-CSP",
        "params": { "enabled": true }
    }))
    .await;
    take_response_by_id(&mut ctx, 24_003);
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 24_004,
        "method": "Page.reload",
        "sessionId": "SID-CSP"
    }))
    .await;
    take_response_by_id(&mut ctx, 24_004);
    wait_until_frame_stopped_loading(&mut ctx, "TID-CSP").await;
    assert_eq!(
        evaluate_csp_state(&mut ctx, 24_005).await,
        json!([true, "CSP Bypassed"])
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_006,
        "method": "Page.setBypassCSP",
        "sessionId": "SID-CSP",
        "params": { "enabled": false }
    }))
    .await;
    take_response_by_id(&mut ctx, 24_006);
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 24_007,
        "method": "Page.reload",
        "sessionId": "SID-CSP"
    }))
    .await;
    take_response_by_id(&mut ctx, 24_007);
    wait_until_frame_stopped_loading(&mut ctx, "TID-CSP").await;
    assert_eq!(
        evaluate_csp_state(&mut ctx, 24_008).await,
        json!([false, "CSP Locked"])
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn child_response_csp_controls_parser_and_dynamic_inline_scripts() {
    async fn parent() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><iframe src="/child"></iframe>"#,
        )
    }

    async fn child() -> impl axum::response::IntoResponse {
        (
            [
                (axum::http::header::CONTENT_TYPE.as_str(), "text/html"),
                ("content-security-policy", "script-src 'nonce-allowed'"),
            ],
            r#"<!doctype html><body>
<script nonce="allowed">
globalThis.__allowedParserInlineRan = true;
globalThis.__inlineCspViolations = [];
addEventListener("securitypolicyviolation", event => {
  __inlineCspViolations.push(`${event.effectiveDirective}:${event.disposition}`);
});
</script>
<script>globalThis.__blockedParserInlineRan = true;</script>
<script nonce="allowed">
const script = document.createElement("script");
script.textContent = "globalThis.__blockedDynamicInlineRan = true";
document.body.append(script);
const allowedScript = document.createElement("script");
allowedScript.nonce = "allowed";
allowedScript.textContent = "globalThis.__allowedDynamicInlineRan = true";
document.body.append(allowedScript);
allowedScript.nonce = "changed-after-connection";
</script>
</body>"#,
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/parent", axum::routing::get(parent))
                .route("/child", axum::routing::get(child)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-CHILD-CSP",
        "TID-CHILD-CSP",
        "SID-CHILD-CSP",
        "about:blank",
    );
    ctx.process_async(json!({
        "id": 24_100,
        "method": "Page.enable",
        "sessionId": "SID-CHILD-CSP"
    }))
    .await;
    take_response_by_id(&mut ctx, 24_100);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_101,
        "method": "Page.navigate",
        "sessionId": "SID-CHILD-CSP",
        "params": { "url": format!("http://{addr}/parent") }
    }))
    .await;
    take_response_by_id(&mut ctx, 24_101);
    wait_until_frame_stopped_loading(&mut ctx, "TID-CHILD-CSP").await;
    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 24_102).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_103,
        "method": "Runtime.enable",
        "sessionId": "SID-CHILD-CSP"
    }))
    .await;
    take_response_by_id(&mut ctx, 24_103);
    let child_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == json!(child_frame_id)
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("child default execution context id");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 24_104,
        "method": "Runtime.evaluate",
        "sessionId": "SID-CHILD-CSP",
        "params": {
            "contextId": child_context_id,
            "expression": "[globalThis.__blockedParserInlineRan === true, globalThis.__allowedParserInlineRan === true, globalThis.__blockedDynamicInlineRan === true, globalThis.__allowedDynamicInlineRan === true, globalThis.__inlineCspViolations]",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 24_104)["result"]["result"]["value"],
        json!([
            false,
            true,
            false,
            true,
            ["script-src-elem:enforce", "script-src-elem:enforce"]
        ])
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn reload_targets_background_owner_without_activation() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    async fn page(
        axum::extract::State(counter): axum::extract::State<Arc<AtomicUsize>>,
    ) -> impl axum::response::IntoResponse {
        let next = counter.fetch_add(1, Ordering::SeqCst) + 1;
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            format!("<!doctype html><html><body>{next}</body></html>"),
        )
    }

    let counter = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_counter = counter.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/page", axum::routing::get(page))
                .with_state(server_counter),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    let page_url = format!("http://{addr}/page");
    let background = PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        page_url.clone(),
    );

    let mut bc = BrowserContext::new("BID-1".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.set_target_url("data:text/html,<title>Active</title><main>active</main>".to_owned());
    bc.insert_page_target_host(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(&page_url, Some("SID-background"))
        .await;
    let initial_html = ctx
        .conn
        .browser_context
        .as_mut()
        .and_then(|browser_context| {
            browser_context
                .background_target_mut("TID-background")
                .and_then(PageTargetHost::loaded_page_mut)
        })
        .expect("loaded background page")
        .serialize_html_async()
        .await
        .expect("background page should serialize HTML");
    assert!(
        initial_html.contains(">1<"),
        "expected first background load to contain counter 1"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 242,
        "method": "Page.reload",
        "sessionId": "SID-background"
    }))
    .await;
    let reload = take_response_by_id(&mut ctx, 242);
    assert_eq!(reload["sessionId"], json!("SID-background"));
    assert_eq!(reload["result"], json!({}));

    let browser_context = ctx.conn.browser_context.as_mut().unwrap();
    assert_eq!(
        browser_context.active_target_id(),
        Some("TID-active"),
        "background Page.reload should not activate the target"
    );
    let background = browser_context
        .background_target_mut("TID-background")
        .expect("background target should remain background");
    let reloaded_html = background
        .loaded_page_mut()
        .expect("loaded background page after reload")
        .serialize_html_async()
        .await
        .expect("background page should serialize HTML");
    assert!(
        reloaded_html.contains(">2<"),
        "expected background reload html to contain counter 2, got {reloaded_html}"
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_passes_referrer_header() {
    async fn page(headers: axum::http::HeaderMap) -> impl axum::response::IntoResponse {
        let referer = headers
            .get(axum::http::header::REFERER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            format!("<!doctype html><html><body>{referer}</body></html>"),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/page", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 401,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": format!("http://{addr}/page"),
            "referrer": "https://www.google.com/"
        }
    }))
    .await;

    let _ = ctx.take_all();
    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(html.contains(">https://www.google.com/<"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_protocol_navigate_to_same_url_does_not_inherit_referrer_or_reload_headers() {
    async fn page(headers: axum::http::HeaderMap) -> impl axum::response::IntoResponse {
        let referer = headers
            .get(axum::http::header::REFERER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let cache_control = headers
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            format!(
                "<!doctype html><html><body>referer={referer};cache={cache_control}</body></html>"
            ),
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let request_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_request_count = request_count.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/page",
                axum::routing::get(move |headers| {
                    let request_count = server_request_count.clone();
                    async move {
                        request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        page(headers).await
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let url = format!("http://{addr}/page");

    let mut loader_ids = Vec::new();
    for id in [402, 403] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": { "url": url.clone() }
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        loader_ids.push(
            response["result"]["loaderId"]
                .as_str()
                .expect("a repeated fragment-free URL remains a cross-document navigation")
                .to_owned(),
        );
        ctx.sent.clear();
    }

    assert_ne!(
        loader_ids[0], loader_ids[1],
        "Chromium assigns a new loader to a repeated fragment-free Page.navigate"
    );
    assert_eq!(
        request_count.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the second Page.navigate must load the URL instead of taking the fragment path"
    );

    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        html.contains(">referer=;cache=<"),
        "protocol Page.navigate without an explicit referrer should stay browser-initiated even when navigating to the current URL, got {html}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_http_navigation_after_runtime_enable_replaces_the_context_group() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><html><body>runtime context replacement</body></html>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let request_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_request_count = request_count.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/page",
                axum::routing::get(move || {
                    let request_count = server_request_count.clone();
                    async move {
                        request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        page().await
                    }
                }),
            ),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let url = format!("http://{addr}/page");

    ctx.process_async(json!({
        "id": 404,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url.clone() }
    }))
    .await;
    let first_loader_id = take_response_by_id(&mut ctx, 404)["result"]["loaderId"]
        .as_str()
        .expect("first HTTP navigation should have a loader")
        .to_owned();
    // Page.navigate acknowledges the navigation before the renderer reaches
    // load. Chromium clients wait for a lifecycle event when they need a
    // fully completed document; reading readyState immediately after the
    // command response is allowed to observe "interactive".
    wait_until_renderer_document_load(&mut ctx, Some("SID-1"), "TID-1", &first_loader_id).await;
    ctx.process_async(json!({
        "id": 4041,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.readyState",
            "returnByValue": true
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 4041)["result"]["result"]["value"],
        json!("complete"),
        "the benchmark enables Runtime only after the first document has completed"
    );
    let state_before_enable = ctx
        .conn
        .navigation_load_inputs_for_session_owner(Some("SID-1"))
        .runtime_inspector_session_restore_snapshots
        .into_iter()
        .find(|restore| restore.inspector_session_id.is_none())
        .and_then(|restore| restore.v8_attach.reattach_state().cloned())
        .expect("Runtime.evaluate should establish the primary V8 session state");
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 405,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["sessionId"] == json!("SID-1")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
        }),
        "Runtime.enable should report the first HTTP document context: {:?}",
        ctx.sent
    );
    let state_after_enable = ctx
        .conn
        .navigation_load_inputs_for_session_owner(Some("SID-1"))
        .runtime_inspector_session_restore_snapshots
        .into_iter()
        .find(|restore| restore.inspector_session_id.is_none())
        .and_then(|restore| restore.v8_attach.reattach_state().cloned())
        .expect("Runtime.enable should retain the primary V8 session state");
    assert_ne!(
        state_before_enable, state_after_enable,
        "Runtime.enable must replace the pre-enable V8 session cookie"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 406,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;
    let second_loader_id = take_response_by_id(&mut ctx, 406)["result"]["loaderId"]
        .as_str()
        .expect("second HTTP navigation should have a loader")
        .to_owned();

    assert_ne!(first_loader_id, second_loader_id);
    assert_eq!(
        request_count.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "same-URL Page.navigate should fetch and commit a replacement document"
    );
    assert_runtime_navigation_context_reset(&ctx.sent, "SID-1", "TID-1");

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn reload_after_crash_emits_target_reloaded_after_crash() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-1",
        "TID-1",
        "SID-1",
        "data:text/html,<body>reload-after-crash</body>",
    );
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .record_inspector_target_crashed();
    bc.active_page_target_mut()
        .owner_state
        .target_crash_state
        .mark_crashed();

    ctx.process_async(json!({
        "id": 248,
        "method": "Page.reload",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 248);

    let events = ctx.take_all();
    assert!(events.iter().any(|message| {
        message["method"] == json!("Inspector.targetReloadedAfterCrash")
            && message["sessionId"] == json!("SID-1")
    }));
    assert!(
        events
            .iter()
            .any(|message| message["method"] == json!("Page.frameNavigated"))
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .owner_state
            .target_crash_state
            .is_crashed()
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_crash_emits_target_reloaded_after_crash() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-1",
        "TID-1",
        "SID-1",
        "data:text/html,<body>before-crash</body>",
    );
    let bc = ctx.conn.browser_context.as_mut().unwrap();
    bc.active_page_target_mut().devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
        .runtime_session_state
        .record_inspector_target_crashed();
    bc.active_page_target_mut()
        .owner_state
        .target_crash_state
        .mark_crashed();

    ctx.process_async(json!({
        "id": 249,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<body>after-crash</body>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 249);

    let events = ctx.take_all();
    assert!(events.iter().any(|message| {
        message["method"] == json!("Inspector.targetReloadedAfterCrash")
            && message["sessionId"] == json!("SID-1")
    }));
    assert!(
        events
            .iter()
            .any(|message| message["method"] == json!("Page.frameNavigated"))
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .owner_state
            .target_crash_state
            .is_crashed()
    );
    assert_eq!(
        loaded_page_html_for_test(&mut ctx).await,
        "<html><head></head><body>after-crash</body></html>"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn navigate_after_crash_without_inspector_enabled_clears_crash_without_event() {
    let mut ctx = TestContext::new();
    load_bc_with_session(
        &mut ctx,
        "BID-1",
        "TID-1",
        "SID-1",
        "data:text/html,<body>before-crash</body>",
    );
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .owner_state
        .target_crash_state
        .mark_crashed();

    ctx.process_async(json!({
        "id": 250,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<body>after-crash</body>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 250);

    let events = ctx.take_all();
    assert!(
        !events
            .iter()
            .any(|message| message["method"] == json!("Inspector.targetReloadedAfterCrash"))
    );
    assert!(
        events
            .iter()
            .any(|message| message["method"] == json!("Page.frameNavigated"))
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .owner_state
            .target_crash_state
            .is_crashed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_script_location_navigation_suppresses_aborted_document_dcl() {
    async fn challenge() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><script>window.location.href = '/final'</script>",
        )
    }

    async fn final_page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Final document</title><main>final content</main>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/challenge", axum::routing::get(challenge))
                .route("/final", axum::routing::get(final_page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({
        "id": 251,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/challenge") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 251);
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "successor document lifecycle after parser-script navigation",
        |messages| {
            messages
                .iter()
                .filter(|message| message["method"] == json!("Page.frameNavigated"))
                .count()
                >= 2
                && messages
                    .iter()
                    .any(|message| message["method"] == json!("Page.domContentEventFired"))
        },
    )
    .await;

    let events = ctx.take_all();
    let domcontentloaded = events
        .iter()
        .filter(|message| message["method"] == json!("Page.domContentEventFired"))
        .collect::<Vec<_>>();
    assert_eq!(
        domcontentloaded.len(),
        1,
        "the synchronously aborted challenge document must not emit DCL: {events:?}"
    );
    let final_frame_commit_index = events
        .iter()
        .rposition(|message| message["method"] == json!("Page.frameNavigated"))
        .expect("final document frame commit");
    let domcontentloaded_index = events
        .iter()
        .position(|message| message["method"] == json!("Page.domContentEventFired"))
        .expect("final document DCL");
    assert!(
        final_frame_commit_index < domcontentloaded_index,
        "the only DCL must belong to the final document: {events:?}"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .target_url(),
        format!("http://{addr}/final")
    );
    assert!(
        loaded_page_html_for_test(&mut ctx)
            .await
            .contains("final content")
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_script_location_navigation_continues_after_target_response() {
    async fn challenge() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><script>window.location.href = '/final'</script>",
        )
    }

    let final_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let final_release = std::sync::Arc::new(tokio::sync::Notify::new());
    let requested_for_handler = std::sync::Arc::clone(&final_requested);
    let release_for_handler = std::sync::Arc::clone(&final_release);
    let final_handler = move || {
        let requested = std::sync::Arc::clone(&requested_for_handler);
        let release = std::sync::Arc::clone(&release_for_handler);
        async move {
            requested.notify_one();
            release.notified().await;
            (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                "<!doctype html><title>Delayed final</title><main>delayed final content</main>",
            )
        }
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/challenge", axum::routing::get(challenge))
                .route("/final", axum::routing::get(final_handler)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({
        "id": 252,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/challenge") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 252);

    // Chromium returns Page.navigate once the new document commits. Parser
    // continuation (and therefore this script navigation) is admitted only
    // after that response boundary has been flushed.
    let release_response = tokio::spawn(async move {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            final_requested.notified(),
        )
        .await
        .expect("successor navigation should request the gated final response");
        final_release.notify_one();
    });
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "response-gated successor document lifecycle",
        |messages| {
            messages
                .iter()
                .filter(|message| message["method"] == json!("Page.frameNavigated"))
                .count()
                >= 2
                && messages
                    .iter()
                    .any(|message| message["method"] == json!("Page.domContentEventFired"))
        },
    )
    .await;
    release_response
        .await
        .expect("gated final-response release task");

    let events = ctx.take_all();
    assert_eq!(
        events
            .iter()
            .filter(|message| message["method"] == json!("Page.domContentEventFired"))
            .count(),
        1,
        "the response-gated source document must stay terminated before DCL: {events:?}"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .target_url(),
        format!("http://{addr}/final")
    );

    server.abort();
}

async fn assert_handler_navigation_renderer_lifecycle(
    event_name: &str,
    expected_source_milestone_sequences: &[&[&str]],
) {
    async fn final_page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Final document</title><main>handler final content</main>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let event_target = if event_name == "DOMContentLoaded" {
        "document"
    } else {
        "window"
    };
    let source_html = format!(
        "<!doctype html><script>{event_target}.addEventListener({event_name:?},()=>{{location.href='/final'}},{{once:true}})</script><main>handler-source-{event_name}</main>"
    );
    let source_handler = move || {
        let html = source_html.clone();
        async move {
            (
                [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                html,
            )
        }
    };
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/source", axum::routing::get(source_handler))
                .route("/final", axum::routing::get(final_page)),
        )
        .await
        .unwrap();
    });

    let final_url = format!("http://{addr}/final");
    let source_url = format!("http://{addr}/source");
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({
        "id": 253,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-1",
        "params": { "enabled": true }
    }))
    .await;
    ctx.expect_result(253, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 254,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": source_url.clone() }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 254);
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "handler-triggered successor document load",
        |messages| {
            let Some(final_commit_index) = messages.iter().position(|message| {
                message["method"] == json!("Page.frameNavigated")
                    && message["params"]["frame"]["url"] == json!(final_url)
            }) else {
                return false;
            };
            messages[final_commit_index + 1..]
                .iter()
                .any(|message| message["method"] == json!("Page.loadEventFired"))
        },
    )
    .await;

    let events = ctx.take_all();
    let source_commit_index = events
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["url"] == json!(source_url)
        })
        .unwrap_or_else(|| panic!("source document should commit: {events:?}"));
    let source_loader = events[source_commit_index]["params"]["frame"]["loaderId"]
        .as_str()
        .expect("source loader id");
    let final_commit_index = events
        .iter()
        .position(|message| {
            message["method"] == json!("Page.frameNavigated")
                && message["params"]["frame"]["url"] == json!(final_url)
        })
        .unwrap_or_else(|| panic!("successor document should commit: {events:?}"));
    let final_loader = events[final_commit_index]["params"]["frame"]["loaderId"]
        .as_str()
        .expect("final loader id");
    assert_ne!(source_loader, final_loader);

    let milestones_for_loader = |loader_id: &str| {
        events
            .iter()
            .filter(|message| {
                message["method"] == json!("Page.lifecycleEvent")
                    && message["params"]["loaderId"] == json!(loader_id)
                    && matches!(
                        message["params"]["name"].as_str(),
                        Some("DOMContentLoaded" | "load")
                    )
            })
            .map(|message| message["params"]["name"].as_str().unwrap())
            .collect::<Vec<_>>()
    };
    let source_milestones = milestones_for_loader(source_loader);
    assert!(
        expected_source_milestone_sequences.contains(&source_milestones.as_slice()),
        "source milestone sequence should reflect the handler return boundary: {events:?}"
    );
    assert_eq!(
        milestones_for_loader(final_loader),
        vec!["DOMContentLoaded", "load"],
        "successor document should complete normally: {events:?}"
    );
    assert!(
        source_milestones.iter().all(|name| {
            events.iter().position(|message| {
                message["method"] == json!("Page.lifecycleEvent")
                    && message["params"]["loaderId"] == json!(source_loader)
                    && message["params"]["name"] == json!(name)
            }) < Some(final_commit_index)
        }),
        "source milestones must be emitted before the successor commit: {events:?}"
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn domcontentloaded_handler_navigation_records_dcl_before_termination() {
    const NO_SOURCE_MILESTONES: &[&str] = &[];
    const SOURCE_DCL_MILESTONE: &[&str] = &["DOMContentLoaded"];

    assert_handler_navigation_renderer_lifecycle(
        "DOMContentLoaded",
        &[NO_SOURCE_MILESTONES, SOURCE_DCL_MILESTONE],
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn load_handler_navigation_records_load_before_termination() {
    const SOURCE_LOAD_MILESTONES: &[&str] = &["DOMContentLoaded", "load"];

    assert_handler_navigation_renderer_lifecycle("load", &[SOURCE_LOAD_MILESTONES]).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn delayed_meta_refresh_keeps_source_document_dcl_and_load() {
    async fn refreshing_page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><meta http-equiv='refresh' content='1; url=/final'>",
        )
    }

    async fn final_page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><title>Meta final</title><main>meta final content</main>",
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/refresh", axum::routing::get(refreshing_page))
                .route("/final", axum::routing::get(final_page)),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({
        "id": 252,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/refresh") }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 252);
    let final_url = format!("http://{addr}/final");
    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "meta-refresh successor document load",
        |messages| {
            let Some(final_commit_index) = messages.iter().position(|message| {
                message["method"] == json!("Page.frameNavigated")
                    && message["params"]["frame"]["url"] == json!(final_url)
            }) else {
                return false;
            };
            messages[final_commit_index + 1..]
                .iter()
                .any(|message| message["method"] == json!("Page.loadEventFired"))
        },
    )
    .await;

    let events = ctx.take_all();
    assert_eq!(
        events
            .iter()
            .filter(|message| message["method"] == json!("Page.domContentEventFired"))
            .count(),
        2,
        "meta refresh is queued after source load, so both documents must report DCL: {events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|message| message["method"] == json!("Page.loadEventFired"))
            .count(),
        2,
        "meta refresh starts after source load, so both documents must report load: {events:?}"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .target_url(),
        final_url
    );

    server.abort();
}

// Ported from WPT
// html/browsers/browsing-the-web/navigating-across-documents/refresh/
// same-document-refresh.html. The protocol boundary additionally proves that
// the fragment update never re-enters the network loader.
#[tokio::test(flavor = "multi_thread")]
async fn fragment_meta_refresh_is_one_same_document_navigation() {
    async fn refreshing_page(
        axum::extract::State(request_count): axum::extract::State<
            std::sync::Arc<std::sync::atomic::AtomicUsize>,
        >,
    ) -> impl axum::response::IntoResponse {
        request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><meta http-equiv='refresh' content='0; url=#done'><main>source</main>",
        )
    }

    let request_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_count = request_count.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/refresh", axum::routing::get(refreshing_page))
                .with_state(server_count),
        )
        .await
        .unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({
        "id": 253,
        "method": "Page.enable",
        "sessionId": "SID-1",
    }))
    .await;
    ctx.expect_result(253, json!({}), Some("SID-1"));
    ctx.sent.clear();

    let source_url = format!("http://{addr}/refresh");
    let fragment_url = format!("{source_url}#done");
    ctx.process_async(json!({
        "id": 254,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": source_url }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 254);
    let source_loader = response["result"]["loaderId"]
        .as_str()
        .expect("cross-document source navigation should report a loader")
        .to_owned();
    wait_until_message(&mut ctx, "SID-1", "fragment meta refresh", |message| {
        message["method"] == json!("Page.navigatedWithinDocument")
            && message["params"]["url"] == json!(fragment_url)
    })
    .await;

    ctx.process_async(json!({
        "id": 255,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "location.href",
            "returnByValue": true,
        }
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 255)["result"]["result"]["value"],
        json!(fragment_url)
    );

    let events = ctx.take_all();
    assert_eq!(
        request_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "a fragment refresh must not request the source document again: {events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|message| {
                message["method"] == json!("Page.frameNavigated")
                    && message["params"]["frame"]["loaderId"] == json!(source_loader)
            })
            .count(),
        1,
        "the refresh must retain the source loader: {events:?}"
    );
    assert!(
        events.iter().any(|message| {
            message["method"] == json!("Page.navigatedWithinDocument")
                && message["params"]["navigationType"] == json!("fragment")
                && message["params"]["url"] == json!(fragment_url)
        }),
        "the renderer must publish one fragment navigation event: {events:?}"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .target_url(),
        fragment_url
    );

    server.abort();
}

// Ported from Blink HTMLDocumentParserLoadingTest::
// ShouldPauseParsingForExternalStylesheetsInBody on the ordinary navigation
// parser path. The phase-one owner test covers the paused intermediate DOM;
// this protocol regression covers completion, cascade, and lifecycle resume.
#[tokio::test(flavor = "multi_thread")]
async fn navigation_body_stylesheet_pauses_parser_tail_until_completion() {
    let stylesheet_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_stylesheet = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_requested = stylesheet_requested.clone();
    let handler_release = release_stylesheet.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let page_html = format!(
        concat!(
            "<!doctype html><html><body>",
            "<main id=navigation-style-before>before</main>",
            "<link rel=stylesheet href='http://{addr}/navigation-pause.css'>",
            "<footer id=navigation-style-after>after</footer>",
            "</body></html>"
        ),
        addr = addr,
    );
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/navigation-page",
                axum::routing::get(move || {
                    let html = page_html.clone();
                    async move {
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                            html,
                        )
                    }
                }),
            )
            .route(
                "/navigation-pause.css",
                axum::routing::get(move || {
                    let requested = handler_requested.clone();
                    let release = handler_release.clone();
                    async move {
                        requested.notify_one();
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#navigation-style-after { color: rgb(161, 162, 163); }",
                        )
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.process_async(json!({
        "id": 600,
        "method": "Page.setLifecycleEventsEnabled",
        "sessionId": "SID-1",
        "params": { "enabled": true },
    }))
    .await;
    ctx.expect_result(600, json!({}), Some("SID-1"));
    ctx.sent.clear();

    let requested_for_release = stylesheet_requested.clone();
    let release_from_server_barrier = release_stylesheet.clone();
    let stylesheet_completion = tokio::spawn(async move {
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            requested_for_release.notified(),
        )
        .await
        .expect("the navigation parser should request its body stylesheet");
        release_from_server_barrier.notify_one();
    });
    ctx.process_async(json!({
        "id": 602,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": format!("http://{addr}/navigation-page") },
    }))
    .await;
    stylesheet_completion
        .await
        .expect("stylesheet release coordinator should complete");
    let response = take_response_by_id(&mut ctx, 602);
    assert_eq!(response["result"]["frameId"], json!("TID-1"));
    wait_until_message(
        &mut ctx,
        "SID-1",
        "navigation body stylesheet load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    ctx.process_async(json!({
        "id": 604,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "(() => { const after = document.getElementById('navigation-style-after'); return { text: after?.textContent, color: getComputedStyle(after).color }; })()",
            "returnByValue": true,
        },
    }))
    .await;
    let completed = take_response_by_id(&mut ctx, 604);
    assert_eq!(
        completed["result"]["result"]["value"],
        json!({ "text": "after", "color": "rgb(161, 162, 163)" }),
    );

    server.abort();
}
