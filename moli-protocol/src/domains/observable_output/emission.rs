use super::items::{
    ObservableOutputItem, ObservableRuntimeEmissionCursor, ObservableRuntimePreparedItem,
    ObservableRuntimePreparedItems,
};
use super::output_queue::{
    ObservableConsoleLogDomain, ObservableConsoleLogEmissionCursor,
    ObservableConsoleLogPreparedRange, ObservablePreparedOutputs,
    ObservableSessionAuditsPreparedRange,
};
use super::runtime_emission::mark_runtime_observable_emission_cursor_for_session_owner;
use crate::conn::{BackgroundProtocolEvent, CdpConnection, monotonic_timestamp_seconds};
use crate::devtools_runtime::DevToolsTargetId;
use crate::domains::activity::{
    ProtocolOutputPayloads, ProtocolOutputProjectionContext, ProtocolOutputSlot,
};
#[cfg(test)]
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ObservableOutputProjectionStep {
    Audits,
    Console,
    Log,
    RuntimeObservable,
}

pub(crate) struct ObservableOutputProjectionContext<'a> {
    pub(in crate::domains::observable_output) step: ObservableOutputProjectionStep,
    pub(in crate::domains::observable_output) session_id: Option<&'a str>,
    pub(in crate::domains::observable_output) prepared_outputs:
        Option<&'a mut ObservablePreparedOutputs>,
}

async fn project_observable_output_async(
    step: ObservableOutputProjectionStep,
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    if let Some(slot) = prepared_outputs.and_then(ProtocolOutputPayloads::observable_mut) {
        slot.emit_activity_background_events_async(
            step,
            conn,
            context.command.protocol_events_mut(),
            context.session_id,
        )
        .await;
    }
}

macro_rules! observable_projector {
    ($name:ident, $step:ident) => {
        pub(in crate::domains) async fn $name(
            conn: &mut CdpConnection,
            context: &mut ProtocolOutputProjectionContext<'_>,
            prepared_outputs: Option<&mut ProtocolOutputPayloads>,
        ) {
            project_observable_output_async(
                ObservableOutputProjectionStep::$step,
                conn,
                context,
                prepared_outputs,
            )
            .await;
        }
    };
}

observable_projector!(project_audits_async, Audits);
observable_projector!(project_console_async, Console);
observable_projector!(project_log_async, Log);
observable_projector!(project_runtime_observable_async, RuntimeObservable);

pub(in crate::domains) const SLOT_CONSOLE: ProtocolOutputSlot = ProtocolOutputSlot::Console;
pub(in crate::domains) const SLOT_AUDITS: ProtocolOutputSlot = ProtocolOutputSlot::Audits;
pub(in crate::domains) const SLOT_LOG: ProtocolOutputSlot = ProtocolOutputSlot::Log;
pub(in crate::domains) const SLOT_RUNTIME_OBSERVABLE: ProtocolOutputSlot =
    ProtocolOutputSlot::RuntimeObservable;

struct ObservableActivityEmissionPlan {
    items: Vec<ObservableOutputItem>,
    cursor: ObservableEmissionCursor,
}

enum ObservableEmissionCursor {
    Audits(crate::domains::audits_output_state::TargetAuditsOutputCursor),
    ConsoleLog(ObservableConsoleLogEmissionCursor),
    RuntimeObservable(ObservableRuntimeEmissionCursor),
}

impl ObservableActivityEmissionPlan {
    fn from_audits_prepared_range(range: ObservableSessionAuditsPreparedRange) -> Self {
        let (items, cursor) = range.into_emission_parts();
        Self {
            items,
            cursor: ObservableEmissionCursor::Audits(cursor),
        }
    }

    fn from_console_log_prepared_range(range: ObservableConsoleLogPreparedRange) -> Self {
        let (items, cursor) = range.into_emission_parts();
        Self {
            items,
            cursor: ObservableEmissionCursor::ConsoleLog(cursor),
        }
    }

    fn from_runtime_prepared_items(prepared_items: ObservableRuntimePreparedItems) -> Self {
        let (items, cursor) = prepared_items.into_emission_parts();
        let mut emission_items = Vec::new();
        for item in items {
            match item {
                ObservableRuntimePreparedItem::Output(output) => {
                    emission_items.push(output);
                }
            }
        }
        Self {
            items: emission_items,
            cursor: ObservableEmissionCursor::RuntimeObservable(cursor),
        }
    }

    async fn prepare_async(
        step: ObservableOutputProjectionStep,
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        prepared_outputs: Option<&mut ObservablePreparedOutputs>,
    ) -> Option<Self> {
        match step {
            ObservableOutputProjectionStep::Audits => None,
            ObservableOutputProjectionStep::Console => Self::prepare_console_log(
                ObservableConsoleLogDomain::Console,
                conn,
                session_id,
                prepared_outputs,
            ),
            ObservableOutputProjectionStep::Log => Self::prepare_console_log(
                ObservableConsoleLogDomain::Log,
                conn,
                session_id,
                prepared_outputs,
            ),
            ObservableOutputProjectionStep::RuntimeObservable => {
                Self::prepare_runtime_observable(conn, session_id, prepared_outputs).await
            }
        }
    }

    async fn prepare_runtime_observable(
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        prepared_outputs: Option<&mut ObservablePreparedOutputs>,
    ) -> Option<Self> {
        if let Some(prepared_outputs) = prepared_outputs
            && let Some(prepared) =
                prepared_outputs.take_runtime_observable_items_for_session(session_id)
            && let Some((_event_session_id, items)) = prepared.materialize_for_owner(conn)
        {
            return Some(Self::from_runtime_prepared_items(items));
        }
        None
    }

