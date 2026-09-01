use crate::conn::{CdpConnection, Cmd, CommandOwnerScope};

use crate::domains::actions::ConsoleAction;
use crate::domains::command_output::CommandOutputPlan;
use crate::domains::runtime::{RuntimeCommandTaskStep, start_console_inspector_command_dispatch};

pub(crate) fn try_start_console_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<RuntimeCommandTaskStep> {
    let Some(action) = cmd.parse_action::<ConsoleAction>() else {
        return Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
            -32601,
            "Unknown Console command-output method",
        )));
    };

    let should_dispatch_to_renderer = console_command_has_renderer_agent(conn, cmd);
    if should_dispatch_to_renderer {
        return Some(start_console_inspector_command_dispatch(conn, cmd, action));
    }

    if apply_console_output_state_for_session(conn, cmd.session_id, action) {
        return Some(RuntimeCommandTaskStep::Complete(
            CommandOutputPlan::success(),
        ));
    }
    Some(RuntimeCommandTaskStep::Complete(CommandOutputPlan::error(
        -31998,
        "BrowserContextNotLoaded",
    )))
}

fn console_command_has_renderer_agent(conn: &CdpConnection, cmd: &Cmd<'_>) -> bool {
    if cmd.session_id.is_some_and(|session_id| {
        conn.shared_worker_target_for_session(Some(session_id))
            .is_some()
    }) {
        return true;
    }
    conn.runtime_session_owner_slot(cmd.session_id)
        .is_ok_and(|slot| slot.has_loaded_page())
}

pub(crate) fn apply_console_output_state_for_session(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    action: ConsoleAction,
) -> bool {
    match action {
        ConsoleAction::Enable => set_console_enabled(conn, session_id, true),
        ConsoleAction::Disable => set_console_enabled(conn, session_id, false),
        ConsoleAction::ClearMessages => clear_console_messages(conn, session_id),
    }
}

pub(crate) fn apply_console_output_state_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    action: ConsoleAction,
) -> bool {
    if owner.session_id().is_some_and(|session_id| {
        conn.shared_worker_target_for_session(Some(session_id))
            .is_some()
            || conn
                .service_worker_target_for_session(Some(session_id))
                .is_some()
    }) {
        return apply_console_output_state_for_session(conn, owner.session_id(), action);
    }
    match action {
        ConsoleAction::Enable => conn.set_console_enabled_for_owner(owner, true),
        ConsoleAction::Disable => conn.set_console_enabled_for_owner(owner, false),
        ConsoleAction::ClearMessages => conn.clear_console_messages_for_owner(owner),
    }
}

fn set_console_enabled(conn: &mut CdpConnection, session_id: Option<&str>, enabled: bool) -> bool {
    if let Some(session_id) = session_id
        && let Some(target) = conn.shared_worker_target_for_session_mut(Some(session_id))
    {
        target.set_console_enabled(session_id, enabled);
        return true;
    }
    if let Some(session_id) = session_id
        && let Some(target) = conn.service_worker_target_for_session_mut(Some(session_id))
    {
        target.set_console_enabled(session_id, enabled);
        return true;
    }
    conn.set_console_enabled_for_session_owner(session_id, enabled)
}

fn clear_console_messages(conn: &mut CdpConnection, session_id: Option<&str>) -> bool {
    if let Some(session_id) = session_id
        && let Some(target) = conn.shared_worker_target_for_session_mut(Some(session_id))
    {
        target.clear_console_messages(session_id);
        return true;
    }
    if let Some(session_id) = session_id
        && let Some(target) = conn.service_worker_target_for_session_mut(Some(session_id))
    {
        target.clear_console_messages(session_id);
        return true;
    }
    conn.clear_console_messages_for_session_owner(session_id)
}

#[cfg(test)]
pub(in crate::domains) struct ConsoleActivityEmissionSnapshot {
    observable: crate::domains::console_output_state::ConsoleLogEmissionSnapshot,
}

#[cfg(test)]
pub(in crate::domains) struct ConsoleActivitySource {
    pub(in crate::domains) observable:
        crate::domains::console_output_state::ConsoleLogEmissionSnapshot,
}

