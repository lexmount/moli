use std::pin::pin;

use crate::frame_owner_model::{
    ChildDynamicModuleCompletedFetchRestoreAction,
    ChildDynamicModuleOwnerFetchCompletionSettlementAction,
    ChildDynamicModuleOwnerFetchWithoutNetworkSettlementAction,
    FrameDocumentDynamicImportEvaluationReadyAction,
    FrameDocumentDynamicImportEvaluationReadyResult,
    FrameDocumentDynamicImportGraphAdvanceFollowup,
    FrameDocumentDynamicImportJoinedFetchRestoreResult,
    FrameDocumentDynamicImportMissingJoinedTerminalClient,
    FrameDocumentDynamicImportMissingJoinedTerminalFetch,
    FrameDocumentDynamicImportOwnerActionDiagnostic, FrameDocumentDynamicImportOwnerActionHooks,
    FrameDocumentDynamicImportOwnerActionQueueHooks,
    FrameDocumentDynamicImportOwnerActionQueueRequest,
    FrameDocumentDynamicImportOwnerActionQueueRunner,
    FrameDocumentDynamicImportOwnerActionQueueTrace,
    FrameDocumentDynamicImportOwnerFetchSettlementResult,
    FrameDocumentDynamicImportOwnerTerminalRestoreAction,
    FrameDocumentDynamicImportQueueTaskOwnerResult, FrameDocumentDynamicImportRejectAction,
    FrameDocumentDynamicImportRejectResult, FrameDocumentDynamicImportSourceReadyAction,
    FrameDocumentDynamicImportSourceReadyResult, FrameDocumentDynamicImportSourceWasmRecordLookup,
    FrameDocumentDynamicImportTerminalClientAction,
    FrameDocumentDynamicImportTerminalClientFinishResult,
    FrameDocumentDynamicImportTerminalPreparedAction,
    FrameDocumentDynamicImportWaitingFetchScheduleAction,
    FrameDocumentDynamicImportWaitingFetchScheduleResult, FrameDocumentModuleFetchClientStart,
    FrameDocumentModuleTerminalQueueFollowup, FrameDocumentOwner, FrameRealmId,
};
use crate::module_runtime::{
    DynamicModuleInflightFetch, ModuleGraphFetchedSource, ModuleLoadError, ModuleMapKey,
};

use super::*;

fn record_child_dynamic_import_terminal_owner_action_resumed(
    diagnostic: FrameDocumentDynamicImportOwnerActionDiagnostic,
) {
    match diagnostic {
        FrameDocumentDynamicImportOwnerActionDiagnostic::TerminalClient {
            owner,
            realm_id,
            tree_client,
            import_phase,
            url,
        } => {
            tracing::debug!(
                owner = ?owner,
                realm_id = ?realm_id,
                tree_id = tree_client.tree_id.0,
                tree_client_sequence = tree_client.sequence,
                import_phase = ?import_phase,
                url = %url,
                "child dynamic import single-module terminal client resumed native dynamic resolver"
            );
        }
        FrameDocumentDynamicImportOwnerActionDiagnostic::FetchCompletion {
            task_owner,
            realm_id,
            load_id,
        } => {
            tracing::debug!(
                owner = ?task_owner,
                realm_id = ?realm_id,
                load_id,
                "child dynamic import fetch completion owner action resumed native dynamic resolver"
            );
        }
        FrameDocumentDynamicImportOwnerActionDiagnostic::Continuation {
            task_owner,
            realm_id,
        } => {
            tracing::debug!(
                owner = ?task_owner,
                realm_id = ?realm_id,
                "child dynamic import continuation owner action resumed native dynamic resolver"
            );
        }
    }
}

pub(in crate::script_vm) fn record_child_dynamic_import_terminal_owner_action_failed(
    diagnostic: FrameDocumentDynamicImportOwnerActionDiagnostic,
    error: &str,
) {
    match diagnostic {
        FrameDocumentDynamicImportOwnerActionDiagnostic::TerminalClient {
            owner,
            realm_id,
            url,
            ..
        } => {
            tracing::warn!(
                owner = ?owner,
                realm_id = ?realm_id,
                url = %url,
                error,
                "child dynamic import single-module terminal client resume failed"
            );
        }
        FrameDocumentDynamicImportOwnerActionDiagnostic::FetchCompletion {
            task_owner,
            realm_id,
            load_id,
        } => {
            tracing::warn!(
                owner = ?task_owner,
                realm_id = ?realm_id,
                load_id,
                error,
                "child dynamic import fetch completion owner action failed"
            );
        }
        FrameDocumentDynamicImportOwnerActionDiagnostic::Continuation {
            task_owner,
            realm_id,
        } => {
            tracing::warn!(
                owner = ?task_owner,
                realm_id = ?realm_id,
                error,
                "child dynamic import continuation owner action failed"
            );
        }
    }
}

pub(in crate::script_vm) struct ScriptVmChildDynamicImportOwnerActionHooks<'vm> {
    vm: &'vm mut ScriptVm,
}

struct ScriptVmChildDynamicImportOwnerActionQueueHooks<'vm> {
    vm: &'vm mut ScriptVm,
}

impl<'vm> ScriptVmChildDynamicImportOwnerActionHooks<'vm> {
    pub(in crate::script_vm) fn new(vm: &'vm mut ScriptVm) -> Self {
        Self { vm }
    }
}

