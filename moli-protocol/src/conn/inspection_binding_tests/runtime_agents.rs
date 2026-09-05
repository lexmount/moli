use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn runtime_controls_start_without_protocol_document() {
    let mut ctx = dom_context().await;
    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    let document = ctx
        .conn
        .runtime_session_owner_slot_mut_for_owner(&owner)
        .unwrap()
        .page_slot_mut()
        .contents
        .main_frame
        .current_document
        .take()
        .unwrap();
    for (id, method) in (1..).zip([
        "Runtime.enable",
        "Runtime.disable",
        "Runtime.runIfWaitingForDebugger",
        "Runtime.discardConsoleEntries",
        "HeapProfiler.enable",
        "HeapProfiler.collectGarbage",
        "HeapProfiler.disable",
    ]) {
        let step = ctx
            .conn
            .start_command_dispatch(&json!({"id": id, "method": method}).to_string());
        assert!(
            matches!(&step, CdpCommandTaskStep::Pending(_)),
            "{method} must enter its renderer binding"
        );
        let (mut messages, _) = ctx.complete_command_task_step_for_test(step).await;
        messages.extend(ctx.take_all());
        let response = match messages
            .into_iter()
            .find(|message| message["id"] == json!(id))
        {
            Some(response) => response,
            // collectGarbage has AsyncCallback completion. Command admission
            // is not its response: wait for the exact real scheduler input.
            None => {
                ctx.wait_for_scheduler_message(method, |message| message["id"] == json!(id))
                    .await
            }
        };
        assert!(response.get("error").is_none(), "{method}: {response}");
    }
    assert!(
        !ctx.conn
            .runtime_session_owner_slot_for_owner(&owner)
            .unwrap()
            .has_loaded_page()
    );
    drop(document);
}

#[tokio::test(flavor = "multi_thread")]
async fn preloads_register_run_remove_and_replay_without_protocol_document() {
    let mut ctx = dom_context().await;
    dom_command(&mut ctx, 1, "Runtime.enable", json!({})).await;
    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    let document = ctx
        .conn
        .runtime_session_owner_slot_mut_for_owner(&owner)
        .unwrap()
        .page_slot_mut()
        .contents
        .main_frame
        .current_document
        .take()
        .unwrap();
    dom_command(
        &mut ctx,
        2,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({
            "source": "document.getElementById('inspected').setAttribute('data-preload', 'ran');",
            "runImmediately": true,
        }),
    )
    .await;
    let ran = dom_command(
        &mut ctx,
        3,
        "Runtime.evaluate",
        json!({
            "expression": "document.getElementById('inspected').getAttribute('data-preload')",
        }),
    )
    .await;
    assert_eq!(ran["result"]["value"], json!("ran"));

    for (id, world, remove) in [(10, "kept-world", false), (20, "removed-world", true)] {
        let registered = dom_command(
            &mut ctx,
            id,
            "Page.addScriptToEvaluateOnNewDocument",
            json!({
                "source": "globalThis.preloadMarker = 'installed';", "worldName": world,
            }),
        )
        .await;
        if remove {
            dom_command(
                &mut ctx,
                id + 1,
                "Page.removeScriptToEvaluateOnNewDocument",
                json!({
                    "identifier": registered["identifier"],
                }),
            )
            .await;
        }
        let created = dom_command(
            &mut ctx,
            id + 2,
            "Page.createIsolatedWorld",
            json!({
                "frameId": "TID-dom-inspection", "worldName": world,
            }),
        )
        .await;
        let marker = dom_command(
            &mut ctx,
            id + 3,
            "Runtime.evaluate",
            json!({
                "contextId": created["executionContextId"], "expression": "typeof preloadMarker",
            }),
        )
        .await;
        assert_eq!(
            marker["result"]["value"],
            json!(if remove { "undefined" } else { "string" }),
            "realm creation must consume the renderer's updated preload registry"
        );
    }
    assert!(
        !ctx.conn
            .runtime_session_owner_slot_for_owner(&owner)
            .unwrap()
            .has_loaded_page()
    );
    drop(document);
}

