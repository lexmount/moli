use std::{
    future::{Future, Ready, ready},
    pin::Pin,
};

use anyhow::Result;

use crate::{
    document_runtime::DomHandle,
    frame_owner_model::{
        FrameDocumentScriptElementEventKind, FrameDocumentTaskOwner, FrameRealmId,
    },
    module_runtime::{ModuleEntryId, ModuleGraphHandle, ModuleLoadError, ModuleMapKey},
    module_script_continuation::ModuleScriptEvaluationReactionState,
    parser_module_evaluation::ParserModuleEvaluationContinuation,
    planning::PreparedScript,
};

use super::{
    DocumentModuleGraphFailedWork, DocumentModuleGraphReadyWork, DocumentModuleScriptReadyWork,
    DocumentScriptExecutionHooks, DocumentScriptExecutionOutcome, DocumentScriptExecutionRunner,
    DocumentScriptExecutionStartReport,
};

pub(crate) enum FrameModuleScriptEvaluationStart {
    /// The shared module record had already completed evaluation before this
    /// script element reached its ordered execution turn. This turn may still
    /// dispatch the element's terminal event, but it did not run module code.
    AlreadyEvaluated,
    /// This turn entered V8 and completed module evaluation synchronously.
    EvaluatedSynchronously,
    Pending {
        root_entry: ModuleEntryId,
    },
}

/// Script-visible activity performed by one exact child module-script task.
///
/// This is deliberately independent from `DocumentScriptExecutionOutcome`:
/// parser/order bookkeeping can progress without entering JavaScript, while a
/// module evaluation can enter V8 even if a later exact-owner finalization is
/// stale. The selected Page-task dispatcher uses this execution-produced fact
/// to choose the task-end completion boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrameModuleScriptTaskActivity {
    NoScriptOrEvent,
    ScriptOrEvent,
}

/// Result produced after a child module body has been authorized and consumed.
///
/// The evaluator may already have performed the module algorithm's required
/// error-handling checkpoint. `activity` does not describe that checkpoint;
/// it describes whether the enclosing HTML task ran script or dispatched an
/// element event and therefore still requires its ordinary task-end
/// completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameModuleScriptRunOutcome<Output> {
    output: Output,
    activity: FrameModuleScriptTaskActivity,
}

impl<Output> FrameModuleScriptRunOutcome<Output> {
    pub(crate) const fn new(output: Output, activity: FrameModuleScriptTaskActivity) -> Self {
        Self { output, activity }
    }

    pub(crate) fn into_output(self) -> Output {
        self.output
    }

    pub(crate) const fn activity(&self) -> FrameModuleScriptTaskActivity {
        self.activity
    }
}

pub(crate) trait FrameModuleScriptGraphReadyWork {
    fn owner(&self) -> FrameDocumentTaskOwner;
    fn realm_id(&self) -> FrameRealmId;
    fn script_handle(&self) -> DomHandle;
    fn request_key(&self) -> &ModuleMapKey;
    fn tree_id(&self) -> moli_module_script_tree::ModuleTreeId;
    fn script(&self) -> &PreparedScript;
    fn entry_id(&self) -> ModuleEntryId;
    fn dependency_count(&self) -> usize;
    fn graph(&self) -> &ModuleGraphHandle;
}

impl FrameModuleScriptGraphReadyWork for DocumentModuleGraphReadyWork {
    fn owner(&self) -> FrameDocumentTaskOwner {
        self.owner()
    }

    fn realm_id(&self) -> FrameRealmId {
        self.realm_id()
    }

    fn script_handle(&self) -> DomHandle {
        self.script_handle()
    }

    fn request_key(&self) -> &ModuleMapKey {
        self.request_key()
    }

    fn tree_id(&self) -> moli_module_script_tree::ModuleTreeId {
        self.tree_id()
    }

    fn script(&self) -> &PreparedScript {
        self.script()
    }

    fn entry_id(&self) -> ModuleEntryId {
        self.entry_id()
    }

    fn dependency_count(&self) -> usize {
        self.dependency_count()
    }

    fn graph(&self) -> &ModuleGraphHandle {
        self.graph()
    }
}

pub(crate) trait FrameModuleScriptGraphFailedWork {
    fn owner(&self) -> FrameDocumentTaskOwner;
    fn realm_id(&self) -> FrameRealmId;
    fn script_handle(&self) -> DomHandle;
    fn request_key(&self) -> &ModuleMapKey;
    fn tree_id(&self) -> Option<moli_module_script_tree::ModuleTreeId>;
    fn script(&self) -> &PreparedScript;
    fn error(&self) -> &ModuleLoadError;
}

impl FrameModuleScriptGraphFailedWork for DocumentModuleGraphFailedWork {
    fn owner(&self) -> FrameDocumentTaskOwner {
        self.owner()
    }

    fn realm_id(&self) -> FrameRealmId {
        self.realm_id()
    }

    fn script_handle(&self) -> DomHandle {
        self.script_handle()
    }

    fn request_key(&self) -> &ModuleMapKey {
        self.request_key()
    }

    fn tree_id(&self) -> Option<moli_module_script_tree::ModuleTreeId> {
        self.tree_id()
    }

    fn script(&self) -> &PreparedScript {
        self.script()
    }

    fn error(&self) -> &ModuleLoadError {
        self.error()
    }
}

pub(crate) trait FrameModuleScriptDocumentScriptHooks {
    type GraphReadyWork: FrameModuleScriptGraphReadyWork;
    type GraphFailureWork: FrameModuleScriptGraphFailedWork;
    type Output<'owner>
    where
        Self: 'owner;

    fn check_current_graph_ready_work(
        &mut self,
        work: &Self::GraphReadyWork,
    ) -> std::result::Result<(), DocumentScriptExecutionOutcome>;

    fn check_current_graph_failure_work(
        &mut self,
        work: &Self::GraphFailureWork,
    ) -> std::result::Result<(), DocumentScriptExecutionOutcome>;

    fn check_current_evaluation_work(
        &mut self,
        work: &Self::GraphReadyWork,
    ) -> std::result::Result<(), DocumentScriptExecutionOutcome>;

    fn output_from_execution_outcome<'owner>(
        &'owner mut self,
        outcome: DocumentScriptExecutionOutcome,
    ) -> Self::Output<'owner>;

    fn start_graph_evaluation(
        &mut self,
        work: &Self::GraphReadyWork,
    ) -> std::result::Result<FrameModuleScriptEvaluationStart, ModuleLoadError>;

    fn mark_graph_evaluated(
        &mut self,
        work: &Self::GraphReadyWork,
        root_entry: ModuleEntryId,
    ) -> std::result::Result<(), DocumentScriptExecutionOutcome>;

    fn finish_graph_success<'owner>(
        &'owner mut self,
        work: &Self::GraphReadyWork,
        root_entry: ModuleEntryId,
    ) -> Self::Output<'owner>;

    fn finish_graph_evaluation_pending<'owner>(
        &'owner mut self,
        work: &Self::GraphReadyWork,
        root_entry: ModuleEntryId,
    ) -> Self::Output<'owner>;

    fn finish_graph_evaluation_failed<'owner>(
        &'owner mut self,
        work: &Self::GraphReadyWork,
        error: &ModuleLoadError,
    ) -> Self::Output<'owner>;

    fn dispatch_script_element_event(
        &mut self,
        work: &Self::GraphReadyWork,
        kind: FrameDocumentScriptElementEventKind,
    ) -> Result<()>;

    fn dispatch_graph_failure_script_element_event(
        &mut self,
        work: &Self::GraphFailureWork,
        kind: FrameDocumentScriptElementEventKind,
    ) -> Result<()>;

    fn finish_graph_failure<'owner>(
        &'owner mut self,
        work: &Self::GraphFailureWork,
    ) -> Self::Output<'owner>;

    fn finish_evaluation_rejected<'owner>(
        &'owner mut self,
        work: &Self::GraphReadyWork,
    ) -> Self::Output<'owner>;

    fn finish_evaluation_pending<'owner>(
        &'owner mut self,
        work: &Self::GraphReadyWork,
    ) -> Self::Output<'owner>;

    fn record_runtime_warning(&mut self, message: std::fmt::Arguments<'_>);
}

