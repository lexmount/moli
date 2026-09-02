use crate::conn::{CdpConnection, Cmd, ProfilerAction, ProfilerInspectorCommand};
use crate::domains::command_output::CommandOutputPlan;
use crate::domains::runtime::{RuntimeCommandTaskStep, start_profiler_inspector_command_dispatch};

pub(crate) fn try_start_profiler_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<RuntimeCommandTaskStep> {
    let Some(action) = cmd.parse_action::<ProfilerAction>() else {
        return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32601,
            "UnknownMethod",
        )));
    };

    let command = ProfilerInspectorCommand::from_action(action);
    Some(start_profiler_inspector_command_dispatch(
        conn, cmd, command,
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use tokio::net::TcpListener;

    use crate::conn::{
        BrowserContext, CdpCommandTaskStep, PendingCdpCommandDispatch, ProfilerAction,
        ProfilerInspectorCommand,
    };
    use crate::testing::{TestContext, wait_until_messages, wait_until_scheduler_message};

    async fn with_loaded_document_async(ctx: &mut TestContext, html: &str) {
        ctx.conn
            .insert_browser_context(BrowserContext::new("BID-profiler".into()));
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context should exist")
            .set_active_target_id("TID-profiler");
        let page = ctx
            .conn
            .load_page_via_runtime_async(&format!("data:text/html,{html}"))
            .await
            .expect("must load document for Profiler domain tests");
        let browser_context = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should exist");
        let _ = browser_context
            .active_page_target_mut()
            .runtime_slot
            .replace_loaded_page(Some(page));
    }

    async fn spawn_profiler_navigation_server() -> (String, tokio::task::JoinHandle<()>) {
        async fn plain(
            axum::extract::Query(query): axum::extract::Query<
                std::collections::HashMap<String, String>,
            >,
        ) -> String {
            let phase = query.get("phase").map(String::as_str).unwrap_or("unknown");
            format!("<!doctype html><body>{phase}</body>")
        }

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind profiler navigation test server");
        let addr = listener.local_addr().expect("profiler server addr");
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route("/plain", axum::routing::get(plain)),
            )
            .await
            .unwrap();
        });
        (format!("http://{addr}"), server)
    }

    async fn with_loaded_target_document_async(ctx: &mut TestContext, target_id: &str, html: &str) {
        let mut browser_context = BrowserContext::new("BID-profiler".into());
        browser_context.set_active_target_id(target_id);
        ctx.conn.insert_browser_context(browser_context);
        let page = ctx
            .conn
            .load_page_via_runtime_async(&format!("data:text/html,{html}"))
            .await
            .expect("must load target document for Profiler domain tests");
        let browser_context = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should exist");
        let _ = browser_context
            .active_page_target_mut()
            .runtime_slot
            .replace_loaded_page(Some(page));
    }

    async fn complete_pending_command_task_for_test(
        ctx: &mut TestContext,
        pending: PendingCdpCommandDispatch,
    ) -> Vec<serde_json::Value> {
        ctx.complete_command_task_step_for_test(CdpCommandTaskStep::Pending(Box::new(pending)))
            .await
            .0
    }

    async fn process_and_take_response(
        ctx: &mut TestContext,
        msg: serde_json::Value,
        id: u64,
    ) -> serde_json::Value {
        ctx.process_and_wait_for_response_async(msg).await;
        let pos = ctx
            .sent
            .iter()
            .position(|message| message["id"] == json!(id))
            .expect("expected response");
        ctx.sent.remove(pos)
    }

    async fn active_runtime_session_diagnostics(ctx: &mut TestContext, id: u64) -> Value {
        process_and_take_response(
            ctx,
            json!({"id": id, "method": "HeapProfiler.moliDiagnostics"}),
            id,
        )
        .await["result"]["activeBrowserContext"]["runtimeSession"]
            .clone()
    }

    async fn active_page_renderer_inspector_session_count(ctx: &mut TestContext) -> u64 {
        let response = ctx
            .conn
            .browser_context
            .as_mut()
            .and_then(|bc| bc.active_page_target_mut().runtime_slot.loaded_page_mut())
            .expect("active target should still have a loaded page")
            .runtime_heap_usage_async()
            .await
            .expect("runtime heap usage diagnostics should be available");
        u64::try_from(response.moli.runtime.inspector_session_count)
            .expect("inspector session count should fit u64")
    }

    fn profile_contains_function(profile: &serde_json::Value, function_name: &str) -> bool {
        profile
            .get("nodes")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|nodes| {
                nodes.iter().any(|node| {
                    node.get("callFrame")
                        .and_then(|call_frame| call_frame.get("functionName"))
                        .and_then(serde_json::Value::as_str)
                        == Some(function_name)
                })
            })
    }

    fn profile_contains_script_url(profile: &serde_json::Value, url_suffix: &str) -> bool {
        profile
            .get("nodes")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|nodes| {
                nodes.iter().any(|node| {
                    node.get("callFrame")
                        .and_then(|call_frame| call_frame.get("url"))
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|url| url.ends_with(url_suffix))
                })
            })
    }

    fn find_script_coverage_by_url<'a>(
        coverage_response: &'a serde_json::Value,
        url_suffix: &str,
    ) -> Option<&'a serde_json::Value> {
        coverage_response
            .get("result")
            .and_then(|result| result.get("result"))
            .and_then(serde_json::Value::as_array)?
            .iter()
            .find(|script| {
                script
                    .get("url")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|url| url.ends_with(url_suffix))
            })
    }

    fn find_coverage_function<'a>(
        script_coverage: &'a serde_json::Value,
        function_name: &str,
    ) -> Option<&'a serde_json::Value> {
        script_coverage
            .get("functions")
            .and_then(serde_json::Value::as_array)?
            .iter()
            .find(|function| {
                function
                    .get("functionName")
                    .and_then(serde_json::Value::as_str)
                    == Some(function_name)
            })
    }

    fn coverage_function_total_count(function: &serde_json::Value) -> i64 {
        function
            .get("ranges")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|range| range.get("count").and_then(serde_json::Value::as_i64))
            .sum()
    }

    fn take_matching_messages<F>(
        ctx: &mut TestContext,
        label: &str,
        mut predicate: F,
    ) -> Vec<serde_json::Value>
    where
        F: FnMut(&serde_json::Value) -> bool,
    {
        let mut messages = Vec::new();
        let mut index = 0;
        while index < ctx.sent.len() {
            if predicate(&ctx.sent[index]) {
                messages.push(ctx.sent.remove(index));
            } else {
                index += 1;
            }
        }
        assert!(!messages.is_empty(), "expected {label}");
        messages
    }

    #[test]
    fn profiler_inspector_command_descriptor_keeps_stop_dispatch_thin() {
        let command = ProfilerInspectorCommand::from_action(ProfilerAction::Stop);
        let params = json!({});

        assert_eq!(
            command
                .runtime_dispatch(Some(7), params.as_object())
                .protocol_method(),
            "Profiler.stop"
        );
    }

    #[tokio::test]
    async fn profiler_start_stop_returns_devtools_cpu_profile() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;

        let enable =
            process_and_take_response(&mut ctx, json!({"id": 1, "method": "Profiler.enable"}), 1)
                .await;
        assert_eq!(enable["result"], json!({}));

        let interval = process_and_take_response(
            &mut ctx,
            json!({
                "id": 2,
                "method": "Profiler.setSamplingInterval",
                "params": {"interval": 100}
            }),
            2,
        )
        .await;
        assert_eq!(interval["result"], json!({}));

        let start =
            process_and_take_response(&mut ctx, json!({"id": 3, "method": "Profiler.start"}), 3)
                .await;
        assert_eq!(start["result"], json!({}));

        let burn = process_and_take_response(
            &mut ctx,
            json!({
                "id": 4,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "(() => { let x = 0; for (let i = 0; i < 20000; ++i) x += Math.sqrt(i); return x > 0; })()",
                    "returnByValue": true
                }
            }),
            4,
        )
        .await;
        assert_eq!(
            burn["result"]["result"]["value"],
            json!(true),
            "Runtime.evaluate after navigation should execute in the replacement document: {burn:?}"
        );

        let stop =
            process_and_take_response(&mut ctx, json!({"id": 5, "method": "Profiler.stop"}), 5)
                .await;
        let profile = &stop["result"]["profile"];
        assert!(
            profile["startTime"].is_number(),
            "profile should include startTime: {stop:?}"
        );
        assert!(
            profile["endTime"].is_number(),
            "profile should include endTime: {stop:?}"
        );
        assert!(
            profile["nodes"]
                .as_array()
                .is_some_and(|nodes| !nodes.is_empty()),
            "profile should include CPU profile nodes: {stop:?}"
        );
    }

    #[tokio::test]
    async fn profiler_agent_state_rejects_invalid_chromium_transitions() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;

        let start_without_enable =
            process_and_take_response(&mut ctx, json!({"id": 31, "method": "Profiler.start"}), 31)
                .await;
        assert_eq!(start_without_enable["error"]["code"], json!(-32000));
        assert!(
            start_without_enable["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("Profiler is not enabled")),
            "unexpected start error: {start_without_enable:?}"
        );
        let interval = process_and_take_response(
            &mut ctx,
            json!({
                "id": 32,
                "method": "Profiler.setSamplingInterval",
                "params": {"interval": 321}
            }),
            32,
        )
        .await;
        assert_eq!(interval["result"], json!({}));

        let enable =
            process_and_take_response(&mut ctx, json!({"id": 33, "method": "Profiler.enable"}), 33)
                .await;
        assert_eq!(enable["result"], json!({}));

        let start =
            process_and_take_response(&mut ctx, json!({"id": 34, "method": "Profiler.start"}), 34)
                .await;
        assert_eq!(start["result"], json!({}));

        let rejected_interval = process_and_take_response(
            &mut ctx,
            json!({
                "id": 35,
                "method": "Profiler.setSamplingInterval",
                "params": {"interval": 654}
            }),
            35,
        )
        .await;
        assert_eq!(rejected_interval["error"]["code"], json!(-32000));
        assert!(
            rejected_interval["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("Cannot change sampling interval")),
            "unexpected setSamplingInterval error: {rejected_interval:?}"
        );

        let disable = process_and_take_response(
            &mut ctx,
            json!({"id": 36, "method": "Profiler.disable"}),
            36,
        )
        .await;
        assert_eq!(disable["result"], json!({}));
    }

    #[tokio::test]
    async fn profiler_disable_does_not_restore_an_active_profiler_agent() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body>before</body>").await;
        let baseline_count = active_page_renderer_inspector_session_count(&mut ctx).await;

        let enable = process_and_take_response(
            &mut ctx,
            json!({"id": 131, "method": "Profiler.enable"}),
            131,
        )
        .await;
        assert_eq!(enable["result"], json!({}));
        let after_enable = active_runtime_session_diagnostics(&mut ctx, 132).await;
        assert!(
            after_enable["v8InspectorStateBytes"]
                .as_u64()
                .is_some_and(|bytes| bytes > 0),
            "Profiler.enable should persist V8-owned Inspector state: {after_enable:?}"
        );

        let disable = process_and_take_response(
            &mut ctx,
            json!({"id": 133, "method": "Profiler.disable"}),
            133,
        )
        .await;
        assert_eq!(disable["result"], json!({}));
        let after_disable = active_runtime_session_diagnostics(&mut ctx, 134).await;
        assert!(
            after_disable["v8InspectorStateBytes"].is_number(),
            "Profiler.disable should leave diagnostics limited to the opaque V8 state: {after_disable:?}"
        );

        ctx.sent.clear();
        let navigate = process_and_take_response(
            &mut ctx,
            json!({
                "id": 135,
                "method": "Page.navigate",
                "params": {
                    "url": "data:text/html,<!doctype html><body>after</body>"
                }
            }),
            135,
        )
        .await;
        assert!(
            navigate["result"]["frameId"].is_string(),
            "Page.navigate should succeed after Profiler.disable: {navigate:?}"
        );
        assert_eq!(
            active_page_renderer_inspector_session_count(&mut ctx).await,
            baseline_count,
            "Profiler.disable should prevent profiler-only renderer inspector session re-establishment on later navigations"
        );
        let after_navigation = active_runtime_session_diagnostics(&mut ctx, 136).await;
        assert!(
            after_navigation.get("profilerEnabled").is_none(),
            "navigation diagnostics must not expose a writable typed Profiler projection: {after_navigation:?}"
        );
    }

    #[tokio::test]
    async fn profiler_disable_stops_frontend_and_console_profiles_like_chromium() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;

        let start_without_enable = process_and_take_response(
            &mut ctx,
            json!({"id": 341, "method": "Profiler.start"}),
            341,
        )
        .await;
        assert_eq!(start_without_enable["error"]["code"], json!(-32000));
        assert!(
            start_without_enable["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("Profiler is not enabled")),
            "Chromium enable-disable.js expects start while disabled to fail: {start_without_enable:?}"
        );

        let enable = process_and_take_response(
            &mut ctx,
            json!({"id": 342, "method": "Profiler.enable"}),
            342,
        )
        .await;
        assert_eq!(enable["result"], json!({}));

        let frontend_start = process_and_take_response(
            &mut ctx,
            json!({"id": 343, "method": "Profiler.start"}),
            343,
        )
        .await;
        assert_eq!(frontend_start["result"], json!({}));

        let console_start = process_and_take_response(
            &mut ctx,
            json!({
                "id": 344,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "console.profile('p1');",
                    "returnByValue": true
                }
            }),
            344,
        )
        .await;
        assert_eq!(console_start["result"]["result"]["value"], json!(null));
        let started = ctx.take_first_matching("Profiler.consoleProfileStarted event", |message| {
            message["method"] == json!("Profiler.consoleProfileStarted")
        });
        assert_eq!(started["params"]["title"], json!("p1"));

        let disable = process_and_take_response(
            &mut ctx,
            json!({"id": 345, "method": "Profiler.disable"}),
            345,
        )
        .await;
        assert_eq!(disable["result"], json!({}));
        assert!(
            ctx.sent
                .iter()
                .all(|message| message["method"] != json!("Profiler.consoleProfileFinished")),
            "Profiler.disable should drop active console profiles without sending stale finished events: {:?}",
            ctx.sent
        );

        let reenable = process_and_take_response(
            &mut ctx,
            json!({"id": 346, "method": "Profiler.enable"}),
            346,
        )
        .await;
        assert_eq!(reenable["result"], json!({}));

        let stop_after_disable =
            process_and_take_response(&mut ctx, json!({"id": 347, "method": "Profiler.stop"}), 347)
                .await;
        assert_eq!(stop_after_disable["error"]["code"], json!(-32000));
        assert!(
            stop_after_disable["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("No recording profiles found")),
            "Profiler.disable should have stopped the frontend profile: {stop_after_disable:?}"
        );

        let console_end_after_disable = process_and_take_response(
            &mut ctx,
            json!({
                "id": 348,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "console.profileEnd();",
                    "returnByValue": true
                }
            }),
            348,
        )
        .await;
        assert_eq!(
            console_end_after_disable["result"]["result"]["value"],
            json!(null)
        );
        assert!(
            ctx.sent
                .iter()
                .all(|message| message["method"] != json!("Profiler.consoleProfileFinished")),
            "console.profileEnd() after Profiler.disable should not finish a stale console profile: {:?}",
            ctx.sent
        );
    }

    #[tokio::test]
    async fn profiler_state_is_isolated_per_attached_target_session() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
        {
            let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
            browser_context.attach_active_session("SID-profiler-primary");
            assert!(browser_context.assign_attached_session_to_target(
                "TID-profiler",
                "SID-profiler-attached".to_owned()
            ));
        }

        let primary_enable = process_and_take_response(
            &mut ctx,
            json!({
                "id": 37,
                "sessionId": "SID-profiler-primary",
                "method": "Profiler.enable"
            }),
            37,
        )
        .await;
        assert_eq!(
            primary_enable["result"],
            json!({}),
            "primary Profiler.enable response: {primary_enable:?}"
        );

        let aux_start_without_enable = process_and_take_response(
            &mut ctx,
            json!({
                "id": 38,
                "sessionId": "SID-profiler-attached",
                "method": "Profiler.start"
            }),
            38,
        )
        .await;
        assert_eq!(aux_start_without_enable["error"]["code"], json!(-32000));
        assert!(
            aux_start_without_enable["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("Profiler is not enabled")),
            "attached session should not see primary enable state: {aux_start_without_enable:?}"
        );

        let aux_enable = process_and_take_response(
            &mut ctx,
            json!({
                "id": 39,
                "sessionId": "SID-profiler-attached",
                "method": "Profiler.enable"
            }),
            39,
        )
        .await;
        assert_eq!(aux_enable["result"], json!({}));
        let aux_start = process_and_take_response(
            &mut ctx,
            json!({
                "id": 40,
                "sessionId": "SID-profiler-attached",
                "method": "Profiler.start"
            }),
            40,
        )
        .await;
        assert_eq!(aux_start["result"], json!({}));

        let primary_stop_without_recording = process_and_take_response(
            &mut ctx,
            json!({
                "id": 48,
                "sessionId": "SID-profiler-primary",
                "method": "Profiler.stop"
            }),
            48,
        )
        .await;
        assert_eq!(
            primary_stop_without_recording["error"]["code"],
            json!(-32000)
        );
        assert!(
            primary_stop_without_recording["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("No recording profiles found")),
            "primary stop should not stop attached profile: {primary_stop_without_recording:?}"
        );

        let aux_stop = process_and_take_response(
            &mut ctx,
            json!({
                "id": 49,
                "sessionId": "SID-profiler-attached",
                "method": "Profiler.stop"
            }),
            49,
        )
        .await;
        assert!(
            aux_stop["result"]["profile"]["nodes"]
                .as_array()
                .is_some_and(|nodes| !nodes.is_empty()),
            "attached session should return its own CPU profile: {aux_stop:?}"
        );
    }

    #[tokio::test]
    async fn profiler_sampling_interval_and_coverage_are_isolated_per_attached_session() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
        {
            let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
            browser_context.attach_active_session("SID-profiler-primary");
            assert!(browser_context.assign_attached_session_to_target(
                "TID-profiler",
                "SID-profiler-attached".to_owned()
            ));
        }

        let primary_enable = process_and_take_response(
            &mut ctx,
            json!({
                "id": 130,
                "sessionId": "SID-profiler-primary",
                "method": "Profiler.enable"
            }),
            130,
        )
        .await;
        assert_eq!(primary_enable["result"], json!({}));
        let primary_interval = process_and_take_response(
            &mut ctx,
            json!({
                "id": 131,
                "sessionId": "SID-profiler-primary",
                "method": "Profiler.setSamplingInterval",
                "params": {"interval": 777}
            }),
            131,
        )
        .await;
        assert_eq!(primary_interval["result"], json!({}));
        let primary_coverage = process_and_take_response(
            &mut ctx,
            json!({
                "id": 132,
                "sessionId": "SID-profiler-primary",
                "method": "Profiler.startPreciseCoverage",
                "params": {
                    "callCount": true,
                    "detailed": false,
                    "allowTriggeredUpdates": true
                }
            }),
            132,
        )
        .await;
        assert!(
            primary_coverage["result"]["timestamp"].is_number(),
            "primary startPreciseCoverage should return timestamp: {primary_coverage:?}"
        );

        let aux_enable = process_and_take_response(
            &mut ctx,
            json!({
                "id": 133,
                "sessionId": "SID-profiler-attached",
                "method": "Profiler.enable"
            }),
            133,
        )
        .await;
        assert_eq!(aux_enable["result"], json!({}));
        let aux_interval = process_and_take_response(
            &mut ctx,
            json!({
                "id": 134,
                "sessionId": "SID-profiler-attached",
                "method": "Profiler.setSamplingInterval",
                "params": {"interval": 111}
            }),
            134,
        )
        .await;
        assert_eq!(aux_interval["result"], json!({}));
        let aux_coverage = process_and_take_response(
            &mut ctx,
            json!({
                "id": 135,
                "sessionId": "SID-profiler-attached",
                "method": "Profiler.startPreciseCoverage",
                "params": {
                    "callCount": false,
                    "detailed": true,
                    "allowTriggeredUpdates": false
                }
            }),
            135,
        )
        .await;
        assert!(
            aux_coverage["result"]["timestamp"].is_number(),
            "attached startPreciseCoverage should return timestamp: {aux_coverage:?}"
        );

        let aux_stop_coverage = process_and_take_response(
            &mut ctx,
            json!({
                "id": 136,
                "sessionId": "SID-profiler-attached",
                "method": "Profiler.stopPreciseCoverage"
            }),
            136,
        )
        .await;
        assert_eq!(aux_stop_coverage["result"], json!({}));
        let primary_take_after_aux_stop = process_and_take_response(
            &mut ctx,
            json!({
                "id": 138,
                "sessionId": "SID-profiler-primary",
                "method": "Profiler.takePreciseCoverage"
            }),
            138,
        )
        .await;
        assert!(
            primary_take_after_aux_stop["result"]["timestamp"].is_number(),
            "stopping attached coverage must not clear primary coverage: {primary_take_after_aux_stop:?}"
        );

        let primary_stop_coverage = process_and_take_response(
            &mut ctx,
            json!({
                "id": 137,
                "sessionId": "SID-profiler-primary",
                "method": "Profiler.stopPreciseCoverage"
            }),
            137,
        )
        .await;
        assert_eq!(primary_stop_coverage["result"], json!({}));
    }

    #[tokio::test]
    async fn profiler_state_is_cleared_when_attached_target_session_detaches() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;
        {
            let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
            browser_context.attach_active_session("SID-profiler-old");
            assert!(browser_context.assign_attached_session_to_target(
                "TID-profiler",
                "SID-profiler-attached-old".to_owned()
            ));
        }

        let primary_enable = process_and_take_response(
            &mut ctx,
            json!({
                "id": 50,
                "sessionId": "SID-profiler-old",
                "method": "Profiler.enable"
            }),
            50,
        )
        .await;
        assert_eq!(primary_enable["result"], json!({}));
        let primary_start = process_and_take_response(
            &mut ctx,
            json!({
                "id": 51,
                "sessionId": "SID-profiler-old",
                "method": "Profiler.start"
            }),
            51,
        )
        .await;
        assert_eq!(primary_start["result"], json!({}));

        let aux_enable = process_and_take_response(
            &mut ctx,
            json!({
                "id": 52,
                "sessionId": "SID-profiler-attached-old",
                "method": "Profiler.enable"
            }),
            52,
        )
        .await;
        assert_eq!(aux_enable["result"], json!({}));

        let detach = process_and_take_response(
            &mut ctx,
            json!({
                "id": 53,
                "method": "Target.detachFromTarget",
                "params": {
                    "targetId": "TID-profiler",
                    "sessionId": "SID-profiler-old"
                }
            }),
            53,
        )
        .await;
        assert_eq!(detach["result"], json!({}));

        {
            let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
            assert_eq!(browser_context.active_session_id(), None);
            assert!(
                browser_context
                    .active_page_target()
                    .devtools_sessions
                    .attached("SID-profiler-attached-old")
                    .is_some(),
                "detaching the primary target session must preserve independent attached sessions"
            );
        }

        {
            let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
            browser_context.attach_active_session("SID-profiler-new");
        }

        let start_without_enable = process_and_take_response(
            &mut ctx,
            json!({
                "id": 54,
                "sessionId": "SID-profiler-new",
                "method": "Profiler.start"
            }),
            54,
        )
        .await;
        assert_eq!(start_without_enable["error"]["code"], json!(-32000));
        assert!(
            start_without_enable["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("Profiler is not enabled")),
            "newly attached session should not inherit the detached session Profiler.enable state: {start_without_enable:?}"
        );
    }

    #[tokio::test]
    async fn profiler_precise_coverage_passthrough_tracks_successful_state() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(
            &mut ctx,
            "<!doctype html><script>function beforeCoverage(){ return 1; } beforeCoverage();</script>",
        )
        .await;

        let start_without_enable = process_and_take_response(
            &mut ctx,
            json!({
                "id": 41,
                "method": "Profiler.startPreciseCoverage",
                "params": {"callCount": true}
            }),
            41,
        )
        .await;
        assert_eq!(start_without_enable["error"]["code"], json!(-32000));
        assert!(
            start_without_enable["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("Profiler is not enabled")),
            "unexpected startPreciseCoverage error: {start_without_enable:?}"
        );

        let enable =
            process_and_take_response(&mut ctx, json!({"id": 42, "method": "Profiler.enable"}), 42)
                .await;
        assert_eq!(enable["result"], json!({}));

        let take_without_start = process_and_take_response(
            &mut ctx,
            json!({"id": 142, "method": "Profiler.takePreciseCoverage"}),
            142,
        )
        .await;
        assert_eq!(take_without_start["error"]["code"], json!(-32000));
        assert!(
            take_without_start["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("Precise coverage has not been started")),
            "unexpected takePreciseCoverage error before start: {take_without_start:?}"
        );

        let start_precise = process_and_take_response(
            &mut ctx,
            json!({
                "id": 43,
                "method": "Profiler.startPreciseCoverage",
                "params": {
                    "callCount": true,
                    "detailed": true,
                    "allowTriggeredUpdates": true
                }
            }),
            43,
        )
        .await;
        assert!(
            start_precise["result"]["timestamp"].is_number(),
            "startPreciseCoverage should return a timestamp: {start_precise:?}"
        );

        let run_script = process_and_take_response(
            &mut ctx,
            json!({
                "id": 44,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "function coveredAfterStart(){ return 42; } coveredAfterStart()",
                    "returnByValue": true
                }
            }),
            44,
        )
        .await;
        assert_eq!(run_script["result"]["result"]["value"], json!(42));

        let take_precise = process_and_take_response(
            &mut ctx,
            json!({"id": 45, "method": "Profiler.takePreciseCoverage"}),
            45,
        )
        .await;
        assert!(
            take_precise["result"]["timestamp"].is_number(),
            "takePreciseCoverage should return a timestamp: {take_precise:?}"
        );
        assert!(
            take_precise["result"]["result"].as_array().is_some(),
            "takePreciseCoverage should return script coverage array: {take_precise:?}"
        );

        let best_effort = process_and_take_response(
            &mut ctx,
            json!({"id": 46, "method": "Profiler.getBestEffortCoverage"}),
            46,
        )
        .await;
        assert!(
            best_effort["result"]["result"].as_array().is_some(),
            "getBestEffortCoverage should return script coverage array: {best_effort:?}"
        );

        let stop_precise = process_and_take_response(
            &mut ctx,
            json!({"id": 47, "method": "Profiler.stopPreciseCoverage"}),
            47,
        )
        .await;
        assert_eq!(stop_precise["result"], json!({}));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn profiler_precise_coverage_reports_detailed_block_ranges() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body>coverage</body>").await;

        let enable =
            process_and_take_response(&mut ctx, json!({"id": 48, "method": "Profiler.enable"}), 48)
                .await;
        assert_eq!(enable["result"], json!({}));
        let start_precise = process_and_take_response(
            &mut ctx,
            json!({
                "id": 49,
                "method": "Profiler.startPreciseCoverage",
                "params": {
                    "callCount": true,
                    "detailed": true,
                    "allowTriggeredUpdates": false
                }
            }),
            49,
        )
        .await;
        assert!(
            start_precise["result"]["timestamp"].is_number(),
            "startPreciseCoverage should return a timestamp: {start_precise:?}"
        );

        let run_script = process_and_take_response(
            &mut ctx,
            json!({
                "id": 50,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "\
            function moliProfilerCoverageBlockSmoke(value) {\
            if (value === 0) return 0;\
            if (value > 0) return value + 1;\
            return value - 1;\
            }\
            moliProfilerCoverageBlockSmoke(41)\
            //# sourceURL=moli-profiler-coverage-block.js",
                    "returnByValue": true
                }
            }),
            50,
        )
        .await;
        assert_eq!(run_script["result"]["result"]["value"], json!(42));

        let take_precise = process_and_take_response(
            &mut ctx,
            json!({"id": 56, "method": "Profiler.takePreciseCoverage"}),
            56,
        )
        .await;
        let script = find_script_coverage_by_url(&take_precise, "moli-profiler-coverage-block.js")
            .unwrap_or_else(|| {
                panic!("takePreciseCoverage should include sourceURL script: {take_precise:?}")
            });
        let function = find_coverage_function(script, "moliProfilerCoverageBlockSmoke")
            .unwrap_or_else(|| {
                panic!("takePreciseCoverage should include target function: {script:?}")
            });
        assert_eq!(
            function.get("isBlockCoverage"),
            Some(&json!(true)),
            "detailed precise coverage should report block coverage: {function:?}"
        );
        let ranges = function["ranges"]
            .as_array()
            .expect("coverage function should include ranges");
        assert!(
            ranges.len() >= 2,
            "block coverage should expose multiple ranges for branch function: {function:?}"
        );
        let counts: Vec<i64> = ranges
            .iter()
            .filter_map(|range_value| range_value.get("count").and_then(serde_json::Value::as_i64))
            .collect();
        assert!(
            counts.contains(&0) && counts.iter().any(|count| *count > 0),
            "block coverage should expose executed and unexecuted ranges: {function:?}"
        );

        let stop_precise = process_and_take_response(
            &mut ctx,
            json!({"id": 57, "method": "Profiler.stopPreciseCoverage"}),
            57,
        )
        .await;
        assert_eq!(stop_precise["result"], json!({}));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn profiler_take_precise_coverage_resets_execution_counters() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body>coverage-reset</body>").await;

        let enable =
            process_and_take_response(&mut ctx, json!({"id": 91, "method": "Profiler.enable"}), 91)
                .await;
        assert_eq!(enable["result"], json!({}));
        let start_precise = process_and_take_response(
            &mut ctx,
            json!({
                "id": 92,
                "method": "Profiler.startPreciseCoverage",
                "params": {"callCount": true, "detailed": false}
            }),
            92,
        )
        .await;
        assert!(
            start_precise["result"]["timestamp"].is_number(),
            "startPreciseCoverage should return a timestamp: {start_precise:?}"
        );

        let run_script = process_and_take_response(
            &mut ctx,
            json!({
                "id": 93,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "\
            function moliProfilerCoverageCounterResetSmoke() {\
              return 41;\
            }\
            moliProfilerCoverageCounterResetSmoke();\
            //# sourceURL=moli-profiler-coverage-counter-reset.js",
                    "returnByValue": true
                }
            }),
            93,
        )
        .await;
        assert_eq!(run_script["result"]["result"]["value"], json!(41));

        let first_take = process_and_take_response(
            &mut ctx,
            json!({"id": 94, "method": "Profiler.takePreciseCoverage"}),
            94,
        )
        .await;
        let first_script =
            find_script_coverage_by_url(&first_take, "moli-profiler-coverage-counter-reset.js")
                .unwrap_or_else(|| {
                    panic!(
                        "first takePreciseCoverage should include sourceURL script: {first_take:?}"
                    )
                });
        let first_function =
            find_coverage_function(first_script, "moliProfilerCoverageCounterResetSmoke")
                .unwrap_or_else(|| {
                    panic!(
                        "first takePreciseCoverage should include target function: {first_script:?}"
                    )
                });
        assert!(
            coverage_function_total_count(first_function) > 0,
            "first takePreciseCoverage should include executed counts: {first_function:?}"
        );

        let second_take = process_and_take_response(
            &mut ctx,
            json!({"id": 95, "method": "Profiler.takePreciseCoverage"}),
            95,
        )
        .await;
        let second_count =
            find_script_coverage_by_url(&second_take, "moli-profiler-coverage-counter-reset.js")
                .and_then(|script| {
                    find_coverage_function(script, "moliProfilerCoverageCounterResetSmoke")
                })
                .map(coverage_function_total_count)
                .unwrap_or(0);
        assert_eq!(
            second_count, 0,
            "takePreciseCoverage should not report stale execution counts until code runs again: {second_take:?}"
        );

        let rerun_script = process_and_take_response(
            &mut ctx,
            json!({
                "id": 96,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "moliProfilerCoverageCounterResetSmoke()",
                    "returnByValue": true
                }
            }),
            96,
        )
        .await;
        assert_eq!(rerun_script["result"]["result"]["value"], json!(41));

        let third_take = process_and_take_response(
            &mut ctx,
            json!({"id": 97, "method": "Profiler.takePreciseCoverage"}),
            97,
        )
        .await;
        let third_script =
            find_script_coverage_by_url(&third_take, "moli-profiler-coverage-counter-reset.js")
                .unwrap_or_else(|| {
                    panic!("third takePreciseCoverage should include sourceURL script after rerun: {third_take:?}")
                });
        let third_function =
            find_coverage_function(third_script, "moliProfilerCoverageCounterResetSmoke")
                .unwrap_or_else(|| {
                    panic!("third takePreciseCoverage should include target function after rerun: {third_script:?}")
                });
        assert!(
            coverage_function_total_count(third_function) > 0,
            "coverage counters should resume after code runs again: {third_function:?}"
        );

        let stop_precise = process_and_take_response(
            &mut ctx,
            json!({"id": 98, "method": "Profiler.stopPreciseCoverage"}),
            98,
        )
        .await;
        assert_eq!(stop_precise["result"], json!({}));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn active_precise_coverage_survives_page_navigation() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body>before</body>").await;

        let enable =
            process_and_take_response(&mut ctx, json!({"id": 51, "method": "Profiler.enable"}), 51)
                .await;
        assert_eq!(enable["result"], json!({}));
        let start_precise = process_and_take_response(
            &mut ctx,
            json!({
                "id": 52,
                "method": "Profiler.startPreciseCoverage",
                "params": {"callCount": true, "detailed": false}
            }),
            52,
        )
        .await;
        assert!(
            start_precise["result"]["timestamp"].is_number(),
            "startPreciseCoverage should succeed before navigation: {start_precise:?}"
        );

        let navigate = process_and_take_response(
            &mut ctx,
            json!({
                "id": 53,
                "method": "Page.navigate",
                "params": {
                    "url": "data:text/html,<!doctype html><script>function afterNavCoverage(){ return 7; }</script>"
                }
            }),
            53,
        )
        .await;
        assert!(
            navigate["result"]["frameId"].is_string(),
            "Page.navigate should succeed before coverage collection: {navigate:?}"
        );

        let run_script = process_and_take_response(
            &mut ctx,
            json!({
                "id": 54,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "afterNavCoverage()",
                    "returnByValue": true
                }
            }),
            54,
        )
        .await;
        assert_eq!(run_script["result"]["result"]["value"], json!(7));

        let take_precise = process_and_take_response(
            &mut ctx,
            json!({"id": 55, "method": "Profiler.takePreciseCoverage"}),
            55,
        )
        .await;
        assert!(
            take_precise.get("error").is_none(),
            "takePreciseCoverage after navigation should not fail: {take_precise:?}"
        );
        assert!(
            take_precise["result"]["result"].as_array().is_some(),
            "takePreciseCoverage should return script coverage after navigation: {take_precise:?}"
        );
        assert!(
            take_precise["result"]["result"]
                .as_array()
                .is_some_and(|scripts| scripts.iter().any(|script| {
                    script["functions"].as_array().is_some_and(|functions| {
                        functions.iter().any(|function| {
                            function["functionName"] == json!("afterNavCoverage")
                                && function["ranges"].as_array().is_some_and(|ranges| {
                                    ranges.iter().any(|range| {
                                        range["count"].as_u64().is_some_and(|count| count > 0)
                                    })
                                })
                        })
                    })
                })),
            "restored precise coverage should report executed replacement-document code: {take_precise:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn active_profiler_recording_survives_page_navigation() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body>before</body>").await;

        let enable =
            process_and_take_response(&mut ctx, json!({"id": 21, "method": "Profiler.enable"}), 21)
                .await;
        assert_eq!(enable["result"], json!({}));
        let sampling_interval = process_and_take_response(
            &mut ctx,
            json!({
                "id": 210,
                "method": "Profiler.setSamplingInterval",
                "params": {"interval": 100}
            }),
            210,
        )
        .await;
        assert_eq!(sampling_interval["result"], json!({}));
        let start =
            process_and_take_response(&mut ctx, json!({"id": 22, "method": "Profiler.start"}), 22)
                .await;
        assert_eq!(start["result"], json!({}));
        let before_diagnostics = process_and_take_response(
            &mut ctx,
            json!({"id": 28, "method": "HeapProfiler.moliDiagnostics"}),
            28,
        )
        .await;
        let before_runtime_session =
            &before_diagnostics["result"]["activeBrowserContext"]["runtimeSession"];
        assert_eq!(
            before_runtime_session["profilerCommandStateSource"],
            json!("renderer-v8-inspector-agent"),
            "Profiler command state should be owned by the renderer V8 inspector agent before navigation: {before_diagnostics:?}"
        );
        assert!(
            before_runtime_session["v8InspectorStateBytes"]
                .as_u64()
                .is_some_and(|bytes| bytes > 0),
            "successful Profiler commands should persist only the opaque V8 state: {before_diagnostics:?}"
        );
        assert!(
            before_runtime_session.get("profilerEnabled").is_none()
                && before_runtime_session
                    .get("profilerProjectionStateSource")
                    .is_none(),
            "diagnostics must not expose a second writable Profiler projection: {before_diagnostics:?}"
        );

        let before_burn = process_and_take_response(
            &mut ctx,
            json!({
                "id": 26,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "(() => { function moliProfilerBeforeNavigationWork() { let x = 0; for (let i = 0; i < 500000; ++i) x += Math.sqrt(i); return x > 0; } return moliProfilerBeforeNavigationWork(); })()\n//# sourceURL=moli-profiler-before-navigation.js",
                    "returnByValue": true
                }
            }),
            26,
        )
        .await;
        assert_eq!(before_burn["result"]["result"]["value"], json!(true));

        let navigate = process_and_take_response(
            &mut ctx,
            json!({
                "id": 23,
                "method": "Page.navigate",
                "params": {
                    "url": "data:text/html,<!doctype html><body>after</body>"
                }
            }),
            23,
        )
        .await;
        assert!(
            navigate["result"]["frameId"].is_string(),
            "Page.navigate should succeed before profiler stop: {navigate:?}"
        );
        let after_diagnostics = process_and_take_response(
            &mut ctx,
            json!({"id": 29, "method": "HeapProfiler.moliDiagnostics"}),
            29,
        )
        .await;
        let after_runtime_session =
            &after_diagnostics["result"]["activeBrowserContext"]["runtimeSession"];
        assert_eq!(
            after_runtime_session["profilerCommandStateSource"],
            before_runtime_session["profilerCommandStateSource"],
            "navigation should keep Profiler command state owned by the renderer V8 inspector agent: before={before_diagnostics:?} after={after_diagnostics:?}"
        );
        assert!(
            after_runtime_session["v8InspectorStateBytes"]
                .as_u64()
                .is_some_and(|bytes| bytes > 0),
            "navigation should retain the opaque V8 Profiler state: {after_diagnostics:?}"
        );
        assert!(
            ctx.sent
                .iter()
                .all(|message| message["method"] != json!("Runtime.executionContextCreated")),
            "Profiler-only navigation restore must not fake Runtime frontend enable events: {:?}",
            ctx.sent
        );

        let repeated_start = process_and_take_response(
            &mut ctx,
            json!({"id": 230, "method": "Profiler.start"}),
            230,
        )
        .await;
        assert_eq!(
            repeated_start["result"],
            json!({}),
            "repeating Profiler.start after navigation should remain a V8 no-op: {repeated_start:?}"
        );

        let burn = process_and_take_response(
            &mut ctx,
            json!({
                "id": 24,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "(() => { if (!document.body.textContent.includes('after')) return false; function moliProfilerAfterNavigationWork() { let x = 0; for (let i = 0; i < 500000; ++i) x += Math.sqrt(i); return x > 0; } return moliProfilerAfterNavigationWork(); })()\n//# sourceURL=moli-profiler-after-navigation.js",
                    "returnByValue": true
                }
            }),
            24,
        )
        .await;
        assert_eq!(
            burn["result"]["result"]["value"],
            json!(true),
            "Runtime.evaluate after navigation should execute in the replacement document: {burn:?}"
        );

        let second_navigate = process_and_take_response(
            &mut ctx,
            json!({
                "id": 240,
                "method": "Page.navigate",
                "params": {
                    "url": "data:text/html,<!doctype html><body>final</body>"
                }
            }),
            240,
        )
        .await;
        assert!(
            second_navigate["result"]["frameId"].is_string(),
            "second Page.navigate should preserve the active recording: {second_navigate:?}"
        );
        let final_burn = process_and_take_response(
            &mut ctx,
            json!({
                "id": 241,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "(() => { if (!document.body.textContent.includes('final')) return false; function moliProfilerFinalNavigationWork() { let x = 0; const deadline = Date.now() + 30; while (Date.now() < deadline) x += Math.sqrt(x + 2); return x > 0; } return moliProfilerFinalNavigationWork(); })()\n//# sourceURL=moli-profiler-final-navigation.js",
                    "returnByValue": true
                }
            }),
            241,
        )
        .await;
        assert_eq!(final_burn["result"]["result"]["value"], json!(true));

        let stop =
            process_and_take_response(&mut ctx, json!({"id": 25, "method": "Profiler.stop"}), 25)
                .await;
        assert!(
            stop.get("error").is_none(),
            "Profiler.stop after navigation should not fail: {stop:?}"
        );
        let profile = &stop["result"]["profile"];
        assert!(
            profile["nodes"]
                .as_array()
                .is_some_and(|nodes| !nodes.is_empty()),
            "profile should include CPU profile nodes after navigation: {stop:?}"
        );
        assert!(
            !profile_contains_script_url(profile, "moli-profiler-before-navigation.js"),
            "replacement profile must not contain samples from the first disposed isolate: {stop:?}"
        );
        assert!(
            !profile_contains_script_url(profile, "moli-profiler-after-navigation.js"),
            "replacement profile must not contain samples from the second disposed isolate: {stop:?}"
        );
        assert!(
            profile_contains_script_url(profile, "moli-profiler-final-navigation.js"),
            "Profiler.stop should return native samples from the current replacement isolate: {stop:?}"
        );
        let disable = process_and_take_response(
            &mut ctx,
            json!({"id": 27, "method": "Profiler.disable"}),
            27,
        )
        .await;
        assert_eq!(
            disable["result"],
            json!({}),
            "Profiler.disable after navigation profile stop should keep targeting the committed page: {disable:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn profiler_stop_after_navigation_does_not_require_second_start() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body>before</body>").await;

        assert_eq!(
            process_and_take_response(
                &mut ctx,
                json!({"id": 181, "method": "Profiler.enable"}),
                181
            )
            .await["result"],
            json!({})
        );
        assert_eq!(
            process_and_take_response(
                &mut ctx,
                json!({
                    "id": 182,
                    "method": "Profiler.setSamplingInterval",
                    "params": {"interval": 100}
                }),
                182,
            )
            .await["result"],
            json!({})
        );
        assert_eq!(
            process_and_take_response(
                &mut ctx,
                json!({"id": 183, "method": "Profiler.start"}),
                183
            )
            .await["result"],
            json!({})
        );
        let inspector_state = &ctx
            .conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .inspector_session_state;
        assert!(
            inspector_state.v8_state.is_some(),
            "successful Profiler commands must persist an opaque V8 state cookie"
        );
        let before = process_and_take_response(
            &mut ctx,
            json!({
                "id": 184,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "(() => { function moliProfilerNoSecondStartBefore() { let x = 0; for (let i = 0; i < 500000; ++i) x += Math.sqrt(i); return x > 0; } return moliProfilerNoSecondStartBefore(); })()\n//# sourceURL=moli-profiler-no-second-start-before.js",
                    "returnByValue": true
                }
            }),
            184,
        )
        .await;
        assert_eq!(before["result"]["result"]["value"], json!(true));

        let navigate = process_and_take_response(
            &mut ctx,
            json!({
                "id": 185,
                "method": "Page.navigate",
                "params": {
                    "url": "data:text/html,<!doctype html><body>after</body>"
                }
            }),
            185,
        )
        .await;
        assert!(
            navigate["result"]["frameId"].is_string(),
            "Page.navigate should commit before Profiler.stop: {navigate:?}"
        );

        let after = process_and_take_response(
            &mut ctx,
            json!({
                "id": 186,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "(() => { if (!document.body.textContent.includes('after')) return false; function moliProfilerNoSecondStartAfter() { let x = 0; for (let i = 0; i < 500000; ++i) x += Math.sqrt(i + 1); return x > 0; } return moliProfilerNoSecondStartAfter(); })()\n//# sourceURL=moli-profiler-no-second-start-after.js",
                    "returnByValue": true
                }
            }),
            186,
        )
        .await;
        assert_eq!(after["result"]["result"]["value"], json!(true));

        let stop =
            process_and_take_response(&mut ctx, json!({"id": 187, "method": "Profiler.stop"}), 187)
                .await;
        assert!(
            stop.get("error").is_none(),
            "Profiler.stop after navigation must restore from the opaque cookie without a typed projection or second Profiler.start: {stop:?}"
        );
        let profile = &stop["result"]["profile"];
        assert!(
            !profile_contains_script_url(profile, "moli-profiler-no-second-start-before.js"),
            "cross-isolate restore should not migrate samples from before navigation: {stop:?}"
        );
        assert!(
            profile_contains_script_url(profile, "moli-profiler-no-second-start-after.js"),
            "profile should include recording after automatic replacement restore: {stop:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn attached_profiler_recording_survives_page_navigation() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body>before</body>").await;
        {
            let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
            browser_context.attach_active_session("SID-profiler-primary");
            assert!(browser_context.assign_attached_session_to_target(
                "TID-profiler",
                "SID-profiler-attached".to_owned()
            ));
        }

        assert_eq!(
            process_and_take_response(
                &mut ctx,
                json!({
                    "id": 220,
                    "sessionId": "SID-profiler-attached",
                    "method": "Profiler.enable"
                }),
                220
            )
            .await["result"],
            json!({})
        );
        assert_eq!(
            process_and_take_response(
                &mut ctx,
                json!({
                    "id": 221,
                    "sessionId": "SID-profiler-attached",
                    "method": "Profiler.setSamplingInterval",
                    "params": {"interval": 100}
                }),
                221,
            )
            .await["result"],
            json!({})
        );
        assert_eq!(
            process_and_take_response(
                &mut ctx,
                json!({
                    "id": 222,
                    "sessionId": "SID-profiler-attached",
                    "method": "Profiler.start"
                }),
                222
            )
            .await["result"],
            json!({})
        );

        let before = process_and_take_response(
            &mut ctx,
            json!({
                "id": 223,
                "sessionId": "SID-profiler-attached",
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "(() => { function moliAuxProfilerBeforeNavigation() { let x = 0; for (let i = 0; i < 500000; ++i) x += Math.sqrt(i); return x > 0; } return moliAuxProfilerBeforeNavigation(); })()\n//# sourceURL=moli-aux-profiler-before-navigation.js",
                    "returnByValue": true
                }
            }),
            223,
        )
        .await;
        assert_eq!(before["result"]["result"]["value"], json!(true));

        let navigate = process_and_take_response(
            &mut ctx,
            json!({
                "id": 224,
                "sessionId": "SID-profiler-attached",
                "method": "Page.navigate",
                "params": {
                    "url": "data:text/html,<!doctype html><body>after</body>"
                }
            }),
            224,
        )
        .await;
        assert!(
            navigate["result"]["frameId"].is_string(),
            "attached Page.navigate should commit before Profiler.stop: {navigate:?}"
        );

        let after = process_and_take_response(
            &mut ctx,
            json!({
                "id": 225,
                "sessionId": "SID-profiler-attached",
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "(() => { if (!document.body.textContent.includes('after')) return false; function moliAuxProfilerAfterNavigation() { let x = 0; for (let i = 0; i < 500000; ++i) x += Math.sqrt(i + 1); return x > 0; } return moliAuxProfilerAfterNavigation(); })()\n//# sourceURL=moli-aux-profiler-after-navigation.js",
                    "returnByValue": true
                }
            }),
            225,
        )
        .await;
        assert_eq!(after["result"]["result"]["value"], json!(true));
        let stop = process_and_take_response(
            &mut ctx,
            json!({
                "id": 226,
                "sessionId": "SID-profiler-attached",
                "method": "Profiler.stop"
            }),
            226,
        )
        .await;
        assert!(
            stop.get("error").is_none(),
            "attached Profiler.stop after navigation should not fail: {stop:?}"
        );
        let profile = &stop["result"]["profile"];
        assert!(
            !profile_contains_script_url(profile, "moli-aux-profiler-before-navigation.js"),
            "attached restore should not migrate old-isolate samples: {stop:?}"
        );
        assert!(
            profile_contains_script_url(profile, "moli-aux-profiler-after-navigation.js"),
            "attached profile should include work from the replacement Profiler agent: {stop:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn inactive_context_attached_profiler_recording_survives_page_navigation() {
        let mut ctx = TestContext::new();

        let default_context = process_and_take_response(
            &mut ctx,
            json!({"id": 228, "method": "Target.createBrowserContext"}),
            228,
        )
        .await;
        assert!(
            default_context["result"]["browserContextId"].is_string(),
            "default browser context should be created: {default_context:?}"
        );
        let inactive_context = process_and_take_response(
            &mut ctx,
            json!({"id": 229, "method": "Target.createBrowserContext"}),
            229,
        )
        .await;
        let inactive_browser_context_id = inactive_context["result"]["browserContextId"]
            .as_str()
            .expect("inactive browser context id")
            .to_owned();

        let create_target = process_and_take_response(
            &mut ctx,
            json!({
                "id": 230,
                "method": "Target.createTarget",
                "params": {
                    "browserContextId": inactive_browser_context_id,
                    "url": "about:blank"
                }
            }),
            230,
        )
        .await;
        let target_id = create_target["result"]["targetId"]
            .as_str()
            .expect("target id")
            .to_owned();

        let browser_attach = process_and_take_response(
            &mut ctx,
            json!({"id": 231, "method": "Target.attachToBrowserTarget"}),
            231,
        )
        .await;
        let browser_session_id = browser_attach["result"]["sessionId"]
            .as_str()
            .expect("browser session id")
            .to_owned();

        let attach = process_and_take_response(
            &mut ctx,
            json!({
                "id": 232,
                "sessionId": browser_session_id,
                "method": "Target.attachToTarget",
                "params": {
                    "targetId": target_id,
                    "flatten": true
                }
            }),
            232,
        )
        .await;
        let profiler_session_id = attach["result"]["sessionId"]
            .as_str()
            .expect("profiler session id")
            .to_owned();

        assert_eq!(
            process_and_take_response(
                &mut ctx,
                json!({
                    "id": 233,
                    "sessionId": profiler_session_id,
                    "method": "Profiler.enable"
                }),
                233
            )
            .await["result"],
            json!({})
        );
        assert_eq!(
            process_and_take_response(
                &mut ctx,
                json!({
                    "id": 234,
                    "sessionId": profiler_session_id,
                    "method": "Profiler.setSamplingInterval",
                    "params": {"interval": 100}
                }),
                234,
            )
            .await["result"],
            json!({})
        );
        assert_eq!(
            process_and_take_response(
                &mut ctx,
                json!({
                    "id": 235,
                    "sessionId": profiler_session_id,
                    "method": "Profiler.start"
                }),
                235
            )
            .await["result"],
            json!({})
        );
        let before = process_and_take_response(
            &mut ctx,
            json!({
                "id": 236,
                "sessionId": profiler_session_id,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "(() => { function moliInactiveAuxProfilerBeforeNavigation() { let x = 0; for (let i = 0; i < 500000; ++i) x += Math.sqrt(i); return x > 0; } return moliInactiveAuxProfilerBeforeNavigation(); })()\n//# sourceURL=moli-inactive-aux-profiler-before-navigation.js",
                    "returnByValue": true
                }
            }),
            236,
        )
        .await;
        assert_eq!(before["result"]["result"]["value"], json!(true));

        let navigate = process_and_take_response(
            &mut ctx,
            json!({
                "id": 237,
                "sessionId": profiler_session_id,
                "method": "Page.navigate",
                "params": {
                    "url": "data:text/html,<!doctype html><body>after</body>"
                }
            }),
            237,
        )
        .await;
        assert!(
            navigate["result"]["frameId"].is_string(),
            "inactive attached Page.navigate should commit before Profiler.stop: {navigate:?}"
        );

        let after = process_and_take_response(
            &mut ctx,
            json!({
                "id": 238,
                "sessionId": profiler_session_id,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "(() => { if (!document.body.textContent.includes('after')) return false; function moliInactiveAuxProfilerAfterNavigation() { let x = 0; for (let i = 0; i < 500000; ++i) x += Math.sqrt(i + 1); return x > 0; } return moliInactiveAuxProfilerAfterNavigation(); })()\n//# sourceURL=moli-inactive-aux-profiler-after-navigation.js",
                    "returnByValue": true
                }
            }),
            238,
        )
        .await;
        assert_eq!(after["result"]["result"]["value"], json!(true));

        let stop = process_and_take_response(
            &mut ctx,
            json!({
                "id": 239,
                "sessionId": profiler_session_id,
                "method": "Profiler.stop"
            }),
            239,
        )
        .await;
        assert!(
            stop.get("error").is_none(),
            "inactive attached Profiler.stop after navigation should not fail: {stop:?}"
        );
        let profile = &stop["result"]["profile"];
        assert!(
            !profile_contains_script_url(
                profile,
                "moli-inactive-aux-profiler-before-navigation.js"
            ),
            "inactive attached restore should not migrate old-isolate samples: {stop:?}"
        );
        assert!(
            profile_contains_script_url(profile, "moli-inactive-aux-profiler-after-navigation.js"),
            "inactive attached profile should include work from the replacement Profiler agent: {stop:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn active_profiler_recording_survives_http_page_navigation_and_disable() {
        let (fixture, _server) = spawn_profiler_navigation_server().await;
        let mut ctx = TestContext::new();
        ctx.conn
            .insert_browser_context(BrowserContext::new("BID-profiler-http".into()));
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context should exist")
            .set_active_target_id("TID-profiler-http");
        let page = ctx
            .conn
            .load_page_via_runtime_async(&format!("{fixture}/plain?phase=before"))
            .await
            .expect("must load initial HTTP document");
        let browser_context = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should exist");
        let _ = browser_context
            .active_page_target_mut()
            .runtime_slot
            .replace_loaded_page(Some(page));

        assert_eq!(
            process_and_take_response(&mut ctx, json!({"id": 200, "method": "Page.enable"}), 200)
                .await["result"],
            json!({})
        );
        assert_eq!(
            process_and_take_response(
                &mut ctx,
                json!({"id": 201, "method": "Profiler.enable"}),
                201
            )
            .await["result"],
            json!({})
        );
        assert_eq!(
            process_and_take_response(
                &mut ctx,
                json!({
                    "id": 202,
                    "method": "Profiler.setSamplingInterval",
                    "params": { "interval": 100 }
                }),
                202,
            )
            .await["result"],
            json!({})
        );
        assert_eq!(
            process_and_take_response(
                &mut ctx,
                json!({"id": 203, "method": "Profiler.start"}),
                203
            )
            .await["result"],
            json!({})
        );
        let before = process_and_take_response(
            &mut ctx,
            json!({
                "id": 204,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "(() => { function moliProfilerHttpBeforeNavigationWork() { let x = 0; for (let i = 0; i < 500000; ++i) x += Math.sqrt(i); return x > 0; } return moliProfilerHttpBeforeNavigationWork(); })()\n//# sourceURL=moli-profiler-http-before-navigation.js",
                    "returnByValue": true
                }
            }),
            204,
        )
        .await;
        assert_eq!(before["result"]["result"]["value"], json!(true));

        let navigate = process_and_take_response(
            &mut ctx,
            json!({
                "id": 205,
                "method": "Page.navigate",
                "params": { "url": format!("{fixture}/plain?phase=after") }
            }),
            205,
        )
        .await;
        assert!(
            navigate["result"]["frameId"].is_string(),
            "HTTP Page.navigate should succeed: {navigate:?}"
        );
        wait_until_scheduler_message(
            &mut ctx,
            "HTTP profiler navigation DOMContentLoaded event",
            |message| message["method"] == json!("Page.domContentEventFired"),
        )
        .await;
        ctx.sent.clear();

        let after = process_and_take_response(
            &mut ctx,
            json!({
                "id": 206,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "(() => { if (!document.body.textContent.includes('after')) return false; function moliProfilerHttpAfterNavigationWork() { let x = 0; for (let i = 0; i < 500000; ++i) x += Math.sqrt(i + 1); return x > 0; } return moliProfilerHttpAfterNavigationWork(); })()\n//# sourceURL=moli-profiler-http-after-navigation.js",
                    "returnByValue": true
                }
            }),
            206,
        )
        .await;
        assert_eq!(after["result"]["result"]["value"], json!(true));

        let stop =
            process_and_take_response(&mut ctx, json!({"id": 207, "method": "Profiler.stop"}), 207)
                .await;
        assert!(
            stop.get("error").is_none(),
            "Profiler.stop after HTTP navigation should not fail: {stop:?}"
        );
        let profile = &stop["result"]["profile"];
        assert!(
            !profile_contains_script_url(profile, "moli-profiler-http-before-navigation.js"),
            "HTTP navigation restore should not migrate old-isolate samples: {stop:?}"
        );
        assert!(
            profile_contains_script_url(profile, "moli-profiler-http-after-navigation.js"),
            "HTTP navigation profile should include post-navigation function: {stop:?}"
        );

        let disable = process_and_take_response(
            &mut ctx,
            json!({"id": 208, "method": "Profiler.disable"}),
            208,
        )
        .await;
        assert_eq!(
            disable["result"],
            json!({}),
            "Profiler.disable after HTTP navigation profile stop should keep targeting the committed page: {disable:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn active_profiler_recording_does_not_survive_target_close_reuse() {
        let mut ctx = TestContext::new();
        let target_id = "TID-profiler-target-close";
        with_loaded_target_document_async(
            &mut ctx,
            target_id,
            "<!doctype html><body>before target close</body>",
        )
        .await;

        let enable = process_and_take_response(
            &mut ctx,
            json!({"id": 121, "method": "Profiler.enable"}),
            121,
        )
        .await;
        assert_eq!(enable["result"], json!({}));
        let start = process_and_take_response(
            &mut ctx,
            json!({"id": 122, "method": "Profiler.start"}),
            122,
        )
        .await;
        assert_eq!(start["result"], json!({}));
        let burn = process_and_take_response(
            &mut ctx,
            json!({
                "id": 123,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": "(() => { let x = 0; for (let i = 0; i < 20000; ++i) x += Math.sqrt(i); return x > 0; })()",
                    "returnByValue": true
                }
            }),
            123,
        )
        .await;
        assert_eq!(burn["result"]["result"]["value"], json!(true));

        let close = process_and_take_response(
            &mut ctx,
            json!({
                "id": 124,
                "method": "Target.closeTarget",
                "params": { "targetId": target_id }
            }),
            124,
        )
        .await;
        assert_eq!(close["result"], json!({ "success": true }));
        assert!(
            ctx.sent.is_empty(),
            "target close without an attached session should not leave unrelated events: {:?}",
            ctx.sent
        );

        {
            let browser_context = ctx
                .conn
                .browser_context
                .as_mut()
                .expect("browser context should survive target close");
            browser_context.set_active_target_id(target_id);
        }
        let page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<!doctype html><body>after target close</body>",
            )
            .await
            .expect("same target id should be reusable after close");
        let browser_context = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should exist after reload");
        let _ = browser_context
            .active_page_target_mut()
            .runtime_slot
            .replace_loaded_page(Some(page));

        let stop_after_target_reuse =
            process_and_take_response(&mut ctx, json!({"id": 125, "method": "Profiler.stop"}), 125)
                .await;
        assert_eq!(stop_after_target_reuse["error"]["code"], json!(-32000));
        assert!(
            stop_after_target_reuse["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("No recording profiles found")),
            "Target.closeTarget must release the target-scoped renderer Profiler agent instead of leaking the old recording into a later same-key target: {stop_after_target_reuse:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn console_profile_stack_is_not_migrated_across_page_isolates() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body>before</body>").await;

        let enable = process_and_take_response(
            &mut ctx,
            json!({"id": 101, "method": "Profiler.enable"}),
            101,
        )
        .await;
        assert_eq!(enable["result"], json!({}));

        let start_console_profile = process_and_take_response(
            &mut ctx,
            json!({
                "id": 102,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": r#"
(() => {
  console.profile("console-profile-across-navigation");
  return true;
})()
"#,
                    "returnByValue": true
                }
            }),
            102,
        )
        .await;
        assert_eq!(
            start_console_profile["result"]["result"]["value"],
            json!(true)
        );
        let started = ctx.take_first_matching("Profiler.consoleProfileStarted event", |message| {
            message["method"] == json!("Profiler.consoleProfileStarted")
        });
        assert_eq!(
            started["params"]["title"],
            json!("console-profile-across-navigation")
        );

        let navigate = process_and_take_response(
            &mut ctx,
            json!({
                "id": 103,
                "method": "Page.navigate",
                "params": {
                    "url": "data:text/html,<!doctype html><body>after</body>"
                }
            }),
            103,
        )
        .await;
        assert!(
            navigate["result"]["frameId"].is_string(),
            "Page.navigate should succeed while a console profile is active: {navigate:?}"
        );

        let finish_console_profile = process_and_take_response(
            &mut ctx,
            json!({
                "id": 104,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": r#"
(() => {
  function moliConsoleProfileAfterNavigationWork() {
    let value = 0;
    for (let i = 0; i < 500000; i += 1)
      value += Math.sqrt(i + 1);
    return value > 0;
  }
  const result = moliConsoleProfileAfterNavigationWork();
  console.profileEnd("console-profile-across-navigation");
  return result;
})()
"#,
                    "returnByValue": true
                }
            }),
            104,
        )
        .await;
        assert_eq!(
            finish_console_profile["result"]["result"]["value"],
            json!(true),
            "Runtime.evaluate after navigation should execute in the replacement document: {finish_console_profile:?}"
        );

        assert!(
            ctx.sent
                .iter()
                .all(|message| message["method"] != json!("Profiler.consoleProfileFinished")),
            "V8 does not serialize active console-profile entries across inspector backends: {:?}",
            ctx.sent
        );

        let replacement_console_profile = process_and_take_response(
            &mut ctx,
            json!({
                "id": 105,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": r#"
(() => {
  function moliReplacementConsoleProfileWork() {
    console.profile("console-profile-after-navigation");
    let value = 0;
    for (let i = 0; i < 500000; i += 1)
      value += Math.sqrt(i + 1);
    console.profileEnd("console-profile-after-navigation");
    return value > 0;
  }
  return moliReplacementConsoleProfileWork();
})()
"#,
                    "returnByValue": true
                }
            }),
            105,
        )
        .await;
        assert_eq!(
            replacement_console_profile["result"]["result"]["value"],
            json!(true)
        );
        let replacement_started = ctx.take_first_matching(
            "replacement Profiler.consoleProfileStarted event",
            |message| {
                message["method"] == json!("Profiler.consoleProfileStarted")
                    && message["params"]["title"] == json!("console-profile-after-navigation")
            },
        );
        let replacement_finished = ctx.take_first_matching(
            "replacement Profiler.consoleProfileFinished event",
            |message| {
                message["method"] == json!("Profiler.consoleProfileFinished")
                    && message["params"]["title"] == json!("console-profile-after-navigation")
            },
        );
        assert_eq!(
            replacement_finished["params"]["id"], replacement_started["params"]["id"],
            "replacement-isolate console profile should retain its own start/end identity"
        );
        assert!(
            profile_contains_function(
                &replacement_finished["params"]["profile"],
                "moliReplacementConsoleProfileWork",
            ),
            "restored Profiler.enable should support a fresh console profile: {replacement_finished:?}"
        );

        let stop_without_frontend_recording =
            process_and_take_response(&mut ctx, json!({"id": 106, "method": "Profiler.stop"}), 106)
                .await;
        assert_eq!(
            stop_without_frontend_recording["error"]["code"],
            json!(-32000)
        );
        assert!(
            stop_without_frontend_recording["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("No recording profiles found")),
            "console profiles should not create a frontend Profiler.start recording: {stop_without_frontend_recording:?}"
        );
    }

    #[tokio::test]
    async fn profiler_commands_can_use_pending_dispatch_path() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;

        let raw = json!({"id": 11, "method": "Profiler.enable"}).to_string();
        let pending = ctx
            .conn
            .try_start_pending_command_dispatch(&raw)
            .expect("Profiler.enable should dispatch through V8 inspector");
        let messages = complete_pending_command_task_for_test(&mut ctx, pending).await;

        assert!(
            messages
                .iter()
                .any(|message| message["id"] == json!(11) && message["result"] == json!({})),
            "Profiler.enable should return inspector success: {messages:?}"
        );
    }

    #[tokio::test]
    async fn console_profile_uses_v8_inspector_console_profile_lifecycle() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;

        let enable =
            process_and_take_response(&mut ctx, json!({"id": 61, "method": "Profiler.enable"}), 61)
                .await;
        assert_eq!(enable["result"], json!({}));

        let interval = process_and_take_response(
            &mut ctx,
            json!({
                "id": 62,
                "method": "Profiler.setSamplingInterval",
                "params": {"interval": 100}
            }),
            62,
        )
        .await;
        assert_eq!(interval["result"], json!({}));

        let evaluate = process_and_take_response(
            &mut ctx,
            json!({
                "id": 63,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": r#"
(() => {
  function moliConsoleProfileWork() {
    console.profile('moli-console-profile');
    let total = 0;
    for (let i = 0; i < 500000; ++i)
      total += Math.sqrt(i + 1);
    console.profileEnd('moli-console-profile');
    return total > 0;
  }
  return moliConsoleProfileWork();
})()
"#,
                    "returnByValue": true
                }
            }),
            63,
        )
        .await;
        assert_eq!(evaluate["result"]["result"]["value"], json!(true));

        let started = ctx.take_first_matching("Profiler.consoleProfileStarted event", |message| {
            message["method"] == json!("Profiler.consoleProfileStarted")
        });
        assert_eq!(started["params"]["title"], json!("moli-console-profile"));
        assert!(
            started["params"]["id"].is_string(),
            "consoleProfileStarted should include a V8 profile id: {started:?}"
        );

        let finished = ctx
            .take_first_matching("Profiler.consoleProfileFinished event", |message| {
                message["method"] == json!("Profiler.consoleProfileFinished")
            });
        assert_eq!(finished["params"]["title"], json!("moli-console-profile"));
        assert_eq!(
            finished["params"]["id"], started["params"]["id"],
            "console profile finished id should match started id"
        );
        let profile = &finished["params"]["profile"];
        assert!(
            profile["nodes"]
                .as_array()
                .is_some_and(|nodes| !nodes.is_empty()),
            "consoleProfileFinished should include a CPU profile: {finished:?}"
        );
        assert!(
            profile_contains_function(profile, "moliConsoleProfileWork"),
            "console profile should include sampled page work: {finished:?}"
        );

        let stop_without_frontend_recording =
            process_and_take_response(&mut ctx, json!({"id": 64, "method": "Profiler.stop"}), 64)
                .await;
        assert_eq!(
            stop_without_frontend_recording["error"]["code"],
            json!(-32000)
        );
        assert!(
            stop_without_frontend_recording["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("No recording profiles found")),
            "console.profile should not create a frontend Profiler.start recording: {stop_without_frontend_recording:?}"
        );
    }

    #[tokio::test]
    async fn console_profile_events_fan_out_to_attached_page_profiler_sessions() {
        let mut ctx = TestContext::new();
        with_loaded_target_document_async(
            &mut ctx,
            "TID-page-console-profile-fanout",
            "<!doctype html><body></body>",
        )
        .await;
        {
            let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
            browser_context.attach_active_session("SID-page-console-profile-primary");
            assert!(browser_context.assign_attached_session_to_target(
                "TID-page-console-profile-fanout",
                "SID-page-console-profile-attached".to_owned(),
            ));
        }
        ctx.sent.clear();

        for (command_id, session_id) in [
            (91_101, "SID-page-console-profile-primary"),
            (91_102, "SID-page-console-profile-attached"),
        ] {
            let enable = process_and_take_response(
                &mut ctx,
                json!({
                    "id": command_id,
                    "method": "Profiler.enable",
                    "sessionId": session_id
                }),
                command_id,
            )
            .await;
            assert_eq!(enable["sessionId"], json!(session_id));
            assert_eq!(enable["result"], json!({}));
        }
        ctx.sent.clear();

        let evaluate = process_and_take_response(
            &mut ctx,
            json!({
                "id": 91_103,
                "method": "Runtime.evaluate",
                "sessionId": "SID-page-console-profile-primary",
                "params": {
                    "expression": r#"
(() => {
  function moliPageConsoleProfileFanoutWork() {
    console.profile("page-console-profile-fanout");
    let value = 0;
    for (let i = 0; i < 200000; i += 1)
      value += Math.sqrt(i + 1);
    console.profileEnd("page-console-profile-fanout");
    return value > 0;
  }
  return moliPageConsoleProfileFanoutWork();
})()
"#,
                    "returnByValue": true
                }
            }),
            91_103,
        )
        .await;
        assert_eq!(
            evaluate["sessionId"],
            json!("SID-page-console-profile-primary")
        );
        assert_eq!(evaluate["result"]["result"]["value"], json!(true));

        wait_until_messages(
            &mut ctx,
            None,
            "page console profile events for both attached sessions",
            |messages| {
                [
                    (
                        "SID-page-console-profile-primary",
                        "Profiler.consoleProfileStarted",
                    ),
                    (
                        "SID-page-console-profile-primary",
                        "Profiler.consoleProfileFinished",
                    ),
                    (
                        "SID-page-console-profile-attached",
                        "Profiler.consoleProfileStarted",
                    ),
                    (
                        "SID-page-console-profile-attached",
                        "Profiler.consoleProfileFinished",
                    ),
                ]
                .into_iter()
                .all(|(session_id, method)| {
                    messages.iter().any(|message| {
                        message["sessionId"] == json!(session_id)
                            && message["method"] == json!(method)
                    })
                })
            },
        )
        .await;

        for session_id in [
            "SID-page-console-profile-primary",
            "SID-page-console-profile-attached",
        ] {
            let started =
                ctx.take_first_matching("page Profiler.consoleProfileStarted event", |message| {
                    message["sessionId"] == json!(session_id)
                        && message["method"] == json!("Profiler.consoleProfileStarted")
                });
            assert_eq!(
                started["params"]["title"],
                json!("page-console-profile-fanout")
            );
            assert!(
                started["params"]["id"].is_string(),
                "consoleProfileStarted should include a V8 profile id: {started:?}"
            );

            let finished =
                ctx.take_first_matching("page Profiler.consoleProfileFinished event", |message| {
                    message["sessionId"] == json!(session_id)
                        && message["method"] == json!("Profiler.consoleProfileFinished")
                });
            assert_eq!(
                finished["params"]["title"],
                json!("page-console-profile-fanout")
            );
            assert_eq!(
                finished["params"]["id"], started["params"]["id"],
                "console profile finished id should match started id for the same attached session"
            );
            assert!(
                profile_contains_function(
                    &finished["params"]["profile"],
                    "moliPageConsoleProfileFanoutWork",
                ),
                "page console profile should include sampled page work for {session_id}: {finished:?}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn console_profile_detached_page_session_drops_active_profile_without_stale_finish() {
        let mut ctx = TestContext::new();
        with_loaded_target_document_async(
            &mut ctx,
            "TID-page-console-profile-detach",
            "<!doctype html><body></body>",
        )
        .await;
        {
            let browser_context = ctx.conn.browser_context.as_mut().expect("browser context");
            browser_context.attach_active_session("SID-page-console-profile-detach-primary");
            assert!(browser_context.assign_attached_session_to_target(
                "TID-page-console-profile-detach",
                "SID-page-console-profile-detach-attached".to_owned(),
            ));
        }
        ctx.sent.clear();

        for (command_id, session_id) in [
            (91_201, "SID-page-console-profile-detach-primary"),
            (91_202, "SID-page-console-profile-detach-attached"),
        ] {
            let enable = process_and_take_response(
                &mut ctx,
                json!({
                    "id": command_id,
                    "method": "Profiler.enable",
                    "sessionId": session_id
                }),
                command_id,
            )
            .await;
            assert_eq!(enable["sessionId"], json!(session_id));
            assert_eq!(enable["result"], json!({}));
        }
        ctx.sent.clear();

        let start_profile = process_and_take_response(
            &mut ctx,
            json!({
                "id": 91_203,
                "method": "Runtime.evaluate",
                "sessionId": "SID-page-console-profile-detach-primary",
                "params": {
                    "expression": r#"
(() => {
  console.profile("page-console-profile-detach");
  return true;
})()
"#,
                    "returnByValue": true
                }
            }),
            91_203,
        )
        .await;
        assert_eq!(
            start_profile["sessionId"],
            json!("SID-page-console-profile-detach-primary")
        );
        assert_eq!(start_profile["result"]["result"]["value"], json!(true));

        wait_until_messages(
            &mut ctx,
            None,
            "page console profile start events for both attached sessions",
            |messages| {
                [
                    "SID-page-console-profile-detach-primary",
                    "SID-page-console-profile-detach-attached",
                ]
                .into_iter()
                .all(|session_id| {
                    messages.iter().any(|message| {
                        message["sessionId"] == json!(session_id)
                            && message["method"] == json!("Profiler.consoleProfileStarted")
                    })
                })
            },
        )
        .await;

        let primary_started =
            ctx.take_first_matching("primary Profiler.consoleProfileStarted event", |message| {
                message["sessionId"] == json!("SID-page-console-profile-detach-primary")
                    && message["method"] == json!("Profiler.consoleProfileStarted")
            });
        let aux_started =
            ctx.take_first_matching("attached Profiler.consoleProfileStarted event", |message| {
                message["sessionId"] == json!("SID-page-console-profile-detach-attached")
                    && message["method"] == json!("Profiler.consoleProfileStarted")
            });
        assert_eq!(
            primary_started["params"]["title"],
            json!("page-console-profile-detach")
        );
        assert_eq!(
            aux_started["params"]["title"],
            json!("page-console-profile-detach")
        );
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 91_204,
            "method": "Target.detachFromTarget",
            "params": {
                "targetId": "TID-page-console-profile-detach",
                "sessionId": "SID-page-console-profile-detach-attached"
            }
        }))
        .await;
        ctx.expect_result(91_204, json!({}), None);
        ctx.expect_event(
            "Target.detachedFromTarget",
            Some(&json!({
                "targetId": "TID-page-console-profile-detach",
                "sessionId": "SID-page-console-profile-detach-attached"
            })),
        );
        ctx.sent.clear();

        let finish_profile = process_and_take_response(
            &mut ctx,
            json!({
                "id": 91_205,
                "method": "Runtime.evaluate",
                "sessionId": "SID-page-console-profile-detach-primary",
                "params": {
                    "expression": r#"
(() => {
  function moliDetachedPageConsoleProfileWork() {
    let value = 0;
    for (let i = 0; i < 300000; i += 1)
      value += Math.sqrt(i + 1);
    console.profileEnd("page-console-profile-detach");
    return value > 0;
  }
  return moliDetachedPageConsoleProfileWork();
})()
"#,
                    "returnByValue": true
                }
            }),
            91_205,
        )
        .await;
        assert_eq!(
            finish_profile["sessionId"],
            json!("SID-page-console-profile-detach-primary")
        );
        assert_eq!(finish_profile["result"]["result"]["value"], json!(true));

        wait_until_messages(
            &mut ctx,
            Some("SID-page-console-profile-detach-primary"),
            "primary page console profile finish after attached detach",
            |messages| {
                messages.iter().any(|message| {
                    message["sessionId"] == json!("SID-page-console-profile-detach-primary")
                        && message["method"] == json!("Profiler.consoleProfileFinished")
                })
            },
        )
        .await;

        let primary_finished =
            ctx.take_first_matching("primary Profiler.consoleProfileFinished event", |message| {
                message["sessionId"] == json!("SID-page-console-profile-detach-primary")
                    && message["method"] == json!("Profiler.consoleProfileFinished")
            });
        assert_eq!(
            primary_finished["params"]["title"],
            json!("page-console-profile-detach")
        );
        assert_eq!(
            primary_finished["params"]["id"], primary_started["params"]["id"],
            "remaining attached session should finish its own console profile"
        );
        assert!(
            profile_contains_function(
                &primary_finished["params"]["profile"],
                "moliDetachedPageConsoleProfileWork",
            ),
            "remaining attached session should keep sampling after attached detach: {primary_finished:?}"
        );
        assert!(
            ctx.sent.iter().all(|message| {
                !(message["sessionId"] == json!("SID-page-console-profile-detach-attached")
                    && message["method"] == json!("Profiler.consoleProfileFinished"))
            }),
            "detached session must not receive a stale consoleProfileFinished event: {:?}",
            ctx.sent
        );
    }

    #[tokio::test]
    async fn console_profile_supports_chromium_nested_profiles_and_numeric_titles() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;

        let enable =
            process_and_take_response(&mut ctx, json!({"id": 71, "method": "Profiler.enable"}), 71)
                .await;
        assert_eq!(enable["result"], json!({}));
        let interval = process_and_take_response(
            &mut ctx,
            json!({
                "id": 72,
                "method": "Profiler.setSamplingInterval",
                "params": {"interval": 100}
            }),
            72,
        )
        .await;
        assert_eq!(interval["result"], json!({}));

        let evaluate = process_and_take_response(
            &mut ctx,
            json!({
                "id": 73,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": r#"
(() => {
  function collectProfiles() {
    function moliNestedConsoleProfileBurn(seed) {
      let total = seed;
      for (let i = 0; i < 500000; ++i)
        total += Math.sqrt(i + seed);
      return total > 0;
    }
    console.profile('outer');
    moliNestedConsoleProfileBurn(1);
    console.profile(42);
    moliNestedConsoleProfileBurn(2);
    console.profileEnd('outer');
    moliNestedConsoleProfileBurn(3);
    console.profileEnd(42);
    return true;
  }
  return collectProfiles();
})()
"#,
                    "returnByValue": true
                }
            }),
            73,
        )
        .await;
        assert_eq!(evaluate["result"]["result"]["value"], json!(true));

        let finished = take_matching_messages(
            &mut ctx,
            "Profiler.consoleProfileFinished events",
            |message| message["method"] == json!("Profiler.consoleProfileFinished"),
        );
        assert_eq!(
            finished.len(),
            2,
            "Chromium console-profile.js expects two finished profiles: {finished:?}"
        );
        assert!(
            finished
                .iter()
                .any(|message| message["params"]["title"] == json!("outer")),
            "outer console profile should finish: {finished:?}"
        );
        let numeric_profile = finished
            .iter()
            .find(|message| message["params"]["title"] == json!("42"))
            .expect("numeric console profile title should stringify to \"42\"");
        assert!(
            profile_contains_function(&numeric_profile["params"]["profile"], "collectProfiles"),
            "numeric nested profile should include collectProfiles like Chromium console-profile.js: {numeric_profile:?}"
        );
    }

    #[tokio::test]
    async fn console_profile_end_without_title_finishes_unnamed_profile() {
        let mut ctx = TestContext::new();
        with_loaded_document_async(&mut ctx, "<!doctype html><body></body>").await;

        let enable =
            process_and_take_response(&mut ctx, json!({"id": 81, "method": "Profiler.enable"}), 81)
                .await;
        assert_eq!(enable["result"], json!({}));

        let evaluate = process_and_take_response(
            &mut ctx,
            json!({
                "id": 82,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": r#"
(() => {
  function collectProfiles() {
    console.profile();
    console.profile('titled');
    console.profileEnd('titled');
    console.profileEnd();
    return true;
  }
  return collectProfiles();
})()
"#,
                    "returnByValue": true
                }
            }),
            82,
        )
        .await;
        assert_eq!(evaluate["result"]["result"]["value"], json!(true));

        let finished = take_matching_messages(
            &mut ctx,
            "Profiler.consoleProfileFinished events",
            |message| message["method"] == json!("Profiler.consoleProfileFinished"),
        );
        assert_eq!(
            finished.len(),
            2,
            "Chromium console-profileEnd-parameterless-crash.js expects two finished profiles: {finished:?}"
        );
        assert!(
            finished
                .iter()
                .any(|message| message["params"]["title"] == json!("titled")),
            "titled console profile should finish when followed by parameterless profileEnd: {finished:?}"
        );

        let stop_without_frontend_recording =
            process_and_take_response(&mut ctx, json!({"id": 83, "method": "Profiler.stop"}), 83)
                .await;
        assert_eq!(
            stop_without_frontend_recording["error"]["code"],
            json!(-32000)
        );
        assert!(
            stop_without_frontend_recording["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("No recording profiles found")),
            "console.profileEnd() should not leave frontend recording state: {stop_without_frontend_recording:?}"
        );
    }
}
