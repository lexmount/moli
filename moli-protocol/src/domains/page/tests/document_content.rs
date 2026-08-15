use super::*;

async fn install_document_content_test_page(ctx: &mut TestContext, url: &str) {
    load_bc_with_session(ctx, "BID-set-content", "TID-1", "SID-1", "about:blank");
    let committed_document = {
        let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
        browser_context.set_target_url(url.to_owned());
        browser_context
            .start_document_navigation_for_active_target(LOADER_ID.to_owned())
            .expect("document-content test navigation should start")
    };
    let mut navigation = ctx
        .conn
        .load_navigation_via_runtime_for_session_owner_async(Some("SID-1"), url)
        .await
        .expect("document-content test page should load");
    let navigation_engine = navigation.navigation_engine.take();
    let artifacts = navigation.page_creation_artifacts;
    {
        let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
        let renderer_agent_candidate = browser_context
            .active_target
            .runtime_slot
            .prepare_renderer_agent_candidate(&committed_document, &mut navigation.page)
            .expect("document-content test renderer candidate should attach");
        browser_context.commit_document_navigation_if_matches(&committed_document);
        browser_context
            .active_target
            .runtime_slot
            .commit_loaded_navigation_renderer_attachment(
                &mut navigation.page,
                Some(renderer_agent_candidate),
            )
            .expect("document-content test renderer candidate should commit");
        browser_context
            .active_target
            .runtime_slot
            .set_loaded_page_for_test(navigation.page);
        assert!(
            browser_context
                .active_target
                .runtime_slot
                .finish_renderer_document_navigation(&committed_document)
                .expect("document-content test renderer navigation should finish")
                .released_output
                .is_empty(),
            "the fixture should not leave buffered Inspector output behind"
        );
    }
    let (binding, _) = ctx.conn.bind_renderer_document_lifecycle_for_session_owner(
        Some("SID-1"),
        artifacts,
        Some(committed_document),
        "TID-1".to_owned(),
        LOADER_ID.to_owned(),
    );
    assert!(binding.is_some(), "renderer lifecycle should bind");
    if let Some(navigation_engine) = navigation_engine {
        ctx.conn
            .adopt_loaded_navigation_engine_for_session_owner(Some("SID-1"), navigation_engine);
    }
    // The fixture commits an already-running renderer Page directly instead
    // of going through the production navigation command. Route that exact
    // Document's initial lifecycle publication before enabling observers.
    // Otherwise Page.enable can replay its old load while a later
    // setDocumentContent replacement is running, and a test may mistake the
    // previous epoch for the replacement's terminal event.
    wait_until_renderer_document_load(ctx, Some("SID-1"), "TID-1", LOADER_ID).await;
}

async fn enable_document_content_observers(ctx: &mut TestContext) {
    for (id, method, params) in [
        (9, "Page.enable", json!({})),
        (
            10,
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
        ),
        (11, "Runtime.enable", json!({})),
        (12, "DOM.enable", json!({})),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "sessionId": "SID-1",
            "params": params,
        }))
        .await;
        let response = take_response_by_id(ctx, id);
        assert_eq!(
            response["result"],
            json!({}),
            "{method} should succeed: {response:?}"
        );
    }
}

async fn frame_tree(ctx: &mut TestContext, id: u64) -> serde_json::Value {
    ctx.process_async(json!({
        "id": id,
        "method": "Page.getFrameTree",
        "sessionId": "SID-1",
    }))
    .await;
    take_response_by_id(ctx, id)["result"]["frameTree"].clone()
}

fn assert_initial_child_frame_is_attached(ctx: &TestContext, frame_id: &str) {
    // `install_document_content_test_page()` settles the initial renderer
    // publication before Page.enable. Chromium's Page.enable only subscribes
    // to future Page events; it does not replay a historical frameNavigated
    // notification (InspectorPageAgent::enable only installs the agent).
    // The authoritative owner state, rather than a late protocol event, is
    // therefore the correct readiness boundary for this already-committed
    // child Document.
    assert_eq!(
        ctx.conn
            .target_owner_has_attached_child_frame_id_for_session(Some("SID-1"), frame_id,),
        Some(true),
        "the initial child frame must be committed before setDocumentContent"
    );
}

async fn evaluate_by_value(
    ctx: &mut TestContext,
    id: u64,
    context_id: Option<i64>,
    expression: &str,
) -> serde_json::Value {
    evaluate_by_value_for_session(ctx, id, "SID-1", context_id, expression).await
}

async fn evaluate_by_value_for_session(
    ctx: &mut TestContext,
    id: u64,
    session_id: &str,
    context_id: Option<i64>,
    expression: &str,
) -> serde_json::Value {
    let mut params = json!({
        "expression": expression,
        "returnByValue": true,
    });
    if let Some(context_id) = context_id {
        params["contextId"] = json!(context_id);
    }
    ctx.process_async(json!({
        "id": id,
        "method": "Runtime.evaluate",
        "sessionId": session_id,
        "params": params,
    }))
    .await;
    let response = take_response_by_id(ctx, id);
    assert!(
        response["error"].is_null(),
        "Runtime.evaluate should succeed: {response:?}"
    );
    assert!(
        response["result"]["exceptionDetails"].is_null(),
        "Runtime.evaluate should not throw: {response:?}"
    );
    assert!(
        !response["result"]["result"]["value"].is_null(),
        "Runtime.evaluate should return a by-value result: {response:?}"
    );
    response["result"]["result"]["value"].clone()
}

// Ported from Chromium's inspector-protocol/sessions/
// page-set-document-content.js, with Unicode content from the single-session
// setDocumentContent inspector test retained in the same regression.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_is_observable_from_another_attached_session() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(
        &mut ctx,
        "data:text/html,<body><main id=before>before</main></body>",
    )
    .await;
    assert!(
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .assign_auxiliary_session_to_target("TID-1", "SID-2".to_owned())
    );

    for (id, session_id, method, params) in [
        (12, "SID-1", "Page.enable", json!({})),
        (
            13,
            "SID-1",
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
        ),
        (14, "SID-1", "DOM.enable", json!({})),
        (15, "SID-2", "Page.enable", json!({})),
        (
            16,
            "SID-2",
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
        ),
        (17, "SID-2", "DOM.enable", json!({})),
        (18, "SID-2", "Runtime.enable", json!({})),
        (118, "SID-1", "Runtime.enable", json!({})),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": method,
            "sessionId": session_id,
            "params": params,
        }))
        .await;
        let response = take_response_by_id(&mut ctx, id);
        assert_eq!(response["result"], json!({}), "{method}: {response:?}");
    }
    assert_eq!(
        evaluate_by_value_for_session(
            &mut ctx,
            19,
            "SID-2",
            None,
            "globalThis.__secondSessionOldDocument = document; 'ready'",
        )
        .await,
        json!("ready")
    );
    ctx.process_async(json!({
        "id": 20,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "new Promise(resolve => { globalThis.__releaseConcurrentDocumentContentAwait = resolve; })"
        }
    }))
    .await;
    let promise_object_id = take_response_by_id(&mut ctx, 20)["result"]["result"]["objectId"]
        .as_str()
        .expect("Runtime.evaluate should return the concurrent promise handle")
        .to_owned();
    ctx.process_command_only_async(json!({
        "id": 21,
        "method": "Runtime.awaitPromise",
        "sessionId": "SID-1",
        "params": {
            "promiseObjectId": promise_object_id,
            "returnByValue": true
        }
    }))
    .await;
    assert!(
        !ctx.sent.iter().any(|message| message["id"] == json!(21)),
        "the primary session Runtime.awaitPromise should remain pending"
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 22,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-2",
        "params": {
            "frameId": "TID-1",
            "html": "<main id=after>こんにちは世界</main><script>console.log('multi-session-set-content')</script>"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 22);
    assert_eq!(response["result"], json!({}), "{response:?}");

    wait_until_scheduler_message(
        &mut ctx,
        "setDocumentContent load event on the auxiliary session",
        |message| {
            message["method"] == json!("Page.loadEventFired")
                && message["sessionId"] == json!("SID-2")
        },
    )
    .await;
    assert!(
        ctx.sent.iter().any(|message| {
            message["method"] == json!("Page.loadEventFired")
                && message["sessionId"] == json!("SID-1")
        }),
        "setDocumentContent lifecycle should fan out to the primary and auxiliary sessions: {:?}",
        ctx.sent
    );
    assert!(
        ["SID-1", "SID-2"].into_iter().all(|session_id| {
            ctx.sent.iter().any(|message| {
                message["method"] == json!("DOM.documentUpdated")
                    && message["sessionId"] == json!(session_id)
            })
        }),
        "setDocumentContent DOM refresh should fan out to every attached session: {:?}",
        ctx.sent
    );
    for session_id in ["SID-1", "SID-2"] {
        assert_eq!(
            ctx.sent
                .iter()
                .filter(|message| {
                    message["method"] == json!("Runtime.consoleAPICalled")
                        && message["sessionId"] == json!(session_id)
                        && message["params"]["args"][0]["value"]
                            == json!("multi-session-set-content")
                })
                .count(),
            1,
            "setDocumentContent console output should reach {session_id} exactly once: {:?}",
            ctx.sent
        );
    }

    assert_eq!(
        evaluate_by_value_for_session(
            &mut ctx,
            23,
            "SID-2",
            None,
            "({ sameDocument: document === __secondSessionOldDocument, text: document.querySelector('#after').textContent })",
        )
        .await,
        json!({ "sameDocument": true, "text": "こんにちは世界" })
    );
    assert_eq!(
        evaluate_by_value_for_session(
            &mut ctx,
            24,
            "SID-2",
            None,
            "__releaseConcurrentDocumentContentAwait('released'); 'released'",
        )
        .await,
        json!("released")
    );
    wait_until_scheduler_message(
        &mut ctx,
        "released primary-session awaitPromise response",
        |message| message["id"] == json!(21) && message["sessionId"] == json!("SID-1"),
    )
    .await;
    let await_response = take_response_by_id(&mut ctx, 21);
    assert_eq!(
        await_response["result"]["result"]["value"],
        json!("released"),
        "{await_response:?}"
    );
}

// Ported from WPT opening-the-input-stream/history-state.window.js and
// history.window.js. Document::SetContent uses Document::open internally and
// must preserve the current session-history item and its serialized state.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_preserves_history_length_and_state() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(
                "/history",
                axum::routing::get(|| async {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        "<!doctype html><body><main id=before>before</main></body>",
                    )
                }),
            ),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, &format!("http://{addr}/history")).await;
    let before = evaluate_by_value(
        &mut ctx,
        17,
        None,
        "history.replaceState({ marker: 41 }, ''); ({ length: history.length, state: history.state })",
    )
    .await;
    assert_eq!(before["state"], json!({ "marker": 41 }));

    ctx.process_async(json!({
        "id": 18,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": "<main id=after>after</main>"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 18);
    assert_eq!(response["result"], json!({}));

    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            19,
            None,
            "({ length: history.length, state: history.state, text: document.querySelector('#after').textContent })",
        )
        .await,
        json!({
            "length": before["length"],
            "state": { "marker": 41 },
            "text": "after",
        })
    );
    server.abort();
}

// Ported from WPT opening-the-input-stream/mutation-observer.window.js and
// verified against Chromium's Page.setDocumentContent path. Unlike a bare
// document.open(), SetContent also exposes the parser's subsequent additions.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_reports_document_open_and_parser_mutations() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(
        &mut ctx,
        "data:text/html,<body><main id=before>before</main></body>",
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            53,
            None,
            r#"
                globalThis.__setContentOldHtml = document.documentElement;
                globalThis.__setContentMutationRecords = [];
                globalThis.__setContentMutationObserver = new MutationObserver(records => {
                    __setContentMutationRecords.push(...records.map(record => ({
                        target: record.target.nodeName,
                        added: Array.from(record.addedNodes, node => node.nodeName),
                        removed: Array.from(record.removedNodes, node => node.nodeName),
                        removedOldHtml: record.removedNodes[0] === __setContentOldHtml,
                    })));
                });
                __setContentMutationObserver.observe(document, { childList: true, subtree: true });
                'ready';
            "#,
        )
        .await,
        json!("ready")
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 54,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": "<main id=after>after</main>"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 54);
    assert_eq!(response["result"], json!({}), "{response:?}");

    assert_eq!(
        evaluate_by_value(&mut ctx, 55, None, "__setContentMutationRecords",).await,
        json!([
            {
                "target": "#document",
                "added": [],
                "removed": ["HTML"],
                "removedOldHtml": true,
            },
            {
                "target": "#document",
                "added": ["HTML"],
                "removed": [],
                "removedOldHtml": false,
            },
            {
                "target": "HTML",
                "added": ["HEAD"],
                "removed": [],
                "removedOldHtml": false,
            },
            {
                "target": "HTML",
                "added": ["BODY"],
                "removed": [],
                "removedOldHtml": false,
            },
            {
                "target": "BODY",
                "added": ["MAIN"],
                "removed": [],
                "removedOldHtml": false,
            },
            {
                "target": "MAIN",
                "added": ["#text"],
                "removed": [],
                "removedOldHtml": false,
            },
        ])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_coalesces_old_roots_and_preserves_parser_insertion_order() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(
        &mut ctx,
        "data:text/html,<!doctype html><body><main>before</main></body>",
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            56,
            None,
            r#"
                globalThis.__setContentOldChildren = Array.from(document.childNodes);
                globalThis.__setContentRootRecords = [];
                globalThis.__setContentRootObserver = new MutationObserver(records => {
                    __setContentRootRecords.push(...records.map(record => ({
                        target: record.target.nodeName,
                        added: Array.from(record.addedNodes, node => `${node.nodeType}:${node.nodeName}`),
                        removed: Array.from(record.removedNodes, node => `${node.nodeType}:${node.nodeName}`),
                        removedOldRootsInOrder: Array.from(
                            record.removedNodes,
                            (node, index) => node === __setContentOldChildren[index]
                        ).every(Boolean),
                    })));
                });
                __setContentRootObserver.observe(document, { childList: true, subtree: true });
                'ready';
            "#,
        )
        .await,
        json!("ready")
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 57,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": "<!doctype html><title>title</title><main>after</main>"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 57);
    assert_eq!(response["result"], json!({}), "{response:?}");

    assert_eq!(
        evaluate_by_value(&mut ctx, 58, None, "__setContentRootRecords").await,
        json!([
            {
                "target": "#document",
                "added": [],
                "removed": ["10:html", "1:HTML"],
                "removedOldRootsInOrder": true,
            },
            {
                "target": "#document",
                "added": ["10:html"],
                "removed": [],
                "removedOldRootsInOrder": true,
            },
            {
                "target": "#document",
                "added": ["1:HTML"],
                "removed": [],
                "removedOldRootsInOrder": true,
            },
            {
                "target": "HTML",
                "added": ["1:HEAD"],
                "removed": [],
                "removedOldRootsInOrder": true,
            },
            {
                "target": "HEAD",
                "added": ["1:TITLE"],
                "removed": [],
                "removedOldRootsInOrder": true,
            },
            {
                "target": "TITLE",
                "added": ["3:#text"],
                "removed": [],
                "removedOldRootsInOrder": true,
            },
            {
                "target": "HTML",
                "added": ["1:BODY"],
                "removed": [],
                "removedOldRootsInOrder": true,
            },
            {
                "target": "BODY",
                "added": ["1:MAIN"],
                "removed": [],
                "removedOldRootsInOrder": true,
            },
            {
                "target": "MAIN",
                "added": ["3:#text"],
                "removed": [],
                "removedOldRootsInOrder": true,
            },
        ])
    );
}

