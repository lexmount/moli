use std::collections::HashMap;
use std::fmt;

use moli_module_script_tree as module_tree;

use crate::document_module_graph::{ModuleEntryId, ModuleMapKey};
use crate::document_runtime::DomHandle;
use crate::document_script_scheduler::{ParserPendingScriptId, ParserPendingScriptKey};
use crate::frame_owner_model::{
    ChildDocumentModuleScriptLoadDelay, FrameDocumentTaskOwner, FrameRealmId,
};
use crate::planning::PreparedScript;

use super::NativeModuleGraphJob;

pub(crate) struct NativeParserModuleTreeRoot {
    owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
    pending_script_key: ParserPendingScriptKey,
    script: PreparedScript,
    script_handle: DomHandle,
    request_key: ModuleMapKey,
    tree_id: module_tree::ModuleTreeId,
    entry_id: ModuleEntryId,
    request_count: usize,
    dependency_count: usize,
    load_delay_token: ChildDocumentModuleScriptLoadDelay,
}

impl NativeParserModuleTreeRoot {
    pub(crate) fn new(
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        pending_script_key: ParserPendingScriptKey,
        script: PreparedScript,
        script_handle: DomHandle,
        request_key: ModuleMapKey,
        tree_id: module_tree::ModuleTreeId,
        entry_id: ModuleEntryId,
        request_count: usize,
        load_delay_token: ChildDocumentModuleScriptLoadDelay,
    ) -> Self {
        Self {
            owner,
            realm_id,
            pending_script_key,
            script,
            script_handle,
            request_key,
            tree_id,
            entry_id,
            request_count,
            dependency_count: 0,
            load_delay_token,
        }
    }

    pub(crate) fn owner(&self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) fn realm_id(&self) -> FrameRealmId {
        self.realm_id
    }

    pub(crate) fn pending_script_id(
        &self,
    ) -> ParserPendingScriptId<crate::frame_owner_model::FrameDocumentOwner> {
        ParserPendingScriptId::from_key(self.owner.document_owner(), self.pending_script_key)
    }

    pub(crate) fn script(&self) -> &PreparedScript {
        &self.script
    }

    pub(crate) fn script_handle(&self) -> DomHandle {
        self.script_handle
    }

    pub(crate) fn request_key(&self) -> &ModuleMapKey {
        &self.request_key
    }

    pub(crate) fn tree_id(&self) -> module_tree::ModuleTreeId {
        self.tree_id
    }

    pub(crate) fn entry_id(&self) -> ModuleEntryId {
        self.entry_id
    }

    pub(crate) fn request_count(&self) -> usize {
        self.request_count
    }

    pub(crate) fn dependency_count(&self) -> usize {
        self.dependency_count
    }

    pub(crate) fn load_delay_token(&self) -> ChildDocumentModuleScriptLoadDelay {
        self.load_delay_token
    }

    pub(crate) fn add_dependencies(&mut self, count: usize) {
        self.dependency_count += count;
    }

    pub(crate) fn set_dependency_count(&mut self, count: usize) {
        self.dependency_count = count;
    }
}

pub(crate) struct NativeParserModuleTreeJobResume {
    root: NativeParserModuleTreeRoot,
    job: NativeModuleGraphJob,
}

impl NativeParserModuleTreeJobResume {
    fn new(root: NativeParserModuleTreeRoot, job: NativeModuleGraphJob) -> Self {
        Self { root, job }
    }

    pub(crate) fn root(&self) -> &NativeParserModuleTreeRoot {
        &self.root
    }

    pub(crate) fn root_mut(&mut self) -> &mut NativeParserModuleTreeRoot {
        &mut self.root
    }

    pub(crate) fn job_mut(&mut self) -> &mut NativeModuleGraphJob {
        &mut self.job
    }
}

#[derive(Default)]
pub(super) struct NativeParserModuleTreeJobRegistry {
    jobs: HashMap<module_tree::ModuleTreeId, NativeParserModuleTreeJobState>,
}

impl fmt::Debug for NativeParserModuleTreeJobRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeParserModuleTreeJobRegistry")
            .field("job_count", &self.jobs.len())
            .finish()
    }
}

struct NativeParserModuleTreeJobState {
    root: NativeParserModuleTreeRoot,
    job: NativeModuleGraphJob,
}

impl NativeParserModuleTreeJobRegistry {
    pub(super) fn clear(&mut self) {
        self.jobs.clear();
    }

    pub(super) fn insert(&mut self, root: NativeParserModuleTreeRoot, job: NativeModuleGraphJob) {
        self.jobs
            .insert(root.tree_id, NativeParserModuleTreeJobState { root, job });
    }

    pub(super) fn take(
        &mut self,
        tree_id: module_tree::ModuleTreeId,
    ) -> Option<NativeParserModuleTreeJobResume> {
        let state = self.jobs.remove(&tree_id)?;
        Some(NativeParserModuleTreeJobResume::new(state.root, state.job))
    }

    pub(super) fn restore(&mut self, resume: NativeParserModuleTreeJobResume) {
        self.insert(resume.root, resume.job);
    }

    #[cfg(test)]
    pub(super) fn contains(&self, tree_id: module_tree::ModuleTreeId) -> bool {
        self.jobs.contains_key(&tree_id)
    }
}