pub(crate) struct FrameModuleScriptDocumentScriptRunner<Hooks> {
    hooks: Hooks,
}

type FrameModuleScriptReadyInput<Hooks> = DocumentModuleScriptReadyWork<
    <Hooks as FrameModuleScriptDocumentScriptHooks>::GraphReadyWork,
    <Hooks as FrameModuleScriptDocumentScriptHooks>::GraphFailureWork,
    ParserModuleEvaluationContinuation<
        <Hooks as FrameModuleScriptDocumentScriptHooks>::GraphReadyWork,
    >,
>;

enum FrameModuleScriptExecutionResult<GraphReady, GraphFailure> {
    GraphReadyEvaluationCompleted {
        work: GraphReady,
        root_entry: ModuleEntryId,
        completion_kind: FrameModuleScriptEvaluationCompletionKind,
    },
    GraphReadyEvaluationPending {
        work: GraphReady,
        root_entry: ModuleEntryId,
    },
    GraphReadyEvaluationFailed {
        work: GraphReady,
        error: ModuleLoadError,
    },
    GraphFailure {
        work: GraphFailure,
    },
    EvaluationRejected {
        work: GraphReady,
        reason: String,
    },
    EvaluationPending {
        work: GraphReady,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FrameModuleScriptEvaluationCompletionKind {
    AlreadyEvaluated,
    Synchronous,
    TopLevelAwait,
}

impl FrameModuleScriptEvaluationCompletionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::AlreadyEvaluated => "already-evaluated",
            Self::Synchronous => "synchronous",
            Self::TopLevelAwait => "top-level-await",
        }
    }

    fn dispatches_script_load_event(self) -> bool {
        matches!(self, Self::AlreadyEvaluated | Self::Synchronous)
    }

    fn evaluated_module_in_this_task(self) -> bool {
        self == Self::Synchronous
    }
}

enum FrameModuleScriptFinalizationAction<GraphReady, GraphFailure> {
    Outcome(DocumentScriptExecutionOutcome),
    FinishGraphSuccess {
        work: GraphReady,
        root_entry: ModuleEntryId,
    },
    FinishGraphEvaluationPending {
        work: GraphReady,
        root_entry: ModuleEntryId,
    },
    FinishGraphEvaluationFailed {
        work: GraphReady,
        error: ModuleLoadError,
    },
    FinishGraphFailure {
        work: GraphFailure,
    },
    FinishEvaluationRejected {
        work: GraphReady,
    },
    FinishEvaluationPending {
        work: GraphReady,
    },
}

struct FrameModuleScriptOutputAction<GraphReady, GraphFailure> {
    finalization: FrameModuleScriptFinalizationAction<GraphReady, GraphFailure>,
    activity: FrameModuleScriptTaskActivity,
}

impl<GraphReady, GraphFailure> FrameModuleScriptOutputAction<GraphReady, GraphFailure> {
    fn without_script_or_event(
        finalization: FrameModuleScriptFinalizationAction<GraphReady, GraphFailure>,
    ) -> Self {
        Self {
            finalization,
            activity: FrameModuleScriptTaskActivity::NoScriptOrEvent,
        }
    }

    fn after_script_or_event(
        finalization: FrameModuleScriptFinalizationAction<GraphReady, GraphFailure>,
    ) -> Self {
        Self {
            finalization,
            activity: FrameModuleScriptTaskActivity::ScriptOrEvent,
        }
    }
}

struct FrameModuleScriptExecutionPhaseHooks<'owner, Hooks> {
    hooks: &'owner mut Hooks,
}

impl<Hooks> FrameModuleScriptDocumentScriptRunner<Hooks> {
    pub(crate) fn new(hooks: Hooks) -> Self {
        Self { hooks }
    }
}

impl<Hooks> FrameModuleScriptDocumentScriptRunner<Hooks>
where
    Hooks: FrameModuleScriptDocumentScriptHooks,
{
    pub(crate) fn run_document_script_work(
        &mut self,
        work: FrameModuleScriptReadyInput<Hooks>,
    ) -> Pin<Box<dyn Future<Output = FrameModuleScriptRunOutcome<Hooks::Output<'_>>> + '_>> {
        Box::pin(async move {
            let action = {
                let phase_hooks = FrameModuleScriptExecutionPhaseHooks {
                    hooks: &mut self.hooks,
                };
                let mut runner = DocumentScriptExecutionRunner::new(phase_hooks);
                runner.run_ready_work(work).await.unwrap_or_else(|error| {
                    self.hooks.record_runtime_warning(format_args!(
                        "frame parser module execution phase failed: {error}"
                    ));
                    FrameModuleScriptOutputAction::without_script_or_event(
                        FrameModuleScriptFinalizationAction::Outcome(
                            DocumentScriptExecutionOutcome::NoProgress,
                        ),
                    )
                })
            };
            self.finish_execution_action(action)
        })
    }

    fn finish_execution_action(
        &mut self,
        action: FrameModuleScriptOutputAction<Hooks::GraphReadyWork, Hooks::GraphFailureWork>,
    ) -> FrameModuleScriptRunOutcome<Hooks::Output<'_>> {
        let output = match action.finalization {
            FrameModuleScriptFinalizationAction::Outcome(outcome) => {
                self.hooks.output_from_execution_outcome(outcome)
            }
            FrameModuleScriptFinalizationAction::FinishGraphSuccess { work, root_entry } => {
                self.hooks.finish_graph_success(&work, root_entry)
            }
            FrameModuleScriptFinalizationAction::FinishGraphEvaluationPending {
                work,
                root_entry,
            } => self
                .hooks
                .finish_graph_evaluation_pending(&work, root_entry),
            FrameModuleScriptFinalizationAction::FinishGraphEvaluationFailed { work, error } => {
                self.hooks.finish_graph_evaluation_failed(&work, &error)
            }
            FrameModuleScriptFinalizationAction::FinishGraphFailure { work } => {
                self.hooks.finish_graph_failure(&work)
            }
            FrameModuleScriptFinalizationAction::FinishEvaluationRejected { work } => {
                self.hooks.finish_evaluation_rejected(&work)
            }
            FrameModuleScriptFinalizationAction::FinishEvaluationPending { work } => {
                self.hooks.finish_evaluation_pending(&work)
            }
        };
        FrameModuleScriptRunOutcome::new(output, action.activity)
    }
}