// Verified against local Chromium. Foster-parented nodes are inserted before
// the table after the TABLE itself has already been observed, so final-tree
// preorder is not a valid substitute for parser mutation order.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_preserves_foster_parent_parser_mutation_order() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            59,
            None,
            r#"
                globalThis.__fosterRecords = [];
                globalThis.__fosterObserver = new MutationObserver(records => {
                    __fosterRecords.push(...records.map(record => ({
                        target: record.target.nodeName,
                        added: Array.from(record.addedNodes, node => node.nodeName),
                        removed: Array.from(record.removedNodes, node => node.nodeName),
                    })));
                });
                __fosterObserver.observe(document, { childList: true, subtree: true });
                'ready';
            "#,
        )
        .await,
        json!("ready")
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 60,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": "<table>before<div>x</div><tr><td>cell</td></tr>after</table>"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 60);
    assert_eq!(response["result"], json!({}), "{response:?}");

    assert_eq!(
        evaluate_by_value(&mut ctx, 61, None, "__fosterRecords").await,
        json!([
            { "target": "#document", "added": [], "removed": ["HTML"] },
            { "target": "#document", "added": ["HTML"], "removed": [] },
            { "target": "HTML", "added": ["HEAD"], "removed": [] },
            { "target": "HTML", "added": ["BODY"], "removed": [] },
            { "target": "BODY", "added": ["TABLE"], "removed": [] },
            { "target": "BODY", "added": ["#text"], "removed": [] },
            { "target": "BODY", "added": ["DIV"], "removed": [] },
            { "target": "DIV", "added": ["#text"], "removed": [] },
            { "target": "TABLE", "added": ["TBODY"], "removed": [] },
            { "target": "TBODY", "added": ["TR"], "removed": [] },
            { "target": "TR", "added": ["TD"], "removed": [] },
            { "target": "TD", "added": ["#text"], "removed": [] },
            { "target": "BODY", "added": ["#text"], "removed": [] },
        ])
    );
}

fn message_index(
    messages: &[serde_json::Value],
    description: &str,
    mut matches: impl FnMut(&serde_json::Value) -> bool,
) -> usize {
    messages
        .iter()
        .position(&mut matches)
        .unwrap_or_else(|| panic!("expected {description}; messages={messages:?}"))
}

fn console_message_index(messages: &[serde_json::Value], value: &str) -> usize {
    message_index(
        messages,
        &format!("Runtime.consoleAPICalled `{value}`"),
        |message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["args"][0]["value"] == json!(value)
        },
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_works_before_any_explicit_runtime_evaluation() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(
        &mut ctx,
        "data:text/html,<body><main id=before>before</main></body>",
    )
    .await;

    ctx.process_async(json!({
        "id": 18,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": "<main id=after>after</main>"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 18);
    assert_eq!(response["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            19,
            None,
            "document.querySelector('#after').textContent",
        )
        .await,
        json!("after")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_set_document_content_replaces_stylesheet_candidates_without_stale_cascade() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(
        &mut ctx,
        "data:text/html,<style>#probe{color:rgb(200,1,2)}</style><main id=probe>old</main>",
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            62,
            None,
            "globalThis.__oldStyledProbe = document.querySelector('#probe'); ({ color: getComputedStyle(__oldStyledProbe).color, sheets: document.styleSheets.length })",
        )
        .await,
        json!({ "color": "rgb(200, 1, 2)", "sheets": 1 })
    );

    ctx.process_async(json!({
        "id": 63,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": { "frameId": "TID-1", "html": "<main id=probe>unstyled</main>" }
    }))
    .await;
    ctx.expect_result(63, json!({}), Some("SID-1"));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            64,
            None,
            "({ color: getComputedStyle(document.querySelector('#probe')).color, sheets: document.styleSheets.length, oldConnected: __oldStyledProbe.isConnected })",
        )
        .await,
        json!({ "color": "rgb(0, 0, 0)", "sheets": 0, "oldConnected": false })
    );

    ctx.process_async(json!({
        "id": 65,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": "<style>#probe{color:rgb(1,2,3)}</style><main id=probe>restyled</main>"
        }
    }))
    .await;
    ctx.expect_result(65, json!({}), Some("SID-1"));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            66,
            None,
            "({ color: getComputedStyle(document.querySelector('#probe')).color, sheets: document.styleSheets.length })",
        )
        .await,
        json!({ "color": "rgb(1, 2, 3)", "sheets": 1 })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_runs_disconnected_callback_once_for_removed_custom_element() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body></body>").await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            67,
            None,
            r#"
                globalThis.__setContentDisconnected = 0;
                customElements.define('set-content-owner', class extends HTMLElement {
                    disconnectedCallback() { __setContentDisconnected++; }
                });
                const owner = document.createElement('set-content-owner');
                document.body.append(owner);
                owner.isConnected;
            "#,
        )
        .await,
        json!(true)
    );

    ctx.process_async(json!({
        "id": 68,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": { "frameId": "TID-1", "html": "<main>replacement</main>" }
    }))
    .await;
    ctx.expect_result(68, json!({}), Some("SID-1"));
    assert_eq!(
        evaluate_by_value(&mut ctx, 69, None, "__setContentDisconnected").await,
        json!(1)
    );
}

// Ported from WPT selection/Document-open.html and Blink's live Range
// adjustment behavior. Document::SetContent keeps the Selection and Range
// objects, but removing the old document element retargets both endpoints to
// the live Document before the replacement parser installs its new tree.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_preserves_selection_identity_and_retargets_removed_range() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body></body>").await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            70,
            None,
            r#"
                const input = document.body.appendChild(document.createElement('input'));
                input.focus();
                const text = document.body.appendChild(document.createTextNode('range'));
                const range = document.createRange();
                range.selectNodeContents(text);
                const selection = getSelection();
                selection.removeAllRanges();
                selection.addRange(range);
                globalThis.__setContentOldInput = input;
                globalThis.__setContentOldRange = range;
                globalThis.__setContentOldSelection = selection;
                'ready';
            "#,
        )
        .await,
        json!("ready")
    );

    ctx.process_async(json!({
        "id": 71,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": { "frameId": "TID-1", "html": "<main>replacement</main>" }
    }))
    .await;
    ctx.expect_result(71, json!({}), Some("SID-1"));

    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            72,
            None,
            r#"
                ({
                  activeElement: document.activeElement?.nodeName,
                  oldInputConnected: __setContentOldInput.isConnected,
                  sameSelection: getSelection() === __setContentOldSelection,
                  rangeCount: getSelection().rangeCount,
                  sameRange: getSelection().getRangeAt(0) === __setContentOldRange,
                  start: `${__setContentOldRange.startContainer.nodeName}:${__setContentOldRange.startOffset}`,
                  end: `${__setContentOldRange.endContainer.nodeName}:${__setContentOldRange.endOffset}`,
                })
            "#,
        )
        .await,
        json!({
            "activeElement": "BODY",
            "oldInputConnected": false,
            "sameSelection": true,
            "rangeCount": 1,
            "sameRange": true,
            "start": "#document:0",
            "end": "#document:0",
        })
    );
}

// Verified against Chromium's InspectorPageAgent::setDocumentContent path.
// Template contents live in a separate inert tree scope, so a subtree observer
// on Document must not see parser insertions below template.content.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_keeps_template_contents_outside_document_observer_subtree() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            73,
            None,
            r#"
                globalThis.__setContentTemplateRecords = [];
                new MutationObserver(records => {
                  __setContentTemplateRecords.push(...records.map(record => ({
                    target: record.target.nodeName,
                    added: Array.from(record.addedNodes, node => node.nodeName),
                  })));
                }).observe(document, { childList: true, subtree: true });
                'ready';
            "#,
        )
        .await,
        json!("ready")
    );

    ctx.process_async(json!({
        "id": 74,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": "<template id=tpl><style>#outside{color:red}</style><span id=inside>inside</span></template><main id=outside>outside</main>"
        }
    }))
    .await;
    ctx.expect_result(74, json!({}), Some("SID-1"));

    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            75,
            None,
            r#"
                (() => {
                  const template = document.querySelector('#tpl');
                  const inside = template.content.querySelector('#inside');
                  return {
                    observedTemplateContent: __setContentTemplateRecords.some(record =>
                      record.target === '#document-fragment' || record.added.includes('SPAN')
                    ),
                    insideFoundInDocument: document.querySelector('#inside') !== null,
                    insideFoundInTemplate: inside?.textContent,
                    insideConnected: inside?.isConnected,
                    rootIsTemplateContent: inside?.getRootNode() === template.content,
                    styleSheets: document.styleSheets.length,
                    outsideColor: getComputedStyle(document.querySelector('#outside')).color,
                  };
                })()
            "#,
        )
        .await,
        json!({
            "observedTemplateContent": false,
            "insideFoundInDocument": false,
            "insideFoundInTemplate": "inside",
            "insideConnected": false,
            "rootIsTemplateContent": true,
            "styleSheets": 0,
            "outsideColor": "rgb(0, 0, 0)",
        })
    );
}

// Blink processes the replacement tree's first <base> through the ordinary
// document base-element owner. Repeated SetContent calls must replace that
// state rather than retaining an URL from a detached predecessor tree.
#[tokio::test(flavor = "multi_thread")]
async fn repeated_set_document_content_replaces_document_base_url_state() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;

    for (id, href, expected) in [
        (
            76,
            "https://first.example/assets/",
            "https://first.example/assets/next.js",
        ),
        (
            78,
            "https://second.example/static/",
            "https://second.example/static/next.js",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.setDocumentContent",
            "sessionId": "SID-1",
            "params": {
                "frameId": "TID-1",
                "html": format!("<base href='{href}'><a id=relative href='next.js'>next</a>")
            }
        }))
        .await;
        ctx.expect_result(id, json!({}), Some("SID-1"));
        assert_eq!(
            evaluate_by_value(
                &mut ctx,
                id + 1,
                None,
                "({ baseURI: document.baseURI, resolved: document.querySelector('#relative').href })",
            )
            .await,
            json!({ "baseURI": href, "resolved": expected })
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_updates_compat_mode_from_the_new_doctype() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(
        &mut ctx,
        "data:text/html,<!doctype html><body>standards</body>",
    )
    .await;

    assert_eq!(
        evaluate_by_value(&mut ctx, 77, None, "document.compatMode").await,
        json!("CSS1Compat")
    );

    for (id, html, expected) in [
        (78, "<main>quirks</main>", "BackCompat"),
        (
            80,
            "<!doctype html><main>standards again</main>",
            "CSS1Compat",
        ),
    ] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.setDocumentContent",
            "sessionId": "SID-1",
            "params": { "frameId": "TID-1", "html": html }
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
        assert_eq!(
            evaluate_by_value(&mut ctx, id + 1, None, "document.compatMode").await,
            json!(expected)
        );
    }
}

// Verified against local Chromium's Page.setDocumentContent path. A
// parser-created custom element whose definition already exists is constructed
// before token attributes are installed, receives attributeChangedCallback,
// and connects before the command completes. The root replacement path must
// therefore use the live-document parser sink rather than importing a fully
// built foreign tree.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_constructs_predefined_custom_elements_at_parser_token_time() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body></body>").await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            80,
            None,
            r#"
                globalThis.__setContentCustomElementLog = [];
                customElements.define('x-set-content-probe', class extends HTMLElement {
                  static observedAttributes = ['data-x'];
                  constructor() {
                    super();
                    __setContentCustomElementLog.push(`constructor:${this.getAttribute('data-x')}`);
                  }
                  attributeChangedCallback(_name, oldValue, newValue) {
                    __setContentCustomElementLog.push(`attribute:${oldValue}>${newValue}`);
                  }
                  connectedCallback() {
                    __setContentCustomElementLog.push(`connected:${this.getAttribute('data-x')}`);
                  }
                  disconnectedCallback() {
                    __setContentCustomElementLog.push('disconnected');
                  }
                });
                'ready';
            "#,
        )
        .await,
        json!("ready")
    );

    for (id, value) in [(81, "one"), (82, "two")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.setDocumentContent",
            "sessionId": "SID-1",
            "params": {
                "frameId": "TID-1",
                "html": format!("<x-set-content-probe data-x='{value}'></x-set-content-probe>")
            }
        }))
        .await;
        ctx.expect_result(id, json!({}), Some("SID-1"));
    }

    assert_eq!(
        evaluate_by_value(&mut ctx, 83, None, "__setContentCustomElementLog").await,
        json!([
            "constructor:null",
            "attribute:null>one",
            "connected:one",
            "disconnected",
            "constructor:null",
            "attribute:null>two",
            "connected:two",
        ])
    );
}

// Ported from the document.write WPT parser-insertion cases and verified
// against Chromium's SetContent implementation. A parser-blocking inline
// script writes at its active insertion point before the network/parser tail
// resumes.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_preserves_reentrant_document_write_insertion_order() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;

    ctx.process_async(json!({
        "id": 84,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": concat!(
                "<main id=before>before</main>",
                "<script id=writer>",
                "window.__setContentCurrentScript = document.currentScript.id;",
                "document.write('<aside id=written>inserted</aside>');",
                "</script>",
                "<footer id=after>after</footer>"
            )
        }
    }))
    .await;
    ctx.expect_result(84, json!({}), Some("SID-1"));

    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            85,
            None,
            r#"({
                ids: Array.from(document.body.children, element => element.id),
                currentScript: __setContentCurrentScript,
                writtenText: document.querySelector('#written').textContent,
            })"#,
        )
        .await,
        json!({
            "ids": ["before", "writer", "written", "after"],
            "currentScript": "writer",
            "writtenText": "inserted",
        })
    );
}

