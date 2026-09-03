use crate::devtools_runtime::{DevToolsError, DevToolsErrorKind};

use crate::conn::{CdpConnection, Cmd, DevToolsLogViolationThreshold};
use crate::domains::actions::LogAction;
use crate::domains::command_output::CommandOutputPlan;
use crate::domains::log_output_state::TargetNetworkLogEntry;
use crate::domains::observable_output::log_lifecycle_error_level_and_text;
use serde::Deserialize;

pub(crate) struct TargetLogReplaySnapshot {
    pub(crate) url: String,
    pub(crate) lifecycle_errors: Vec<String>,
    pub(crate) network_entries: Vec<TargetNetworkLogEntry>,
}

pub(crate) enum SessionOwnerLogEnableResult {
    Handled {
        replay: Option<TargetLogReplaySnapshot>,
    },
    UnknownSession,
}

pub(crate) enum SessionOwnerLogControlResult {
    Handled,
    LogNotEnabled,
    UnknownSession,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartViolationsReportParams {
    config: Vec<ViolationSetting>,
}

#[derive(Deserialize)]
struct ViolationSetting {
    name: String,
    threshold: f64,
}

pub(crate) fn command_output_plan(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    match cmd.parse_action::<LogAction>() {
        Some(LogAction::Clear) => clear_command(conn, cmd),
        Some(LogAction::Enable) => enable_command(conn, cmd),
        Some(LogAction::Disable) => disable_command(conn, cmd),
        Some(LogAction::StartViolationsReport) => start_violations_report_command(conn, cmd),
        Some(LogAction::StopViolationsReport) => stop_violations_report_command(conn, cmd),
        None => CommandOutputPlan::error(-32601, "Unknown Log command-output method"),
    }
}

fn enable_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    match conn.enable_log_for_session_owner(cmd.session_id) {
        SessionOwnerLogEnableResult::Handled { replay } => {
            let mut plan = CommandOutputPlan::default();
            if let Some(replay) = replay {
                append_log_replay_snapshot(conn, &mut plan, cmd.session_id, replay);
            }
            // Blink's Log.enable synchronously replays ConsoleMessageStorage
            // before the generated dispatcher writes the command response.
            plan.push_success();
            plan
        }
        SessionOwnerLogEnableResult::UnknownSession => unknown_session_output_plan(),
    }
}

fn clear_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    if conn.clear_log_for_session_owner(cmd.session_id) {
        return CommandOutputPlan::success();
    }
    unknown_session_output_plan()
}

fn start_violations_report_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let Ok(Some(params)) = cmd.get_params::<StartViolationsReportParams>() else {
        return CommandOutputPlan::error(-32602, "Invalid parameters");
    };
    let thresholds = params
        .config
        .into_iter()
        .filter(|setting| is_known_violation_name(&setting.name))
        .map(|setting| DevToolsLogViolationThreshold {
            name: setting.name,
            threshold: setting.threshold,
        })
        .collect();
    match conn.start_log_violations_for_session_owner(cmd.session_id, thresholds) {
        SessionOwnerLogControlResult::Handled => CommandOutputPlan::success(),
        SessionOwnerLogControlResult::LogNotEnabled => {
            CommandOutputPlan::error(-32000, "Log is not enabled")
        }
        SessionOwnerLogControlResult::UnknownSession => unknown_session_output_plan(),
    }
}

fn stop_violations_report_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    if conn.stop_log_violations_for_session_owner(cmd.session_id) {
        return CommandOutputPlan::success();
    }
    unknown_session_output_plan()
}

fn is_known_violation_name(name: &str) -> bool {
    matches!(
        name,
        "longTask"
            | "longLayout"
            | "blockedEvent"
            | "blockedParser"
            | "discouragedAPIUse"
            | "handler"
            | "recurringHandler"
    )
}

fn disable_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    if conn.disable_log_for_session_owner(cmd.session_id) {
        return CommandOutputPlan::success();
    }

    unknown_session_output_plan()
}

fn unknown_session_output_plan() -> CommandOutputPlan {
    CommandOutputPlan::from_devtools_error(DevToolsError::new(
        DevToolsErrorKind::NoSuchSession,
        "Unknown sessionId",
    ))
}