#[tokio::test(flavor = "multi_thread")]
async fn isolated_world_restarts_on_replacement_binding_without_protocol_document() {
    let mut ctx = dom_context().await;
    dom_command(&mut ctx, 1, "Runtime.enable", json!({})).await;
    let step = ctx.conn.start_command_dispatch(
        &json!({"id": 2, "method": "Page.createIsolatedWorld",
        "params": {"frameId": "TID-dom-inspection", "worldName": "rebound-world"}})
        .to_string(),
    );
    assert!(matches!(&step, CdpCommandTaskStep::Pending(_)));
    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    let old_document = ctx
        .conn
        .runtime_session_owner_slot_mut_for_owner(&owner)
        .unwrap()
        .page_slot_mut()
        .contents
        .main_frame
        .current_document
        .take()
        .unwrap();
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<main id='inspected'>replacement</main>",
        None,
    )
    .await;
    let replacement = ctx
        .conn
        .runtime_session_owner_slot_mut_for_owner(&owner)
        .unwrap()
        .page_slot_mut()
        .contents
        .main_frame
        .current_document
        .take()
        .unwrap();
    let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(2))
        .unwrap();
    assert!(response.get("error").is_none(), "{response}");
    let value = dom_command(
        &mut ctx,
        3,
        "Runtime.evaluate",
        json!({
            "contextId": response["result"]["executionContextId"],
            "expression": "document.getElementById('inspected').textContent",
        }),
    )
    .await;
    assert_eq!(value["result"]["value"], json!("replacement"));
    drop((old_document, replacement));
}

