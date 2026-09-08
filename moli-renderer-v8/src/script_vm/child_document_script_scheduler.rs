use super::ScriptVm;
use crate::{
    document_script_scheduler::{
        DocumentModuleGraphFailedWork, DocumentModuleGraphReadyWork,
        FrameDocumentClassicScriptSchedulerWork,
    },
    frame_owner_model::{
        FrameDocumentModuleScriptGraphNotification, FrameDocumentModuleScriptTerminalFollowup,
    },
    module_runtime::ModuleEntryId,
    types::ScriptErrorValue,
};

pub(super) struct ChildDocumentScriptSchedulerOwner<'vm> {
    vm: &'vm mut ScriptVm,
}

impl<'vm> ChildDocumentScriptSchedulerOwner<'vm> {
    pub(super) fn new(vm: &'vm mut ScriptVm) -> Self {
        Self { vm }
    }

    pub(super) fn notify_parser_classic_next_owner_action(
        &mut self,
        work: FrameDocumentClassicScriptSchedulerWork,
    ) {
        let mut host = self.vm._context_host.borrow_mut();
        let _ = host.push_child_document_script_ready_input(work);
    }

    pub(super) fn notify_module_script_graph_ready_work(
        &mut self,
        work: DocumentModuleGraphReadyWork,
    ) -> bool {
        let mut host = self.vm._context_host.borrow_mut();
        let queued_ready_work = host
            .child_document_script_schedulers_mut()
            .notify_module_script_graph_ready_work(work);
        host.admit_runnable_child_document_script_tasks();
        queued_ready_work
    }

    pub(super) fn notify_module_script_graph_failed_action(
        &mut self,
        work: DocumentModuleGraphFailedWork,
    ) -> bool {
        let mut host = self.vm._context_host.borrow_mut();
        let queued_ready_work = host
            .child_document_script_schedulers_mut()
            .notify_module_script_graph_failed_action(work);
        host.admit_runnable_child_document_script_tasks();
        queued_ready_work
    }

    pub(super) fn notify_module_script_graph_terminal_work(
        &mut self,
        notification: FrameDocumentModuleScriptGraphNotification,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        match notification {
            FrameDocumentModuleScriptGraphNotification::Ready(work) => {
                self.notify_terminal_graph_ready_work(*work)
            }
            FrameDocumentModuleScriptGraphNotification::Failed(work) => {
                self.notify_terminal_graph_failed_work(*work)
            }
        }
    }

    fn notify_terminal_graph_ready_work(
        &mut self,
        work: DocumentModuleGraphReadyWork,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        let owner = work.owner();
        let realm_id = work.realm_id();
        let script_node_id = work.script().node_id;
        let script_url = work.script().url.clone();
        let script_handle = work.script_handle();
        let tree_id = work.tree_id();
        let entry_id = work.entry_id();
        let graph_entry_count = work.graph().entries.len();
        let dependency_count = work.dependency_count();
        let request_url = work.request_key().url().clone();
        tracing::debug!(
            owner = ?owner,
            realm_id = ?realm_id,
            script_node_id = ?script_node_id,
            script_handle = ?script_handle,
            script_url = %script_url,
            url = %request_url,
            tree_id = tree_id.0,
            entry_id = entry_id.raw(),
            graph_entry_count,
            dependency_count,
            "child module-script graph-ready work returned from module terminal owner"
        );
        if self
            .vm
            ._context_host
            .borrow()
            .current_child_module_route_task_owner(owner.document_owner(), realm_id)
            != Some(owner)
        {
            tracing::debug!(
                owner = ?owner,
                realm_id = ?realm_id,
                script_node_id = ?script_node_id,
                script_handle = ?script_handle,
                script_url = %script_url,
                url = %request_url,
                tree_id = tree_id.0,
                entry_id = entry_id.raw(),
                graph_entry_count,
                dependency_count,
                "dropping stale child module-script graph-ready work before scheduler notification"
            );
            return FrameDocumentModuleScriptTerminalFollowup::none();
        }
        let queued_ready_work = self.notify_module_script_graph_ready_work(work);
        let parser_ordered_ready_work = !queued_ready_work
            && self
                .vm
                ._context_host
                .borrow_mut()
                .queue_next_child_parser_deferred_script_for_document_realm(owner, realm_id);
        tracing::debug!(
            owner = ?owner,
            realm_id = ?realm_id,
            script_node_id = ?script_node_id,
            script_handle = ?script_handle,
            script_url = %script_url,
            url = %request_url,
            tree_id = tree_id.0,
            entry_id = entry_id.raw(),
            graph_entry_count,
            dependency_count,
            queued_ready_work,
            parser_ordered_ready_work,
            "child module-script graph-ready work notified child document script scheduler"
        );
        // Runnable script work has already entered the stable ChildFrameTask
        // source and published its own readiness edge. The module terminal has
        // no second scheduler task to publish.
        FrameDocumentModuleScriptTerminalFollowup::none()
    }

