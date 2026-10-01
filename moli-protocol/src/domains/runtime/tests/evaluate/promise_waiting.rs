// Tests grouped by behavior. Shared fixtures live in the parent module.
use super::*;

#[test]
fn runtime_evaluate_set_timeout_flushes_runtime_activity_inside_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;
        ctx.process_async(json!({
            "id": 2059,
            "method": "Runtime.enable"
        }))
        .await;
        let response = take_response_by_id(&mut ctx, 2059);
        assert_eq!(response["result"], json!({}));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 2060,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "setTimeout(() => { globalThis.__timerRan = 1; }, 0)"
            }
        }))
        .await;

        let response = take_response_by_id(&mut ctx, 2060);
        assert_eq!(response["id"], 2060);
        assert!(
            response.get("error").is_none(),
            "runtime evaluate should not fail: {response:?}"
        );

        // The setTimeout(0) callback now fires from the owner loop's idle tick
        // branch (Step A/B refactor) instead of inline as part of the evaluate
        // reply. Retry the observation evaluate to give the owner a chance to
        // tick before the next command arrives.
        for attempt in 0..16 {
            ctx.process_async(json!({
                "id": 2061,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "globalThis.__timerRan ?? 0"
                }
            }))
            .await;

            let response = take_response_by_id(&mut ctx, 2061);
            if response["result"]["result"]["value"] == json!(1) {
                return;
            }
            if attempt + 1 == 16 {
                panic!("background timer did not fire within retry budget: {response:?}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    });
}

#[test]
fn runtime_evaluate_await_promise_waits_for_set_timeout_settlement_inside_current_thread_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 2062,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "new Promise(resolve => setTimeout(() => resolve(7), 0))",
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 2062).await;
        assert_eq!(response["result"]["result"]["type"], json!("number"));
        assert_eq!(response["result"]["result"]["value"], json!(7));
    });
}

#[test]
fn runtime_evaluate_await_promise_waits_for_nonzero_timeout_settlement() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 2063,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "new Promise(resolve => setTimeout(() => resolve(11), 40))",
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 2063).await;
        assert_eq!(response["result"]["result"]["type"], json!("number"));
        assert_eq!(response["result"]["result"]["value"], json!(11));
    });
}

#[test]
fn runtime_evaluate_request_idle_callback_time_remaining_expires_within_callback() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 2064,
            "method": "Runtime.evaluate",
            "params": {
                "expression": r#"
new Promise(resolve => requestIdleCallback(deadline => {
  const first = deadline.timeRemaining();
  const start = Date.now();
  while (Date.now() - start < 80) {}
  const second = deadline.timeRemaining();
  resolve({
    firstPositive: first > 0,
    secondExpired: second === 0,
    decreased: second < first
  });
}))
"#,
                "awaitPromise": true,
                "returnByValue": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 2064).await;
        assert_eq!(
            response["result"]["result"]["value"],
            json!({
                "firstPositive": true,
                "secondExpired": true,
                "decreased": true
            })
        );
    });
}

#[test]
fn runtime_call_function_on_await_promise_settles_after_request_animation_frame() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 2065,
            "method": "Runtime.enable"
        }))
        .await;
        let enabled = take_response_by_id(&mut ctx, 2065);
        assert_eq!(enabled["result"], json!({}));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 2066,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__lmAsyncProbe = { check() { return new Promise(resolve => requestAnimationFrame(() => resolve({ ready: true, source: 'raf' }))); } }; globalThis.__lmAsyncProbe"
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, 2066)["result"]["result"]["objectId"]
            .as_str()
            .map(str::to_owned)
            .expect("Runtime.evaluate should return an objectId for the async probe");

        ctx.process_async(json!({
            "id": 2067,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": object_id,
                "functionDeclaration": "function() { return this.check(); }",
                "returnByValue": true,
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 2067).await;
        assert_eq!(response["result"]["result"]["type"], json!("object"));
        assert_eq!(
            response["result"]["result"]["value"],
            json!({ "ready": true, "source": "raf" })
        );
    });
}

