// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

#[tokio::test]
async fn call_function_on_without_page_errors() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 2_1,
        "method": "Runtime.callFunctionOn",
        "params": {
            "functionDeclaration": "() => 1"
        }
    }))
    .await;
    ctx.expect_error(2_1, -32000, "NoDocumentLoaded");
}

#[tokio::test]
async fn evaluate_returns_number_value() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 30).await;

    ctx.process_async(json!({"id": 3, "method": "Runtime.evaluate",
                             "params": {"expression": "1 + 1"}}))
        .await;

    let msg = take_response_by_id(&mut ctx, 3);
    assert_eq!(msg["id"], json!(3));
    assert_eq!(msg["result"]["result"]["type"], json!("number"));
    assert_eq!(msg["result"]["result"]["value"], json!(2));
}

#[tokio::test]
async fn evaluate_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    let raw = json!({
        "id": 3_01,
        "method": "Runtime.evaluate",
        "params": {"expression": "2 + 3"}
    })
    .to_string();
    let pending = ctx.with_fixture(|conn| {
        conn.try_start_pending_command_dispatch(&raw)
            .expect("simple Runtime.evaluate should start as a pending command")
    });
    let (mut messages, scheduler_events) =
        super::complete_pending_command_task_for_test(&mut ctx, pending).await;

    let msg = messages
        .pop()
        .expect("pending Runtime.evaluate should produce a response");
    assert_eq!(msg["id"], json!(3_01));
    assert_eq!(msg["result"]["result"]["type"], json!("number"));
    assert_eq!(msg["result"]["result"]["value"], json!(5));
    assert!(
        scheduler_events.is_empty(),
        "a Runtime command without an owner action must not publish scheduler work: {scheduler_events:?}"
    );
}

#[tokio::test]
async fn evaluate_await_promise_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    let raw = json!({
        "id": 3_02,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "Promise.resolve(7)",
            "awaitPromise": true,
            "returnByValue": true
        }
    })
    .to_string();
    let pending = ctx.with_fixture(|conn| {
        conn.try_start_pending_command_dispatch(&raw).expect(
            "Runtime.evaluate awaitPromise without contextId should start as a pending command",
        )
    });
    let (mut messages, _scheduler_events) =
        super::complete_pending_command_task_for_test(&mut ctx, pending).await;

    let msg = messages
        .pop()
        .expect("pending Runtime.evaluate awaitPromise should produce a response");
    assert_eq!(msg["id"], json!(3_02));
    assert_eq!(msg["result"]["result"]["type"], json!("number"));
    assert_eq!(msg["result"]["result"]["value"], json!(7));
    ctx.with_fixture(|conn| {
        assert!(
            !conn.has_pending_inspector_awaits(),
            "settled pending Runtime.evaluate awaitPromise should drain its pending inspector entry"
        )
    });
}

