use crate::conn::{
    CdpConnection, CdpRendererCommandAccess, CdpSessionRoute, Cmd, CommandOwnerScope,
    PerformanceTimeDomain, RendererCommandCorrelation, RendererCommandDescriptor,
    monotonic_timestamp_seconds,
};
use crate::domains::actions::PerformanceAction;
use crate::domains::command_output::CommandOutputPlan;
use moli_core::page::{
    CompletedDevToolsIoCommandDispatch, CompletedPageCommand, Page,
    PendingDevToolsIoCommandDispatch, PendingPageCommand, RendererPerformanceMetricSnapshot,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Default, Deserialize)]
struct PerformanceEnableParams {
    #[serde(default, rename = "timeDomain")]
    time_domain: Option<String>,
}

#[derive(Deserialize)]
struct PerformanceSetTimeDomainParams {
    #[serde(rename = "timeDomain")]
    time_domain: String,
}

pub(crate) struct PendingPerformanceCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    renderer_access: CdpRendererCommandAccess,
    renderer_page: crate::conn::RendererPageResidenceIdentity,
    pending: Box<PendingPerformanceRendererCommand>,
}

pub(crate) struct CompletedPerformanceCommandDispatch {
    command_id: Option<u64>,
    owner_scope: CommandOwnerScope,
    renderer_access: CdpRendererCommandAccess,
    renderer_page: crate::conn::RendererPageResidenceIdentity,
    completed: Result<CompletedPerformanceRendererCommand, String>,
}

enum PendingPerformanceRendererCommand {
    Main(PendingPageCommand),
    IoCommandReply {
        pending: PendingDevToolsIoCommandDispatch,
        snapshot: RendererPerformanceMetricSnapshot,
    },
    IoSessionOutput {
        pending: PendingDevToolsIoCommandDispatch,
        correlation: RendererCommandCorrelation,
    },
}

enum CompletedPerformanceRendererCommand {
    Main(CompletedPageCommand),
    IoCommandReply(RendererPerformanceMetricSnapshot),
    IoSessionOutput {
        completed: Result<CompletedDevToolsIoCommandDispatch, String>,
        correlation: RendererCommandCorrelation,
    },
}

pub(crate) enum PerformanceCommandTaskStep {
    Pending(PendingPerformanceCommandDispatch),
    Complete(CommandOutputPlan),
}

impl PendingPerformanceCommandDispatch {
    pub async fn wait(self) -> CompletedPerformanceCommandDispatch {
        let completed = match *self.pending {
            PendingPerformanceRendererCommand::Main(pending) => pending
                .wait()
                .await
                .map(CompletedPerformanceRendererCommand::Main),
            PendingPerformanceRendererCommand::IoCommandReply { pending, snapshot } => {
                match pending.wait().await {
                    Ok(CompletedDevToolsIoCommandDispatch::Dispatched) => Ok(
                        CompletedPerformanceRendererCommand::IoCommandReply(snapshot),
                    ),
                    Ok(CompletedDevToolsIoCommandDispatch::SessionResponse { .. }) => Err(
                        anyhow::anyhow!("command-reply Performance dispatch used session output"),
                    ),
                    Err(error) => Err(error),
                }
            }
            PendingPerformanceRendererCommand::IoSessionOutput {
                pending,
                correlation,
            } => Ok(CompletedPerformanceRendererCommand::IoSessionOutput {
                completed: pending.wait().await.map_err(|error| error.to_string()),
                correlation,
            }),
        };
        CompletedPerformanceCommandDispatch {
            command_id: self.command_id,
            owner_scope: self.owner_scope,
            renderer_access: self.renderer_access,
            renderer_page: self.renderer_page,
            completed: completed.map_err(|error| error.to_string()),
        }
    }
}

impl CompletedPerformanceCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.owner_scope.session_id()
    }
}

fn loaded_page_mut_for_renderer_access<'a>(
    conn: &'a mut CdpConnection,
    session_id: Option<&str>,
    renderer_access: CdpRendererCommandAccess,
    owner_route: Option<&CdpSessionRoute>,
) -> Result<&'a mut Page, String> {
    match renderer_access {
        CdpRendererCommandAccess::MainThread => {
            conn.loaded_page_mut_for_protocol_access_for_route(session_id, owner_route)
        }
        CdpRendererCommandAccess::Io => conn
            .loaded_page_mut_for_interruptible_protocol_access_for_route(session_id, owner_route),
        CdpRendererCommandAccess::OwnerIndependent => {
            Err("Performance.getMetrics requires a renderer Page".to_owned())
        }
    }
}