#[test]
fn runtime_call_function_on_await_promise_waits_for_nonzero_timeout_settlement() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 20_671,
            "method": "Runtime.enable"
        }))
        .await;
        let enabled = take_response_by_id(&mut ctx, 20_671);
        assert_eq!(enabled["result"], json!({}));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 20_672,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__lmDelayedAsyncProbe = { check() { return new Promise(resolve => setTimeout(() => resolve({ ready: true, delayMs: 40 }), 40)); } }; globalThis.__lmDelayedAsyncProbe"
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, 20_672)["result"]["result"]["objectId"]
            .as_str()
            .map(str::to_owned)
            .expect("Runtime.evaluate should return an objectId for the delayed async probe");

        ctx.process_async(json!({
            "id": 20_673,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": object_id,
                "functionDeclaration": "function() { return this.check(); }",
                "returnByValue": true,
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 20_673).await;
        assert_eq!(response["result"]["result"]["type"], json!("object"));
        assert_eq!(
            response["result"]["result"]["value"],
            json!({ "ready": true, "delayMs": 40 })
        );
    });
}

#[test]
fn runtime_call_function_on_await_promise_waits_for_polling_handle_result_until_settled() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 20_674,
            "method": "Runtime.enable"
        }))
        .await;
        let enabled = take_response_by_id(&mut ctx, 20_674);
        assert_eq!(enabled["result"], json!({}));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 20_675,
            "method": "Runtime.evaluate",
            "params": {
                "expression": r#"(() => {
  globalThis.__lmWaitProbeDone = false;
  setTimeout(() => { globalThis.__lmWaitProbeDone = true; }, 50);
  globalThis.__lmWaitProbe = {
    result: new Promise(resolve => {
      const next = () => {
        if (globalThis.__lmWaitProbeDone) {
          resolve("wait-ready");
          return;
        }
        requestAnimationFrame(next);
      };
      next();
    })
  };
  return globalThis.__lmWaitProbe;
})()"#
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, 20_675)["result"]["result"]["objectId"]
            .as_str()
            .map(str::to_owned)
            .expect("Runtime.evaluate should return an objectId for the polling await probe");

        ctx.process_async(json!({
            "id": 20_676,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": object_id,
                "functionDeclaration": "function() { return this.result; }",
                "returnByValue": true,
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 20_676).await;
        assert_eq!(response["result"]["result"]["type"], json!("string"));
        assert_eq!(response["result"]["result"]["value"], json!("wait-ready"));
    });
}

#[test]
fn runtime_evaluate_await_promise_waits_for_request_animation_frame_polling_condition() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 206_761,
            "method": "Runtime.evaluate",
            "params": {
                "expression": r#"(() => {
  window.__done = false;
  setTimeout(() => { window.__done = true; }, 50);
  return new Promise(resolve => {
    const poll = () => {
      if (window.__done === true) {
        resolve(true);
        return;
      }
      requestAnimationFrame(poll);
    };
    poll();
  });
})()"#,
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 206_761).await;
        assert_eq!(
            response["result"]["result"]["type"],
            json!("boolean"),
            "unexpected rAF polling await response: {response:?}"
        );
        assert_eq!(response["result"]["result"]["value"], json!(true));
    });
}

#[test]
fn runtime_evaluate_await_promise_waits_for_request_animation_frame_polling_with_indirect_eval() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 206_762,
            "method": "Runtime.evaluate",
            "params": {
                "expression": r#"(() => {
  window.__done = false;
  setTimeout(() => { window.__done = true; }, 50);
  return new Promise(resolve => {
    const poll = () => {
      if (globalThis.eval("window.__done === true")) {
        resolve(true);
        return;
      }
      requestAnimationFrame(poll);
    };
    poll();
  });
})()"#,
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 206_762).await;
        assert_eq!(
            response["result"]["result"]["type"],
            json!("boolean"),
            "unexpected indirect-eval rAF polling await response: {response:?}"
        );
        assert_eq!(response["result"]["result"]["value"], json!(true));
    });
}