// Ported from WPT custom-elements/throw-on-dynamic-markup-insertion-counter-*
// parser cases. Token-time construction must hold the counter while the
// constructor runs, without aborting the surrounding SetContent parse.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_blocks_dynamic_markup_in_parser_custom_element_constructor() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body></body>").await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            86,
            None,
            r#"
                globalThis.__dynamicMarkupConstructorLog = [];
                customElements.define('x-dynamic-markup-probe', class extends HTMLElement {
                  constructor() {
                    super();
                    for (const [name, callback] of [
                      ['open', () => document.open()],
                      ['write', () => document.write('<p>forbidden</p>')],
                    ]) {
                      try {
                        callback();
                        __dynamicMarkupConstructorLog.push(`${name}:no-throw`);
                      } catch (error) {
                        __dynamicMarkupConstructorLog.push(`${name}:${error.name}`);
                      }
                    }
                  }
                  connectedCallback() {
                    __dynamicMarkupConstructorLog.push('connected');
                  }
                });
                'ready';
            "#,
        )
        .await,
        json!("ready")
    );

    ctx.process_async(json!({
        "id": 87,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": "<x-dynamic-markup-probe></x-dynamic-markup-probe><main id=after>after</main>"
        }
    }))
    .await;
    ctx.expect_result(87, json!({}), Some("SID-1"));

    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            88,
            None,
            "({ log: __dynamicMarkupConstructorLog, after: document.querySelector('#after').textContent })",
        )
        .await,
        json!({
            "log": ["open:InvalidStateError", "write:InvalidStateError", "connected"],
            "after": "after",
        })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_set_document_content_resets_dirty_form_control_state() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;

    let markup = concat!(
        "<input id=text value=initial>",
        "<textarea id=textarea>initial</textarea>",
        "<input id=checkbox type=checkbox checked>",
        "<select id=select><option>first</option><option selected>second</option></select>"
    );
    for id in [87, 89] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.setDocumentContent",
            "sessionId": "SID-1",
            "params": { "frameId": "TID-1", "html": markup }
        }))
        .await;
        assert_eq!(take_response_by_id(&mut ctx, id)["result"], json!({}));
        if id == 87 {
            assert_eq!(
                evaluate_by_value(
                    &mut ctx,
                    88,
                    None,
                    r#"(() => {
                        text.value = 'dirty';
                        textarea.value = 'dirty';
                        checkbox.checked = false;
                        select.value = 'first';
                        return [text.value, textarea.value, checkbox.checked, select.value];
                    })()"#,
                )
                .await,
                json!(["dirty", "dirty", false, "first"])
            );
        }
    }

    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            90,
            None,
            "[text.value, textarea.value, checkbox.checked, select.value]",
        )
        .await,
        json!(["initial", "initial", true, "second"]),
        "replacement must not restore dirty state from controls in the removed tree",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_set_document_content_replaces_declarative_shadow_tree_scope() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;

    ctx.process_async(json!({
        "id": 91,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": concat!(
                "<div id=old-host>",
                "<template shadowrootmode=open>",
                "<style>#inside { color: rgb(1, 2, 3) }</style>",
                "<span id=inside>shadow</span>",
                "</template>",
                "<span id=light>light</span>",
                "</div>"
            )
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 91)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            92,
            None,
            r#"(() => {
                globalThis.__oldSetContentHost = document.getElementById('old-host');
                globalThis.__oldSetContentRoot = __oldSetContentHost.shadowRoot;
                const inside = __oldSetContentRoot.getElementById('inside');
                return {
                    hasRoot: !!__oldSetContentRoot,
                    shadowText: inside.textContent,
                    shadowScope: inside.getRootNode() === __oldSetContentRoot,
                    hostBackref: __oldSetContentRoot.host === __oldSetContentHost,
                    hiddenFromDocumentScope: document.getElementById('inside') === null,
                    lightChildren: __oldSetContentHost.children.length,
                    shadowColor: getComputedStyle(inside).color,
                };
            })()"#,
        )
        .await,
        json!({
            "hasRoot": true,
            "shadowText": "shadow",
            "shadowScope": true,
            "hostBackref": true,
            "hiddenFromDocumentScope": true,
            "lightChildren": 1,
            "shadowColor": "rgb(1, 2, 3)",
        })
    );

    ctx.process_async(json!({
        "id": 93,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": "<div id=new-host><template shadowrootmode=open><span id=inside>new</span></template></div>"
        }
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 93)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            94,
            None,
            r#"({
                oldHostConnected: __oldSetContentHost.isConnected,
                oldRootStillOwnedByOldHost: __oldSetContentRoot.host === __oldSetContentHost,
                newText: document.getElementById('new-host').shadowRoot.getElementById('inside').textContent,
                documentStyleSheets: document.styleSheets.length,
            })"#,
        )
        .await,
        json!({
            "oldHostConnected": false,
            "oldRootStillOwnedByOldHost": true,
            "newText": "new",
            "documentStyleSheets": 0,
        }),
        "the old shadow TreeScope and its stylesheet candidates must stay detached",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_replaces_root_document_without_navigation_or_realm_churn() {
    let mut ctx = TestContext::new();
    let url = "data:text/html,<body><main id=old-root>old</main></body>";
    install_document_content_test_page(&mut ctx, url).await;
    enable_document_content_observers(&mut ctx).await;

    let before = frame_tree(&mut ctx, 20).await["frame"].clone();
    assert_eq!(before["id"], json!("TID-1"));
    let setup = evaluate_by_value(
        &mut ctx,
        21,
        None,
        r#"
            globalThis.__setContentOldDocument = document;
            globalThis.__setContentOldNode = document.querySelector('#old-root');
            globalThis.__setContentRealmMarker = 73;
            globalThis.__setContentPublicCalls = [];
            document.open = () => __setContentPublicCalls.push('open');
            document.write = () => __setContentPublicCalls.push('write');
            document.close = () => __setContentPublicCalls.push('close');
            'ready';
        "#,
    )
    .await;
    assert_eq!(setup, json!("ready"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 22,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": before["id"],
            "html": "<main id=updated-root>updated</main><script>window.__setContentInlineRuns = (window.__setContentInlineRuns || 0) + 1; console.log('set-content-root-classic');</script>",
        },
    }))
    .await;
    wait_until_scheduler_message(
        &mut ctx,
        "root setDocumentContent load lifecycle",
        |message| message["method"] == json!("Page.loadEventFired"),
    )
    .await;
    let messages = ctx.take_all();
    let response_index = message_index(&messages, "Page.setDocumentContent response", |message| {
        message["id"] == json!(22) && message["result"] == json!({})
    });
    let document_opened_index = message_index(&messages, "Page.documentOpened", |message| {
        message["method"] == json!("Page.documentOpened")
            && message["params"]["frame"]["id"] == before["id"]
    });
    let lifecycle_init_index = message_index(&messages, "lifecycle init", |message| {
        message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["name"] == json!("init")
            && message["params"]["frameId"] == before["id"]
    });
    let classic_script_index = console_message_index(&messages, "set-content-root-classic");
    let document_updated_index = message_index(&messages, "DOM.documentUpdated", |message| {
        message["method"] == json!("DOM.documentUpdated")
    });
    let dom_content_loaded_index =
        message_index(&messages, "Page.domContentEventFired", |message| {
            message["method"] == json!("Page.domContentEventFired")
        });
    let load_index = message_index(&messages, "Page.loadEventFired", |message| {
        message["method"] == json!("Page.loadEventFired")
    });
    assert!(
        document_opened_index < lifecycle_init_index
            && lifecycle_init_index < classic_script_index
            && classic_script_index < document_updated_index
            && document_updated_index < dom_content_loaded_index
            && dom_content_loaded_index < load_index
            && classic_script_index < response_index,
        "setDocumentContent should preserve Chromium's document-open, classic-script, DOM, and lifecycle order without overconstraining load relative to the command response: {messages:?}"
    );
    let opened_frame = &messages[document_opened_index]["params"]["frame"];
    assert_eq!(opened_frame["loaderId"], before["loaderId"]);
    assert_eq!(opened_frame["url"], before["url"]);
    assert!(opened_frame["parentId"].is_null());
    for forbidden in [
        "Page.frameStartedNavigating",
        "Page.frameStartedLoading",
        "Page.frameNavigated",
        "Page.frameStoppedLoading",
        "Runtime.executionContextsCleared",
        "Runtime.executionContextDestroyed",
        "Runtime.executionContextCreated",
    ] {
        assert!(
            messages
                .iter()
                .all(|message| message["method"] != json!(forbidden)),
            "setDocumentContent must not emit {forbidden}: {messages:?}"
        );
    }

    let after = frame_tree(&mut ctx, 23).await["frame"].clone();
    assert_eq!(after["id"], before["id"]);
    assert_eq!(after["loaderId"], before["loaderId"]);
    assert_eq!(after["url"], before["url"]);
    let state = evaluate_by_value(
        &mut ctx,
        24,
        None,
        r#"({
            sameDocument: document === __setContentOldDocument,
            oldNodeConnected: __setContentOldNode.isConnected,
            text: document.querySelector('#updated-root').textContent,
            inlineRuns: __setContentInlineRuns,
            publicCalls: __setContentPublicCalls,
            realmMarker: __setContentRealmMarker,
            readyState: document.readyState,
        })"#,
    )
    .await;
    assert_eq!(
        state,
        json!({
            "sameDocument": true,
            "oldNodeConnected": false,
            "text": "updated",
            "inlineRuns": 1,
            "publicCalls": [],
            "realmMarker": 73,
            "readyState": "complete",
        })
    );
}

/// Chromium's `Page.setDocumentContent` command runs parser-blocking classic
/// scripts synchronously, returns before parser-owned modules, then publishes
/// the root DOM refresh and document lifecycle milestones.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_orders_classic_module_response_and_lifecycle() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    let before = frame_tree(&mut ctx, 120).await["frame"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 121,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": before["id"],
            "html": concat!(
                "<script>window.__setContentScriptOrder = ['classic']; console.log('set-content-ordered-classic');</script>",
                "<script type=module>window.__setContentScriptOrder.push('module'); console.log('set-content-ordered-module');</script>",
                "<main id=ordered-tail>tail</main>",
            ),
        },
    }))
    .await;
    wait_until_scheduler_message(
        &mut ctx,
        "setDocumentContent parser module console",
        |message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["args"][0]["value"] == json!("set-content-ordered-module")
        },
    )
    .await;
    wait_until_scheduler_message(
        &mut ctx,
        "setDocumentContent classic/module load lifecycle",
        |message| message["method"] == json!("Page.loadEventFired"),
    )
    .await;
    let messages = ctx.take_all();
    let init = message_index(&messages, "setDocumentContent lifecycle init", |message| {
        message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["frameId"] == before["id"]
            && message["params"]["name"] == json!("init")
    });
    let classic = console_message_index(&messages, "set-content-ordered-classic");
    let response = message_index(&messages, "Page.setDocumentContent response", |message| {
        message["id"] == json!(121) && message["result"] == json!({})
    });
    let module = console_message_index(&messages, "set-content-ordered-module");
    let document_updated = message_index(&messages, "DOM.documentUpdated", |message| {
        message["method"] == json!("DOM.documentUpdated")
    });
    let dom_content_loaded = message_index(&messages, "Page.domContentEventFired", |message| {
        message["method"] == json!("Page.domContentEventFired")
    });
    let load = message_index(&messages, "Page.loadEventFired", |message| {
        message["method"] == json!("Page.loadEventFired")
    });
    assert!(
        init < classic
            && classic < response
            && response < module
            && module < document_updated
            && document_updated < dom_content_loaded
            && dom_content_loaded < load,
        "setDocumentContent event order should match Chromium: {messages:?}"
    );
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            122,
            None,
            "[window.__setContentScriptOrder.join(','), document.querySelector('#ordered-tail').textContent, document.readyState].join('|')",
        )
        .await,
        json!("classic,module|tail|complete")
    );
}