impl FrameDocumentDynamicImportOwnerActionQueueHooks
    for ScriptVmChildDynamicImportOwnerActionQueueHooks<'_>
{
    fn current_dynamic_import_task_owner(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
    ) -> FrameDocumentDynamicImportQueueTaskOwnerResult {
        self.vm
            ._context_host
            .borrow()
            .current_child_dynamic_import_task_owner(owner, realm_id)
            .map(FrameDocumentDynamicImportQueueTaskOwnerResult::Current)
            .unwrap_or(FrameDocumentDynamicImportQueueTaskOwnerResult::Stale)
    }

    fn queue_dynamic_import_owner_actions(
        &mut self,
        actions: Vec<FrameDocumentDynamicImportTerminalPreparedAction>,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        self.vm
            .route_child_dynamic_import_owner_actions_to_page_source(actions)
    }

    fn record_stale_dynamic_import_owner_action(
        &mut self,
        trace: FrameDocumentDynamicImportOwnerActionQueueTrace,
    ) {
        match trace {
            FrameDocumentDynamicImportOwnerActionQueueTrace::Continuation { owner, realm_id } => {
                self.vm.record_runtime_warning(format_args!(
                    "child dynamic import continuation for {:?}/{:?} could not queue owner action because the owner is no longer current",
                    owner, realm_id
                ));
            }
            FrameDocumentDynamicImportOwnerActionQueueTrace::FetchCompletion {
                owner,
                realm_id,
                load_id,
            } => {
                self.vm.record_runtime_warning(format_args!(
                    "child dynamic import fetch completion {load_id} for {:?}/{:?} could not queue owner action because the owner is no longer current",
                    owner, realm_id
                ));
            }
            FrameDocumentDynamicImportOwnerActionQueueTrace::Waiting {
                owner,
                realm_id,
                fetch_count,
            } => {
                self.vm.record_runtime_warning(format_args!(
                    "child dynamic import continuation for {:?}/{:?} could not queue waiting owner action with {fetch_count} fetches because the owner is no longer current",
                    owner, realm_id
                ));
            }
        }
    }
}

