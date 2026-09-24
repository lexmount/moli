use moli_core::page::RendererPageDiagnosticsSnapshot;
use std::time::Instant;

use super::{CdpConnection, CommandOwnerScope, PendingChildFrameLifecycleWork};

impl CdpConnection {
    pub(crate) fn start_child_frame_lifecycle_work_for_owner(
        &mut self,
        owner: CommandOwnerScope,
        timeout: std::time::Duration,
    ) -> Result<PendingChildFrameLifecycleWork, String> {
        let document = self.loaded_browser_document_for_owner(&owner)?;
        self.start_document_child_frame_lifecycle_work(document, timeout)
    }

    pub async fn page_diagnostics_snapshot_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<RendererPageDiagnosticsSnapshot, String> {
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        trace_activity_source_stage(
            "conn_page_diagnostics_snapshot_start",
            session_id,
            trace_started,
        );
        let owner = CommandOwnerScope::capture(self, session_id);
        let Ok(document) = self.loaded_browser_document_for_owner(&owner) else {
            trace_activity_source_stage(
                "conn_page_diagnostics_snapshot_missing_owner",
                session_id,
                trace_started,
            );
            return Ok(RendererPageDiagnosticsSnapshot::default());
        };
        let renderer_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let completed = self
            .start_document_diagnostics_snapshot(document)?
            .wait()
            .await;
        trace_activity_source_stage(
            "conn_page_diagnostics_snapshot_renderer_done",
            session_id,
            renderer_started,
        );
        let ingest_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let snapshot = self.finish_document_diagnostics_snapshot(completed)?;
        trace_activity_source_stage_with_bool(
            "conn_page_diagnostics_snapshot_ingest_done",
            session_id,
            ingest_started,
            true,
        );
        trace_activity_source_stage(
            "conn_page_diagnostics_snapshot_done",
            session_id,
            trace_started,
        );
        Ok(snapshot)
    }
}

fn trace_activity_source_stage(
    stage: &'static str,
    session_id: Option<&str>,
    started: Option<Instant>,
) {
    if let Some(started) = started {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage = stage,
            session_id = ?session_id,
            elapsed_us = %started.elapsed().as_micros(),
        );
    }
}

fn trace_activity_source_stage_with_bool(
    stage: &'static str,
    session_id: Option<&str>,
    started: Option<Instant>,
    value: bool,
) {
    if let Some(started) = started {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage = stage,
            session_id = ?session_id,
            value,
            elapsed_us = %started.elapsed().as_micros(),
        );
    }
}