pub(crate) fn command_output_plan(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    match cmd.parse_action::<PerformanceAction>() {
        Some(PerformanceAction::Enable) => enable_performance_command(conn, cmd),
        Some(PerformanceAction::Disable) => disable_performance_command(conn, cmd),
        Some(PerformanceAction::SetTimeDomain) => set_performance_time_domain_command(conn, cmd),
        Some(PerformanceAction::GetMetrics) | None => {
            CommandOutputPlan::error(-32601, "Unknown Performance command-output method")
        }
    }
}

fn enable_performance_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params = match cmd.get_params::<PerformanceEnableParams>() {
        Ok(Some(params)) => params,
        Ok(None) => PerformanceEnableParams::default(),
        Err(_) => return CommandOutputPlan::error(-32602, "Invalid parameters"),
    };
    let time_domain = match parse_performance_time_domain(params.time_domain.as_deref()) {
        Some(time_domain) => time_domain,
        None => {
            return CommandOutputPlan::error(-32000, "Invalid time domain specification.");
        }
    };
    match conn.enable_performance_for_session_owner(cmd.session_id, time_domain) {
        Some(true) => CommandOutputPlan::success(),
        Some(false) => CommandOutputPlan::error(
            -32000,
            "Cannot change time domain while performance metrics collection is enabled.",
        ),
        None => CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"),
    }
}

fn disable_performance_command(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    if !conn.disable_performance_for_session_owner(cmd.session_id) {
        return CommandOutputPlan::error(-31998, "BrowserContextNotLoaded");
    }
    CommandOutputPlan::success()
}

fn set_performance_time_domain_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> CommandOutputPlan {
    let params = match cmd.get_params::<PerformanceSetTimeDomainParams>() {
        Ok(Some(params)) => params,
        _ => return CommandOutputPlan::error(-32602, "Invalid parameters"),
    };
    let time_domain = match parse_performance_time_domain(Some(&params.time_domain)) {
        Some(time_domain) => time_domain,
        None => {
            return CommandOutputPlan::error(-32000, "Invalid time domain specification.");
        }
    };
    match conn.set_performance_time_domain_for_session_owner(cmd.session_id, time_domain) {
        Some(true) => CommandOutputPlan::success(),
        Some(false) => CommandOutputPlan::error(
            -32000,
            "Cannot set time domain while performance metrics collection is enabled.",
        ),
        None => CommandOutputPlan::error(-31998, "BrowserContextNotLoaded"),
    }
}

fn parse_performance_time_domain(value: Option<&str>) -> Option<PerformanceTimeDomain> {
    match value.unwrap_or("timeTicks") {
        "timeTicks" => Some(PerformanceTimeDomain::TimeTicks),
        "threadTicks" => Some(PerformanceTimeDomain::ThreadTicks),
        _ => None,
    }
}