impl FrameDocumentDynamicImportOwnerActionHooks for ScriptVmChildDynamicImportOwnerActionHooks<'_> {
    fn finish_terminal_client(
        &mut self,
        action: FrameDocumentDynamicImportTerminalClientAction,
    ) -> std::result::Result<FrameDocumentDynamicImportTerminalClientFinishResult, String> {
        let (task_owner, realm_id, key, client) = action.into_parts();
        Ok(self.vm.finish_child_dynamic_import_terminal_client(
            task_owner.document_owner(),
            realm_id,
            &key,
            client,
        ))
    }

    fn queue_owner_action_followups(
        &mut self,
        actions: Vec<FrameDocumentDynamicImportTerminalPreparedAction>,
    ) -> std::result::Result<FrameDocumentModuleTerminalQueueFollowup, String> {
        Ok(self
            .vm
            .route_child_dynamic_import_owner_actions_to_page_source(actions))
    }

    fn record_missing_joined_terminal_client(
        &mut self,
        missing: FrameDocumentDynamicImportMissingJoinedTerminalClient,
    ) -> std::result::Result<(), String> {
        let owner = missing.owner();
        let realm_id = missing.realm_id();
        let tree_client = missing.tree_client();
        self.vm.record_runtime_warning(format_args!(
            "child dynamic import single-module terminal client {:?} for {:?}/{:?} had no dynamic import continuation",
            tree_client, owner, realm_id
        ));
        Ok(())
    }

    fn settle_owner_module_fetch_completion(
        &mut self,
        action: ChildDynamicModuleOwnerFetchCompletionSettlementAction,
    ) -> std::result::Result<FrameDocumentDynamicImportOwnerFetchSettlementResult, String> {
        let (start, source) = action.into_parts();
        Ok(
            FrameDocumentDynamicImportOwnerFetchSettlementResult::from_settled(
                self.vm
                    .settle_child_dynamic_import_owner_module_fetch(&start, &source),
            ),
        )
    }

    fn restore_completed_owner_module_fetch_as_joined_terminal_client(
        &mut self,
        restore: ChildDynamicModuleCompletedFetchRestoreAction,
    ) -> std::result::Result<FrameDocumentDynamicImportJoinedFetchRestoreResult, String> {
        let (owner, realm_id, inflight) = restore.into_parts();
        Ok(
            FrameDocumentDynamicImportJoinedFetchRestoreResult::from_restored(
                self.vm
                    .child_document_modulator_store
                    .restore_dynamic_module_import_fetch_as_joined_owner_client(
                        owner, realm_id, inflight,
                    )
                    .is_some(),
            ),
        )
    }

    fn finish_owner_module_fetch_without_network(
        &mut self,
        action: ChildDynamicModuleOwnerFetchWithoutNetworkSettlementAction,
    ) -> std::result::Result<FrameDocumentDynamicImportOwnerFetchSettlementResult, String> {
        let start = action.into_start();
        Ok(
            FrameDocumentDynamicImportOwnerFetchSettlementResult::from_settled(
                self.vm
                    ._context_host
                    .borrow_mut()
                    .finish_child_owner_module_fetch_without_network(&start),
            ),
        )
    }

    fn restore_scheduled_fetch_as_joined_terminal_client(
        &mut self,
        action: FrameDocumentDynamicImportOwnerTerminalRestoreAction,
    ) -> std::result::Result<FrameDocumentDynamicImportJoinedFetchRestoreResult, String> {
        let (owner, realm_id, restore) = action.into_parts();
        let load_id = restore.load_id();
        Ok(
            FrameDocumentDynamicImportJoinedFetchRestoreResult::from_restored(
                self.vm
                    .child_document_modulator_store
                    .restore_inflight_dynamic_module_import_fetch_as_joined_owner_client(
                        owner, realm_id, load_id,
                    )
                    .is_some(),
            ),
        )
    }

    fn schedule_waiting_fetch(
        &mut self,
        action: FrameDocumentDynamicImportWaitingFetchScheduleAction,
    ) -> std::result::Result<FrameDocumentDynamicImportWaitingFetchScheduleResult, String> {
        let (owner, realm_id, fetch) = action.into_parts();
        let (load_id, request) = fetch.into_parts();
        let request_url = request.source_url().clone();
        let producer = {
            let context_host = self.vm._context_host.borrow();
            context_host.capture_child_dynamic_import_fetch_producer(owner, realm_id, request_url)
        };
        let Some((target, network_attribution)) = producer else {
            self.vm.record_runtime_warning(format_args!(
                "child dynamic import waiting fetch {load_id} for {owner:?}/{realm_id:?} lost its exact producer before native fetch"
            ));
            return Ok(FrameDocumentDynamicImportWaitingFetchScheduleResult::StaleOwner);
        };
        let Some(loader) = self
            .vm
            ._context_host
            .borrow()
            .document_resource_loader_for_owner(target.task_owner())
        else {
            self.vm.record_runtime_warning(format_args!(
                "child dynamic import waiting fetch action had no committed Document authority"
            ));
            return Ok(FrameDocumentDynamicImportWaitingFetchScheduleResult::MissingLoader);
        };
        self.vm
            .resource_scheduler()
            .schedule_child_dynamic_module_graph_fetch(
                loader,
                target,
                load_id,
                request,
                network_attribution,
            );
        Ok(FrameDocumentDynamicImportWaitingFetchScheduleResult::Scheduled)
    }

    fn record_missing_joined_terminal_fetch(
        &mut self,
        missing: FrameDocumentDynamicImportMissingJoinedTerminalFetch,
    ) -> std::result::Result<(), String> {
        let owner = missing.owner();
        let realm_id = missing.realm_id();
        let load_id = missing.load_id();
        self.vm.record_runtime_warning(format_args!(
            "child dynamic import owner module fetch {load_id} for {:?}/{:?} could not be restored as a joined terminal client",
            owner, realm_id
        ));
        Ok(())
    }

    fn resolve_ready_source_import(
        &mut self,
        action: FrameDocumentDynamicImportSourceReadyAction,
    ) -> std::result::Result<FrameDocumentDynamicImportSourceReadyResult, String> {
        let (request, root_entry) = action.into_parts();
        let document_owner = request.owner();
        if !self
            .vm
            .dynamic_module_import_owner_is_current(document_owner)
        {
            self.vm.record_runtime_warning(format_args!(
                "dropped stale child dynamic import source resolution: owner={document_owner:?}"
            ));
            return Ok(FrameDocumentDynamicImportSourceReadyResult::DroppedStaleOwner);
        }
        let (_child_handle, task_owner, realm_id) =
            document_owner.child_parts().ok_or_else(|| {
                "child dynamic import source-ready action had a main-document owner".to_owned()
            })?;
        match self
            .vm
            .resolve_child_native_dynamic_module_source_import(
                task_owner.document_owner(),
                realm_id,
                request,
                root_entry,
            )
            .map_err(|error| error.message().to_owned())?
        {
            NativeDynamicModuleSourceImportResolution::Resolved => {
                Ok(FrameDocumentDynamicImportSourceReadyResult::Resolved)
            }
            NativeDynamicModuleSourceImportResolution::Rejected => {
                Ok(FrameDocumentDynamicImportSourceReadyResult::Rejected)
            }
        }
    }

    fn continue_ready_evaluation_import(
        &mut self,
        action: FrameDocumentDynamicImportEvaluationReadyAction,
    ) -> std::result::Result<FrameDocumentDynamicImportEvaluationReadyResult, String> {
        let (request, graph) = action.into_parts();
        let document_owner = request.owner();
        if !self
            .vm
            .dynamic_module_import_owner_is_current(document_owner)
        {
            self.vm.record_runtime_warning(format_args!(
                "dropped stale child dynamic import before evaluation: owner={document_owner:?}"
            ));
            return Ok(FrameDocumentDynamicImportEvaluationReadyResult::DroppedStaleOwner);
        }
        let (_child_handle, task_owner, realm_id) =
            document_owner.child_parts().ok_or_else(|| {
                "child dynamic import evaluation-ready action had a main-document owner".to_owned()
            })?;
        let evaluation = self.vm.start_child_native_dynamic_module_import_evaluation(
            task_owner.document_owner(),
            realm_id,
            graph,
        );
        if !self
            .vm
            .dynamic_module_import_owner_is_current(document_owner)
        {
            self.vm.record_runtime_warning(format_args!(
                "dropped child dynamic import settlement after evaluation replaced its document: owner={document_owner:?}"
            ));
            return Ok(FrameDocumentDynamicImportEvaluationReadyResult::DroppedStaleOwner);
        }
        match evaluation {
            Ok(DynamicModuleImportEvaluationStart::Completed(target)) => {
                self.vm
                    .resolve_native_dynamic_module_import_selected_task_body(request, &target)
                    .map_err(|error| error.message().to_owned())?;
                Ok(FrameDocumentDynamicImportEvaluationReadyResult::Resolved)
            }
            Ok(DynamicModuleImportEvaluationStart::Pending { target, promise }) => {
                self.vm
                    .attach_native_dynamic_module_import_reactions(request, target, promise)
                    .map_err(|error| error.message().to_owned())?;
                Ok(FrameDocumentDynamicImportEvaluationReadyResult::Pending)
            }
            Err(error) => {
                self.vm
                    .reject_native_dynamic_module_import_with_error_selected_task_body(
                        request, &error,
                    )
                    .map_err(|error| error.message().to_owned())?;
                Ok(FrameDocumentDynamicImportEvaluationReadyResult::Rejected)
            }
        }
    }

    fn record_restored_after_unexpected_complete(
        &mut self,
        _diagnostic: FrameDocumentDynamicImportOwnerActionDiagnostic,
    ) -> std::result::Result<(), String> {
        self.vm.record_runtime_warning(format_args!(
            "child native dynamic import tree completed while its pending tree still had clients"
        ));
        Ok(())
    }

    fn reject_dynamic_import(
        &mut self,
        action: FrameDocumentDynamicImportRejectAction,
    ) -> std::result::Result<FrameDocumentDynamicImportRejectResult, String> {
        let (request, error) = action.into_parts();
        let document_owner = request.owner();
        if !self
            .vm
            .dynamic_module_import_owner_is_current(document_owner)
        {
            self.vm.record_runtime_warning(format_args!(
                "dropped stale child dynamic import rejection: owner={document_owner:?}"
            ));
            return Ok(FrameDocumentDynamicImportRejectResult::DroppedStaleOwner);
        }
        self.vm
            .reject_native_dynamic_module_import_with_error_selected_task_body(request, &error)
            .map_err(|error| error.message().to_owned())?;
        Ok(FrameDocumentDynamicImportRejectResult::Rejected)
    }

    fn record_action_resumed(
        &mut self,
        diagnostic: FrameDocumentDynamicImportOwnerActionDiagnostic,
    ) {
        record_child_dynamic_import_terminal_owner_action_resumed(diagnostic);
    }

    fn record_action_failed(
        &mut self,
        diagnostic: FrameDocumentDynamicImportOwnerActionDiagnostic,
        error: &str,
    ) {
        record_child_dynamic_import_terminal_owner_action_failed(diagnostic, error);
    }
}