#[tokio::test]
async fn evaluate_await_promise_connected_style_events_advance_on_owner_turns() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><head></head><body></body></html>").await;

    ctx.process_async(json!({
        "id": 3_020,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"
Promise.all(Array.from({ length: 129 }, (_, index) => new Promise((resolve, reject) => {
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.href = `data:text/css,:root{--runtime-owner-turn-${index}:${index}}`;
  link.addEventListener("load", () => resolve(index));
  link.addEventListener("error", () => reject(new Error(`stylesheet ${index} failed`)));
  document.head.appendChild(link);
}))).then(values => values.length)
"#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;

    let response = wait_for_response_by_id_async(&mut ctx, None, 3_020).await;
    assert_eq!(
        response["result"]["result"]["value"],
        json!(129),
        "connected stylesheet events beyond the old protocol drain cap should settle through owner turns: {response:?}"
    );
    ctx.with_fixture(|conn| {
        assert!(
            !conn.has_pending_inspector_awaits(),
            "owner-turn stylesheet delivery should retire the pending inspector await"
        )
    });
}

#[tokio::test]
async fn evaluate_await_promise_scheduler_routed_reply_clears_pending_await_registry() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    ctx.process_async(json!({
        "id": 3_021,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "Promise.resolve(17)",
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;

    let msg = take_response_by_id(&mut ctx, 3_021);
    assert_eq!(msg["result"]["result"]["type"], json!("number"));
    assert_eq!(msg["result"]["result"]["value"], json!(17));
    ctx.with_fixture(|conn| { assert!(
        !conn.has_pending_inspector_awaits(),
        "scheduler-routed Runtime.evaluate awaitPromise completion must consume the pending inspector await registry"
    ) });
}

#[tokio::test]
async fn runtime_await_promise_releases_dispatch_before_session_response() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    ctx.process_async(json!({
        "id": 3_022,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "new Promise(() => {})"
        }
    }))
    .await;
    let promise_object_id = take_response_by_id(&mut ctx, 3_022)["result"]["result"]["objectId"]
        .as_str()
        .expect("Runtime.evaluate should return a promise object handle")
        .to_owned();

    let raw = json!({
        "id": 3_023,
        "method": "Runtime.awaitPromise",
        "params": {
            "promiseObjectId": promise_object_id,
            "returnByValue": true
        }
    })
    .to_string();
    let pending = ctx.with_fixture(|conn| {
        conn.try_start_pending_command_dispatch(&raw)
            .expect("Runtime.awaitPromise should start as a pending command")
    });
    let completed = tokio::time::timeout(std::time::Duration::from_secs(1), pending.wait())
        .await
        .expect("initial Page Inspector dispatch must not wait for Promise settlement");
    let CdpCommandTaskStep::Complete(outcome) = ({
        ctx.state
            .with_state(|state| {
                state.complete_command_dispatch_with_context(
                    completed,
                    &mut crate::conn::CommandDispatchContext::default(),
                )
            })
            .expect("live test owner")
    }) else {
        panic!("a Page session response must not enter the AdapterReply deferred-reply lane");
    };
    drop(outcome);
    ctx.with_fixture(|conn| {
        assert!(
            conn.take_scheduler_events().is_empty(),
            "an unresolved session response must not manufacture scheduler follow-up work"
        )
    });

    ctx.with_fixture(|conn| {
        assert!(
            conn.has_pending_inspector_awaits_for_session_owner(None),
            "the unresolved await must remain registered to its exact Page session"
        )
    });
    ctx.with_fixture(|conn| {
        assert!(
            conn.has_unclaimed_pending_inspector_awaits_for_session_owner(None),
            "the session output path must leave the await under renderer ownership"
        )
    });
    ctx.with_fixture(|conn| {
        assert!(
            !conn.has_claimed_pending_inspector_awaits_for_session_owner(None),
            "the AdapterReply claimed-await index must remain unused"
        )
    });

    ctx.process_async(json!({
        "id": 3_024,
        "method": "Page.close"
    }))
    .await;
    assert_eq!(
        take_response_by_id(&mut ctx, 3_023)["error"]["message"],
        json!("Page closed")
    );
}

#[tokio::test]
async fn runtime_await_promise_armed_timer_reply_arrives_through_renderer_receiver() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    ctx.process_async(json!({
            "id": 3_024,
            "method": "Runtime.evaluate",
            "params": {
            "expression": "new Promise(resolve => { globalThis.__armAwaitTimer = () => setTimeout(() => resolve('await-timer'), 0); })"
            }
    }))
    .await;
    let promise_object_id = take_response_by_id(&mut ctx, 3_024)["result"]["result"]["objectId"]
        .as_str()
        .expect("Runtime.evaluate should return a timer-backed promise object handle")
        .to_owned();

    ctx.process_command_only_async(json!({
        "id": 3_025,
        "method": "Runtime.awaitPromise",
        "params": {
            "promiseObjectId": promise_object_id,
            "returnByValue": true
        }
    }))
    .await;

    assert!(
        !ctx.sent.iter().any(|message| message["id"] == json!(3_025)),
        "timer-backed Runtime.awaitPromise should defer until the renderer callback response arrives: {:?}",
        ctx.sent
    );
    ctx.process_async(json!({
        "id": 3_026,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__armAwaitTimer(); 'armed'"
        }
    }))
    .await;
    let arm_response = take_response_by_id(&mut ctx, 3_026);
    assert_eq!(arm_response["result"]["result"]["value"], json!("armed"));

    let response = wait_for_response_by_id_async(&mut ctx, None, 3_025).await;
    assert_eq!(response["result"]["result"]["type"], json!("string"));
    assert_eq!(response["result"]["result"]["value"], json!("await-timer"));
    ctx.with_fixture(|conn| {
        assert!(
            !conn.has_pending_inspector_awaits(),
            "renderer callback completion must consume Runtime.awaitPromise pending inspector state"
        )
    });
}

#[tokio::test]
async fn isolated_evaluate_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .expect("browser context should exist")
            .set_active_target_id("TID-1")
    });
    let utility_context_id = create_isolated_world_async(&mut ctx, 3_021, "utility").await;

    let raw = json!({
        "id": 3_022,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": utility_context_id,
            "returnByValue": true,
            "expression": "globalThis.__pendingIsolated = 11"
        }
    })
    .to_string();
    let pending = ctx.with_fixture(|conn| {
        conn.try_start_pending_command_dispatch(&raw)
            .expect("Runtime.evaluate with isolated contextId should start as a pending command")
    });
    let (messages, _) = super::complete_pending_command_task_for_test(&mut ctx, pending).await;
    let response = messages
        .iter()
        .find(|message| message["id"] == json!(3_022))
        .expect("pending isolated Runtime.evaluate should produce a response");
    assert_eq!(response["result"]["result"]["value"], json!(11));
}