#[cfg(test)]
impl ConsoleActivityEmissionSnapshot {
    pub(in crate::domains) fn is_empty(&self) -> bool {
        self.observable.is_empty()
    }

    pub(in crate::domains) fn observable(
        &self,
    ) -> &crate::domains::console_output_state::ConsoleLogEmissionSnapshot {
        &self.observable
    }
}

#[cfg(test)]
pub(in crate::domains) fn pending_console_activity_snapshot(
    source: ConsoleActivitySource,
) -> Option<ConsoleActivityEmissionSnapshot> {
    let snapshot = ConsoleActivityEmissionSnapshot {
        observable: source.observable,
    };
    (!snapshot.is_empty()).then_some(snapshot)
}

#[cfg(test)]
mod tests {
    use crate::conn::{BrowserContext, CdpCommandTaskStep};
    use crate::domains::observable_output::{
        ObservableOutputProjectionStep,
        observable_backlog_activity_outputs_for_session_owner as observable_backlog_activity_outputs,
    };
    use crate::testing::{TestContext, wait_until_message};
    use moli_core::page::RendererSharedWorkerConsoleMessage;
    use moli_shared_worker::SharedWorkerInstanceId;
    use serde_json::json;

    #[tokio::test]
    async fn console_enable_succeeds() {
        let mut ctx = TestContext::new();
        ctx.process_async(json!({"id": 1, "method": "Console.enable"}))
            .await;
        ctx.expect_result(1, json!({}), None);
    }

    #[tokio::test]
    async fn console_disable_succeeds() {
        let mut ctx = TestContext::new();
        ctx.process_async(json!({"id": 2, "method": "Console.disable"}))
            .await;
        ctx.expect_result(2, json!({}), None);
    }

    #[tokio::test]
    async fn console_clear_messages_succeeds() {
        let mut ctx = TestContext::new();
        ctx.process_async(json!({"id": 3, "method": "Console.clearMessages"}))
            .await;
        ctx.expect_result(3, json!({}), None);
    }

