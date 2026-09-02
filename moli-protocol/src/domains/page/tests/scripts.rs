use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn add_script_to_evaluate_on_new_document_injects_script_into_future_navigation() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 25,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_preload = 'ready';"
        }
    }))
    .await;
    ctx.expect_result(25, json!({ "identifier": "1" }), Some("SID-1"));

    ctx.process_async(json!({
            "id": 26,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<body><script>document.body.textContent = globalThis.__lm_preload || 'missing';</script></body>"
            }
        })).await;
    let _ = ctx.take_all();

    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        html.contains(">ready<"),
        "expected preload script to run first, got {html}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn add_script_to_evaluate_on_new_document_preserves_registration_order() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 27,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_order = ['first'];"
        }
    }))
    .await;
    ctx.expect_result(27, json!({ "identifier": "1" }), Some("SID-1"));

    ctx.process_async(json!({
        "id": 28,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_order.push('second');"
        }
    }))
    .await;
    ctx.expect_result(28, json!({ "identifier": "2" }), Some("SID-1"));

    ctx.process_async(json!({
            "id": 29,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<body><script>document.body.textContent = globalThis.__lm_order.join(',');</script></body>"
            }
        })).await;
    let _ = ctx.take_all();

    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        html.contains(">first,second<"),
        "expected ordered preload scripts, got {html}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn add_script_to_evaluate_on_new_document_requires_browser_context() {
    let mut ctx = TestContext::new();

    ctx.process_async(json!({
        "id": 30,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "params": {
            "source": "globalThis.__lm_preload = true;"
        }
    }))
    .await;

    ctx.expect_error(30, -31998, "BrowserContextNotLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn add_script_to_evaluate_on_new_document_validates_params() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 31,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {}
    }))
    .await;

    ctx.expect_error(31, -32602, "InvalidParams");
}
#[tokio::test(flavor = "multi_thread")]
async fn add_script_to_evaluate_on_new_document_does_not_mutate_existing_page() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>stable</body>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    ctx.process_async(json!({
        "id": 35,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "document.body.setAttribute('data-preload', 'yes');"
        }
    }))
    .await;
    ctx.expect_result(35, json!({ "identifier": "1" }), Some("SID-1"));

    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        html.contains(">stable<"),
        "expected existing page content to stay unchanged, got {html}"
    );
    assert!(
        !html.contains("data-preload=\"yes\""),
        "expected preload script not to run against current page, got {html}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn add_script_to_evaluate_on_new_document_invalid_call_does_not_advance_identifier() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 36,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_preload = 1;"
        }
    }))
    .await;
    ctx.expect_result(36, json!({ "identifier": "1" }), Some("SID-1"));

    ctx.process_async(json!({
        "id": 37,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {}
    }))
    .await;
    ctx.expect_error(37, -32602, "InvalidParams");

    ctx.process_async(json!({
        "id": 38,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_preload = 2;"
        }
    }))
    .await;
    ctx.expect_result(38, json!({ "identifier": "2" }), Some("SID-1"));
}
#[tokio::test(flavor = "multi_thread")]
async fn add_script_to_evaluate_on_new_document_keeps_duplicate_registrations_distinct() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    for (id, identifier) in [(39, "1"), (40, "2")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "sessionId": "SID-1",
            "params": {
                "source": "globalThis.__lm_duplicate_count = (globalThis.__lm_duplicate_count || 0) + 1;"
            }
        }))
        .await;
        ctx.expect_result(id, json!({ "identifier": identifier }), Some("SID-1"));
    }

    let script_count = ctx
        .conn
        .browser_context
        .as_ref()
        .expect("browser context")
        .active_page_target()
        .owner_state
        .document_start_scripts
        .len();
    assert_eq!(
        script_count, 2,
        "each duplicate source registration must remain independently removable",
    );

    ctx.process_async(json!({
            "id": 41,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<body><script>document.body.textContent = String(globalThis.__lm_duplicate_count || 0);</script></body>"
            }
        })).await;
    let _ = ctx.take_all();

    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        html.contains(">2<"),
        "expected both duplicate preload registrations to execute, got {html}"
    );

    for (id, identifier) in [(42, "1"), (43, "2")] {
        ctx.process_async(json!({
            "id": id,
            "method": "Page.removeScriptToEvaluateOnNewDocument",
            "sessionId": "SID-1",
            "params": { "identifier": identifier }
        }))
        .await;
        ctx.expect_result(id, json!({}), Some("SID-1"));
    }
    ctx.process_async(json!({
        "id": 44,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": { "source": "globalThis.__lm_reused_identifier = true;" }
    }))
    .await;
    ctx.expect_result(44, json!({ "identifier": "1" }), Some("SID-1"));
}
#[tokio::test(flavor = "multi_thread")]
async fn add_script_to_evaluate_on_new_document_persists_across_multiple_navigations() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 32,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_nav_count = (globalThis.__lm_nav_count || 0) + 1;"
        }
    }))
    .await;
    ctx.expect_result(32, json!({ "identifier": "1" }), Some("SID-1"));

    ctx.process_async(json!({
            "id": 33,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<body><script>document.body.textContent = String(globalThis.__lm_nav_count);</script></body>"
            }
        })).await;
    let _ = ctx.take_all();
    let first_html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        first_html.contains(">1<"),
        "expected first navigation preload value, got {first_html}"
    );

    ctx.process_async(json!({
            "id": 34,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<body><script>document.body.textContent = String(globalThis.__lm_nav_count);</script></body>"
            }
        })).await;
    let _ = ctx.take_all();
    let second_html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        second_html.contains(">1<"),
        "expected second navigation preload value, got {second_html}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn add_script_to_evaluate_on_new_document_world_name_runs_in_isolated_world() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 348,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_preload = 'utility-ready';",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_result(348, json!({ "identifier": "1" }), Some("SID-1"));

    ctx.process_async(json!({
            "id": 349,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<body><script>document.body.textContent = String(globalThis.__lm_preload || 'missing');</script></body>"
            }
        })).await;
    let _ = ctx.take_all();

    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        html.contains(">missing<"),
        "expected default world to stay untouched, got {html}"
    );

    ctx.process_async(json!({"id": 350, "method": "Runtime.enable", "sessionId": "SID-1"}))
        .await;
    let _ = take_response_by_id(&mut ctx, 350);
    let utility_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility")
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("utility isolated world context id");

    ctx.process_async(json!({
        "id": 351,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.__lm_preload",
            "contextId": utility_context_id
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 351);
    assert_eq!(result["result"]["result"]["type"], json!("string"));
    assert_eq!(result["result"]["result"]["value"], json!("utility-ready"));
}
#[tokio::test(flavor = "multi_thread")]
async fn add_script_to_evaluate_on_new_document_world_name_preserves_order_within_world() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 352,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_order = ['first'];",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_result(352, json!({ "identifier": "1" }), Some("SID-1"));

    ctx.process_async(json!({
        "id": 353,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_order.push('second');",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_result(353, json!({ "identifier": "2" }), Some("SID-1"));

    ctx.process_async(json!({
        "id": 354,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": {
            "url": "data:text/html,<body>ok</body>"
        }
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({"id": 355, "method": "Runtime.enable", "sessionId": "SID-1"}))
        .await;
    let _ = take_response_by_id(&mut ctx, 355);
    let utility_context_id = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility")
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("utility isolated world context id");

    ctx.process_async(json!({
        "id": 356,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.__lm_order.join(',')",
            "contextId": utility_context_id
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 356);
    assert_eq!(result["result"]["result"]["type"], json!("string"));
    assert_eq!(result["result"]["result"]["value"], json!("first,second"));
}
#[tokio::test(flavor = "multi_thread")]
async fn add_script_to_evaluate_on_new_document_run_immediately_mutates_existing_page() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>stable</body>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    let raw = json!({
        "id": 357,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "document.body.setAttribute('data-preload', 'yes');",
            "runImmediately": true
        }
    })
    .to_string();
    let pending = ctx
        .conn
        .try_start_pending_command_dispatch(&raw)
        .expect("runImmediately addScript should use Page pending dispatch");
    let (messages, scheduler_events) =
        complete_pending_command_task_for_test(&mut ctx, pending).await;
    assert!(
        scheduler_events.is_empty(),
        "runImmediately addScript should not enqueue scheduler events: {scheduler_events:?}"
    );
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(357))
        .expect("Page.addScriptToEvaluateOnNewDocument response");
    assert_eq!(response["sessionId"], json!("SID-1"));
    assert_eq!(response["result"], json!({ "identifier": "1" }));

    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        html.contains("data-preload=\"yes\""),
        "expected preload script to run against current page, got {html}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn add_script_to_evaluate_on_new_document_run_immediately_world_name_creates_context() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>stable</body>")
        .await
        .expect("page should load");
    let bc = ctx.conn.browser_context.as_mut().expect("browser context");
    let _ = bc
        .active_page_target_mut()
        .runtime_slot
        .replace_loaded_page(Some(page));
    bc.set_target_security_origin("https://stale-top-origin.example".into());

    ctx.process_async(json!({
        "id": 357,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 357);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 358,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_now = 'utility-ready';",
            "worldName": "utility",
            "runImmediately": true
        }
    }))
    .await;

    let created = ctx.take_one();
    assert_eq!(created["method"], "Runtime.executionContextCreated");
    assert_eq!(created["params"]["context"]["name"], "utility");
    assert_eq!(created["params"]["context"]["origin"], "");
    assert_eq!(created["params"]["context"]["auxData"]["isDefault"], false);
    assert_eq!(created["params"]["context"]["auxData"]["frameId"], "TID-1");
    assert!(
        created["params"]["context"]["uniqueId"].as_str().is_some(),
        "runImmediately isolated-world event should come from V8 native batch: {created:?}"
    );
    let utility_context_id = created["params"]["context"]["id"]
        .as_i64()
        .expect("execution context id");
    ctx.expect_result(358, json!({ "identifier": "1" }), Some("SID-1"));

    ctx.process_async(json!({
        "id": 359,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "String(globalThis.__lm_now || 'missing')",
            "contextId": utility_context_id
        }
    }))
    .await;
    let utility_result = take_response_by_id(&mut ctx, 359);
    assert_eq!(
        utility_result["result"]["result"]["value"],
        json!("utility-ready")
    );

    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        html.contains(">stable<"),
        "expected default world page content to remain stable, got {html}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn remove_script_to_evaluate_on_new_document_prevents_future_navigation_injection() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 360,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_removed = 'gone';"
        }
    }))
    .await;
    ctx.expect_result(360, json!({ "identifier": "1" }), Some("SID-1"));

    ctx.process_async(json!({
        "id": 361,
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "identifier": "1"
        }
    }))
    .await;
    ctx.expect_result(361, json!({}), Some("SID-1"));

    ctx.process_async(json!({
            "id": 362,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<body><script>document.body.textContent = String(globalThis.__lm_removed || 'missing');</script></body>"
            }
        })).await;
    let _ = ctx.take_all();

    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        html.contains(">missing<"),
        "expected removed preload script to stop affecting future navigations, got {html}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn remove_script_to_evaluate_on_new_document_removes_named_world_preload() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 363,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_preload = 'utility-ready';",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_result(363, json!({ "identifier": "1" }), Some("SID-1"));

    ctx.process_async(json!({
        "id": 364,
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "identifier": "1"
        }
    }))
    .await;
    ctx.expect_result(364, json!({}), Some("SID-1"));

    ctx.process_async(json!({
            "id": 365,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<body><script>document.body.textContent = String(globalThis.__lm_preload || 'missing');</script></body>"
            }
        })).await;
    let _ = ctx.take_all();

    ctx.process_async(json!({"id": 366, "method": "Runtime.enable", "sessionId": "SID-1"}))
        .await;
    let _ = take_response_by_id(&mut ctx, 366);

    let utility_context = ctx.sent.iter().find(|message| {
        message["method"] == json!("Runtime.executionContextCreated")
            && message["params"]["context"]["name"] == json!("utility")
    });
    assert!(
        utility_context.is_none(),
        "expected removed named preload script not to create utility world on navigation"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn remove_script_to_evaluate_on_new_document_does_not_rollback_run_immediately_effect() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>stable</body>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    ctx.process_async(json!({
        "id": 367,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "document.body.setAttribute('data-preload', 'yes');",
            "runImmediately": true
        }
    }))
    .await;
    ctx.expect_result(367, json!({ "identifier": "1" }), Some("SID-1"));

    ctx.process_async(json!({
        "id": 368,
        "method": "Page.removeScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "identifier": "1"
        }
    }))
    .await;
    ctx.expect_result(368, json!({}), Some("SID-1"));

    let html = loaded_page_html_for_test(&mut ctx).await;
    assert!(
        html.contains("data-preload=\"yes\""),
        "expected removeScriptToEvaluateOnNewDocument not to rollback current-page runImmediately effects, got {html}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_requires_browser_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 39,
        "method": "Page.createIsolatedWorld",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_error(39, -31998, "BrowserContextNotLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_requires_matching_frame_and_uses_fresh_initial_document_without_adapter()
 {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 40,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-OTHER",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_error(40, -32000, "NoFrameForGivenId");

    let iframe_page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<iframe srcdoc=\"<p>child</p>\"></iframe>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(iframe_page);
    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 400).await;

    ctx.process_async(json!({
        "id": 401,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": child_frame_id,
            "worldName": "utility-child"
        }
    }))
    .await;
    let child_created = take_response_by_id(&mut ctx, 401);
    assert_eq!(child_created["sessionId"], "SID-1");
    assert!(
        child_created["result"]["executionContextId"]
            .as_i64()
            .is_some(),
        "child frame isolated world should now be created successfully"
    );

    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .clear_loaded_page();
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    ctx.process_async(json!({
        "id": 41,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility"
        }
    }))
    .await;
    let created = take_response_by_id(&mut ctx, 41);
    assert_eq!(created["sessionId"], "SID-1");
    assert!(created["result"]["executionContextId"].as_i64().is_some());
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|bc| bc.loaded_page())
            .is_some(),
        "Page.createIsolatedWorld should observe the target-lifecycle initial page"
    );

    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .clear_loaded_page();
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .set_target_url("data:text/html,<main>utility</main>".into());
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .begin_active_target_initial_empty_document("about:blank".into());
    ctx.process_async(json!({
        "id": 42,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility-2"
        }
    }))
    .await;
    ctx.expect_error(42, -32000, "NoDocumentLoaded");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;
    ctx.process_async(json!({
        "id": 43,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility-2"
        }
    }))
    .await;
    let created = take_response_by_id(&mut ctx, 43);
    assert_eq!(created["sessionId"], "SID-1");
    assert!(created["result"]["executionContextId"].as_i64().is_some());
    assert!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|bc| bc.loaded_page())
            .is_some_and(|page| page.final_url().as_str() == "data:text/html,<main>utility</main>"),
        "createIsolatedWorld should load the target initial URL before creating the world"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_reports_no_document_without_legacy_materialization_adapter() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 43,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility-no-document"
        }
    }))
    .await;
    ctx.expect_error(43, -32000, "NoDocumentLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_async_accepts_child_frame_from_async_dispatch() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ensure_initial_document_for_session(&mut ctx, Some("SID-1")).await;

    ctx.process_async(json!({
        "id": 4020,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-OTHER",
            "worldName": "utility"
        }
    }))
    .await;
    ctx.expect_error(4020, -32000, "NoFrameForGivenId");

    let iframe_page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<iframe srcdoc=\"<p>child</p>\"></iframe>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(iframe_page);

    ctx.process_async(json!({
        "id": 4021,
        "method": "Page.getFrameTree",
        "sessionId": "SID-1"
    }))
    .await;
    let child_frame_id = take_response_by_id(&mut ctx, 4021)["result"]["frameTree"]["childFrames"]
        [0]["frame"]["id"]
        .as_str()
        .expect("child frame id")
        .to_owned();

    ctx.process_async(json!({
        "id": 4022,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": child_frame_id,
            "worldName": "utility-child"
        }
    }))
    .await;

    let created = take_response_by_id(&mut ctx, 4022);
    assert_eq!(created["sessionId"], "SID-1");
    assert!(
        created["result"]["executionContextId"].as_i64().is_some(),
        "child frame isolated world should be created through the async dispatch path"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_for_child_frame_evaluates_in_child_scope() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<iframe srcdoc=\"<body>child-frame</body>\"></iframe>",
        Some("SID-1"),
    )
    .await;

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 402).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;

    ctx.process_async(json!({
        "id": 403,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": child_frame_id,
            "worldName": "utility-child"
        }
    }))
    .await;

    let context_id = take_response_by_id(&mut ctx, 403)["result"]["executionContextId"]
        .as_i64()
        .expect("child isolated execution context id");

    ctx.process_async(json!({
        "id": 404,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": context_id,
            "expression": "document.body.textContent.trim()"
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 404);
    assert_eq!(result["result"]["result"]["type"], json!("string"));
    assert_eq!(result["result"]["result"]["value"], json!("child-frame"));

    ctx.process_async(json!({
        "id": 405,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": context_id,
            "expression": "globalThis.eval('document.body.textContent.trim()')"
        }
    }))
    .await;
    let eval_result = take_response_by_id(&mut ctx, 405);
    assert_eq!(eval_result["result"]["result"]["type"], json!("string"));
    assert_eq!(
        eval_result["result"]["result"]["value"],
        json!("child-frame")
    );

    ctx.process_async(json!({
        "id": 406,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": context_id,
            "expression": "window.eval('document.body.textContent.trim()')"
        }
    }))
    .await;
    let window_eval_result = take_response_by_id(&mut ctx, 406);
    assert_eq!(
        window_eval_result["result"]["result"]["type"],
        json!("string")
    );
    assert_eq!(
        window_eval_result["result"]["result"]["value"],
        json!("child-frame")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_for_child_frame_replays_world_scoped_document_start_scripts() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<iframe srcdoc=\"<body>child-frame</body>\"></iframe>",
        Some("SID-1"),
    )
    .await;

    ctx.process_async(json!({
        "id": 408,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_child_world = document.body.textContent.trim();",
            "worldName": "utility-child"
        }
    }))
    .await;
    ctx.expect_result(408, json!({ "identifier": "1" }), Some("SID-1"));

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 409).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;

    ctx.process_async(json!({
        "id": 410,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": child_frame_id,
            "worldName": "utility-child"
        }
    }))
    .await;
    let context_id = take_response_by_id(&mut ctx, 410)["result"]["executionContextId"]
        .as_i64()
        .expect("child isolated execution context id");

    ctx.process_async(json!({
        "id": 411,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": context_id,
            "expression": "String(globalThis.__lm_child_world || 'missing')"
        }
    }))
    .await;
    let child_result = take_response_by_id(&mut ctx, 411);
    assert_eq!(
        child_result["result"]["result"]["value"],
        json!("child-frame")
    );

    ctx.process_async(json!({
        "id": 412,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "typeof globalThis.__lm_child_world"
        }
    }))
    .await;
    let default_result = take_response_by_id(&mut ctx, 412);
    assert_eq!(
        default_result["result"]["result"]["value"],
        json!("undefined")
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_for_child_frame_replays_world_scoped_runtime_bindings() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<iframe srcdoc=\"<body>child-frame</body>\"></iframe>",
        Some("SID-1"),
    )
    .await;

    ctx.process_async(json!({
        "id": 413,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "childUtilityBinding",
            "executionContextName": "utility-child"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 413);
    assert_eq!(add_binding["result"], json!({}));

    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 414).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;

    ctx.process_async(json!({
        "id": 415,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": child_frame_id,
            "worldName": "utility-child"
        }
    }))
    .await;
    let context_id = take_response_by_id(&mut ctx, 415)["result"]["executionContextId"]
        .as_i64()
        .expect("child isolated execution context id");

    ctx.process_async(json!({
        "id": 416,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": context_id,
            "expression": "globalThis.childUtilityBinding('child-payload'); 9"
        }
    }))
    .await;
    let call = take_response_by_id(&mut ctx, 416);
    assert_eq!(call["result"]["result"]["value"], json!(9));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("childUtilityBinding")
        })
        .cloned()
        .expect("child scoped binding should emit Runtime.bindingCalled");
    assert_eq!(binding_called["params"]["payload"], json!("child-payload"));
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(context_id)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_installs_matching_bindings_before_document_start_scripts() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>top-frame</body>")
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
        "id": 416,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(416, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 417,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "createWorldBinding",
            "executionContextName": "utility-create"
        }
    }))
    .await;
    ctx.expect_result(417, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 418,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "globalThis.__lm_binding_type = typeof createWorldBinding; createWorldBinding('from-create-world');",
            "worldName": "utility-create"
        }
    }))
    .await;
    ctx.expect_result(418, json!({ "identifier": "1" }), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 419,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility-create"
        }
    }))
    .await;
    let create_response = take_response_by_id(&mut ctx, 419);
    let context_id = create_response["result"]["executionContextId"]
        .as_i64()
        .expect("top-level isolated execution context id");

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("createWorldBinding")
        })
        .cloned()
        .expect("document-start script should be able to call the matching binding");
    assert_eq!(
        binding_called["params"]["payload"],
        json!("from-create-world")
    );
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(context_id)
    );

    ctx.process_async(json!({
        "id": 420,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": context_id,
            "expression": "JSON.stringify([globalThis.__lm_binding_type, typeof createWorldBinding])"
        }
    }))
    .await;
    let eval_result = take_response_by_id(&mut ctx, 420);
    assert_eq!(
        eval_result["result"]["result"]["value"],
        json!("[\"function\",\"function\"]")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn add_script_to_evaluate_on_new_document_default_world_replays_into_runtime_materialized_child_frame()
 {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>top</body>",
        Some("SID-1"),
    )
    .await;

    ctx.process_async(json!({
        "id": 417,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "window.__lm_child_auto_preload = document.body.textContent.trim();"
        }
    }))
    .await;
    ctx.expect_result(417, json!({ "identifier": "1" }), Some("SID-1"));

    ctx.process_async(json!({
        "id": 418,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
                const iframe = document.createElement('iframe');
                iframe.id = 'child-default-preload';
                iframe.srcdoc = '<body>child-preload</body>';
                document.body.appendChild(iframe);
                return true;
            })()"#
        }
    }))
    .await;
    let materialize = take_response_by_id(&mut ctx, 418);
    assert_eq!(materialize["result"]["result"]["value"], json!(true));
    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 4181).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;

    ctx.process_async(json!({
        "id": 419,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "document.getElementById('child-default-preload').contentWindow.__lm_child_auto_preload"
        }
    })).await;
    let child_result = take_response_by_id(&mut ctx, 419);
    assert_eq!(
        child_result["result"]["result"]["value"],
        json!("child-preload")
    );

    ctx.process_async(json!({
        "id": 420,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "typeof globalThis.__lm_child_auto_preload"
        }
    }))
    .await;
    let top_result = take_response_by_id(&mut ctx, 420);
    assert_eq!(top_result["result"]["result"]["value"], json!("undefined"));
}

