use anyhow::Result;

use crate::{
    document_runtime::DomHandle,
    document_script_scheduler::{
        DocumentModuleGraphFailedWork, DocumentModuleGraphReadyWork,
        DocumentScriptExecutionOutcome, FrameModuleScriptDocumentScriptHooks,
        FrameModuleScriptEvaluationStart,
    },
    frame_owner_model::{
        FrameDocumentScriptElementEventKind, FrameDocumentTaskOwner, FrameRealmId,
    },
    module_runtime::{ModuleEntryId, ModuleLoadError},
    types::{ScriptErrorValue, ScriptMode},
};

use super::super::super::{ScriptVm, child_document_event::ChildDocumentEventOwner};

pub(in crate::script_vm::native_module) struct ChildModuleScriptExecutionOwner<'vm> {
    vm: &'vm mut ScriptVm,
}

impl<'vm> ChildModuleScriptExecutionOwner<'vm> {
    pub(in crate::script_vm::native_module) fn new(vm: &'vm mut ScriptVm) -> Self {
        Self { vm }
    }
}

impl FrameModuleScriptDocumentScriptHooks for ChildModuleScriptExecutionOwner<'_> {
    type GraphReadyWork = DocumentModuleGraphReadyWork;
    type GraphFailureWork = DocumentModuleGraphFailedWork;
    type Output<'owner>
        = DocumentScriptExecutionOutcome
    where
        Self: 'owner;

    fn check_current_graph_ready_work(
        &mut self,
        work: &Self::GraphReadyWork,
    ) -> std::result::Result<(), DocumentScriptExecutionOutcome> {
        self.check_current_module_work(
            work.owner(),
            work.realm_id(),
            work.script().mode,
            work.pending_script_id().key(),
            work.load_delay_token(),
        )
    }

    fn check_current_graph_failure_work(
        &mut self,
        work: &Self::GraphFailureWork,
    ) -> std::result::Result<(), DocumentScriptExecutionOutcome> {
        self.check_current_module_work(
            work.owner(),
            work.realm_id(),
            work.script().mode,
            work.pending_script_id().key(),
            work.load_delay_token(),
        )
    }

    fn check_current_evaluation_work(
        &mut self,
        work: &Self::GraphReadyWork,
    ) -> std::result::Result<(), DocumentScriptExecutionOutcome> {
        self.check_current_module_work(
            work.owner(),
            work.realm_id(),
            work.script().mode,
            work.pending_script_id().key(),
            work.load_delay_token(),
        )
    }

    fn output_from_execution_outcome<'owner>(
        &'owner mut self,
        outcome: DocumentScriptExecutionOutcome,
    ) -> Self::Output<'owner> {
        outcome
    }

    fn start_graph_evaluation(
        &mut self,
        work: &DocumentModuleGraphReadyWork,
    ) -> std::result::Result<FrameModuleScriptEvaluationStart, ModuleLoadError> {
        self.vm.start_child_parser_module_graph_evaluation(work)
    }

    fn mark_graph_evaluated(
        &mut self,
        work: &DocumentModuleGraphReadyWork,
        root_entry: ModuleEntryId,
    ) -> std::result::Result<(), DocumentScriptExecutionOutcome> {
        if self.vm.mark_child_parser_module_graph_evaluated(
            work.owner(),
            work.realm_id(),
            root_entry,
        ) {
            Ok(())
        } else {
            let order_released = self.complete_parser_deferred_module_script(
                work.owner(),
                work.realm_id(),
                work.script().mode,
                work.pending_script_id().key(),
            );
            let released = self.release_module_script_load_delay(
                work.owner(),
                work.script().mode,
                work.load_delay_token(),
            );
            if released || order_released {
                self.queue_lifecycle_followups_for_module_work(work.owner(), work.realm_id());
                Err(DocumentScriptExecutionOutcome::Progressed)
            } else {
                Err(DocumentScriptExecutionOutcome::NoProgress)
            }
        }
    }

    fn finish_graph_success<'owner>(
        &'owner mut self,
        work: &DocumentModuleGraphReadyWork,
        _root_entry: ModuleEntryId,
    ) -> Self::Output<'owner> {
        self.complete_parser_deferred_module_script(
            work.owner(),
            work.realm_id(),
            work.script().mode,
            work.pending_script_id().key(),
        );
        self.release_module_script_load_delay(
            work.owner(),
            work.script().mode,
            work.load_delay_token(),
        );
        self.queue_lifecycle_followups_for_module_work(work.owner(), work.realm_id());
        DocumentScriptExecutionOutcome::Progressed
    }

    fn finish_graph_evaluation_pending<'owner>(
        &'owner mut self,
        work: &DocumentModuleGraphReadyWork,
        _root_entry: ModuleEntryId,
    ) -> Self::Output<'owner> {
        self.complete_parser_deferred_module_script(
            work.owner(),
            work.realm_id(),
            work.script().mode,
            work.pending_script_id().key(),
        );
        self.release_module_script_load_delay(
            work.owner(),
            work.script().mode,
            work.load_delay_token(),
        );
        self.queue_lifecycle_followups_for_module_work(work.owner(), work.realm_id());
        DocumentScriptExecutionOutcome::Progressed
    }

    fn finish_graph_evaluation_failed<'owner>(
        &'owner mut self,
        work: &DocumentModuleGraphReadyWork,
        _error: &ModuleLoadError,
    ) -> Self::Output<'owner> {
        self.complete_parser_deferred_module_script(
            work.owner(),
            work.realm_id(),
            work.script().mode,
            work.pending_script_id().key(),
        );
        self.release_module_script_load_delay(
            work.owner(),
            work.script().mode,
            work.load_delay_token(),
        );
        self.queue_lifecycle_followups_for_module_work(work.owner(), work.realm_id());
        DocumentScriptExecutionOutcome::Progressed
    }

    fn dispatch_script_element_event(
        &mut self,
        work: &DocumentModuleGraphReadyWork,
        kind: FrameDocumentScriptElementEventKind,
    ) -> Result<()> {
        self.dispatch_script_element_event_for_route_parts(
            work.owner(),
            work.realm_id(),
            work.script_handle(),
            kind,
        )
    }

    fn dispatch_graph_failure_script_element_event(
        &mut self,
        work: &DocumentModuleGraphFailedWork,
        kind: FrameDocumentScriptElementEventKind,
    ) -> Result<()> {
        self.dispatch_script_element_event_for_route_parts(
            work.owner(),
            work.realm_id(),
            work.script_handle(),
            kind,
        )
    }

    fn report_module_exception(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        message: &str,
        filename: &str,
        error_value: Option<ScriptErrorValue>,
    ) -> Result<()> {
        self.vm
            .report_child_window_error_body(owner, realm_id, message, filename, error_value)
    }

    fn finish_graph_failure<'owner>(
        &'owner mut self,
        work: &DocumentModuleGraphFailedWork,
    ) -> Self::Output<'owner> {
        self.complete_parser_deferred_module_script(
            work.owner(),
            work.realm_id(),
            work.script().mode,
            work.pending_script_id().key(),
        );
        self.release_module_script_load_delay(
            work.owner(),
            work.script().mode,
            work.load_delay_token(),
        );
        self.queue_lifecycle_followups_for_module_work(work.owner(), work.realm_id());
        DocumentScriptExecutionOutcome::Progressed
    }

    fn finish_evaluation_rejected<'owner>(
        &'owner mut self,
        work: &DocumentModuleGraphReadyWork,
    ) -> Self::Output<'owner> {
        self.release_module_script_load_delay(
            work.owner(),
            work.script().mode,
            work.load_delay_token(),
        );
        self.queue_lifecycle_followups_for_module_work(work.owner(), work.realm_id());
        DocumentScriptExecutionOutcome::Progressed
    }

    fn finish_evaluation_pending<'owner>(
        &'owner mut self,
        _work: &DocumentModuleGraphReadyWork,
    ) -> Self::Output<'owner> {
        DocumentScriptExecutionOutcome::NoProgress
    }

    fn record_runtime_warning(&mut self, message: std::fmt::Arguments<'_>) {
        self.vm.record_runtime_warning(message);
    }
}