#[tokio::test]
async fn call_function_context_resolution_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<html><title>default-title</title><body></body></html>",
    )
    .await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .expect("browser context should exist")
            .set_active_target_id("TID-1")
    });
    let _default_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 3_020).await;
    let utility_context_id = create_isolated_world_async(&mut ctx, 3_023, "utility").await;

    ctx.process_async(json!({
        "id": 3_024,
        "method": "Runtime.evaluate",
        "params": {
            "contextId": utility_context_id,
            "expression": "globalThis.__pendingIsolated = 17",
            "returnByValue": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 3_024);

    let isolated_raw = json!({
        "id": 3_025,
        "method": "Runtime.callFunctionOn",
        "params": {
            "executionContextId": utility_context_id,
            "functionDeclaration": "function() { return globalThis.__pendingIsolated + 1; }",
            "returnByValue": true
        }
    })
    .to_string();
    let isolated_pending = ctx.with_fixture(|conn| {
        conn.try_start_pending_command_dispatch(&isolated_raw)
            .expect("Runtime.callFunctionOn with isolated executionContextId should start pending")
    });
    let (isolated_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, isolated_pending).await;
    let isolated_response = isolated_messages
        .iter()
        .find(|message| message["id"] == json!(3_025))
        .expect("pending isolated Runtime.callFunctionOn should produce a response");
    assert_eq!(isolated_response["result"]["result"]["value"], json!(18));

    let default_raw = json!({
        "id": 3_026,
        "method": "Runtime.callFunctionOn",
        "params": {
            "functionDeclaration": "function() { return document.title; }",
            "returnByValue": true
        }
    })
    .to_string();
    let default_pending = ctx.with_fixture(|conn| {
        conn.try_start_pending_command_dispatch(&default_raw)
            .expect("Runtime.callFunctionOn without objectId should start pending")
    });
    let (default_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, default_pending).await;
    let default_response = default_messages
        .iter()
        .find(|message| message["id"] == json!(3_026))
        .expect("pending default Runtime.callFunctionOn should produce a response");
    assert_eq!(
        default_response["result"]["result"]["value"],
        json!("default-title")
    );
}

#[tokio::test]
async fn object_runtime_commands_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    ctx.process_async(json!({
        "id": 3_03,
        "method": "Runtime.evaluate",
        "params": {"expression": "({ answer: 41, label: 'ok', promise: Promise.resolve('done') })"}
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 3_03)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("Runtime.evaluate should return an objectId");

    let call_raw = json!({
        "id": 3_04,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": object_id,
            "functionDeclaration": "function() { return this.answer + 1; }",
            "returnByValue": true
        }
    })
    .to_string();
    let call_pending = ctx.with_fixture(|conn| {
        conn.try_start_pending_command_dispatch(&call_raw)
            .expect("Runtime.callFunctionOn with objectId should start as a pending command")
    });
    let (call_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, call_pending).await;
    let call_response = call_messages
        .iter()
        .find(|message| message["id"] == json!(3_04))
        .expect("pending Runtime.callFunctionOn should produce a response");
    assert_eq!(call_response["result"]["result"]["value"], json!(42));

    let properties_raw = json!({
        "id": 3_05,
        "method": "Runtime.getProperties",
        "params": {"objectId": object_id, "ownProperties": true}
    })
    .to_string();
    let properties_pending = ctx.with_fixture(|conn| {
        conn.try_start_pending_command_dispatch(&properties_raw)
            .expect("Runtime.getProperties with objectId should start as a pending command")
    });
    let (properties_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, properties_pending).await;
    let properties_response = properties_messages
        .iter()
        .find(|message| message["id"] == json!(3_05))
        .expect("pending Runtime.getProperties should produce a response");
    let properties = properties_response["result"]["result"]
        .as_array()
        .expect("Runtime.getProperties should return a property array");
    let promise_object_id = properties
        .iter()
        .find(|property| property["name"] == json!("promise"))
        .and_then(|property| property["value"]["objectId"].as_str())
        .map(str::to_owned)
        .expect("Runtime.getProperties should expose the promise objectId");

    let await_raw = json!({
        "id": 3_06,
        "method": "Runtime.awaitPromise",
        "params": {
            "promiseObjectId": promise_object_id,
            "returnByValue": true
        }
    })
    .to_string();
    let await_pending = ctx.with_fixture(|conn| {
        conn.try_start_pending_command_dispatch(&await_raw)
            .expect("Runtime.awaitPromise with promiseObjectId should start as a pending command")
    });
    let (await_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, await_pending).await;
    let await_response = await_messages
        .iter()
        .find(|message| message["id"] == json!(3_06))
        .expect("pending Runtime.awaitPromise should produce a response");
    assert_eq!(await_response["result"]["result"]["value"], json!("done"));

    let release_raw = json!({
        "id": 3_07,
        "method": "Runtime.releaseObject",
        "params": {"objectId": object_id}
    })
    .to_string();
    let release_pending = ctx.with_fixture(|conn| {
        conn.try_start_pending_command_dispatch(&release_raw)
            .expect("Runtime.releaseObject should start as a pending command")
    });
    let (release_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, release_pending).await;
    let release_response = release_messages
        .iter()
        .find(|message| message["id"] == json!(3_07))
        .expect("pending Runtime.releaseObject should produce a response");
    assert_eq!(release_response["result"], json!({}));
    ctx.with_fixture(|conn| {
        assert!(
            conn.validate_runtime_remote_object_ids_for_session_owner(None, &[object_id])
                .is_ok(),
            "pending Runtime.releaseObject should unregister released handles"
        )
    });
}

#[tokio::test]
async fn heap_usage_and_release_group_can_complete_through_pending_command_dispatch() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><title>ok</title><body></body></html>").await;

    let heap_raw = json!({
        "id": 3_08,
        "method": "Runtime.getHeapUsage"
    })
    .to_string();
    let heap_pending = ctx.with_fixture(|conn| {
        conn.try_start_pending_command_dispatch(&heap_raw)
            .expect("Runtime.getHeapUsage should start as a pending command")
    });
    let (heap_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, heap_pending).await;
    let heap_response = heap_messages
        .iter()
        .find(|message| message["id"] == json!(3_08))
        .expect("pending Runtime.getHeapUsage should produce a response");
    assert!(
        heap_response["result"]["usedSize"].as_u64().is_some(),
        "pending Runtime.getHeapUsage should return usedSize: {heap_response:?}"
    );

    ctx.process_async(json!({
        "id": 3_09,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "({ grouped: true })",
            "objectGroup": "pending-release-group"
        }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 3_09)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("Runtime.evaluate should return a grouped objectId");

    let release_group_raw = json!({
        "id": 3_10,
        "method": "Runtime.releaseObjectGroup",
        "params": {"objectGroup": "pending-release-group"}
    })
    .to_string();
    let release_group_pending = ctx.with_fixture(|conn| {
        conn.try_start_pending_command_dispatch(&release_group_raw)
            .expect("Runtime.releaseObjectGroup should start as a pending command")
    });
    let (release_group_messages, _) =
        super::complete_pending_command_task_for_test(&mut ctx, release_group_pending).await;
    let release_group_response = release_group_messages
        .iter()
        .find(|message| message["id"] == json!(3_10))
        .expect("pending Runtime.releaseObjectGroup should produce a response");
    assert_eq!(release_group_response["result"], json!({}));
    ctx.with_fixture(|conn| {
        assert!(
            conn.validate_runtime_remote_object_ids_for_session_owner(None, &[object_id])
                .is_ok(),
            "pending Runtime.releaseObjectGroup should unregister grouped handles"
        )
    });
}

#[tokio::test]
async fn evaluate_without_runtime_enable_uses_inspector_default_context_silently() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<html><body><script>globalThis.__runtimelessProbe = 41;</script></body></html>",
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 3_1,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__runtimelessProbe + 1",
            "returnByValue": true
        }
    }))
    .await;

    let msg = take_response_by_id(&mut ctx, 3_1);
    assert_eq!(msg["result"]["result"]["type"], json!("number"));
    assert_eq!(msg["result"]["result"]["value"], json!(42));
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.executionContextCreated")),
        "runtimeless evaluate must not open Runtime event surface: {:?}",
        ctx.sent
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn process_message_async_observes_background_timer_between_evaluates() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;

    ctx.process_async(json!({
        "id": 3_100,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  globalThis.__lm_async_drain_marker = "pending";
  setTimeout(() => { globalThis.__lm_async_drain_marker = "drained"; }, 0);
  return "scheduled";
})()"#
        }
    }))
    .await;
    let scheduled = take_response_by_id(&mut ctx, 3_100);
    assert_eq!(scheduled["result"]["result"]["value"], json!("scheduled"));

    // Yield long enough for the owner loop to fire the due timer in its
    // background tick branch before the next evaluate is dispatched.
    for _ in 0..16 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        ctx.process_async(json!({
            "id": 3_101,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__lm_async_drain_marker"
            }
        }))
        .await;
        let observed = take_response_by_id(&mut ctx, 3_101);
        if observed["result"]["result"]["value"] == json!("drained") {
            return;
        }
    }
    panic!("background timer did not fire within retry budget");
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_document_replacement_clears_timer_mutated_inline_style_state() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    ctx.capture_fixture_layout(None).await;

    ctx.process_and_wait_for_response_async(json!({
        "id": 3_120,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  document.body.innerHTML = "<div id='before' style='display:none'>before</div>";
  return new Promise(resolve => setTimeout(() => {
    document.getElementById('before').style.display = 'block';
    resolve("mutated");
  }, 0));
})()"#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    let mutated = take_response_by_id(&mut ctx, 3_120);
    assert_eq!(mutated["result"]["result"]["value"], json!("mutated"));

    ctx.capture_fixture_layout(None).await;
    ctx.process_async(json!({
        "id": 3_121,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  const before = document.getElementById('before');
  return `${before.style.display}:${getComputedStyle(before).display}:${before.getClientRects().length}`;
})()"#
        }
    }))
    .await;
    let warmed = take_response_by_id(&mut ctx, 3_121);
    assert_eq!(warmed["result"]["result"]["value"], json!("block:block:1"));

    ctx.process_async(json!({
        "id": 3_122,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  document.open();
  document.write("<!doctype html><html><body><div id='after' style='display:none'>after</div></body></html>");
  document.close();
  return "replaced";
})()"#
        }
    }))
    .await;
    let replaced = take_response_by_id(&mut ctx, 3_122);
    assert_eq!(replaced["result"]["result"]["value"], json!("replaced"));

    ctx.process_async(json!({
        "id": 3_123,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  const after = document.getElementById('after');
  return `${after.getAttribute('style')}:${after.style.display}:${getComputedStyle(after).display}:${after.getClientRects().length}:${after.offsetWidth}`;
})()"#
        }
    }))
    .await;
    let after = take_response_by_id(&mut ctx, 3_123);
    assert_eq!(
        after["result"]["result"]["value"],
        json!("display:none:none:none:0:0")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn isolated_call_function_document_replacement_clears_default_world_style_state() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    ctx.capture_fixture_layout(None).await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .expect("browser context should exist")
            .set_active_target_id("TID-1")
    });
    let _default_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 3_130).await;
    let utility_context_id = create_isolated_world_async(&mut ctx, 3_131, "utility").await;

    ctx.process_and_wait_for_response_async(json!({
        "id": 3_132,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  document.body.innerHTML = "<div id='before' style='display:none'>before</div>";
  return new Promise(resolve => setTimeout(() => {
    document.getElementById('before').style.display = 'block';
    resolve("mutated");
  }, 0));
})()"#,
            "awaitPromise": true,
            "returnByValue": true
        }
    }))
    .await;
    let mutated = take_response_by_id(&mut ctx, 3_132);
    assert_eq!(mutated["result"]["result"]["value"], json!("mutated"));

    ctx.capture_fixture_layout(None).await;
    ctx.process_async(json!({
        "id": 3_133,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  const before = document.getElementById('before');
  return `${before.style.display}:${getComputedStyle(before).display}:${before.getClientRects().length}`;
})()"#
        }
    }))
    .await;
    let warmed = take_response_by_id(&mut ctx, 3_133);
    assert_eq!(warmed["result"]["result"]["value"], json!("block:block:1"));

    ctx.process_async(json!({
        "id": 3_134,
        "method": "Runtime.callFunctionOn",
        "params": {
            "executionContextId": utility_context_id,
            "functionDeclaration": r#"function() {
  document.open();
  document.write("<!doctype html><html><body><div id='after' style='display:none'>after</div></body></html>");
  document.close();
  return "replaced";
}"#,
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;
    let replaced = take_response_by_id(&mut ctx, 3_134);
    assert_eq!(replaced["result"]["result"]["value"], json!("replaced"));

    ctx.process_async(json!({
        "id": 3_135,
        "method": "Runtime.evaluate",
        "params": {
            "expression": r#"(() => {
  const after = document.getElementById('after');
  return `${after.getAttribute('style')}:${after.style.display}:${getComputedStyle(after).display}:${after.getClientRects().length}:${after.offsetWidth}`;
})()"#
        }
    }))
    .await;
    let after = take_response_by_id(&mut ctx, 3_135);
    assert_eq!(
        after["result"]["result"]["value"],
        json!("display:none:none:none:0:0")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_utility_document_replacement_clears_default_world_style_state() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    ctx.capture_fixture_layout(None).await;
    ctx.with_fixture(|conn| {
        conn.browser_context
            .as_mut()
            .expect("browser context should exist")
            .set_active_target_id("TID-1")
    });
    let default_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 3_140).await;
    let utility_context_id = create_isolated_world_async(&mut ctx, 3_141, "utility").await;

    for (id, context_id) in [(3_142, default_context_id), (3_143, utility_context_id)] {
        ctx.process_async(json!({
            "id": id,
            "method": "Runtime.evaluate",
            "params": {
                "contextId": context_id,
                "expression": r#"(() => ({
  evaluate(expression) {
    return globalThis.eval(expression);
  }
}))()"#
            }
        }))
        .await;
    }
    let default_utility = take_response_by_id(&mut ctx, 3_142)["result"]["result"]["objectId"]
        .as_str()
        .expect("default utility object id")
        .to_owned();
    let isolated_utility = take_response_by_id(&mut ctx, 3_143)["result"]["result"]["objectId"]
        .as_str()
        .expect("isolated utility object id")
        .to_owned();

    ctx.process_async(json!({
        "id": 3_144,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": isolated_utility,
            "functionDeclaration": "(utility, expression) => utility.evaluate(expression)",
            "arguments": [
                { "objectId": isolated_utility },
                { "value": r#"(() => {
  document.open();
  document.write("<!doctype html><html><body><div id='before' style='display:none'>before</div><script>globalThis.__lm_timer_style_mutation = new Promise(resolve => { setTimeout(() => { document.getElementById('before').style.display = 'block'; resolve('mutated'); }, 0); });</script></body></html>");
  document.close();
})()"# }
            ],
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 3_144);

    ctx.capture_fixture_layout(None).await;
    ctx.process_async(json!({
        "id": 3_145,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": default_utility,
            "functionDeclaration": "(utility, expression) => utility.evaluate(expression)",
            "arguments": [
                { "objectId": default_utility },
                { "value": r#"globalThis.__lm_timer_style_mutation.then(() => {
  const before = document.getElementById('before');
  return `${before.style.display}:${getComputedStyle(before).display}:${before.getClientRects().length}`;
})"# }
            ],
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;
    let warmed = take_response_by_id(&mut ctx, 3_145);
    assert_eq!(warmed["result"]["result"]["value"], json!("block:block:1"));

    ctx.process_async(json!({
        "id": 3_146,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": isolated_utility,
            "functionDeclaration": "(utility, expression) => utility.evaluate(expression)",
            "arguments": [
                { "objectId": isolated_utility },
                { "value": r#"(() => {
  document.open();
  document.write("<!doctype html><html><body><div id='after' style='display:none'>after</div></body></html>");
  document.close();
})()"# }
            ],
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;
    let _ = take_response_by_id(&mut ctx, 3_146);

    ctx.process_async(json!({
        "id": 3_147,
        "method": "Runtime.callFunctionOn",
        "params": {
            "objectId": default_utility,
            "functionDeclaration": "(utility, expression) => utility.evaluate(expression)",
            "arguments": [
                { "objectId": default_utility },
                { "value": r#"(() => {
  const after = document.getElementById('after');
  return `${after.getAttribute('style')}:${after.style.display}:${getComputedStyle(after).display}:${after.getClientRects().length}:${after.offsetWidth}`;
})()"# }
            ],
            "returnByValue": true,
            "awaitPromise": true
        }
    }))
    .await;
    let after = take_response_by_id(&mut ctx, 3_147);
    assert_eq!(
        after["result"]["result"]["value"],
        json!("display:none:none:none:0:0")
    );
}

#[tokio::test]
async fn evaluate_returns_remote_object_id_for_objects() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 31).await;

    ctx.process_async(json!({"id": 6, "method": "Runtime.evaluate",
                             "params": {"expression": "({ answer: 42 })"}}))
        .await;

    let msg = take_response_by_id(&mut ctx, 6);
    assert_eq!(msg["id"], json!(6));
    assert_eq!(
        msg["result"]["result"]["type"],
        json!("object"),
        "unexpected isolated Runtime.evaluate response: {msg:?}"
    );
    assert!(msg["result"]["result"]["objectId"].as_str().is_some());
}

#[tokio::test]
async fn scoped_binding_persists_across_navigation_for_registered_named_world() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body>before</body></html>").await;
    ctx.with_fixture(|conn| {
        let bc = {
            conn.browser_context
                .as_mut()
                .expect("browser context should exist")
        };
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
    });
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 331).await;
    let _ = create_isolated_world_async(&mut ctx, 332, "utility").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 333,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "persistedUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 333);
    assert_eq!(add_binding["result"], json!({}));
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 3331,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "sessionId": "SID-1",
        "params": {
            "source": "",
            "worldName": "utility"
        }
    }))
    .await;
    let add_script = take_response_by_id(&mut ctx, 3331);
    assert!(add_script["result"]["identifier"].as_str().is_some());
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 334,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": "data:text/html,<body>after</body>" }
    }))
    .await;
    let navigation_messages = ctx.take_all();
    let replayed_context_id = navigation_messages
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.executionContextCreated")
                && message["params"]["context"]["name"] == json!("utility")
        })
        .and_then(|message| message["params"]["context"]["id"].as_i64())
        .expect("registered named world should be recreated while Runtime is enabled");

    ctx.process_async(json!({
        "id": 336,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "typeof globalThis.persistedUtilityBinding"
        }
    }))
    .await;
    let default_world = take_response_by_id(&mut ctx, 336);
    assert_eq!(default_world["result"]["result"]["type"], json!("string"));
    assert_eq!(
        default_world["result"]["result"]["value"],
        json!("undefined")
    );

    ctx.process_async(json!({
        "id": 337,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.persistedUtilityBinding('after-nav'); 11",
            "contextId": replayed_context_id
        }
    }))
    .await;
    let call = take_response_by_id(&mut ctx, 337);
    assert_eq!(call["result"]["result"]["type"], json!("number"));
    assert_eq!(call["result"]["result"]["value"], json!(11));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("persistedUtilityBinding")
        })
        .cloned()
        .expect("scoped binding should survive navigation");
    assert_eq!(binding_called["params"]["payload"], json!("after-nav"));
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(replayed_context_id)
    );
}