impl<Hooks> FrameModuleScriptExecutionPhaseHooks<'_, Hooks>
where
    Hooks: FrameModuleScriptDocumentScriptHooks,
{
    fn prepare_graph_failed(
        &mut self,
        work: Hooks::GraphFailureWork,
    ) -> DocumentScriptExecutionStartReport<
        FrameModuleScriptReadyInput<Hooks>,
        DocumentScriptExecutionOutcome,
    > {
        let owner = work.owner();
        let realm_id = work.realm_id();
        let script_handle = work.script_handle();
        let request_url = work.request_key().url().clone();
        if let Err(outcome) = self.hooks.check_current_graph_failure_work(&work) {
            tracing::debug!(
                owner = ?owner,
                realm_id = ?realm_id,
                script_handle = ?script_handle,
                url = %request_url,
                tree_id = ?work.tree_id().map(|tree_id| tree_id.0),
                "dropping stale frame parser module graph failure"
            );
            return DocumentScriptExecutionStartReport::dropped(outcome);
        }
        DocumentScriptExecutionStartReport::execute(
            DocumentModuleScriptReadyWork::GraphFailed(work),
            DocumentScriptExecutionOutcome::Progressed,
        )
    }

    fn prepare_graph_ready(
        &mut self,
        work: Hooks::GraphReadyWork,
    ) -> DocumentScriptExecutionStartReport<
        FrameModuleScriptReadyInput<Hooks>,
        DocumentScriptExecutionOutcome,
    > {
        let request_url = work.request_key().url().clone();
        let owner = work.owner();
        let realm_id = work.realm_id();
        let script_node_id = work.script().node_id;
        let script_url = work.script().url.clone();
        let script_handle = work.script_handle();
        let tree_id = work.tree_id();
        let entry_id = work.entry_id();
        let graph_entry_count = work.graph().entries.len();
        let dependency_count = work.dependency_count();
        if let Err(outcome) = self.hooks.check_current_graph_ready_work(&work) {
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
                "dropping stale frame parser module ready work"
            );
            return DocumentScriptExecutionStartReport::dropped(outcome);
        }
        DocumentScriptExecutionStartReport::execute(
            DocumentModuleScriptReadyWork::GraphReady(work),
            DocumentScriptExecutionOutcome::Progressed,
        )
    }

    fn prepare_evaluation_completed(
        &mut self,
        evaluation: ParserModuleEvaluationContinuation<Hooks::GraphReadyWork>,
    ) -> DocumentScriptExecutionStartReport<
        FrameModuleScriptReadyInput<Hooks>,
        DocumentScriptExecutionOutcome,
    > {
        let work = evaluation.work();
        let request_url = work.request_key().url().clone();
        let owner = work.owner();
        let realm_id = work.realm_id();
        let script_handle = work.script_handle();
        if let Err(outcome) = self.hooks.check_current_evaluation_work(work) {
            tracing::debug!(
                owner = ?owner,
                realm_id = ?realm_id,
                script_handle = ?script_handle,
                url = %request_url,
                "dropping stale frame parser module evaluation completion"
            );
            return DocumentScriptExecutionStartReport::dropped(outcome);
        }
        DocumentScriptExecutionStartReport::execute(
            DocumentModuleScriptReadyWork::EvaluationCompleted(evaluation),
            DocumentScriptExecutionOutcome::Progressed,
        )
    }

    fn execute_graph_ready(
        &mut self,
        work: Hooks::GraphReadyWork,
    ) -> FrameModuleScriptExecutionResult<Hooks::GraphReadyWork, Hooks::GraphFailureWork> {
        let request_url = work.request_key().url().clone();
        let owner = work.owner();
        let realm_id = work.realm_id();
        let script_node_id = work.script().node_id;
        let script_url = work.script().url.clone();
        let script_handle = work.script_handle();
        let tree_id = work.tree_id();
        let entry_id = work.entry_id();
        let graph_entry_count = work.graph().entries.len();
        let dependency_count = work.dependency_count();
        match self.hooks.start_graph_evaluation(&work) {
            Ok(
                completion @ (FrameModuleScriptEvaluationStart::AlreadyEvaluated
                | FrameModuleScriptEvaluationStart::EvaluatedSynchronously),
            ) => {
                let completion_kind = match completion {
                    FrameModuleScriptEvaluationStart::AlreadyEvaluated => {
                        FrameModuleScriptEvaluationCompletionKind::AlreadyEvaluated
                    }
                    FrameModuleScriptEvaluationStart::EvaluatedSynchronously => {
                        FrameModuleScriptEvaluationCompletionKind::Synchronous
                    }
                    FrameModuleScriptEvaluationStart::Pending { .. } => unreachable!(),
                };
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
                    completion_kind = completion_kind.as_str(),
                    "frame parser module ready work completed its evaluation start"
                );
                FrameModuleScriptExecutionResult::GraphReadyEvaluationCompleted {
                    root_entry: work.entry_id(),
                    work,
                    completion_kind,
                }
            }
            Ok(FrameModuleScriptEvaluationStart::Pending { root_entry }) => {
                tracing::debug!(
                    owner = ?owner,
                    realm_id = ?realm_id,
                    script_node_id = ?script_node_id,
                    script_handle = ?script_handle,
                    script_url = %script_url,
                    url = %request_url,
                    tree_id = tree_id.0,
                    entry_id = entry_id.raw(),
                    root_entry = root_entry.raw(),
                    graph_entry_count,
                    dependency_count,
                    "frame parser module ready work started pending evaluation"
                );
                FrameModuleScriptExecutionResult::GraphReadyEvaluationPending { work, root_entry }
            }
            Err(error) => {
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
                    message = %error.message(),
                    "frame parser module ready work evaluation failed"
                );
                FrameModuleScriptExecutionResult::GraphReadyEvaluationFailed { work, error }
            }
        }
    }

    fn execute_evaluation_completed(
        &mut self,
        evaluation: ParserModuleEvaluationContinuation<Hooks::GraphReadyWork>,
    ) -> FrameModuleScriptExecutionResult<Hooks::GraphReadyWork, Hooks::GraphFailureWork> {
        let (work, root_entry, _reaction_id, reaction_state) = evaluation.into_parts();
        let request_url = work.request_key().url().clone();
        let owner = work.owner();
        let realm_id = work.realm_id();
        let script_handle = work.script_handle();
        match reaction_state {
            ModuleScriptEvaluationReactionState::Fulfilled => {
                tracing::debug!(
                    owner = ?owner,
                    realm_id = ?realm_id,
                    script_handle = ?script_handle,
                    url = %request_url,
                    "frame parser module pending evaluation fulfilled"
                );
                FrameModuleScriptExecutionResult::GraphReadyEvaluationCompleted {
                    work,
                    root_entry,
                    completion_kind: FrameModuleScriptEvaluationCompletionKind::TopLevelAwait,
                }
            }
            ModuleScriptEvaluationReactionState::Rejected { reason, .. } => {
                FrameModuleScriptExecutionResult::EvaluationRejected { work, reason }
            }
            ModuleScriptEvaluationReactionState::Pending => {
                FrameModuleScriptExecutionResult::EvaluationPending { work }
            }
        }
    }

    fn apply_graph_success(
        &mut self,
        work: Hooks::GraphReadyWork,
        root_entry: ModuleEntryId,
        completion_kind: FrameModuleScriptEvaluationCompletionKind,
    ) -> FrameModuleScriptOutputAction<Hooks::GraphReadyWork, Hooks::GraphFailureWork> {
        let owner = work.owner();
        let realm_id = work.realm_id();
        let script_handle = work.script_handle();
        if let Err(outcome) = self.hooks.mark_graph_evaluated(&work, root_entry) {
            tracing::debug!(
                owner = ?owner,
                realm_id = ?realm_id,
                script_handle = ?script_handle,
                root_entry = root_entry.raw(),
                completion_kind = completion_kind.as_str(),
                "dropping frame parser module success finalization for stale document modulator"
            );
            let activity = if completion_kind.evaluated_module_in_this_task() {
                FrameModuleScriptTaskActivity::ScriptOrEvent
            } else {
                FrameModuleScriptTaskActivity::NoScriptOrEvent
            };
            return FrameModuleScriptOutputAction {
                finalization: FrameModuleScriptFinalizationAction::Outcome(outcome),
                activity,
            };
        }
        let mut activity = if completion_kind.evaluated_module_in_this_task() {
            FrameModuleScriptTaskActivity::ScriptOrEvent
        } else {
            FrameModuleScriptTaskActivity::NoScriptOrEvent
        };
        if completion_kind.dispatches_script_load_event() {
            let event_result = self
                .hooks
                .dispatch_script_element_event(&work, FrameDocumentScriptElementEventKind::Load);
            if event_result.is_ok() {
                activity = FrameModuleScriptTaskActivity::ScriptOrEvent;
            }
            if let Err(error) = event_result {
                tracing::warn!(
                    owner = ?owner,
                    realm_id = ?realm_id,
                    script_handle = ?script_handle,
                    root_entry = root_entry.raw(),
                    completion_kind = completion_kind.as_str(),
                    error = ?error,
                    "frame parser module script load event dispatch failed"
                );
            }
        }
        tracing::debug!(
            owner = ?owner,
            realm_id = ?realm_id,
            script_handle = ?script_handle,
            root_entry = root_entry.raw(),
            completion_kind = completion_kind.as_str(),
            "frame parser module success finalized through DocumentScriptRunner owner action"
        );
        FrameModuleScriptOutputAction {
            finalization: FrameModuleScriptFinalizationAction::FinishGraphSuccess {
                work,
                root_entry,
            },
            activity,
        }
    }

    fn apply_graph_evaluation_pending(
        &mut self,
        work: Hooks::GraphReadyWork,
        root_entry: ModuleEntryId,
    ) -> FrameModuleScriptOutputAction<Hooks::GraphReadyWork, Hooks::GraphFailureWork> {
        let owner = work.owner();
        let realm_id = work.realm_id();
        let script_handle = work.script_handle();
        if let Err(error) = self
            .hooks
            .dispatch_script_element_event(&work, FrameDocumentScriptElementEventKind::Load)
        {
            tracing::warn!(
                owner = ?owner,
                realm_id = ?realm_id,
                script_handle = ?script_handle,
                root_entry = root_entry.raw(),
                error = ?error,
                "frame parser module script load event dispatch failed after starting top-level await"
            );
        }
        FrameModuleScriptOutputAction::after_script_or_event(
            FrameModuleScriptFinalizationAction::FinishGraphEvaluationPending { work, root_entry },
        )
    }

    fn apply_graph_failure(
        &mut self,
        work: Hooks::GraphFailureWork,
    ) -> FrameModuleScriptOutputAction<Hooks::GraphReadyWork, Hooks::GraphFailureWork> {
        let owner = work.owner();
        let realm_id = work.realm_id();
        let script_handle = work.script_handle();
        let request_url = work.request_key().url().clone();
        let event_result = self.hooks.dispatch_graph_failure_script_element_event(
            &work,
            FrameDocumentScriptElementEventKind::Error,
        );
        let event_dispatched = event_result.is_ok();
        if let Err(error) = event_result {
            tracing::warn!(
                owner = ?owner,
                realm_id = ?realm_id,
                script_handle = ?script_handle,
                url = %request_url,
                error = ?error,
                "frame parser module script error event dispatch failed after graph failure"
            );
        }
        self.hooks.record_runtime_warning(format_args!(
            "frame parser module graph `{}` failed through DocumentScriptRunner ready lane: {}",
            request_url,
            work.error().message()
        ));
        tracing::debug!(
            owner = ?owner,
            realm_id = ?realm_id,
            script_node_id = ?work.script().node_id,
            script_url = %work.script().url,
            script_handle = ?script_handle,
            url = %request_url,
            tree_id = ?work.tree_id().map(|tree_id| tree_id.0),
            message = %work.error().message(),
            "frame parser module graph failure consumed from document script scheduler"
        );
        let finalization = FrameModuleScriptFinalizationAction::FinishGraphFailure { work };
        if event_dispatched {
            FrameModuleScriptOutputAction::after_script_or_event(finalization)
        } else {
            FrameModuleScriptOutputAction::without_script_or_event(finalization)
        }
    }

    fn apply_graph_evaluation_failed(
        &mut self,
        work: Hooks::GraphReadyWork,
        error: ModuleLoadError,
    ) -> FrameModuleScriptOutputAction<Hooks::GraphReadyWork, Hooks::GraphFailureWork> {
        let request_url = work.request_key().url().clone();
        let owner = work.owner();
        let realm_id = work.realm_id();
        let script_handle = work.script_handle();
        let event_result = self
            .hooks
            .dispatch_script_element_event(&work, FrameDocumentScriptElementEventKind::Error);
        if let Err(dispatch_error) = event_result {
            tracing::warn!(
                owner = ?owner,
                realm_id = ?realm_id,
                script_handle = ?script_handle,
                error = ?dispatch_error,
                "frame parser module script error event dispatch failed"
            );
        }
        self.hooks.record_runtime_warning(format_args!(
            "frame parser module graph `{}` failed during ready-lane evaluation: {}",
            request_url,
            error.message()
        ));
        FrameModuleScriptOutputAction::after_script_or_event(
            FrameModuleScriptFinalizationAction::FinishGraphEvaluationFailed { work, error },
        )
    }

    fn apply_evaluation_rejected(
        &mut self,
        work: Hooks::GraphReadyWork,
        reason: String,
    ) -> FrameModuleScriptOutputAction<Hooks::GraphReadyWork, Hooks::GraphFailureWork> {
        let request_url = work.request_key().url().clone();
        self.hooks.record_runtime_warning(format_args!(
            "frame parser module graph `{}` pending evaluation rejected: {}",
            request_url, reason
        ));
        FrameModuleScriptOutputAction::without_script_or_event(
            FrameModuleScriptFinalizationAction::FinishEvaluationRejected { work },
        )
    }
}