fn append_log_replay_snapshot(
    conn: &mut CdpConnection,
    plan: &mut CommandOutputPlan,
    session_id: Option<&str>,
    snapshot: TargetLogReplaySnapshot,
) {
    let base_timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1_000.0)
        .unwrap_or_default();
    let mut index = 0;
    for error in snapshot.lifecycle_errors {
        index += 1;
        let (level, text) = log_lifecycle_error_level_and_text(&error);
        plan.push_background_event(crate::conn::BackgroundProtocolEvent::log_entry_added(
            session_id,
            "javascript",
            level,
            text,
            &snapshot.url,
            base_timestamp + (index as f64 * 0.001),
            None,
        ));
    }
    for entry in snapshot.network_entries {
        let request_id = entry.request_handle().and_then(|handle| {
            conn.network_request_id_for_subresource_handle_for_session_owner(session_id, handle)
        });
        plan.push_background_event(crate::conn::BackgroundProtocolEvent::log_entry_added(
            session_id,
            "network",
            "error",
            entry.text(),
            entry.url(),
            entry.timestamp_millis(),
            request_id.as_deref(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use crate::conn::BrowserContext;
    use crate::devtools_runtime::AutomationEvent;
    use crate::domains::observable_output::{
        ObservableOutputProjectionStep, ObservablePreparedOutputSlot,
        live_log_prepared_outputs_for_renderer_network_fact,
    };
    use crate::testing::{TestContext, wait_until_message, wait_until_messages};
    use moli_core::page::{
        ScriptNetworkOutputItem, SubresourceNetworkRequestHandle, SubresourceResponseStarted,
    };
    use serde_json::json;
    use url::Url;

    #[tokio::test]
    async fn log_enable_succeeds() {
        let mut ctx = TestContext::new();
        ctx.process_async(json!({"id": 1, "method": "Log.enable"}))
            .await;
        ctx.expect_result(1, json!({}), None);
    }

    async fn load_document(ctx: &mut TestContext, html: &str) {
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1".to_owned());
        bc.set_target_url("data:text/html,log-test".to_owned());
        bc.attach_active_session("SID-1".to_owned());
        ctx.conn.insert_browser_context(bc);
        ctx.install_navigation_fixture_for_session_owner(
            &format!("data:text/html,{html}"),
            Some("SID-1"),
        )
        .await;
    }

    fn loaded_lifecycle_error_contains(ctx: &TestContext, needle: &str) -> bool {
        ctx.conn
            .runtime_session_owner_slot(Some("SID-1"))
            .ok()
            .and_then(|slot| slot.observable_output_latest_source_tail())
            .is_some_and(|source| {
                source.observable_output_items().iter().any(|item| {
                    matches!(
                        item,
                        moli_core::page::ScriptObservableOutputItem::LifecycleError(text)
                            if text.contains(needle)
                    )
                })
            })
    }

    async fn load_document_with_timer_error(ctx: &mut TestContext, message: &str) {
        load_document(
            ctx,
            &format!(
                "<!doctype html><script>setTimeout(function(){{ throw new Error('{message}') }}, 0)</script>"
            ),
        )
        .await;
        wait_until_lifecycle_error_is_ingested(ctx, message).await;
        // The navigation and timer are setup for the retained Log storage.
        // Keep their independently scheduled Page lifecycle output out of the
        // command-under-test queue.
        ctx.sent.clear();
    }

    async fn wait_until_lifecycle_error_is_ingested(ctx: &mut TestContext, message: &str) {
        for _ in 0..256 {
            ctx.complete_one_ready_scheduler_input_for_test().await;
            if loaded_lifecycle_error_contains(ctx, message) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("expected loaded lifecycle error containing {message}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn log_enable_ignores_buffered_console_api_messages() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><script>console.warn('boot warning');</script>",
        )
        .await;

        ctx.process_async(json!({"id": 1, "method": "Log.enable", "sessionId": "SID-1"}))
            .await;

        ctx.expect_result(1, json!({}), Some("SID-1"));
        assert!(
            !ctx.sent
                .iter()
                .any(|message| message["method"] == json!("Log.entryAdded")),
            "console API messages should be emitted by Runtime/Console domains, not Log: {:?}",
            ctx.sent
        );
        assert!(ctx.sent.is_empty(), "unexpected messages: {:?}", ctx.sent);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn log_and_runtime_enable_route_console_api_through_runtime_only() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><body></body>").await;

        ctx.process_async(json!({"id": 1, "method": "Log.enable", "sessionId": "SID-1"}))
            .await;
        ctx.expect_result(1, json!({}), Some("SID-1"));
        ctx.process_async(json!({"id": 2, "method": "Runtime.enable", "sessionId": "SID-1"}))
            .await;
        ctx.expect_result(2, json!({}), Some("SID-1"));
        ctx.sent.clear();

        ctx.process_async(json!({
            "id": 3,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "console.log('hello from runtime only')"
            }
        }))
        .await;
        ctx.expect_result(3, json!({"result": {"type": "undefined"}}), Some("SID-1"));
        wait_until_message(
            &mut ctx,
            "SID-1",
            "Runtime.consoleAPICalled log",
            |message| {
                message["method"] == json!("Runtime.consoleAPICalled")
                    && message["params"]["type"] == json!("log")
                    && message["params"]["args"]
                        .as_array()
                        .and_then(|args| args.first())
                        .is_some_and(|arg| arg["value"] == json!("hello from runtime only"))
            },
        )
        .await;
        assert!(
            !ctx.sent
                .iter()
                .any(|message| message["method"] == json!("Log.entryAdded")),
            "console API should not produce Log.entryAdded when Runtime and Log are both enabled: {:?}",
            ctx.sent
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn log_enable_emits_buffered_lifecycle_errors() {
        let mut ctx = TestContext::new();
        load_document_with_timer_error(&mut ctx, "boot failure").await;

        ctx.process_async(json!({"id": 1, "method": "Log.enable", "sessionId": "SID-1"}))
            .await;

        ctx.expect_result(1, json!({}), Some("SID-1"));
        let log_entry = ctx.take_first_matching("Log.entryAdded lifecycle replay", |message| {
            message["method"] == json!("Log.entryAdded")
        });
        assert_eq!(log_entry["params"]["entry"]["source"], json!("javascript"));
        assert_eq!(log_entry["params"]["entry"]["level"], json!("error"));
        assert!(
            log_entry["params"]["entry"]["text"]
                .as_str()
                .is_some_and(|text| text.contains("boot failure")),
            "unexpected Log.entryAdded text: {log_entry}"
        );
        assert!(ctx.sent.is_empty(), "unexpected messages: {:?}", ctx.sent);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn log_enable_turn_outcome_preserves_buffered_log_sidecar() {
        let mut ctx = TestContext::new();
        load_document_with_timer_error(&mut ctx, "boot failure").await;

        let raw = json!({"id": 1, "method": "Log.enable", "sessionId": "SID-1"}).to_string();
        let outcome = ctx.conn.process_message_with_turn_outcome_async(&raw).await;
        let (events, _, renderer_output_predecessor) = outcome.into_protocol_event_parts();
        assert!(renderer_output_predecessor.is_none());

        assert_eq!(events.len(), 2, "expected response and replay event");
        let (log_message, log_sidecar) = events[0].clone().into_parts();
        assert_eq!(log_message["method"], json!("Log.entryAdded"));
        let Some(AutomationEvent::LogEntryAdded(log_event)) = log_sidecar else {
            panic!("Log.entryAdded replay should preserve typed sidecar");
        };
        assert!(
            log_event.text.contains("boot failure"),
            "unexpected Log.entryAdded sidecar text: {log_event:?}"
        );
        assert_eq!(log_event.level, "error");
        assert_eq!(
            log_event.url.as_deref(),
            Some(
                "data:text/html,<!doctype html><script>setTimeout(function(){ throw new Error('boot failure') }, 0)</script>"
            )
        );

        let (response, response_sidecar) = events[1].clone().into_parts();
        assert_eq!(response["id"], json!(1));
        assert!(response_sidecar.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn log_enable_replays_network_error_without_console_or_lifecycle_output() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><body></body>").await;
        let source_document = ctx
            .conn
            .runtime_session_owner_slot(Some("SID-1"))
            .expect("loaded target runtime slot")
            .committed_renderer_document_binding()
            .expect("loaded target Document binding")
            .renderer_document_identity();
        let response = SubresourceResponseStarted::new(
            SubresourceNetworkRequestHandle::new(7),
            Vec::new(),
            Url::parse("https://example.test/network-only-missing").unwrap(),
            404,
            Vec::new(),
            Vec::new(),
        )
        .with_status_text(Some("Not Found".to_owned()));
        assert!(
            ctx.conn
                .ingest_renderer_network_output_item_and_prepare_live_delivery_for_session_owner(
                    Some("SID-1"),
                    source_document,
                    &ScriptNetworkOutputItem::SubresourceResponseStarted(Box::new(response)),
                )
                .is_some()
        );

        ctx.process_async(json!({
            "id": 1,
            "method": "Log.enable",
            "sessionId": "SID-1"
        }))
        .await;

        assert_eq!(
            ctx.sent.len(),
            2,
            "network replay must precede the response"
        );
        assert_eq!(ctx.sent[0]["method"], json!("Log.entryAdded"));
        assert_eq!(ctx.sent[0]["sessionId"], json!("SID-1"));
        assert_eq!(
            ctx.sent[0]["params"]["entry"]["url"],
            json!("https://example.test/network-only-missing")
        );
        assert_eq!(
            ctx.sent[1],
            json!({"id": 1, "result": {}, "sessionId": "SID-1"})
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn log_enable_after_cross_document_commit_does_not_replay_old_network_error() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><body>old Document</body>").await;
        let source_document = ctx
            .conn
            .runtime_session_owner_slot(Some("SID-1"))
            .expect("loaded target runtime slot")
            .committed_renderer_document_binding()
            .expect("loaded target Document binding")
            .renderer_document_identity();
        let response = SubresourceResponseStarted::new(
            SubresourceNetworkRequestHandle::new(7),
            Vec::new(),
            Url::parse("https://example.test/old-document-missing").unwrap(),
            404,
            Vec::new(),
            Vec::new(),
        )
        .with_status_text(Some("Not Found".to_owned()));
        assert!(
            ctx.conn
                .ingest_renderer_network_output_item_and_prepare_live_delivery_for_session_owner(
                    Some("SID-1"),
                    source_document,
                    &ScriptNetworkOutputItem::SubresourceResponseStarted(Box::new(response)),
                )
                .is_some()
        );

        ctx.install_navigation_fixture_for_session_owner(
            "data:text/html,<!doctype html><body>replacement Document</body>",
            Some("SID-1"),
        )
        .await;
        ctx.sent.clear();
        ctx.process_async(json!({
            "id": 1,
            "method": "Log.enable",
            "sessionId": "SID-1"
        }))
        .await;

        assert_eq!(
            ctx.sent,
            [json!({"id": 1, "result": {}, "sessionId": "SID-1"})],
            "Blink clears Page ConsoleMessageStorage at main-frame commit, so late Log.enable must only replay the replacement Document"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concrete_network_error_fans_out_live_log_to_enabled_sessions_only() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><body></body>").await;
        let browser_context = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should be loaded");
        assert!(browser_context.assign_attached_session_to_target("TID-1", "SID-2".to_owned()));
        ctx.process_async(json!({"id": 1, "method": "Log.enable", "sessionId": "SID-1"}))
            .await;
        ctx.expect_result(1, json!({}), Some("SID-1"));

        let source_document = ctx
            .conn
            .runtime_session_owner_slot(Some("SID-1"))
            .expect("loaded target runtime slot")
            .committed_renderer_document_binding()
            .expect("loaded target Document binding")
            .renderer_document_identity();
        let response = SubresourceResponseStarted::new(
            SubresourceNetworkRequestHandle::new(8),
            Vec::new(),
            Url::parse("https://example.test/live-network-missing").unwrap(),
            404,
            Vec::new(),
            Vec::new(),
        )
        .with_status_text(Some("Not Found".to_owned()));
        assert!(
            ctx.conn
                .ingest_renderer_network_output_item_and_prepare_live_delivery_for_session_owner(
                    Some("SID-1"),
                    source_document,
                    &ScriptNetworkOutputItem::SubresourceResponseStarted(Box::new(response)),
                )
                .is_some()
        );

        let owner = crate::conn::CommandOwnerScope::for_session("SID-1");
        let prepared = live_log_prepared_outputs_for_renderer_network_fact(&ctx.conn, &owner);
        let mut slot = ObservablePreparedOutputSlot::from_outputs(prepared);
        let mut events = Vec::new();
        slot.emit_activity_background_events_async(
            ObservableOutputProjectionStep::Log,
            &mut ctx.conn,
            &mut events,
            Some("SID-1"),
        )
        .await;
        let messages = events
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();

        assert_eq!(
            messages.len(),
            1,
            "only the enabled Log agent may observe it"
        );
        assert_eq!(messages[0]["method"], json!("Log.entryAdded"));
        assert_eq!(messages[0]["sessionId"], json!("SID-1"));
        assert_eq!(
            messages[0]["params"]["entry"]["url"],
            json!("https://example.test/live-network-missing")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn log_events_are_not_replayed_after_first_emit() {
        let mut ctx = TestContext::new();
        load_document_with_timer_error(&mut ctx, "first failure").await;

        ctx.process_async(json!({"id": 1, "method": "Log.enable", "sessionId": "SID-1"}))
            .await;
        ctx.expect_result(1, json!({}), Some("SID-1"));
        ctx.expect_event("Log.entryAdded", None);

        ctx.process_async(json!({"id": 2, "method": "Runtime.evaluate", "sessionId": "SID-1", "params": {"expression": "1 + 1"}}))
            .await;
        ctx.expect_result(
            2,
            json!({
                "result": {
                    "type": "number",
                    "value": 2,
                }
            }),
            Some("SID-1"),
        );
        assert!(
            !ctx.sent
                .iter()
                .any(|message| message["method"] == json!("Log.entryAdded")),
            "log entries should not replay: {:?}",
            ctx.sent
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn log_disable_stops_live_entries_and_reenable_replays_shared_storage() {
        let mut ctx = TestContext::new();
        load_document_with_timer_error(&mut ctx, "before disable").await;

        ctx.process_async(json!({"id": 1, "method": "Log.enable", "sessionId": "SID-1"}))
            .await;
        ctx.expect_result(1, json!({}), Some("SID-1"));
        ctx.expect_event("Log.entryAdded", None);

        ctx.process_async(json!({"id": 2, "method": "Log.disable", "sessionId": "SID-1"}))
            .await;
        ctx.expect_result(2, json!({}), Some("SID-1"));

        ctx.process_async(json!({"id": 3, "method": "Runtime.evaluate", "sessionId": "SID-1", "params": {"expression": "setTimeout(function(){ throw new Error('during disabled') }, 0)"}}))
            .await;
        ctx.expect_result(
            3,
            json!({
                "result": {
                    "type": "number",
                }
            }),
            Some("SID-1"),
        );
        wait_until_lifecycle_error_is_ingested(&mut ctx, "during disabled").await;
        assert!(
            !ctx.sent
                .iter()
                .any(|message| message["method"] == json!("Log.entryAdded")),
            "disabled Log domain should not emit: {:?}",
            ctx.sent
        );

        ctx.sent.clear();
        ctx.process_async(json!({"id": 4, "method": "Log.enable", "sessionId": "SID-1"}))
            .await;
        // Chromium's InspectorLogAgent::InnerEnable walks the target-owned
        // ConsoleMessageStorage from the beginning on every disabled→enabled
        // transition. Therefore both the previously delivered entry and the
        // entry retained while disabled are replayed before the response.
        let replay_texts = ctx
            .sent
            .iter()
            .filter(|message| message["method"] == json!("Log.entryAdded"))
            .filter_map(|message| message["params"]["entry"]["text"].as_str())
            .collect::<Vec<_>>();
        assert!(
            replay_texts
                .iter()
                .any(|text| text.contains("before disable"))
                && replay_texts
                    .iter()
                    .any(|text| text.contains("during disabled")),
            "re-enable should replay the target's retained storage: {:?}",
            ctx.sent
        );
        assert_eq!(
            ctx.sent.last(),
            Some(&json!({"id": 4, "result": {}, "sessionId": "SID-1"})),
            "Chromium replays stored Log entries before resolving Log.enable"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn log_enable_stages_background_target_session_state() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-active".to_owned());
        bc.attach_active_session("SID-active".to_owned());
        bc.stage_background_target(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "about:blank".to_owned(),
            None,
            None,
        );
        ctx.conn.install_browser_context_fixture_for_test(bc);

        ctx.process_async(json!({
            "id": 1,
            "method": "Log.enable",
            "sessionId": "SID-background"
        }))
        .await;

        ctx.expect_result(1, json!({}), Some("SID-background"));
        let active = ctx.conn.browser_context.as_ref().expect("browser context");
        assert!(
            !active.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .log_enabled
        );
        assert!(
            active
                .background_target("TID-background")
                .filter(|target| target.has_non_default_session_state())
                .is_some_and(|state| state.devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .page_session_state
                    .log_enabled),
            "background target should stage Log.enable"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn log_replay_is_session_local_and_clear_is_target_shared() {
        let mut ctx = TestContext::new();
        load_document_with_timer_error(&mut ctx, "shared boot failure").await;
        assert!(
            ctx.conn
                .browser_context
                .as_mut()
                .expect("browser context")
                .assign_attached_session_to_target("TID-1", "SID-2".to_owned())
        );

        ctx.process_async(json!({"id": 1, "method": "Log.enable", "sessionId": "SID-1"}))
            .await;
        assert_eq!(ctx.sent[0]["method"], json!("Log.entryAdded"));
        assert_eq!(ctx.sent[0]["sessionId"], json!("SID-1"));
        assert_eq!(
            ctx.sent[1],
            json!({"id": 1, "result": {}, "sessionId": "SID-1"})
        );
        ctx.sent.clear();

        ctx.process_async(json!({"id": 2, "method": "Log.enable", "sessionId": "SID-1"}))
            .await;
        assert_eq!(
            ctx.sent,
            [json!({"id": 2, "result": {}, "sessionId": "SID-1"})],
            "repeated Log.enable must be idempotent"
        );
        ctx.sent.clear();

        ctx.process_async(json!({"id": 3, "method": "Log.enable", "sessionId": "SID-2"}))
            .await;
        assert_eq!(ctx.sent[0]["method"], json!("Log.entryAdded"));
        assert_eq!(ctx.sent[0]["sessionId"], json!("SID-2"));
        assert_eq!(
            ctx.sent[1],
            json!({"id": 3, "result": {}, "sessionId": "SID-2"})
        );
        ctx.sent.clear();

        ctx.process_async(json!({"id": 4, "method": "Log.clear", "sessionId": "SID-1"}))
            .await;
        ctx.expect_result(4, json!({}), Some("SID-1"));
        ctx.process_async(json!({"id": 5, "method": "Log.disable", "sessionId": "SID-2"}))
            .await;
        ctx.expect_result(5, json!({}), Some("SID-2"));
        ctx.process_async(json!({"id": 6, "method": "Log.enable", "sessionId": "SID-2"}))
            .await;
        assert_eq!(
            ctx.sent,
            [json!({"id": 6, "result": {}, "sessionId": "SID-2"})],
            "Log.clear from one session must clear shared target storage"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn live_log_entries_fan_out_to_each_enabled_target_session() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><body></body>").await;
        let browser_context = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should be loaded");
        assert!(browser_context.assign_attached_session_to_target("TID-1", "SID-2".to_owned()));
        assert!(browser_context.assign_attached_session_to_target("TID-1", "SID-3".to_owned()));

        ctx.process_async(json!({"id": 1, "method": "Log.enable", "sessionId": "SID-1"}))
            .await;
        ctx.expect_result(1, json!({}), Some("SID-1"));
        ctx.process_async(json!({"id": 2, "method": "Log.enable", "sessionId": "SID-2"}))
            .await;
        ctx.expect_result(2, json!({}), Some("SID-2"));

        ctx.process_async(json!({
            "id": 3,
            "method": "Runtime.evaluate",
            "sessionId": "SID-2",
            "params": {
                "expression": "setTimeout(function(){ throw new Error('multi-session live log') }, 0)"
            }
        }))
        .await;
        ctx.expect_result(3, json!({"result": {"type": "number"}}), Some("SID-2"));

        wait_until_messages(
            &mut ctx,
            "SID-2",
            "multi-session Log.entryAdded fanout",
            |messages| {
                ["SID-1", "SID-2"].into_iter().all(|session_id| {
                    messages.iter().any(|message| {
                        message["method"] == json!("Log.entryAdded")
                            && message["sessionId"] == json!(session_id)
                            && message["params"]["entry"]["text"]
                                .as_str()
                                .is_some_and(|text| text.contains("multi-session live log"))
                    })
                })
            },
        )
        .await;

        for session_id in ["SID-1", "SID-2"] {
            assert_eq!(
                ctx.sent
                    .iter()
                    .filter(|message| {
                        message["method"] == json!("Log.entryAdded")
                            && message["sessionId"] == json!(session_id)
                            && message["params"]["entry"]["text"]
                                .as_str()
                                .is_some_and(|text| text.contains("multi-session live log"))
                    })
                    .count(),
                1,
                "each enabled target session should receive the live entry exactly once"
            );
        }
        assert!(
            !ctx.sent.iter().any(|message| {
                message["method"] == json!("Log.entryAdded")
                    && message["sessionId"] == json!("SID-3")
            }),
            "a target session with Log disabled must not receive live entries: {:?}",
            ctx.sent
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn log_violation_controls_match_chromium_state_and_validation() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><body></body>").await;

        ctx.process_async(json!({"id": 1, "method": "Log.clear", "sessionId": "SID-1"}))
            .await;
        ctx.expect_result(1, json!({}), Some("SID-1"));
        ctx.process_async(
            json!({"id": 2, "method": "Log.stopViolationsReport", "sessionId": "SID-1"}),
        )
        .await;
        ctx.expect_result(2, json!({}), Some("SID-1"));
        ctx.process_async(json!({
            "id": 3,
            "method": "Log.startViolationsReport",
            "sessionId": "SID-1",
            "params": {"config": []}
        }))
        .await;
        ctx.expect_error(3, -32000, "Log is not enabled");
        ctx.process_async(json!({
            "id": 4,
            "method": "Log.startViolationsReport",
            "sessionId": "SID-1"
        }))
        .await;
        ctx.expect_error(4, -32602, "Invalid parameters");

        ctx.process_async(json!({"id": 5, "method": "Log.enable", "sessionId": "SID-1"}))
            .await;
        ctx.expect_result(5, json!({}), Some("SID-1"));
        ctx.process_async(json!({
            "id": 6,
            "method": "Log.startViolationsReport",
            "sessionId": "SID-1",
            "params": {"config": [
                {"name": "discouragedAPIUse", "threshold": -1},
                {"name": "unknownThing", "threshold": 100}
            ]}
        }))
        .await;
        ctx.expect_result(6, json!({}), Some("SID-1"));
        let thresholds = &ctx
            .conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .active_page_target()
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .log_violation_thresholds;
        assert_eq!(thresholds.len(), 1);
        assert_eq!(thresholds[0].name, "discouragedAPIUse");
        assert_eq!(thresholds[0].threshold, -1.0);

        ctx.process_async(
            json!({"id": 7, "method": "Log.stopViolationsReport", "sessionId": "SID-1"}),
        )
        .await;
        ctx.expect_result(7, json!({}), Some("SID-1"));
        assert!(
            ctx.conn
                .browser_context
                .as_ref()
                .expect("browser context")
                .active_page_target()
                .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .log_violation_thresholds
                .is_empty()
        );
    }
}
