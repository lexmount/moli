use moli_core::page::{InspectorIssueSnapshot, RuntimeConsoleMessageSnapshot};
#[cfg(test)]
use moli_core::page::{RendererPageDiagnosticsSnapshot, RendererRuntimeObservableSourceSummary};

#[cfg(test)]
use super::ObservableOutputProjectionStep;
use super::output_queue::{
    ObservablePreparedOutputs, ObservableSessionAuditsPreparedRange, TargetObservableOutputQueue,
};
use crate::conn::CdpConnection;

#[derive(Clone, Copy)]
enum RuntimeObservableAudience {
    SourceSession,
    EnabledTargetAttachments,
}

#[cfg(test)]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::domains) struct ObservableActivityOutputs {
    prepared: ObservablePreparedOutputs,
}

#[cfg(test)]
impl ObservableActivityOutputs {
    pub(in crate::domains) fn outputs(&self) -> Vec<ObservableOutputProjectionStep> {
        let mut outputs = Vec::new();
        if self.prepared.has_audits() {
            outputs.push(ObservableOutputProjectionStep::Audits);
        }
        if self.prepared.has_console() {
            outputs.push(ObservableOutputProjectionStep::Console);
        }
        if self.prepared.has_log() {
            outputs.push(ObservableOutputProjectionStep::Log);
        }
        if self.prepared.has_runtime_observable() {
            outputs.push(ObservableOutputProjectionStep::RuntimeObservable);
        }
        outputs
    }

    pub(crate) fn into_prepared_slot(self) -> super::ObservablePreparedOutputSlot {
        super::ObservablePreparedOutputSlot::from_outputs(self.prepared)
    }