#[tokio::test(flavor = "multi_thread")]
async fn add_script_to_evaluate_on_new_document_handles_null_prototype_object_in_child_frame_replay()
 {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>top</body>",
        Some("SID-1"),
    )
    .await;

    ctx.process_async(json!({
        "id": 421,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": r#"(() => {
                const value = Object.create(null);
                value.marker = 'from-null-prototype-preload';
                window.__lm_null_proto_preload = value;
            })();"#
        }
    }))
    .await;
    ctx.expect_result(421, json!({ "identifier": "1" }), Some("SID-1"));

    ctx.process_async(json!({
        "id": 422,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
                const iframe = document.createElement('iframe');
                iframe.id = 'child-null-prototype-preload';
                iframe.srcdoc = '<body>child-null-prototype</body>';
                document.body.appendChild(iframe);
                return true;
            })()"#
        }
    }))
    .await;
    let materialize = take_response_by_id(&mut ctx, 422);
    assert_eq!(materialize["result"]["result"]["value"], json!(true));
    let child_frame_id = child_frame_id_for_single_iframe(&mut ctx, 4221).await;
    wait_until_frame_stopped_loading(&mut ctx, &child_frame_id).await;

    ctx.process_async(json!({
        "id": 423,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"(() => {
                const child = document.getElementById('child-null-prototype-preload').contentWindow;
                const childValue = child.__lm_null_proto_preload;
                return JSON.stringify({
                    topType: typeof window.__lm_null_proto_preload,
                    childMarker: childValue && childValue.marker,
                    childPrototypeIsNull: child.Object.getPrototypeOf(childValue) === null
                });
            })()"#
        }
    }))
    .await;
    let child_result = take_response_by_id(&mut ctx, 423);
    assert_eq!(
        child_result["result"]["result"]["value"],
        json!(
            r#"{"topType":"undefined","childMarker":"from-null-prototype-preload","childPrototypeIsNull":true}"#
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_validates_params() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");

    ctx.process_async(json!({
        "id": 44,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1"
        }
    }))
    .await;
    ctx.expect_error(44, -32602, "InvalidParams");
}
#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_without_runtime_frontend_enabled_only_returns_result() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>hello</body>")
        .await
        .expect("page should load");
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .set_loaded_page_for_test(page);

    ctx.process_async(json!({
        "id": 45,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility",
            "grantUniveralAccess": true
        }
    }))
    .await;

    let result = ctx.take_one();
    let execution_context_id = result["result"]["executionContextId"]
        .as_i64()
        .expect("executionContextId");
    assert!(execution_context_id > 0);
    assert_eq!(result["id"], 45);
    assert_eq!(result["sessionId"], "SID-1");
    assert!(ctx.sent.is_empty());
}
#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_accepts_cdp_and_corrected_grant_universal_access_spellings() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>hello</body>")
        .await
        .expect("page should load");
    let bc = ctx.conn.browser_context.as_mut().expect("browser context");
    let _ = bc
        .active_page_target_mut()
        .runtime_slot
        .replace_loaded_page(Some(page));

    ctx.process_async(json!({
        "id": 47,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 47);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 48,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "cdp-spelling",
            "grantUniveralAccess": true
        }
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(
        created["params"]["context"]["auxData"]["grantUniversalAccess"],
        true
    );
    ctx.expect_result(
        48,
        json!({
            "executionContextId": created["params"]["context"]["id"]
        }),
        Some("SID-1"),
    );

    ctx.process_async(json!({
        "id": 49,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "corrected-spelling",
            "grantUniversalAccess": true
        }
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(
        created["params"]["context"]["auxData"]["grantUniversalAccess"],
        true
    );
    ctx.expect_result(
        49,
        json!({
            "executionContextId": created["params"]["context"]["id"]
        }),
        Some("SID-1"),
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_returns_unique_context_ids_and_emits_runtime_event() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>hello</body>")
        .await
        .expect("page should load");
    let bc = ctx.conn.browser_context.as_mut().expect("browser context");
    let _ = bc
        .active_page_target_mut()
        .runtime_slot
        .replace_loaded_page(Some(page));
    bc.set_target_security_origin("https://stale-top-origin.example".into());

    ctx.process_async(json!({
        "id": 41,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 41);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 42,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility"
        }
    }))
    .await;

    let created = ctx.take_one();
    assert_eq!(created["method"], "Runtime.executionContextCreated");
    assert_eq!(created["sessionId"], "SID-1");
    assert_eq!(created["params"]["context"]["name"], "utility");
    assert_eq!(created["params"]["context"]["origin"], "");
    assert_eq!(created["params"]["context"]["auxData"]["isDefault"], false);
    assert_eq!(created["params"]["context"]["auxData"]["type"], "isolated");
    assert_eq!(created["params"]["context"]["auxData"]["frameId"], "TID-1");
    assert!(
        created["params"]["context"]["uniqueId"].as_str().is_some(),
        "Page.createIsolatedWorld Runtime event should come from V8 native batch: {created:?}"
    );
    let first_id = created["params"]["context"]["id"]
        .as_i64()
        .expect("execution context id");
    assert!(first_id > 0);
    assert_eq!(
        ctx.conn
            .inspector_execution_context_id_for_isolated_context_for_session_owner_async(
                Some("SID-1"),
                first_id
            )
            .await
            .expect("isolated context should be known to renderer inspector"),
        Some(first_id),
        "Page.createIsolatedWorld should expose the renderer V8 inspector context id directly"
    );

    ctx.expect_result(42, json!({ "executionContextId": first_id }), Some("SID-1"));

    ctx.process_async(json!({
        "id": 43,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility-2",
            "grantUniversalAccess": true
        }
    }))
    .await;

    let created = ctx.take_one();
    let second_id = created["params"]["context"]["id"]
        .as_i64()
        .expect("execution context id");
    assert!(
        created["params"]["context"]["uniqueId"].as_str().is_some(),
        "second Page.createIsolatedWorld Runtime event should come from V8 native batch: {created:?}"
    );
    assert_ne!(first_id, second_id);
    assert_eq!(
        created["params"]["context"]["auxData"]["grantUniversalAccess"],
        true
    );
    ctx.expect_result(
        43,
        json!({ "executionContextId": second_id }),
        Some("SID-1"),
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_targets_loaded_background_owner_without_activation() {
    let mut ctx = TestContext::new();
    let background = PageTargetHost::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );

    let mut bc = BrowserContext::new("BID-1".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.insert_page_target_host(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<body>background</body>",
        Some("SID-background"),
    )
    .await;
    ctx.sent.clear();
    assert_eq!(
        ctx.conn
            .target_owner_identity_for_session(Some("SID-background")),
        Some(("BID-1".to_owned(), Some("TID-background".to_owned())))
    );

    ctx.process_async(json!({
        "id": 429,
        "method": "Runtime.enable",
        "sessionId": "SID-background"
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 429);
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 430,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-background",
        "params": {
            "frameId": "TID-background",
            "worldName": "utility"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 430);
    let isolated_context_id = response["result"]["executionContextId"]
        .as_i64()
        .unwrap_or_else(|| panic!("isolated execution context id: {response:?}"));
    let created = ctx
        .sent
        .iter()
        .find(|message| {
            message["sessionId"] == json!("SID-background")
                && message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["id"] == json!(isolated_context_id)
                && message["params"]["context"]["name"] == json!("utility")
        })
        .unwrap_or_else(|| {
            panic!("runtime-enabled background owner should receive the isolated context event")
        });
    assert!(
        created["params"]["context"]["uniqueId"].as_str().is_some(),
        "background Page.createIsolatedWorld event should come from V8 native batch: {created:?}"
    );
    assert_eq!(
        ctx.conn
            .browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.active_target_id()),
        Some("TID-active"),
        "background Page.createIsolatedWorld should not activate the target"
    );
    ctx.sent.clear();
    ctx.process_async(json!({
        "id": 431,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "contextId": isolated_context_id,
            "expression": "globalThis.__backgroundUtility = 'ready'; globalThis.__backgroundUtility",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 431);
    assert_eq!(response["result"]["result"]["value"], json!("ready"));

    ctx.process_async(json!({
        "id": 432,
        "method": "Runtime.evaluate",
        "sessionId": "SID-background",
        "params": {
            "expression": "typeof globalThis.__backgroundUtility",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 432);
    assert_eq!(response["result"]["result"]["value"], json!("undefined"));
}
#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_does_not_persist_across_navigation() {
    let mut ctx = TestContext::new();
    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>hello</body>")
        .await
        .expect("page should load");
    let bc = ctx.conn.browser_context.as_mut().expect("browser context");
    let _ = bc
        .active_page_target_mut()
        .runtime_slot
        .replace_loaded_page(Some(page));
    ctx.process_async(json!({
        "id": 45,
        "method": "Runtime.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(45, json!({}), Some("SID-1"));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 46,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility"
        }
    }))
    .await;
    let _ = ctx.take_all();

    ctx.process_async(json!({
        "id": 47,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>next</body>" }
    }))
    .await;

    let sent = ctx.take_all();
    assert!(
        sent.iter().any(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["auxData"]["isDefault"] == json!(true)
        }),
        "navigation should create the new default context: {sent:?}"
    );
    assert!(
        sent.iter().all(|message| {
            message["method"] != json!("Runtime.executionContextCreated")
                || message["params"]["context"]["name"] != json!("utility")
        }),
        "bare Page.createIsolatedWorld world must not be recreated after navigation: {sent:?}"
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn create_isolated_world_after_reactivating_browser_context_with_another_loaded_page() {
    let mut ctx = TestContext::new();

    load_bc_with_session(&mut ctx, "BID-1", "TID-1", "SID-1", "about:blank");
    let first_page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>first</body>")
        .await
        .expect("first page should load");
    {
        let bc = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("first browser context");
        let _ = bc
            .active_page_target_mut()
            .runtime_slot
            .replace_loaded_page(Some(first_page));
        bc.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = false;
    }

    let mut second = BrowserContext::new("BID-2".into());
    second.set_active_target_id("TID-2");
    second.attach_active_session("SID-2");
    second.set_target_url("about:blank".into());
    ctx.conn.insert_browser_context(second);

    assert!(ctx.conn.activate_browser_context_by_id_async("BID-2").await);
    let second_page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<body>second</body>")
        .await
        .expect("second page should load");
    {
        let bc = ctx
            .conn
            .browser_context_by_id_mut("BID-2")
            .expect("second browser context");
        let _ = bc
            .active_page_target_mut()
            .runtime_slot
            .replace_loaded_page(Some(second_page));
        bc.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = false;
    }

    assert!(ctx.conn.activate_browser_context_by_id_async("BID-1").await);
    ctx.process_async(json!({
        "id": 47_001,
        "method": "Page.createIsolatedWorld",
        "sessionId": "SID-1",
        "params": {
            "frameId": "TID-1",
            "worldName": "utility-after-reactivation"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 47_001);
    assert_eq!(response["sessionId"], json!("SID-1"));
    let isolated_context_id = response["result"]["executionContextId"]
        .as_i64()
        .unwrap_or_default();
    assert!(isolated_context_id > 0);
}