#[tokio::test]
async fn scoped_binding_applies_to_matching_isolated_world_created_after_registration() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    ctx.with_fixture(|conn| {
        let bc = {
            conn.browser_context
                .as_mut()
                .expect("browser context should exist")
        };
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
    });
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 338).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 339,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "lateUtilityBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 339);
    assert_eq!(add_binding["result"], json!({}));
    ctx.sent.clear();

    let utility_context_id = create_isolated_world_async(&mut ctx, 340, "utility").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 341,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "globalThis.lateUtilityBinding('created-late'); 13",
            "contextId": utility_context_id
        }
    }))
    .await;
    let call = take_response_by_id(&mut ctx, 341);
    assert_eq!(call["result"]["result"]["type"], json!("number"));
    assert_eq!(call["result"]["result"]["value"], json!(13));

    let binding_called = ctx
        .sent
        .iter()
        .find(|message| {
            message["method"] == json!("Runtime.bindingCalled")
                && message["params"]["name"] == json!("lateUtilityBinding")
        })
        .cloned()
        .expect("late-created matching world should receive binding");
    assert_eq!(binding_called["params"]["payload"], json!("created-late"));
    assert_eq!(
        binding_called["params"]["executionContextId"],
        json!(utility_context_id)
    );
}

#[tokio::test]
async fn scoped_binding_does_not_apply_to_non_matching_isolated_world() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    ctx.with_fixture(|conn| {
        let bc = {
            conn.browser_context
                .as_mut()
                .expect("browser context should exist")
        };
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
    });
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 342).await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 343,
        "method": "Runtime.addBinding",
        "sessionId": "SID-1",
        "params": {
            "name": "utilityOnlyBinding",
            "executionContextName": "utility"
        }
    }))
    .await;
    let add_binding = take_response_by_id(&mut ctx, 343);
    assert_eq!(add_binding["result"], json!({}));
    ctx.sent.clear();

    let other_context_id = create_isolated_world_async(&mut ctx, 344, "other").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 345,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": "typeof globalThis.utilityOnlyBinding",
            "contextId": other_context_id
        }
    }))
    .await;
    let result = take_response_by_id(&mut ctx, 345);
    assert_eq!(result["result"]["result"]["type"], json!("string"));
    assert_eq!(result["result"]["result"]["value"], json!("undefined"));
    assert!(!ctx.sent.iter().any(|message| {
        message["method"] == json!("Runtime.bindingCalled")
            && message["params"]["name"] == json!("utilityOnlyBinding")
    }));
}

