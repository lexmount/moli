use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_session_can_stage_its_own_runtime_enable_before_activation() {
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-PRE-RUNTIME",
        "TID-000000000PR",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 104194936,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(104194936, json!({}), None);

    ctx.process_async(json!({
        "id": 104194937,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-PRE-RUNTIME", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(104194937, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 104194938,
        "method": "Runtime.enable",
        "sessionId": second_session_id
    }))
    .await;
    ctx.expect_result(104194938, json!({}), Some(&second_session_id));
    take_staged_about_blank_runtime_context(&mut ctx, &second_session_id, &second_target_id);

    {
        let active = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("active browser context");
        assert!(
            !active.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled
        );
        let staged = active
            .background_target(&second_target_id)
            .filter(|target| target.has_non_default_session_state())
            .expect("second target should have staged background page session state");
        assert!(
            staged.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled
        );
    }

    ctx.process_async(json!({
        "id": 104194939,
        "method": "Page.navigate",
        "sessionId": "SID-active",
        "params": {
            "url": "data:text/html,<title>page-a</title><div id='ok'>page a</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 104194939);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during active navigation: {:?}",
        ctx.sent
    );
    assert!(
        !ctx.sent.iter().any(|message| {
            message["sessionId"] == json!("SID-active") && is_runtime_context_event(message)
        }),
        "active target should not emit runtime context events before activation: {:?}",
        ctx.sent
    );
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949391,
        "method": "Target.closeTarget",
        "params": {"targetId": "TID-000000000PR"}
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949391);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949392,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>page-b</title><div id='ok'>page b</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949392);
    assert!(
        ctx.sent
            .iter()
            .all(|message| message.get("error").is_none()),
        "unexpected protocol error during activated navigation: {:?}",
        ctx.sent
    );

    let runtime_events = vec![
        ctx.wait_for_scheduler_message("activated Runtime context reset", |message| {
            message["sessionId"] == json!(second_session_id)
                && message["method"] == json!("Runtime.executionContextsCleared")
        })
        .await,
        ctx.wait_for_scheduler_message("activated Runtime default context", |message| {
            message["sessionId"] == json!(second_session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
        })
        .await,
    ];
    assert_eq!(
        runtime_events.len(),
        2,
        "staged Runtime.enable should use the owner-safe native path after activation: {runtime_events:?}"
    );
    assert_eq!(
        runtime_events[0]["method"],
        "Runtime.executionContextsCleared"
    );
    assert_eq!(
        runtime_events[1]["method"],
        "Runtime.executionContextCreated"
    );
    assert_eq!(
        runtime_events[1]["params"]["context"]["name"],
        json!("data:text/html,<title>page-b</title><div id='ok'>page b</div>")
    );
    assert_eq!(
        runtime_events[1]["params"]["context"]["auxData"]["frameId"],
        json!(second_target_id)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_loaded_background_session_runtime_enable_replays_context_without_activation()
{
    let mut ctx = TestContext::new();
    load_bc_with_titled_page_async(
        &mut ctx,
        "BID-9-RUNTIME-DIRECT",
        "TID-000000000RDA",
        "<title>active</title><div id='ok'>active target</div>",
    )
    .await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-active");

    ctx.process_async(json!({
        "id": 1041949310,
        "method": "Target.setAutoAttach",
        "params": { "autoAttach": true, "waitForDebuggerOnStart": false }
    }))
    .await;
    ctx.expect_result(1041949310, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949311,
        "method": "Target.createTarget",
        "params": {
            "background": true, "browserContextId": "BID-9-RUNTIME-DIRECT", "url": "about:blank#second"}
    }))
    .await;
    let created = ctx.take_one();
    assert_eq!(created["method"], "Target.targetCreated");
    let second_target_id = created["params"]["targetInfo"]["targetId"]
        .as_str()
        .expect("second target id")
        .to_owned();
    let attached = ctx.take_one();
    assert_eq!(attached["method"], "Target.attachedToTarget");
    let second_session_id = attached["params"]["sessionId"]
        .as_str()
        .expect("second target session id")
        .to_owned();
    ctx.expect_result(1041949311, json!({ "targetId": second_target_id }), None);

    ctx.process_async(json!({
        "id": 1041949312,
        "method": "Target.activateTarget",
        "params": { "targetId": second_target_id }
    }))
    .await;
    ctx.expect_result(1041949312, json!({}), None);

    ctx.process_async(json!({
        "id": 1041949313,
        "method": "Page.navigate",
        "sessionId": second_session_id,
        "params": {
            "url": "data:text/html,<title>second</title><div id='ok'>second target</div>"
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 1041949313);
    ctx.take_all();

    ctx.process_async(json!({
        "id": 1041949314,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000RDA" }
    }))
    .await;
    ctx.expect_result(1041949314, json!({}), None);
    ctx.take_all();

    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(bc.active_target_id(), Some("TID-000000000RDA"));
        assert!(
            bc.background_target(&second_target_id)
                .is_some_and(|target| target.has_loaded_page()),
            "second target should be background with a loaded page before Runtime.enable"
        );
    }

    ctx.process_async(json!({
        "id": 1041949315,
        "method": "Runtime.enable",
        "sessionId": second_session_id
    }))
    .await;
    let messages = ctx.take_all();
    assert!(
        messages.iter().any(|message| {
            message["id"] == json!(1041949315)
                && message["result"] == json!({})
                && message["sessionId"] == json!(second_session_id)
        }),
        "Runtime.enable should return a session-scoped success response: {messages:?}"
    );
    let context_events = messages
        .iter()
        .filter(|message| {
            message["sessionId"] == json!(second_session_id)
                && message["method"] == json!("Runtime.executionContextCreated")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        context_events.len(),
        1,
        "loaded background Runtime.enable should replay exactly one default context event: {messages:?}"
    );
    assert_eq!(
        context_events[0]["params"]["context"]["auxData"]["frameId"],
        json!(second_target_id)
    );

    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(
            bc.active_target_id(),
            Some("TID-000000000RDA"),
            "direct Runtime.enable should not activate the loaded background target"
        );
        assert!(
            bc.background_target(&second_target_id)
                .filter(|target| target.has_non_default_session_state())
                .is_some_and(|state| state.devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .runtime_session_state
                    .runtime_frontend_enabled),
            "Runtime.enable should be staged on the background target owner"
        );
        assert!(
            bc.background_target(&second_target_id)
                .is_some_and(|target| target.has_loaded_page()),
            "direct Runtime.enable should leave the loaded page background"
        );
    }
}

pub(super) struct LoadedBackgroundRuntimeOwner {
    pub(super) target_id: String,
    pub(super) session_id: String,
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_loaded_background_session_runtime_evaluate_reads_owner_page_without_activation()
 {
    let mut ctx = TestContext::new();
    let owner = load_same_context_loaded_background_runtime_owner_async(
        &mut ctx,
        "BID-9-RUNTIME-EVAL",
        "TID-000000000REA",
        "<title>active</title><div id='ok'>active target</div>",
        "data:text/html,<title>second</title><div id='ok'>second target</div>",
        1041949410,
    )
    .await;

    ctx.process_async(json!({
        "id": 1041949416,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "document.title",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949416);
    assert_eq!(response["sessionId"], json!(owner.session_id));
    assert_eq!(response["result"]["result"]["value"], json!("second"));

    ctx.process_async(json!({
        "id": 1041949417,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "document.querySelector('#ok')"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949417);
    assert_eq!(response["sessionId"], json!(owner.session_id));
    assert_eq!(response["result"]["result"]["subtype"], json!("node"));

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        bc.active_target_id(),
        Some("TID-000000000REA"),
        "direct Runtime.evaluate should not activate the loaded background target"
    );
    assert!(
        bc.background_target(&owner.target_id)
            .is_some_and(|target| target.has_loaded_page()),
        "direct Runtime.evaluate should leave the owner page background"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_loaded_background_window_open_self_navigates_owner_without_activation() {
    let mut ctx = TestContext::new();
    let owner = load_same_context_loaded_background_runtime_owner_async(
        &mut ctx,
        "BID-9-POPUP-SELF",
        "TID-000000000PSA",
        "<title>active</title><main>active target</main>",
        "data:text/html,<title>background</title><main>background target</main>",
        1041949430,
    )
    .await;

    ctx.process_async(json!({
        "id": 1041949436,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "window.open('data:text/html,<title>self</title><main>self target</main>', '_self') !== null"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949436);
    assert_eq!(response["result"]["result"]["type"], json!("boolean"));
    assert_eq!(response["result"]["result"]["value"], json!(true));
    // Chromium returns the Runtime.evaluate response before the `_self`
    // navigation starts. The renderer-owned navigation is a later task, so
    // wait for its exact owner action instead of strengthening the command
    // response into an implicit navigation barrier.
    ctx.wait_until_scheduler_state("background _self navigation owner action", |conn| {
        conn.browser_context
            .as_ref()
            .and_then(|browser_context| browser_context.background_target(&owner.target_id))
            .is_some_and(|target| {
                target.target_url() == "data:text/html,<title>self</title><main>self target</main>"
            })
    })
    .await;
    let emitted = ctx.take_all();
    assert!(
        !emitted
            .iter()
            .any(|message| message["method"] == json!("Target.targetCreated")),
        "_self popup must navigate the owner target instead of creating a popup target: {emitted:?}"
    );

    {
        let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(
            browser_context.active_target_id(),
            Some("TID-000000000PSA"),
            "background _self navigation must not activate the background target"
        );
        let background_target = browser_context
            .background_target(&owner.target_id)
            .expect("background target should remain background");
        assert_eq!(
            background_target.target_url(),
            "data:text/html,<title>self</title><main>self target</main>"
        );
        assert!(
            background_target.has_loaded_page(),
            "background _self navigation should replace the owner loaded page"
        );
    }

    ctx.process_async(json!({
        "id": 1041949437,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "document.querySelector('main')?.textContent"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949437);
    assert_eq!(response["result"]["result"]["value"], json!("self target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_named_popup_reuse_navigates_and_activates_loaded_owner() {
    let mut ctx = TestContext::new();
    tokio::task::LocalSet::new()
        .run_until(async {
    let active_target_id = "TID-000000000NPA";
    let owner = load_same_context_loaded_background_runtime_owner_async(
        &mut ctx,
        "BID-9-NAMED-POPUP",
        active_target_id,
        "<title>active</title><main>active target</main>",
        "data:text/html,<title>background</title><main>background target</main>",
        1041949440,
    )
    .await;
    ctx.enable_background_navigation_scheduler_for_test();
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .remember_target_opener(
            &owner.target_id,
            active_target_id.to_owned(),
            active_target_id.to_owned(),
            true,
        );
    ctx.conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .remember_target_window_name("reportWindow", &owner.target_id);

    ctx.process_async(json!({
        "id": 1041949446,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "window.open('data:text/html,<title>named</title><main>named target</main>', 'reportWindow') !== null"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949446);
    assert_eq!(response["result"]["result"]["type"], json!("boolean"));
    assert_eq!(response["result"]["result"]["value"], json!(true));
    ctx.wait_until_scheduler_state(
        "named popup navigation commit and foreground activation",
        |conn| {
            conn.browser_context_by_id("BID-9-NAMED-POPUP")
                .is_some_and(|browser_context| {
                    browser_context.active_target_id() == Some(owner.target_id.as_str())
                        && loaded_page_for_target(browser_context, &owner.target_id).is_some_and(
                            |page| {
                                page.final_url().as_str()
                                    == "data:text/html,<title>named</title><main>named target</main>"
                            },
                        )
                })
        },
    )
    .await;
    let emitted = ctx.take_all();
    assert!(
        !emitted
            .iter()
            .any(|message| message["method"] == json!("Target.targetCreated")),
        "reusing a loaded named target must not create a new popup target: {emitted:?}"
    );
    let changed = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Target.targetInfoChanged")
                && message["params"]["targetInfo"]["targetId"] == json!(owner.target_id)
                && message["params"]["targetInfo"]["url"]
                    == json!("data:text/html,<title>named</title><main>named target</main>")
        })
        .unwrap_or_else(|| {
            panic!("loaded named target reuse should report targetInfoChanged: {emitted:?}")
        });
    assert_eq!(
        changed["params"]["targetInfo"]["targetId"],
        json!(owner.target_id)
    );
    assert_eq!(
        changed["params"]["targetInfo"]["url"],
        json!("data:text/html,<title>named</title><main>named target</main>")
    );

    {
        let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(
            browser_context.active_target_id(),
            Some(owner.target_id.as_str()),
            "ordinary window.open should activate its reused named target"
        );
        assert_eq!(
            browser_context.target_url(),
            "data:text/html,<title>named</title><main>named target</main>"
        );
        assert!(
            browser_context.has_loaded_page(),
            "named popup reuse should replace the existing owner loaded page"
        );
        assert!(
            browser_context
                .background_target("TID-000000000NPA")
                .is_some(),
            "foreground named-target reuse should deactivate the previous active target"
        );
    }

    ctx.process_async(json!({
        "id": 1041949447,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "document.querySelector('main')?.textContent"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949447);
    assert_eq!(response["result"]["result"]["value"], json!("named target"));

    ctx.process_async(json!({
        "id": 1041949448,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "window.name"
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949448);
    assert_eq!(
        response["result"]["result"]["value"],
        json!("reportWindow"),
        "a reused named target must retain its browsing-context name across Document replacement"
    );
    let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        browser_context.target_id_for_window_name(active_target_id, "reportWindow"),
        Some(owner.target_id.as_str()),
        "the popup proxy name and the live target engine must share one browsing-context owner"
    );
        })
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_loaded_background_session_runtime_call_function_on_uses_owner_object_without_activation()
 {
    let mut ctx = TestContext::new();
    let owner = load_same_context_loaded_background_runtime_owner_async(
        &mut ctx,
        "BID-9-RUNTIME-CALL",
        "TID-000000000RCA",
        "<title>active</title><div id='ok'>active target</div>",
        "data:text/html,<title>second</title><div id='ok'>second target</div>",
        1041949510,
    )
    .await;

    ctx.process_async(json!({
        "id": 1041949516,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "document.querySelector('#ok')"
        }
    }))
    .await;
    let handle_response = take_response_by_id(&mut ctx, 1041949516);
    let object_id = handle_response["result"]["result"]["objectId"]
        .as_str()
        .expect("background node object id")
        .to_owned();

    ctx.process_async(json!({
        "id": 1041949517,
        "method": "Runtime.callFunctionOn",
        "sessionId": owner.session_id,
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { return this.textContent; }",
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949517);
    assert_eq!(response["sessionId"], json!(owner.session_id));
    assert_eq!(
        response["result"]["result"]["value"],
        json!("second target")
    );

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        bc.active_target_id(),
        Some("TID-000000000RCA"),
        "direct Runtime.callFunctionOn should not activate the loaded background target"
    );
    assert!(
        bc.background_target(&owner.target_id)
            .is_some_and(|target| target.has_loaded_page()),
        "direct Runtime.callFunctionOn should leave the owner page background"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_loaded_background_session_runtime_await_promise_uses_owner_without_activation()
 {
    let mut ctx = TestContext::new();
    let owner = load_same_context_loaded_background_runtime_owner_async(
        &mut ctx,
        "BID-9-RUNTIME-AWAIT",
        "TID-000000000RWA",
        "<title>active</title><div id='ok'>active target</div>",
        "data:text/html,<title>second</title><div id='ok'>second target</div>",
        1041949610,
    )
    .await;

    ctx.process_async(json!({
        "id": 1041949616,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "Promise.resolve(document.title)",
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949616);
    assert_eq!(response["sessionId"], json!(owner.session_id));
    assert_eq!(response["result"]["result"]["value"], json!("second"));

    ctx.process_async(json!({
        "id": 1041949617,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "document.querySelector('#ok')"
        }
    }))
    .await;
    let handle_response = take_response_by_id(&mut ctx, 1041949617);
    let object_id = handle_response["result"]["result"]["objectId"]
        .as_str()
        .expect("background node object id")
        .to_owned();

    ctx.process_async(json!({
        "id": 1041949618,
        "method": "Runtime.callFunctionOn",
        "sessionId": owner.session_id,
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { return Promise.resolve(this.textContent); }",
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    let response = take_response_by_id(&mut ctx, 1041949618);
    assert_eq!(response["sessionId"], json!(owner.session_id));
    assert_eq!(
        response["result"]["result"]["value"],
        json!("second target")
    );

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        bc.active_target_id(),
        Some("TID-000000000RWA"),
        "direct Runtime awaitPromise should not activate the loaded background target"
    );
    assert!(
        bc.background_target(&owner.target_id)
            .is_some_and(|target| target.has_loaded_page()),
        "direct Runtime awaitPromise should leave the owner page background"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn same_context_background_pending_await_survives_active_target_switch() {
    let mut ctx = TestContext::new();
    let owner = load_same_context_loaded_background_runtime_owner_async(
        &mut ctx,
        "BID-9-RUNTIME-AWAIT-SWITCH",
        "TID-000000000RAS",
        "<title>active</title><div id='ok'>active target</div>",
        "data:text/html,<title>second</title><div id='ok'>second target</div>",
        1041949650,
    )
    .await;

    ctx.process_async(json!({
        "id": 1041949656,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": r#"new Promise(resolve => {
  globalThis.__lmResolvePendingAwaitAfterSwitch = () => {
  globalThis.__lmPendingAwaitOwnerMarker = document.title;
  resolve(document.title + ':' + globalThis.__lmPendingAwaitOwnerMarker);
  return 'resolved:' + document.title;
  };
})"#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["id"] == json!(1041949656)),
        "timer-backed background awaitPromise should still be pending before active target switch: {:?}",
        ctx.sent
    );
    assert!(
        ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some(&owner.session_id)),
        "background awaitPromise should register against the background target session"
    );

    ctx.process_async(json!({
        "id": 1041949657,
        "method": "Target.activateTarget",
        "params": { "targetId": owner.target_id }
    }))
    .await;
    ctx.expect_result(1041949657, json!({}), None);
    let first_switch_messages = ctx.take_all();
    assert!(
        !first_switch_messages
            .iter()
            .any(|message| message["id"] == json!(1041949656)),
        "background awaitPromise must remain pending until the test resolves it explicitly: {:?}",
        first_switch_messages
    );

    ctx.process_async(json!({
        "id": 1041949658,
        "method": "Target.activateTarget",
        "params": { "targetId": "TID-000000000RAS" }
    }))
    .await;
    ctx.expect_result(1041949658, json!({}), None);
    let second_switch_messages = ctx.take_all();
    assert!(
        !second_switch_messages
            .iter()
            .any(|message| message["id"] == json!(1041949656)),
        "background awaitPromise must remain pending after active target switch: {:?}",
        second_switch_messages
    );

    ctx.process_async(json!({
        "id": 1041949660,
        "method": "Runtime.evaluate",
        "sessionId": owner.session_id,
        "params": {
            "expression": "globalThis.__lmResolvePendingAwaitAfterSwitch()",
            "returnByValue": true
        }
    }))
    .await;
    let resolve_response = take_response_by_id(&mut ctx, 1041949660);
    assert_eq!(resolve_response["sessionId"], json!(owner.session_id));
    assert_eq!(
        resolve_response["result"]["result"]["value"],
        json!("resolved:second")
    );

    crate::testing::wait_until_message(
        &mut ctx,
        Some(owner.session_id.as_str()),
        "background awaitPromise after active target switch",
        |message| message["id"] == json!(1041949656),
    )
    .await;
    let response = take_response_by_id(&mut ctx, 1041949656);
    assert_eq!(response["sessionId"], json!(owner.session_id));
    assert_eq!(
        response["result"]["result"]["value"],
        json!("second:second")
    );
    assert!(
        !ctx.conn
            .has_pending_inspector_awaits_for_session_owner(Some(&owner.session_id)),
        "settled background awaitPromise should clear the background target pending-await entry"
    );

    ctx.process_async(json!({
        "id": 1041949659,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "typeof globalThis.__lmPendingAwaitOwnerMarker",
            "returnByValue": true
        }
    }))
    .await;
    let active_response = take_response_by_id(&mut ctx, 1041949659);
    assert_eq!(
        active_response["result"]["result"]["value"],
        json!("undefined")
    );

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(
        bc.active_target_id(),
        Some("TID-000000000RAS"),
        "pending background awaitPromise completion must not activate the owner target"
    );
    assert!(
        bc.background_target(&owner.target_id)
            .is_some_and(|target| target.has_loaded_page()),
        "pending background awaitPromise completion should leave the owner page background"
    );
}