pub(crate) fn try_start_performance_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    renderer_access: CdpRendererCommandAccess,
) -> PerformanceCommandTaskStep {
    match cmd.parse_action::<PerformanceAction>() {
        Some(
            PerformanceAction::Enable
            | PerformanceAction::Disable
            | PerformanceAction::SetTimeDomain,
        ) => {
            return PerformanceCommandTaskStep::Complete(command_output_plan(conn, cmd));
        }
        Some(PerformanceAction::GetMetrics) => {}
        None => {
            return PerformanceCommandTaskStep::Complete(CommandOutputPlan::error(
                -32601,
                "UnknownMethod",
            ));
        }
    }
    if !conn.performance_enabled_for_session_owner(cmd.session_id) {
        return PerformanceCommandTaskStep::Complete(empty_metrics_command_output_plan());
    }
    if !conn
        .runtime_session_owner_slot(cmd.session_id)
        .ok()
        .is_some_and(|slot| slot.has_loaded_page())
    {
        return PerformanceCommandTaskStep::Complete(default_metrics_command_output_plan());
    }
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    if renderer_access == CdpRendererCommandAccess::Io {
        let (renderer_page, attachment_id, snapshot) = {
            let page = match loaded_page_mut_for_renderer_access(
                conn,
                cmd.session_id,
                renderer_access,
                owner_scope.session_owner_route(),
            ) {
                Ok(page) => page,
                Err(_) => {
                    return PerformanceCommandTaskStep::Complete(
                        default_metrics_command_output_plan(),
                    );
                }
            };
            (
                crate::conn::RendererPageResidenceIdentity::from_page(page),
                page.renderer_agent_attachment_id(),
                page.cached_performance_metric_snapshot(),
            )
        };
        let response_delivery = cmd.terminal_response_delivery(
            moli_page_types::RendererInspectorResponseDelivery::DevToolsSession,
        );
        if cmd.id.is_none()
            || response_delivery == moli_page_types::RendererInspectorResponseDelivery::CommandReply
        {
            let page = loaded_page_mut_for_renderer_access(
                conn,
                cmd.session_id,
                renderer_access,
                owner_scope.session_owner_route(),
            )
            .expect("the captured Performance Page must remain loaded synchronously");
            let (pending, snapshot) = page.start_performance_metric_snapshot_from_io();
            return PerformanceCommandTaskStep::Pending(PendingPerformanceCommandDispatch {
                command_id: cmd.id,
                owner_scope,
                renderer_access,
                renderer_page,
                pending: Box::new(PendingPerformanceRendererCommand::IoCommandReply {
                    pending,
                    snapshot,
                }),
            });
        }
        let command_id = cmd
            .id
            .expect("session output requires a frontend command id");
        let Some(attachment_id) = attachment_id else {
            return PerformanceCommandTaskStep::Complete(default_metrics_command_output_plan());
        };
        let renderer_inspector_session_id =
            conn.target_renderer_runtime_inspector_session_id_for_session(cmd.session_id);
        let result = performance_metrics_result(&snapshot);
        let descriptor = RendererCommandDescriptor::performance_get_metrics(
            cmd.json.to_owned(),
            cmd.renderer_policy(),
            response_delivery,
        );
        let prepared = match conn.try_register_renderer_call_for_session_owner(
            cmd.session_id,
            command_id,
            Some(attachment_id),
            descriptor,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                return PerformanceCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000, error,
                ));
            }
        };
        let (correlation, response, response_rx) = prepared.into_parts();
        debug_assert!(
            response_rx.is_none(),
            "Performance session output must not allocate a command-reply receiver",
        );
        let pending = loaded_page_mut_for_renderer_access(
            conn,
            cmd.session_id,
            renderer_access,
            owner_scope.session_owner_route(),
        )
        .ok()
        .filter(|page| {
            crate::conn::RendererPageResidenceIdentity::from_page(page) == renderer_page
                && page.renderer_agent_attachment_id() == Some(attachment_id)
        })
        .ok_or_else(|| "Performance renderer attachment changed before IO dispatch".to_owned())
        .and_then(|page| {
            page.start_performance_get_metrics_from_io_with_response(
                renderer_inspector_session_id,
                result,
                response,
            )
            .map_err(|error| error.to_string())
        });
        let pending = match pending {
            Ok(pending) => pending,
            Err(error) => {
                let removed = conn.take_renderer_call_if_correlation_matches_for_session_owner(
                    cmd.session_id,
                    correlation,
                );
                debug_assert!(removed);
                return PerformanceCommandTaskStep::Complete(CommandOutputPlan::error(
                    -32000, error,
                ));
            }
        };
        return PerformanceCommandTaskStep::Pending(PendingPerformanceCommandDispatch {
            command_id: cmd.id,
            owner_scope,
            renderer_access,
            renderer_page,
            pending: Box::new(PendingPerformanceRendererCommand::IoSessionOutput {
                pending,
                correlation,
            }),
        });
    }
    let page = match loaded_page_mut_for_renderer_access(
        conn,
        cmd.session_id,
        renderer_access,
        owner_scope.session_owner_route(),
    ) {
        Ok(page) => page,
        Err(_) => {
            return PerformanceCommandTaskStep::Complete(default_metrics_command_output_plan());
        }
    };
    let renderer_page = crate::conn::RendererPageResidenceIdentity::from_page(page);
    let pending = match page.start_performance_metric_snapshot() {
        Ok(pending) => pending,
        Err(_) => {
            return PerformanceCommandTaskStep::Complete(default_metrics_command_output_plan());
        }
    };
    PerformanceCommandTaskStep::Pending(PendingPerformanceCommandDispatch {
        command_id: cmd.id,
        owner_scope,
        renderer_access,
        renderer_page,
        pending: Box::new(PendingPerformanceRendererCommand::Main(pending)),
    })
}

fn empty_metrics_command_output_plan() -> CommandOutputPlan {
    CommandOutputPlan::result(json!({ "metrics": [] }))
}

fn default_metrics_command_output_plan() -> CommandOutputPlan {
    performance_metrics_command_output_plan(&RendererPerformanceMetricSnapshot::default())
}

fn performance_metrics_command_output_plan(
    snapshot: &RendererPerformanceMetricSnapshot,
) -> CommandOutputPlan {
    CommandOutputPlan::result(performance_metrics_result(snapshot))
}

pub(crate) fn performance_metrics_result(snapshot: &RendererPerformanceMetricSnapshot) -> Value {
    json!({ "metrics": build_performance_metrics(snapshot) })
}

