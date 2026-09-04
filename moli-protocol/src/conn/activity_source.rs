use moli_core::page::{CompletedPageCommand, PendingPageCommand, RendererPageDiagnosticsSnapshot};
use std::time::Instant;

use super::{CdpConnection, CdpSessionRoute, CommandOwnerScope, TargetRuntimeSlot};

pub(crate) struct PendingChildFrameLifecycleWork {
    owner: CommandOwnerScope,
    pending: PendingPageCommand,
}

pub(crate) struct CompletedChildFrameLifecycleWork {
    owner: CommandOwnerScope,
    completion: CompletedPageCommand,
}

impl PendingChildFrameLifecycleWork {
    pub(crate) async fn wait(self) -> Result<CompletedChildFrameLifecycleWork, String> {
        let completion = self
            .pending
            .wait()
            .await
            .map_err(|error| error.to_string())?;
        Ok(CompletedChildFrameLifecycleWork {
            owner: self.owner,
            completion,
        })
    }
}

impl CdpConnection {
    fn activity_source_runtime_slot_mut(
        &mut self,
        session_id: Option<&str>,
    ) -> Option<&mut TargetRuntimeSlot> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.activity_source_runtime_slot_mut_for_owner(&owner)
    }

    fn activity_source_runtime_slot_mut_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Option<&mut TargetRuntimeSlot> {
        let route = owner.resolve_route(self)?;
        let (browser_context_id, target_id) = match &route {
            CdpSessionRoute::PageTarget {
                browser_context_id,
                target_id,
                ..
            } => (browser_context_id.clone(), target_id.clone()),
            CdpSessionRoute::Browser => {
                let context = self.browser_context.as_ref()?;
                (context.id.clone(), context.active_target_id()?.to_owned())
            }
            CdpSessionRoute::BrowserContext { browser_context_id } => (
                browser_context_id.clone(),
                self.browser_context_by_id(browser_context_id)?
                    .active_target_id()?
                    .to_owned(),
            ),
            CdpSessionRoute::TabTarget { .. }
            | CdpSessionRoute::SharedWorkerTarget { .. }
            | CdpSessionRoute::DedicatedWorkerTarget { .. }
            | CdpSessionRoute::ServiceWorkerTarget { .. } => return None,
        };
        self.ensure_page_navigation_engine_for_target(&browser_context_id, &target_id)?;
        let target = self
            .browser_context_by_id_mut(&browser_context_id)?
            .page_target_mut(&target_id)?;
        target.navigation_engine()?;
        Some(&mut target.runtime_slot)
    }

    pub(crate) fn start_child_frame_lifecycle_work_for_owner(
        &mut self,
        owner: CommandOwnerScope,
        timeout: std::time::Duration,
    ) -> Result<PendingChildFrameLifecycleWork, String> {
        let storage = self
            .navigation_load_inputs_for_owner(&owner)
            .resource_storage_handles();
        let Some(slot) = self.activity_source_runtime_slot_mut_for_owner(&owner) else {
            return Err("NoDocumentLoaded".to_owned());
        };
        let contents = &mut slot.page_slot_mut().contents;
        let Some(document) = contents.main_frame.current_document.as_ref() else {
            return Err("NoDocumentLoaded".to_owned());
        };
        let engine = contents
            .navigation_engine
            .as_mut()
            .expect("resolved navigation engine");
        let pending = engine
            .start_page_child_frame_lifecycle_work_with_storage_best_effort(
                storage.into_navigation_storage(),
                &document.page,
                timeout,
            )
            .map_err(|error| error.to_string())?;
        Ok(PendingChildFrameLifecycleWork { owner, pending })
    }

    pub(crate) fn complete_child_frame_lifecycle_work_command_turn_for_session_owner(
        &mut self,
        pending: CompletedChildFrameLifecycleWork,
    ) -> Result<(bool, moli_core::page::RendererCommandTurnOutput), String> {
        let Some(slot) = self.activity_source_runtime_slot_mut_for_owner(&pending.owner) else {
            return Err("NoDocumentLoaded".to_owned());
        };
        let contents = &mut slot.page_slot_mut().contents;
        let Some(document) = contents.main_frame.current_document.as_mut() else {
            return Err("NoDocumentLoaded".to_owned());
        };
        let engine = contents
            .navigation_engine
            .as_mut()
            .expect("resolved navigation engine");
        let completed = engine
            .complete_page_child_frame_lifecycle_work_best_effort(
                &mut document.page,
                pending.completion,
            )
            .map_err(|error| error.to_string())?;
        let _ = slot.ingest_owner_page_observable_output_updates();
        Ok(completed)
    }

    #[cfg(test)]
    pub(crate) fn complete_child_frame_lifecycle_work_for_session_owner(
        &mut self,
        pending: CompletedChildFrameLifecycleWork,
    ) -> Result<bool, String> {
        self.complete_child_frame_lifecycle_work_command_turn_for_session_owner(pending)
            .map(|(completed, _output)| completed)
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
        let Some(slot) = self.activity_source_runtime_slot_mut(session_id) else {
            trace_activity_source_stage(
                "conn_page_diagnostics_snapshot_missing_owner",
                session_id,
                trace_started,
            );
            return Ok(RendererPageDiagnosticsSnapshot::default());
        };
        let Some(page) = slot.loaded_page_mut() else {
            trace_activity_source_stage(
                "conn_page_diagnostics_snapshot_missing_page",
                session_id,
                trace_started,
            );
            return Ok(RendererPageDiagnosticsSnapshot::default());
        };
        let renderer_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let snapshot = page
            .page_diagnostics_snapshot_async()
            .await
            .map_err(|error| error.to_string())?;
        trace_activity_source_stage(
            "conn_page_diagnostics_snapshot_renderer_done",
            session_id,
            renderer_started,
        );
        let ingest_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let ingested = slot.ingest_owner_page_observable_output_updates();
        trace_activity_source_stage_with_bool(
            "conn_page_diagnostics_snapshot_ingest_done",
            session_id,
            ingest_started,
            ingested,
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