    fn prepare_console_log(
        domain: ObservableConsoleLogDomain,
        conn: &mut CdpConnection,
        session_id: Option<&str>,
        prepared_outputs: Option<&mut ObservablePreparedOutputs>,
    ) -> Option<Self> {
        if let Some(range) = prepared_outputs
            .and_then(|prepared| prepared.take_console_log_range(domain, session_id))
        {
            return range
                .materialize_for_owner(conn, session_id)
                .map(Self::from_console_log_prepared_range);
        }
        None
    }

    fn emit_background_events(
        self,
        conn: &mut CdpConnection,
        out: &mut Vec<BackgroundProtocolEvent>,
        session_id: Option<&str>,
    ) {
        let base_timestamp = monotonic_timestamp_seconds();
        // `session_id == None` is routed through a temporary exact Page-owner
        // scope while a concrete renderer publication is projected. Freeze
        // that target into the typed automation sidecar now: downstream BiDi
        // delivery happens after the scope is restored and must not infer the
        // source from whichever tab is active by then.
        let target_id = conn
            .target_owner_identity_for_session(session_id)
            .and_then(|(_, target_id)| target_id)
            .map(DevToolsTargetId::from);
        let mut output_index = 0;
        for output in self.items {
            output_index += 1;
            if output.duplicates_existing_background_event(out) {
                continue;
            }
            out.push(output.emit_background_event(
                session_id,
                target_id.as_ref(),
                base_timestamp + (output_index as f64 * 0.000_001),
            ));
        }
        self.cursor.mark_emitted(conn, session_id);
    }

    #[cfg(test)]
    fn emit(self, conn: &mut CdpConnection, out: &mut Vec<Value>, session_id: Option<&str>) {
        let mut events = Vec::new();
        self.emit_background_events(conn, &mut events, session_id);
        out.extend(
            events
                .into_iter()
                .map(BackgroundProtocolEvent::into_protocol_message),
        );
    }

    #[cfg(test)]
    fn step(&self) -> ObservableOutputProjectionStep {
        self.cursor.step()
    }
}

impl<'a> ObservableOutputProjectionContext<'a> {
    pub(crate) fn new(step: ObservableOutputProjectionStep, session_id: Option<&'a str>) -> Self {
        Self {
            step,
            session_id,
            prepared_outputs: None,
        }
    }

    pub(crate) fn with_prepared_outputs(
        mut self,
        prepared_outputs: Option<&'a mut ObservablePreparedOutputs>,
    ) -> Self {
        self.prepared_outputs = prepared_outputs;
        self
    }
}

impl ObservableEmissionCursor {
    #[cfg(test)]
    fn step(&self) -> ObservableOutputProjectionStep {
        match self {
            Self::Audits(_) => ObservableOutputProjectionStep::Audits,
            Self::ConsoleLog(cursor) => cursor.projection_step(),
            Self::RuntimeObservable(_) => ObservableOutputProjectionStep::RuntimeObservable,
        }
    }

    fn mark_emitted(self, conn: &mut CdpConnection, session_id: Option<&str>) {
        match self {
            Self::Audits(cursor) => {
                let _ = conn
                    .with_target_devtools_session_state_for_session_mut(session_id, |state| {
                        state.page_session_state.audits.mark_emitted(cursor)
                    });
            }
            Self::ConsoleLog(cursor) => cursor.mark_emitted_for_owner(conn, session_id),
            Self::RuntimeObservable(cursor) => {
                mark_runtime_observable_emission_cursor_for_session_owner(conn, session_id, cursor)
            }
        }
    }
}