/// Ports the top-level-await lifecycle boundary exercised by Chromium/WPT:
/// starting a parser-owned module precedes DOMContentLoaded, while completion
/// of its evaluation promise does not hold DOMContentLoaded or load open.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_module_top_level_await_does_not_block_lifecycle() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    let before = frame_tree(&mut ctx, 123).await["frame"].clone();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 124,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": before["id"],
            "html": concat!(
                "<script type=module>",
                "window.__setContentTlaTrace = ['start']; console.log('set-content-tla-start');",
                "await new Promise(resolve => { window.__releaseSetContentTla = resolve; });",
                "window.__setContentTlaTrace.push('end'); console.log('set-content-tla-end');",
                "</script>",
                "<main id=tla-tail>tail</main>",
            ),
        },
    }))
    .await;
    wait_until_scheduler_message(&mut ctx, "setDocumentContent TLA module start", |message| {
        message["method"] == json!("Runtime.consoleAPICalled")
            && message["params"]["args"][0]["value"] == json!("set-content-tla-start")
    })
    .await;
    wait_until_scheduler_message(
        &mut ctx,
        "setDocumentContent TLA load lifecycle",
        |message| message["method"] == json!("Page.loadEventFired"),
    )
    .await;

    let messages = ctx.take_all();
    let response = message_index(&messages, "Page.setDocumentContent response", |message| {
        message["id"] == json!(124) && message["result"] == json!({})
    });
    let module_start = console_message_index(&messages, "set-content-tla-start");
    let document_updated = message_index(&messages, "DOM.documentUpdated", |message| {
        message["method"] == json!("DOM.documentUpdated")
    });
    let dom_content_loaded = message_index(&messages, "Page.domContentEventFired", |message| {
        message["method"] == json!("Page.domContentEventFired")
    });
    let load = message_index(&messages, "Page.loadEventFired", |message| {
        message["method"] == json!("Page.loadEventFired")
    });
    assert!(
        response < module_start
            && module_start < document_updated
            && document_updated < dom_content_loaded
            && dom_content_loaded < load,
        "TLA module start, but not its completion, should gate lifecycle: {messages:?}"
    );
    assert!(
        messages.iter().all(|message| {
            message["method"] != json!("Runtime.consoleAPICalled")
                || message["params"]["args"][0]["value"] != json!("set-content-tla-end")
        }),
        "the suspended TLA continuation must not run before load: {messages:?}"
    );
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            125,
            None,
            "[document.readyState, window.__setContentTlaTrace.join(','), typeof window.__releaseSetContentTla, document.querySelector('#tla-tail').textContent].join('|')",
        )
        .await,
        json!("complete|start|function|tail")
    );
    ctx.sent.clear();

    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            126,
            None,
            "window.__releaseSetContentTla(); 'released'",
        )
        .await,
        json!("released")
    );
    wait_until_scheduler_message(&mut ctx, "setDocumentContent TLA continuation", |message| {
        message["method"] == json!("Runtime.consoleAPICalled")
            && message["params"]["args"][0]["value"] == json!("set-content-tla-end")
    })
    .await;
    assert_eq!(
        evaluate_by_value(&mut ctx, 127, None, "window.__setContentTlaTrace.join(',')",).await,
        json!("start,end")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn root_set_document_content_unloads_descendant_frame_before_clearing_parent_listeners() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(
        &mut ctx,
        "data:text/html,<body><iframe srcdoc='<body>child</body>'></iframe></body>",
    )
    .await;
    let pending = ctx
        .conn
        .start_child_frame_lifecycle_work_for_session_owner(
            Some("SID-1"),
            std::time::Duration::from_secs(2),
        )
        .expect("loaded page should expose child-frame lifecycle work");
    let completed = pending
        .wait()
        .await
        .expect("srcdoc child lifecycle should complete");
    assert!(
        ctx.conn
            .complete_child_frame_lifecycle_work_for_session_owner(completed)
            .expect("srcdoc child lifecycle completion should apply")
    );

    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            25,
            None,
            r#"
                globalThis.__documentOpenFrameEvents = [];
                addEventListener('beforeunload', () => __documentOpenFrameEvents.push('root-beforeunload'));
                addEventListener('pagehide', () => __documentOpenFrameEvents.push('root-pagehide'));
                addEventListener('unload', () => __documentOpenFrameEvents.push('root-unload'));
                addEventListener('child-unload-probe', () => __documentOpenFrameEvents.push('root-probe'));
                const frame = document.querySelector('iframe');
                frame.contentWindow.addEventListener('beforeunload', () => __documentOpenFrameEvents.push('child-beforeunload'));
                frame.contentWindow.addEventListener('pagehide', () => __documentOpenFrameEvents.push('child-pagehide'));
                frame.contentDocument.addEventListener('visibilitychange', () => __documentOpenFrameEvents.push('child-visibilitychange'));
                frame.contentWindow.addEventListener('unload', () => {
                    __documentOpenFrameEvents.push('child-unload');
                    dispatchEvent(new Event('child-unload-probe'));
                });
                'ready';
            "#,
        )
        .await,
        json!("ready")
    );
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 26,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": "<main id=replacement>replacement</main>"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 26);
    assert_eq!(response["result"], json!({}));

    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            27,
            None,
            "JSON.stringify({ events: __documentOpenFrameEvents, frameCount: document.querySelectorAll('iframe').length })",
        )
        .await,
        json!(
            r#"{"events":["child-pagehide","child-visibilitychange","child-unload","root-probe"],"frameCount":0}"#
        ),
        "Document::open unloads child frames, but does not unload the document being opened"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_preserves_child_frame_loader_document_and_context() {
    let mut ctx = TestContext::new();
    let url = "data:text/html,<body><iframe name=child-frame srcdoc='<main id=old-child>old</main>'></iframe></body>";
    install_document_content_test_page(&mut ctx, url).await;
    enable_document_content_observers(&mut ctx).await;

    let before_tree = frame_tree(&mut ctx, 30).await;
    let before = before_tree["childFrames"][0]["frame"].clone();
    let child_frame_id = before["id"].as_str().expect("child frame id");
    let child_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
                && message["params"]["context"]["auxData"]["frameId"] == before["id"]
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("child default execution context");
    let setup = evaluate_by_value(
        &mut ctx,
        31,
        Some(child_context_id),
        r#"
            globalThis.__setContentOldDocument = document;
            globalThis.__setContentOldNode = document.querySelector('#old-child');
            globalThis.__setContentRealmMarker = 91;
            globalThis.__setContentPublicCalls = [];
            globalThis.__setContentListenerRuns = { node: 0, document: 0, window: 0, handler: 0 };
            __setContentOldNode.addEventListener('click', () => __setContentListenerRuns.node++);
            __setContentOldNode.onclick = () => __setContentListenerRuns.handler++;
            document.addEventListener('replacement-probe', () => __setContentListenerRuns.document++);
            window.addEventListener('replacement-probe', () => __setContentListenerRuns.window++);
            document.open = () => __setContentPublicCalls.push('open');
            document.write = () => __setContentPublicCalls.push('write');
            document.close = () => __setContentPublicCalls.push('close');
            'ready';
        "#,
    )
    .await;
    assert_eq!(setup, json!("ready"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 32,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": child_frame_id,
            "html": concat!(
                "<main id=updated-child>updated child</main>",
                "<script>window.__setContentInlineRuns = (window.__setContentInlineRuns || 0) + 1; console.log('set-content-child-classic');</script>",
                "<script type=module>window.__setContentModuleRuns = (window.__setContentModuleRuns || 0) + 1; console.log('set-content-child-module');</script>",
            ),
        },
    }))
    .await;
    let child_frame_id_for_load = child_frame_id.to_owned();
    wait_until_scheduler_message(
        &mut ctx,
        "child setDocumentContent load lifecycle",
        move |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!(child_frame_id_for_load)
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    let messages = ctx.take_all();
    let response_index = message_index(&messages, "Page.setDocumentContent response", |message| {
        message["id"] == json!(32) && message["result"] == json!({})
    });
    let opened_index = message_index(&messages, "child Page.documentOpened", |message| {
        message["method"] == json!("Page.documentOpened")
            && message["params"]["frame"]["id"] == before["id"]
    });
    let opened_frame = &messages[opened_index]["params"]["frame"];
    assert_eq!(opened_frame["parentId"], before_tree["frame"]["id"]);
    assert_eq!(opened_frame["loaderId"], before["loaderId"]);
    assert_eq!(opened_frame["url"], before["url"]);
    assert_eq!(opened_frame["name"], before["name"]);
    let lifecycle_indexes = ["init", "DOMContentLoaded", "load"].map(|lifecycle_name| {
        message_index(&messages, lifecycle_name, |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == before["id"]
                && message["params"]["loaderId"] == before["loaderId"]
                && message["params"]["name"] == json!(lifecycle_name)
        })
    });
    let classic_script_index = console_message_index(&messages, "set-content-child-classic");
    let module_script_index = console_message_index(&messages, "set-content-child-module");
    assert!(
        opened_index < lifecycle_indexes[0]
            && lifecycle_indexes[0] < classic_script_index
            && classic_script_index < response_index
            && response_index < module_script_index
            && module_script_index < lifecycle_indexes[1]
            && lifecycle_indexes[1] < lifecycle_indexes[2],
        "child setDocumentContent should return after synchronous scripts and before DCL/load: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .all(|message| message["method"] != json!("DOM.documentUpdated")),
        "Chromium does not invalidate the root DOM agent for child replacement: {messages:?}"
    );
    for forbidden in [
        "Page.frameStartedNavigating",
        "Page.frameStartedLoading",
        "Page.frameNavigated",
        "Page.frameStoppedLoading",
        "Runtime.executionContextsCleared",
        "Runtime.executionContextDestroyed",
        "Runtime.executionContextCreated",
    ] {
        assert!(
            messages
                .iter()
                .all(|message| message["method"] != json!(forbidden)),
            "child setDocumentContent must not emit {forbidden}: {messages:?}"
        );
    }

    let after_tree = frame_tree(&mut ctx, 33).await;
    let after = &after_tree["childFrames"][0]["frame"];
    assert_eq!(after["id"], before["id"]);
    assert_eq!(after["loaderId"], before["loaderId"]);
    assert_eq!(after["url"], before["url"]);
    let state = evaluate_by_value(
        &mut ctx,
        34,
        Some(child_context_id),
        r#"(() => {
            __setContentOldNode.dispatchEvent(new Event('click'));
            document.dispatchEvent(new Event('replacement-probe', { bubbles: true }));
            window.dispatchEvent(new Event('replacement-probe'));
            return {
                sameDocument: document === __setContentOldDocument,
                oldNodeConnected: __setContentOldNode.isConnected,
                text: document.querySelector('#updated-child').textContent,
                inlineRuns: __setContentInlineRuns,
                moduleRuns: __setContentModuleRuns,
                publicCalls: __setContentPublicCalls,
                listenerRuns: __setContentListenerRuns,
                realmMarker: __setContentRealmMarker,
                readyState: document.readyState,
            };
        })()"#,
    )
    .await;
    assert_eq!(
        state,
        json!({
            "sameDocument": true,
            "oldNodeConnected": false,
            "text": "updated child",
            "inlineRuns": 1,
            "moduleRuns": 1,
            "publicCalls": [],
            "listenerRuns": { "node": 0, "document": 0, "window": 0, "handler": 0 },
            "realmMarker": 91,
            "readyState": "complete",
        })
    );
}

// Calibrated against Chromium's Page.setDocumentContent behavior: a module graph
// fetch in one child frame must not serialize module execution in a sibling
// frame. Module-task readiness is independent for each child document.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_sibling_module_runs_while_other_child_graph_is_blocked() {
    let slow_module_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_slow_module = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_requested = slow_module_requested.clone();
    let handler_release = release_slow_module.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/root",
                axum::routing::get(|| async {
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
                        concat!(
                            "<!doctype html><body>",
                            "<iframe name=blocked srcdoc='<main>blocked old</main>'></iframe>",
                            "<iframe name=ready srcdoc='<main>ready old</main>'></iframe>",
                            "</body>",
                        ),
                    )
                }),
            )
            .route(
                "/slow-child-module.js",
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
                            "console.log('slow-child-dependency-ran');",
                        )
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, &format!("http://{addr}/root")).await;
    enable_document_content_observers(&mut ctx).await;
    let tree = frame_tree(&mut ctx, 170).await;
    let child_frames = tree["childFrames"]
        .as_array()
        .expect("two child frames in frame tree");
    let blocked_frame_id = child_frames
        .iter()
        .find(|child| child["frame"]["name"] == json!("blocked"))
        .and_then(|child| child["frame"]["id"].as_str())
        .expect("blocked child frame id")
        .to_owned();
    let ready_frame_id = child_frames
        .iter()
        .find(|child| child["frame"]["name"] == json!("ready"))
        .and_then(|child| child["frame"]["id"].as_str())
        .expect("ready child frame id")
        .to_owned();
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 171,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": blocked_frame_id,
            "html": format!(
                concat!(
                    "<script type=module>",
                    "import 'http://{addr}/slow-child-module.js';",
                    "globalThis.__blockedChildRootModuleRan = true;",
                    "console.log('blocked-child-root-module-ran');",
                    "</script>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 171)["result"], json!({}));
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        slow_module_requested.notified(),
    )
    .await
    .expect("first child module dependency should start loading");

    ctx.process_async(json!({
        "id": 172,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": ready_frame_id,
            "html": concat!(
                "<script type=module>",
                "globalThis.__readySiblingModuleRan = true;",
                "console.log('ready-sibling-module-ran');",
                "</script>"
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 172)["result"], json!({}));

    let sibling_result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        wait_until_scheduler_message(
            &mut ctx,
            "ready sibling module execution while the other graph is blocked",
            |message| {
                message["method"] == json!("Runtime.consoleAPICalled")
                    && message["params"]["args"][0]["value"] == json!("ready-sibling-module-ran")
            },
        ),
    )
    .await;
    if sibling_result.is_ok() {
        assert!(ctx.sent.iter().all(|message| {
            !(message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["args"][0]["value"] == json!("blocked-child-root-module-ran"))
        }));
    }
    release_slow_module.notify_one();
    sibling_result
        .expect("a blocked module graph in one child must not block a ready sibling child module");

    wait_until_scheduler_message(
        &mut ctx,
        "blocked child module execution after dependency release",
        |message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["args"][0]["value"] == json!("blocked-child-root-module-ran")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            173,
            None,
            r#"(() => {
                const blocked = document.querySelector('iframe[name=blocked]');
                const ready = document.querySelector('iframe[name=ready]');
                return {
                    blocked: blocked.contentWindow.__blockedChildRootModuleRan === true,
                    ready: ready.contentWindow.__readySiblingModuleRan === true,
                };
            })()"#,
        )
        .await,
        json!({ "blocked": true, "ready": true }),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_returns_before_slow_root_parser_script_load() {
    let release_script = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_release = release_script.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/slow.js",
            axum::routing::get(move || {
                let release = handler_release.clone();
                async move {
                    release.notified().await;
                    (
                        [(
                            axum::http::header::CONTENT_TYPE.as_str(),
                            "application/javascript",
                        )],
                        "window.__slowRootSetContentScriptRan = true;",
                    )
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    let before = frame_tree(&mut ctx, 35).await["frame"].clone();
    ctx.sent.clear();

    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        ctx.process_async(json!({
            "id": 36,
            "method": "Page.setDocumentContent",
            "sessionId": "SID-1",
            "params": {
                "frameId": before["id"],
                "html": format!(
                    "<script src='http://{addr}/slow.js'></script><main id=after-slow-root>new</main>"
                ),
            },
        })),
    )
    .await
    .expect("setDocumentContent must not wait for the root external parser script");

    let immediate = ctx.take_all();
    let opened_index = message_index(&immediate, "root Page.documentOpened", |message| {
        message["method"] == json!("Page.documentOpened")
            && message["params"]["frame"]["id"] == before["id"]
    });
    let init_index = message_index(&immediate, "root lifecycle init", |message| {
        message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["frameId"] == before["id"]
            && message["params"]["name"] == json!("init")
    });
    let response_index = message_index(&immediate, "Page.setDocumentContent response", |message| {
        message["id"] == json!(36) && message["result"] == json!({})
    });
    assert!(
        opened_index < init_index && init_index < response_index,
        "root document-open events should precede the nonblocking response: {immediate:?}"
    );
    assert!(
        immediate.iter().all(|message| {
            !(matches!(
                message["method"].as_str(),
                Some("DOM.documentUpdated" | "Page.domContentEventFired" | "Page.loadEventFired")
            ) || (message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == before["id"]
                && matches!(
                    message["params"]["name"].as_str(),
                    Some("DOMContentLoaded" | "load")
                )))
        }),
        "DOM observer refresh and DCL/load must remain pending behind the parser script: {immediate:?}"
    );

    release_script.notify_one();
    wait_until_scheduler_message(&mut ctx, "root document-open load lifecycle", |message| {
        message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["frameId"] == before["id"]
            && message["params"]["name"] == json!("load")
    })
    .await;
    let completed = ctx.take_all();
    let document_updated_index = message_index(&completed, "DOM.documentUpdated", |message| {
        message["method"] == json!("DOM.documentUpdated")
    });
    let dom_content_loaded_index = message_index(&completed, "DOMContentLoaded", |message| {
        message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["frameId"] == before["id"]
            && message["params"]["name"] == json!("DOMContentLoaded")
    });
    let load_index = message_index(&completed, "load", |message| {
        message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["frameId"] == before["id"]
            && message["params"]["name"] == json!("load")
    });
    assert!(
        document_updated_index < dom_content_loaded_index && dom_content_loaded_index < load_index,
        "DOM observers should refresh after parser completion and before DCL/load: {completed:?}"
    );
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            37,
            None,
            r#"({
                scriptRan: globalThis.__slowRootSetContentScriptRan === true,
                parserTail: document.querySelector('#after-slow-root')?.textContent,
            })"#,
        )
        .await,
        json!({ "scriptRan": true, "parserTail": "new" }),
        "the source-ready owner turn must execute the script and resume its parser tail before load",
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_resumes_parser_script_after_blocking_stylesheet() {
    let release_stylesheet = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_release = release_stylesheet.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/slow.css",
            axum::routing::get(move || {
                let release = handler_release.clone();
                async move {
                    release.notified().await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                        "#stylesheet-gated-probe { color: rgb(7, 8, 9); }",
                    )
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.sent.clear();

    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        ctx.process_async(json!({
            "id": 95,
            "method": "Page.setDocumentContent",
            "sessionId": "SID-1",
            "params": {
                "frameId": "TID-1",
                "html": format!(
                    concat!(
                        "<main id=stylesheet-gated-probe>probe</main>",
                        "<link rel=stylesheet href='http://{addr}/slow.css'>",
                        "<script>",
                        "globalThis.__setContentStylesheetGateColor = ",
                        "getComputedStyle(document.getElementById('stylesheet-gated-probe')).color;",
                        "</script>",
                        "<footer id=after-stylesheet-gate>tail</footer>"
                    ),
                    addr = addr,
                ),
            }
        })),
    )
    .await
    .expect("setDocumentContent must not wait for a blocking stylesheet");
    assert_eq!(take_response_by_id(&mut ctx, 95)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            96,
            None,
            "globalThis.__setContentStylesheetGateColor ?? 'pending'",
        )
        .await,
        json!("pending"),
        "the parser script must remain behind the unresolved stylesheet gate",
    );
    assert!(ctx.sent.iter().all(|message| {
        !(matches!(
            message["method"].as_str(),
            Some("Page.domContentEventFired" | "Page.loadEventFired")
        ) || (message["method"] == json!("Page.lifecycleEvent")
            && matches!(
                message["params"]["name"].as_str(),
                Some("DOMContentLoaded" | "load")
            )))
    }));

    release_stylesheet.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "stylesheet-gated document-open load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            97,
            None,
            r#"({
                scriptColor: __setContentStylesheetGateColor,
                finalColor: getComputedStyle(document.getElementById('stylesheet-gated-probe')).color,
                parserTail: document.getElementById('after-stylesheet-gate')?.textContent,
                sheets: document.styleSheets.length,
            })"#,
        )
        .await,
        json!({
            "scriptColor": "rgb(7, 8, 9)",
            "finalColor": "rgb(7, 8, 9)",
            "parserTail": "tail",
            "sheets": 1,
        }),
    );

    server.abort();
}