#[tokio::test]
async fn evaluate_in_isolated_world_uses_separate_global_context() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    ctx.with_fixture(|conn| {
        let bc = {
            conn.browser_context
                .as_mut()
                .expect("browser context should exist")
        };
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
    });
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 33).await;

    let isolated_context_id = create_isolated_world_async(&mut ctx, 34, "utility").await;

    ctx.process_async(json!({
        "id": 35,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__lmIso = (globalThis.__lmIso || 0) + 1; globalThis.__lmIso",
            "contextId": isolated_context_id
        }
    }))
    .await;
    let isolated = take_response_by_id(&mut ctx, 35);
    assert_eq!(isolated["result"]["result"]["type"], json!("number"));
    assert_eq!(isolated["result"]["result"]["value"], json!(1));

    ctx.process_async(json!({
        "id": 36,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "typeof globalThis.__lmIso"
        }
    }))
    .await;
    let default_context = take_response_by_id(&mut ctx, 36);
    assert_eq!(default_context["result"]["result"]["type"], json!("string"));
    assert_eq!(
        default_context["result"]["result"]["value"],
        json!("undefined")
    );

    ctx.process_async(json!({
        "id": 37,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "globalThis.__lmIso",
            "contextId": isolated_context_id
        }
    }))
    .await;
    let isolated_again = take_response_by_id(&mut ctx, 37);
    assert_eq!(isolated_again["result"]["result"]["type"], json!("number"));
    assert_eq!(isolated_again["result"]["result"]["value"], json!(1));
}