pub(crate) async fn complete_pending_performance_command(
    conn: &mut CdpConnection,
    completed: CompletedPerformanceCommandDispatch,
) -> CommandOutputPlan {
    let CompletedPerformanceCommandDispatch {
        command_id: _,
        owner_scope,
        renderer_access,
        renderer_page,
        completed,
    } = completed;
    let session_id = owner_scope.session_id().map(str::to_owned);
    let snapshot = match completed {
        Ok(CompletedPerformanceRendererCommand::Main(completed_page)) => {
            loaded_page_mut_for_renderer_access(
                conn,
                session_id.as_deref(),
                renderer_access,
                owner_scope.session_owner_route(),
            )
            .ok()
            .filter(|page| {
                crate::conn::RendererPageResidenceIdentity::from_page(page) == renderer_page
            })
            .and_then(|page| page.finish_performance_metric_snapshot(completed_page).ok())
            .unwrap_or_default()
        }
        Ok(CompletedPerformanceRendererCommand::IoCommandReply(snapshot)) => {
            let remains_current = loaded_page_mut_for_renderer_access(
                conn,
                session_id.as_deref(),
                renderer_access,
                owner_scope.session_owner_route(),
            )
            .ok()
            .is_some_and(|page| {
                crate::conn::RendererPageResidenceIdentity::from_page(page) == renderer_page
            });
            if remains_current {
                snapshot
            } else {
                RendererPerformanceMetricSnapshot::default()
            }
        }
        Ok(CompletedPerformanceRendererCommand::IoSessionOutput {
            completed,
            correlation,
        }) => {
            if let Ok(CompletedDevToolsIoCommandDispatch::SessionResponse { predecessor, .. }) =
                completed
            {
                let mut plan = CommandOutputPlan::default();
                plan.set_renderer_output_predecessor(predecessor);
                return plan;
            }
            if !conn.take_renderer_call_if_correlation_matches_for_route(
                session_id.as_deref(),
                owner_scope.session_owner_route(),
                correlation,
            ) {
                return CommandOutputPlan::default();
            }
            RendererPerformanceMetricSnapshot::default()
        }
        Err(_) => RendererPerformanceMetricSnapshot::default(),
    };
    CommandOutputPlan::result(json!({ "metrics": build_performance_metrics(&snapshot) }))
}

fn metric(name: &str, value: f64) -> Value {
    json!({ "name": name, "value": finite_non_negative(value) })
}

fn finite_non_negative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn millis_to_seconds(value: Option<f64>) -> f64 {
    finite_non_negative(value.unwrap_or(0.0)) / 1000.0
}

fn count_metric(value: Option<f64>) -> f64 {
    finite_non_negative(value.unwrap_or(0.0)).round()
}