// Ported from Blink's HTMLParserScriptRunner resource-resume ordering.  The
// stylesheet's load task runs before the parser-blocking script, and resuming
// the script must restore the original insertion point for document.write().
#[tokio::test(flavor = "multi_thread")]
async fn stylesheet_gated_set_document_content_restores_parser_insertion_point() {
    let release_stylesheet = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_release = release_stylesheet.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/gate.css",
            axum::routing::get(move || {
                let release = handler_release.clone();
                async move {
                    release.notified().await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                        "#gated-written { color: rgb(11, 12, 13); }",
                    )
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 98,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": format!(
                concat!(
                    "<body>",
                    "<script>globalThis.__stylesheetGateOrder = [];</script>",
                    "<link id=gate rel=stylesheet href='http://{addr}/gate.css' ",
                    "onload=\"__stylesheetGateOrder.push('style-load')\">",
                    "<script id=gated-script>",
                    "__stylesheetGateOrder.push('script');",
                    "document.write('<aside id=gated-written>written</aside>');",
                    "</script>",
                    "<footer id=gated-tail>tail</footer>",
                    "</body>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 98)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            99,
            None,
            r#"({
                order: __stylesheetGateOrder,
                written: document.getElementById('gated-written'),
                tail: document.getElementById('gated-tail'),
            })"#,
        )
        .await,
        json!({ "order": [], "written": null, "tail": null }),
    );

    release_stylesheet.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "stylesheet-gated insertion-point load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            100,
            None,
            r#"({
                order: __stylesheetGateOrder,
                writtenColor: getComputedStyle(document.getElementById('gated-written')).color,
                bodyChildren: Array.from(document.body.children).map(node => node.id || node.tagName),
            })"#,
        )
        .await,
        json!({
            "order": ["style-load", "script"],
            "writtenColor": "rgb(11, 12, 13)",
            "bodyChildren": ["SCRIPT", "gate", "gated-script", "gated-written", "gated-tail"],
        }),
    );

    server.abort();
}

// A failed script-blocking stylesheet still releases Blink's pending parser
// script.  Its link error task is observable before the resumed script.
#[tokio::test(flavor = "multi_thread")]
async fn failed_stylesheet_releases_set_document_content_parser_script() {
    let release_stylesheet = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_release = release_stylesheet.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/missing.css",
            axum::routing::get(move || {
                let release = handler_release.clone();
                async move {
                    release.notified().await;
                    (axum::http::StatusCode::NOT_FOUND, "missing")
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 101,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": format!(
                concat!(
                    "<script>globalThis.__failedGateOrder = [];</script>",
                    "<link rel=stylesheet href='http://{addr}/missing.css' ",
                    "onerror=\"__failedGateOrder.push('style-error')\">",
                    "<script>__failedGateOrder.push('script'); globalThis.__failedGateRan = true;</script>",
                    "<footer id=failed-gate-tail>tail</footer>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 101)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            102,
            None,
            "({ order: __failedGateOrder, ran: globalThis.__failedGateRan === true })",
        )
        .await,
        json!({ "order": [], "ran": false }),
    );

    release_stylesheet.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "failed stylesheet-gated load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            103,
            None,
            r#"({
                order: __failedGateOrder,
                ran: __failedGateRan,
                tail: document.getElementById('failed-gate-tail')?.textContent,
                sheets: document.styleSheets.length,
            })"#,
        )
        .await,
        json!({
            "order": ["style-error", "script"],
            "ran": true,
            "tail": "tail",
            "sheets": 1,
        }),
    );

    server.abort();
}

// Ported from Blink's incremental body-stylesheet parser tests.  Each link
// owns its load event independently, while the parser-blocking script waits
// until every stylesheet that precedes it has settled.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_waits_for_all_prior_stylesheets_but_dispatches_each_load() {
    let release_first = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_second = std::sync::Arc::new(tokio::sync::Notify::new());
    let first_handler_release = release_first.clone();
    let second_handler_release = release_second.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/first.css",
                axum::routing::get(move || {
                    let release = first_handler_release.clone();
                    async move {
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#multi-gate-probe { color: rgb(21, 22, 23); }",
                        )
                    }
                }),
            )
            .route(
                "/second.css",
                axum::routing::get(move || {
                    let release = second_handler_release.clone();
                    async move {
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#multi-gate-probe { background-color: rgb(31, 32, 33); }",
                        )
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 104,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": format!(
                concat!(
                    "<script>globalThis.__multiGateOrder = [];</script>",
                    "<main id=multi-gate-probe>probe</main>",
                    "<link rel=stylesheet href='http://{addr}/first.css' ",
                    "onload=\"__multiGateOrder.push('first-load'); console.log('first-sheet-loaded')\">",
                    "<link rel=stylesheet href='http://{addr}/second.css' ",
                    "onload=\"__multiGateOrder.push('second-load')\">",
                    "<script>__multiGateOrder.push('script'); globalThis.__multiGateRan = true;</script>",
                    "<footer id=multi-gate-tail>tail</footer>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 104)["result"], json!({}));

    release_first.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "first stylesheet load before the second stylesheet settles",
        |message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["args"][0]["value"] == json!("first-sheet-loaded")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            105,
            None,
            r#"({
                order: __multiGateOrder,
                ran: globalThis.__multiGateRan === true,
                tail: document.getElementById('multi-gate-tail'),
            })"#,
        )
        .await,
        json!({ "order": ["first-load"], "ran": false, "tail": null }),
        "one completed stylesheet must not release a script blocked by another",
    );

    release_second.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "multi-stylesheet document-open load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            106,
            None,
            r#"({
                order: __multiGateOrder,
                ran: __multiGateRan,
                tail: document.getElementById('multi-gate-tail')?.textContent,
                color: getComputedStyle(document.getElementById('multi-gate-probe')).color,
                background: getComputedStyle(document.getElementById('multi-gate-probe')).backgroundColor,
            })"#,
        )
        .await,
        json!({
            "order": ["first-load", "second-load", "script"],
            "ran": true,
            "tail": "tail",
            "color": "rgb(21, 22, 23)",
            "background": "rgb(31, 32, 33)",
        }),
    );

    server.abort();
}

// Ported from Blink's ShouldNotPauseParsingForExternalNonMatchingStylesheetsInBody.
// A print-only sheet still loads and delays the load event, but it is not a
// script-blocking stylesheet for the screen document.
#[tokio::test(flavor = "multi_thread")]
async fn nonmatching_media_stylesheet_does_not_block_set_document_content_parser() {
    let release_stylesheet = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_release = release_stylesheet.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/print.css",
            axum::routing::get(move || {
                let release = handler_release.clone();
                async move {
                    release.notified().await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                        "#media-gate-probe { color: rgb(41, 42, 43); }",
                    )
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 107,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": format!(
                concat!(
                    "<main id=media-gate-probe>probe</main>",
                    "<link rel=stylesheet media=print href='http://{addr}/print.css'>",
                    "<script>globalThis.__nonmatchingMediaScriptRan = true;</script>",
                    "<footer id=nonmatching-media-tail>tail</footer>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 107)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            108,
            None,
            r#"({
                ran: __nonmatchingMediaScriptRan,
                tail: document.getElementById('nonmatching-media-tail')?.textContent,
            })"#,
        )
        .await,
        json!({ "ran": true, "tail": "tail" }),
    );

    release_stylesheet.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "nonmatching-media stylesheet load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;

    server.abort();
}

// Exact port of Blink HTMLDocumentParserLoadingTest::
// ShouldNotPauseParsingForExternalNonMatchingStylesheetsInBody. `type=print`
// is an unsupported stylesheet MIME type, so the link neither blocks parsing
// nor exposes a CSSStyleSheet.
#[tokio::test(flavor = "multi_thread")]
async fn unsupported_type_stylesheet_does_not_block_or_create_sheet() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 140,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": concat!(
                "<main id=unsupported-type-before>before</main>",
                "<link id=unsupported-type-link rel=stylesheet type=print ",
                "href='data:text/css,%23unsupported-type-after%7Bcolor%3Ared%7D'>",
                "<footer id=unsupported-type-after>after</footer>"
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 140)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            141,
            None,
            r#"({
                after: document.getElementById('unsupported-type-after')?.textContent,
                sheetIsNull: document.getElementById('unsupported-type-link').sheet === null,
            })"#,
        )
        .await,
        json!({ "after": "after", "sheetIsNull": true }),
    );
}

// A later Document::SetContent/ImplicitOpen replaces Blink's parser and its
// pending parsing-blocking script.  Completion from the removed document must
// not execute stale script or resume stale parser input in the surviving realm.
#[tokio::test(flavor = "multi_thread")]
async fn replacement_discards_stylesheet_blocked_set_document_content_script() {
    let release_old = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_new = std::sync::Arc::new(tokio::sync::Notify::new());
    let old_handler_release = release_old.clone();
    let new_handler_release = release_new.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/old.css",
                axum::routing::get(move || {
                    let release = old_handler_release.clone();
                    async move {
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#old-tail { color: red; }",
                        )
                    }
                }),
            )
            .route(
                "/new.css",
                axum::routing::get(move || {
                    let release = new_handler_release.clone();
                    async move {
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#new-tail { color: rgb(51, 52, 53); }",
                        )
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;

    ctx.process_async(json!({
        "id": 109,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": format!(
                concat!(
                    "<link rel=stylesheet href='http://{addr}/old.css'>",
                    "<script>globalThis.__staleStylesheetGateRan = true;</script>",
                    "<footer id=old-tail>old tail</footer>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 109)["result"], json!({}));

    ctx.process_async(json!({
        "id": 110,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": format!(
                concat!(
                    "<link rel=stylesheet href='http://{addr}/new.css'>",
                    "<script>",
                    "globalThis.__replacementStylesheetGateRan = true;",
                    "console.log('replacement-stylesheet-gate-ran');",
                    "</script>",
                    "<footer id=new-tail>new tail</footer>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 110)["result"], json!({}));
    ctx.sent.clear();

    release_old.notify_one();
    release_new.notify_one();
    wait_until_scheduler_message(&mut ctx, "replacement stylesheet-gated script", |message| {
        message["method"] == json!("Runtime.consoleAPICalled")
            && message["params"]["args"][0]["value"] == json!("replacement-stylesheet-gate-ran")
    })
    .await;
    wait_until_scheduler_message(
        &mut ctx,
        "replacement stylesheet-gated load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            111,
            None,
            r#"({
                staleRan: globalThis.__staleStylesheetGateRan === true,
                replacementRan: globalThis.__replacementStylesheetGateRan === true,
                oldTail: document.getElementById('old-tail'),
                newTail: document.getElementById('new-tail')?.textContent,
                newColor: getComputedStyle(document.getElementById('new-tail')).color,
            })"#,
        )
        .await,
        json!({
            "staleRan": false,
            "replacementRan": true,
            "oldTail": null,
            "newTail": "new tail",
            "newColor": "rgb(51, 52, 53)",
        }),
    );

    server.abort();
}

// Blink's preload scanner prepares parsing-blocking external scripts while an
// earlier body stylesheet pauses token consumption. Cover both completion
// races: the first source becomes ready before the sheet, while the parser
// claims the still-pending second source after the sheet settles. Both requests
// remain parser-initiated and execution stays in DOM order.
#[tokio::test(flavor = "multi_thread")]
async fn stylesheet_blocked_external_set_document_content_script_fetches_in_parallel() {
    let stylesheet_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let script_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let second_script_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let script_request_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let second_script_request_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let release_stylesheet = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_script = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_second_script = std::sync::Arc::new(tokio::sync::Notify::new());
    let stylesheet_handler_requested = stylesheet_requested.clone();
    let script_handler_requested = script_requested.clone();
    let second_script_handler_requested = second_script_requested.clone();
    let script_handler_request_count = script_request_count.clone();
    let second_script_handler_request_count = second_script_request_count.clone();
    let stylesheet_handler_release = release_stylesheet.clone();
    let script_handler_release = release_script.clone();
    let second_script_handler_release = release_second_script.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/parallel.css",
                axum::routing::get(move || {
                    let requested = stylesheet_handler_requested.clone();
                    let release = stylesheet_handler_release.clone();
                    async move {
                        requested.notify_one();
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#parallel-probe { color: rgb(61, 62, 63); }",
                        )
                    }
                }),
            )
            .route(
                "/parallel.js",
                axum::routing::get(move || {
                    let requested = script_handler_requested.clone();
                    let request_count = script_handler_request_count.clone();
                    let release = script_handler_release.clone();
                    async move {
                        request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        requested.notify_one();
                        release.notified().await;
                        (
                            [(
                                axum::http::header::CONTENT_TYPE.as_str(),
                                "application/javascript",
                            )],
                            concat!(
                                "__parallelGateOrder.push('script-1');",
                                "globalThis.__parallelGateColor = ",
                                "getComputedStyle(document.getElementById('parallel-probe')).color;",
                                "console.log('parallel-first-script-ran');"
                            ),
                        )
                    }
                }),
            )
            .route(
                "/parallel-second.js",
                axum::routing::get(move || {
                    let requested = second_script_handler_requested.clone();
                    let request_count = second_script_handler_request_count.clone();
                    let release = second_script_handler_release.clone();
                    async move {
                        request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        requested.notify_one();
                        release.notified().await;
                        (
                            [(
                                axum::http::header::CONTENT_TYPE.as_str(),
                                "application/javascript",
                            )],
                            "__parallelGateOrder.push('script-2');",
                        )
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.process_async(json!({
        "id": 114,
        "method": "Network.enable",
        "sessionId": "SID-1",
        "params": {},
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 114)["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 112,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": format!(
                concat!(
                    "<script>globalThis.__parallelGateOrder = [];</script>",
                    "<main id=parallel-probe>probe</main>",
                    "<link rel=stylesheet href='http://{addr}/parallel.css' ",
                    "onload=\"__parallelGateOrder.push('style-load')\">",
                    "<script src='http://{addr}/parallel.js' ",
                    "onload=\"__parallelGateOrder.push('script-1-load')\"></script>",
                    "<script src='http://{addr}/parallel-second.js' ",
                    "onload=\"__parallelGateOrder.push('script-2-load')\"></script>",
                    "<footer id=parallel-tail>tail</footer>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 112)["result"], json!({}));

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(
            stylesheet_requested.notified(),
            script_requested.notified(),
            second_script_requested.notified(),
        );
    })
    .await
    .expect("stylesheet and both external parser-script fetches must start before any completes");

    release_script.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "external parser-script source completion while stylesheet remains pending",
        |message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["response"]["url"]
                    == json!(format!("http://{addr}/parallel.js"))
        },
    )
    .await;
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.requestWillBeSent")
            && message["params"]["request"]["url"] == json!(format!("http://{addr}/parallel.js"))
            && message["params"]["initiator"]["type"] == json!("parser")
    }));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            115,
            None,
            r#"({
                order: __parallelGateOrder,
                ran: globalThis.__parallelGateColor !== undefined,
                tail: document.getElementById('parallel-tail'),
            })"#,
        )
        .await,
        json!({ "order": [], "ran": false, "tail": null }),
        "ready external sources must remain pending behind their stylesheet blocker",
    );

    release_stylesheet.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "first preloaded parser script after stylesheet completion",
        |message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["args"][0]["value"] == json!("parallel-first-script-ran")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            117,
            None,
            r#"({
                order: __parallelGateOrder,
                color: __parallelGateColor,
                tail: document.getElementById('parallel-tail'),
            })"#,
        )
        .await,
        json!({
            "order": ["style-load", "script-1", "script-1-load"],
            "color": "rgb(61, 62, 63)",
            "tail": null,
        }),
        "the parser must stop at the second source that is still pending",
    );

    release_second_script.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "parser-claimed speculative source completion",
        |message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["response"]["url"]
                    == json!(format!("http://{addr}/parallel-second.js"))
        },
    )
    .await;
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.requestWillBeSent")
            && message["params"]["request"]["url"]
                == json!(format!("http://{addr}/parallel-second.js"))
            && message["params"]["initiator"]["type"] == json!("parser")
    }));
    wait_until_scheduler_message(
        &mut ctx,
        "parallel stylesheet/external-script load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            116,
            None,
            r#"({
                order: __parallelGateOrder,
                color: __parallelGateColor,
                tail: document.getElementById('parallel-tail')?.textContent,
            })"#,
        )
        .await,
        json!({
            "order": [
                "style-load",
                "script-1",
                "script-1-load",
                "script-2",
                "script-2-load",
            ],
            "color": "rgb(61, 62, 63)",
            "tail": "tail",
        }),
    );
    assert_eq!(
        script_request_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the parser must consume the first speculative source without refetching it",
    );
    assert_eq!(
        second_script_request_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the parser must consume the second speculative source without refetching it",
    );

    server.abort();
}

