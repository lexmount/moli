use serde_json::{Map, Value};

use crate::conn::{CdpConnection, Cmd};
use crate::domains::runtime::{RuntimeCommandTaskStep, start_debugger_inspector_command_dispatch};

pub(crate) fn try_start_debugger_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<RuntimeCommandTaskStep> {
    let mut message = Map::new();
    if let Some(command_id) = cmd.id {
        message.insert("id".to_owned(), Value::from(command_id));
    }
    message.insert("method".to_owned(), Value::String(cmd.method.to_owned()));
    if let Some(params) = cmd.params {
        message.insert("params".to_owned(), Value::Object(params.clone()));
    }
    Some(start_debugger_inspector_command_dispatch(
        conn,
        cmd,
        Value::Object(message).to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::{Value, json};

    use crate::conn::BrowserContext;
    use crate::testing::TestContext;

    async fn with_loaded_document(ctx: &mut TestContext) {
        ctx.conn
            .insert_browser_context(BrowserContext::new("BID-debugger".into()));
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .set_active_target_id("TID-debugger");
        let page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<body>debugger</body>")
            .await
            .expect("load debugger test page");
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .active_page_state_mut()
            .runtime_slot
            .replace_loaded_page(Some(page));
    }

    async fn command(ctx: &mut TestContext, message: Value, command_id: u64) -> Value {
        ctx.process_and_wait_for_response_async(message).await;
        let position = ctx
            .sent
            .iter()
            .position(|message| message["id"] == json!(command_id))
            .expect("command response");
        ctx.sent.remove(position)
    }

    #[tokio::test]
    async fn debugger_script_events_and_source_use_v8_inspector() {
        let mut ctx = TestContext::new();
        with_loaded_document(&mut ctx).await;

        let enable = command(&mut ctx, json!({"id": 1, "method": "Debugger.enable"}), 1).await;
        assert!(
            enable["result"]["debuggerId"]
                .as_str()
                .is_some_and(|id| !id.is_empty()),
            "Debugger.enable should return V8's debugger id: {enable:?}"
        );

        let evaluate = command(
            &mut ctx,
            json!({
                "id": 2,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "function moliDebuggerSource() { return 42; }\n//# sourceURL=moli-debugger-source.js"
                }
            }),
            2,
        )
        .await;
        assert!(evaluate.get("error").is_none(), "{evaluate:?}");

        let parsed = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Debugger.scriptParsed")
                    && message["params"]["url"] == json!("moli-debugger-source.js")
            })
            .cloned()
            .expect("Debugger.scriptParsed for evaluated source");
        let script_id = parsed["params"]["scriptId"]
            .as_str()
            .expect("script id")
            .to_owned();
        let source = command(
            &mut ctx,
            json!({
                "id": 3,
                "method": "Debugger.getScriptSource",
                "params": {"scriptId": script_id}
            }),
            3,
        )
        .await;
        assert!(
            source["result"]["scriptSource"]
                .as_str()
                .is_some_and(|source| source.contains("moliDebuggerSource")),
            "{source:?}"
        );
    }

    #[tokio::test]
    async fn debugger_interruptible_source_lookup_addresses_suspended_renderer() {
        let mut ctx = TestContext::new();
        with_loaded_document(&mut ctx).await;

        let enable = command(&mut ctx, json!({"id": 71, "method": "Debugger.enable"}), 71).await;
        assert!(enable.get("error").is_none(), "{enable:?}");
        let evaluate = command(
            &mut ctx,
            json!({
                "id": 72,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "function moliSuspendedDebuggerSource() { return 72; }\n//# sourceURL=moli-suspended-debugger-source.js"
                }
            }),
            72,
        )
        .await;
        assert!(evaluate.get("error").is_none(), "{evaluate:?}");
        let script_id = ctx
            .sent
            .iter()
            .find(|message| {
                message["method"] == json!("Debugger.scriptParsed")
                    && message["params"]["url"] == json!("moli-suspended-debugger-source.js")
            })
            .and_then(|message| message["params"]["scriptId"].as_str())
            .map(str::to_owned)
            .expect("Debugger.scriptParsed for the suspended-renderer source");

        let navigation = ctx
            .conn
            .start_document_navigation_for_session_owner(
                None,
                "LOADER-debugger-interrupt".to_owned(),
            )
            .expect("start cross-Document navigation");
        assert!(
            ctx.conn
                .renderer_document_navigation_is_suspended_for_session_owner(None)
        );

        let source = command(
            &mut ctx,
            json!({
                "id": 73,
                "method": "Debugger.getScriptSource",
                "params": {"scriptId": script_id}
            }),
            73,
        )
        .await;
        assert!(
            source["result"]["scriptSource"]
                .as_str()
                .is_some_and(|source| source.contains("moliSuspendedDebuggerSource")),
            "interruptible Debugger command must address the suspended renderer: {source:?}"
        );

        let _ = ctx
            .conn
            .finish_renderer_document_navigation_for_session_owner(None, &navigation);
        ctx.conn
            .clear_pending_document_navigation_for_session_owner_if_loader_matches(
                None,
                &navigation.loader_id,
            );
    }

    #[tokio::test]
    async fn debugger_resume_dispatches_while_renderer_owner_is_paused() {
        let mut ctx = TestContext::new();
        with_loaded_document(&mut ctx).await;

        let enable = command(&mut ctx, json!({"id": 11, "method": "Debugger.enable"}), 11).await;
        assert!(enable.get("error").is_none(), "{enable:?}");
        let timer = command(
            &mut ctx,
            json!({
                "id": 12,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "setTimeout(() => { debugger; }, 50); true"
                }
            }),
            12,
        )
        .await;
        assert_eq!(timer["result"]["result"]["value"], json!(true));

        ctx.wait_for_scheduler_message("Debugger.paused", |message| {
            message["method"] == json!("Debugger.paused")
                && message["params"]["callFrames"]
                    .as_array()
                    .is_some_and(|frames| !frames.is_empty())
        })
        .await;

        let resume = command(&mut ctx, json!({"id": 13, "method": "Debugger.resume"}), 13).await;
        assert_eq!(resume["result"], json!({}), "{resume:?}");
        ctx.wait_for_scheduler_message("Debugger.resumed", |message| {
            message["method"] == json!("Debugger.resumed")
        })
        .await;
    }

    #[tokio::test]
    async fn debugger_pause_responds_and_target_close_cancels_pending_pause() {
        let mut ctx = TestContext::new();
        with_loaded_document(&mut ctx).await;

        let enable = command(&mut ctx, json!({"id": 21, "method": "Debugger.enable"}), 21).await;
        assert!(enable.get("error").is_none(), "{enable:?}");
        let timer = command(
            &mut ctx,
            json!({
                "id": 22,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "setInterval(() => { globalThis.__debuggerPauseTick = (globalThis.__debuggerPauseTick || 0) + 1; }, 50); true"
                }
            }),
            22,
        )
        .await;
        assert_eq!(timer["result"]["result"]["value"], json!(true));

        let response_start = ctx.sent.len();
        let scheduler_events = ctx
            .process_command_only_async(json!({"id": 22, "method": "Debugger.pause"}))
            .await;
        ctx.wait_for_test_command_response(22, response_start).await;
        let pause_position = ctx
            .sent
            .iter()
            .position(|message| message["id"] == json!(22))
            .expect("Debugger.pause response must be flushed before deferred renderer activity");
        let pause = ctx.sent.remove(pause_position);
        assert_eq!(
            pause["result"],
            json!({}),
            "Debugger.pause must respond before the next page statement enters the pause loop: {pause:?}"
        );
        assert!(
            scheduler_events.is_empty(),
            "Debugger.pause should not enqueue command-completed snapshot work before its response"
        );
        let close = tokio::time::timeout(
            Duration::from_secs(2),
            command(
                &mut ctx,
                json!({
                    "id": 24,
                    "method": "Target.closeTarget",
                    "params": {"targetId": "TID-debugger"}
                }),
                24,
            ),
        )
        .await
        .expect("Target.closeTarget must cancel a pending debugger pause");
        assert_eq!(close["result"], json!({"success": true}), "{close:?}");
    }

    #[tokio::test]
    async fn debugger_instrumentation_breakpoint_ignores_internal_snapshot_scripts() {
        let mut ctx = TestContext::new();
        with_loaded_document(&mut ctx).await;

        let enable = command(&mut ctx, json!({"id": 31, "method": "Debugger.enable"}), 31).await;
        assert!(enable.get("error").is_none(), "{enable:?}");

        let set_breakpoint = command(
            &mut ctx,
            json!({
                "id": 32,
                "method": "Debugger.setInstrumentationBreakpoint",
                "params": {"instrumentation": "beforeScriptExecution"}
            }),
            32,
        )
        .await;
        assert!(
            set_breakpoint["result"]["breakpointId"]
                .as_str()
                .is_some_and(|id| !id.is_empty()),
            "{set_breakpoint:?}"
        );
        assert!(
            ctx.sent
                .iter()
                .all(|message| message["method"] != json!("Debugger.paused")),
            "protocol snapshot bookkeeping must not trigger instrumentation pauses: {:?}",
            ctx.sent
        );

        let disable = command(
            &mut ctx,
            json!({"id": 33, "method": "Debugger.disable"}),
            33,
        )
        .await;
        assert_eq!(disable["result"], json!({}), "{disable:?}");
    }

    #[tokio::test]
    async fn debugger_instrumentation_navigation_publishes_new_context_with_bound_origin() {
        let mut ctx = TestContext::new();
        with_loaded_document(&mut ctx).await;

        let runtime = command(&mut ctx, json!({"id": 34, "method": "Runtime.enable"}), 34).await;
        assert_eq!(runtime["result"], json!({}), "{runtime:?}");
        let debugger = command(&mut ctx, json!({"id": 35, "method": "Debugger.enable"}), 35).await;
        assert!(debugger.get("error").is_none(), "{debugger:?}");
        let breakpoint = command(
            &mut ctx,
            json!({
                "id": 36,
                "method": "Debugger.setInstrumentationBreakpoint",
                "params": {"instrumentation": "beforeScriptExecution"}
            }),
            36,
        )
        .await;
        assert!(breakpoint.get("error").is_none(), "{breakpoint:?}");
        ctx.sent.clear();

        let navigate = command(
            &mut ctx,
            json!({
                "id": 37,
                "method": "Page.navigate",
                "params": {
                    "url": "data:text/html,<script>globalThis.__instrumentedNavigation = true</script>"
                }
            }),
            37,
        )
        .await;
        assert!(
            navigate["result"]["frameId"].as_str().is_some(),
            "{navigate:?}"
        );
        let parsed = ctx
            .wait_for_scheduler_message(
                "instrumented replacement scriptParsed after Page.navigate response",
                |message| {
                    message["method"] == json!("Debugger.scriptParsed")
                        && message["params"]["url"]
                            == json!(
                                "data:text/html,<script>globalThis.__instrumentedNavigation = true</script>"
                            )
                },
            )
            .await;

        let created = ctx
            .sent
            .iter()
            .find(|message| message["method"] == json!("Runtime.executionContextCreated"))
            .expect("replacement default context must precede its instrumentation pause");
        assert_eq!(created["params"]["context"]["origin"], json!("://"));
        assert_eq!(
            parsed["params"]["url"],
            json!("data:text/html,<script>globalThis.__instrumentedNavigation = true</script>"),
            "the instrumented replacement script must reach V8"
        );
    }

    #[tokio::test]
    async fn debugger_paused_auxiliary_session_detach_wakes_owner() {
        let mut ctx = TestContext::new();
        with_loaded_document(&mut ctx).await;
        {
            let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
            browser_context.attach_active_session("SID-debugger-primary");
            assert!(
                browser_context.assign_auxiliary_session_to_target(
                    "TID-debugger",
                    "SID-debugger-aux".to_owned(),
                )
            );
        }

        let enable = command(
            &mut ctx,
            json!({
                "id": 41,
                "sessionId": "SID-debugger-aux",
                "method": "Debugger.enable"
            }),
            41,
        )
        .await;
        assert!(enable.get("error").is_none(), "{enable:?}");
        let timer = command(
            &mut ctx,
            json!({
                "id": 42,
                "sessionId": "SID-debugger-aux",
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "setTimeout(() => { debugger; globalThis.__afterDebuggerDetach = 1; }, 50); true"
                }
            }),
            42,
        )
        .await;
        assert_eq!(timer["result"]["result"]["value"], json!(true));

        ctx.wait_for_scheduler_message("auxiliary Debugger.paused", |message| {
            message["method"] == json!("Debugger.paused")
                && message["sessionId"] == json!("SID-debugger-aux")
        })
        .await;

        let detach = tokio::time::timeout(
            Duration::from_secs(2),
            command(
                &mut ctx,
                json!({
                    "id": 43,
                    "method": "Target.detachFromTarget",
                    "params": {
                        "targetId": "TID-debugger",
                        "sessionId": "SID-debugger-aux"
                    }
                }),
                43,
            ),
        )
        .await
        .expect("detaching a paused Debugger session must wake the renderer owner");
        assert_eq!(detach["result"], json!({}), "{detach:?}");

        let continued = command(
            &mut ctx,
            json!({
                "id": 44,
                "sessionId": "SID-debugger-primary",
                "method": "Runtime.evaluate",
                "params": {"expression": "globalThis.__afterDebuggerDetach"}
            }),
            44,
        )
        .await;
        assert_eq!(continued["result"]["result"]["value"], json!(1));
    }

    #[tokio::test]
    async fn debugger_pause_events_keep_multi_session_routes() {
        let mut ctx = TestContext::new();
        with_loaded_document(&mut ctx).await;
        {
            let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
            browser_context.attach_active_session("SID-debugger-primary");
            assert!(
                browser_context.assign_auxiliary_session_to_target(
                    "TID-debugger",
                    "SID-debugger-aux".to_owned(),
                )
            );
        }

        for (id, session_id) in [(61, "SID-debugger-primary"), (62, "SID-debugger-aux")] {
            let enable = command(
                &mut ctx,
                json!({
                    "id": id,
                    "sessionId": session_id,
                    "method": "Debugger.enable"
                }),
                id,
            )
            .await;
            assert!(enable.get("error").is_none(), "{enable:?}");
        }
        let timer = command(
            &mut ctx,
            json!({
                "id": 63,
                "sessionId": "SID-debugger-primary",
                "method": "Runtime.evaluate",
                "params": {"expression": "setTimeout(() => { debugger; }, 50); true"}
            }),
            63,
        )
        .await;
        assert_eq!(timer["result"]["result"]["value"], json!(true));

        for session_id in ["SID-debugger-primary", "SID-debugger-aux"] {
            ctx.wait_for_scheduler_message("multi-session Debugger.paused", |message| {
                message["method"] == json!("Debugger.paused")
                    && message["sessionId"] == json!(session_id)
            })
            .await;
        }

        let resume = command(
            &mut ctx,
            json!({
                "id": 64,
                "sessionId": "SID-debugger-primary",
                "method": "Debugger.resume"
            }),
            64,
        )
        .await;
        assert_eq!(resume["result"], json!({}), "{resume:?}");
        for session_id in ["SID-debugger-primary", "SID-debugger-aux"] {
            ctx.wait_for_scheduler_message("multi-session Debugger.resumed", |message| {
                message["method"] == json!("Debugger.resumed")
                    && message["sessionId"] == json!(session_id)
            })
            .await;
        }
    }

    #[tokio::test]
    async fn debugger_enable_state_is_restored_after_navigation() {
        let mut ctx = TestContext::new();
        with_loaded_document(&mut ctx).await;

        let enable = command(&mut ctx, json!({"id": 51, "method": "Debugger.enable"}), 51).await;
        assert!(enable.get("error").is_none(), "{enable:?}");
        ctx.sent.clear();

        let navigate = command(
            &mut ctx,
            json!({
                "id": 52,
                "method": "Page.navigate",
                "params": {
                    "url": "data:text/html,<body>debugger after navigation</body>"
                }
            }),
            52,
        )
        .await;
        assert!(
            navigate["result"]["frameId"].as_str().is_some(),
            "{navigate:?}"
        );

        let evaluate = command(
            &mut ctx,
            json!({
                "id": 53,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "globalThis.__debuggerAfterNavigation = 1;\n//# sourceURL=debugger-after-navigation.js"
                }
            }),
            53,
        )
        .await;
        assert!(evaluate.get("error").is_none(), "{evaluate:?}");
        assert!(
            ctx.sent.iter().any(|message| {
                message["method"] == json!("Debugger.scriptParsed")
                    && message["params"]["url"] == json!("debugger-after-navigation.js")
            }),
            "replacement PageVM should restore Debugger.enable before later scripts: {:?}",
            ctx.sent
        );

        let disable = command(
            &mut ctx,
            json!({"id": 54, "method": "Debugger.disable"}),
            54,
        )
        .await;
        assert_eq!(disable["result"], json!({}), "{disable:?}");
    }
}
