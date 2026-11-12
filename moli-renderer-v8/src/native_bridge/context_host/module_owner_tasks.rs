use super::PageScriptHost;
use crate::frame_owner_model::{
    FrameDocumentDynamicImportTerminalPreparedAction, FrameDocumentModuleDependencyFetchTask,
    FrameDocumentModuleScriptTerminalBatchTask, FrameDocumentModulepreloadEventAction,
    FrameDocumentOwner, FrameDocumentTaskOwner, FrameDocumentTaskRealmCurrentness, FrameRealmId,
};
use crate::module_runtime::{ModuleLoadError, ModuleLoadStage};
use crate::page_task_queue::RendererPageChildModuleDependencyFetchStartEnqueue;

impl PageScriptHost {
    pub(crate) fn current_main_document_task_owner(&self) -> Option<FrameDocumentTaskOwner> {
        self.frame_owner_store.current_main_document_task_owner()
    }

    pub(crate) fn current_child_document_task_owner(
        &self,
        handle: crate::document_runtime::DomHandle,
    ) -> Option<FrameDocumentTaskOwner> {
        self.frame_owner_store
            .current_child_document_task_owner(handle)
    }

    pub(crate) fn route_child_module_dependency_fetch_start(
        &self,
        task: FrameDocumentModuleDependencyFetchTask,
    ) -> Result<RendererPageChildModuleDependencyFetchStartEnqueue, ModuleLoadError> {
        let owner = task.owner();
        let realm_id = task.realm_id();
        let dependency_key = task.dependency_key().clone();
        let Some(target) = self.current_child_module_fetch_target_for_realm(owner, realm_id) else {
            tracing::debug!(
                ?owner,
                ?realm_id,
                ?dependency_key,
                "discarded child module dependency fetch start without an exact current target"
            );
            return Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                format!(
                    "child module dependency {} lost its exact Document/realm target before fetch start",
                    dependency_key.url()
                ),
            ));
        };
        match self
            .page_child_module_dependency_fetch_start_sender()
            .send(target, task)
        {
            Ok(outcome) => Ok(outcome),
            Err(closed) => {
                let rejected_task = closed.into_task();
                debug_assert_eq!(rejected_task.owner(), owner);
                debug_assert_eq!(rejected_task.realm_id(), realm_id);
                tracing::debug!(
                    ?target,
                    ?dependency_key,
                    "discarded child module dependency fetch start after its stable Page route closed"
                );
                Err(ModuleLoadError::new(
                    ModuleLoadStage::Fetch,
                    format!(
                        "child module dependency {} lost its stable Page route before fetch start",
                        dependency_key.url()
                    ),
                ))
            }
        }
    }

    pub(crate) fn route_child_module_script_terminal(
        &self,
        task: FrameDocumentModuleScriptTerminalBatchTask,
    ) -> bool {
        let owner = task.owner();
        let realm_id = task.realm_id();
        match self.page_child_module_script_terminal_sender().send(task) {
            Ok(()) => true,
            Err(closed) => {
                let rejected = closed.into_terminal();
                debug_assert_eq!(rejected.owner(), owner);
                debug_assert_eq!(rejected.realm_id(), realm_id);
                tracing::debug!(
                    ?owner,
                    ?realm_id,
                    "discarded child module-script terminal after its stable Page route closed"
                );
                false
            }
        }
    }

    pub(crate) fn route_child_modulepreload_event_action(
        &self,
        action: FrameDocumentModulepreloadEventAction,
    ) -> bool {
        let sender = self.page_child_modulepreload_event_action_sender().clone();
        match sender.send(action) {
            Ok(()) => true,
            Err(closed) => {
                let action = closed.into_action();
                tracing::debug!(
                    owner = ?action.owner(),
                    realm_id = ?action.realm_id(),
                    link_handle = ?action.link_handle(),
                    "discarded child modulepreload event after its stable Page route closed"
                );
                false
            }
        }
    }

    pub(crate) fn route_child_dynamic_import_owner_actions(
        &mut self,
        actions: Vec<FrameDocumentDynamicImportTerminalPreparedAction>,
    ) -> bool {
        if actions.is_empty() {
            return false;
        }
        match self
            .page_dynamic_import_owner_action_sender()
            .send_all(actions)
        {
            Ok(queued) => queued,
            Err(_) => {
                tracing::debug!(
                    "discarded child dynamic-import owner actions after their stable Page route closed"
                );
                false
            }
        }
    }

    pub(crate) fn current_child_module_route_task_owner(
        &self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
    ) -> Option<FrameDocumentTaskOwner> {
        match self
            .frame_owner_store
            .frame_document_owner_realm_currentness(owner, realm_id)
        {
            FrameDocumentTaskRealmCurrentness::Current { owner, .. } => Some(owner),
            FrameDocumentTaskRealmCurrentness::StaleOwner
            | FrameDocumentTaskRealmCurrentness::MissingRealm { .. }
            | FrameDocumentTaskRealmCurrentness::PendingRealm { .. }
            | FrameDocumentTaskRealmCurrentness::StaleRealm { .. } => None,
        }
    }

    pub(crate) fn current_child_modulepreload_event_action_is_runnable(
        &self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) -> bool {
        self.current_child_module_route_task_owner(owner.document_owner(), realm_id) == Some(owner)
            && !self.has_pending_child_document_script_ready_task_for_owner(owner)
    }

    pub(crate) fn current_child_module_terminal_work_is_runnable(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        !self.has_pending_child_document_script_ready_task_for_owner(owner)
    }

    pub(crate) fn current_child_dynamic_import_task_owner(
        &self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
    ) -> Option<FrameDocumentTaskOwner> {
        self.frame_owner_store
            .current_document_task_owner_for_execution_context(owner.local_window_id, realm_id)
    }
}