// Ported from Blink HTMLDocumentParserLoadingTest::
// ShouldPauseParsingForExternalStylesheetsInBody and the corresponding
// wpt_internal parser-blocking-stylesheet-001.html.  A parser-created blocking
// stylesheet is itself a parser boundary; no following script is required.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_pauses_parser_tail_at_body_stylesheet() {
    let release_stylesheet = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_release = release_stylesheet.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/parser-pause.css",
            axum::routing::get(move || {
                let release = handler_release.clone();
                async move {
                    release.notified().await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                        "#after-parser-pause { color: rgb(71, 72, 73); }",
                    )
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 117,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": format!(
                concat!(
                    "<main id=before-parser-pause>before</main>",
                    "<link rel=stylesheet href='http://{addr}/parser-pause.css'>",
                    "<footer id=after-parser-pause>after</footer>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 117)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            118,
            None,
            r#"({
                before: document.getElementById('before-parser-pause')?.textContent,
                after: document.getElementById('after-parser-pause'),
            })"#,
        )
        .await,
        json!({ "before": "before", "after": null }),
        "tokens following a parser-created blocking stylesheet must remain unconsumed",
    );

    release_stylesheet.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "body stylesheet parser-pause load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            119,
            None,
            r#"({
                after: document.getElementById('after-parser-pause')?.textContent,
                color: getComputedStyle(document.getElementById('after-parser-pause')).color,
            })"#,
        )
        .await,
        json!({ "after": "after", "color": "rgb(71, 72, 73)" }),
    );

    server.abort();
}

// Ported from Blink HTMLDocumentParserLoadingTest::
// ShouldPauseParsingForExternalStylesheetsInBodyIncremental.  Resuming at one
// completed stylesheet must consume input only up to the next unresolved
// parser-created stylesheet boundary.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_resumes_incrementally_across_body_stylesheets() {
    let release_first = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_second = std::sync::Arc::new(tokio::sync::Notify::new());
    let first_handler_release = release_first.clone();
    let second_handler_release = release_second.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/incremental-first.css",
                axum::routing::get(move || {
                    let release = first_handler_release.clone();
                    async move {
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#incremental-after-first { color: rgb(81, 82, 83); }",
                        )
                    }
                }),
            )
            .route(
                "/incremental-second.css",
                axum::routing::get(move || {
                    let release = second_handler_release.clone();
                    async move {
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#incremental-after-second { color: rgb(84, 85, 86); }",
                        )
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 120,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": format!(
                concat!(
                    "<script>globalThis.__incrementalSheetOrder = [];</script>",
                    "<main id=incremental-before>before</main>",
                    "<link rel=stylesheet href='http://{addr}/incremental-first.css' ",
                    "onload=\"__incrementalSheetOrder.push('first-load'); ",
                    "console.log('incremental-first-loaded')\">",
                    "<section id=incremental-after-first>after first</section>",
                    "<link rel=stylesheet href='http://{addr}/incremental-second.css' ",
                    "onload=\"__incrementalSheetOrder.push('second-load')\">",
                    "<footer id=incremental-after-second>after second</footer>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 120)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            121,
            None,
            r#"({
                before: document.getElementById('incremental-before')?.textContent,
                afterFirst: document.getElementById('incremental-after-first'),
                afterSecond: document.getElementById('incremental-after-second'),
            })"#,
        )
        .await,
        json!({ "before": "before", "afterFirst": null, "afterSecond": null }),
    );

    release_first.notify_one();
    wait_until_scheduler_message(&mut ctx, "first incremental body stylesheet", |message| {
        message["method"] == json!("Runtime.consoleAPICalled")
            && message["params"]["args"][0]["value"] == json!("incremental-first-loaded")
    })
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            122,
            None,
            r#"({
                order: __incrementalSheetOrder,
                afterFirst: document.getElementById('incremental-after-first')?.textContent,
                afterSecond: document.getElementById('incremental-after-second'),
            })"#,
        )
        .await,
        json!({
            "order": ["first-load"],
            "afterFirst": "after first",
            "afterSecond": null,
        }),
        "the first completion must resume only as far as the second stylesheet",
    );

    release_second.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "incremental body stylesheet load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            123,
            None,
            r#"({
                order: __incrementalSheetOrder,
                afterSecond: document.getElementById('incremental-after-second')?.textContent,
                firstColor: getComputedStyle(document.getElementById('incremental-after-first')).color,
                secondColor: getComputedStyle(document.getElementById('incremental-after-second')).color,
            })"#,
        )
        .await,
        json!({
            "order": ["first-load", "second-load"],
            "afterSecond": "after second",
            "firstColor": "rgb(81, 82, 83)",
            "secondColor": "rgb(84, 85, 86)",
        }),
    );

    server.abort();
}

// Ported from Blink HTMLDocumentParserLoadingTest::
// ShouldPauseParsingForExternalStylesheetsImportedInBody.  A parser-created
// style element with an unresolved @import is a parser boundary as well.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_pauses_parser_tail_at_style_import() {
    let release_root_stylesheet = std::sync::Arc::new(tokio::sync::Notify::new());
    let nested_stylesheet_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_nested_stylesheet = std::sync::Arc::new(tokio::sync::Notify::new());
    let root_request_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let nested_request_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let font_request_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let root_handler_release = release_root_stylesheet.clone();
    let root_handler_request_count = root_request_count.clone();
    let nested_handler_requested = nested_stylesheet_requested.clone();
    let nested_handler_release = release_nested_stylesheet.clone();
    let nested_handler_request_count = nested_request_count.clone();
    let font_handler_request_count = font_request_count.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/imported.css",
                axum::routing::get(move || {
                    let release = root_handler_release.clone();
                    let request_count = root_handler_request_count.clone();
                    async move {
                        request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            concat!(
                                "@import url('/nested.css');",
                                "#after-style-import { color: rgb(91, 92, 93); }"
                            ),
                        )
                    }
                }),
            )
            .route(
                "/nested.css",
                axum::routing::get(move || {
                    let requested = nested_handler_requested.clone();
                    let release = nested_handler_release.clone();
                    let request_count = nested_handler_request_count.clone();
                    async move {
                        request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        requested.notify_one();
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            concat!(
                                "@font-face { font-family: NestedImport; ",
                                "src: url('/nested.woff2') format('woff2'); }",
                                "#after-style-import { ",
                                "background-color: rgb(94, 95, 96); ",
                                "font-family: NestedImport; }"
                            ),
                        )
                    }
                }),
            )
            .route(
                "/nested.woff2",
                axum::routing::get(move || {
                    let request_count = font_handler_request_count.clone();
                    async move {
                        request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "font/woff2")],
                            "font-body",
                        )
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new_with_target_discovery_and_optional_resource_fetch_mask(
        true,
        moli_core::OptionalResourceFetchMask::FONT,
    );
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 124,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": format!(
                concat!(
                    "<main id=before-style-import>before</main>",
                    "<style>@import url('http://{addr}/imported.css');</style>",
                    "<footer id=after-style-import>after</footer>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 124)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            125,
            None,
            "({ after: document.getElementById('after-style-import') })",
        )
        .await,
        json!({ "after": null }),
    );

    release_root_stylesheet.notify_one();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        nested_stylesheet_requested.notified(),
    )
    .await
    .expect("the nested import should start after its parent stylesheet arrives");
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            126,
            None,
            "({ after: document.getElementById('after-style-import') })",
        )
        .await,
        json!({ "after": null }),
        "the parser tail must remain paused until the complete import graph settles",
    );

    release_nested_stylesheet.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "style import parser-pause load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            127,
            None,
            r#"({
                after: document.getElementById('after-style-import')?.textContent,
                color: getComputedStyle(document.getElementById('after-style-import')).color,
                background: getComputedStyle(document.getElementById('after-style-import')).backgroundColor,
                fontFamily: getComputedStyle(document.getElementById('after-style-import')).fontFamily,
                importSheet: document.querySelector('style').sheet.cssRules[0].styleSheet instanceof CSSStyleSheet,
                importRulesBlocked: (() => {
                    try {
                        document.querySelector('style').sheet.cssRules[0].styleSheet.cssRules;
                        return false;
                    } catch (error) {
                        return error.name === 'SecurityError';
                    }
                })(),
            })"#,
        )
        .await,
        json!({
            "after": "after",
            "color": "rgb(91, 92, 93)",
            "background": "rgb(94, 95, 96)",
            "fontFamily": "NestedImport",
            "importSheet": true,
            "importRulesBlocked": true,
        }),
    );
    assert_eq!(
        root_request_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the parser blocker and live stylesheet must share the root import request",
    );
    assert_eq!(
        nested_request_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the complete import graph must request each nested stylesheet once",
    );
    assert_eq!(
        font_request_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "an imported stylesheet must retain authority to schedule its dependent resources",
    );

    server.abort();
}

// Blink starts independent top-level @import fetches from one parser-created
// style sheet without serializing them. Their rules retain import order and
// all precede the ordinary rules in the owning inline sheet.
#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_style_imports_fetch_in_parallel_and_all_apply() {
    let first_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let second_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let first_request_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let second_request_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let release_first = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_second = std::sync::Arc::new(tokio::sync::Notify::new());
    let first_handler_requested = first_requested.clone();
    let second_handler_requested = second_requested.clone();
    let first_handler_request_count = first_request_count.clone();
    let second_handler_request_count = second_request_count.clone();
    let first_handler_release = release_first.clone();
    let second_handler_release = release_second.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/first-import.css",
                axum::routing::get(move || {
                    let requested = first_handler_requested.clone();
                    let request_count = first_handler_request_count.clone();
                    let release = first_handler_release.clone();
                    async move {
                        request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        requested.notify_one();
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#multi-import-target { color: rgb(121, 122, 123); }",
                        )
                    }
                }),
            )
            .route(
                "/second-import.css",
                axum::routing::get(move || {
                    let requested = second_handler_requested.clone();
                    let request_count = second_handler_request_count.clone();
                    let release = second_handler_release.clone();
                    async move {
                        request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        requested.notify_one();
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#multi-import-target { background-color: rgb(124, 125, 126); }",
                        )
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 133,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": format!(
                concat!(
                    "<style>",
                    "@import url('http://{addr}/first-import.css');",
                    "@import url('http://{addr}/second-import.css');",
                    "#multi-import-target {{ border-top-color: rgb(127, 128, 129); }}",
                    "</style>",
                    "<main id=multi-import-target>target</main>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 133)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            134,
            None,
            "({ target: document.getElementById('multi-import-target')?.textContent })",
        )
        .await,
        json!({ "target": "target" }),
        "a head stylesheet blocks scripts and load, not parser token consumption",
    );

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(first_requested.notified(), second_requested.notified());
    })
    .await
    .expect("all top-level imports should start before either response is released");

    release_first.notify_one();
    release_second.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "multiple style imports load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            135,
            None,
            r#"(() => {
                const style = getComputedStyle(document.getElementById('multi-import-target'));
                return {
                    color: style.color,
                    background: style.backgroundColor,
                    border: style.borderTopColor,
                    importSheets: Array.from(document.querySelector('style').sheet.cssRules)
                        .filter(rule => rule instanceof CSSImportRule)
                        .map(rule => ({
                            href: rule.href,
                            loaded: rule.styleSheet instanceof CSSStyleSheet,
                            rulesBlocked: (() => {
                                try {
                                    rule.styleSheet.cssRules;
                                    return false;
                                } catch (error) {
                                    return error.name === 'SecurityError';
                                }
                            })(),
                        })),
                };
            })()"#,
        )
        .await,
        json!({
            "color": "rgb(121, 122, 123)",
            "background": "rgb(124, 125, 126)",
            "border": "rgb(127, 128, 129)",
            "importSheets": [
                {
                    "href": format!("http://{addr}/first-import.css"),
                    "loaded": true,
                    "rulesBlocked": true,
                },
                {
                    "href": format!("http://{addr}/second-import.css"),
                    "loaded": true,
                    "rulesBlocked": true,
                },
            ],
        }),
    );
    assert_eq!(
        first_request_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "one parser-created style owner must have one canonical first-import request",
    );
    assert_eq!(
        second_request_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "one parser-created style owner must have one canonical second-import request",
    );

    server.abort();
}