    fn notify_terminal_graph_failed_work(
        &mut self,
        work: DocumentModuleGraphFailedWork,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        let owner = work.owner();
        let realm_id = work.realm_id();
        let script_node_id = work.script().node_id;
        let script_url = work.script().url.clone();
        let script_handle = work.script_handle();
        let request_url = work.request_key().url().clone();
        let tree_id = work.tree_id();
        if self
            .vm
            ._context_host
            .borrow()
            .current_child_module_route_task_owner(owner.document_owner(), realm_id)
            != Some(owner)
        {
            tracing::debug!(
                owner = ?owner,
                realm_id = ?realm_id,
                script_node_id = ?script_node_id,
                script_handle = ?script_handle,
                script_url = %script_url,
                url = %request_url,
                tree_id = ?tree_id.map(|tree_id| tree_id.0),
                message = %work.error().message(),
                "dropping stale child module-script graph-failed work before scheduler notification"
            );
            return FrameDocumentModuleScriptTerminalFollowup::none();
        }
        let queued_ready_work = self.notify_module_script_graph_failed_action(work);
        let parser_ordered_ready_work = !queued_ready_work
            && self
                .vm
                ._context_host
                .borrow_mut()
                .queue_next_child_parser_deferred_script_for_document_realm(owner, realm_id);
        tracing::debug!(
            owner = ?owner,
            realm_id = ?realm_id,
            script_node_id = ?script_node_id,
            script_handle = ?script_handle,
            script_url = %script_url,
            url = %request_url,
            tree_id = ?tree_id.map(|tree_id| tree_id.0),
            queued_ready_work,
            parser_ordered_ready_work,
            "child module-script graph-failed work notified child document script scheduler"
        );
        FrameDocumentModuleScriptTerminalFollowup::none()
    }

    pub(super) fn reserve_pending_parser_module_evaluation(
        &mut self,
        work: &DocumentModuleGraphReadyWork,
        root_entry: ModuleEntryId,
    ) -> u64 {
        self.vm
            ._context_host
            .borrow_mut()
            .child_document_script_schedulers_mut()
            .reserve_pending_parser_module_evaluation(work.clone(), root_entry)
    }

    pub(super) fn remove_pending_parser_module_evaluation(&mut self, reaction_id: u64) -> bool {
        self.vm
            ._context_host
            .borrow_mut()
            .child_document_script_schedulers_mut()
            .remove_pending_parser_module_evaluation(reaction_id)
    }

    pub(super) fn mark_parser_module_evaluation_fulfilled(&mut self, reaction_id: u64) -> usize {
        let mut host = self.vm._context_host.borrow_mut();
        let queued_ready_action_count = host
            .child_document_script_schedulers_mut()
            .mark_parser_module_evaluation_fulfilled(reaction_id, |evaluation| evaluation)
            .map(|update| update.queued_ready_action_count())
            .unwrap_or_default();
        host.admit_runnable_child_document_script_tasks();
        queued_ready_action_count
    }

    pub(super) fn mark_parser_module_evaluation_rejected(
        &mut self,
        reaction_id: u64,
        reason: String,
        error_value: Option<ScriptErrorValue>,
    ) -> usize {
        let mut host = self.vm._context_host.borrow_mut();
        let queued_ready_action_count = host
            .child_document_script_schedulers_mut()
            .mark_parser_module_evaluation_rejected(
                reaction_id,
                reason,
                error_value,
                |evaluation| evaluation,
            )
            .map(|update| update.queued_ready_action_count())
            .unwrap_or_default();
        host.admit_runnable_child_document_script_tasks();
        queued_ready_action_count
    }
}