impl ChildModuleScriptExecutionOwner<'_> {
    fn complete_parser_deferred_module_script(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        mode: ScriptMode,
        pending_script_key: crate::document_script_scheduler::ParserPendingScriptKey,
    ) -> bool {
        if mode != ScriptMode::ModuleDefer {
            return false;
        }
        let application = self
            .vm
            ._context_host
            .borrow_mut()
            .complete_child_parser_deferred_module_script_for_document_realm(
                owner,
                realm_id,
                crate::document_script_scheduler::ParserPendingScriptId::from_key(
                    owner.document_owner(),
                    pending_script_key,
                ),
            );
        let released = application.order_slot_was_released();
        let queued = application.document_script_ready_was_queued();
        tracing::debug!(
            owner = ?owner,
            realm_id = ?realm_id,
            parser_position = pending_script_key.parser_position(),
            script_node_id = ?pending_script_key.script_node_id(),
            released,
            queued,
            "child module execution completed parser-deferred order slot"
        );
        released
    }

    fn queue_lifecycle_followups_for_module_work(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) -> bool {
        let mut context_host = self.vm._context_host.borrow_mut();
        let domcontentloaded_queued = context_host
            .queue_child_document_domcontentloaded_if_ready_for_document_realm(owner, realm_id);
        let lifecycle_followup_queued = context_host
            .queue_child_document_complete_lifecycle_if_ready_for_document_realm(
                owner.document_owner(),
                realm_id,
            );
        domcontentloaded_queued || lifecycle_followup_queued
    }

    fn dispatch_script_element_event_for_route_parts(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        script_handle: DomHandle,
        kind: FrameDocumentScriptElementEventKind,
    ) -> Result<()> {
        ChildDocumentEventOwner::new(self.vm)
            .dispatch_script_element_event_for_parts_selected_task_body(
                owner,
                realm_id,
                script_handle,
                kind,
            )
    }

    fn check_current_frame_parser_module_route(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) -> std::result::Result<(), DocumentScriptExecutionOutcome> {
        if self
            .vm
            .child_parser_module_route_task_is_current(owner, realm_id)
        {
            Ok(())
        } else {
            Err(DocumentScriptExecutionOutcome::NoProgress)
        }
    }

    fn check_current_module_work(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        mode: ScriptMode,
        pending_script_key: crate::document_script_scheduler::ParserPendingScriptKey,
        load_delay_token: crate::frame_owner_model::DocumentLoadDelayTokenId,
    ) -> std::result::Result<(), DocumentScriptExecutionOutcome> {
        let current = self.check_current_frame_parser_module_route(owner, realm_id);
        if current.is_err() && mode == ScriptMode::ModuleDefer {
            let application = self
                .vm
                ._context_host
                .borrow_mut()
                .cancel_current_child_parser_deferred_module_script(owner, pending_script_key);
            let order_slot_released = application.order_slot_was_released();
            tracing::debug!(
                owner = ?owner,
                realm_id = ?realm_id,
                parser_position = pending_script_key.parser_position(),
                script_node_id = ?pending_script_key.script_node_id(),
                order_slot_released,
                "disposed stale child module-defer execution work"
            );
            if order_slot_released {
                let released = self.release_module_script_load_delay(owner, mode, load_delay_token);
                if released {
                    self.queue_lifecycle_followups_for_module_work(owner, realm_id);
                }
                return Err(DocumentScriptExecutionOutcome::Progressed);
            }
        }
        if current.is_err() && self.release_module_script_load_delay(owner, mode, load_delay_token)
        {
            self.queue_lifecycle_followups_for_module_work(owner, realm_id);
            return Err(DocumentScriptExecutionOutcome::Progressed);
        }
        current
    }

    fn release_module_script_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
        mode: ScriptMode,
        load_delay_token: crate::frame_owner_model::DocumentLoadDelayTokenId,
    ) -> bool {
        let released = self
            .vm
            ._context_host
            .borrow_mut()
            .release_child_module_script_load_delay(
                owner,
                load_delay_token,
                mode == ScriptMode::ModuleDefer,
            );
        tracing::debug!(
            ?owner,
            ?mode,
            ?load_delay_token,
            released,
            "released child module-script lifecycle delay at execution terminal"
        );
        released
    }
}