#[tokio::test(flavor = "multi_thread")]
async fn session_preload_cleanup_updates_renderer_without_protocol_document() {
    let mut ctx = dom_context().await;
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-preload-cleanup");
    ctx.conn.commit_declared_session_fixtures_for_test();
    ctx.process_async(json!({"id": 1, "method": "Page.addScriptToEvaluateOnNewDocument", "sessionId": "SID-preload-cleanup", "params": {
        "source": "globalThis.detachedPreload = true;", "worldName": "detached-preload-world",
    }})).await;
    ctx.expect_result(1, json!({"identifier": "1"}), Some("SID-preload-cleanup"));
    let owner = CommandOwnerScope::capture(&ctx.conn, Some("SID-preload-cleanup"));
    let document = ctx
        .conn
        .runtime_session_owner_slot_mut_for_owner(&owner)
        .unwrap()
        .page_slot_mut()
        .contents
        .main_frame
        .current_document
        .take()
        .unwrap();
    ctx.process_async(
        json!({"id": 10, "method": "Target.detachFromTarget", "params": {
            "targetId": "TID-dom-inspection", "sessionId": "SID-preload-cleanup",
        }}),
    )
    .await;
    ctx.expect_result(10, json!({}), None);
    // Reusing the primary renderer session key must not resurrect its old
    // preload, even while the Browser Document is held outside Protocol.
    ctx.conn
        .browser_context
        .as_mut()
        .unwrap()
        .attach_active_session("SID-preload-cleanup");
    ctx.conn.commit_declared_session_fixtures_for_test();
    ctx.process_async(json!({"id": 2, "method": "Page.createIsolatedWorld", "sessionId": "SID-preload-cleanup", "params": {
        "frameId": "TID-dom-inspection", "worldName": "detached-preload-world",
    }})).await;
    let created = ctx.take_response_by_id(2);
    assert!(created.get("error").is_none(), "{created}");
    ctx.process_async(json!({"id": 3, "method": "Runtime.evaluate", "sessionId": "SID-preload-cleanup", "params": {
        "contextId": created["result"]["executionContextId"], "expression": "typeof detachedPreload",
    }})).await;
    let result = ctx.take_response_by_id(3);
    assert_eq!(
        result["result"]["result"]["value"],
        json!("undefined"),
        "cleanup must update the renderer registry, not only service metadata"
    );
    drop(document);
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_binding_completion_rejects_replacement_attachment() {
    let mut ctx = dom_context().await;
    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    let pending = ctx
        .conn
        .start_install_runtime_binding_for_owner(&owner, "retiredBinding", None, None)
        .unwrap();
    let completed = pending.wait().await.unwrap();
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<main>replacement</main>",
        None,
    )
    .await;
    assert_eq!(
        ctx.conn
            .complete_runtime_binding_page_command(completed)
            .unwrap_err(),
        "Renderer attachment changed"
    );
    let value = dom_command(
        &mut ctx,
        1,
        "Runtime.evaluate",
        json!({"expression": "typeof retiredBinding"}),
    )
    .await;
    assert_eq!(value["result"]["value"], json!("undefined"));
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_bindings_start_without_protocol_document() {
    runtime_agent_round_trip("Runtime.addBinding", false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_debugger_completion_cannot_register_breakpoints_after_rebind() {
    use crate::conn::{Cmd, ParsedCdpCommand};
    use crate::domains::dom_debugger::{
        DomDebuggerCommandTaskStep, complete_pending_dom_debugger_command,
        try_start_dom_debugger_command_dispatch,
    };
    let mut ctx = dom_context().await;
    let raw =
        json!({"id": 1, "method": "DOMDebugger.setXHRBreakpoint", "params": {"url": "/retired"}})
            .to_string();
    let parsed = ParsedCdpCommand::parse_str(&raw).unwrap();
    let cmd = Cmd::from_parsed(&parsed).unwrap();
    let DomDebuggerCommandTaskStep::Pending(pending) =
        try_start_dom_debugger_command_dispatch(&mut ctx.conn, &cmd)
    else {
        panic!("breakpoint must start on the original renderer");
    };
    let completed = pending.wait().await;
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<main>replacement</main>",
        None,
    )
    .await;
    let plan = complete_pending_dom_debugger_command(&mut ctx.conn, completed);
    let messages = plan
        .into_background_events(Some(1), None)
        .into_iter()
        .map(|event| event.into_protocol_message())
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![json!({"id": 1, "error": {"code": -32000, "message": "Renderer attachment changed"}})]
    );
    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    assert!(
        ctx.conn
            .target_devtools_session_state_for_owner(&owner)
            .unwrap()
            .dom_debugger_xhr_breakpoints
            .is_empty(),
        "retired completion must not add replay state to the replacement's session"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_bindings_complete_without_protocol_document() {
    runtime_agent_round_trip("Runtime.addBinding", true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn isolated_world_starts_without_protocol_document() {
    runtime_agent_round_trip("Page.createIsolatedWorld", false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn isolated_world_completes_without_protocol_document() {
    runtime_agent_round_trip("Page.createIsolatedWorld", true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_debugger_starts_without_protocol_document() {
    runtime_agent_round_trip("DOMDebugger.getEventListeners", false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dom_debugger_completes_without_protocol_document() {
    runtime_agent_round_trip("DOMDebugger.getEventListeners", true).await;
}

async fn runtime_agent_round_trip(method: &str, start_before_move: bool) {
    let mut ctx = dom_context().await;
    dom_command(&mut ctx, 1, "Runtime.enable", json!({})).await;
    let object = dom_command(&mut ctx, 2, "Runtime.evaluate", json!({
        "expression": "(() => { const node = document.getElementById('inspected'); node.addEventListener('click', () => {}); return node; })()",
    })).await["result"]["objectId"].clone();
    assert!(object.is_string());
    let owner = CommandOwnerScope::capture(&ctx.conn, None);
    let take_document = |ctx: &mut TestContext| {
        ctx.conn
            .runtime_session_owner_slot_mut_for_owner(&owner)
            .unwrap()
            .page_slot_mut()
            .contents
            .main_frame
            .current_document
            .take()
            .unwrap()
    };
    let mut document = (!start_before_move).then(|| take_document(&mut ctx));
    let params = match method {
        "Runtime.addBinding" => json!({"name": "inspectBinding"}),
        "Page.createIsolatedWorld" => {
            json!({"frameId": "TID-dom-inspection", "worldName": "inspection-world"})
        }
        "DOMDebugger.getEventListeners" => json!({"objectId": object}),
        _ => unreachable!(),
    };
    let step = ctx
        .conn
        .start_command_dispatch(&json!({"id": 3, "method": method, "params": params}).to_string());
    assert!(
        matches!(&step, CdpCommandTaskStep::Pending(_)),
        "{method} must dispatch to the live binding"
    );
    if start_before_move {
        document = Some(take_document(&mut ctx));
    }
    let document = document.unwrap();
    let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(3))
        .unwrap();
    assert!(
        response.get("error").is_none(),
        "{method} completion: {response}"
    );

    match method {
        "Runtime.addBinding" => {
            let called = dom_exchange(
                &mut ctx,
                4,
                "Runtime.evaluate",
                json!({"expression": "inspectBinding('bound-payload'); 42"}),
            )
            .await;
            assert!(
                called.iter().any(
                    |message| message["method"] == json!("Runtime.bindingCalled")
                        && message["params"]["name"] == json!("inspectBinding")
                        && message["params"]["payload"] == json!("bound-payload")
                ),
                "binding output must reach the frontend without a Protocol Document: {called:?}"
            );
            // The stored-binding phase is also used after realm creation and replay.
            // A successful raw Inspector response must not conceal a skipped phase.
            let pending = ctx
                .conn
                .start_apply_stored_runtime_bindings_for_owner(&owner)
                .expect("stored binding replay must start through the same live binding");
            let completed = pending.wait().await;
            ctx.conn
                .complete_runtime_binding_page_command(completed.unwrap())
                .unwrap();
            dom_command(
                &mut ctx,
                5,
                "Runtime.removeBinding",
                json!({"name": "inspectBinding"}),
            )
            .await;
            let after = dom_exchange(
                &mut ctx,
                6,
                "Runtime.evaluate",
                json!({"expression": "inspectBinding('after-remove'); typeof inspectBinding"}),
            )
            .await;
            assert!(!after.iter().any(|message| message["method"]
                == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("inspectBinding")));
            assert_eq!(
                after
                    .iter()
                    .find(|message| message["id"] == json!(6))
                    .unwrap()["result"]["result"]["value"],
                json!("function")
            );
        }
        "Page.createIsolatedWorld" => {
            let context_id = response["result"]["executionContextId"].as_i64().unwrap();
            let value = dom_command(&mut ctx, 4, "Runtime.evaluate", json!({"contextId": context_id, "expression": "document.getElementById('inspected').textContent"})).await;
            assert_eq!(value["result"]["value"], json!("value"));
            let realms = ctx
                .conn
                .runtime_realm_inventory_for_owner_async(&owner)
                .await
                .expect("realm inventory must not borrow Page");
            assert!(!realms.is_empty());
            assert!(
                ctx.conn
                    .has_isolated_execution_context_id_for_session_owner_async(None, context_id)
                    .await
                    .unwrap()
            );
        }
        "DOMDebugger.getEventListeners" => {
            assert!(
                response["result"]["listeners"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|listener| listener["type"] == json!("click"))
            );
            dom_command(
                &mut ctx,
                4,
                "DOMDebugger.setXHRBreakpoint",
                json!({"url": "/inspected"}),
            )
            .await;
            dom_command(
                &mut ctx,
                5,
                "DOMDebugger.removeXHRBreakpoint",
                json!({"url": "/inspected"}),
            )
            .await;
            let again = dom_command(
                &mut ctx,
                6,
                "DOMDebugger.getEventListeners",
                json!({"objectId": object}),
            )
            .await;
            assert_eq!(
                again["listeners"].as_array().unwrap().len(),
                response["result"]["listeners"].as_array().unwrap().len()
            );
        }
        _ => unreachable!(),
    }
    assert!(
        !ctx.conn
            .runtime_session_owner_slot_for_owner(&owner)
            .unwrap()
            .has_loaded_page()
    );
    drop(document);
}