impl<Hooks> DocumentScriptExecutionHooks for FrameModuleScriptExecutionPhaseHooks<'_, Hooks>
where
    Hooks: FrameModuleScriptDocumentScriptHooks,
{
    type Ready = FrameModuleScriptReadyInput<Hooks>;
    type PreparedWork = FrameModuleScriptReadyInput<Hooks>;
    type PrepareFollowup = DocumentScriptExecutionOutcome;
    type ExecutionResult =
        FrameModuleScriptExecutionResult<Hooks::GraphReadyWork, Hooks::GraphFailureWork>;
    type PostExecutionFollowup =
        FrameModuleScriptExecutionResult<Hooks::GraphReadyWork, Hooks::GraphFailureWork>;
    type Output = FrameModuleScriptOutputAction<Hooks::GraphReadyWork, Hooks::GraphFailureWork>;
    type ExecuteFuture<'owner>
        = Ready<Result<Self::ExecutionResult>>
    where
        Self: 'owner;

    fn prepare_execution(
        &mut self,
        ready: Self::Ready,
    ) -> DocumentScriptExecutionStartReport<Self::PreparedWork, Self::PrepareFollowup> {
        match ready {
            DocumentModuleScriptReadyWork::GraphReady(work) => self.prepare_graph_ready(work),
            DocumentModuleScriptReadyWork::GraphFailed(work) => self.prepare_graph_failed(work),
            DocumentModuleScriptReadyWork::EvaluationCompleted(evaluation) => {
                self.prepare_evaluation_completed(evaluation)
            }
        }
    }

    fn execute_work(&mut self, work: Self::PreparedWork) -> Self::ExecuteFuture<'_> {
        ready(Ok(match work {
            DocumentModuleScriptReadyWork::GraphReady(work) => self.execute_graph_ready(work),
            DocumentModuleScriptReadyWork::GraphFailed(work) => {
                FrameModuleScriptExecutionResult::GraphFailure { work }
            }
            DocumentModuleScriptReadyWork::EvaluationCompleted(evaluation) => {
                self.execute_evaluation_completed(evaluation)
            }
        }))
    }

    fn prepare_post_execution_followup(
        &mut self,
        execution_result: Self::ExecutionResult,
    ) -> Result<Self::PostExecutionFollowup> {
        Ok(execution_result)
    }

    fn apply_post_execution_followup(
        &mut self,
        followup: Self::PostExecutionFollowup,
    ) -> Result<Self::Output> {
        Ok(match followup {
            FrameModuleScriptExecutionResult::GraphReadyEvaluationCompleted {
                work,
                root_entry,
                completion_kind,
            } => self.apply_graph_success(work, root_entry, completion_kind),
            FrameModuleScriptExecutionResult::GraphReadyEvaluationPending { work, root_entry } => {
                self.apply_graph_evaluation_pending(work, root_entry)
            }
            FrameModuleScriptExecutionResult::GraphReadyEvaluationFailed { work, error } => {
                self.apply_graph_evaluation_failed(work, error)
            }
            FrameModuleScriptExecutionResult::GraphFailure { work } => {
                self.apply_graph_failure(work)
            }
            FrameModuleScriptExecutionResult::EvaluationRejected { work, reason } => {
                self.apply_evaluation_rejected(work, reason)
            }
            FrameModuleScriptExecutionResult::EvaluationPending { work } => {
                FrameModuleScriptOutputAction::without_script_or_event(
                    FrameModuleScriptFinalizationAction::FinishEvaluationPending { work },
                )
            }
        })
    }

    fn outcome_for_dropped_ready(
        &mut self,
        prepare_followup: Self::PrepareFollowup,
    ) -> Result<Self::Output> {
        Ok(FrameModuleScriptOutputAction::without_script_or_event(
            FrameModuleScriptFinalizationAction::Outcome(prepare_followup),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document_script_scheduler::{FrameDocumentModuleScriptReadyWork, ParserPendingScriptId},
        dom::NodeId,
        frame_owner_model::{DocumentId, FrameSchedulerLaneId, LocalWindowId},
        module_runtime::{ModuleGraphHandle, ModuleLoadStage, ModuleMapKey},
        parser_module_evaluation::ParserModuleEvaluationStore,
        planning::{PreparedScript, ScriptFetchMetadata, ScriptSource},
        types::{ScriptKind, ScriptMode, ScriptSourceKind},
    };
    use moli_module_script_tree as module_tree;

    struct FakeFrameModuleScriptHooks {
        currentness_outcome: std::result::Result<(), DocumentScriptExecutionOutcome>,
        start_graph_evaluation: Option<FrameModuleScriptEvaluationStart>,
        start_graph_evaluation_error: Option<ModuleLoadError>,
        mark_graph_outcome: std::result::Result<(), DocumentScriptExecutionOutcome>,
        graph_success_outcome: DocumentScriptExecutionOutcome,
        graph_pending_outcome: DocumentScriptExecutionOutcome,
        graph_evaluation_failed_outcome: DocumentScriptExecutionOutcome,
        graph_failure_outcome: DocumentScriptExecutionOutcome,
        evaluation_rejected_outcome: DocumentScriptExecutionOutcome,
        evaluation_pending_outcome: DocumentScriptExecutionOutcome,
        start_graph_evaluation_called: bool,
        mark_graph_evaluated_called: bool,
        finish_graph_success_called: bool,
        finish_graph_evaluation_pending_called: bool,
        finish_graph_evaluation_failed_called: bool,
        dispatch_script_element_event_called: bool,
        dispatch_graph_failure_script_element_event_called: bool,
        finish_evaluation_rejected_called: bool,
        finish_evaluation_pending_called: bool,
        record_runtime_warning_called: bool,
    }

    impl FakeFrameModuleScriptHooks {
        fn stale_currentness() -> Self {
            Self {
                currentness_outcome: Err(DocumentScriptExecutionOutcome::NoProgress),
                start_graph_evaluation: None,
                start_graph_evaluation_error: None,
                mark_graph_outcome: Ok(()),
                graph_success_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_pending_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_evaluation_failed_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_failure_outcome: DocumentScriptExecutionOutcome::Progressed,
                evaluation_rejected_outcome: DocumentScriptExecutionOutcome::Progressed,
                evaluation_pending_outcome: DocumentScriptExecutionOutcome::NoProgress,
                start_graph_evaluation_called: false,
                mark_graph_evaluated_called: false,
                finish_graph_success_called: false,
                finish_graph_evaluation_pending_called: false,
                finish_graph_evaluation_failed_called: false,
                dispatch_script_element_event_called: false,
                dispatch_graph_failure_script_element_event_called: false,
                finish_evaluation_rejected_called: false,
                finish_evaluation_pending_called: false,
                record_runtime_warning_called: false,
            }
        }

        fn stale_success_finalization() -> Self {
            Self {
                currentness_outcome: Ok(()),
                start_graph_evaluation: Some(
                    FrameModuleScriptEvaluationStart::EvaluatedSynchronously,
                ),
                start_graph_evaluation_error: None,
                mark_graph_outcome: Err(DocumentScriptExecutionOutcome::NoProgress),
                graph_success_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_pending_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_evaluation_failed_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_failure_outcome: DocumentScriptExecutionOutcome::Progressed,
                evaluation_rejected_outcome: DocumentScriptExecutionOutcome::Progressed,
                evaluation_pending_outcome: DocumentScriptExecutionOutcome::NoProgress,
                start_graph_evaluation_called: false,
                mark_graph_evaluated_called: false,
                finish_graph_success_called: false,
                finish_graph_evaluation_pending_called: false,
                finish_graph_evaluation_failed_called: false,
                dispatch_script_element_event_called: false,
                dispatch_graph_failure_script_element_event_called: false,
                finish_evaluation_rejected_called: false,
                finish_evaluation_pending_called: false,
                record_runtime_warning_called: false,
            }
        }

        fn graph_success(outcome: DocumentScriptExecutionOutcome) -> Self {
            Self {
                currentness_outcome: Ok(()),
                start_graph_evaluation: Some(
                    FrameModuleScriptEvaluationStart::EvaluatedSynchronously,
                ),
                start_graph_evaluation_error: None,
                mark_graph_outcome: Ok(()),
                graph_success_outcome: outcome,
                graph_pending_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_evaluation_failed_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_failure_outcome: DocumentScriptExecutionOutcome::Progressed,
                evaluation_rejected_outcome: DocumentScriptExecutionOutcome::Progressed,
                evaluation_pending_outcome: DocumentScriptExecutionOutcome::NoProgress,
                start_graph_evaluation_called: false,
                mark_graph_evaluated_called: false,
                finish_graph_success_called: false,
                finish_graph_evaluation_pending_called: false,
                finish_graph_evaluation_failed_called: false,
                dispatch_script_element_event_called: false,
                dispatch_graph_failure_script_element_event_called: false,
                finish_evaluation_rejected_called: false,
                finish_evaluation_pending_called: false,
                record_runtime_warning_called: false,
            }
        }

        fn graph_pending(outcome: DocumentScriptExecutionOutcome) -> Self {
            let pending_root = ModuleEntryId::for_test(5);
            Self {
                currentness_outcome: Ok(()),
                start_graph_evaluation: Some(FrameModuleScriptEvaluationStart::Pending {
                    root_entry: pending_root,
                }),
                start_graph_evaluation_error: None,
                mark_graph_outcome: Ok(()),
                graph_success_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_pending_outcome: outcome,
                graph_evaluation_failed_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_failure_outcome: DocumentScriptExecutionOutcome::Progressed,
                evaluation_rejected_outcome: DocumentScriptExecutionOutcome::Progressed,
                evaluation_pending_outcome: DocumentScriptExecutionOutcome::NoProgress,
                start_graph_evaluation_called: false,
                mark_graph_evaluated_called: false,
                finish_graph_success_called: false,
                finish_graph_evaluation_pending_called: false,
                finish_graph_evaluation_failed_called: false,
                dispatch_script_element_event_called: false,
                dispatch_graph_failure_script_element_event_called: false,
                finish_evaluation_rejected_called: false,
                finish_evaluation_pending_called: false,
                record_runtime_warning_called: false,
            }
        }

        fn graph_evaluation_failed(outcome: DocumentScriptExecutionOutcome) -> Self {
            Self {
                currentness_outcome: Ok(()),
                start_graph_evaluation: None,
                start_graph_evaluation_error: Some(ModuleLoadError::new(
                    ModuleLoadStage::Evaluate,
                    "evaluation failed",
                )),
                mark_graph_outcome: Ok(()),
                graph_success_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_pending_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_evaluation_failed_outcome: outcome,
                graph_failure_outcome: DocumentScriptExecutionOutcome::Progressed,
                evaluation_rejected_outcome: DocumentScriptExecutionOutcome::Progressed,
                evaluation_pending_outcome: DocumentScriptExecutionOutcome::NoProgress,
                start_graph_evaluation_called: false,
                mark_graph_evaluated_called: false,
                finish_graph_success_called: false,
                finish_graph_evaluation_pending_called: false,
                finish_graph_evaluation_failed_called: false,
                dispatch_script_element_event_called: false,
                dispatch_graph_failure_script_element_event_called: false,
                finish_evaluation_rejected_called: false,
                finish_evaluation_pending_called: false,
                record_runtime_warning_called: false,
            }
        }

        fn graph_failure(outcome: DocumentScriptExecutionOutcome) -> Self {
            Self {
                currentness_outcome: Ok(()),
                start_graph_evaluation: None,
                start_graph_evaluation_error: None,
                mark_graph_outcome: Ok(()),
                graph_success_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_pending_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_evaluation_failed_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_failure_outcome: outcome,
                evaluation_rejected_outcome: DocumentScriptExecutionOutcome::Progressed,
                evaluation_pending_outcome: DocumentScriptExecutionOutcome::NoProgress,
                start_graph_evaluation_called: false,
                mark_graph_evaluated_called: false,
                finish_graph_success_called: false,
                finish_graph_evaluation_pending_called: false,
                finish_graph_evaluation_failed_called: false,
                dispatch_script_element_event_called: false,
                dispatch_graph_failure_script_element_event_called: false,
                finish_evaluation_rejected_called: false,
                finish_evaluation_pending_called: false,
                record_runtime_warning_called: false,
            }
        }

        fn evaluation_rejected(outcome: DocumentScriptExecutionOutcome) -> Self {
            Self {
                currentness_outcome: Ok(()),
                start_graph_evaluation: None,
                start_graph_evaluation_error: None,
                mark_graph_outcome: Ok(()),
                graph_success_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_pending_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_evaluation_failed_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_failure_outcome: DocumentScriptExecutionOutcome::Progressed,
                evaluation_rejected_outcome: outcome,
                evaluation_pending_outcome: DocumentScriptExecutionOutcome::NoProgress,
                start_graph_evaluation_called: false,
                mark_graph_evaluated_called: false,
                finish_graph_success_called: false,
                finish_graph_evaluation_pending_called: false,
                finish_graph_evaluation_failed_called: false,
                dispatch_script_element_event_called: false,
                dispatch_graph_failure_script_element_event_called: false,
                finish_evaluation_rejected_called: false,
                finish_evaluation_pending_called: false,
                record_runtime_warning_called: false,
            }
        }

        fn evaluation_pending(outcome: DocumentScriptExecutionOutcome) -> Self {
            Self {
                currentness_outcome: Ok(()),
                start_graph_evaluation: None,
                start_graph_evaluation_error: None,
                mark_graph_outcome: Ok(()),
                graph_success_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_pending_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_evaluation_failed_outcome: DocumentScriptExecutionOutcome::Progressed,
                graph_failure_outcome: DocumentScriptExecutionOutcome::Progressed,
                evaluation_rejected_outcome: DocumentScriptExecutionOutcome::Progressed,
                evaluation_pending_outcome: outcome,
                start_graph_evaluation_called: false,
                mark_graph_evaluated_called: false,
                finish_graph_success_called: false,
                finish_graph_evaluation_pending_called: false,
                finish_graph_evaluation_failed_called: false,
                dispatch_script_element_event_called: false,
                dispatch_graph_failure_script_element_event_called: false,
                finish_evaluation_rejected_called: false,
                finish_evaluation_pending_called: false,
                record_runtime_warning_called: false,
            }
        }
    }

    impl FrameModuleScriptDocumentScriptHooks for FakeFrameModuleScriptHooks {
        type GraphReadyWork = DocumentModuleGraphReadyWork;
        type GraphFailureWork = DocumentModuleGraphFailedWork;
        type Output<'owner>
            = DocumentScriptExecutionOutcome
        where
            Self: 'owner;

        fn check_current_graph_ready_work(
            &mut self,
            _work: &Self::GraphReadyWork,
        ) -> std::result::Result<(), DocumentScriptExecutionOutcome> {
            self.currentness_outcome
        }

        fn check_current_graph_failure_work(
            &mut self,
            _work: &Self::GraphFailureWork,
        ) -> std::result::Result<(), DocumentScriptExecutionOutcome> {
            self.currentness_outcome
        }

        fn check_current_evaluation_work(
            &mut self,
            _work: &Self::GraphReadyWork,
        ) -> std::result::Result<(), DocumentScriptExecutionOutcome> {
            self.currentness_outcome
        }

        fn output_from_execution_outcome<'owner>(
            &'owner mut self,
            outcome: DocumentScriptExecutionOutcome,
        ) -> Self::Output<'owner> {
            outcome
        }

        fn start_graph_evaluation(
            &mut self,
            _work: &DocumentModuleGraphReadyWork,
        ) -> std::result::Result<FrameModuleScriptEvaluationStart, ModuleLoadError> {
            self.start_graph_evaluation_called = true;
            if let Some(error) = self.start_graph_evaluation_error.take() {
                return Err(error);
            }
            Ok(self
                .start_graph_evaluation
                .take()
                .expect("test hook should provide one graph-evaluation start result"))
        }

        fn mark_graph_evaluated(
            &mut self,
            _work: &DocumentModuleGraphReadyWork,
            _root_entry: ModuleEntryId,
        ) -> std::result::Result<(), DocumentScriptExecutionOutcome> {
            self.mark_graph_evaluated_called = true;
            self.mark_graph_outcome
        }

        fn finish_graph_success<'owner>(
            &'owner mut self,
            _work: &DocumentModuleGraphReadyWork,
            _root_entry: ModuleEntryId,
        ) -> Self::Output<'owner> {
            self.finish_graph_success_called = true;
            self.graph_success_outcome
        }

        fn finish_graph_evaluation_pending<'owner>(
            &'owner mut self,
            _work: &DocumentModuleGraphReadyWork,
            _root_entry: ModuleEntryId,
        ) -> Self::Output<'owner> {
            self.finish_graph_evaluation_pending_called = true;
            self.graph_pending_outcome
        }

        fn finish_graph_evaluation_failed<'owner>(
            &'owner mut self,
            _work: &DocumentModuleGraphReadyWork,
            _error: &ModuleLoadError,
        ) -> Self::Output<'owner> {
            self.finish_graph_evaluation_failed_called = true;
            self.graph_evaluation_failed_outcome
        }

        fn dispatch_script_element_event(
            &mut self,
            _work: &DocumentModuleGraphReadyWork,
            _kind: FrameDocumentScriptElementEventKind,
        ) -> Result<()> {
            self.dispatch_script_element_event_called = true;
            Ok(())
        }

        fn dispatch_graph_failure_script_element_event(
            &mut self,
            _work: &DocumentModuleGraphFailedWork,
            _kind: FrameDocumentScriptElementEventKind,
        ) -> Result<()> {
            self.dispatch_graph_failure_script_element_event_called = true;
            Ok(())
        }

        fn finish_graph_failure<'owner>(
            &'owner mut self,
            _work: &DocumentModuleGraphFailedWork,
        ) -> Self::Output<'owner> {
            self.graph_failure_outcome
        }

        fn finish_evaluation_rejected<'owner>(
            &'owner mut self,
            _work: &DocumentModuleGraphReadyWork,
        ) -> Self::Output<'owner> {
            self.finish_evaluation_rejected_called = true;
            self.evaluation_rejected_outcome
        }

        fn finish_evaluation_pending<'owner>(
            &'owner mut self,
            _work: &DocumentModuleGraphReadyWork,
        ) -> Self::Output<'owner> {
            self.finish_evaluation_pending_called = true;
            self.evaluation_pending_outcome
        }

        fn record_runtime_warning(&mut self, _message: std::fmt::Arguments<'_>) {
            self.record_runtime_warning_called = true;
        }
    }

    fn parser_module_script(url: &str) -> PreparedScript {
        let script_url = url::Url::parse(url).expect("module script url");
        PreparedScript {
            position: 1,
            node_id: NodeId::new(1),
            kind: ScriptKind::Module,
            mode: ScriptMode::ModuleDefer,
            source_kind: ScriptSourceKind::External,
            fetch_metadata: ScriptFetchMetadata::default(),
            source: ScriptSource::External,
            url: script_url.clone(),
            base_url: script_url.clone(),
            initiator_url: script_url,
            host_script_handle: None,
        }
    }

    fn module_script_graph_ready_work() -> DocumentModuleGraphReadyWork {
        let task_owner =
            FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
        let realm_id = FrameRealmId(4);
        let script = parser_module_script("https://frame-parser-module.test/module.js");
        let pending_script_id = ParserPendingScriptId::new(task_owner.document_owner(), &script);
        let request_key = ModuleMapKey::java_script(script.url.clone());
        let root_entry = ModuleEntryId::for_test(5);
        DocumentModuleGraphReadyWork::new(
            task_owner,
            realm_id,
            pending_script_id,
            script,
            DomHandle::new(6),
            request_key,
            module_tree::ModuleTreeId(7),
            crate::frame_owner_model::DocumentLoadDelayTokenId(8),
            ModuleGraphHandle {
                root_entry,
                entries: vec![root_entry],
            },
        )
    }

    fn module_script_graph_failed_work() -> DocumentModuleGraphFailedWork {
        let task_owner =
            FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
        let realm_id = FrameRealmId(4);
        let script = parser_module_script("https://frame-parser-module.test/failed-module.js");
        let pending_script_id = ParserPendingScriptId::new(task_owner.document_owner(), &script);
        let request_key = ModuleMapKey::java_script(script.url.clone());
        DocumentModuleGraphFailedWork::new(
            task_owner,
            realm_id,
            pending_script_id,
            script,
            DomHandle::new(6),
            request_key,
            Some(module_tree::ModuleTreeId(7)),
            crate::frame_owner_model::DocumentLoadDelayTokenId(8),
            ModuleLoadError::new(ModuleLoadStage::Fetch, "network failed"),
        )
    }

    fn rejected_parser_module_evaluation()
    -> ParserModuleEvaluationContinuation<DocumentModuleGraphReadyWork> {
        let work = module_script_graph_ready_work();
        let root_entry = work.entry_id();
        let mut store = ParserModuleEvaluationStore::default();
        store.push_pending_with_reaction_id(work, root_entry, 1);
        assert_eq!(
            store.mark_rejected(1, "evaluation rejected".to_owned(), None),
            Some(root_entry)
        );
        store
            .take_ready()
            .expect("rejected parser-module evaluation should become ready")
    }

    fn fulfilled_parser_module_evaluation()
    -> ParserModuleEvaluationContinuation<DocumentModuleGraphReadyWork> {
        let work = module_script_graph_ready_work();
        let root_entry = work.entry_id();
        let mut store = ParserModuleEvaluationStore::default();
        store.push_pending_with_reaction_id(work, root_entry, 1);
        assert_eq!(store.mark_fulfilled(1), Some(root_entry));
        store
            .take_ready()
            .expect("fulfilled parser-module evaluation should become ready")
    }

    fn pending_parser_module_evaluation()
    -> ParserModuleEvaluationContinuation<DocumentModuleGraphReadyWork> {
        let work = module_script_graph_ready_work();
        let root_entry = work.entry_id();
        ParserModuleEvaluationContinuation::pending_for_test(work, root_entry, 1)
    }

    #[tokio::test]
    async fn frame_parser_module_ready_returns_hook_currentness_outcome() {
        let hooks = FakeFrameModuleScriptHooks::stale_currentness();
        let mut runner = FrameModuleScriptDocumentScriptRunner::new(hooks);

        assert_eq!(
            runner
                .run_document_script_work(FrameDocumentModuleScriptReadyWork::GraphReady(
                    module_script_graph_ready_work()
                ))
                .await
                .into_output(),
            DocumentScriptExecutionOutcome::NoProgress
        );
        assert!(
            !runner.hooks.start_graph_evaluation_called,
            "stale ready work should stop at the hook currentness gate"
        );
        assert!(
            !runner.hooks.mark_graph_evaluated_called,
            "stale ready work should not finalize graph evaluation"
        );
        assert!(
            !runner.hooks.dispatch_script_element_event_called,
            "stale ready work should not dispatch script events"
        );
        assert!(
            !runner
                .hooks
                .dispatch_graph_failure_script_element_event_called,
            "stale ready work should not dispatch script events"
        );
        assert!(
            !runner.hooks.record_runtime_warning_called,
            "stale ready work should not record runtime warnings"
        );
    }

    #[tokio::test]
    async fn frame_parser_module_success_returns_hook_finalization_outcome() {
        let hooks = FakeFrameModuleScriptHooks::stale_success_finalization();
        let mut runner = FrameModuleScriptDocumentScriptRunner::new(hooks);

        assert_eq!(
            runner
                .run_document_script_work(FrameDocumentModuleScriptReadyWork::GraphReady(
                    module_script_graph_ready_work()
                ))
                .await
                .into_output(),
            DocumentScriptExecutionOutcome::NoProgress
        );
        assert!(
            runner.hooks.start_graph_evaluation_called,
            "current ready work should start graph evaluation"
        );
        assert!(
            runner.hooks.mark_graph_evaluated_called,
            "completed graph evaluation should reach the success finalization hook"
        );
        assert!(
            !runner.hooks.finish_graph_success_called,
            "stale success finalization should stop before returning a success outcome"
        );
        assert!(
            !runner.hooks.dispatch_script_element_event_called,
            "stale success finalization should stop before dispatching the load event"
        );
    }

    #[tokio::test]
    async fn frame_parser_module_success_returns_hook_success_outcome() {
        let hooks =
            FakeFrameModuleScriptHooks::graph_success(DocumentScriptExecutionOutcome::NoProgress);
        let mut runner = FrameModuleScriptDocumentScriptRunner::new(hooks);

        let outcome = runner
            .run_document_script_work(FrameDocumentModuleScriptReadyWork::GraphReady(
                module_script_graph_ready_work(),
            ))
            .await;
        assert_eq!(
            outcome.activity(),
            FrameModuleScriptTaskActivity::ScriptOrEvent,
            "synchronous module evaluation and its load event require selected callback completion"
        );
        assert_eq!(
            outcome.into_output(),
            DocumentScriptExecutionOutcome::NoProgress
        );
        assert!(
            runner.hooks.start_graph_evaluation_called,
            "current ready work should start graph evaluation"
        );
        assert!(
            runner.hooks.mark_graph_evaluated_called,
            "completed graph evaluation should mark the graph evaluated"
        );
        assert!(
            runner.hooks.dispatch_script_element_event_called,
            "completed graph evaluation should dispatch the load event"
        );
        assert!(
            runner.hooks.finish_graph_success_called,
            "completed graph evaluation should return through the success hook outcome"
        );
    }

    #[tokio::test]
    async fn frame_parser_module_pending_evaluation_returns_hook_outcome() {
        let hooks =
            FakeFrameModuleScriptHooks::graph_pending(DocumentScriptExecutionOutcome::NoProgress);
        let mut runner = FrameModuleScriptDocumentScriptRunner::new(hooks);

        let outcome = runner
            .run_document_script_work(FrameDocumentModuleScriptReadyWork::GraphReady(
                module_script_graph_ready_work(),
            ))
            .await;
        assert_eq!(
            outcome.activity(),
            FrameModuleScriptTaskActivity::ScriptOrEvent,
            "the task that enters module evaluation until TLA and dispatches load requires callback completion"
        );
        assert_eq!(
            outcome.into_output(),
            DocumentScriptExecutionOutcome::NoProgress
        );
        assert!(
            runner.hooks.start_graph_evaluation_called,
            "current ready work should start graph evaluation"
        );
        assert!(
            runner.hooks.finish_graph_evaluation_pending_called,
            "pending graph evaluation should return through the pending hook outcome"
        );
        assert!(
            !runner.hooks.mark_graph_evaluated_called,
            "pending graph evaluation should not mark the graph evaluated"
        );
        assert!(
            runner.hooks.dispatch_script_element_event_called,
            "starting top-level await should complete the script element load event"
        );
    }

    #[tokio::test]
    async fn frame_parser_module_fulfilled_tla_does_not_redispatch_load_event() {
        let hooks =
            FakeFrameModuleScriptHooks::graph_success(DocumentScriptExecutionOutcome::NoProgress);
        let mut runner = FrameModuleScriptDocumentScriptRunner::new(hooks);

        let outcome = runner
            .run_document_script_work(FrameDocumentModuleScriptReadyWork::EvaluationCompleted(
                fulfilled_parser_module_evaluation(),
            ))
            .await;
        assert_eq!(
            outcome.activity(),
            FrameModuleScriptTaskActivity::NoScriptOrEvent,
            "TLA code resumes in its ModuleReaction task; its later scheduler settlement must not claim callback follow-up"
        );
        assert_eq!(
            outcome.into_output(),
            DocumentScriptExecutionOutcome::NoProgress
        );
        assert!(runner.hooks.mark_graph_evaluated_called);
        assert!(runner.hooks.finish_graph_success_called);
        assert!(
            !runner.hooks.dispatch_script_element_event_called,
            "TLA fulfillment must not dispatch a second script load event"
        );
    }

    #[tokio::test]
    async fn frame_parser_module_evaluation_failure_returns_hook_outcome() {
        let hooks = FakeFrameModuleScriptHooks::graph_evaluation_failed(
            DocumentScriptExecutionOutcome::NoProgress,
        );
        let mut runner = FrameModuleScriptDocumentScriptRunner::new(hooks);

        assert_eq!(
            runner
                .run_document_script_work(FrameDocumentModuleScriptReadyWork::GraphReady(
                    module_script_graph_ready_work()
                ))
                .await
                .into_output(),
            DocumentScriptExecutionOutcome::NoProgress
        );
        assert!(
            runner.hooks.start_graph_evaluation_called,
            "current ready work should start graph evaluation"
        );
        assert!(
            runner.hooks.dispatch_script_element_event_called,
            "failed graph evaluation should dispatch the error event"
        );
        assert!(
            runner.hooks.record_runtime_warning_called,
            "failed graph evaluation should record its warning"
        );
        assert!(
            runner.hooks.finish_graph_evaluation_failed_called,
            "failed graph evaluation should return through the hook outcome"
        );
    }

    #[tokio::test]
    async fn frame_parser_module_graph_failure_returns_hook_outcome() {
        let hooks =
            FakeFrameModuleScriptHooks::graph_failure(DocumentScriptExecutionOutcome::NoProgress);
        let mut runner = FrameModuleScriptDocumentScriptRunner::new(hooks);

        assert_eq!(
            runner
                .run_document_script_work(FrameDocumentModuleScriptReadyWork::GraphFailed(
                    module_script_graph_failed_work()
                ))
                .await
                .into_output(),
            DocumentScriptExecutionOutcome::NoProgress
        );
        assert!(
            runner
                .hooks
                .dispatch_graph_failure_script_element_event_called,
            "graph failure should dispatch the error event before returning the hook outcome"
        );
        assert!(
            runner.hooks.record_runtime_warning_called,
            "graph failure should record its warning before returning the hook outcome"
        );
    }

    #[tokio::test]
    async fn frame_parser_module_rejected_evaluation_returns_hook_outcome() {
        let hooks = FakeFrameModuleScriptHooks::evaluation_rejected(
            DocumentScriptExecutionOutcome::NoProgress,
        );
        let mut runner = FrameModuleScriptDocumentScriptRunner::new(hooks);

        assert_eq!(
            runner
                .run_document_script_work(FrameDocumentModuleScriptReadyWork::EvaluationCompleted(
                    rejected_parser_module_evaluation()
                ))
                .await
                .into_output(),
            DocumentScriptExecutionOutcome::NoProgress
        );
        assert!(
            !runner.hooks.dispatch_script_element_event_called,
            "TLA rejection is reported as an evaluation exception, not a script element error"
        );
        assert!(
            runner.hooks.record_runtime_warning_called,
            "rejected evaluation should record its warning before returning the hook outcome"
        );
        assert!(
            runner.hooks.finish_evaluation_rejected_called,
            "rejected evaluation should return through the hook outcome"
        );
    }

    #[tokio::test]
    async fn frame_parser_module_pending_evaluation_completion_returns_hook_outcome() {
        let hooks = FakeFrameModuleScriptHooks::evaluation_pending(
            DocumentScriptExecutionOutcome::NoProgress,
        );
        let mut runner = FrameModuleScriptDocumentScriptRunner::new(hooks);

        assert_eq!(
            runner
                .run_document_script_work(FrameDocumentModuleScriptReadyWork::EvaluationCompleted(
                    pending_parser_module_evaluation()
                ))
                .await
                .into_output(),
            DocumentScriptExecutionOutcome::NoProgress
        );
        assert!(
            runner.hooks.finish_evaluation_pending_called,
            "pending evaluation completion should return through the hook outcome"
        );
        assert!(
            !runner.hooks.dispatch_script_element_event_called,
            "pending evaluation completion should not dispatch script events"
        );
        assert!(
            !runner.hooks.record_runtime_warning_called,
            "pending evaluation completion should not record runtime warnings"
        );
    }
}