#[test]
fn runtime_evaluate_await_promise_without_enable_does_not_create_legacy_global_token() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        // Keep the test promise reachable so V8's legitimate collection of an
        // otherwise unreferenced pending promise cannot race the registry
        // assertion below. The behavior under test is that moli itself
        // does not create a `__lmAwaitPromise_*` polling token.
        ctx.process_async(json!({
            "id": 206_763,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__testPendingInspectorPromise = new Promise(() => {})",
                "awaitPromise": true
            }
        }))
        .await;

        assert!(
            !ctx.sent.iter().any(|message| message["id"] == json!(206_763)),
            "never-settling inspector await should defer the response: {:?}",
            ctx.sent
        );
        assert!(
            ctx.conn.has_pending_inspector_awaits(),
            "runtimeless awaitPromise should use the inspector pending-await registry"
        );

        ctx.process_async(json!({
            "id": 206_764,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "Object.keys(globalThis).filter(key => key.startsWith('__lmAwaitPromise_')).length",
                "returnByValue": true
            }
        }))
        .await;

        let probe = take_response_by_id(&mut ctx, 206_764);
        assert_eq!(
            probe["result"]["result"]["value"],
            json!(0),
            "inspector awaitPromise should not create legacy global polling tokens: {probe:?}"
        );
    });
}

#[test]
fn runtime_evaluate_await_promise_pending_is_failed_when_page_closes() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;
        if let Some(bc) = ctx.conn.browser_context.as_mut() {
            bc.set_active_target_id("TID-1");
        }
        let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 9_001).await;
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 9_002,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__pageClosePendingPromise = new Promise(() => {})",
                "awaitPromise": true
            }
        }))
        .await;

        assert!(
            !ctx.sent.iter().any(|message| message["id"] == json!(9_002)),
            "awaitPromise on a never-resolving promise should defer the response, got: {:?}",
            ctx.sent
        );
        assert!(
            ctx.conn.has_pending_inspector_awaits(),
            "deferred awaitPromise must register a pending inspector entry"
        );

        ctx.process_async(json!({
            "id": 9_003,
            "method": "Page.close"
        }))
        .await;

        let failed = take_response_by_id(&mut ctx, 9_002);
        assert_eq!(failed["error"]["code"], json!(-32000));
        assert_eq!(failed["error"]["message"], json!("Page closed"));
        assert!(
            !ctx.conn.has_pending_inspector_awaits(),
            "all pending inspector awaits should be cleared after Page.close"
        );
    });
}

#[test]
fn runtime_evaluate_await_promise_pending_is_terminated_once_by_navigation_replacement() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_for_active_target_async(
            &mut ctx,
            "<html><body>old</body></html>",
            "SID-1",
            "TID-1",
        )
        .await;
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 9_012,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "globalThis.__navigationPendingPromise = new Promise(() => {})",
                "awaitPromise": true
            }
        }))
        .await;
        assert!(
            !ctx.sent.iter().any(|message| message["id"] == json!(9_012)),
            "never-settling evaluate must remain outstanding before navigation"
        );

        ctx.process_async(json!({
            "id": 9_013,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<html><body>new</body></html>"
            }
        }))
        .await;

        wait_until_message(
            &mut ctx,
            "SID-1",
            "navigation terminal response for old Runtime.evaluate",
            |message| message["id"] == json!(9_012),
        )
        .await;
        let responses = ctx
            .sent
            .iter()
            .filter(|message| message["id"] == json!(9_012))
            .collect::<Vec<_>>();
        assert_eq!(
            responses.len(),
            1,
            "navigation replacement must complete the old evaluate exactly once: {:?}",
            ctx.sent
        );
        assert_eq!(responses[0]["error"]["code"], json!(-32000));
        assert_eq!(responses[0]["sessionId"], json!("SID-1"));
        assert_eq!(
            responses[0]["error"]["message"],
            // Chromium's native Inspector response for a pending promise
            // whose Document is destroyed during navigation.
            json!("Execution context was destroyed.")
        );
        assert!(
            ctx.conn
                .renderer_runtime_command_cause_for_frontend(Some("SID-1"), 9_012)
                .is_none(),
            "terminal replacement response must consume the renderer correlation"
        );
        assert!(
            !ctx.conn.has_pending_inspector_awaits(),
            "terminal replacement response must consume the pending command owner"
        );
    });
}