#[tokio::test]
async fn evaluate_in_isolated_world_does_not_require_runtime_frontend_enabled() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    ctx.with_fixture(|conn| {
        let bc = {
            conn.browser_context
                .as_mut()
                .expect("browser context should exist")
        };
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
    });

    let isolated_context_id = create_isolated_world_async(&mut ctx, 38, "utility").await;

    ctx.process_async(json!({
        "id": 39,
        "method": "Runtime.evaluate",
        "params": {
            "expression": "({ answer: 42 })",
            "contextId": isolated_context_id
        }
    }))
    .await;
    let msg = take_response_by_id(&mut ctx, 39);
    assert_eq!(
        msg["result"]["result"]["type"],
        json!("object"),
        "unexpected isolated Runtime.evaluate response: {msg:?}"
    );
    assert!(
        msg["result"]["result"]["objectId"].as_str().is_some(),
        "isolated Runtime.evaluate without Runtime.enable should use inspector handles"
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|message| message["method"] == json!("Runtime.executionContextCreated")),
        "silent isolated inspector materialization must not open Runtime event surface"
    );
}

#[tokio::test]
async fn isolated_world_file_assignment_updates_main_world_input_files_surface() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(
        &mut ctx,
        "<html><body><input id='upload' type='file'></body></html>",
    )
    .await;
    ctx.with_fixture(|conn| {
        let bc = {
            conn.browser_context
                .as_mut()
                .expect("browser context should exist")
        };
        bc.set_active_target_id("TID-1");
        bc.attach_active_session("SID-1");
    });
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 381).await;
    let isolated_context_id = create_isolated_world_async(&mut ctx, 382, "utility").await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 383,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"
