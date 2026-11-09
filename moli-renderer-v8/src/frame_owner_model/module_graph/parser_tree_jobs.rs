use crate::{
    frame_owner_model::{
        FrameDocumentModuleDependencyFetchTask, FrameDocumentModuleDependencyTerminalWork,
        FrameDocumentModuleFetchTerminalResult, FrameDocumentModuleScriptTerminalBatchTask,
        FrameDocumentModuleScriptTerminalTask, FrameDocumentModuleTerminalBatch,
        FrameDocumentOwner, FrameRealmId,
    },
    module_runtime::{
        ModuleLoadError, NativeModuleGraphFetchRequest, NativeParserModuleTreeJobResume,
    },
};

use super::{
    ChildDocumentModulatorStore,
    tree_jobs::{
        module_load_error_for_missing_child_document_modulator_entry,
        trace_child_module_dependency_build_failure, trace_child_module_dependency_fetch_tasks,
    },
};
use moli_module_script_tree as module_tree;

impl ChildDocumentModulatorStore {
    pub(crate) fn take_parser_module_tree_job(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        tree_id: module_tree::ModuleTreeId,
    ) -> Option<NativeParserModuleTreeJobResume> {
        let document_modulator_entry =
            self.current_document_modulator_entry_mut(owner, realm_id)?;
        document_modulator_entry
            .document_modulator
            .take_parser_module_tree_job(tree_id)
    }

    pub(crate) fn restore_parser_module_tree_job(
        &mut self,
        resume: NativeParserModuleTreeJobResume,
    ) {
        let owner = resume.root().owner().document_owner();
        let realm_id = resume.root().realm_id();
        let Some(document_modulator_entry) =
            self.current_document_modulator_entry_mut(owner, realm_id)
        else {
            return;
        };
        document_modulator_entry
            .document_modulator
            .restore_parser_module_tree_job(resume);
    }

    pub(crate) fn record_parser_module_tree_fetches(
        &mut self,
        resume: &mut NativeParserModuleTreeJobResume,
        fetches: Vec<NativeModuleGraphFetchRequest>,
    ) -> Result<Vec<FrameDocumentModuleDependencyFetchTask>, ModuleLoadError> {
        let owner = resume.root().owner();
        let realm_id = resume.root().realm_id();
        let Some(document_modulator_entry) =
            self.current_document_modulator_entry_mut(owner.document_owner(), realm_id)
        else {
            return Err(
                module_load_error_for_missing_child_document_modulator_entry(owner, realm_id),
            );
        };
        let tasks = document_modulator_entry
            .document_modulator
            .frame_document_static_dependency_fetch_tasks(resume, fetches)
            .map_err(|failure| {
                let failure = *failure;
                trace_child_module_dependency_build_failure(owner, realm_id, &failure);
                failure.into_error()
            })?;
        trace_child_module_dependency_fetch_tasks(&tasks);
        Ok(tasks)
    }

    pub(crate) fn finish_parser_module_dependency_fetch(
        &mut self,
        owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        task: FrameDocumentModuleDependencyFetchTask,
        result: FrameDocumentModuleFetchTerminalResult,
    ) -> Vec<FrameDocumentModuleScriptTerminalBatchTask> {
        let Some(_document_modulator_entry) =
            self.current_document_modulator_entry(owner, realm_id)
        else {
            return Vec::new();
        };
        if task.owner().document_owner() != owner || task.realm_id() != realm_id {
            return Vec::new();
        }
        let work = FrameDocumentModuleDependencyTerminalWork::from_fetch_task_result(task, result);
        let task_owner = work.owner();
        let task_realm_id = work.realm_id();
        let mut batch = FrameDocumentModuleTerminalBatch::default();
        batch.push_module_script_terminals(
            task_owner,
            task_realm_id,
            vec![FrameDocumentModuleScriptTerminalTask::dependency(work)],
        );
        let (tasks, _modulepreload_terminal_works, _dynamic_import_owner_actions, _warnings) =
            batch.into_parts();
        tasks
    }
}