#[test]
fn replay_policy_command_rotates_lease_and_completes_on_replacement_attachment() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_for_active_target_async(
            &mut ctx,
            "<html><body>old</body></html>",
            "SID-1",
            "TID-1",
        )
        .await;
        let old_attachment = ctx
            .conn
            .current_renderer_agent_attachment_id_for_owner(
                &crate::conn::CommandOwnerScope::for_session("SID-1"),
            )
            .expect("old Page attachment");
        let frontend_id = 9_021;
        let payload = json!({
            "id": frontend_id,
            "method": "Console.clearMessages",
            "sessionId": "SID-1",
            "params": {}
        })
        .to_string();
        let descriptor = crate::conn::RendererCommandDescriptor::from_synthesized_payload(payload)
            .expect("supported replay command");
        let prepared = ctx
            .conn
            .try_register_renderer_call_for_session_owner(
                Some("SID-1"),
                frontend_id,
                Some(old_attachment),
                descriptor,
            )
            .expect("register replay command");
        let (old_correlation, old_sender, response_receiver) = prepared.into_parts();
        let response_receiver = response_receiver
            .expect("a synthesized AdapterReply call must allocate a response receiver");

        ctx.process_async(json!({
            "id": 9_022,
            "method": "Page.navigate",
            "sessionId": "SID-1",
            "params": {
                "url": "data:text/html,<html><body>new</body></html>"
            }
        }))
        .await;

        let new_attachment = ctx
            .conn
            .current_renderer_agent_attachment_id_for_owner(
                &crate::conn::CommandOwnerScope::for_session("SID-1"),
            )
            .expect("replacement Page attachment");
        assert_ne!(new_attachment, old_attachment);
        assert!(
            old_sender
                .send(json!({
                    "id": old_correlation.renderer_call_id().get(),
                    "result": { "stale": true }
                }))
                .is_err(),
            "attachment commit must invalidate the old sender before Page teardown"
        );
        let completion = tokio::time::timeout(std::time::Duration::from_secs(5), response_receiver)
            .await
            .expect("replacement replay should complete")
            .expect("replacement response channel should remain open");
        assert_ne!(completion.call_id, old_correlation.renderer_call_id().get());
        assert_eq!(
            completion.renderer_agent_attachment_id(),
            Some(new_attachment)
        );
        assert_eq!(
            completion
                .output
                .protocol_response(completion.call_id)
                .expect("Console.clearMessages replay response")["result"],
            json!({})
        );

        let resolved = ctx
            .conn
            .resolve_runtime_inspector_response_ready(
                crate::conn::RuntimeInspectorResponseReady::new(
                    frontend_id,
                    Some("SID-1"),
                    Ok(completion),
                ),
            )
            .expect("current replay completion must consume its frontend correlation");
        assert_eq!(resolved.command_id(), frontend_id);
    });
}

#[test]
fn runtime_evaluate_await_promise_timer_reply_ignores_unrelated_output_stream_control() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;
        let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 9_101).await;
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 9_102,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "new Promise(resolve => setTimeout(() => resolve('timer-only'), 500))",
                "awaitPromise": true,
                "returnByValue": true
            }
        }))
        .await;

        assert!(
            !ctx.sent.iter().any(|message| message["id"] == json!(9_102)),
            "timer-backed awaitPromise should defer until the timer settles: {:?}",
            ctx.sent
        );
        let unrelated_output = ctx
            .route_renderer_publication_for_test(
                RendererOutputStreamControl::Opened {
                    stream: RendererOutputStreamIdentity::new_page_for_protocol_test(
                        PageId::new_for_testing(9_102),
                    ),
                }
                .into(),
            )
            .await;
        assert!(
            !unrelated_output
                .iter()
                .any(|message| message["id"] == json!(9_102)),
            "an unrelated stream control must not synthesize the pending awaitPromise response: {unrelated_output:?}"
        );

        let response = wait_for_response_by_id_async(&mut ctx, None, 9_102).await;
        assert_eq!(
            response["result"]["result"]["value"],
            json!("timer-only"),
            "timer-backed awaitPromise should still complete through the renderer receiver after a stale runtime wake: {response:?}"
        );
    });
}