pub(crate) async fn emit_pending_observable_activity_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    context: ObservableOutputProjectionContext<'_>,
) {
    let ObservableOutputProjectionContext {
        step,
        session_id,
        prepared_outputs,
    } = context;
    if step == ObservableOutputProjectionStep::Audits {
        let Some(prepared_outputs) = prepared_outputs else {
            return;
        };
        for prepared in prepared_outputs.take_audits_ranges() {
            let event_session_id = prepared.session_id().map(str::to_owned);
            let Some(plan) = prepared
                .materialize_for_owner(conn)
                .map(ObservableActivityEmissionPlan::from_audits_prepared_range)
            else {
                continue;
            };
            plan.emit_background_events(conn, out, event_session_id.as_deref());
        }
        return;
    }
    if step == ObservableOutputProjectionStep::Log {
        let Some(prepared_outputs) = prepared_outputs else {
            return;
        };
        for prepared in prepared_outputs.take_log_ranges() {
            let event_session_id = prepared.session_id().map(str::to_owned);
            let Some(plan) = prepared
                .into_range()
                .materialize_for_owner(conn, event_session_id.as_deref())
                .map(ObservableActivityEmissionPlan::from_console_log_prepared_range)
            else {
                continue;
            };
            plan.emit_background_events(conn, out, event_session_id.as_deref());
        }
        return;
    }
    if step == ObservableOutputProjectionStep::RuntimeObservable {
        let Some(prepared_outputs) = prepared_outputs else {
            return;
        };
        for prepared in prepared_outputs.take_runtime_observable_items() {
            let Some((event_session_id, items)) = prepared.materialize_for_owner(conn) else {
                continue;
            };
            ObservableActivityEmissionPlan::from_runtime_prepared_items(items)
                .emit_background_events(conn, out, event_session_id.as_deref());
        }
        return;
    }
    if let Some(plan) =
        ObservableActivityEmissionPlan::prepare_async(step, conn, session_id, prepared_outputs)
            .await
    {
        plan.emit_background_events(conn, out, session_id);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use moli_core::page::{
        RendererActivityDiagnostics, RendererPageDiagnosticsSnapshot,
        RendererRuntimeObservableSourceSummary, RuntimeConsoleMessageSnapshot,
        ScriptObservableOutputItem,
    };
    use serde_json::json;

    use crate::conn::{
        BackgroundProtocolEvent, BrowserContext, PageTargetHost, TargetIdentityState,
        TargetPageSlot,
    };
    use crate::domains::observable_output::{
        ObservablePreparedOutputSlot, TargetRuntimeObservableSourceSummary,
        observable_backlog_activity_outputs_for_session_owner as observable_backlog_activity_outputs,
        observable_source_activity_outputs,
    };
    use crate::testing::TestContext;

    use super::super::output_queue::TargetObservableOutputQueue;
    use super::{
        ObservableActivityEmissionPlan, ObservableOutputItem, ObservableOutputProjectionContext,
        ObservableOutputProjectionStep, ObservableRuntimePreparedItems,
    };

    async fn runtime_observable_source_snapshot(
        ctx: &mut TestContext,
    ) -> RendererPageDiagnosticsSnapshot {
        let bc = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should be loaded");
        let page = bc
            .active_target
            .runtime_slot
            .loaded_page_mut()
            .expect("loaded page should be installed");
        let console_messages = page
            .runtime_console_messages_with_context_async()
            .await
            .expect("runtime console messages should load");
        let default_execution_context_id = console_messages
            .first()
            .map(|message| message.execution_context_id);
        RendererPageDiagnosticsSnapshot::from_runtime_observable_source(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                default_execution_context_id,
                console_messages,
                Vec::new(),
            ),
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn observable_emission_plan_prepares_console_log_payloads_and_advances_cursors() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1".to_owned());
        bc.set_target_url("data:text/html,observable-emission-plan-test".to_owned());
        bc.attach_active_session("SID-1".to_owned());
        let page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<!doctype html><body></body>")
            .await
            .expect("test page should load");
        let _ = bc
            .active_target
            .runtime_slot
            .replace_loaded_page(Some(page));
        bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .console_enabled = true;
        bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .log_enabled = true;
        ctx.conn.browser_context = Some(bc);

        let queue = TargetObservableOutputQueue::for_test(vec![
            ScriptObservableOutputItem::ConsoleMessage("warn: planned".to_owned()),
            ScriptObservableOutputItem::LifecycleError("planned failure".to_owned()),
        ]);
        let bc = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("browser context should be loaded");
        let prepared_outputs = queue.console_log_backlog_ranges(
            bc.target_url(),
            bc.page_attachment_id()
                .expect("loaded Page must have an attachment id"),
            bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled,
            bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .log_enabled,
            true,
            &bc.active_target.owner_state,
            &bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state,
            Some("SID-1"),
        );
        let mut duplicate_slot =
            ObservablePreparedOutputSlot::from_outputs(prepared_outputs.clone());
        let mut prepared_slot = ObservablePreparedOutputSlot::from_outputs(prepared_outputs);
        let console_plan = ObservableActivityEmissionPlan::prepare_async(
            ObservableOutputProjectionStep::Console,
            &mut ctx.conn,
            None,
            Some(prepared_slot.outputs_mut_for_test()),
        )
        .await
        .expect("console output should produce an observable emission plan");
        assert_eq!(console_plan.step(), ObservableOutputProjectionStep::Console);
        let mut out = Vec::new();
        console_plan.emit(&mut ctx.conn, &mut out, Some("SID-1"));
        assert!(
            out.iter()
                .any(|message| message["method"] == json!("Console.messageAdded")),
            "console plan should emit Console.messageAdded: {out:?}"
        );
        assert!(
            ctx.conn
                .browser_context
                .as_ref()
                .expect("browser context should be loaded")
                .active_target
                .owner_state
                .console_output_state
                .console_domain_cursor()
                == (1, 1),
            "executing the console plan should advance the Console cursor"
        );

        let log_plan = ObservableActivityEmissionPlan::prepare_async(
            ObservableOutputProjectionStep::Log,
            &mut ctx.conn,
            Some("SID-1"),
            Some(prepared_slot.outputs_mut_for_test()),
        )
        .await
        .expect("log output should produce an observable emission plan");
        assert_eq!(log_plan.step(), ObservableOutputProjectionStep::Log);
        out.clear();
        log_plan.emit(&mut ctx.conn, &mut out, Some("SID-1"));
        assert!(
            out.iter()
                .any(|message| message["method"] == json!("Log.entryAdded")
                    && message["params"]["entry"]["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("planned failure"))),
            "log plan should emit Log.entryAdded: {out:?}"
        );
        assert!(
            ctx.conn
                .browser_context
                .as_ref()
                .expect("browser context should be loaded")
                .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .log_lifecycle_entries
                == 1,
            "executing the log plan should advance the session-local Log cursor"
        );
        assert!(
            ObservableActivityEmissionPlan::prepare_async(
                ObservableOutputProjectionStep::Log,
                &mut ctx.conn,
                Some("SID-1"),
                Some(duplicate_slot.outputs_mut_for_test()),
            )
            .await
            .is_none(),
            "a captured Log range must become stale after the session cursor advances"
        );

        assert!(
            ObservableActivityEmissionPlan::prepare_async(
                ObservableOutputProjectionStep::RuntimeObservable,
                &mut ctx.conn,
                None,
                None,
            )
            .await
            .is_none(),
            "RuntimeObservable should not prepare an emission plan without Runtime.enable"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn observable_projection_context_carries_captured_slots_to_emitter() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1".to_owned());
        bc.set_target_url("data:text/html,observable-projection-context-test".to_owned());
        bc.attach_active_session("SID-1".to_owned());
        let page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<!doctype html><script>console.warn('prepared context')</script>",
            )
            .await
            .expect("test page should load");
        let _ = bc
            .active_target
            .runtime_slot
            .replace_loaded_page(Some(page));
        bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .console_enabled = true;
        ctx.conn.browser_context = Some(bc);

        let mut prepared_slot =
            observable_backlog_activity_outputs(&ctx.conn, None).into_prepared_slot();
        ctx.conn
            .evaluate_runtime_expression_with_await_async("console.warn('late context')", false)
            .await
            .expect("late console message should evaluate");

        let mut out = Vec::new();
        super::emit_pending_observable_activity_background_events_async(
            &mut ctx.conn,
            &mut out,
            ObservableOutputProjectionContext::new(
                ObservableOutputProjectionStep::Console,
                Some("SID-1"),
            )
            .with_prepared_outputs(Some(prepared_slot.outputs_mut_for_test())),
        )
        .await;
        let out = out
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();

        let console_events = out
            .iter()
            .filter(|message| message["method"] == json!("Console.messageAdded"))
            .collect::<Vec<_>>();
        assert_eq!(
            console_events.len(),
            1,
            "observable projection context should use the captured Console slot exactly once: {out:?}"
        );
        assert_eq!(
            console_events[0]["params"]["message"]["text"],
            json!("prepared context"),
            "late output must remain outside an already-captured observable projection context"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepared_console_backlog_range_is_bounded_by_prepare_time_watermark() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1".to_owned());
        bc.set_target_url("data:text/html,observable-prepared-range-test".to_owned());
        bc.attach_active_session("SID-1".to_owned());
        let page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<!doctype html><script>console.warn('prepared')</script>",
            )
            .await
            .expect("test page should load");
        let _ = bc
            .active_target
            .runtime_slot
            .replace_loaded_page(Some(page));
        bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .console_enabled = true;
        ctx.conn.browser_context = Some(bc);

        let mut prepared_slot =
            observable_backlog_activity_outputs(&ctx.conn, None).into_prepared_slot();
        ctx.conn
            .evaluate_runtime_expression_with_await_async("console.warn('late')", false)
            .await
            .expect("late console message should evaluate");

        let console_plan = ObservableActivityEmissionPlan::prepare_async(
            ObservableOutputProjectionStep::Console,
            &mut ctx.conn,
            None,
            Some(prepared_slot.outputs_mut_for_test()),
        )
        .await
        .expect("prepared console range should produce a plan");
        let mut out = Vec::new();
        console_plan.emit(&mut ctx.conn, &mut out, Some("SID-1"));

        let console_events = out
            .iter()
            .filter(|message| message["method"] == json!("Console.messageAdded"))
            .collect::<Vec<_>>();
        assert_eq!(
            console_events.len(),
            1,
            "prepared range should emit only outputs visible when the range was prepared: {out:?}"
        );
        assert_eq!(
            console_events[0]["params"]["message"]["text"],
            json!("prepared"),
            "late console output must not be pulled into an already-prepared range"
        );
        assert!(
            observable_backlog_activity_outputs(&ctx.conn, None)
                .console_outputs()
                .contains(&ObservableOutputProjectionStep::Console),
            "late output should remain visible to a later backlog preparation"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepared_console_backlog_range_is_bound_to_page_attachment_id() {
        let mut ctx = TestContext::new();
        let page_url = "data:text/html,observable-prepared-attachment-test";
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1".to_owned());
        bc.set_target_url(page_url.to_owned());
        bc.attach_active_session("SID-1".to_owned());
        let first_page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<!doctype html><script>console.warn('old')</script>",
            )
            .await
            .expect("first test page should load");
        let _ = bc.replace_loaded_page(Some(first_page));
        bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .console_enabled = true;
        ctx.conn.browser_context = Some(bc);

        let mut prepared_slot =
            observable_backlog_activity_outputs(&ctx.conn, None).into_prepared_slot();
        let second_page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<!doctype html><script>console.warn('new')</script>",
            )
            .await
            .expect("second test page should load");
        {
            let bc = ctx
                .conn
                .browser_context
                .as_mut()
                .expect("browser context should remain loaded");
            let _ = bc.replace_loaded_page(Some(second_page));
            bc.set_target_url(page_url.to_owned());
            bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled = true;
        }

        assert!(
            ObservableActivityEmissionPlan::prepare_async(
                ObservableOutputProjectionStep::Console,
                &mut ctx.conn,
                None,
                Some(prepared_slot.outputs_mut_for_test()),
            )
            .await
            .is_none(),
            "prepared console range from the old Page attachment must not materialize against the replacement Page"
        );
        assert!(
            observable_backlog_activity_outputs(&ctx.conn, None)
                .console_outputs()
                .contains(&ObservableOutputProjectionStep::Console),
            "replacement page console output should remain visible to a fresh backlog preparation"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepared_console_backlog_emits_for_background_owner_session() {
        let mut ctx = TestContext::new();
        let active_page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<!doctype html><script>console.warn('active owner')</script>",
            )
            .await
            .expect("active test page should load");
        let background_page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<!doctype html><script>console.warn('background owner')</script>",
            )
            .await
            .expect("background test page should load");

        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-active".to_owned());
        bc.set_target_url("data:text/html,active-owner".to_owned());
        bc.attach_active_session("SID-active".to_owned());
        let _ = bc.replace_loaded_page(Some(active_page));
        bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .console_enabled = true;
        bc.insert_page_target_host(PageTargetHost::new(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            TargetIdentityState::with_url("data:text/html,background-owner".to_owned()),
            TargetPageSlot::with_loaded_page_for_test(background_page),
        ));
        bc.mutate_parked_page_session_state("TID-background", |state| {
            state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled = true;
        });
        ctx.conn.browser_context = Some(bc);

        assert_eq!(
            observable_backlog_activity_outputs(&ctx.conn, Some("SID-background"))
                .console_outputs(),
            &[ObservableOutputProjectionStep::Console],
            "background owner should expose its own Console backlog"
        );
        assert_eq!(
            observable_backlog_activity_outputs(&ctx.conn, None).console_outputs(),
            &[ObservableOutputProjectionStep::Console],
            "active owner should still expose its own independent Console backlog"
        );

        let mut prepared_slot =
            observable_backlog_activity_outputs(&ctx.conn, Some("SID-background"))
                .into_prepared_slot();
        let console_plan = ObservableActivityEmissionPlan::prepare_async(
            ObservableOutputProjectionStep::Console,
            &mut ctx.conn,
            Some("SID-background"),
            Some(prepared_slot.outputs_mut_for_test()),
        )
        .await
        .expect("background prepared console range should produce a plan");
        let mut out = Vec::new();
        console_plan.emit(&mut ctx.conn, &mut out, Some("SID-background"));

        let console_events = out
            .iter()
            .filter(|message| message["method"] == json!("Console.messageAdded"))
            .collect::<Vec<_>>();
        assert_eq!(console_events.len(), 1, "unexpected output: {out:?}");
        assert_eq!(console_events[0]["sessionId"], json!("SID-background"));
        assert_eq!(
            console_events[0]["params"]["message"]["text"],
            json!("background owner")
        );
        assert!(
            observable_backlog_activity_outputs(&ctx.conn, Some("SID-background"))
                .console_outputs()
                .is_empty(),
            "emitting for the background session should advance the background owner cursor"
        );
        assert_eq!(
            observable_backlog_activity_outputs(&ctx.conn, None).console_outputs(),
            &[ObservableOutputProjectionStep::Console],
            "emitting for the background session must not advance the active owner cursor"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn target_promotion_transfers_page_attachment_without_retargeting_prepared_output() {
        let mut ctx = TestContext::new();
        let active_page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<!doctype html><script>console.warn('old active owner')</script>",
            )
            .await
            .expect("active test page should load");
        let promoted_page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<!doctype html><script>console.warn('promoted owner')</script>",
            )
            .await
            .expect("promoted test page should load");

        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-active".to_owned());
        bc.set_target_url("data:text/html,old-active-owner".to_owned());
        bc.attach_active_session("SID-active".to_owned());
        let _ = bc.replace_loaded_page(Some(active_page));
        bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .console_enabled = true;
        bc.insert_page_target_host(PageTargetHost::new(
            "TID-promoted".to_owned(),
            Some("SID-promoted".to_owned()),
            TargetIdentityState::with_url("data:text/html,promoted-owner".to_owned()),
            TargetPageSlot::with_loaded_page_for_test(promoted_page),
        ));
        bc.mutate_parked_page_session_state("TID-promoted", |state| {
            state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled = true;
        });
        ctx.conn.browser_context = Some(bc);

        let (old_active_attachment, promoted_attachment) = {
            let bc = ctx.conn.browser_context.as_ref().expect("browser context");
            (
                bc.active_target
                    .runtime_slot
                    .page_attachment_id()
                    .expect("active Page attachment"),
                bc.background_target("TID-promoted")
                    .expect("promoted target")
                    .page_attachment_id()
                    .expect("promoted Page attachment"),
            )
        };
        assert_ne!(old_active_attachment, promoted_attachment);

        let mut old_active_prepared =
            observable_backlog_activity_outputs(&ctx.conn, None).into_prepared_slot();
        let mut old_active_owner_prepared = old_active_prepared.clone();

        assert!(
            ctx.conn
                .browser_context
                .as_mut()
                .expect("browser context")
                .promote_background_target_to_active_slot_async("TID-promoted")
                .await
                .expect("target promotion should succeed")
        );

        {
            let bc = ctx.conn.browser_context.as_ref().expect("browser context");
            assert_eq!(bc.active_target_id(), Some("TID-promoted"));
            assert_eq!(
                bc.active_target.runtime_slot.page_attachment_id(),
                Some(promoted_attachment),
                "promotion must transfer the installed Page attachment without reallocating it"
            );
            assert_eq!(
                bc.background_target("TID-active")
                    .expect("previous active target should be parked")
                    .page_attachment_id(),
                Some(old_active_attachment),
                "parking must transfer the previous active Page attachment with its target slot"
            );
        }

        assert!(
            ObservableActivityEmissionPlan::prepare_async(
                ObservableOutputProjectionStep::Console,
                &mut ctx.conn,
                None,
                Some(old_active_prepared.outputs_mut_for_test()),
            )
            .await
            .is_none(),
            "prepared output from the old active owner must not materialize on the promoted target"
        );

        let old_owner_plan = ObservableActivityEmissionPlan::prepare_async(
            ObservableOutputProjectionStep::Console,
            &mut ctx.conn,
            Some("SID-active"),
            Some(old_active_owner_prepared.outputs_mut_for_test()),
        )
        .await
        .expect("prepared output must remain addressable through its parked owner");
        let mut old_owner_out = Vec::new();
        old_owner_plan.emit(&mut ctx.conn, &mut old_owner_out, Some("SID-active"));
        assert!(old_owner_out.iter().any(|message| {
            message["method"] == json!("Console.messageAdded")
                && message["params"]["message"]["text"] == json!("old active owner")
        }));

        let mut promoted_prepared =
            observable_backlog_activity_outputs(&ctx.conn, None).into_prepared_slot();
        let promoted_plan = ObservableActivityEmissionPlan::prepare_async(
            ObservableOutputProjectionStep::Console,
            &mut ctx.conn,
            None,
            Some(promoted_prepared.outputs_mut_for_test()),
        )
        .await
        .expect("promoted target current Page output should remain visible");
        let mut promoted_out = Vec::new();
        promoted_plan.emit(&mut ctx.conn, &mut promoted_out, None);
        assert!(promoted_out.iter().any(|message| {
            message["method"] == json!("Console.messageAdded")
                && message["params"]["message"]["text"] == json!("promoted owner")
        }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepared_runtime_observable_range_is_bounded_by_source_summary() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1".to_owned());
        bc.set_target_url("data:text/html,observable-runtime-prepared-range-test".to_owned());
        bc.attach_active_session("SID-1".to_owned());
        let page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<!doctype html><body></body>")
            .await
            .expect("test page should load");
        let _ = bc.replace_loaded_page(Some(page));
        bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
        ctx.conn.browser_context = Some(bc);
        ctx.sent.clear();
        ctx.conn
            .evaluate_runtime_expression_with_await_async("console.warn('runtime prepared')", false)
            .await
            .expect("prepared runtime console message should evaluate");

        let snapshot = runtime_observable_source_snapshot(&mut ctx).await;
        let mut prepared_slot =
            observable_source_activity_outputs(&mut ctx.conn, &snapshot, None).into_prepared_slot();
        ctx.conn
            .evaluate_runtime_expression_with_await_async("console.warn('runtime late')", false)
            .await
            .expect("late runtime console message should evaluate");

        let runtime_plan = ObservableActivityEmissionPlan::prepare_async(
            ObservableOutputProjectionStep::RuntimeObservable,
            &mut ctx.conn,
            None,
            Some(prepared_slot.outputs_mut_for_test()),
        )
        .await
        .expect("prepared runtime observable range should produce a plan");
        let mut out = Vec::new();
        runtime_plan.emit(&mut ctx.conn, &mut out, Some("SID-1"));

        let runtime_events = out
            .iter()
            .filter(|message| message["method"] == json!("Runtime.consoleAPICalled"))
            .collect::<Vec<_>>();
        assert_eq!(
            runtime_events.len(),
            1,
            "prepared runtime range should emit only outputs visible in the source summary: {out:?}"
        );
        assert_eq!(
            runtime_events[0]["params"]["args"][0]["value"],
            json!("runtime prepared"),
            "late runtime console output must not be pulled into an already-prepared source range"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepared_runtime_observable_uses_source_payload_without_page_readback() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1".to_owned());
        bc.set_target_url("data:text/html,observable-runtime-source-payload-test".to_owned());
        bc.attach_active_session("SID-1".to_owned());
        let page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<!doctype html><body></body>")
            .await
            .expect("test page should load");
        let _ = bc.replace_loaded_page(Some(page));
        bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
        ctx.conn.browser_context = Some(bc);
        ctx.sent.clear();

        let snapshot = RendererPageDiagnosticsSnapshot::from_runtime_observable_source(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![runtime_console_message(7, "source payload only")],
                Vec::new(),
            ),
        );
        let mut prepared_slot =
            observable_source_activity_outputs(&mut ctx.conn, &snapshot, None).into_prepared_slot();

        let runtime_plan = ObservableActivityEmissionPlan::prepare_async(
            ObservableOutputProjectionStep::RuntimeObservable,
            &mut ctx.conn,
            None,
            Some(prepared_slot.outputs_mut_for_test()),
        )
        .await
        .expect("source payload should produce a prepared runtime observable plan");
        let mut out = Vec::new();
        runtime_plan.emit(&mut ctx.conn, &mut out, Some("SID-1"));

        let runtime_events = out
            .iter()
            .filter(|message| message["method"] == json!("Runtime.consoleAPICalled"))
            .collect::<Vec<_>>();
        assert_eq!(
            runtime_events.len(),
            1,
            "prepared runtime source payload should emit without pulling from page console state: {out:?}"
        );
        assert_eq!(
            runtime_events[0]["params"]["args"][0]["value"],
            json!("source payload only")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepared_runtime_observable_uses_stored_source_queue_payload() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1".to_owned());
        bc.set_target_url("data:text/html,observable-runtime-stored-source-test".to_owned());
        bc.attach_active_session("SID-1".to_owned());
        let page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<!doctype html><body></body>")
            .await
            .expect("test page should load");
        let _ = bc.replace_loaded_page(Some(page));
        bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
        ctx.conn.browser_context = Some(bc);

        let snapshot = RendererPageDiagnosticsSnapshot::from_runtime_observable_source(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![runtime_console_message(7, "stored source payload")],
                Vec::new(),
            ),
        );
        let mut prepared_slot =
            observable_source_activity_outputs(&mut ctx.conn, &snapshot, None).into_prepared_slot();

        let runtime_plan = ObservableActivityEmissionPlan::prepare_async(
            ObservableOutputProjectionStep::RuntimeObservable,
            &mut ctx.conn,
            None,
            Some(prepared_slot.outputs_mut_for_test()),
        )
        .await
        .expect("stored source queue payload should produce a prepared runtime observable plan");
        let mut out = Vec::new();
        runtime_plan.emit(&mut ctx.conn, &mut out, Some("SID-1"));

        let runtime_events = out
            .iter()
            .filter(|message| message["method"] == json!("Runtime.consoleAPICalled"))
            .collect::<Vec<_>>();
        assert_eq!(
            runtime_events.len(),
            1,
            "prepared RuntimeObservable payload should use stored source queue payload before live page readback: {out:?}"
        );
        assert_eq!(
            runtime_events[0]["params"]["args"][0]["value"],
            json!("stored source payload")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn renderer_runtime_agent_ownership_suppresses_runtime_console_source_fallback() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1".to_owned());
        bc.set_target_url("data:text/html,observable-runtime-agent-owner-test".to_owned());
        bc.attach_active_session("SID-1".to_owned());
        let page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<!doctype html><body></body>")
            .await
            .expect("test page should load");
        let _ = bc.replace_loaded_page(Some(page));
        bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
        bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .renderer_runtime_agent_owns_page_console_api_events = true;
        ctx.conn.browser_context = Some(bc);

        let console_only_snapshot = RendererPageDiagnosticsSnapshot::from_runtime_observable_source(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![runtime_console_message(7, "native runtime owns console")],
                Vec::new(),
            ),
        );
        assert!(
            observable_source_activity_outputs(&mut ctx.conn, &console_only_snapshot, None)
                .runtime_observable_outputs()
                .is_empty(),
            "renderer Runtime agent ownership should suppress console-only RuntimeObservable fallback"
        );

        let lifecycle_snapshot = RendererPageDiagnosticsSnapshot::from_runtime_observable_source(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![runtime_console_message(
                    7,
                    "native runtime still owns console",
                )],
                vec!["fallback lifecycle still visible".to_owned()],
            ),
        );
        let mut prepared_slot =
            observable_source_activity_outputs(&mut ctx.conn, &lifecycle_snapshot, None)
                .into_prepared_slot();
        let runtime_plan = ObservableActivityEmissionPlan::prepare_async(
            ObservableOutputProjectionStep::RuntimeObservable,
            &mut ctx.conn,
            None,
            Some(prepared_slot.outputs_mut_for_test()),
        )
        .await
        .expect("lifecycle source should still produce a RuntimeObservable plan");
        let mut out = Vec::new();
        runtime_plan.emit(&mut ctx.conn, &mut out, Some("SID-1"));

        assert!(
            out.iter()
                .all(|message| message["method"] != json!("Runtime.consoleAPICalled")),
            "RuntimeObservable fallback must not duplicate renderer Runtime agent console events: {out:?}"
        );
        assert!(
            out.iter().any(|message| {
                message["method"] == json!("Runtime.exceptionThrown")
                    && message["params"]["exceptionDetails"]["text"]
                        == json!("fallback lifecycle still visible")
            }),
            "RuntimeObservable fallback should keep lifecycle/exception output: {out:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_observable_drain_requires_prepared_source_payload() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1".to_owned());
        bc.set_target_url("data:text/html,observable-runtime-no-source-test".to_owned());
        bc.attach_active_session("SID-1".to_owned());
        let page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<!doctype html><script>console.log('live page only')</script>",
            )
            .await
            .expect("test page should load");
        let _ = bc.replace_loaded_page(Some(page));
        bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
        ctx.conn.browser_context = Some(bc);
        ctx.conn
            .runtime_session_owner_slot_mut(None)
            .expect("runtime slot should exist")
            .ingest_owner_page_observable_output_updates();

        let runtime_plan = ObservableActivityEmissionPlan::prepare_async(
            ObservableOutputProjectionStep::RuntimeObservable,
            &mut ctx.conn,
            None,
            None,
        )
        .await;
        assert!(
            runtime_plan.is_none(),
            "RuntimeObservable drain must not live-read page console payload without a stored source queue item"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepared_runtime_observable_source_payload_is_bound_to_page_attachment_id() {
        let mut ctx = TestContext::new();
        let page_url = "data:text/html,observable-runtime-source-payload-attachment-test";
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1".to_owned());
        bc.set_target_url(page_url.to_owned());
        bc.attach_active_session("SID-1".to_owned());
        let first_page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<!doctype html><body></body>")
            .await
            .expect("first test page should load");
        let _ = bc.replace_loaded_page(Some(first_page));
        bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
        ctx.conn.browser_context = Some(bc);

        let snapshot = RendererPageDiagnosticsSnapshot::from_runtime_observable_source(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![runtime_console_message(7, "old source payload")],
                Vec::new(),
            ),
        );
        let mut prepared_slot =
            observable_source_activity_outputs(&mut ctx.conn, &snapshot, None).into_prepared_slot();

        let second_page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<!doctype html><body></body>")
            .await
            .expect("second test page should load");
        {
            let bc = ctx
                .conn
                .browser_context
                .as_mut()
                .expect("browser context should remain loaded");
            let _ = bc.replace_loaded_page(Some(second_page));
            bc.set_target_url(page_url.to_owned());
            bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled = true;
        }

        assert!(
            ObservableActivityEmissionPlan::prepare_async(
                ObservableOutputProjectionStep::RuntimeObservable,
                &mut ctx.conn,
                None,
                Some(prepared_slot.outputs_mut_for_test()),
            )
            .await
            .is_none(),
            "source-time prepared RuntimeObservable payload must remain bound to the Page attachment"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prepared_runtime_observable_range_is_bound_to_page_attachment_id() {
        let mut ctx = TestContext::new();
        let page_url = "data:text/html,observable-runtime-attachment-test";
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1".to_owned());
        bc.set_target_url(page_url.to_owned());
        bc.attach_active_session("SID-1".to_owned());
        let first_page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<!doctype html><body></body>")
            .await
            .expect("first test page should load");
        let _ = bc.replace_loaded_page(Some(first_page));
        bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
        ctx.conn.browser_context = Some(bc);
        ctx.sent.clear();
        ctx.conn
            .evaluate_runtime_expression_with_await_async("console.warn('runtime old')", false)
            .await
            .expect("old runtime console message should evaluate");

        let old_snapshot = runtime_observable_source_snapshot(&mut ctx).await;
        let mut old_prepared_slot =
            observable_source_activity_outputs(&mut ctx.conn, &old_snapshot, None)
                .into_prepared_slot();

        let second_page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<!doctype html><body></body>")
            .await
            .expect("second test page should load");
        {
            let bc = ctx
                .conn
                .browser_context
                .as_mut()
                .expect("browser context should remain loaded");
            let _ = bc.replace_loaded_page(Some(second_page));
            bc.set_target_url(page_url.to_owned());
            bc.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled = true;
        }
        ctx.sent.clear();
        ctx.conn
            .evaluate_runtime_expression_with_await_async("console.warn('runtime new')", false)
            .await
            .expect("new runtime console message should evaluate");

        assert!(
            ObservableActivityEmissionPlan::prepare_async(
                ObservableOutputProjectionStep::RuntimeObservable,
                &mut ctx.conn,
                None,
                Some(old_prepared_slot.outputs_mut_for_test()),
            )
            .await
            .is_none(),
            "prepared RuntimeObservable range from the old Page attachment must not materialize against the replacement Page"
        );

        let new_snapshot = runtime_observable_source_snapshot(&mut ctx).await;
        assert!(
            observable_source_activity_outputs(&mut ctx.conn, &new_snapshot, None)
                .runtime_observable_outputs()
                .contains(&ObservableOutputProjectionStep::RuntimeObservable),
            "replacement page runtime output should remain visible to a fresh source preparation"
        );
    }

    #[tokio::test]
    async fn observable_emission_plan_prepares_runtime_payloads_and_advances_cursors() {
        let mut conn = crate::conn::CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new_with_page_for_test("BID-1", "TID-1"));
        let runtime_plan = ObservableActivityEmissionPlan::from_runtime_prepared_items(
            ObservableRuntimePreparedItems::for_test(
                vec![
                    ObservableOutputItem::RuntimeConsoleApiCalled {
                        console_type: "error".to_owned(),
                        text: "runtime planned".to_owned(),
                        args: Vec::new(),
                        stack: None,
                        execution_context_id: 7,
                    },
                    ObservableOutputItem::RuntimeExceptionThrown {
                        text: "runtime lifecycle".to_owned(),
                        url: "http://example.test/runtime".to_owned(),
                        execution_context_id: 7,
                        exception_index: 0,
                    },
                ],
                std::collections::HashMap::from([(7, 1)]),
                1,
            ),
        );
        assert_eq!(
            runtime_plan.step(),
            ObservableOutputProjectionStep::RuntimeObservable
        );
        let mut out = Vec::new();
        runtime_plan.emit(&mut conn, &mut out, Some("SID-1"));
        assert!(
            out.iter().any(|message| {
                message["method"] == json!("Runtime.consoleAPICalled")
                    && message["params"]["type"] == json!("error")
                    && message["params"]["args"][0]["value"] == json!("runtime planned")
            }),
            "runtime plan should emit Runtime.consoleAPICalled: {out:?}"
        );
        assert!(
            out.iter().any(|message| {
                message["method"] == json!("Runtime.exceptionThrown")
                    && message["params"]["exceptionDetails"]["exceptionId"] == json!(1)
                    && message["params"]["exceptionDetails"]["text"] == json!("runtime lifecycle")
            }),
            "runtime plan should emit Runtime.exceptionThrown: {out:?}"
        );
        let state = &conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_target
            .owner_state
            .runtime_observable_state;
        assert!(
            !state.has_unemitted_source(
                &TargetRuntimeObservableSourceSummary::from_renderer_snapshot(
                    &RendererPageDiagnosticsSnapshot::from_diagnostics(
                        RendererActivityDiagnostics {
                            runtime_console_messages_with_context: 1,
                            runtime_console_messages_by_context: BTreeMap::from([(7, 1)]),
                            runtime_lifecycle_errors: 1,
                            ..Default::default()
                        },
                    ),
                )
            ),
            "executing the runtime plan should advance RuntimeObservable cursors",
        );
    }

    #[tokio::test]
    async fn observable_runtime_plan_advances_lifecycle_cursor_without_emittable_items() {
        let mut conn = crate::conn::CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new_with_page_for_test("BID-1", "TID-1"));
        let runtime_plan = ObservableActivityEmissionPlan::from_runtime_prepared_items(
            ObservableRuntimePreparedItems::for_test(
                Vec::new(),
                std::collections::HashMap::new(),
                1,
            ),
        );
        let mut out = Vec::new();
        runtime_plan.emit(&mut conn, &mut out, Some("SID-1"));

        assert!(
            out.is_empty(),
            "no Runtime.exceptionThrown can be emitted without an execution context"
        );
        let state = &conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_target
            .owner_state
            .runtime_observable_state;
        assert!(
            !state.has_unemitted_source(
                &TargetRuntimeObservableSourceSummary::from_renderer_snapshot(
                    &RendererPageDiagnosticsSnapshot::from_diagnostics(
                        RendererActivityDiagnostics {
                            runtime_lifecycle_errors: 1,
                            ..Default::default()
                        },
                    ),
                )
            ),
            "RuntimeObservable must still advance the exception cursor when no wire event is emitted",
        );
    }

    fn runtime_console_message(
        execution_context_id: i64,
        message: &str,
    ) -> RuntimeConsoleMessageSnapshot {
        RuntimeConsoleMessageSnapshot {
            execution_context_id,
            message: message.to_owned(),
            args: vec![json!({"type": "string", "value": message})],
            stack: None,
        }
    }
}