    #[cfg(test)]
    pub(crate) fn console_outputs(&self) -> Vec<ObservableOutputProjectionStep> {
        self.outputs()
            .into_iter()
            .filter(|output| matches!(output, ObservableOutputProjectionStep::Console))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn log_outputs(&self) -> Vec<ObservableOutputProjectionStep> {
        self.outputs()
            .into_iter()
            .filter(|output| matches!(output, ObservableOutputProjectionStep::Log))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn runtime_observable_outputs(&self) -> Vec<ObservableOutputProjectionStep> {
        self.outputs()
            .into_iter()
            .filter(|output| matches!(output, ObservableOutputProjectionStep::RuntimeObservable))
            .collect()
    }
}

#[cfg(test)]
pub(in crate::domains) fn observable_source_activity_outputs(
    conn: &mut CdpConnection,
    snapshot: &RendererPageDiagnosticsSnapshot,
    session_id: Option<&str>,
) -> ObservableActivityOutputs {
    ObservableActivityOutputs {
        prepared: observable_source_prepared_outputs(conn, snapshot, session_id),
    }
}

pub(in crate::domains) fn runtime_console_message_prepared_outputs(
    conn: &mut CdpConnection,
    message: RuntimeConsoleMessageSnapshot,
    session_id: Option<&str>,
) -> ObservablePreparedOutputs {
    let mut prepared = ObservablePreparedOutputs::default();
    let Some(source) = runtime_console_source_tail_for_session_owner(conn, message, session_id)
    else {
        return prepared;
    };
    push_runtime_observable_tail_prepared_outputs(
        &mut prepared,
        conn,
        &source,
        session_id,
        true,
        RuntimeObservableAudience::SourceSession,
    );
    prepared
}

pub(in crate::domains) fn runtime_lifecycle_error_prepared_outputs(
    conn: &mut CdpConnection,
    text: String,
    execution_context_id: Option<i64>,
    session_id: Option<&str>,
) -> ObservablePreparedOutputs {
    let mut prepared = ObservablePreparedOutputs::default();
    let Some(source) = runtime_lifecycle_error_source_tail_for_session_owner(
        conn,
        text,
        execution_context_id,
        session_id,
    ) else {
        return prepared;
    };
    push_runtime_observable_tail_prepared_outputs(
        &mut prepared,
        conn,
        &source,
        session_id,
        true,
        RuntimeObservableAudience::EnabledTargetAttachments,
    );
    prepared
}

pub(in crate::domains) fn inspector_issue_prepared_outputs(
    conn: &mut CdpConnection,
    source_document: moli_core::RendererDocumentLifecycleIdentity,
    issue: InspectorIssueSnapshot,
    session_id: Option<&str>,
) -> ObservablePreparedOutputs {
    let mut prepared = ObservablePreparedOutputs::default();
    // Inspector issues are Document facts. Resolve them only against the
    // exact committed Document captured by the renderer; otherwise a late
    // issue from a retired generation could be stored and replayed as if it
    // belonged to its replacement.
    let Some(document_binding) = conn
        .runtime_session_owner_slot(session_id)
        .ok()
        .and_then(|slot| slot.committed_renderer_document_binding())
    else {
        return prepared;
    };
    if document_binding.renderer_document_identity() != source_document {
        return prepared;
    }
    let page_attachment_id = document_binding.page_attachment_id;
    let frame_id = document_binding.frame_id.clone();
    let loader_id = document_binding.loader_id.clone();
    let Some(storage) = conn.with_target_owner_state_for_session_mut(session_id, |owner_state| {
        owner_state
            .audits_storage_state
            .append_concrete_issue(issue);
        owner_state.audits_storage_state.clone()
    }) else {
        return prepared;
    };

    // One concrete Page fact updates the target-owned replay storage once,
    // then fans out to every enabled frontend session. Each session keeps its
    // own cursor, matching Blink's per-session Audits agent state without
    // rediscovering the issue from a later Page snapshot.
    for event_session_id in conn.page_event_session_ids_for_session_owner(session_id) {
        let event_session_id = event_session_id.as_deref();
        let Some(cursor) = conn
            .target_page_session_state_for_session(event_session_id)
            .and_then(|state| state.audits.pending_cursor(&storage))
        else {
            continue;
        };
        let Some(issues) = storage.issues_for_cursor(cursor) else {
            continue;
        };
        prepared.push_audits(ObservableSessionAuditsPreparedRange::new(
            event_session_id,
            source_document,
            frame_id.clone(),
            loader_id.clone(),
            page_attachment_id,
            issues,
            cursor,
        ));
    }
    prepared
}

#[cfg(test)]
fn observable_source_prepared_outputs(
    conn: &mut CdpConnection,
    snapshot: &RendererPageDiagnosticsSnapshot,
    session_id: Option<&str>,
) -> ObservablePreparedOutputs {
    let mut prepared = ObservablePreparedOutputs::default();
    if let Some(source) = snapshot.runtime_observable_source() {
        push_runtime_observable_source_prepared_outputs(&mut prepared, conn, source, session_id);
    }
    prepared
}

#[cfg(test)]
fn push_runtime_observable_source_prepared_outputs(
    prepared: &mut ObservablePreparedOutputs,
    conn: &mut CdpConnection,
    source: &RendererRuntimeObservableSourceSummary,
    session_id: Option<&str>,
) {
    let Some(source) = runtime_observable_source_tail_for_session_owner(conn, source, session_id)
    else {
        return;
    };
    push_runtime_observable_tail_prepared_outputs(
        prepared,
        conn,
        &source,
        session_id,
        false,
        RuntimeObservableAudience::SourceSession,
    );
}

fn push_runtime_observable_tail_prepared_outputs(
    prepared: &mut ObservablePreparedOutputs,
    conn: &mut CdpConnection,
    source: &super::TargetRuntimeObservableSourceOutput,
    session_id: Option<&str>,
    deliver_log_to_enabled_sessions: bool,
    runtime_audience: RuntimeObservableAudience,
) {
    let owner_state = conn
        .target_owner_state_for_session(session_id)
        .cloned()
        .unwrap_or_default();
    let include_console_api_messages =
        !renderer_agent_owns_page_console_api_events(conn, session_id);

    push_console_log_source_prepared_outputs(
        prepared,
        conn,
        source,
        session_id,
        &owner_state,
        include_console_api_messages,
        deliver_log_to_enabled_sessions,
    );

    let attachments = match runtime_audience {
        RuntimeObservableAudience::SourceSession => conn
            .target_page_protocol_attachment_identity_for_session(session_id)
            .into_iter()
            .collect(),
        RuntimeObservableAudience::EnabledTargetAttachments => conn
            .runtime_event_protocol_attachments_for_session_owner(session_id)
            .unwrap_or_default(),
    };
    for attachment in attachments {
        let event_session_id = attachment.session_id();
        if !conn
            .target_runtime_session_state_for_session(event_session_id)
            .is_some_and(|state| state.runtime_frontend_enabled)
        {
            continue;
        }
        let include_runtime_console_api_messages =
            !renderer_runtime_agent_owns_page_console_api_events(conn, event_session_id);
        if let Some(items) = source.source_items_prepared_for_state(
            &owner_state.runtime_observable_state,
            include_runtime_console_api_messages,
        ) {
            prepared.push_runtime_observable_items(attachment, items);
        }
    }
}

fn push_console_log_source_prepared_outputs(
    prepared: &mut ObservablePreparedOutputs,
    conn: &CdpConnection,
    source: &super::TargetRuntimeObservableSourceOutput,
    session_id: Option<&str>,
    owner_state: &crate::conn::TargetOwnerState,
    include_console_api_messages: bool,
    deliver_log_to_enabled_sessions: bool,
) {
    let queue = TargetObservableOutputQueue::from_runtime_source_output_ref(Some(source));
    if let Some(devtools_session_state) = conn.target_devtools_session_state_for_session(session_id)
    {
        prepared.extend(
            queue.console_log_backlog_ranges(
                source.url(),
                source.page_attachment_id(),
                devtools_session_state
                    .console_output_session_state
                    .console_enabled,
                false,
                include_console_api_messages,
                owner_state,
                &devtools_session_state.console_output_session_state,
                session_id,
            ),
        );
    }

    // The cfg(test) snapshot synchronizer above may update replay storage, but
    // it is deliberately not a second live-event producer. Only the concrete
    // renderer record which introduced the fact may fan it out to Log agents.
    if !deliver_log_to_enabled_sessions {
        return;
    }

    push_log_prepared_outputs_for_enabled_sessions(
        prepared,
        conn,
        &queue,
        source.url(),
        source.page_attachment_id(),
        owner_state,
        session_id,
    );
}

fn renderer_agent_owns_page_console_api_events(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> bool {
    if session_id.is_some_and(|session_id| {
        conn.shared_worker_target_for_session(Some(session_id))
            .is_some()
    }) {
        return false;
    }
    let Ok(runtime_slot) = conn.runtime_session_owner_slot(session_id) else {
        return false;
    };
    if !runtime_slot.has_loaded_page() {
        return false;
    }
    conn.target_devtools_session_state_for_session(session_id)
        .is_some_and(|state| {
            state
                .console_output_session_state
                .renderer_console_agent_owns_page_console_api_events
        })
}

fn renderer_runtime_agent_owns_page_console_api_events(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> bool {
    if session_id.is_some_and(|session_id| {
        conn.shared_worker_target_for_session(Some(session_id))
            .is_some()
    }) {
        return false;
    }
    let Ok(runtime_slot) = conn.runtime_session_owner_slot(session_id) else {
        return false;
    };
    if !runtime_slot.has_loaded_page() {
        return false;
    }
    conn.target_devtools_session_state_for_session(session_id)
        .is_some_and(|state| {
            state
                .console_output_session_state
                .renderer_runtime_agent_owns_page_console_api_events
        })
}

#[cfg(test)]
fn runtime_observable_source_tail_for_session_owner(
    conn: &mut CdpConnection,
    source: &RendererRuntimeObservableSourceSummary,
    session_id: Option<&str>,
) -> Option<super::TargetRuntimeObservableSourceOutput> {
    let url = conn.runtime_session_owner_target_url(session_id)?;
    let runtime_slot = conn.runtime_session_owner_slot_mut(session_id).ok()?;
    runtime_slot.sync_observable_output_source_from_renderer_runtime_source(url, source)
}

fn runtime_console_source_tail_for_session_owner(
    conn: &mut CdpConnection,
    message: RuntimeConsoleMessageSnapshot,
    session_id: Option<&str>,
) -> Option<super::TargetRuntimeObservableSourceOutput> {
    let url = conn.runtime_session_owner_target_url(session_id)?;
    let runtime_slot = conn.runtime_session_owner_slot_mut(session_id).ok()?;
    runtime_slot.append_renderer_runtime_console_message(url, message)
}

fn runtime_lifecycle_error_source_tail_for_session_owner(
    conn: &mut CdpConnection,
    text: String,
    execution_context_id: Option<i64>,
    session_id: Option<&str>,
) -> Option<super::TargetRuntimeObservableSourceOutput> {
    let url = conn.runtime_session_owner_target_url(session_id)?;
    let runtime_slot = conn.runtime_session_owner_slot_mut(session_id).ok()?;
    runtime_slot.append_renderer_runtime_lifecycle_error(url, text, execution_context_id)
}

#[cfg(test)]
pub(in crate::domains) fn observable_backlog_activity_outputs_for_session_owner(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> ObservableActivityOutputs {
    ObservableActivityOutputs {
        prepared: observable_backlog_prepared_outputs_for_session_owner(conn, session_id),
    }
}

#[cfg(test)]
pub(in crate::domains) fn observable_backlog_prepared_outputs_for_session_owner(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> ObservablePreparedOutputs {
    observable_console_log_prepared_outputs_for_session_owner(conn, session_id, true, true)
        .unwrap_or_default()
}

/// Freezes the Log-domain fan-out caused by the renderer Network fact that was
/// just ingested. Late `Log.enable` replay uses `TargetLogReplaySnapshot`
/// instead and never calls this live projection entry point.
pub(in crate::domains) fn live_log_prepared_outputs_for_renderer_network_fact(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> ObservablePreparedOutputs {
    let owner_state = conn
        .target_owner_state_for_session(session_id)
        .cloned()
        .unwrap_or_default();
    let Some(runtime_slot) = conn.runtime_session_owner_slot(session_id).ok() else {
        return ObservablePreparedOutputs::default();
    };
    let Some(url) = conn.runtime_session_owner_target_url(session_id) else {
        return ObservablePreparedOutputs::default();
    };
    let Some(queue) = TargetObservableOutputQueue::from_log_storage(runtime_slot) else {
        return ObservablePreparedOutputs::default();
    };
    let Some(page_attachment_id) = runtime_slot.page_attachment_id() else {
        return ObservablePreparedOutputs::default();
    };
    let mut prepared = ObservablePreparedOutputs::default();
    push_log_prepared_outputs_for_enabled_sessions(
        &mut prepared,
        conn,
        &queue,
        &url,
        page_attachment_id,
        &owner_state,
        session_id,
    );
    prepared
}

#[cfg(test)]
fn observable_console_log_prepared_outputs_for_session_owner(
    conn: &CdpConnection,
    session_id: Option<&str>,
    console_allowed: bool,
    log_allowed: bool,
) -> Option<ObservablePreparedOutputs> {
    let owner_state = conn
        .target_owner_state_for_session(session_id)
        .cloned()
        .unwrap_or_default();
    let runtime_slot = conn.runtime_session_owner_slot(session_id).ok()?;
    let url = conn.runtime_session_owner_target_url(session_id)?;
    let queue = super::output_queue::TargetObservableOutputQueue::from_runtime_slot(runtime_slot)?;
    let page_attachment_id = runtime_slot.page_attachment_id()?;
    let include_console_api_messages =
        !renderer_agent_owns_page_console_api_events(conn, session_id);
    let mut prepared = ObservablePreparedOutputs::default();

    if console_allowed
        && let Some(devtools_session_state) =
            conn.target_devtools_session_state_for_session(session_id)
    {
        prepared.extend(
            queue.console_log_backlog_ranges(
                &url,
                page_attachment_id,
                devtools_session_state
                    .console_output_session_state
                    .console_enabled,
                false,
                include_console_api_messages,
                &owner_state,
                &devtools_session_state.console_output_session_state,
                session_id,
            ),
        );
    }

    if log_allowed {
        push_log_prepared_outputs_for_enabled_sessions(
            &mut prepared,
            conn,
            &queue,
            &url,
            page_attachment_id,
            &owner_state,
            session_id,
        );
    }

    Some(prepared)
}

fn push_log_prepared_outputs_for_enabled_sessions(
    prepared: &mut ObservablePreparedOutputs,
    conn: &CdpConnection,
    queue: &TargetObservableOutputQueue,
    url: &str,
    page_attachment_id: crate::conn::TargetPageAttachmentId,
    owner_state: &crate::conn::TargetOwnerState,
    session_id: Option<&str>,
) {
    // A concrete Page error is stored once but observed by every Log agent
    // attached to that target. The publication route names one canonical
    // session only; it must not accidentally turn a target-owned fact into a
    // single-session event.
    for event_session_id in conn.page_event_session_ids_for_session_owner(session_id) {
        let event_session_id = event_session_id.as_deref();
        let Some(devtools_session_state) =
            conn.target_devtools_session_state_for_session(event_session_id)
        else {
            continue;
        };
        if !devtools_session_state.page_session_state.log_enabled {
            continue;
        }
        prepared.extend(queue.console_log_backlog_ranges(
            url,
            page_attachment_id,
            false,
            true,
            true,
            owner_state,
            &devtools_session_state.console_output_session_state,
            event_session_id,
        ));
    }
}

#[cfg(test)]
mod tests {
    use moli_core::page::{
        RendererActivityDiagnostics, RendererPageDiagnosticsSnapshot,
        RendererRuntimeObservableSourceSummary, RuntimeConsoleMessageSnapshot,
    };

    use crate::conn::BrowserContext;
    use crate::domains::observable_output::output_queue::TargetObservableOutputQueue;
    use crate::testing::TestContext;

    use super::{
        ObservableOutputProjectionStep,
        observable_backlog_activity_outputs_for_session_owner as observable_backlog_activity_outputs,
        observable_source_activity_outputs, runtime_lifecycle_error_prepared_outputs,
    };

    fn runtime_lifecycle_error_audience(enabled_session_ids: &[&str]) -> Vec<Option<String>> {
        let mut conn = crate::conn::CdpConnection::default();
        let mut bc = BrowserContext::new("BID-runtime-lifecycle".to_owned());
        bc.set_active_target_id("TID-runtime-lifecycle".to_owned());
        bc.set_target_url("https://example.test/runtime-lifecycle".to_owned());
        bc.attach_active_session("SID-runtime-a".to_owned());
        assert!(bc.assign_auxiliary_session_to_target(
            "TID-runtime-lifecycle",
            "SID-runtime-b".to_owned(),
        ));
        assert!(bc.assign_auxiliary_session_to_target(
            "TID-runtime-lifecycle",
            "SID-runtime-disabled".to_owned(),
        ));
        bc.active_page_state_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(17);
        conn.browser_context = Some(bc);
        for session_id in enabled_session_ids {
            conn.with_target_devtools_session_state_for_session_mut(Some(session_id), |state| {
                state.runtime_session_state.runtime_frontend_enabled = true
            })
            .expect("Runtime audience session should be mutable");
        }

        runtime_lifecycle_error_prepared_outputs(
            &mut conn,
            "uncaught timer error".to_owned(),
            Some(7),
            Some("SID-runtime-a"),
        )
        .take_runtime_observable_items()
        .into_iter()
        .map(|prepared| prepared.session_id().map(str::to_owned))
        .collect()
    }

    #[test]
    fn observable_source_outputs_own_runtime_observable_presence() {
        let mut conn = crate::conn::CdpConnection::default();
        let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        bc.active_page_state_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        bc.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
        conn.browser_context = Some(bc);

        assert_eq!(
            observable_source_activity_outputs(
                &mut conn,
                &RendererPageDiagnosticsSnapshot::from_runtime_observable_source(
                    RendererRuntimeObservableSourceSummary::from_source_messages(
                        Some(7),
                        Vec::new(),
                        vec!["runtime observable source".to_owned()],
                    ),
                ),
                None,
            )
            .runtime_observable_outputs(),
            &[ObservableOutputProjectionStep::RuntimeObservable],
            "RuntimeObservable source presence should be part of the observable output queue batch"
        );
    }

    #[test]
    fn runtime_lifecycle_error_freezes_every_enabled_attachment_only() {
        assert_eq!(
            runtime_lifecycle_error_audience(&["SID-runtime-b"]),
            vec![Some("SID-runtime-b".to_owned())],
            "a disabled source attachment must not hide its enabled peer"
        );
        assert_eq!(
            runtime_lifecycle_error_audience(&["SID-runtime-a", "SID-runtime-b"]),
            vec![
                Some("SID-runtime-a".to_owned()),
                Some("SID-runtime-b".to_owned()),
            ],
            "one target-owned exception must freeze all Runtime-enabled attachments"
        );
    }

    #[test]
    fn observable_source_sync_is_independent_from_runtime_emission() {
        let mut conn = crate::conn::CdpConnection::default();
        let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        bc.set_target_url("data:text/html,console-only-source".to_owned());
        bc.active_page_state_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        bc.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .console_enabled = true;
        bc.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .log_enabled = true;
        bc.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = false;
        conn.browser_context = Some(bc);

        let outputs = observable_source_activity_outputs(
            &mut conn,
            &RendererPageDiagnosticsSnapshot::from_runtime_observable_source(
                RendererRuntimeObservableSourceSummary::from_source_messages(
                    Some(7),
                    vec![RuntimeConsoleMessageSnapshot {
                        execution_context_id: 7,
                        message: "console before runtime enable".to_owned(),
                        args: Vec::new(),
                        stack: None,
                    }],
                    vec!["lifecycle before runtime enable".to_owned()],
                ),
            ),
            None,
        );

        assert_eq!(
            outputs.console_outputs(),
            &[ObservableOutputProjectionStep::Console],
            "Console source output should be visible even when Runtime is disabled"
        );
        assert_eq!(
            outputs.log_outputs(),
            &[],
            "renderer source output should synchronize Log storage without creating a second delivery owner"
        );
        assert!(
            outputs.runtime_observable_outputs().is_empty(),
            "Runtime.consoleAPICalled emission should remain gated by Runtime.enable"
        );
    }

    #[test]
    fn observable_source_outputs_require_concrete_runtime_prepared_items() {
        let mut conn = crate::conn::CdpConnection::default();
        let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-1");
        bc.active_page_state_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        bc.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
        conn.browser_context = Some(bc);

        assert!(
            observable_source_activity_outputs(
                &mut conn,
                &RendererPageDiagnosticsSnapshot::from_diagnostics(RendererActivityDiagnostics {
                    runtime_console_messages_with_context: 1,
                    runtime_lifecycle_errors: 1,
                    ..Default::default()
                },),
                None,
            )
            .runtime_observable_outputs()
            .is_empty(),
            "RuntimeObservable source presence should require concrete prepared items, not a diagnostics-only readback token"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn observable_prepared_outputs_keep_console_api_out_of_log_backlog() {
        let mut ctx = TestContext::new();
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_active_target_id("TID-1".to_owned());
        bc.set_target_url("data:text/html,observable-backlog-test".to_owned());
        bc.attach_active_session("SID-1".to_owned());
        let page = ctx
            .conn
            .load_page_via_runtime_async(
                "data:text/html,<!doctype html><script>console.warn('observable')</script>",
            )
            .await
            .expect("test page should load");
        let _ = bc
            .active_page_state_mut()
            .runtime_slot
            .replace_loaded_page(Some(page));
        bc.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .console_enabled = true;
        bc.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .log_enabled = true;
        ctx.conn.browser_context = Some(bc);

        let outputs = observable_backlog_activity_outputs(&ctx.conn, None);
        assert_eq!(
            outputs.console_outputs(),
            &[ObservableOutputProjectionStep::Console],
            "Console backlog cursor should be batched as observable output"
        );
        assert_eq!(
            outputs.log_outputs(),
            &[],
            "Log backlog should ignore console API messages"
        );
        let mut prepared = outputs.into_prepared_slot();
        assert!(
            prepared
                .outputs_mut_for_test()
                .take_console_range()
                .is_some(),
            "Console backlog output should carry a prepared Console drain range"
        );
        assert!(
            prepared.outputs_mut_for_test().take_log_range().is_none(),
            "console-only backlog should not carry a prepared Log drain range"
        );

        let (console_message_count, lifecycle_error_count) = {
            let runtime_slot = ctx
                .conn
                .browser_context
                .as_ref()
                .map(|bc| &bc.active_page_state().runtime_slot)
                .expect("browser context should be loaded");
            let queue = TargetObservableOutputQueue::from_runtime_slot(runtime_slot)
                .expect("queue should load");
            (queue.console_message_count(), queue.lifecycle_error_count())
        };
        assert_eq!(
            console_message_count, 1,
            "observable backlog queue should capture the renderer console output count"
        );
        assert_eq!(
            lifecycle_error_count, 0,
            "observable backlog queue should capture the renderer lifecycle error count"
        );
        assert_eq!(
            observable_backlog_activity_outputs(&ctx.conn, None).console_outputs(),
            &[ObservableOutputProjectionStep::Console],
            "Log domain absence should not consume Console domain output"
        );
        assert!(
            observable_backlog_activity_outputs(&ctx.conn, None)
                .log_outputs()
                .is_empty(),
            "console-only backlog should not leave Log visible"
        );
    }
}