(() => {
  const input = document.getElementById('upload');
  globalThis.__mainWorldUploadEvents = [];
  input.addEventListener('input', () => {
    globalThis.__mainWorldUploadEvents.push(`input:${input.files.length}:${input.value}`);
  });
  input.addEventListener('change', () => {
    globalThis.__mainWorldUploadEvents.push(`change:${input.files[0].name}:${input.files[0].type}`);
  });
  return 'ready';
})()
"#
        }
    }))
    .await;
    let setup = take_response_by_id(&mut ctx, 383);
    assert_eq!(setup["result"]["result"]["value"], json!("ready"));

    ctx.process_async(json!({
        "id": 384,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"
(() => {
  const input = document.getElementById('upload');
  const dt = new DataTransfer();
  dt.items.add(new File([new Uint8Array([1, 2, 3])], 'note.txt', {
    type: 'text/plain',
    lastModified: 42
  }));
  input.files = dt.files;
  input.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
  return `${input.files.length}|${input.files[0].name}|${input.value}`;
})()
"#,
            "contextId": isolated_context_id
        }
    }))
    .await;
    let utility_result = take_response_by_id(&mut ctx, 384);
    assert_eq!(
        utility_result["result"]["result"]["value"],
        json!("1|note.txt|C:\\fakepath\\note.txt")
    );

    ctx.process_async(json!({
        "id": 385,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "expression": r#"
JSON.stringify({
  count: document.getElementById('upload').files.length,
  name: document.getElementById('upload').files[0].name,
  type: document.getElementById('upload').files[0].type,
  lastModified: document.getElementById('upload').files[0].lastModified,
  value: document.getElementById('upload').value,
  events: globalThis.__mainWorldUploadEvents
})
"#
        }
    }))
    .await;
    let main_world = take_response_by_id(&mut ctx, 385);
    assert_eq!(
        main_world["result"]["result"]["value"],
        json!(
            r#"{"count":1,"name":"note.txt","type":"text/plain","lastModified":42,"value":"C:\\fakepath\\note.txt","events":["input:1:C:\\fakepath\\note.txt","change:note.txt:text/plain"]}"#
        )
    );
}

