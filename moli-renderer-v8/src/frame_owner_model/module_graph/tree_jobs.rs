use crate::document_module_graph::{ModuleEntryId, ModuleMapKey};
use crate::document_runtime::DomHandle;
use crate::document_script_scheduler::{
    DocumentModuleGraphFailedWork, DocumentModuleGraphReadyWork,
};
use crate::frame_owner_model::{
    FrameDocumentModuleDependencyFetchTask, FrameDocumentOwner, FrameDocumentTaskOwner,
    FrameRealmId,
};
use crate::module_runtime::{
    ModuleGraphHandle, ModuleLoadError, ModuleLoadStage,
    NativeFrameDocumentDependencyFetchBuildFailure, NativeModuleGraphFetchRequest,
    NativeModuleGraphJobAdvance, NativeParserModuleTreeJobResume,
};
use moli_module_script_tree::ModuleTreeId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameDocumentParserModuleTreeAdvanceFailureTrace {
    DependencyFetchTaskConversion,
    DependencyFetchStartRoute,
    OwnerLaneAdvance,
}

pub(crate) enum FrameDocumentParserModuleTreeAdvanceAction {
    QueueDependencyFetches {
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        tree_id: ModuleTreeId,
        resume: Box<NativeParserModuleTreeJobResume>,
        fetches: Vec<NativeModuleGraphFetchRequest>,
    },
    RestoreWaiting {
        resume: Box<NativeParserModuleTreeJobResume>,
    },
    NotifyGraphReady(Box<DocumentModuleGraphReadyWork>),
    NotifyGraphFailed {
        trace: FrameDocumentParserModuleTreeAdvanceFailureTrace,
        work: Box<DocumentModuleGraphFailedWork>,
    },
}

pub(crate) enum FrameDocumentParserModuleTreeAdvanceDependencyFetchResult {
    Followup(FrameDocumentModuleScriptTerminalFollowup),
    DependencyFetchStartFailed {
        trace: FrameDocumentParserModuleTreeAdvanceFailureTrace,
        work: Box<DocumentModuleGraphFailedWork>,
    },
}

pub(crate) trait FrameDocumentParserModuleTreeAdvanceHooks {
    fn queue_dependency_fetches(
        &mut self,
        document_owner: FrameDocumentOwner,
        realm_id: FrameRealmId,
        tree_id: ModuleTreeId,
        resume: Box<NativeParserModuleTreeJobResume>,
        fetches: Vec<NativeModuleGraphFetchRequest>,
    ) -> FrameDocumentParserModuleTreeAdvanceDependencyFetchResult;

    fn restore_waiting(&mut self, resume: Box<NativeParserModuleTreeJobResume>);

    fn notify_graph_ready(
        &mut self,
        work: Box<DocumentModuleGraphReadyWork>,
    ) -> FrameDocumentModuleScriptTerminalFollowup;

    fn notify_graph_failed(
        &mut self,
        trace: FrameDocumentParserModuleTreeAdvanceFailureTrace,
        work: Box<DocumentModuleGraphFailedWork>,
    ) -> FrameDocumentModuleScriptTerminalFollowup;
}

pub(crate) struct FrameDocumentParserModuleTreeAdvanceRunner<Hooks> {
    hooks: Hooks,
}

pub(crate) enum FrameDocumentModuleScriptGraphNotification {
    Ready(Box<DocumentModuleGraphReadyWork>),
    Failed(Box<DocumentModuleGraphFailedWork>),
}

impl FrameDocumentModuleScriptGraphNotification {
    pub(crate) fn ready(work: DocumentModuleGraphReadyWork) -> Self {
        Self::Ready(Box::new(work))
    }