// Ported from Blink HTMLDocumentParserLoadingTest::
// ShouldPauseParsingForExternalStylesheetsWrittenInBody.  A link written by a
// parser-blocking script remains parser-created and therefore blocks the outer
// parser after the writing script returns.
#[tokio::test(flavor = "multi_thread")]
async fn document_written_stylesheet_pauses_set_document_content_parser_tail() {
    let stylesheet_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let script_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_stylesheet = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_script = std::sync::Arc::new(tokio::sync::Notify::new());
    let script_request_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let stylesheet_handler_requested = stylesheet_requested.clone();
    let script_handler_requested = script_requested.clone();
    let stylesheet_handler_release = release_stylesheet.clone();
    let script_handler_release = release_script.clone();
    let script_handler_request_count = script_request_count.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/written.css",
                axum::routing::get(move || {
                    let requested = stylesheet_handler_requested.clone();
                    let release = stylesheet_handler_release.clone();
                    async move {
                        requested.notify_one();
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#before-written-stylesheet { color: rgb(101, 102, 103); }",
                        )
                    }
                }),
            )
            .route(
                "/after-written.js",
                axum::routing::get(move || {
                    let requested = script_handler_requested.clone();
                    let release = script_handler_release.clone();
                    let request_count = script_handler_request_count.clone();
                    async move {
                        request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        requested.notify_one();
                        release.notified().await;
                        (
                            [(
                                axum::http::header::CONTENT_TYPE.as_str(),
                                "application/javascript",
                            )],
                            concat!(
                                "__writtenStylesheetOrder.push('external-script');",
                                "globalThis.__writtenStylesheetScriptColor = ",
                                "getComputedStyle(document.getElementById('before-written-stylesheet')).color;"
                            ),
                        )
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.process_async(json!({
        "id": 126,
        "method": "Network.enable",
        "sessionId": "SID-1",
        "params": {},
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 126)["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 127,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": format!(
                concat!(
                    "<main id=before-written-stylesheet>before</main>",
                    "<script>",
                    "globalThis.__writtenStylesheetOrder = [];",
                    "document.write(`<link rel=stylesheet ",
                    "href='http://{addr}/written.css' ",
                    "onload=\"__writtenStylesheetOrder.push('style-load')\">`);",
                    "__writtenStylesheetOrder.push('script-end');",
                    "</script>",
                    "<script src='http://{addr}/after-written.js' ",
                    "onload=\"__writtenStylesheetOrder.push('external-load')\"></script>",
                    "<footer id=after-written-stylesheet>after</footer>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 127)["result"], json!({}));
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(stylesheet_requested.notified(), script_requested.notified());
    })
    .await
    .expect("the preload scanner must cross a document-written stylesheet parser pause");

    release_script.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "preloaded script after document-written stylesheet",
        |message| {
            message["method"] == json!("Network.responseReceived")
                && message["params"]["response"]["url"]
                    == json!(format!("http://{addr}/after-written.js"))
        },
    )
    .await;
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Network.requestWillBeSent")
            && message["params"]["request"]["url"]
                == json!(format!("http://{addr}/after-written.js"))
            && message["params"]["initiator"]["type"] == json!("parser")
    }));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            128,
            None,
            r#"({
                order: __writtenStylesheetOrder,
                externalRan: globalThis.__writtenStylesheetScriptColor !== undefined,
                after: document.getElementById('after-written-stylesheet'),
            })"#,
        )
        .await,
        json!({ "order": ["script-end"], "externalRan": false, "after": null }),
        "a document-written stylesheet must block the outer parser, not the writing script",
    );

    release_stylesheet.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "document-written stylesheet load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            129,
            None,
            r#"({
                order: __writtenStylesheetOrder,
                after: document.getElementById('after-written-stylesheet')?.textContent,
                color: getComputedStyle(document.getElementById('before-written-stylesheet')).color,
                scriptColor: globalThis.__writtenStylesheetScriptColor ?? null,
            })"#,
        )
        .await,
        json!({
            "order": ["script-end", "style-load", "external-script", "external-load"],
            "after": "after",
            "color": "rgb(101, 102, 103)",
            "scriptColor": "rgb(101, 102, 103)",
        }),
    );
    assert_eq!(
        script_request_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the parser must reuse the speculative external-script request",
    );

    server.abort();
}

// Exact sequencing from Blink HTMLDocumentParserLoadingTest::
// ShouldPauseParsingForExternalStylesheetsWrittenInBody. The head sheet first
// gates the parser script. Once it completes, that script writes a body sheet,
// which establishes a second gate before the remaining parser tail.
#[tokio::test(flavor = "multi_thread")]
async fn document_written_body_stylesheet_remains_blocked_after_head_sheet_completes() {
    let head_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let body_requested = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_head = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_body = std::sync::Arc::new(tokio::sync::Notify::new());
    let head_handler_requested = head_requested.clone();
    let body_handler_requested = body_requested.clone();
    let head_handler_release = release_head.clone();
    let body_handler_release = release_body.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/written-head.css",
                axum::routing::get(move || {
                    let requested = head_handler_requested.clone();
                    let release = head_handler_release.clone();
                    async move {
                        requested.notify_one();
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#written-with-head-before { color: rgb(141, 142, 143); }",
                        )
                    }
                }),
            )
            .route(
                "/written-body.css",
                axum::routing::get(move || {
                    let requested = body_handler_requested.clone();
                    let release = body_handler_release.clone();
                    async move {
                        requested.notify_one();
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#written-with-head-after { color: rgb(144, 145, 146); }",
                        )
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 142,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": format!(
                concat!(
                    "<!doctype html><html><head>",
                    "<link rel=stylesheet href='http://{addr}/written-head.css' ",
                    "onload=\"console.log('written-head-loaded')\">",
                    "</head><body>",
                    "<main id=written-with-head-before>before</main>",
                    "<script>document.write(",
                    "`<link rel=stylesheet href='http://{addr}/written-body.css'>`",
                    ");</script>",
                    "<footer id=written-with-head-after>after</footer>",
                    "</body></html>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 142)["result"], json!({}));
    tokio::time::timeout(std::time::Duration::from_secs(2), head_requested.notified())
        .await
        .expect("the head stylesheet should start before its parser script");
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            143,
            None,
            r#"({
                before: document.getElementById('written-with-head-before')?.textContent,
                after: document.getElementById('written-with-head-after'),
            })"#,
        )
        .await,
        json!({ "before": "before", "after": null }),
    );

    release_head.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "head stylesheet completion before parser-written body stylesheet",
        |message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["args"][0]["value"] == json!("written-head-loaded")
        },
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(2), body_requested.notified())
        .await
        .expect(
            "the parser script should request its body stylesheet after the head sheet completes",
        );
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            144,
            None,
            "({ after: document.getElementById('written-with-head-after') })",
        )
        .await,
        json!({ "after": null }),
        "completing the head sheet must not release the written body sheet gate",
    );

    release_body.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "parser-written body stylesheet after head completion",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            145,
            None,
            r#"(() => {
                const before = document.getElementById('written-with-head-before');
                const after = document.getElementById('written-with-head-after');
                return {
                    after: after?.textContent,
                    beforeColor: getComputedStyle(before).color,
                    afterColor: getComputedStyle(after).color,
                };
            })()"#,
        )
        .await,
        json!({
            "after": "after",
            "beforeColor": "rgb(141, 142, 143)",
            "afterColor": "rgb(144, 145, 146)",
        }),
    );

    server.abort();
}

// Ported from Blink HTMLDocumentParserLoadingTest::
// ShouldNotPauseParsingForExternalStylesheetsAttachedInBody.  A stylesheet
// dynamically attached by script delays load, but it was not created by the
// parser and must not consume the parser's insertion point.
#[tokio::test(flavor = "multi_thread")]
async fn dynamically_attached_stylesheet_does_not_pause_set_document_content_parser() {
    let release_stylesheet = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_release = release_stylesheet.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/dynamic.css",
            axum::routing::get(move || {
                let release = handler_release.clone();
                async move {
                    release.notified().await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                        "#after-dynamic-stylesheet { color: rgb(111, 112, 113); }",
                    )
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 130,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": format!(
                concat!(
                    "<main id=before-dynamic-stylesheet>before</main>",
                    "<script>",
                    "globalThis.__dynamicStylesheetOrder = [];",
                    "const link = document.createElement('link');",
                    "link.rel = 'stylesheet';",
                    "link.href = 'http://{addr}/dynamic.css';",
                    "link.onload = () => {{",
                    "  __dynamicStylesheetOrder.push('style-load');",
                    "  console.log('dynamic-stylesheet-loaded');",
                    "}};",
                    "document.head.append(link);",
                    "__dynamicStylesheetOrder.push('script-end');",
                    "</script>",
                    "<footer id=after-dynamic-stylesheet>after</footer>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 130)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            131,
            None,
            r#"({
                order: __dynamicStylesheetOrder,
                after: document.getElementById('after-dynamic-stylesheet')?.textContent,
            })"#,
        )
        .await,
        json!({ "order": ["script-end"], "after": "after" }),
        "a dynamically attached stylesheet may delay load but must not pause parsing",
    );

    release_stylesheet.notify_one();
    wait_until_scheduler_message(&mut ctx, "dynamic stylesheet load handler", |message| {
        message["method"] == json!("Runtime.consoleAPICalled")
            && message["params"]["args"][0]["value"] == json!("dynamic-stylesheet-loaded")
    })
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            132,
            None,
            r#"({
                order: __dynamicStylesheetOrder,
                color: getComputedStyle(document.getElementById('after-dynamic-stylesheet')).color,
            })"#,
        )
        .await,
        json!({
            "order": ["script-end", "style-load"],
            "color": "rgb(111, 112, 113)",
        }),
    );

    server.abort();
}

// Blink clears HTMLLinkElement::created_by_parser_ in FinishParsingChildren.
// Reprocessing that same node after script removal/reinsertion is therefore a
// dynamic stylesheet load: it may delay load, but must not block the parser
// script that follows the reinsertion.
#[tokio::test(flavor = "multi_thread")]
async fn reinserted_parser_created_link_does_not_reenter_the_parser_blocking_gate() {
    let release_initial = std::sync::Arc::new(tokio::sync::Notify::new());
    let initial_handler_release = release_initial.clone();
    let release_reprocessed = std::sync::Arc::new(tokio::sync::Notify::new());
    let reprocessed_handler_release = release_reprocessed.clone();
    let reprocessed_request_arrived = std::sync::Arc::new(tokio::sync::Notify::new());
    let reprocessed_handler_arrived = reprocessed_request_arrived.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/initial.css",
                axum::routing::get(move || {
                    let release = initial_handler_release.clone();
                    async move {
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#reprocessed-tail { color: rgb(151, 152, 153); }",
                        )
                    }
                }),
            )
            .route(
                "/reprocessed.css",
                axum::routing::get(move || {
                    let release = reprocessed_handler_release.clone();
                    let arrived = reprocessed_handler_arrived.clone();
                    async move {
                        arrived.notify_one();
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#reprocessed-tail { color: rgb(154, 155, 156); }",
                        )
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>old</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 133,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": format!(
                concat!(
                    "<body>",
                    "<link id=reprocessed-link rel=stylesheet ",
                    "href='http://{addr}/initial.css'>",
                    "<script>",
                    "globalThis.__reprocessedLinkOrder = [];",
                    "const link = document.getElementById('reprocessed-link');",
                    "link.remove();",
                    "link.href = 'http://{addr}/reprocessed.css';",
                    "document.body.append(link);",
                    "__reprocessedLinkOrder.push('reinserted');",
                    "console.log('parser-link-reinserted');",
                    "</script>",
                    "<script>",
                    "__reprocessedLinkOrder.push('following-script');",
                    "console.log('parser-following-script');",
                    "</script>",
                    "<footer id=reprocessed-tail>tail</footer>",
                    "</body>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 133)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            134,
            None,
            "({ tail: document.getElementById('reprocessed-tail') })",
        )
        .await,
        json!({ "tail": null }),
        "the original parser-created body stylesheet must hold the parser tail",
    );

    release_initial.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "parser script following the reinserted link",
        |message| {
            message["method"] == json!("Runtime.consoleAPICalled")
                && message["params"]["args"][0]["value"] == json!("parser-following-script")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            135,
            None,
            r#"({
                order: __reprocessedLinkOrder,
                tail: document.getElementById('reprocessed-tail')?.textContent,
            })"#,
        )
        .await,
        json!({
            "order": ["reinserted", "following-script"],
            "tail": "tail",
        }),
        "dynamic reprocessing must not reuse the link's consumed parser-blocking identity",
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reprocessed_request_arrived.notified(),
    )
    .await
    .expect("dynamic reinsertion must start the link's replacement stylesheet request");
    assert!(
        ctx.sent.iter().all(|message| {
            message["method"] != json!("Page.lifecycleEvent")
                || message["params"]["frameId"] != json!("TID-1")
                || message["params"]["name"] != json!("load")
        }),
        "the dynamically reprocessed stylesheet may not block parser progress, but must still delay load",
    );

    release_reprocessed.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "dynamically reprocessed stylesheet load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == json!("TID-1")
                && message["params"]["name"] == json!("load")
        },
    )
    .await;

    server.abort();
}