    async fn load_document(ctx: &mut TestContext, html: &str) {
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1".to_owned());
        bc.set_target_url("data:text/html,console-test".to_owned());
        bc.attach_active_session("SID-1".to_owned());
        ctx.conn.browser_context = Some(bc);
        ctx.install_navigation_fixture_for_session_owner(
            &format!("data:text/html,{html}"),
            Some("SID-1"),
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn console_commands_on_loaded_page_use_v8_inspector_agent() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><body></body>").await;

        for (id, method) in [
            (21, "Console.enable"),
            (22, "Console.disable"),
            (23, "Console.clearMessages"),
        ] {
            let raw = json!({"id": id, "method": method, "sessionId": "SID-1"}).to_string();
            let pending = ctx
                .conn
                .try_start_pending_command_dispatch(&raw)
                .unwrap_or_else(|| {
                    panic!("loaded page {method} should dispatch through V8 inspector")
                });
            let (messages, _) = ctx
                .complete_command_task_step_for_test(CdpCommandTaskStep::Pending(Box::new(pending)))
                .await;

            assert!(
                messages
                    .iter()
                    .any(|message| message["id"] == json!(id) && message["result"] == json!({})),
                "{method} should return V8 inspector success: {messages:?}"
            );
        }
        let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
        assert!(
            !browser_context.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled,
            "transitional observable-output enabled bit should track Console.disable"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loaded_page_console_enable_projection_waits_for_v8_success() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><body></body>").await;

        let raw = json!({"id": 24, "method": "Console.enable", "sessionId": "SID-1"}).to_string();
        let pending = ctx
            .conn
            .try_start_pending_command_dispatch(&raw)
            .expect("loaded page Console.enable should dispatch through V8 inspector");
        assert!(
            !ctx.conn
                .browser_context
                .as_ref()
                .expect("browser context should exist")
                .active_page_target()
                .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled,
            "transitional Console projection must not flip before V8 Console.enable succeeds"
        );

        let (messages, scheduler_events) = ctx
            .complete_command_task_step_for_test(CdpCommandTaskStep::Pending(Box::new(pending)))
            .await;
        assert!(
            scheduler_events.is_empty(),
            "Console.enable should not enqueue scheduler work: {scheduler_events:?}"
        );
        assert!(
            messages
                .iter()
                .any(|message| message["id"] == json!(24) && message["result"] == json!({})),
            "Console.enable should return V8 inspector success: {messages:?}"
        );
        assert!(
            ctx.conn
                .browser_context
                .as_ref()
                .expect("browser context should exist")
                .active_page_target()
                .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled,
            "transitional Console projection should flip after V8 Console.enable succeeds"
        );
    }

    fn load_shared_worker_target(ctx: &mut TestContext, session_id: &str) {
        let mut bc = BrowserContext::new("BID-shared".to_owned());
        let mut target = crate::conn::SharedWorkerTargetState::new(
            moli_core::RendererOwnerLocalHostId::new_for_testing(1),
            SharedWorkerInstanceId::from_u64(91),
            "TID-shared-worker".to_owned(),
            None,
            "https://example.test/shared-worker.js".to_owned(),
            "worker".to_owned(),
        );
        target.attach_session(session_id.to_owned());
        target.record_console_message(RendererSharedWorkerConsoleMessage {
            message: "log: before enable".to_owned(),
            args: Vec::new(),
            stack: None,
        });
        bc.insert_shared_worker_target(target);
        ctx.conn.browser_context = Some(bc);
    }

    #[tokio::test]
    async fn console_enable_on_shared_worker_session_starts_at_current_target_cursor() {
        let mut ctx = TestContext::new();
        load_shared_worker_target(&mut ctx, "SID-shared-worker");

        ctx.process_async(json!({
            "id": 41,
            "method": "Console.enable",
            "sessionId": "SID-shared-worker"
        }))
        .await;
        ctx.expect_result(41, json!({}), Some("SID-shared-worker"));
        let target = ctx
            .conn
            .shared_worker_target_for_session_mut(Some("SID-shared-worker"))
            .expect("shared worker target session should exist");
        assert!(
            target
                .pending_console_domain_messages("SID-shared-worker")
                .is_empty()
        );

        target.record_console_message(RendererSharedWorkerConsoleMessage {
            message: "error: after enable".to_owned(),
            args: Vec::new(),
            stack: None,
        });
        let pending = target.pending_console_domain_messages("SID-shared-worker");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].message, "error: after enable");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn console_backlog_activity_outputs_track_enabled_domain_cursor() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><script>console.warn('boot warning')</script>",
        )
        .await;

        assert_eq!(
            observable_backlog_activity_outputs(&ctx.conn, None).console_outputs(),
            &[]
        );
        let bc = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should be loaded");
        bc.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .console_enabled = true;
        assert_eq!(
            observable_backlog_activity_outputs(&ctx.conn, None).console_outputs(),
            &[ObservableOutputProjectionStep::Console]
        );
        let bc = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("browser context should be loaded");
        let page = bc.loaded_page().expect("page should be loaded");
        let script_execution = page.script_execution();
        let snapshot = super::pending_console_activity_snapshot(super::ConsoleActivitySource {
            observable: bc
                .active_page_target()
                .owner_state
                .console_output_state
                .console_domain_emission_snapshot(
                    script_execution.console_messages(),
                    script_execution.lifecycle_errors(),
                ),
        })
        .expect("console output should be pending");
        ctx.conn
            .browser_context
            .as_mut()
            .expect("browser context should be loaded")
            .active_page_target_mut()
            .owner_state
            .console_output_state
            .advance_console_domain_to_current(
                snapshot.observable().console_end(),
                snapshot.observable().lifecycle_end(),
            );
        assert_eq!(
            observable_backlog_activity_outputs(&ctx.conn, None).console_outputs(),
            &[]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn console_enable_replays_v8_buffered_messages() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><script>console.warn('boot warning')</script>",
        )
        .await;

        ctx.process_async(json!({"id": 4, "method": "Console.enable", "sessionId": "SID-1"}))
            .await;