#[test]
fn runtime_call_function_on_playwright_style_handle_result_awaits_polling_promise_until_settled() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;
        let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 20_677).await;
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 20_679,
            "method": "Runtime.evaluate",
            "params": {
                "expression": r#"(() => {
  function parseEvaluationResultValue(value, handles = [], refs = new Map()) {
    if (Object.is(value, undefined))
      return undefined;
    if (typeof value === 'object' && value) {
      if ('ref' in value)
        return refs.get(value.ref);
      if ('v' in value) {
        if (value.v === 'undefined')
          return undefined;
        if (value.v === 'null')
          return null;
        return undefined;
      }
      if ('a' in value) {
        const result = [];
        refs.set(value.id, result);
        for (const item of value.a)
          result.push(parseEvaluationResultValue(item, handles, refs));
        return result;
      }
      if ('o' in value) {
        const result = {};
        refs.set(value.id, result);
        for (const { k, v } of value.o) {
          if (k === '__proto__')
            continue;
          result[k] = parseEvaluationResultValue(v, handles, refs);
        }
        return result;
      }
      if ('h' in value)
        return handles[value.h];
    }
    return value;
  }

  class UtilityScript {
    constructor(global, isUnderTest) {
      this.global = global;
      this.isUnderTest = isUnderTest;
    }

    evaluate(isFunction, returnByValue, expression, argCount, ...argsAndHandles) {
      const args = argsAndHandles.slice(0, argCount);
      const handles = argsAndHandles.slice(argCount);
      const parameters = [];
      for (let i = 0; i < args.length; ++i)
        parameters[i] = parseEvaluationResultValue(args[i], handles);
      let result = this.global.eval(expression);
      if (isFunction === true) {
        result = result(...parameters);
      } else if (isFunction === false) {
        result = result;
      } else if (typeof result === 'function') {
        result = result(...parameters);
      }
      return returnByValue ? Promise.resolve(result).then(value => JSON.parse(JSON.stringify(value))) : result;
    }
  }

  globalThis.__lmPlaywrightInjected = {
    utils: {
      builtins: {
        requestAnimationFrame: globalThis.requestAnimationFrame.bind(globalThis),
        setTimeout: globalThis.setTimeout.bind(globalThis),
      }
    }
  };

  return new UtilityScript(globalThis, false);
})()"#
            }
        }))
        .await;
        let utility_object_id =
            take_response_by_id(&mut ctx, 20_679)["result"]["result"]["objectId"]
                .as_str()
                .map(str::to_owned)
                .expect("Runtime.evaluate should return a utilityScript object id");

        ctx.process_async(json!({
            "id": 206_710,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__lmPlaywrightInjected"
            }
        }))
        .await;
        let injected_object_id =
            take_response_by_id(&mut ctx, 206_710)["result"]["result"]["objectId"]
                .as_str()
                .map(str::to_owned)
                .expect("Runtime.evaluate should return an injected helper object id");

        ctx.process_command_only_async(json!({
            "id": 206_711,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": utility_object_id.clone(),
                "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
                "arguments": [
                    { "objectId": utility_object_id.clone() },
                    { "value": true },
                    { "value": false },
                    { "value": "(injected, { expression: expression2, isFunction: isFunction2, polling, arg: arg2 }) => {\n  let evaledExpression;\n  const predicate = () => {\n    let result2 = evaledExpression ?? globalThis.eval(expression2);\n    if (isFunction2 === true) {\n      evaledExpression = result2;\n      result2 = result2(arg2);\n    } else if (isFunction2 === false) {\n      result2 = result2;\n    } else if (typeof result2 === 'function') {\n      evaledExpression = result2;\n      result2 = result2(arg2);\n    }\n    return result2;\n  };\n  let fulfill;\n  let reject;\n  let aborted = false;\n  const result = new Promise((f, r) => {\n    fulfill = f;\n    reject = r;\n  });\n  const next = () => {\n    if (aborted)\n      return;\n    try {\n      const success = predicate();\n      if (success) {\n        fulfill(success);\n        return;\n      }\n      if (typeof polling !== 'number')\n        injected.utils.builtins.requestAnimationFrame(next);\n      else\n        injected.utils.builtins.setTimeout(next, polling);\n    } catch (e) {\n      reject(e);\n    }\n  };\n  next();\n  globalThis.__lmPlaywrightWaitHandle = { result, abort: () => aborted = true };\n  return globalThis.__lmPlaywrightWaitHandle;\n}" },
                    { "value": 2 },
                    { "value": { "h": 0 } },
                    { "value": { "o": [
                      { "k": "expression", "v": "window.__done === true" },
                      { "k": "isFunction", "v": { "v": "undefined" } },
                      { "k": "polling", "v": { "v": "undefined" } },
                      { "k": "arg", "v": { "v": "null" } }
                    ], "id": 1 } },
                    { "objectId": injected_object_id }
                ],
                "returnByValue": false,
                "awaitPromise": true
            }
        }))
        .await;
        let handle_object_id =
            take_response_by_id(&mut ctx, 206_711)["result"]["result"]["objectId"]
                .as_str()
                .map(str::to_owned)
                .expect("Runtime.callFunctionOn should return a handle object id");

        ctx.process_async(json!({
            "id": 206_712,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__done = false; setTimeout(() => { globalThis.__done = true; }, 50); 'armed'"
            }
        }))
        .await;
        let armed = take_response_by_id(&mut ctx, 206_712);
        assert_eq!(armed["result"]["result"]["value"], json!("armed"));

        ctx.process_async(json!({
            "id": 206_713,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": utility_object_id.clone(),
                "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
                "arguments": [
                    { "objectId": utility_object_id },
                    { "value": true },
                    { "value": false },
                    { "value": "(h) => h.result" },
                    { "value": 2 },
                    { "value": { "h": 0 } },
                    { "value": { "v": "undefined" } },
                    { "objectId": handle_object_id }
                ],
                "returnByValue": false,
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 206_713).await;
        if response.get("error").is_some() {
            ctx.process_async(json!({
                "id": 206_714,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "window.__done === true"
                }
            }))
            .await;
            let done_state = take_response_by_id(&mut ctx, 206_714);

            ctx.process_async(json!({
                "id": 206_715,
                "method": "Runtime.callFunctionOn",
                "params": {
                    "objectId": handle_object_id.clone(),
                    "functionDeclaration": "function() { return [typeof this.result, typeof this.result?.then]; }",
                    "returnByValue": true,
                    "awaitPromise": false
                }
            }))
            .await;
            let handle_state = take_response_by_id(&mut ctx, 206_715);

            ctx.process_async(json!({
                "id": 206_716,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "globalThis.__lmPlaywrightWaitHandle.result",
                    "awaitPromise": true
                }
            }))
            .await;
            let direct_wait = take_response_by_id(&mut ctx, 206_716);

            panic!(
                "unexpected Playwright-style await response: {response:?}; done_state={done_state:?}; handle_state={handle_state:?}; direct_wait={direct_wait:?}"
            );
        }
        assert_eq!(
            response["result"]["result"]["type"],
            json!("boolean"),
            "unexpected Playwright-style await response: {response:?}"
        );
        assert_eq!(response["result"]["result"]["value"], json!(true));
    });
}