// The parser-blocking stylesheet boundary is document-local, not restricted
// to the root PageVm parser. A child Document::SetContent must park the exact
// child insertion session and resume it on that child's stylesheet owner.
#[tokio::test(flavor = "multi_thread")]
async fn child_set_document_content_pauses_parser_tail_at_body_stylesheet() {
    let release_stylesheet = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_release = release_stylesheet.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/child-pause.css",
            axum::routing::get(move || {
                let release = handler_release.clone();
                async move {
                    release.notified().await;
                    (
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                        "#child-after-stylesheet { color: rgb(131, 132, 133); }",
                    )
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(
        &mut ctx,
        "data:text/html,<body><iframe srcdoc='<main>old</main>'></iframe></body>",
    )
    .await;
    enable_document_content_observers(&mut ctx).await;
    let before_tree = frame_tree(&mut ctx, 136).await;
    let child = before_tree["childFrames"][0]["frame"].clone();
    let child_frame_id = child["id"].as_str().expect("child frame id");
    assert_initial_child_frame_is_attached(&ctx, child_frame_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 137,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": child_frame_id,
            "html": format!(
                concat!(
                    "<main id=child-before-stylesheet>before</main>",
                    "<link rel=stylesheet href='http://{addr}/child-pause.css'>",
                    "<footer id=child-after-stylesheet>after</footer>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 137)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            138,
            None,
            r#"(() => {
                const child = document.querySelector('iframe').contentDocument;
                return {
                    before: child.getElementById('child-before-stylesheet')?.textContent,
                    after: child.getElementById('child-after-stylesheet'),
                };
            })()"#,
        )
        .await,
        json!({ "before": "before", "after": null }),
    );

    release_stylesheet.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "child stylesheet parser-pause load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == child["id"]
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            139,
            None,
            r#"(() => {
                const child = document.querySelector('iframe').contentDocument;
                const after = child.getElementById('child-after-stylesheet');
                return {
                    after: after?.textContent,
                    color: child.defaultView.getComputedStyle(after).color,
                };
            })()"#,
        )
        .await,
        json!({ "after": "after", "color": "rgb(131, 132, 133)" }),
    );

    server.abort();
}

// The child parser owns one continuation and can encounter a new stylesheet
// gate after the previous one releases. This mirrors Blink's incremental body
// stylesheet test on a child Document::SetContent path.
#[tokio::test(flavor = "multi_thread")]
async fn child_set_document_content_resumes_only_to_next_stylesheet_gate() {
    let release_first = std::sync::Arc::new(tokio::sync::Notify::new());
    let release_second = std::sync::Arc::new(tokio::sync::Notify::new());
    let first_handler_release = release_first.clone();
    let second_handler_release = release_second.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new()
            .route(
                "/child-first.css",
                axum::routing::get(move || {
                    let release = first_handler_release.clone();
                    async move {
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#child-after-first { color: rgb(151, 152, 153); }",
                        )
                    }
                }),
            )
            .route(
                "/child-second.css",
                axum::routing::get(move || {
                    let release = second_handler_release.clone();
                    async move {
                        release.notified().await;
                        (
                            [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                            "#child-after-second { color: rgb(154, 155, 156); }",
                        )
                    }
                }),
            );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(
        &mut ctx,
        "data:text/html,<body><iframe srcdoc='<main>old</main>'></iframe></body>",
    )
    .await;
    enable_document_content_observers(&mut ctx).await;
    let before_tree = frame_tree(&mut ctx, 146).await;
    let child = before_tree["childFrames"][0]["frame"].clone();
    let child_frame_id = child["id"].as_str().expect("child frame id");
    assert_initial_child_frame_is_attached(&ctx, child_frame_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 147,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": child_frame_id,
            "html": format!(
                concat!(
                    "<main id=child-incremental-before>before</main>",
                    "<link rel=stylesheet href='http://{addr}/child-first.css'>",
                    "<section id=child-after-first>after first</section>",
                    "<link rel=stylesheet href='http://{addr}/child-second.css'>",
                    "<footer id=child-after-second>after second</footer>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 147)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            148,
            None,
            r#"(() => {
                const child = document.querySelector('iframe').contentDocument;
                return {
                    before: child.getElementById('child-incremental-before')?.textContent,
                    afterFirst: child.getElementById('child-after-first'),
                    afterSecond: child.getElementById('child-after-second'),
                };
            })()"#,
        )
        .await,
        json!({ "before": "before", "afterFirst": null, "afterSecond": null }),
    );

    release_first.notify_one();
    let mut first_gate_resumed = false;
    for id in 1146..=1210 {
        first_gate_resumed = evaluate_by_value(
            &mut ctx,
            id,
            None,
            "document.querySelector('iframe').contentDocument.getElementById('child-after-first') !== null",
        )
        .await
            == json!(true);
        if first_gate_resumed {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        first_gate_resumed,
        "the first stylesheet terminal must resume the child parser"
    );
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            149,
            None,
            r#"(() => {
                const child = document.querySelector('iframe').contentDocument;
                return {
                    afterFirst: child.getElementById('child-after-first')?.textContent,
                    afterSecond: child.getElementById('child-after-second'),
                };
            })()"#,
        )
        .await,
        json!({ "afterFirst": "after first", "afterSecond": null }),
    );

    release_second.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "second child incremental stylesheet lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == child["id"]
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            150,
            None,
            r#"(() => {
                const child = document.querySelector('iframe').contentDocument;
                const first = child.getElementById('child-after-first');
                const second = child.getElementById('child-after-second');
                return {
                    afterSecond: second?.textContent,
                    firstColor: child.defaultView.getComputedStyle(first).color,
                    secondColor: child.defaultView.getComputedStyle(second).color,
                };
            })()"#,
        )
        .await,
        json!({
            "afterSecond": "after second",
            "firstColor": "rgb(151, 152, 153)",
            "secondColor": "rgb(154, 155, 156)",
        }),
    );

    server.abort();
}

// A failed child stylesheet settles the same parser-owned gate as a successful
// response. Failure must resume that child only and still complete its load
// lifecycle; Chromium retains the failed link's CSSStyleSheet owner object.
#[tokio::test(flavor = "multi_thread")]
async fn failed_child_stylesheet_releases_set_document_content_parser_tail() {
    let release_stylesheet = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_release = release_stylesheet.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/missing-child.css",
            axum::routing::get(move || {
                let release = handler_release.clone();
                async move {
                    release.notified().await;
                    (
                        axum::http::StatusCode::NOT_FOUND,
                        [(axum::http::header::CONTENT_TYPE.as_str(), "text/css")],
                        "missing",
                    )
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(
        &mut ctx,
        "data:text/html,<body><iframe srcdoc='<main>old</main>'></iframe></body>",
    )
    .await;
    enable_document_content_observers(&mut ctx).await;
    let before_tree = frame_tree(&mut ctx, 151).await;
    let child = before_tree["childFrames"][0]["frame"].clone();
    let child_frame_id = child["id"].as_str().expect("child frame id");
    assert_initial_child_frame_is_attached(&ctx, child_frame_id);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 152,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": child_frame_id,
            "html": format!(
                concat!(
                    "<main id=failed-child-before>before</main>",
                    "<link id=failed-child-link rel=stylesheet ",
                    "href='http://{addr}/missing-child.css'>",
                    "<footer id=failed-child-after>after</footer>"
                ),
                addr = addr,
            ),
        },
    }))
    .await;
    assert_eq!(take_response_by_id(&mut ctx, 152)["result"], json!({}));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            153,
            None,
            r#"(() => {
                const child = document.querySelector('iframe').contentDocument;
                return { after: child.getElementById('failed-child-after') };
            })()"#,
        )
        .await,
        json!({ "after": null }),
    );

    release_stylesheet.notify_one();
    wait_until_scheduler_message(
        &mut ctx,
        "failed child stylesheet load lifecycle",
        |message| {
            message["method"] == json!("Page.lifecycleEvent")
                && message["params"]["frameId"] == child["id"]
                && message["params"]["name"] == json!("load")
        },
    )
    .await;
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            154,
            None,
            r#"(() => {
                const child = document.querySelector('iframe').contentDocument;
                return {
                    after: child.getElementById('failed-child-after')?.textContent,
                    sheetIsNull: child.getElementById('failed-child-link').sheet === null,
                };
            })()"#,
        )
        .await,
        json!({ "after": "after", "sheetIsNull": false }),
    );

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_emits_child_document_open_before_slow_parser_script_load() {
    let release_script = std::sync::Arc::new(tokio::sync::Notify::new());
    let handler_release = release_script.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/slow.js",
            axum::routing::get(move || {
                let release = handler_release.clone();
                async move {
                    release.notified().await;
                    (
                        [(
                            axum::http::header::CONTENT_TYPE.as_str(),
                            "application/javascript",
                        )],
                        "window.__slowSetContentScriptRan = true;",
                    )
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let mut ctx = TestContext::new();
    install_document_content_test_page(
        &mut ctx,
        "data:text/html,<body><iframe srcdoc='<main>old</main>'></iframe></body>",
    )
    .await;
    enable_document_content_observers(&mut ctx).await;
    let before_tree = frame_tree(&mut ctx, 35).await;
    let child = before_tree["childFrames"][0]["frame"].clone();
    let child_frame_id = child["id"].as_str().expect("child frame id");
    assert_initial_child_frame_is_attached(&ctx, child_frame_id);
    ctx.sent.clear();

    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        ctx.process_async(json!({
            "id": 36,
            "method": "Page.setDocumentContent",
            "sessionId": "SID-1",
            "params": {
                "frameId": child_frame_id,
                "html": format!(
                    "<script src='http://{addr}/slow.js'></script><main id=after-slow>new</main>"
                ),
            },
        })),
    )
    .await
    .expect("setDocumentContent must not wait for the external parser script");

    let immediate = ctx.take_all();
    let opened_index = message_index(&immediate, "child Page.documentOpened", |message| {
        message["method"] == json!("Page.documentOpened")
            && message["params"]["frame"]["id"] == child["id"]
    });
    let init_index = message_index(&immediate, "child lifecycle init", |message| {
        message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["frameId"] == child["id"]
            && message["params"]["name"] == json!("init")
    });
    let response_index = message_index(&immediate, "Page.setDocumentContent response", |message| {
        message["id"] == json!(36) && message["result"] == json!({})
    });
    assert!(
        opened_index < init_index && init_index < response_index,
        "document-open events should precede the nonblocking response: {immediate:?}"
    );
    assert!(
        immediate.iter().all(|message| {
            message["method"] != json!("DOM.documentUpdated")
                && !(message["method"] == json!("Page.lifecycleEvent")
                    && message["params"]["frameId"] == child["id"]
                    && matches!(
                        message["params"]["name"].as_str(),
                        Some("DOMContentLoaded" | "load")
                    ))
        }),
        "DCL/load must remain pending behind the parser script: {immediate:?}"
    );

    release_script.notify_one();
    wait_until_scheduler_message(&mut ctx, "child document-open load lifecycle", |message| {
        message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["frameId"] == child["id"]
            && message["params"]["name"] == json!("load")
    })
    .await;
    assert!(
        ctx.sent
            .iter()
            .all(|message| message["method"] != json!("Page.documentOpened")),
        "load completion must not repeat Page.documentOpened: {:?}",
        ctx.sent
    );
    assert!(ctx.sent.iter().any(|message| {
        message["method"] == json!("Page.lifecycleEvent")
            && message["params"]["frameId"] == child["id"]
            && message["params"]["name"] == json!("DOMContentLoaded")
    }));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_reports_chromium_errors_without_mutating_the_document() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(
        &mut ctx,
        "data:text/html,<body><main id=still-present>unchanged</main></body>",
    )
    .await;
    enable_document_content_observers(&mut ctx).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 40,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": { "frameId": "TID-1" },
    }))
    .await;
    ctx.expect_error(40, -32602, "Invalid parameters");
    ctx.process_async(json!({
        "id": 41,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": { "frameId": "FRAME-does-not-exist", "html": "<main>bad</main>" },
    }))
    .await;
    ctx.expect_error(41, -32000, "No frame for given id found");
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            42,
            None,
            "document.querySelector('#still-present').textContent",
        )
        .await,
        json!("unchanged")
    );

    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .start_document_navigation_for_active_target("PENDING-LOADER".to_owned())
        .expect("pending navigation should start");
    ctx.process_async(json!({
        "id": 44,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": { "frameId": "TID-1", "html": "<main>stale</main>" },
    }))
    .await;
    ctx.expect_error(44, -32000, "Navigation is changing the document");

    let mut no_document = TestContext::new();
    load_bc_with_session(
        &mut no_document,
        "BID-no-document",
        "TID-no-document",
        "SID-no-document",
        "about:blank",
    );
    no_document
        .process_async(json!({
            "id": 43,
            "method": "Page.setDocumentContent",
            "sessionId": "SID-no-document",
            "params": { "frameId": "TID-no-document", "html": "<main>missing</main>" },
        }))
        .await;
    no_document.expect_error(43, -32000, "No Document instance to set HTML for");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_document_content_respects_script_execution_disabled() {
    let mut ctx = TestContext::new();
    install_document_content_test_page(&mut ctx, "data:text/html,<body>initial</body>").await;
    enable_document_content_observers(&mut ctx).await;
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 50,
        "method": "Emulation.setScriptExecutionDisabled",
        "sessionId": "SID-1",
        "params": { "value": true },
    }))
    .await;
    ctx.expect_result(50, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 51,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": concat!(
                "<main id=script-disabled>updated</main>",
                "<script>window.__disabledScriptRan = true;</script>",
                "<iframe id=disabled-child srcdoc=\"<script>parent.__disabledChildScriptRan=true;</script><main>child</main>\"></iframe>"
            ),
        },
    }))
    .await;
    ctx.expect_result(51, json!({}), Some("SID-1"));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            52,
            None,
            "({ text: document.querySelector('#script-disabled').textContent, scriptType: typeof __disabledScriptRan, childScriptType: typeof __disabledChildScriptRan, childText: document.querySelector('#disabled-child').contentDocument.querySelector('main').textContent })",
        )
        .await,
        json!({
            "text": "updated",
            "scriptType": "undefined",
            "childScriptType": "undefined",
            "childText": "child",
        })
    );

    ctx.process_async(json!({
        "id": 89,
        "method": "Emulation.setScriptExecutionDisabled",
        "sessionId": "SID-1",
        "params": { "value": false },
    }))
    .await;
    ctx.expect_result(89, json!({}), Some("SID-1"));
    ctx.process_async(json!({
        "id": 90,
        "method": "Page.setDocumentContent",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "html": "<script>window.__reenabledSetContentScriptRan = true;</script>"
        },
    }))
    .await;
    ctx.expect_result(90, json!({}), Some("SID-1"));
    assert_eq!(
        evaluate_by_value(
            &mut ctx,
            91,
            None,
            "globalThis.__reenabledSetContentScriptRan === true",
        )
        .await,
        json!(true)
    );
}