impl ScriptVm {
    pub(crate) fn current_child_dynamic_import_task_owner(
        &self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
    ) -> Option<crate::frame_owner_model::FrameDocumentTaskOwner> {
        self._context_host
            .borrow()
            .current_child_dynamic_import_task_owner(owner, realm_id)
    }

    /// Executes one action only after the Page arbiter has authorized its
    /// exact root-Document and child Document/realm target.
    pub(crate) fn apply_current_child_dynamic_import_owner_action(
        &mut self,
        authorization: crate::runtime::AuthorizedCurrentChildDynamicImportOwnerAction,
    ) -> FrameDocumentDynamicImportTerminalOutcome {
        FrameDocumentDynamicImportOwnerActionRunner::new(
            ScriptVmChildDynamicImportOwnerActionHooks::new(self),
        )
        .run_prepared_action(authorization.into_action())
    }

    fn queue_child_dynamic_import_owner_action_request(
        &mut self,
        request: FrameDocumentDynamicImportOwnerActionQueueRequest,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        FrameDocumentDynamicImportOwnerActionQueueRunner::new(
            ScriptVmChildDynamicImportOwnerActionQueueHooks { vm: self },
        )
        .run_queue_request(request)
        .into_followup()
    }

    /// Applies a child dynamic-import fetch terminal only after the Page
    /// owner has proved its complete root-Document and child/document/realm
    /// target current.
    pub(crate) fn apply_current_child_dynamic_import_fetch_completion(
        &mut self,
        authorization: crate::runtime::AuthorizedCurrentChildModuleFetchCompletion<
            ChildDynamicImportFetchCompletion,
        >,
    ) -> Result<FrameDocumentModuleTerminalQueueFollowup> {
        let (target, load_id, result) = authorization.into_completion().into_terminal_parts();
        self.finish_child_dynamic_module_graph_fetch_completion(
            target.task_owner().document_owner(),
            target.realm_id(),
            load_id,
            result,
        )
    }