fn build_performance_metrics(snapshot: &RendererPerformanceMetricSnapshot) -> Vec<Value> {
    let timestamp = monotonic_timestamp_seconds();
    let time_origin = millis_to_seconds(snapshot.time_origin_ms);
    let navigation_start = millis_to_seconds(snapshot.navigation_start_ms).max(time_origin);
    let dom_content_loaded = millis_to_seconds(snapshot.dom_content_loaded_ms);
    let load_event = millis_to_seconds(snapshot.load_event_ms);
    let frame_count = count_metric(snapshot.frame_count);
    let document_count = count_metric(snapshot.document_count);
    let node_count = count_metric(snapshot.node_count).max(document_count);
    let resource_count = count_metric(snapshot.resource_count);
    let now_seconds = millis_to_seconds(snapshot.now_ms);

    vec![
        metric("Timestamp", timestamp),
        metric("AudioHandlers", 0.0),
        metric("AudioWorkletProcessors", 0.0),
        metric("Documents", document_count),
        metric("Frames", frame_count),
        metric("JSEventListeners", 0.0),
        metric("LayoutObjects", node_count),
        metric("MediaKeySessions", 0.0),
        metric("MediaKeys", 0.0),
        metric("Nodes", node_count),
        metric("Resources", resource_count),
        metric("ContextLifecycleStateObservers", 0.0),
        metric("V8PerContextDatas", document_count.max(1.0)),
        metric("WorkerGlobalScopes", 0.0),
        metric("UACSSResources", 0.0),
        metric("RTCPeerConnections", 0.0),
        metric("ResourceFetchers", document_count.max(1.0)),
        metric("AdSubframes", 0.0),
        metric("DetachedScriptStates", 0.0),
        metric("ArrayBufferContents", 0.0),
        metric("LayoutCount", 0.0),
        metric("RecalcStyleCount", 0.0),
        metric("LayoutDuration", 0.0),
        metric("RecalcStyleDuration", 0.0),
        metric("DevToolsCommandDuration", 0.0),
        metric("ScriptDuration", now_seconds),
        metric("V8CompileDuration", 0.0),
        metric("TaskDuration", now_seconds),
        metric("TaskOtherDuration", 0.0),
        metric("ThreadTime", now_seconds),
        metric("ProcessTime", 0.0),
        metric("JSHeapUsedSize", 0.0),
        metric("JSHeapTotalSize", 0.0),
        metric("FirstMeaningfulPaint", 0.0),
        metric("DomContentLoaded", dom_content_loaded),
        metric("NavigationStart", navigation_start),
        metric("LoadEvent", load_event),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{BrowserContext, CdpCommandTaskStep};
    use crate::testing::TestContext;
    use serde_json::json;
    use std::collections::HashMap;

    #[tokio::test(flavor = "multi_thread")]
    async fn performance_enable_disable_and_time_domain_match_chromium() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><body>performance state</body>").await;

        ctx.process_async(json!({
            "id": 1,
            "method": "Performance.getMetrics",
            "sessionId": "SID-1"
        }))
        .await;
        assert!(metric_map(&ctx.take_response_by_id(1)).is_empty());

        for id in [2, 3] {
            ctx.process_async(json!({
                "id": id,
                "method": "Performance.enable",
                "sessionId": "SID-1",
                "params": { "timeDomain": "threadTicks" }
            }))
            .await;
            ctx.expect_result(id, json!({}), Some("SID-1"));
        }
        let performance = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .performance;
        assert!(performance.enabled());
        assert_eq!(
            performance.time_domain(),
            PerformanceTimeDomain::ThreadTicks
        );

        ctx.process_async(json!({
            "id": 4,
            "method": "Performance.enable",
            "sessionId": "SID-1"
        }))
        .await;
        ctx.expect_error(
            4,
            -32000,
            "Cannot change time domain while performance metrics collection is enabled.",
        );

        for id in [5, 6] {
            ctx.process_async(json!({
                "id": id,
                "method": "Performance.disable",
                "sessionId": "SID-1"
            }))
            .await;
            ctx.expect_result(id, json!({}), Some("SID-1"));
        }
        ctx.process_async(json!({
            "id": 7,
            "method": "Performance.getMetrics",
            "sessionId": "SID-1"
        }))
        .await;
        assert!(metric_map(&ctx.take_response_by_id(7)).is_empty());

        for (id, value) in [(8, "bogusTicks"), (9, "TimeTicks")] {
            ctx.process_async(json!({
                "id": id,
                "method": "Performance.enable",
                "sessionId": "SID-1",
                "params": { "timeDomain": value }
            }))
            .await;
            ctx.expect_error(id, -32000, "Invalid time domain specification.");
        }
        ctx.process_async(json!({
            "id": 10,
            "method": "Performance.enable",
            "sessionId": "SID-1",
            "params": { "timeDomain": 1 }
        }))
        .await;
        ctx.expect_error(10, -32602, "Invalid parameters");

        ctx.process_async(json!({
            "id": 11,
            "method": "Performance.setTimeDomain",
            "sessionId": "SID-1",
            "params": { "timeDomain": "threadTicks" }
        }))
        .await;
        ctx.expect_result(11, json!({}), Some("SID-1"));

        ctx.process_async(json!({
            "id": 12,
            "method": "Performance.enable",
            "sessionId": "SID-1",
            "params": { "timeDomain": null }
        }))
        .await;
        ctx.expect_result(12, json!({}), Some("SID-1"));
        let performance = ctx
            .conn
            .browser_context
            .as_ref()
            .expect("browser context")
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .performance;
        assert_eq!(performance.time_domain(), PerformanceTimeDomain::TimeTicks);

        ctx.process_async(json!({
            "id": 13,
            "method": "Performance.setTimeDomain",
            "sessionId": "SID-1",
            "params": { "timeDomain": "timeTicks" }
        }))
        .await;
        ctx.expect_error(
            13,
            -32000,
            "Cannot set time domain while performance metrics collection is enabled.",
        );

        ctx.process_async(json!({
            "id": 14,
            "method": "Performance.disable",
            "sessionId": "SID-1"
        }))
        .await;
        ctx.expect_result(14, json!({}), Some("SID-1"));
        ctx.process_async(json!({
            "id": 15,
            "method": "Performance.setTimeDomain",
            "sessionId": "SID-1",
            "params": { "timeDomain": "bogusTicks" }
        }))
        .await;
        ctx.expect_error(15, -32000, "Invalid time domain specification.");
    }

    #[tokio::test]
    async fn performance_get_metrics_returns_empty_while_disabled_without_page() {
        let mut ctx = TestContext::new();
        ctx.process_async(json!({"id": 16, "method": "Performance.getMetrics"}))
            .await;
        assert!(metric_map(&ctx.take_response_by_id(16)).is_empty());
    }

    async fn load_document(ctx: &mut TestContext, html: &str) {
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_target_url("data:text/html,performance-test".to_owned());
        bc.set_active_target_id("TID-1".to_owned());
        bc.attach_active_session("SID-1".to_owned());
        ctx.conn.browser_context = Some(bc);
        ctx.install_navigation_fixture_for_session_owner(
            &format!("data:text/html,{html}"),
            Some("SID-1"),
        )
        .await;
    }

    fn metric_map(response: &Value) -> HashMap<String, f64> {
        response["result"]["metrics"]
            .as_array()
            .expect("metrics array")
            .iter()
            .map(|metric| {
                (
                    metric["name"].as_str().expect("metric name").to_owned(),
                    metric["value"].as_f64().expect("metric value"),
                )
            })
            .collect()
    }

    async fn complete_command_task_step_for_test(
        ctx: &mut TestContext,
        step: CdpCommandTaskStep,
        command_id: u64,
    ) -> Vec<Value> {
        let response_start = ctx.sent.len();
        let (mut messages, _) = ctx.complete_command_task_step_for_test(step).await;
        if !messages
            .iter()
            .any(|message| message["id"] == json!(command_id))
        {
            ctx.wait_for_test_command_response(command_id, response_start)
                .await;
            messages.push(ctx.take_response_by_id(command_id));
        }
        messages
    }

    fn complete_immediate_command_task_step_for_test(step: CdpCommandTaskStep) -> Vec<Value> {
        match step {
            CdpCommandTaskStep::Complete(outcome) => outcome.into_parts().0,
            CdpCommandTaskStep::Pending(_) => panic!("expected immediate command completion"),
        }
    }

    #[test]
    fn performance_enable_disable_and_unknown_do_not_use_legacy_fallback() {
        let mut ctx = TestContext::new();

        for (id, method, expects_result) in [
            (401, "Performance.enable", true),
            (402, "Performance.disable", true),
            (403, "Performance.setTimeDomain", true),
            (404, "Performance.noSuchMethod", false),
        ] {
            let params = (method == "Performance.setTimeDomain")
                .then(|| json!({ "timeDomain": "timeTicks" }));
            let raw = json!({ "id": id, "method": method, "params": params }).to_string();
            let step = ctx.conn.start_command_dispatch(&raw);
            let messages = complete_immediate_command_task_step_for_test(step);
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0]["id"], json!(id));
            if expects_result {
                assert!(messages[0].get("result").is_some());
            } else {
                assert_eq!(
                    messages[0]["error"],
                    json!({"code": -32601, "message": "UnknownMethod"})
                );
            }
        }
    }

    #[test]
    fn unloaded_performance_get_metrics_does_not_use_legacy_fallback() {
        let mut ctx = TestContext::new();
        let raw = json!({ "id": 404, "method": "Performance.getMetrics" }).to_string();
        let step = ctx.conn.start_command_dispatch(&raw);
        let messages = complete_immediate_command_task_step_for_test(step);
        assert_eq!(messages.len(), 1);
        let metrics = metric_map(&messages[0]);
        assert!(metrics.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn performance_get_metrics_reads_live_document_shape() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><body><main><section></section><iframe srcdoc='<p>child</p>'></iframe></main></body>",
        )
        .await;

        ctx.process_async(json!({
            "id": 3,
            "method": "Performance.enable",
            "sessionId": "SID-1"
        }))
        .await;
        ctx.expect_result(3, json!({}), Some("SID-1"));

        ctx.process_async(json!({
            "id": 4,
            "method": "Performance.getMetrics",
            "sessionId": "SID-1"
        }))
        .await;

        let response = ctx.take_response_by_id(4);
        let metrics = metric_map(&response);
        assert!(metrics["Timestamp"] > 0.0);
        assert!(
            metrics["NavigationStart"] > 0.0,
            "navigation start should come from performance.timeOrigin"
        );
        assert!(metrics["DomContentLoaded"] >= metrics["NavigationStart"]);
        assert!(
            metrics["Frames"] >= 2.0,
            "iframe should contribute to frame count"
        );
        assert!(metrics["Documents"] >= 1.0);
        assert!(
            metrics["Nodes"] >= 4.0,
            "document element/body/content nodes should be counted"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn performance_state_is_isolated_between_inspector_sessions() {
        let mut ctx = TestContext::new();
        load_document(&mut ctx, "<!doctype html><body>session isolation</body>").await;
        assert!(
            ctx.conn
                .browser_context
                .as_mut()
                .expect("browser context")
                .assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned())
        );
        ctx.conn
            .apply_runtime_binding_state_for_session_owner_async(Some("SID-aux"))
            .await
            .expect("target attachment should establish the auxiliary renderer session");

        ctx.process_async(json!({
            "id": 4_100,
            "method": "Performance.enable",
            "sessionId": "SID-1"
        }))
        .await;
        ctx.expect_result(4_100, json!({}), Some("SID-1"));
        ctx.process_async(json!({
            "id": 4_101,
            "method": "Performance.getMetrics",
            "sessionId": "SID-aux"
        }))
        .await;
        assert!(metric_map(&ctx.take_response_by_id(4_101)).is_empty());

        ctx.process_async(json!({
            "id": 4_102,
            "method": "Performance.enable",
            "sessionId": "SID-aux",
            "params": { "timeDomain": "threadTicks" }
        }))
        .await;
        ctx.expect_result(4_102, json!({}), Some("SID-aux"));

        ctx.process_async(json!({
            "id": 4_103,
            "method": "Performance.disable",
            "sessionId": "SID-1"
        }))
        .await;
        ctx.expect_result(4_103, json!({}), Some("SID-1"));
        ctx.process_async(json!({
            "id": 4_104,
            "method": "Performance.getMetrics",
            "sessionId": "SID-aux"
        }))
        .await;
        assert!(!metric_map(&ctx.take_response_by_id(4_104)).is_empty());

        let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
        assert!(
            !browser_context.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .performance
                .enabled()
        );
        let auxiliary = browser_context
            .devtools_sessions
            .attached("SID-aux")
            .expect("auxiliary session state")
            .page_session_state
            .performance;
        assert!(auxiliary.enabled());
        assert_eq!(auxiliary.time_domain(), PerformanceTimeDomain::ThreadTicks);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn performance_get_metrics_can_complete_through_pending_command_dispatch() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><body><main><section></section><section></section></main></body>",
        )
        .await;

        ctx.process_async(json!({
            "id": 4_000,
            "method": "Performance.enable",
            "sessionId": "SID-1"
        }))
        .await;
        ctx.expect_result(4_000, json!({}), Some("SID-1"));

        let raw = json!({
            "id": 4_001,
            "method": "Performance.getMetrics",
            "sessionId": "SID-1"
        })
        .to_string();
        let step = ctx.conn.start_command_dispatch(&raw);
        let mut messages = complete_command_task_step_for_test(&mut ctx, step, 4_001).await;

        let response = messages
            .iter()
            .position(|message| message["id"] == json!(4_001))
            .map(|index| messages.remove(index))
            .expect("pending Performance.getMetrics should emit command response");
        assert_eq!(response["sessionId"], json!("SID-1"));
        let metrics = metric_map(&response);
        assert!(metrics["Timestamp"] > 0.0);
        assert!(metrics["Nodes"] >= 4.0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pending_performance_metrics_keep_captured_implicit_page_owner() {
        let mut ctx = TestContext::new();
        let active_url = "data:text/html,<body>active</body>";
        let background_url = "data:text/html,<body><main><section></section><section></section><section></section><section></section></main></body>";
        let mut browser_context = BrowserContext::new("BID-performance-owner".to_owned());
        browser_context.set_active_target_id("TID-performance-active");
        browser_context.attach_active_session("SID-performance-active");
        browser_context.set_target_url(active_url.to_owned());
        browser_context.stage_background_target(
            "TID-performance-background".to_owned(),
            Some("SID-performance-background".to_owned()),
            background_url.to_owned(),
            None,
            None,
        );
        ctx.conn.browser_context = Some(browser_context);
        ctx.install_navigation_fixture_for_session_owner(
            active_url,
            Some("SID-performance-active"),
        )
        .await;
        ctx.install_navigation_fixture_for_session_owner(
            background_url,
            Some("SID-performance-background"),
        )
        .await;

        let background_route = ctx
            .conn
            .target_session_route_for_target_id("TID-performance-background")
            .expect("background Performance route");
        let previous_route = ctx
            .conn
            .replace_none_session_owner_route_override(Some(background_route.clone()));
        let enable = ctx
            .conn
            .start_command_dispatch(r#"{"id":4200,"method":"Performance.enable"}"#);
        let enable_messages = complete_immediate_command_task_step_for_test(enable);
        ctx.conn
            .replace_none_session_owner_route_override(previous_route);
        assert_eq!(enable_messages[0]["result"], json!({}));

        let previous_route = ctx
            .conn
            .replace_none_session_owner_route_override(Some(background_route));
        let pending = ctx
            .conn
            .start_command_dispatch(r#"{"id":4201,"method":"Performance.getMetrics"}"#);
        ctx.conn
            .replace_none_session_owner_route_override(previous_route);
        assert!(matches!(pending, CdpCommandTaskStep::Pending(_)));

        let active_route = ctx
            .conn
            .target_session_route_for_target_id("TID-performance-active")
            .expect("active Performance route");
        let previous_route = ctx
            .conn
            .replace_none_session_owner_route_override(Some(active_route));
        let messages = complete_command_task_step_for_test(&mut ctx, pending, 4201).await;
        ctx.conn
            .replace_none_session_owner_route_override(previous_route);
        let response = messages
            .iter()
            .find(|message| message["id"] == json!(4201))
            .expect("pending background Performance response");
        assert!(
            metric_map(response)["Nodes"] >= 8.0,
            "completion must finish the snapshot on the captured background Page"
        );
        assert_eq!(ctx.conn.none_session_owner_route_override(), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn performance_get_metrics_reads_the_snapshot_bound_at_each_replacement() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><body><main><section></section><section></section></main></body>",
        )
        .await;

        ctx.process_async(json!({
            "id": 4_010,
            "method": "Performance.enable",
            "sessionId": "SID-1"
        }))
        .await;
        ctx.expect_result(4_010, json!({}), Some("SID-1"));

        let raw = json!({
            "id": 4_011,
            "method": "Performance.getMetrics",
            "sessionId": "SID-1"
        })
        .to_string();
        let step = ctx.conn.start_command_dispatch(&raw);
        let old_messages = complete_command_task_step_for_test(&mut ctx, step, 4_011).await;
        let old_response = old_messages
            .iter()
            .find(|message| message["id"] == json!(4_011))
            .expect("old-page getMetrics response");
        let old_metrics = metric_map(old_response);
        assert!(old_metrics["Documents"] >= 1.0);
        assert!(old_metrics["Nodes"] >= 6.0);

        ctx.install_navigation_fixture_for_session_owner(
            "data:text/html,<!doctype html><body><article>replacement</article></body>",
            Some("SID-1"),
        )
        .await;

        let replacement_raw = json!({
            "id": 4_012,
            "method": "Performance.getMetrics",
            "sessionId": "SID-1"
        })
        .to_string();
        let step = ctx.conn.start_command_dispatch(&replacement_raw);
        let replacement_messages = complete_command_task_step_for_test(&mut ctx, step, 4_012).await;
        let replacement_response = replacement_messages
            .iter()
            .find(|message| message["id"] == json!(4_012))
            .expect("replacement getMetrics response");
        let replacement_metrics = metric_map(replacement_response);
        assert!(replacement_metrics["Documents"] >= 1.0);
        assert!(replacement_metrics["Nodes"] < old_metrics["Nodes"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn performance_get_metrics_targets_loaded_background_owner_without_promotion() {
        let mut ctx = TestContext::new();
        let background_url = "data:text/html,<!doctype html><body><main><section></section><section></section></main></body>";
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-active".to_owned());
        bc.attach_active_session("SID-active".to_owned());
        bc.set_target_url("data:text/html,<body>active</body>".to_owned());
        bc.stage_background_target(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            background_url.to_owned(),
            None,
            None,
        );
        ctx.conn.browser_context = Some(bc);
        ctx.install_navigation_fixture_for_session_owner(background_url, Some("SID-background"))
            .await;

        ctx.process_async(json!({
            "id": 5,
            "method": "Performance.enable",
            "sessionId": "SID-background"
        }))
        .await;
        ctx.expect_result(5, json!({}), Some("SID-background"));

        ctx.process_async(json!({
            "id": 6,
            "method": "Performance.getMetrics",
            "sessionId": "SID-background"
        }))
        .await;

        let response = ctx.take_response_by_id(6);
        assert_eq!(response["sessionId"], json!("SID-background"));
        let metrics = metric_map(&response);
        assert!(metrics["Nodes"] >= 5.0);
        let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(browser_context.active_target_id(), Some("TID-active"));
        assert!(
            browser_context
                .background_target("TID-background")
                .is_some_and(|target| target.has_loaded_page()),
            "Performance.getMetrics should not promote the loaded background owner"
        );
    }

    #[tokio::test]
    async fn performance_get_metrics_on_unloaded_background_owner_does_not_promote() {
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
            "id": 7,
            "method": "Performance.getMetrics",
            "sessionId": "SID-background"
        }))
        .await;

        let response = ctx.take_response_by_id(7);
        assert_eq!(response["sessionId"], json!("SID-background"));
        let metrics = metric_map(&response);
        assert!(metrics.is_empty());

        ctx.process_async(json!({
            "id": 8,
            "method": "Performance.enable",
            "sessionId": "SID-background"
        }))
        .await;
        ctx.expect_result(8, json!({}), Some("SID-background"));
        ctx.process_async(json!({
            "id": 9,
            "method": "Performance.getMetrics",
            "sessionId": "SID-background"
        }))
        .await;
        assert!(metric_map(&ctx.take_response_by_id(9))["Timestamp"] > 0.0);
        let browser_context = ctx.conn.browser_context.as_ref().expect("browser context");
        assert_eq!(browser_context.active_target_id(), Some("TID-active"));
        assert!(
            browser_context
                .background_target("TID-background")
                .is_some_and(|target| !target.has_loaded_page()),
            "unloaded Performance owner fallback should not promote the background target"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn performance_enable_stages_background_target_session_state() {
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
            "id": 5,
            "method": "Performance.enable",
            "sessionId": "SID-background"
        }))
        .await;

        ctx.expect_result(5, json!({}), Some("SID-background"));
        let active = ctx.conn.browser_context.as_ref().expect("browser context");
        assert!(
            !active.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .performance
                .enabled()
        );
        assert!(
            active
                .parked_page_session_state("TID-background")
                .is_some_and(|state| state.devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .page_session_state
                    .performance
                    .enabled()),
            "background target should stage Performance.enable"
        );
    }
}
