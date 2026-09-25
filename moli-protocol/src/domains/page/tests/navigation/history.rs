use super::*;

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
async fn joint_session_history_syncs_child_steps_cursor_and_reload_bootstrap() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><body><script>globalThis.initialHistoryLength=history.length</script>",
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
    ctx.process_async(json!({"id": 9280, "method":"Page.navigate", "sessionId":"SID-1", "params":{"url":format!("http://{addr}/page")}})).await;
    assert!(take_response_by_id(&mut ctx, 9280)["error"].is_null());
    wait_until_message(
        &mut ctx,
        Some("SID-1"),
        "joint history page load",
        |message| message["method"] == json!("Page.domContentEventFired"),
    )
    .await;

    let operations = [
        (
            r#"(async () => {
            for (const id of ['a', 'b']) {
                const f = document.createElement('iframe'); f.id = id; f.srcdoc = '<p>child</p>';
                await new Promise(resolve => { f.onload = resolve; document.body.append(f); });
            }
            globalThis.a = document.getElementById('a').contentWindow;
            globalThis.b = document.getElementById('b').contentWindow;
            globalThis.held = [history, a.history, b.history];
            history.replaceState('top', ''); a.history.replaceState('a0', ''); b.history.replaceState('b0', '');
            a.history.pushState('a1', ''); b.history.pushState('b1', ''); a.history.pushState('a2', '');
            return held.map(h => h.length);
        })()"#,
            json!([5, 5, 5]),
            4,
            5,
        ),
        (
            r#"new Promise(resolve => {
            b.addEventListener('popstate', () => resolve(held.map(h => h.length)), {once:true});
            history.go(-2);
        })"#,
            json!([5, 5, 5]),
            2,
            5,
        ),
        (
            "b.history.pushState('b2', ''); held.map(h => h.length)",
            json!([4, 4, 4]),
            3,
            4,
        ),
    ];
    for (expression, expected, index, length) in operations {
        ctx.process_async(json!({"id":9281,"method":"Runtime.evaluate","sessionId":"SID-1", "params":{"expression":expression,"awaitPromise":true,"returnByValue":true}})).await;
        wait_until_message(
            &mut ctx,
            Some("SID-1"),
            "joint history operation",
            |message| message["id"] == json!(9281),
        )
        .await;
        let value = take_response_by_id(&mut ctx, 9281);
        assert!(value["error"].is_null(), "{value}");
        assert_eq!(value["result"]["result"]["value"], expected, "{value}");
        ctx.process_async(
            json!({"id":9282,"method":"Page.getNavigationHistory","sessionId":"SID-1"}),
        )
        .await;
        let browser = take_response_by_id(&mut ctx, 9282);
        assert_eq!(browser["result"]["currentIndex"], json!(index), "{browser}");
        assert_eq!(
            browser["result"]["entries"].as_array().unwrap().len(),
            length,
            "{browser}"
        );
    }
    ctx.process_async(json!({"id":9285,"method":"Runtime.evaluate","sessionId":"SID-1", "params":{"expression":r#"
        history.pushState('before-reset', '');
        navigation.entries()[0].addEventListener('dispose', () => history.pushState('from-dispose', ''));
        'armed'
    "#}})).await;
    assert!(take_response_by_id(&mut ctx, 9285)["result"]["exceptionDetails"].is_null());
    ctx.process_async(
        json!({"id":9286,"method":"Page.resetNavigationHistory","sessionId":"SID-1"}),
    )
    .await;
    assert!(take_response_by_id(&mut ctx, 9286)["error"].is_null());
    ctx.process_async(json!({"id":9287,"method":"Runtime.evaluate","sessionId":"SID-1", "params":{"expression":"[...held.map(h => h.length), a.navigation.entries().length, b.navigation.entries().length, history.state]","returnByValue":true}})).await;
    assert_eq!(
        take_response_by_id(&mut ctx, 9287)["result"]["result"]["value"],
        json!([2, 2, 2, 1, 1, "from-dispose"])
    );
    ctx.process_async(json!({"id":9288,"method":"Page.getNavigationHistory","sessionId":"SID-1"}))
        .await;
    let reset = take_response_by_id(&mut ctx, 9288);
    assert_eq!(reset["result"]["currentIndex"], json!(1), "{reset}");
    assert_eq!(reset["result"]["entries"].as_array().unwrap().len(), 2);
    ctx.sent.clear();
    ctx.process_async(json!({"id":9283,"method":"Page.reload","sessionId":"SID-1"}))
        .await;
    assert!(take_response_by_id(&mut ctx, 9283)["error"].is_null());
    wait_until_message(&mut ctx, Some("SID-1"), "joint history reload", |message| {
        message["method"] == json!("Page.domContentEventFired")
    })
    .await;
    ctx.process_async(json!({"id":9284,"method":"Runtime.evaluate","sessionId":"SID-1", "params":{"expression":"[initialHistoryLength, history.length]","returnByValue":true}})).await;
    assert_eq!(
        take_response_by_id(&mut ctx, 9284)["result"]["result"]["value"],
        json!([2, 2])
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn joint_history_traversal_info_stays_with_first_participant() {
    assert_joint_history_traversal_info(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn joint_history_traversal_info_stays_with_last_participant() {
    assert_joint_history_traversal_info(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn joint_history_traversal_cross_document_before_same_document() {
    assert_joint_history_multi_frame_traversal("a", false, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn queued_history_back_preserves_same_document_before_cross_document() {
    assert_queued_history_same_then_cross_document(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn queued_history_forward_preserves_same_document_before_cross_document() {
    assert_queued_history_same_then_cross_document(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn joint_history_traversal_same_document_before_cross_document() {
    assert_joint_history_multi_frame_traversal("b", false, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn joint_history_traversal_schedules_both_cross_document_frames() {
    assert_joint_history_multi_frame_traversal("a", true, false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn joint_history_traversal_reentrant_admission_cancels_every_participant() {
    for cross in ["a", "b"] {
        assert_joint_history_multi_frame_traversal(cross, false, true).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn joint_history_traversal_precommit_holds_every_participant_and_browser_cursor() {
    assert_joint_history_precommit("resolve").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn joint_history_traversal_precommit_rejection_aborts_the_whole_step() {
    assert_joint_history_precommit("reject").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn joint_history_traversal_stop_during_pagehide_preserves_accepted_commit() {
    assert_joint_history_precommit("stop-pagehide").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn joint_history_traversal_participant_replacement_rejects_surviving_api_promises() {
    assert_joint_history_precommit("replace-participant").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn joint_history_traversal_precommit_cannot_restore_a_pruned_step() {
    assert_joint_history_precommit("prune").await;
}

#[tokio::test]
async fn joint_history_traversal_precommit_keeps_an_unchanged_attached_context() {
    assert_joint_history_precommit("attach").await;
}

#[tokio::test]
async fn joint_history_single_target_precommit_keeps_an_unchanged_attached_context() {
    assert_joint_history_precommit("attach-single").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn joint_history_traversal_late_response_cannot_overwrite_a_successor_entry() {
    assert_joint_history_late_response(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn joint_history_traversal_outgoing_state_update_keeps_the_accepted_destination() {
    assert_joint_history_late_response(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_bootstraps_browser_history_length_before_author_scripts() {
    async fn page() -> impl axum::response::IntoResponse {
        (
            [(axum::http::header::CONTENT_TYPE.as_str(), "text/html")],
            "<!doctype html><script>globalThis.initialHistoryLength=history.length</script>",
        )
    }
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind history bootstrap server");
    let addr = listener
        .local_addr()
        .expect("history bootstrap server address");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route("/{page}", axum::routing::get(page)),
        )
        .await
        .unwrap();
    });
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let first_url = format!("http://{addr}/first");
    let second_url = format!("http://{addr}/second");
    let third_url = format!("http://{addr}/third");
    let mut first_entry_id = None;

    for step in 0..5 {
        ctx.sent.clear();
        let (method, params) = match step {
            0 => ("Page.navigate", json!({ "url": first_url })),
            1 => ("Page.navigate", json!({ "url": second_url })),
            2 => ("Page.reload", json!({})),
            3 => (
                "Page.navigateToHistoryEntry",
                json!({ "entryId": first_entry_id }),
            ),
            _ => ("Page.navigate", json!({ "url": third_url })),
        };
        ctx.process_async(json!({
            "id": 9210,
            "method": method,
            "sessionId": "SID-1",
            "params": params
        }))
        .await;
        let response = take_response_by_id(&mut ctx, 9210);
        assert!(response["error"].is_null(), "{method}: {response}");
        wait_until_message(
            &mut ctx,
            Some("SID-1"),
            "history bootstrap document DOMContentLoaded",
            |message| message["method"] == json!("Page.domContentEventFired"),
        )
        .await;

        ctx.process_async(json!({
            "id": 9211,
            "method": "Page.getNavigationHistory",
            "sessionId": "SID-1"
        }))
        .await;
        let history = take_response_by_id(&mut ctx, 9211);
        let entries = history["result"]["entries"]
            .as_array()
            .expect("history entries");
        let expected_length = if step == 0 { 2 } else { 3 };
        assert_eq!(entries.len(), expected_length, "{method}");
        if step == 0 {
            first_entry_id = entries[1]["id"].as_i64();
        }

        ctx.process_async(json!({
            "id": 9212,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "({initial:initialHistoryLength, current:history.length})",
                "returnByValue": true
            }
        }))
        .await;
        let state = take_response_by_id(&mut ctx, 9212);
        assert_eq!(
            state["result"]["result"]["value"],
            json!({
                "initial": expected_length,
                "current": expected_length
            }),
            "{method}: {state}"
        );
    }
    server.abort();
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
    ctx.enable_page_events_for_test(Some("SID-RENDERER-HISTORY"));
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
        ctx.wait_for_scheduler_message("initial history entry commit", |message| {
            message["method"] == "Page.frameNavigated"
                && message["sessionId"] == "SID-RENDERER-HISTORY"
                && message["params"]["frame"]["url"] == url
        })
        .await;
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
    ctx.wait_for_scheduler_message("renderer history.back() commit", |message| {
        message["method"] == "Page.frameNavigated"
            && message["sessionId"] == "SID-RENDERER-HISTORY"
            && message["params"]["frame"]["url"] == first_url
    })
    .await;
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
    ctx.wait_for_scheduler_message("renderer history.back() to initial document", |message| {
        message["method"] == "Page.frameNavigated"
            && message["sessionId"] == "SID-RENDERER-HISTORY"
            && message["params"]["frame"]["url"] == "about:blank"
    })
    .await;
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
    ctx.wait_for_scheduler_message("renderer history.forward() commit", |message| {
        message["method"] == "Page.frameNavigated"
            && message["sessionId"] == "SID-RENDERER-HISTORY"
            && message["params"]["frame"]["url"] == first_url
    })
    .await;
    assert_eq!(
        ctx.conn.browser_context.as_ref().unwrap().target_url(),
        first_url
    );
    // The forward commit also drains the earlier out-of-range traversal.
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("id").is_none_or(|id| !id.is_null())),
        "an out-of-range page traversal must not emit an id:null command response: {:?}",
        ctx.sent
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
            "historyLength": 2,
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
    ctx.conn.install_browser_context_fixture_for_test(bc);
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
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);
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
    ctx.conn.install_browser_context_fixture_for_test(bc);
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
    ctx.conn
        .push_inactive_browser_context_fixture_for_test(inactive);

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

#[tokio::test(flavor = "multi_thread")]
async fn browser_document_navigation_keeps_page_residence_and_replaces_document_agent() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/first", axum::routing::get(|| async {
                    axum::response::Html("<!doctype html><title>first</title><script>window.firstDocument = true</script>")
                }))
                .route("/feed", axum::routing::get(|| async {
                    (
                        [(axum::http::header::CONTENT_TYPE, "application/atom+xml; charset=windows-1252")],
                        b"<feed xmlns='http://www.w3.org/2005/Atom'><title>caf\xe9</title></feed>".to_vec(),
                    )
                })),
        ).await.unwrap();
    });
    let mut ctx = TestContext::new();
    let session = "SID-stable-document";
    load_bc_with_session(
        &mut ctx,
        "BID-stable-document",
        "TID-stable-document",
        session,
        "about:blank",
    );
    ensure_initial_document_for_session(&mut ctx, Some(session)).await;
    for (id, method) in [
        (90_200, "Page.enable"),
        (90_201, "Runtime.enable"),
        (90_202, "DOM.enable"),
    ] {
        ctx.process_async(json!({"id":id,"method":method,"sessionId":session}))
            .await;
        assert!(take_response_by_id(&mut ctx, id)["error"].is_null());
    }
    let owner = crate::conn::CommandOwnerScope::for_session(session);
    let page = ctx
        .conn
        .renderer_page_residence_identity_for_owner(&owner)
        .unwrap();
    let residence = ctx
        .conn
        .target_page_residence_identity_for_owner(&owner)
        .unwrap();
    let mut agent = ctx
        .conn
        .current_renderer_agent_attachment_id_for_owner(&owner)
        .unwrap();
    let large_html = format!(
        "<!doctype html><pre>{}</pre><script>window.lastScriptRan = true</script>",
        "x".repeat(20 * 64 * 1024),
    );
    let urls = [
        format!("http://{addr}/first"),
        format!(
            "data:text/html;base64,{}",
            BASE64_STANDARD.encode(large_html)
        ),
        format!("http://{addr}/feed"),
        "about:blank".to_owned(),
    ];
    let checks = [
        ("[document.title, firstDocument]", json!(["first", true])),
        (
            "[typeof firstDocument, lastScriptRan, document.querySelector('pre').textContent.length]",
            json!(["undefined", true, 1310720]),
        ),
        (
            "(() => { const feed = document.getElementsByTagNameNS('http://www.w3.org/2005/Atom', 'feed')[0]; return [feed.namespaceURI, feed.textContent, document.characterSet]; })()",
            json!(["http://www.w3.org/2005/Atom", "café", "windows-1252"]),
        ),
        (
            "[document.URL, typeof lastScriptRan]",
            json!(["about:blank", "undefined"]),
        ),
    ];
    for (index, (url, (expression, expected))) in urls.into_iter().zip(checks).enumerate() {
        ctx.sent.clear();
        let id = 90_210 + index as u64 * 2;
        ctx.process_async(
            json!({"id":id,"method":"Page.navigate","sessionId":session,"params":{"url":url}}),
        )
        .await;
        assert!(
            take_response_by_id(&mut ctx, id)["error"].is_null(),
            "navigation {index}"
        );
        wait_until_message(
            &mut ctx,
            Some(session),
            "stable Page replacement load",
            |message| message["method"] == "Page.loadEventFired",
        )
        .await;
        assert_eq!(
            ctx.conn.renderer_page_residence_identity_for_owner(&owner),
            Some(page)
        );
        assert_eq!(
            ctx.conn.target_page_residence_identity_for_owner(&owner),
            Some(residence.clone())
        );
        let next_agent = ctx
            .conn
            .current_renderer_agent_attachment_id_for_owner(&owner)
            .unwrap();
        assert_ne!(
            next_agent, agent,
            "new Document needs a new Inspector attachment"
        );
        agent = next_agent;
        ctx.process_async(json!({"id":id+1,"method":"Runtime.evaluate","sessionId":session,"params":{"expression":expression,"returnByValue":true}})).await;
        let result = take_response_by_id(&mut ctx, id + 1);
        assert!(result["error"].is_null(), "{result}");
        assert_eq!(
            result["result"]["result"]["value"], expected,
            "navigation {index}: {result}"
        );

        let dom_id = 90_300 + index as u64 * 3;
        ctx.process_async(json!({"id":dom_id,"method":"DOM.getDocument","sessionId":session,"params":{"depth":1}})).await;
        let root = take_response_by_id(&mut ctx, dom_id)["result"]["root"].clone();
        let element_id = root["children"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["nodeType"] == 1)
            .unwrap()["nodeId"]
            .clone();
        ctx.sent.clear();
        ctx.process_async(json!({"id":dom_id+1,"method":"Runtime.evaluate","sessionId":session,"params":{"expression":"document.documentElement.setAttribute('data-replacement', 'current')"}})).await;
        assert!(take_response_by_id(&mut ctx, dom_id + 1)["error"].is_null());
        wait_until_message(
            &mut ctx,
            Some(session),
            "replacement Document DOM mutation",
            |message| {
                message["method"] == "DOM.attributeModified"
                    && message["params"]["nodeId"] == element_id
                    && message["params"]["name"] == "data-replacement"
                    && message["params"]["value"] == "current"
            },
        )
        .await;
    }
    server.abort();
}