    fn finish_child_dynamic_module_graph_fetch_completion(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        load_id: u64,
        result: std::result::Result<ModuleGraphFetchedSource, String>,
    ) -> Result<FrameDocumentModuleTerminalQueueFollowup> {
        let Some(child_inflight) =
            self.take_child_dynamic_module_import_fetch(owner, realm_id, load_id)
        else {
            self.record_runtime_warning(format_args!(
                "child dynamic import fetch completion {load_id} for {:?}/{:?} had no current in-flight owner",
                owner, realm_id
            ));
            return Ok(FrameDocumentModuleTerminalQueueFollowup::terminal_warning_recorded());
        };
        let source = match result {
            Ok(fetched_source) => self.module_graph_fetched_source_or_csp_error(
                load_id,
                fetched_source,
                child_inflight.inflight.fetch_metadata(),
            ),
            Err(error) => Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                format!("native dynamic import fetch completion {load_id} failed: {error}"),
            )),
        };
        let inflight = child_inflight.inflight;
        if let Some(owner_start) = inflight.owner_module_fetch_start().cloned()
            && inflight.tree_client().is_some()
        {
            return Ok(self.enqueue_dynamic_module_owner_fetch_completion_action(
                owner,
                realm_id,
                load_id,
                owner_start,
                source,
                inflight,
            ));
        }
        let finish = self
            .finish_child_native_dynamic_module_inflight_fetch(owner, realm_id, inflight, source);
        Ok(self.enqueue_dynamic_module_fetch_finish_owner_action(owner, realm_id, load_id, finish))
    }

    pub(super) fn settle_child_dynamic_import_owner_module_fetch(
        &mut self,
        owner_start: &FrameDocumentModuleFetchClientStart,
        source: &std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> bool {
        let result = (*source)
            .clone()
            .map_err(|error| error.message().to_owned());
        let settled = self
            ._context_host
            .borrow_mut()
            .settle_current_child_dynamic_import_owner_module_fetch(owner_start, result);
        if !settled {
            self.record_runtime_warning(format_args!(
                "child dynamic import owner module fetch completion did not match the current frame document owner"
            ));
        }
        settled
    }

    pub(super) fn advance_child_native_dynamic_module_import_job(
        &mut self,
        mut job: NativeModuleGraphJob,
    ) -> std::result::Result<(), String> {
        let needs_graph_start = job.needs_dynamic_import_graph_start();
        let request = job
            .dynamic_import_request()
            .expect("child dynamic module graph job must retain its import request");
        let (_child_handle, task_owner, realm_id) =
            request.owner().child_parts().ok_or_else(|| {
                "child dynamic module graph job carried a main-document owner".to_owned()
            })?;
        let document_owner = task_owner.document_owner();
        let fallback_url = request.base_url().clone();
        if needs_graph_start {
            self.ensure_child_document_modulator_for_graph_start(document_owner, realm_id);
        }
        let advance = match self.with_current_child_module_tree_owner_or_module_load_error(
            document_owner,
            realm_id,
            &fallback_url,
            |owner| job.advance_dynamic_import_owner_lane_with_owner(owner),
        ) {
            Ok(advance) => advance,
            Err(error) => {
                return self
                    .enqueue_child_dynamic_import_graph_advance_failed_owner_action(job, error)
                    .map(|_| ());
            }
        };
        self.continue_child_native_dynamic_module_import_after_tree_advance(job, advance)
    }

    pub(super) fn continue_child_native_dynamic_module_import_after_tree_advance(
        &mut self,
        job: NativeModuleGraphJob,
        advance: NativeModuleGraphJobAdvance,
    ) -> std::result::Result<(), String> {
        let request = job
            .dynamic_import_request()
            .expect("child dynamic module graph continuation must retain its import request");
        let (_child_handle, task_owner, realm_id) =
            request.owner().child_parts().ok_or_else(|| {
                "child dynamic module graph continuation carried a main-document owner".to_owned()
            })?;
        let followup = self
            .child_document_modulator_store
            .dynamic_import_graph_advance_followup(
                task_owner.document_owner(),
                realm_id,
                job,
                advance,
            );
        self.apply_child_dynamic_import_followup(followup);
        Ok(())
    }

    pub(crate) fn apply_child_dynamic_import_followup(
        &mut self,
        followup: FrameDocumentDynamicImportGraphAdvanceFollowup,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        match followup {
            FrameDocumentDynamicImportGraphAdvanceFollowup::QueueOwnerAction(request) => {
                self.queue_child_dynamic_import_owner_action_request(*request)
            }
            FrameDocumentDynamicImportGraphAdvanceFollowup::ResumePendingJob(resume) => {
                self.document_runtime
                    .resume_native_dynamic_module_import_front(resume.into_job());
                FrameDocumentModuleTerminalQueueFollowup::dynamic_import_job_resumed()
            }
            FrameDocumentDynamicImportGraphAdvanceFollowup::RecordMissingJoinedTerminalFetch(
                missing,
            ) => {
                let owner = missing.owner();
                let realm_id = missing.realm_id();
                let load_id = missing.load_id();
                self.record_runtime_warning(format_args!(
                    "child dynamic import fetch finish {load_id} for {:?}/{:?} had no joined terminal client",
                    owner, realm_id
                ));
                FrameDocumentModuleTerminalQueueFollowup::terminal_warning_recorded()
            }
            FrameDocumentDynamicImportGraphAdvanceFollowup::RecordUnexpectedCompleteWarning(
                warning,
            ) => {
                let (owner, realm_id) = warning.into_parts();
                self.record_runtime_warning(format_args!(
                    "child native dynamic import tree completed while its pending tree still had clients for {:?}/{:?}",
                    owner, realm_id
                ));
                FrameDocumentModuleTerminalQueueFollowup::terminal_warning_from_recorded(true)
            }
            FrameDocumentDynamicImportGraphAdvanceFollowup::WaitRetained => {
                FrameDocumentModuleTerminalQueueFollowup::dynamic_import_wait_retained()
            }
        }
    }

    pub(super) fn enqueue_child_dynamic_import_graph_advance_failed_owner_action(
        &mut self,
        job: NativeModuleGraphJob,
        error: ModuleLoadError,
    ) -> std::result::Result<FrameDocumentModuleTerminalQueueFollowup, String> {
        let request = job
            .dynamic_import_request()
            .expect("failed child dynamic module graph job must retain its import request");
        let (_child_handle, task_owner, realm_id) =
            request.owner().child_parts().ok_or_else(|| {
                "failed child dynamic module graph job carried a main-document owner".to_owned()
            })?;
        let followup = self
            .child_document_modulator_store
            .dynamic_import_graph_advance_failure_followup(
                task_owner.document_owner(),
                realm_id,
                job,
                error,
            );
        Ok(self.apply_child_dynamic_import_followup(followup))
    }

    pub(super) fn enqueue_dynamic_module_fetch_finish_owner_action(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        load_id: u64,
        finish: DynamicModuleFetchFinish,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let followup = self
            .child_document_modulator_store
            .dynamic_import_fetch_finish_followup(owner, realm_id, load_id, finish);
        self.apply_child_dynamic_import_followup(followup)
    }

    fn enqueue_dynamic_module_owner_fetch_completion_action(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        load_id: u64,
        owner_start: FrameDocumentModuleFetchClientStart,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
        inflight: DynamicModuleInflightFetch,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let followup = self
            .child_document_modulator_store
            .dynamic_import_owner_module_fetch_completion_followup(
                owner,
                realm_id,
                load_id,
                owner_start,
                source,
                inflight,
            );
        self.apply_child_dynamic_import_followup(followup)
    }

    pub(super) fn finish_native_dynamic_module_inflight_fetch(
        &mut self,
        inflight: DynamicModuleInflightFetch,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> DynamicModuleFetchFinish {
        let Some((_child_handle, task_owner, realm_id)) = inflight.owner().child_parts() else {
            return inflight.finish_for_owner(self, source);
        };
        let document_owner = task_owner.document_owner();
        let module_request_initiator_url = self.child_module_request_initiator_url_for_owner(
            document_owner,
            realm_id,
            inflight.import_base_url(),
        );
        self.finish_child_dynamic_module_inflight_fetch_with_modulator(
            document_owner,
            realm_id,
            module_request_initiator_url,
            inflight,
            source,
            "child dynamic import fetch has no current document modulator",
        )
    }

    pub(super) fn finish_child_native_dynamic_module_inflight_fetch(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        inflight: DynamicModuleInflightFetch,
        source: std::result::Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> DynamicModuleFetchFinish {
        let module_request_initiator_url = self.child_module_request_initiator_url_for_owner(
            owner,
            realm_id,
            inflight.import_base_url(),
        );
        self.finish_child_dynamic_module_inflight_fetch_with_modulator(
            owner,
            realm_id,
            module_request_initiator_url,
            inflight,
            source,
            "child dynamic import fetch has no current document modulator",
        )
    }

    pub(super) fn finish_native_dynamic_module_joined_fetch(
        &mut self,
        joined: DynamicModuleJoinedFetch,
        key: &ModuleMapKey,
    ) -> DynamicModuleFetchFinish {
        let Some((_child_handle, task_owner, realm_id)) = joined.owner().child_parts() else {
            return joined.finish_for_owner(self, key);
        };
        let document_owner = task_owner.document_owner();
        let module_request_initiator_url = self.child_module_request_initiator_url_for_owner(
            document_owner,
            realm_id,
            joined.import_base_url(),
        );
        self.finish_child_dynamic_module_joined_fetch_with_modulator(
            document_owner,
            realm_id,
            module_request_initiator_url,
            joined,
            key,
            "child dynamic import joined fetch has no current document modulator",
        )
    }

    pub(super) fn instantiate_native_module_graph_with_modulator_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        document_modulator: &mut NativeDocumentModulator,
        graph: &crate::module_runtime::ModuleGraphHandle,
    ) -> std::result::Result<(), ModuleLoadError> {
        let root_entry = graph.root_entry;
        let graph_urls = graph
            .entries
            .iter()
            .map(|entry_id| document_modulator.entry_url(*entry_id))
            .collect::<Vec<_>>();
        let has_wasm_entry = graph
            .entries
            .iter()
            .any(|entry_id| document_modulator.module_wasm_record(*entry_id).is_some());
        let document_modulator_ptr = document_modulator as *const NativeDocumentModulator;
        let root_module = document_modulator
            .compiled_record(root_entry)
            .map(|record| record.compiled_module().clone())
            .ok_or_else(|| {
                ModuleLoadError::new(
                    ModuleLoadStage::Instantiate,
                    format!("native root module entry {root_entry:?} is not compiled"),
                )
            })?;
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let try_catch = pin!(v8::TryCatch::new(scope));
                let scope = try_catch.init();

                let root_module = v8::Local::new(&scope, &root_module);
                let _resolver_scope = ResolverScopeGuard::new(document_modulator_ptr);
                match root_module.instantiate_module2(
                    &scope,
                    resolve_static_module_callback,
                    resolve_static_source_callback,
                ) {
                    Some(true) => Ok(()),
                    Some(false) => Err(anyhow::anyhow!("v8 reported module instantiate failure")),
                    None => {
                        let exception = scope
                            .exception()
                            .and_then(|exception| exception.to_string(&scope))
                            .map(|message| message.to_rust_string_lossy(&scope))
                            .unwrap_or_else(|| "unknown instantiate exception".to_owned());
                        Err(anyhow::anyhow!(
                            "{}",
                            canonical_native_module_instantiate_error(&exception, &graph_urls)
                        ))
                    }
                }
            })
            .map_err(|error| {
                let message = error.to_string();
                let load_error =
                    ModuleLoadError::new(ModuleLoadStage::Instantiate, message.clone());
                if message.contains("does not provide an export named")
                    || message.contains("does not export")
                {
                    load_error.with_error_constructor(ScriptErrorConstructorKind::SyntaxError)
                } else if has_wasm_entry {
                    load_error
                        .with_error_constructor(ScriptErrorConstructorKind::WebAssemblyLinkError)
                } else {
                    load_error
                }
            })?;
        document_modulator.mark_instantiated(root_entry);
        Ok(())
    }

    fn evaluate_native_dynamic_module_graph_with_modulator(
        &mut self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        document_modulator: &mut NativeDocumentModulator,
        root_entry: crate::module_runtime::ModuleEntryId,
    ) -> std::result::Result<NativeDynamicModuleEvaluation, ModuleLoadError> {
        let context_ptr = self.frame_realm_context_ptr(realm_id).map_err(|error| {
            ModuleLoadError::new(
                ModuleLoadStage::Evaluate,
                format!("failed to find FrameRealm {realm_id:?} for module evaluation: {error}"),
            )
        })?;
        self.evaluate_native_module_graph_with_modulator_in_context(
            context_ptr,
            document_owner,
            realm_id,
            document_modulator,
            root_entry,
            NativeModuleEvaluationOwner::DynamicImport,
        )
        .map(|result| NativeDynamicModuleEvaluation {
            target: DynamicModuleEvaluationTarget::new(root_entry, result.module),
            promise: result.promise,
        })
    }

    fn start_child_native_dynamic_module_import_evaluation(
        &mut self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        graph: ModuleGraphHandle,
    ) -> std::result::Result<DynamicModuleImportEvaluationStart, ModuleLoadError> {
        self.with_current_child_document_modulator_or_module_load_error(
            document_owner,
            realm_id,
            |vm, document_modulator| {
                match document_modulator.entry(graph.root_entry).state() {
                    ModuleMapEntryState::Compiled => {
                        let context_ptr = vm.frame_realm_context_ptr(realm_id).map_err(|error| {
                            ModuleLoadError::new(
                                ModuleLoadStage::Instantiate,
                                format!(
                                    "failed to find FrameRealm {realm_id:?} for module instantiate: {error}"
                                ),
                            )
                        })?;
                        vm.instantiate_native_module_graph_with_modulator_in_context(
                            context_ptr,
                            document_modulator,
                            &graph,
                        )?;
                        vm.start_child_native_module_graph_evaluation(
                            document_owner,
                            realm_id,
                            document_modulator,
                            graph.root_entry,
                        )
                    }
                    ModuleMapEntryState::Instantiated | ModuleMapEntryState::Evaluating => vm
                        .start_child_native_module_graph_evaluation(
                            document_owner,
                            realm_id,
                            document_modulator,
                            graph.root_entry,
                        ),
                    ModuleMapEntryState::Evaluated => {
                        let module = document_modulator
                            .compiled_record(graph.root_entry)
                            .map(|record| record.compiled_module().clone())
                            .ok_or_else(|| {
                                ModuleLoadError::new(
                                    ModuleLoadStage::Evaluate,
                                    "native dynamic import root was evaluated without a compiled module",
                                )
                            })?;
                        Ok(DynamicModuleImportEvaluationStart::Completed(
                            DynamicModuleEvaluationTarget::new(graph.root_entry, module),
                        ))
                    }
                    ModuleMapEntryState::Fetching
                    | ModuleMapEntryState::Fetched
                    | ModuleMapEntryState::Failed => Err(ModuleLoadError::new(
                        ModuleLoadStage::Evaluate,
                        "native dynamic import root was not ready to evaluate",
                    )),
                }
            },
        )
    }

    fn start_child_native_module_graph_evaluation(
        &mut self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        document_modulator: &mut NativeDocumentModulator,
        root_entry: crate::module_runtime::ModuleEntryId,
    ) -> std::result::Result<DynamicModuleImportEvaluationStart, ModuleLoadError> {
        let evaluation = self.evaluate_native_dynamic_module_graph_with_modulator(
            document_owner,
            realm_id,
            document_modulator,
            root_entry,
        )?;
        let (target, promise) = evaluation.into_parts();
        let Some(promise) = promise else {
            return Ok(DynamicModuleImportEvaluationStart::Completed(target));
        };
        Ok(DynamicModuleImportEvaluationStart::Pending { target, promise })
    }

    pub(super) fn evaluate_native_module_graph_with_modulator_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        document_modulator: &mut NativeDocumentModulator,
        root_entry: crate::module_runtime::ModuleEntryId,
        owner: NativeModuleEvaluationOwner,
    ) -> std::result::Result<NativeModuleEvaluationResult, ModuleLoadError> {
        let root_module = document_modulator
            .compiled_record(root_entry)
            .map(|record| record.compiled_module().clone())
            .ok_or_else(|| {
                ModuleLoadError::new(
                    ModuleLoadStage::Evaluate,
                    format!("native root module entry {root_entry:?} is not compiled"),
                )
            })?;
        document_modulator.mark_evaluating(root_entry);
        let promise = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let try_catch = pin!(v8::TryCatch::new(scope));
                let mut scope = try_catch.init();

                let root_module = v8::Local::new(&scope, &root_module);
                let Some(value) = root_module.evaluate(&scope) else {
                    let error = scope
                        .exception()
                        .map(|exception| {
                            native_module_evaluation_exception_error(
                                &mut scope,
                                exception,
                                "v8 failed to evaluate native module graph",
                            )
                        })
                        .unwrap_or_else(|| {
                            ModuleLoadError::new(
                                ModuleLoadStage::Evaluate,
                                "v8 failed to evaluate native module graph: unknown exception",
                            )
                        });
                    return Ok(Err(error));
                };
                let promise = v8::Local::<v8::Promise>::try_from(value).ok();
                if owner == NativeModuleEvaluationOwner::DynamicImport
                    && let Some(promise) = promise
                {
                    match promise.state() {
                        v8::PromiseState::Fulfilled => return Ok(Ok(None)),
                        v8::PromiseState::Rejected | v8::PromiseState::Pending => {
                            let promise = v8::Global::new(scope.as_ref(), promise);
                            return Ok(Ok(Some(promise)));
                        }
                    }
                }
                if let Err(error) = Self::perform_microtask_checkpoints(&mut scope, None) {
                    return Ok(Err(ModuleLoadError::new(
                        ModuleLoadStage::Evaluate,
                        error.to_string(),
                    )));
                }
                if root_module.get_status() == v8::ModuleStatus::Errored {
                    let exception = root_module.get_exception();
                    return Ok(Err(native_module_evaluation_exception_error(
                        &mut scope,
                        exception,
                        "native module graph evaluation rejected",
                    )));
                }
                if let Some(promise) = promise {
                    match promise.state() {
                        v8::PromiseState::Fulfilled => return Ok(Ok(None)),
                        v8::PromiseState::Rejected => {
                            let result = promise.result(&scope);
                            return Ok(Err(native_module_evaluation_exception_error(
                                &mut scope,
                                result,
                                "native module graph evaluation rejected",
                            )));
                        }
                        v8::PromiseState::Pending => {
                            let promise = v8::Global::new(scope.as_ref(), promise);
                            return Ok(Ok(Some(promise)));
                        }
                    }
                }
                Ok(Ok(None))
            })
            .map_err(|error| {
                ModuleLoadError::new(ModuleLoadStage::Evaluate, error.to_string())
            })??;
        let document_owner_is_current = self
            ._context_host
            .borrow()
            .current_child_module_route_task_owner(document_owner, realm_id)
            .is_some();
        if promise.is_none() && document_owner_is_current {
            document_modulator.mark_evaluated(root_entry);
        }
        Ok(NativeModuleEvaluationResult {
            module: root_module,
            promise,
        })
    }

    fn resolve_child_native_dynamic_module_source_import(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        request: PendingDynamicModuleImport,
        root_entry: crate::module_runtime::ModuleEntryId,
    ) -> std::result::Result<NativeDynamicModuleSourceImportResolution, ModuleLoadError> {
        let wasm_record = match self
            .child_document_modulator_store
            .dynamic_module_source_wasm_record_lookup(
                owner,
                realm_id,
                root_entry,
                request.specifier(),
            ) {
            FrameDocumentDynamicImportSourceWasmRecordLookup::Found(wasm_record) => wasm_record,
            FrameDocumentDynamicImportSourceWasmRecordLookup::MissingDocumentModulator(error) => {
                return Err(error);
            }
            FrameDocumentDynamicImportSourceWasmRecordLookup::NotWasm(error) => {
                self.reject_native_dynamic_module_import_with_error_selected_task_body(
                    request, &error,
                )?;
                return Ok(NativeDynamicModuleSourceImportResolution::Rejected);
            }
        };
        self.resolve_native_dynamic_module_source_import_with_wasm_record_selected_task_body(
            request,
            wasm_record,
        )
    }

    pub(super) fn mark_child_native_dynamic_module_evaluated(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        root_entry: crate::module_runtime::ModuleEntryId,
    ) {
        if !self
            .child_document_modulator_store
            .mark_dynamic_module_graph_evaluated(owner, realm_id, root_entry)
        {
            self.record_runtime_warning(format_args!(
                "failed to mark child dynamic module evaluated: owner {:?} realm {:?} had no current document modulator",
                owner, realm_id
            ));
        }
    }
}