        ctx.expect_result(4, json!({}), Some("SID-1"));
        let event = ctx.take_first_matching("Console.messageAdded replay", |message| {
            message["method"] == json!("Console.messageAdded")
                && message["params"]["message"]["source"] == json!("console-api")
                && message["params"]["message"]["level"] == json!("warning")
                && message["params"]["message"]["text"] == json!("boot warning")
        });
        assert_eq!(event["sessionId"], json!("SID-1"));
        assert!(
            ctx.sent
                .iter()
                .all(|message| message["method"] != json!("Console.messageAdded")),
            "Console.enable should not duplicate V8 replay through observable output: {:?}",
            ctx.sent
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn console_message_added_is_incremental_after_enable() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><body></body>").await;
        ctx.process_async(json!({"id": 5, "method": "Console.enable", "sessionId": "SID-1"}))
            .await;
        ctx.expect_result(5, json!({}), Some("SID-1"));

        ctx.process_async(json!({
            "id": 6,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "console.error('after enable')"
            }
        }))
        .await;

        ctx.expect_result(
            6,
            json!({
                "result": {
                    "type": "undefined",
                }
            }),
            Some("SID-1"),
        );
        ctx.expect_event(
            "Console.messageAdded",
            Some(&json!({
                "message": {
                    "source": "console-api",
                    "level": "error",
                    "text": "after enable",
                }
            })),
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn console_message_added_reports_lifecycle_error_after_enable() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><body></body>").await;
        ctx.process_async(json!({"id": 7, "method": "Console.enable", "sessionId": "SID-1"}))
            .await;
        ctx.expect_result(7, json!({}), Some("SID-1"));

        ctx.process_async(json!({
            "id": 8,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "setTimeout(function(){ throw new Error('console timer boom') }, 0)"
            }
        }))
        .await;
        ctx.expect_result(8, json!({"result": {"type": "number"}}), Some("SID-1"));

        wait_until_message(
            &mut ctx,
            "SID-1",
            "Console.messageAdded timer error",
            |message| {
                message["method"] == json!("Console.messageAdded")
                    && message["params"]["message"]["source"] == json!("javascript")
                    && message["params"]["message"]["level"] == json!("error")
                    && message["params"]["message"]["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("console timer boom"))
            },
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn console_clear_messages_advances_cursor() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><body></body>").await;
        ctx.process_async(json!({"id": 9, "method": "Console.enable", "sessionId": "SID-1"}))
            .await;
        ctx.expect_result(9, json!({}), Some("SID-1"));

        ctx.process_async(json!({
            "id": 10,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "console.log('cleared')"
            }
        }))
        .await;
        ctx.expect_result(10, json!({"result": {"type": "undefined"}}), Some("SID-1"));
        ctx.expect_event("Console.messageAdded", None);

        ctx.process_async(
            json!({"id": 11, "method": "Console.clearMessages", "sessionId": "SID-1"}),
        )
        .await;
        ctx.expect_result(11, json!({}), Some("SID-1"));
        ctx.process_async(json!({
            "id": 12,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "1 + 1"
            }
        }))
        .await;
        ctx.expect_result(
            12,
            json!({
                "result": {
                    "type": "number",
                    "value": 2,
                }
            }),
            Some("SID-1"),
        );
        assert!(
            ctx.sent
                .iter()
                .all(|message| message["method"] != json!("Console.messageAdded")),
            "Console.clearMessages should prevent replay: {:?}",
            ctx.sent
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn console_enable_stages_background_target_session_state() {
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
        ctx.conn.browser_context = Some(bc);

        ctx.process_async(json!({
            "id": 13,
            "method": "Console.enable",
            "sessionId": "SID-background"
        }))
        .await;

        ctx.expect_result(13, json!({}), Some("SID-background"));
        let active = ctx.conn.browser_context.as_ref().expect("browser context");
        assert!(
            !active.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled
        );
        assert!(
            active
                .non_default_background_page_target_for_test("TID-background")
                .is_some_and(|state| state.devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .console_output_session_state
                    .console_enabled),
            "background target should stage Console.enable"
        );
    }
}