#[tokio::test]
async fn call_function_on_with_args() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let execution_context_id =
        enable_runtime_and_take_execution_context_id_async(&mut ctx, 40).await;

    ctx.process_async(json!({
        "id": 4,
        "method": "Runtime.callFunctionOn",
        "params": {
            "functionDeclaration": "(a, b) => a + b",
            "executionContextId": execution_context_id,
            "arguments": [{"value": 2}, {"value": 3}]
        }
    }))
    .await;

    let msg = take_response_by_id(&mut ctx, 4);
    assert_eq!(msg["id"], json!(4));
    assert_eq!(msg["result"]["result"]["type"], json!("number"));
    assert_eq!(msg["result"]["result"]["value"], json!(5));
}

#[tokio::test]
async fn get_properties_reads_object_via_object_id() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 32).await;

    ctx.process_async(json!({
        "id": 7,
        "method": "Runtime.evaluate",
        "params": {"expression": "({ answer: 42, label: 'ok' })"}
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 7)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("Runtime.evaluate must return an objectId for objects");

    ctx.process_async(json!({
        "id": 8,
        "method": "Runtime.getProperties",
        "params": {"objectId": object_id, "ownProperties": true}
    }))
    .await;

    let msg = take_response_by_id(&mut ctx, 8);
    assert_eq!(msg["id"], json!(8));
    let props = msg["result"]["result"]
        .as_array()
        .expect("Runtime.getProperties must return a property array");
    assert!(
        props
            .iter()
            .any(|prop| { prop["name"] == json!("answer") && prop["value"]["value"] == json!(42) })
    );
}

#[tokio::test]
async fn runtime_evaluate_node_result_reports_node_subtype() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body><input id='box'></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 320).await;

    ctx.process_async(json!({
        "id": 321,
        "method": "Runtime.evaluate",
        "params": { "expression": "document.querySelector('#box')" }
    }))
    .await;

    let msg = take_response_by_id(&mut ctx, 321);
    assert_eq!(msg["result"]["result"]["type"], json!("object"));
    assert_eq!(msg["result"]["result"]["subtype"], json!("node"));
    assert!(msg["result"]["result"]["objectId"].as_str().is_some());
}

#[tokio::test]
async fn runtime_get_properties_reports_node_subtype_for_node_properties() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body><input id='box'></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 322).await;

    ctx.process_async(json!({
        "id": 323,
        "method": "Runtime.evaluate",
        "params": { "expression": "({ element: document.querySelector('#box') })" }
    }))
    .await;
    let object_id = take_response_by_id(&mut ctx, 323)["result"]["result"]["objectId"]
        .as_str()
        .map(str::to_owned)
        .expect("Runtime.evaluate must return an objectId for objects");

    ctx.process_async(json!({
        "id": 324,
        "method": "Runtime.getProperties",
        "params": { "objectId": object_id, "ownProperties": true }
    }))
    .await;

    let msg = take_response_by_id(&mut ctx, 324);
    let props = msg["result"]["result"]
        .as_array()
        .expect("Runtime.getProperties must return a property array");
    let element = props
        .iter()
        .find(|prop| prop["name"] == json!("element"))
        .expect("element property should be present");
    assert_eq!(element["value"]["type"], json!("object"));
    assert_eq!(element["value"]["subtype"], json!("node"));
    assert!(element["value"]["objectId"].as_str().is_some());
}

#[tokio::test]
async fn evaluate_reports_exception_details() {
    let mut ctx = TestContext::new();
    with_loaded_document_async(&mut ctx, "<html><body></body></html>").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 33).await;

    ctx.process_async(json!({
        "id": 5,
        "method": "Runtime.evaluate",
        "params": {"expression": "(() => { throw new Error('boom'); })()"}
    }))
    .await;

    let msg = take_response_by_id(&mut ctx, 5);
    assert_eq!(msg["id"], json!(5));
    assert!(msg["result"]["exceptionDetails"].is_object());
    assert!(
        msg["result"]["exceptionDetails"]["exceptionId"]
            .as_u64()
            .is_some(),
        "Runtime.evaluate exceptionDetails should include exceptionId: {msg:?}"
    );
    let text = msg["result"]["exceptionDetails"]["exception"]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(text.contains("boom"), "unexpected exception text: {text}");
}