    pub(crate) fn failed(work: DocumentModuleGraphFailedWork) -> Self {
        Self::Failed(Box::new(work))
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameDocumentModuleScriptTerminalFollowup {
    module_dependency_fetch_queued: bool,
    document_script_ready_queued: bool,
    module_script_wait_retained: bool,
}

impl FrameDocumentModuleScriptTerminalFollowup {
    pub(crate) fn none() -> Self {
        Self::default()
    }

    pub(crate) fn module_dependency_fetch_queued() -> Self {
        Self {
            module_dependency_fetch_queued: true,
            document_script_ready_queued: false,
            module_script_wait_retained: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn document_script_ready_queued() -> Self {
        Self {
            module_dependency_fetch_queued: false,
            document_script_ready_queued: true,
            module_script_wait_retained: false,
        }
    }

    pub(crate) fn module_script_wait_retained() -> Self {
        Self {
            module_dependency_fetch_queued: false,
            document_script_ready_queued: false,
            module_script_wait_retained: true,
        }
    }

    #[cfg(test)]
    pub(crate) fn module_dependency_fetch_was_queued(self) -> bool {
        self.module_dependency_fetch_queued
    }

    #[cfg(test)]
    pub(crate) fn document_script_ready_was_queued(self) -> bool {
        self.document_script_ready_queued
    }

    #[cfg(test)]
    pub(crate) fn module_script_wait_was_retained(self) -> bool {
        self.module_script_wait_retained
    }

    #[cfg(test)]
    pub(crate) fn made_progress(self) -> bool {
        self.module_dependency_fetch_queued
            || self.document_script_ready_queued
            || self.module_script_wait_retained
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.module_dependency_fetch_queued |= other.module_dependency_fetch_queued;
        self.document_script_ready_queued |= other.document_script_ready_queued;
        self.module_script_wait_retained |= other.module_script_wait_retained;
    }
}

impl<Hooks> FrameDocumentParserModuleTreeAdvanceRunner<Hooks> {
    pub(crate) fn new(hooks: Hooks) -> Self {
        Self { hooks }
    }

    #[cfg(test)]
    pub(crate) fn into_hooks(self) -> Hooks {
        self.hooks
    }
}

impl<Hooks> FrameDocumentParserModuleTreeAdvanceRunner<Hooks>
where
    Hooks: FrameDocumentParserModuleTreeAdvanceHooks,
{
    pub(crate) fn run_tree_advance_action(
        &mut self,
        action: FrameDocumentParserModuleTreeAdvanceAction,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        match action {
            FrameDocumentParserModuleTreeAdvanceAction::QueueDependencyFetches {
                document_owner,
                realm_id,
                tree_id,
                resume,
                fetches,
            } => {
                let result = self.hooks.queue_dependency_fetches(
                    document_owner,
                    realm_id,
                    tree_id,
                    resume,
                    fetches,
                );
                match result {
                    FrameDocumentParserModuleTreeAdvanceDependencyFetchResult::DependencyFetchStartFailed { trace, work } => {
                        self.run_tree_advance_action(
                            FrameDocumentParserModuleTreeAdvanceAction::NotifyGraphFailed {
                                trace,
                                work,
                            },
                        )
                    }
                    FrameDocumentParserModuleTreeAdvanceDependencyFetchResult::Followup(
                        followup,
                    ) => followup,
                }
            }
            FrameDocumentParserModuleTreeAdvanceAction::RestoreWaiting { resume } => {
                self.hooks.restore_waiting(resume);
                FrameDocumentModuleScriptTerminalFollowup::module_script_wait_retained()
            }
            FrameDocumentParserModuleTreeAdvanceAction::NotifyGraphReady(work) => {
                self.hooks.notify_graph_ready(work)
            }
            FrameDocumentParserModuleTreeAdvanceAction::NotifyGraphFailed { trace, work } => {
                self.hooks.notify_graph_failed(trace, work)
            }
        }
    }
}

pub(crate) fn frame_document_parser_module_tree_advance_action(
    document_owner: FrameDocumentOwner,
    realm_id: FrameRealmId,
    tree_id: ModuleTreeId,
    resume: NativeParserModuleTreeJobResume,
    advance_result: Result<NativeModuleGraphJobAdvance, ModuleLoadError>,
) -> FrameDocumentParserModuleTreeAdvanceAction {
    match advance_result {
        Ok(NativeModuleGraphJobAdvance::NeedFetches(fetches)) => {
            FrameDocumentParserModuleTreeAdvanceAction::QueueDependencyFetches {
                document_owner,
                realm_id,
                tree_id,
                resume: Box::new(resume),
                fetches,
            }
        }
        Ok(NativeModuleGraphJobAdvance::WaitingForFetches) => {
            FrameDocumentParserModuleTreeAdvanceAction::RestoreWaiting {
                resume: Box::new(resume),
            }
        }
        Ok(NativeModuleGraphJobAdvance::Complete(graph)) => {
            FrameDocumentParserModuleTreeAdvanceAction::NotifyGraphReady(Box::new(
                module_script_graph_ready_work_from_tree_job(resume, graph),
            ))
        }
        Err(error) => FrameDocumentParserModuleTreeAdvanceAction::NotifyGraphFailed {
            trace: FrameDocumentParserModuleTreeAdvanceFailureTrace::OwnerLaneAdvance,
            work: Box::new(module_script_graph_failed_work_from_tree_job(resume, error)),
        },
    }
}

pub(super) fn module_load_error_for_missing_child_document_modulator_entry(
    owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
) -> ModuleLoadError {
    let error = ModuleLoadError::new(
        ModuleLoadStage::Fetch,
        "child parser module dependency fetch had no current document modulator entry",
    );
    tracing::debug!(
        owner = ?owner,
        realm_id = ?realm_id,
        message = %error.message(),
        "child parser module dependency fetch dropped before graph-ready work"
    );
    error
}

pub(super) fn trace_child_module_dependency_fetch_tasks(
    tasks: &[FrameDocumentModuleDependencyFetchTask],
) {
    for task in tasks {
        let entry_id = task.reservation().entry_id();
        tracing::debug!(
            owner = ?task.owner(),
            realm_id = ?task.realm_id(),
            parent_entry_id = task.client().parent_entry_id().raw(),
            parent_url = %task.client().parent_key().url(),
            specifier = %task.client().specifier(),
            dependency_url = %task.dependency_key().url(),
            phase = ?task.client().phase(),
            tree_id = task.client().tree_client().tree_id.0,
            tree_client_sequence = task.client().tree_client().sequence,
            entry_id = entry_id.raw(),
            "child parser module dependency fetch task emitted from shared tree job"
        );
    }
}

pub(super) fn trace_child_module_dependency_build_failure(
    owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
    failure: &NativeFrameDocumentDependencyFetchBuildFailure,
) {
    trace_child_module_dependency_failure(
        owner,
        realm_id,
        failure.parent_entry_id(),
        failure.dependency_key(),
        failure.error(),
    );
}

pub(crate) fn module_script_graph_ready_work_from_tree_job(
    mut resume: NativeParserModuleTreeJobResume,
    graph: ModuleGraphHandle,
) -> DocumentModuleGraphReadyWork {
    resume
        .root_mut()
        .set_dependency_count(graph.entries.len().saturating_sub(1));
    let root = resume.root();
    DocumentModuleGraphReadyWork::new(
        root.owner(),
        root.realm_id(),
        root.pending_script_id(),
        root.script().clone(),
        root.script_handle(),
        root.request_key().clone(),
        root.tree_id(),
        root.load_delay_token(),
        graph,
    )
}

pub(crate) fn module_script_graph_failed_work_from_tree_job(
    resume: NativeParserModuleTreeJobResume,
    error: ModuleLoadError,
) -> DocumentModuleGraphFailedWork {
    let root = resume.root();
    DocumentModuleGraphFailedWork::new(
        root.owner(),
        root.realm_id(),
        root.pending_script_id(),
        root.script().clone(),
        root.script_handle(),
        root.request_key().clone(),
        Some(root.tree_id()),
        root.load_delay_token(),
        error,
    )
}

pub(crate) fn module_script_graph_failed_work_from_root_client(
    owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
    pending_script_id: crate::document_script_scheduler::ParserPendingScriptId<
        crate::frame_owner_model::FrameDocumentOwner,
    >,
    script: crate::planning::PreparedScript,
    script_handle: DomHandle,
    request_key: ModuleMapKey,
    load_delay_token: crate::frame_owner_model::ChildDocumentModuleScriptLoadDelay,
    error: ModuleLoadError,
) -> DocumentModuleGraphFailedWork {
    DocumentModuleGraphFailedWork::new(
        owner,
        realm_id,
        pending_script_id,
        script,
        script_handle,
        request_key,
        None,
        load_delay_token,
        error,
    )
}

pub(crate) fn trace_child_parser_module_root_failure(
    owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
    script_handle: DomHandle,
    request_key: &ModuleMapKey,
    error: &ModuleLoadError,
) {
    tracing::debug!(
        owner = ?owner,
        realm_id = ?realm_id,
        script_handle = ?script_handle,
        url = %request_key.url(),
        message = %error.message(),
        "child parser module root failed before graph-ready work"
    );
}

pub(crate) fn trace_child_module_dependency_failure(
    owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
    parent_entry_id: Option<ModuleEntryId>,
    dependency_key: &ModuleMapKey,
    error: &ModuleLoadError,
) {
    tracing::debug!(
        owner = ?owner,
        realm_id = ?realm_id,
        parent_entry_id = ?parent_entry_id.map(ModuleEntryId::raw),
        dependency_url = %dependency_key.url(),
        message = %error.message(),
        "child module dependency failed before graph-ready work"
    );
}