#[test]
fn runtime_call_function_on_playwright_style_utility_promise_awaits_polling_condition() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;
        let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 206_720).await;
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 206_721,
            "method": "Runtime.evaluate",
            "params": {
                "expression": r#"(() => {
  class UtilityScript {
    constructor(global) {
      this.global = global;
    }

    evaluate(isFunction, returnByValue, expression, argCount, ...argsAndHandles) {
      const args = argsAndHandles.slice(0, argCount);
      let result = this.global.eval(expression);
      if (isFunction === true)
        result = result(...args);
      else if (isFunction === false)
        result = result;
      else if (typeof result === 'function')
        result = result(...args);
      return returnByValue ? Promise.resolve(result).then(value => JSON.parse(JSON.stringify(value))) : result;
    }
  }

  return new UtilityScript(globalThis);
})()"#
            }
        }))
        .await;
        let utility_object_id =
            take_response_by_id(&mut ctx, 206_721)["result"]["result"]["objectId"]
                .as_str()
                .map(str::to_owned)
                .expect("Runtime.evaluate should return a utilityScript object id");

        ctx.process_async(json!({
            "id": 206_722,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": utility_object_id.clone(),
                "functionDeclaration": "(utilityScript, ...args) => utilityScript.evaluate(...args)",
                "arguments": [
                    { "objectId": utility_object_id },
                    { "value": true },
                    { "value": true },
                    { "value": r#"() => {
  window.__done = false;
  setTimeout(() => { window.__done = true; }, 50);
  return new Promise(resolve => {
    const poll = () => {
      if (window.__done === true) {
        resolve(true);
        return;
      }
      requestAnimationFrame(poll);
    };
    poll();
  });
}"# },
                    { "value": 0 }
                ],
                "returnByValue": true,
                "awaitPromise": true
            }
        }))
        .await;

        let response = wait_for_response_by_id_async(&mut ctx, None, 206_722).await;
        assert_eq!(
            response["result"]["result"]["type"],
            json!("boolean"),
            "unexpected utility promise await response: {response:?}"
        );
        assert_eq!(response["result"]["result"]["value"], json!(true));
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_evaluate_await_promise_in_isolated_world_waits_for_fetch_driven_dom_change() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html><html><body>waiting<script>
setTimeout(() => {
  fetch('/api').then(r => r.text()).then(text => {
    document.body.dataset.ready = text;
  });
}, 0);
</script></body></html>"#,
        )
    }

    async fn api() -> impl IntoResponse {
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        ([(CONTENT_TYPE.as_str(), "text/plain")], "fetch-ready")
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/api", get(api)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    with_loaded_http_document_async(&mut ctx, &page_url, "SID-1", "TID-1").await;
    let _ = enable_runtime_and_take_execution_context_id_async(&mut ctx, 20_701).await;
    let utility_context_id = create_isolated_world_async(&mut ctx, 20_702, "utility").await;
    ctx.sent.clear();

    // Timer polling lets the test harness return from the command turn; rAF
    // would continuously enqueue immediate runtime work before the fetch settles.
    ctx.process_async(json!({
        "id": 20_703,
        "method": "Runtime.evaluate",
        "sessionId": "SID-1",
        "params": {
            "contextId": utility_context_id,
            "expression": r#"new Promise(resolve => {
  const poll = () => {
    const ready = document.body.dataset.ready;
    if (ready) {
      resolve(ready);
      return;
    }
    setTimeout(poll, 5);
  };
  poll();
})"#,
            "awaitPromise": true
        }
    }))
    .await;

    let response = wait_for_response_by_id_async(&mut ctx, "SID-1", 20_703).await;
    assert_eq!(
        response["result"]["result"]["type"],
        json!("string"),
        "unexpected Runtime.evaluate awaitPromise response: {response:?}"
    );
    assert_eq!(response["result"]["result"]["value"], json!("fetch-ready"));

    server.abort();
}

#[test]
fn runtime_call_function_on_await_promise_handles_sync_undefined_result() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime should build");

    runtime.block_on(async {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<html><body>ok</body></html>").await;

        ctx.process_async(json!({
            "id": 2068,
            "method": "Runtime.enable"
        }))
        .await;
        let enabled = take_response_by_id(&mut ctx, 2068);
        assert_eq!(enabled["result"], json!({}));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 2069,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "globalThis.__lmSyncStopProbe = { stop() {} }; globalThis.__lmSyncStopProbe"
            }
        }))
        .await;
        let object_id = take_response_by_id(&mut ctx, 2069)["result"]["result"]["objectId"]
            .as_str()
            .map(str::to_owned)
            .expect("Runtime.evaluate should return an objectId for the sync stop probe");

        ctx.process_async(json!({
            "id": 2070,
            "method": "Runtime.callFunctionOn",
            "params": {
                "objectId": object_id,
                "functionDeclaration": "function() { return this.stop(); }",
                "returnByValue": true,
                "awaitPromise": true
            }
        }))
        .await;

        let response = take_response_by_id(&mut ctx, 2070);
        assert_eq!(response["result"]["result"]["type"], json!("undefined"));
    });
}
