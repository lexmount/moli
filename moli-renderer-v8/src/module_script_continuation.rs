use std::collections::HashMap;
use std::fmt;

use moli_module_script_tree as module_tree;

use crate::document_script_scheduler::{
    DocumentOwnedScriptReadyAction, DocumentScriptReadyActionDispatchRoute,
    DocumentScriptReadyActionRoute, DocumentScriptReadyWork, DocumentScriptSchedulerStore,
    MainDocumentReadyActionRoute, ModuleScriptGraphReadyWork,
    ParserDeferredClassicSourceLoadCompletion, ParserPendingScriptId, ParserPendingScriptRoute,
};
use crate::dom::NodeId;
use crate::dynamic_script_owner::DynamicScriptOwnerId;
use crate::frame_owner_model::{FrameDocumentTaskOwner, MainDocumentScriptLoadDelayLease};
use crate::planning::PreparedScript;
use crate::types::ScriptErrorConstructorKind;

pub(crate) struct ModuleScriptContinuation {
    pub(crate) script: PreparedScript,
    owner: ModuleScriptContinuationOwner,
    active_fetch_load_id: Option<u64>,
    resumed_graph_job: Option<crate::module_runtime::NativeModuleGraphJob>,
    pub(crate) completed_graph: Option<crate::module_runtime::ModuleGraphHandle>,
    main_document_load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModuleScriptContinuationOwner {
    Parser(ParserPendingScriptId<MainParserDocumentOwner>),
    Runtime {
        owner_id: DynamicScriptOwnerId,
        document_owner: FrameDocumentTaskOwner,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct MainParserDocumentOwner {
    task_owner: FrameDocumentTaskOwner,
}

impl MainParserDocumentOwner {
    pub(crate) fn new(task_owner: FrameDocumentTaskOwner) -> Self {
        Self { task_owner }
    }

    pub(crate) fn task_owner(self) -> FrameDocumentTaskOwner {
        self.task_owner
    }
}

#[derive(Debug)]
pub(crate) struct MainDocumentModuleGraphReadyTarget {
    pending_script_id: ParserPendingScriptId<MainParserDocumentOwner>,
    main_document_load_delay_binding: Option<MainDocumentScriptLoadDelayLease>,
}

pub(crate) type MainDocumentModuleGraphReadyWork =
    ModuleScriptGraphReadyWork<MainDocumentModuleGraphReadyTarget>;
pub(crate) type MainParserOwnedModuleScriptContinuation =
    DocumentOwnedScriptReadyAction<MainParserDocumentOwner, ModuleScriptContinuation>;
pub(crate) type MainParserOwnedModuleScriptEvaluation =
    DocumentOwnedScriptReadyAction<MainParserDocumentOwner, ModuleScriptEvaluationContinuation>;
pub(crate) type MainParserOwnedModuleScriptFailure =
    DocumentOwnedScriptReadyAction<MainParserDocumentOwner, ParserModuleScriptFailure>;
pub(crate) type MainDocumentScriptSchedulerStore = DocumentScriptSchedulerStore<
    MainParserDocumentOwner,
    MainDocumentModuleGraphReadyTarget,
    MainParserOwnedModuleScriptEvaluation,
    MainParserOwnedModuleScriptFailure,
    MainParserOwnedModuleScriptContinuation,
>;
pub(crate) type MainParserDeferredClassicSourceLoadCompletion =
    ParserDeferredClassicSourceLoadCompletion<MainParserDocumentOwner>;

pub(crate) type MainParserOwnedDocumentScriptWork = DocumentScriptReadyWork<
    MainDocumentModuleGraphReadyTarget,
    MainParserOwnedModuleScriptEvaluation,
    MainParserOwnedModuleScriptFailure,
>;

impl MainDocumentModuleGraphReadyTarget {
    pub(crate) fn owner(&self) -> MainParserDocumentOwner {
        self.pending_script_id.owner()
    }

    pub(crate) fn pending_script_id(&self) -> ParserPendingScriptId<MainParserDocumentOwner> {
        self.pending_script_id
    }

    pub(crate) fn take_main_document_load_delay_binding(
        &mut self,
    ) -> Option<MainDocumentScriptLoadDelayLease> {
        self.main_document_load_delay_binding.take()
    }
}

impl ParserPendingScriptRoute<MainParserDocumentOwner> for MainDocumentModuleGraphReadyWork {
    fn parser_pending_script_id(&self) -> ParserPendingScriptId<MainParserDocumentOwner> {
        self.target().pending_script_id()
    }
}

impl fmt::Debug for ModuleScriptContinuation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModuleScriptContinuation")
            .field("script", &self.script)
            .field("owner", &self.owner)
            .field("active_fetch_load_id", &self.active_fetch_load_id)
            .field("has_resumed_graph_job", &self.resumed_graph_job.is_some())
            .field("has_completed_graph", &self.completed_graph.is_some())
            .field(
                "main_document_load_delay_binding",
                &self.main_document_load_delay_binding,
            )
            .finish()
    }
}

pub(crate) enum ModuleScriptContinuationGraphAdvance {
    Ready(Box<ModuleScriptContinuation>),
    NeedFetches {
        continuation: Box<ModuleScriptContinuation>,
        job: Box<crate::module_runtime::NativeModuleGraphJob>,
        fetches: Vec<crate::module_runtime::ModuleScriptGraphFetchContinuation>,
    },
    Failed {
        continuation: Box<ModuleScriptContinuation>,
        error: crate::module_runtime::ModuleLoadError,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModuleScriptCompletionOwner {
    /// Parser-created module/defer work.
    ///
    /// Use this for scripts discovered by the document parser, such as
    /// `<script type=module>` and parser-owned deferred script work. Their
    /// completion is DCL-critical and belongs to the parser lifecycle lane,
    /// matching Chromium's HTMLParserScriptRunner / pending-defer-script
    /// ownership.
    Parser,
    /// Runtime-created module work.
    ///
    /// Use this for scripts that are scheduled after parser discovery, such as
    /// async/in-order dynamic script owner work. Their observable load/error
    /// completion belongs to the runtime script owner, matching Chromium's
    /// ScriptRunner ownership. The runtime owner owns graph/evaluation
    /// continuation payloads; PageVm only executes the owner-selected work and
    /// bridges resource completions back to the owner.
    Runtime,
}

impl ModuleScriptCompletionOwner {
    pub(crate) fn is_runtime_owned(self) -> bool {
        matches!(self, Self::Runtime)
    }
}

impl ModuleScriptContinuation {
    pub(crate) fn new_parser(
        script: PreparedScript,
        pending_script_id: ParserPendingScriptId<MainParserDocumentOwner>,
    ) -> Self {
        debug_assert_eq!(pending_script_id.script_node_id(), script.node_id);
        debug_assert_eq!(pending_script_id.parser_position(), script.position);
        Self {
            script,
            owner: ModuleScriptContinuationOwner::Parser(pending_script_id),
            active_fetch_load_id: None,
            resumed_graph_job: None,
            completed_graph: None,
            main_document_load_delay_binding: None,
        }
    }

    pub(crate) fn new_runtime(
        script: PreparedScript,
        owner_id: DynamicScriptOwnerId,
        document_owner: FrameDocumentTaskOwner,
    ) -> Self {
        Self {
            script,
            owner: ModuleScriptContinuationOwner::Runtime {
                owner_id,
                document_owner,
            },
            active_fetch_load_id: None,
            resumed_graph_job: None,
            completed_graph: None,
            main_document_load_delay_binding: None,
        }
    }

    pub(crate) fn with_main_document_load_delay_binding(
        mut self,
        binding: MainDocumentScriptLoadDelayLease,
    ) -> Self {
        debug_assert!(
            self.main_document_load_delay_binding.is_none(),
            "module script continuation must bind document lifecycle ownership once"
        );
        self.main_document_load_delay_binding = Some(binding);
        self
    }

    pub(crate) fn take_main_document_load_delay_binding(
        &mut self,
    ) -> Option<MainDocumentScriptLoadDelayLease> {
        self.main_document_load_delay_binding.take()
    }

    pub(crate) fn completion_owner(&self) -> ModuleScriptCompletionOwner {
        match self.owner {
            ModuleScriptContinuationOwner::Parser(_) => ModuleScriptCompletionOwner::Parser,
            ModuleScriptContinuationOwner::Runtime { .. } => ModuleScriptCompletionOwner::Runtime,
        }
    }

    pub(crate) fn parser_document_owner(&self) -> Option<MainParserDocumentOwner> {
        match self.owner {
            ModuleScriptContinuationOwner::Parser(pending_script_id) => {
                Some(pending_script_id.owner())
            }
            ModuleScriptContinuationOwner::Runtime { .. } => None,
        }
    }

    pub(crate) fn parser_pending_script_id(
        &self,
    ) -> Option<ParserPendingScriptId<MainParserDocumentOwner>> {
        match self.owner {
            ModuleScriptContinuationOwner::Parser(pending_script_id) => Some(pending_script_id),
            ModuleScriptContinuationOwner::Runtime { .. } => None,
        }
    }

    pub(crate) fn dynamic_script_owner_id(&self) -> Option<DynamicScriptOwnerId> {
        match self.owner {
            ModuleScriptContinuationOwner::Parser(_) => None,
            ModuleScriptContinuationOwner::Runtime { owner_id, .. } => Some(owner_id),
        }
    }

    pub(crate) fn document_owner(&self) -> FrameDocumentTaskOwner {
        match self.owner {
            ModuleScriptContinuationOwner::Parser(pending_script_id) => {
                pending_script_id.owner().task_owner()
            }
            ModuleScriptContinuationOwner::Runtime { document_owner, .. } => document_owner,
        }
    }

    pub(crate) fn with_resumed_graph_job(
        mut self,
        job: crate::module_runtime::NativeModuleGraphJob,
    ) -> Self {
        debug_assert!(
            self.resumed_graph_job.is_none(),
            "module script continuation should install only one active graph job"
        );
        self.resumed_graph_job = Some(job);
        self
    }

    pub(crate) fn with_completed_graph(
        mut self,
        graph: crate::module_runtime::ModuleGraphHandle,
    ) -> Self {
        debug_assert!(
            self.resumed_graph_job.is_none(),
            "completed module script continuation should not also own an active graph job"
        );
        self.completed_graph = Some(graph);
        self
    }

    pub(crate) fn into_main_document_graph_ready_work(
        mut self,
    ) -> MainDocumentModuleGraphReadyWork {
        let graph = self
            .completed_graph
            .take()
            .expect("parser-owned ready module script should carry a completed graph");
        let target = MainDocumentModuleGraphReadyTarget {
            pending_script_id: self
                .parser_pending_script_id()
                .expect("main graph-ready work requires a parser pending script id"),
            main_document_load_delay_binding: self.main_document_load_delay_binding,
        };
        MainDocumentModuleGraphReadyWork::with_target(target, self.script, graph)
    }

    pub(crate) fn from_main_document_graph_ready_work(
        work: MainDocumentModuleGraphReadyWork,
    ) -> Self {
        let (mut target, script, graph) = work.into_parts();
        let mut continuation =
            ModuleScriptContinuation::new_parser(script, target.pending_script_id());
        if let Some(binding) = target.take_main_document_load_delay_binding() {
            continuation = continuation.with_main_document_load_delay_binding(binding);
        }
        continuation.with_completed_graph(graph)
    }

    pub(crate) fn active_fetch_load_id(&self) -> Option<u64> {
        self.active_fetch_load_id
    }

    fn take_resumed_graph_job(&mut self) -> Option<crate::module_runtime::NativeModuleGraphJob> {
        self.resumed_graph_job.take()
    }

    fn set_resumed_graph_job(&mut self, job: crate::module_runtime::NativeModuleGraphJob) {
        debug_assert!(
            self.resumed_graph_job.is_none(),
            "module script continuation should restore into an empty graph job slot"
        );
        self.resumed_graph_job = Some(job);
    }

    pub(crate) fn with_pending_graph_fetches(
        mut self,
        job: crate::module_runtime::NativeModuleGraphJob,
        active_fetch_load_id: Option<u64>,
    ) -> Self {
        self.active_fetch_load_id = active_fetch_load_id;
        self.set_resumed_graph_job(job);
        self
    }

    pub(crate) fn advance_graph(
        mut self,
        vm: &mut crate::script_vm::ScriptVm,
    ) -> ModuleScriptContinuationGraphAdvance {
        let Some(job) = self.take_resumed_graph_job() else {
            return ModuleScriptContinuationGraphAdvance::Ready(Box::new(self));
        };
        match crate::module_runtime::advance_module_script_graph(vm, job) {
            Ok(crate::module_runtime::ModuleScriptGraphAdvance::NeedFetches(fetch_batch)) => {
                let (job, fetches) = fetch_batch.into_parts();
                ModuleScriptContinuationGraphAdvance::NeedFetches {
                    continuation: Box::new(self),
                    job: Box::new(job),
                    fetches,
                }
            }
            Ok(crate::module_runtime::ModuleScriptGraphAdvance::Complete(graph)) => {
                self.active_fetch_load_id = None;
                self.completed_graph = Some(graph);
                ModuleScriptContinuationGraphAdvance::Ready(Box::new(self))
            }
            Err(error) => ModuleScriptContinuationGraphAdvance::Failed {
                continuation: Box::new(self),
                error,
            },
        }
    }

    pub(crate) fn finish_fetch_into_resumed_graph(
        mut self,
        vm: &mut crate::script_vm::ScriptVm,
        graph_continuation: crate::module_runtime::ModuleScriptGraphFetchContinuation,
        source: std::result::Result<
            crate::module_runtime::ModuleGraphFetchedSource,
            crate::module_runtime::ModuleLoadError,
        >,
    ) -> ModuleScriptContinuationGraphAdvance {
        let Some(mut active_job) = self.resumed_graph_job.take() else {
            return ModuleScriptContinuationGraphAdvance::Failed {
                continuation: Box::new(self),
                error: crate::module_runtime::ModuleLoadError::new(
                    crate::module_runtime::ModuleLoadStage::Fetch,
                    "module script continuation did not own its graph job",
                ),
            };
        };
        let top_level_fetch_failure =
            source.is_err() && graph_continuation.is_top_level_tree_fetch();
        match graph_continuation.finish_fetch_into_job(vm, &mut active_job, source) {
            Ok(advance) => self.finish_graph_advance(active_job, advance),
            Err(error) => {
                let error = if top_level_fetch_failure {
                    error.with_top_level_module_load_failure()
                } else {
                    error
                };
                ModuleScriptContinuationGraphAdvance::Failed {
                    continuation: Box::new(self),
                    error,
                }
            }
        }
    }

    pub(crate) fn finish_joined_fetch_into_resumed_graph(
        mut self,
        vm: &mut crate::script_vm::ScriptVm,
        key: module_tree::ModuleMapKey,
        client: module_tree::SingleModuleClientToken,
    ) -> ModuleScriptContinuationGraphAdvance {
        let Some(mut active_job) = self.resumed_graph_job.take() else {
            return ModuleScriptContinuationGraphAdvance::Failed {
                continuation: Box::new(self),
                error: crate::module_runtime::ModuleLoadError::new(
                    crate::module_runtime::ModuleLoadStage::Fetch,
                    "module script joined fetch continuation did not own its graph job",
                ),
            };
        };
        match active_job.finish_joined_module_map_fetch(vm, key, client) {
            Ok(advance) => self.finish_graph_advance(active_job, advance),
            Err(error) => ModuleScriptContinuationGraphAdvance::Failed {
                continuation: Box::new(self),
                error,
            },
        }
    }

    fn finish_graph_advance(
        mut self,
        active_job: crate::module_runtime::NativeModuleGraphJob,
        advance: crate::module_runtime::NativeModuleGraphJobAdvance,
    ) -> ModuleScriptContinuationGraphAdvance {
        match crate::module_runtime::module_script_graph_advance_from_native(active_job, advance) {
            crate::module_runtime::ModuleScriptGraphAdvance::NeedFetches(fetch_batch) => {
                let (job, fetches) = fetch_batch.into_parts();
                ModuleScriptContinuationGraphAdvance::NeedFetches {
                    continuation: Box::new(self),
                    job: Box::new(job),
                    fetches,
                }
            }
            crate::module_runtime::ModuleScriptGraphAdvance::Complete(graph) => {
                self.active_fetch_load_id = None;
                self.completed_graph = Some(graph);
                ModuleScriptContinuationGraphAdvance::Ready(Box::new(self))
            }
        }
    }
}

impl DocumentScriptReadyActionDispatchRoute<MainDocumentReadyActionRoute>
    for ModuleScriptContinuation
{
    fn dispatch_route(&self) -> MainDocumentReadyActionRoute {
        MainDocumentReadyActionRoute::new(self.script.node_id)
    }
}

impl From<MainDocumentModuleGraphReadyWork> for Box<MainParserOwnedModuleScriptContinuation> {
    fn from(work: MainDocumentModuleGraphReadyWork) -> Self {
        let (mut target, script, graph) = work.into_parts();
        let mut continuation =
            ModuleScriptContinuation::new_parser(script, target.pending_script_id());
        if let Some(binding) = target.take_main_document_load_delay_binding() {
            continuation = continuation.with_main_document_load_delay_binding(binding);
        }
        let continuation = continuation.with_completed_graph(graph);
        Box::new(DocumentOwnedScriptReadyAction::new(
            target.owner(),
            continuation,
        ))
    }
}

impl DocumentScriptReadyActionRoute<MainParserDocumentOwner> for MainDocumentModuleGraphReadyWork {
    fn payload_document_owner(&self) -> MainParserDocumentOwner {
        self.target().owner()
    }
}

impl DocumentScriptReadyActionDispatchRoute<MainDocumentReadyActionRoute>
    for MainDocumentModuleGraphReadyWork
{
    fn dispatch_route(&self) -> MainDocumentReadyActionRoute {
        MainDocumentReadyActionRoute::new(self.script().node_id)
    }
}

pub(crate) struct ModuleScriptEvaluationContinuation {
    pub(crate) script_continuation: ModuleScriptContinuation,
    pub(crate) root_entry: crate::module_runtime::ModuleEntryId,
    pub(crate) reaction_id: u64,
    pub(crate) reaction_state: ModuleScriptEvaluationReactionState,
    pub(crate) completion_applied_at_evaluation_start: bool,
}

impl DocumentScriptReadyActionDispatchRoute<MainDocumentReadyActionRoute>
    for ModuleScriptEvaluationContinuation
{
    fn dispatch_route(&self) -> MainDocumentReadyActionRoute {
        self.script_continuation.dispatch_route()
    }
}

impl fmt::Debug for ModuleScriptEvaluationContinuation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModuleScriptEvaluationContinuation")
            .field("script_continuation", &self.script_continuation)
            .field("root_entry", &self.root_entry)
            .field("reaction_id", &self.reaction_id)
            .field("reaction_state", &self.reaction_state)
            .field(
                "completion_applied_at_evaluation_start",
                &self.completion_applied_at_evaluation_start,
            )
            .finish()
    }
}

pub(crate) struct ModuleScriptEvaluationUpdate {
    pub(crate) root_entry: crate::module_runtime::ModuleEntryId,
}

#[derive(Debug)]
pub(crate) struct ParserModuleScriptFailure {
    pub(crate) continuation: ModuleScriptContinuation,
    pub(crate) error: crate::module_runtime::ModuleLoadError,
}

impl DocumentScriptReadyActionDispatchRoute<MainDocumentReadyActionRoute>
    for ParserModuleScriptFailure
{
    fn dispatch_route(&self) -> MainDocumentReadyActionRoute {
        self.continuation.dispatch_route()
    }
}

impl ParserPendingScriptRoute<MainParserDocumentOwner> for MainParserOwnedModuleScriptFailure {
    fn parser_pending_script_id(&self) -> ParserPendingScriptId<MainParserDocumentOwner> {
        self.action()
            .continuation
            .parser_pending_script_id()
            .expect("main parser module failure must retain its PendingScript id")
    }
}

pub(crate) type ModuleScriptGraphResumeResult = ModuleScriptContinuationGraphAdvance;

#[derive(Default)]
pub(crate) struct NativeModuleOwnerActions {
    ready_module_scripts: Vec<ModuleScriptContinuation>,
    ready_module_evaluations: Vec<ModuleScriptEvaluationContinuation>,
    runtime_module_failures: Vec<(
        ModuleScriptContinuation,
        crate::module_runtime::ModuleLoadError,
    )>,
}

impl NativeModuleOwnerActions {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn from_runtime_module_failure(
        continuation: ModuleScriptContinuation,
        error: crate::module_runtime::ModuleLoadError,
    ) -> Self {
        let mut actions = Self::empty();
        actions.runtime_module_failures.push((continuation, error));
        actions
    }

    pub(crate) fn from_ready_module_script(continuation: ModuleScriptContinuation) -> Self {
        let mut actions = Self::empty();
        actions.push_ready_module_script(continuation);
        actions
    }

    pub(crate) fn push_ready_module_script(&mut self, continuation: ModuleScriptContinuation) {
        self.ready_module_scripts.push(continuation);
    }

    pub(crate) fn push_ready_module_evaluation(
        &mut self,
        evaluation: ModuleScriptEvaluationContinuation,
    ) {
        self.ready_module_evaluations.push(evaluation);
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.ready_module_scripts.extend(other.ready_module_scripts);
        self.ready_module_evaluations
            .extend(other.ready_module_evaluations);
        self.runtime_module_failures
            .extend(other.runtime_module_failures);
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<ModuleScriptContinuation>,
        Vec<ModuleScriptEvaluationContinuation>,
        Vec<(
            ModuleScriptContinuation,
            crate::module_runtime::ModuleLoadError,
        )>,
    ) {
        (
            self.ready_module_scripts,
            self.ready_module_evaluations,
            self.runtime_module_failures,
        )
    }

    pub(crate) fn into_runtime_module_failures(
        self,
    ) -> Vec<(
        ModuleScriptContinuation,
        crate::module_runtime::ModuleLoadError,
    )> {
        self.runtime_module_failures
    }
}

#[derive(Default)]
pub(crate) struct NativeDynamicModuleTerminalFanout {
    ready_imports: Vec<crate::module_runtime::NativeDynamicModuleImportReady>,
    scheduled_dynamic_import_fetches: Vec<crate::module_runtime::DynamicModuleScheduledFetch>,
    failed_fetches: Vec<crate::module_runtime::DynamicModuleFetchFailure>,
    graph_advance_failures: Vec<(
        crate::module_runtime::NativeModuleGraphJob,
        crate::module_runtime::ModuleLoadError,
    )>,
    restored_after_unexpected_complete: bool,
}

impl NativeDynamicModuleTerminalFanout {
    pub(crate) fn from_owner_advance(
        advance: crate::module_runtime::DynamicModuleFetchOwnerAdvance,
    ) -> Self {
        let mut fanout = Self::default();
        fanout.absorb_owner_advance(advance);
        fanout
    }

    pub(crate) fn absorb_owner_advance(
        &mut self,
        advance: crate::module_runtime::DynamicModuleFetchOwnerAdvance,
    ) {
        match advance {
            crate::module_runtime::DynamicModuleFetchOwnerAdvance::Waiting { scheduled_fetches } => {
                self.extend_scheduled_dynamic_import_fetches(scheduled_fetches);
            }
            crate::module_runtime::DynamicModuleFetchOwnerAdvance::Ready(dynamic_import) => {
                self.push_ready_import(*dynamic_import);
            }
            crate::module_runtime::DynamicModuleFetchOwnerAdvance::RestoredAfterUnexpectedComplete => {
                self.restored_after_unexpected_complete = true;
            }
        }
    }

    pub(crate) fn push_ready_import(
        &mut self,
        dynamic_import: crate::module_runtime::NativeDynamicModuleImportReady,
    ) {
        self.ready_imports.push(dynamic_import);
    }

    pub(crate) fn extend_scheduled_dynamic_import_fetches(
        &mut self,
        scheduled: Vec<crate::module_runtime::DynamicModuleScheduledFetch>,
    ) {
        self.scheduled_dynamic_import_fetches.extend(scheduled);
    }

    pub(crate) fn push_failed_fetch(
        &mut self,
        failure: crate::module_runtime::DynamicModuleFetchFailure,
    ) {
        self.failed_fetches.push(failure);
    }

    pub(crate) fn push_graph_advance_failure(
        &mut self,
        job: crate::module_runtime::NativeModuleGraphJob,
        error: crate::module_runtime::ModuleLoadError,
    ) {
        self.graph_advance_failures.push((job, error));
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<crate::module_runtime::NativeDynamicModuleImportReady>,
        Vec<crate::module_runtime::DynamicModuleScheduledFetch>,
        Vec<crate::module_runtime::DynamicModuleFetchFailure>,
        Vec<(
            crate::module_runtime::NativeModuleGraphJob,
            crate::module_runtime::ModuleLoadError,
        )>,
        bool,
    ) {
        (
            self.ready_imports,
            self.scheduled_dynamic_import_fetches,
            self.failed_fetches,
            self.graph_advance_failures,
            self.restored_after_unexpected_complete,
        )
    }
}

#[derive(Default)]
pub(crate) struct ModuleMapTerminalFanout {
    module_script_results: Vec<ModuleScriptGraphResumeResult>,
    dynamic_imports: NativeDynamicModuleTerminalFanout,
}

impl ModuleMapTerminalFanout {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn push_module_script_result(&mut self, result: ModuleScriptGraphResumeResult) {
        self.module_script_results.push(result);
    }

    pub(crate) fn absorb_dynamic_import_owner_advance(
        &mut self,
        advance: crate::module_runtime::DynamicModuleFetchOwnerAdvance,
    ) {
        self.dynamic_imports.absorb_owner_advance(advance);
    }

    pub(crate) fn push_dynamic_import_fetch_failure(
        &mut self,
        failure: crate::module_runtime::DynamicModuleFetchFailure,
    ) {
        self.dynamic_imports.push_failed_fetch(failure);
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<ModuleScriptGraphResumeResult>,
        NativeDynamicModuleTerminalFanout,
    ) {
        (self.module_script_results, self.dynamic_imports)
    }
}

pub(crate) enum ModuleScriptGraphFetchResume {
    Finished {
        result: Box<ModuleScriptGraphResumeResult>,
    },
    RestoredMissingGraphContinuation,
}

impl ModuleScriptGraphFetchResume {
    pub(crate) fn finished(result: ModuleScriptGraphResumeResult) -> Self {
        Self::Finished {
            result: Box::new(result),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ParserPendingModuleScriptActiveTree {
    continuation: ModuleScriptContinuation,
}

impl ParserPendingModuleScriptActiveTree {
    fn new(continuation: ModuleScriptContinuation) -> Self {
        assert_eq!(
            continuation.completion_owner(),
            ModuleScriptCompletionOwner::Parser,
            "only parser-owned module graph continuations belong to ParserPendingModuleScript"
        );
        Self { continuation }
    }

    pub(crate) fn into_continuation(self) -> ModuleScriptContinuation {
        self.continuation
    }

    pub(crate) fn finish_fetch_into_graph(
        self,
        vm: &mut crate::script_vm::ScriptVm,
        graph_continuation: crate::module_runtime::ModuleScriptGraphFetchContinuation,
        source: std::result::Result<
            crate::module_runtime::ModuleGraphFetchedSource,
            crate::module_runtime::ModuleLoadError,
        >,
    ) -> ModuleScriptGraphResumeResult {
        self.continuation
            .finish_fetch_into_resumed_graph(vm, graph_continuation, source)
    }

    pub(crate) fn finish_joined_fetch_into_graph(
        self,
        vm: &mut crate::script_vm::ScriptVm,
        key: module_tree::ModuleMapKey,
        client: module_tree::SingleModuleClientToken,
    ) -> ModuleScriptGraphResumeResult {
        self.continuation
            .finish_joined_fetch_into_resumed_graph(vm, key, client)
    }
}

#[derive(Debug)]
struct ParserModuleGraphContinuation {
    active_tree: Option<ModuleScriptContinuation>,
}

impl ParserModuleGraphContinuation {
    fn new() -> Self {
        Self { active_tree: None }
    }

    fn install_active_tree(&mut self, continuation: ModuleScriptContinuation) {
        debug_assert!(
            self.active_tree.is_none(),
            "parser module graph continuation should own only one active tree"
        );
        self.active_tree = Some(continuation);
    }

    fn take_active_tree(&mut self) -> Option<ModuleScriptContinuation> {
        self.active_tree.take()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ModuleScriptEvaluationReactionState {
    Pending,
    Fulfilled,
    Rejected {
        reason: String,
        error_constructor: Option<ScriptErrorConstructorKind>,
    },
}

impl ModuleScriptEvaluationReactionState {
    pub(crate) fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }
}

#[derive(Debug, Default)]
pub(crate) struct ModuleScriptContinuationStore {
    parser: ModuleScriptOwnerContinuations,
}

#[derive(Debug, Default)]
struct ModuleScriptOwnerContinuations {
    pending_fetches: HashMap<u64, NodeId>,
    pending_joined_clients: HashMap<module_tree::SingleModuleClientToken, NodeId>,
    active_graphs: HashMap<NodeId, ParserModuleGraphContinuation>,
}

impl ModuleScriptContinuationStore {
    pub(crate) fn clear_for_document_replacement(&mut self) {
        self.parser = ModuleScriptOwnerContinuations::default();
    }

    pub(crate) fn insert_parser_pending(
        &mut self,
        load_id: u64,
        continuation: ModuleScriptContinuation,
    ) {
        self.insert_parser_pending_fetches(std::iter::once(load_id), continuation);
    }

    pub(crate) fn insert_parser_pending_fetches(
        &mut self,
        load_ids: impl IntoIterator<Item = u64>,
        continuation: ModuleScriptContinuation,
    ) {
        self.insert_parser_pending_waits(
            load_ids,
            std::iter::empty::<module_tree::SingleModuleClientToken>(),
            continuation,
        );
    }

    pub(crate) fn insert_parser_pending_waits(
        &mut self,
        load_ids: impl IntoIterator<Item = u64>,
        joined_clients: impl IntoIterator<Item = module_tree::SingleModuleClientToken>,
        continuation: ModuleScriptContinuation,
    ) {
        assert_eq!(
            continuation.completion_owner(),
            ModuleScriptCompletionOwner::Parser,
            "runtime-owned pending graph continuations should be owned by DynamicScriptOwner"
        );
        let node_id = continuation.script.node_id;
        self.parser
            .active_graphs
            .entry(node_id)
            .or_insert_with(ParserModuleGraphContinuation::new)
            .install_active_tree(continuation);
        for load_id in load_ids {
            self.parser.pending_fetches.insert(load_id, node_id);
        }
        for client in joined_clients {
            self.parser.pending_joined_clients.insert(client, node_id);
        }
    }

    pub(crate) fn take_active_tree_for_fetch_completion(
        &mut self,
        load_id: u64,
    ) -> Option<ParserPendingModuleScriptActiveTree> {
        let node_id = self.parser.pending_fetches.remove(&load_id)?;
        let mut graph = self.parser.active_graphs.remove(&node_id)?;
        let continuation = graph.take_active_tree()?;
        Some(ParserPendingModuleScriptActiveTree::new(continuation))
    }

    pub(crate) fn pending_script_id_for_fetch(
        &self,
        load_id: u64,
    ) -> Option<ParserPendingScriptId<MainParserDocumentOwner>> {
        let node_id = *self.parser.pending_fetches.get(&load_id)?;
        self.parser
            .active_graphs
            .get(&node_id)?
            .active_tree
            .as_ref()?
            .parser_pending_script_id()
            .filter(|pending_script_id| pending_script_id.script_node_id() == node_id)
    }

    pub(crate) fn take_active_tree_for_joined_client(
        &mut self,
        client: module_tree::SingleModuleClientToken,
    ) -> Option<ParserPendingModuleScriptActiveTree> {
        let node_id = self.parser.pending_joined_clients.remove(&client)?;
        let mut graph = self.parser.active_graphs.remove(&node_id)?;
        let continuation = graph.take_active_tree()?;
        Some(ParserPendingModuleScriptActiveTree::new(continuation))
    }

    pub(crate) fn restore_pending_continuation(&mut self, continuation: ModuleScriptContinuation) {
        assert_eq!(
            continuation.completion_owner(),
            ModuleScriptCompletionOwner::Parser,
            "runtime-owned pending graph continuations should be owned by DynamicScriptOwner"
        );
        let node_id = continuation.script.node_id;
        self.parser
            .active_graphs
            .entry(node_id)
            .or_insert_with(ParserModuleGraphContinuation::new)
            .install_active_tree(continuation);
    }

    pub(crate) fn restore_active_tree_for_fetch(
        &mut self,
        load_id: u64,
        active_tree: ParserPendingModuleScriptActiveTree,
    ) {
        self.insert_parser_pending(load_id, active_tree.into_continuation());
    }

    pub(crate) fn clear_pending_fetches_for_script(
        &mut self,
        node_id: NodeId,
    ) -> (Vec<u64>, Vec<module_tree::SingleModuleClientToken>) {
        let mut removed_load_ids = Vec::new();
        self.parser.pending_fetches.retain(|load_id, pending_node| {
            if *pending_node == node_id {
                removed_load_ids.push(*load_id);
                false
            } else {
                true
            }
        });
        let mut removed_joined_clients = Vec::new();
        self.parser
            .pending_joined_clients
            .retain(|client, pending_node| {
                if *pending_node == node_id {
                    removed_joined_clients.push(*client);
                    false
                } else {
                    true
                }
            });
        self.parser.active_graphs.remove(&node_id);
        (removed_load_ids, removed_joined_clients)
    }

    #[cfg(test)]
    pub(crate) fn has_pending_fetch(
        &self,
        ready_actions: &MainDocumentScriptSchedulerStore,
    ) -> bool {
        self.parser
            .active_graphs
            .values()
            .filter_map(|graph| graph.active_tree.as_ref())
            .any(|continuation| {
                continuation
                    .parser_pending_script_id()
                    .is_some_and(|pending_script_id| {
                        ready_actions.module_script_is_watching_for_test(pending_script_id)
                    })
            })
    }
}

pub(crate) fn parser_module_evaluation_continuation_into_ready_action(
    evaluation: crate::parser_module_evaluation::ParserModuleEvaluationContinuation<
        MainParserOwnedModuleScriptContinuation,
    >,
) -> MainParserOwnedModuleScriptEvaluation {
    let (owned_script_continuation, root_entry, reaction_id, reaction_state) =
        evaluation.into_parts();
    let owner = *owned_script_continuation.owner();
    DocumentOwnedScriptReadyAction::new(
        owner,
        ModuleScriptEvaluationContinuation {
            script_continuation: owned_script_continuation.into_action(),
            root_entry,
            reaction_id,
            reaction_state,
            completion_applied_at_evaluation_start: true,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_script_scheduler::DocumentScriptReadyWork;
    use crate::module_runtime::{
        DynamicModuleFetchFailure, DynamicModuleScheduledFetch, ModuleAttributesKey, ModuleEntryId,
        ModuleFetchMetadata, ModuleGraphHandle, ModuleImportPhase, ModuleKind, ModuleLoadError,
        ModuleLoadStage, NativeDynamicModuleImportReady, NativeModuleGraphFetchRequest,
        NativeModuleGraphJob, PendingDynamicModuleImport,
    };
    use crate::planning::{ScriptFetchMetadata, ScriptSource};
    use crate::types::{ScriptKind, ScriptMode, ScriptSourceKind};
    use url::Url;

    fn main_task_owner() -> FrameDocumentTaskOwner {
        FrameDocumentTaskOwner::new(
            crate::frame_owner_model::FrameSchedulerLaneId(0),
            crate::frame_owner_model::LocalWindowId(0),
            crate::frame_owner_model::DocumentId(0),
        )
    }

    fn main_parser_document_owner() -> MainParserDocumentOwner {
        MainParserDocumentOwner::new(main_task_owner())
    }

    fn pending_dynamic_module_import() -> PendingDynamicModuleImport {
        let _js_runtime = crate::JsRuntime::initialize();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
        PendingDynamicModuleImport::new(
            v8::Global::new(scope, scope.get_current_context()),
            v8::Global::new(scope, resolver),
            crate::module_runtime::DynamicModuleImportOwner::main_for_test_parts(0, 0, 0),
            "./dynamic.mjs",
            Url::parse("https://module-terminal-fanout.test/page.html").expect("page url"),
            ModuleAttributesKey::empty(),
            ModuleImportPhase::Evaluation,
        )
    }

    fn dynamic_fetch_request() -> NativeModuleGraphFetchRequest {
        let base_url = Url::parse("https://module-terminal-fanout.test/app/").expect("base url");
        NativeModuleGraphFetchRequest::new_for_test(
            base_url.join("dynamic.mjs").expect("dynamic URL"),
            base_url,
            ModuleFetchMetadata::default(),
            ModuleKind::JavaScript,
        )
    }

    fn prepared_module_script(node: u32) -> PreparedScript {
        PreparedScript {
            position: node as usize,
            node_id: NodeId::new(node as usize),
            kind: ScriptKind::Module,
            mode: ScriptMode::ModuleDefer,
            source_kind: ScriptSourceKind::External,
            fetch_metadata: ScriptFetchMetadata::default(),
            source: ScriptSource::External,
            url: Url::parse(&format!("https://example.test/{node}.mjs")).expect("module url"),
            base_url: Url::parse(&format!("https://example.test/{node}.mjs")).expect("module url"),
            initiator_url: Url::parse("https://example.test/page").expect("page url"),
            host_script_handle: None,
        }
    }

    fn parser_continuation(script: PreparedScript) -> ModuleScriptContinuation {
        let owner = MainParserDocumentOwner::new(main_task_owner());
        let pending_script_id = ParserPendingScriptId::new(owner, &script);
        ModuleScriptContinuation::new_parser(script, pending_script_id).with_completed_graph(
            crate::module_runtime::ModuleGraphHandle {
                root_entry: crate::module_runtime::ModuleEntryId::for_test(1),
                entries: vec![crate::module_runtime::ModuleEntryId::for_test(1)],
            },
        )
    }

    fn queue_parser_ready_completed(
        scheduler: &mut MainDocumentScriptSchedulerStore,
        owner: MainParserDocumentOwner,
        continuation: ModuleScriptContinuation,
    ) {
        assert_eq!(continuation.parser_document_owner(), Some(owner));
        let work = continuation.into_main_document_graph_ready_work();
        scheduler.notify_module_script_graph_ready_work(work);
    }

    fn queue_parser_ready_failure(
        scheduler: &mut MainDocumentScriptSchedulerStore,
        owner: MainParserDocumentOwner,
        failure: ParserModuleScriptFailure,
    ) {
        let payload_owner = failure
            .continuation
            .parser_document_owner()
            .expect("parser failure should retain its document owner");
        assert_eq!(payload_owner, owner);
        scheduler.notify_module_script_graph_failed_action(DocumentOwnedScriptReadyAction::new(
            payload_owner,
            failure,
        ));
    }

    fn queue_parser_evaluation(
        scheduler: &mut MainDocumentScriptSchedulerStore,
        owner: MainParserDocumentOwner,
        evaluation: ModuleScriptEvaluationContinuation,
    ) {
        let payload_owner = evaluation
            .script_continuation
            .parser_document_owner()
            .expect("parser evaluation should retain its document owner");
        assert_eq!(payload_owner, owner);
        if evaluation.reaction_state.is_pending() {
            scheduler.push_pending_parser_module_evaluation_with_reaction_id(
                DocumentOwnedScriptReadyAction::new(payload_owner, evaluation.script_continuation),
                evaluation.root_entry,
                evaluation.reaction_id,
            );
        } else {
            scheduler.notify_module_script_evaluation_completed(
                DocumentOwnedScriptReadyAction::new(payload_owner, evaluation),
            );
        }
    }

    #[test]
    fn module_map_terminal_fanout_separates_dynamic_import_outputs() {
        let mut fanout = ModuleMapTerminalFanout::empty();
        fanout.absorb_dynamic_import_owner_advance(
            crate::module_runtime::DynamicModuleFetchOwnerAdvance::Ready(Box::new(
                NativeDynamicModuleImportReady {
                    job: NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
                    graph: ModuleGraphHandle {
                        root_entry: ModuleEntryId::for_test(42),
                        entries: vec![ModuleEntryId::for_test(42)],
                    },
                },
            )),
        );
        fanout.absorb_dynamic_import_owner_advance(
            crate::module_runtime::DynamicModuleFetchOwnerAdvance::Waiting {
                scheduled_fetches: vec![DynamicModuleScheduledFetch::new(
                    7,
                    dynamic_fetch_request(),
                    None,
                )],
            },
        );

        let (module_script_results, dynamic_import_fanout) = fanout.into_parts();
        let (
            ready_imports,
            scheduled_fetches,
            failed_fetches,
            graph_advance_failures,
            restored_after_unexpected_complete,
        ) = dynamic_import_fanout.into_parts();

        assert!(
            module_script_results.is_empty(),
            "dynamic-import terminal outputs must not appear as module-script graph results"
        );
        assert_eq!(ready_imports.len(), 1);
        assert_eq!(scheduled_fetches.len(), 1);
        assert!(failed_fetches.is_empty());
        assert!(graph_advance_failures.is_empty());
        assert!(!restored_after_unexpected_complete);
    }

    #[test]
    fn native_dynamic_terminal_fanout_absorbs_owner_advances() {
        let mut fanout = NativeDynamicModuleTerminalFanout::default();
        fanout.absorb_owner_advance(
            crate::module_runtime::DynamicModuleFetchOwnerAdvance::Ready(Box::new(
                NativeDynamicModuleImportReady {
                    job: NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
                    graph: ModuleGraphHandle {
                        root_entry: ModuleEntryId::for_test(42),
                        entries: vec![ModuleEntryId::for_test(42)],
                    },
                },
            )),
        );
        fanout.absorb_owner_advance(
            crate::module_runtime::DynamicModuleFetchOwnerAdvance::Waiting {
                scheduled_fetches: vec![DynamicModuleScheduledFetch::new(
                    8,
                    dynamic_fetch_request(),
                    None,
                )],
            },
        );
        fanout.absorb_owner_advance(
            crate::module_runtime::DynamicModuleFetchOwnerAdvance::RestoredAfterUnexpectedComplete,
        );

        let (
            ready_imports,
            scheduled_fetches,
            failed_fetches,
            graph_advance_failures,
            restored_after_unexpected_complete,
        ) = fanout.into_parts();

        assert_eq!(ready_imports.len(), 1);
        assert_eq!(scheduled_fetches.len(), 1);
        assert!(failed_fetches.is_empty());
        assert!(graph_advance_failures.is_empty());
        assert!(restored_after_unexpected_complete);
    }

    #[test]
    fn native_dynamic_terminal_fanout_carries_fetch_failures() {
        let mut fanout = NativeDynamicModuleTerminalFanout::default();
        fanout.push_failed_fetch(DynamicModuleFetchFailure::for_test(
            pending_dynamic_module_import(),
            ModuleLoadError::new(ModuleLoadStage::Fetch, "forced dynamic import failure"),
        ));

        let (
            ready_imports,
            scheduled_fetches,
            failed_fetches,
            graph_advance_failures,
            restored_after_unexpected_complete,
        ) = fanout.into_parts();

        assert!(ready_imports.is_empty());
        assert!(scheduled_fetches.is_empty());
        assert_eq!(failed_fetches.len(), 1);
        assert!(graph_advance_failures.is_empty());
        assert!(!restored_after_unexpected_complete);
    }

    #[test]
    fn native_dynamic_terminal_fanout_carries_graph_advance_failures() {
        let mut fanout = NativeDynamicModuleTerminalFanout::default();
        fanout.push_graph_advance_failure(
            NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
            ModuleLoadError::new(ModuleLoadStage::Resolve, "forced graph advance failure"),
        );

        let (
            ready_imports,
            scheduled_fetches,
            failed_fetches,
            graph_advance_failures,
            restored_after_unexpected_complete,
        ) = fanout.into_parts();

        assert!(ready_imports.is_empty());
        assert!(scheduled_fetches.is_empty());
        assert!(failed_fetches.is_empty());
        assert_eq!(graph_advance_failures.len(), 1);
        assert!(!restored_after_unexpected_complete);
    }

    #[test]
    fn parser_pending_module_script_ready_waits_for_runner_watch() {
        let script = prepared_module_script(7);
        let mut scheduler = MainDocumentScriptSchedulerStore::default();
        let owner = main_parser_document_owner();
        let pending_script_id = scheduler.register_module_script(owner, &script);

        queue_parser_ready_completed(&mut scheduler, owner, parser_continuation(script.clone()));
        assert!(
            !scheduler.has_ready_work(),
            "unwatched pending module script must not enter the executable ready queue"
        );
        assert!(
            !scheduler.has_load_blocking_document_script_work(owner),
            "unwatched ready pending module script must not block the parser runner from reaching its page task"
        );

        let watch = scheduler.watch_module_script(pending_script_id);
        assert!(watch.watched());
        assert!(watch.queued_ready_work());
        let work = scheduler
            .take_next_ready_work()
            .expect("ready pending module script should enter the document ready queue")
            .into_module_script_graph_ready();
        assert_eq!(work.script().node_id, script.node_id);
    }

    #[test]
    fn watched_parser_async_module_script_completes_to_ready_queue_candidate() {
        let mut script = prepared_module_script(9);
        script.mode = ScriptMode::Async;
        let mut scheduler = MainDocumentScriptSchedulerStore::default();
        let owner = main_parser_document_owner();
        let pending_script_id = scheduler.register_module_script(owner, &script);

        let watch = scheduler.watch_module_script(pending_script_id);
        assert!(watch.watched());
        assert!(
            !watch.queued_ready_work(),
            "watching before graph completion should not enqueue executable work"
        );
        assert!(
            !scheduler.has_load_blocking_document_script_work(owner),
            "plain module watch must not invent parser-defer lifecycle ownership"
        );
        queue_parser_ready_completed(&mut scheduler, owner, parser_continuation(script.clone()));
        assert!(
            scheduler.has_ready_work(),
            "terminal graph completion should enqueue through the scheduler once watched"
        );
        let work = scheduler
            .take_next_ready_work()
            .expect("watched graph completion should enter the scheduler ready lane")
            .into_module_script_graph_ready();
        assert_eq!(work.script().node_id, script.node_id);
    }

    #[test]
    fn parser_ready_completed_uses_shared_scheduler_ready_lane() {
        let script = prepared_module_script(17);
        let mut scheduler = MainDocumentScriptSchedulerStore::default();
        let owner = main_parser_document_owner();
        let pending_script_id = scheduler.register_module_script(owner, &script);
        assert!(scheduler.watch_module_script(pending_script_id).watched());

        queue_parser_ready_completed(&mut scheduler, owner, parser_continuation(script.clone()));

        assert!(
            scheduler.has_ready_work(),
            "main parser-owned ready module script should enter the shared document scheduler ready lane"
        );
        let work = scheduler
            .take_next_ready_work()
            .expect("shared scheduler ready lane should return document ready work")
            .into_module_script_graph_ready();
        let continuation = ModuleScriptContinuation::from_main_document_graph_ready_work(work);
        assert_eq!(continuation.script.node_id, script.node_id);
        assert!(
            continuation.completed_graph.is_some(),
            "restored parser continuation should preserve the completed module graph"
        );
        assert!(!scheduler.has_ready_work());
    }

    #[test]
    fn main_parser_module_owned_ready_work_matches_payload_owner_and_route() {
        let graph_script = prepared_module_script(18);
        let mut graph_store = MainDocumentScriptSchedulerStore::default();
        let owner = main_parser_document_owner();
        let pending_script_id = graph_store.register_module_script(owner, &graph_script);
        assert!(graph_store.watch_module_script(pending_script_id).watched());

        queue_parser_ready_completed(
            &mut graph_store,
            owner,
            parser_continuation(graph_script.clone()),
        );

        let graph_dispatch = graph_store
            .take_next_ready_dispatch::<MainDocumentReadyActionRoute>()
            .expect("main parser graph-ready work should be owner tagged")
            .expect("main parser graph-ready work should route back to the ready-lane owner");
        assert_eq!(*graph_dispatch.queued_owner(), main_parser_document_owner());
        let graph_route = graph_dispatch.route();
        assert_eq!(graph_route.script_node_id(), graph_script.node_id);

        let failure_script = prepared_module_script(32);
        let mut failure_store = MainDocumentScriptSchedulerStore::default();
        let pending_script_id = failure_store.register_module_script(owner, &failure_script);
        assert!(
            failure_store
                .watch_module_script(pending_script_id)
                .watched()
        );
        queue_parser_ready_failure(
            &mut failure_store,
            owner,
            ParserModuleScriptFailure {
                continuation: parser_continuation(failure_script.clone()),
                error: crate::module_runtime::ModuleLoadError::new(
                    crate::module_runtime::ModuleLoadStage::Fetch,
                    "network error",
                ),
            },
        );
        let failure_dispatch = failure_store
            .take_next_ready_dispatch::<MainDocumentReadyActionRoute>()
            .expect("main parser graph-failed work should be owner tagged")
            .expect("main parser graph-failed work should route back to the ready-lane owner");
        assert_eq!(
            *failure_dispatch.queued_owner(),
            main_parser_document_owner()
        );
        let failure_route = failure_dispatch.route();
        assert_eq!(failure_route.script_node_id(), failure_script.node_id);

        let evaluation_script = prepared_module_script(33);
        let mut evaluation_store = MainDocumentScriptSchedulerStore::default();
        queue_parser_evaluation(
            &mut evaluation_store,
            main_parser_document_owner(),
            ModuleScriptEvaluationContinuation {
                script_continuation: parser_continuation(evaluation_script.clone()),
                root_entry: crate::module_runtime::ModuleEntryId::for_test(1),
                reaction_id: 77,
                reaction_state: ModuleScriptEvaluationReactionState::Fulfilled,
                completion_applied_at_evaluation_start: true,
            },
        );
        let evaluation_dispatch = evaluation_store
            .take_next_ready_dispatch::<MainDocumentReadyActionRoute>()
            .expect("main parser evaluation work should be owner tagged")
            .expect("main parser evaluation work should route back to the ready-lane owner");
        assert_eq!(
            *evaluation_dispatch.queued_owner(),
            main_parser_document_owner()
        );
        let evaluation_route = evaluation_dispatch.route();
        assert_eq!(evaluation_route.script_node_id(), evaluation_script.node_id);
    }

    #[test]
    fn parser_pending_module_tree_is_watched_when_eof_waits() {
        let script = prepared_module_script(12);
        let mut store = ModuleScriptContinuationStore::default();
        let mut scheduler = MainDocumentScriptSchedulerStore::default();
        let owner = main_parser_document_owner();
        let Some(crate::document_script_scheduler::ParserDeferredScriptStartAction::ModuleGraph(
            start,
        )) = scheduler.claim_parser_deferred_script(
            owner,
            script.clone(),
            None,
            None,
            Default::default(),
            crate::frame_owner_model::DocumentLoadDelayTokenId(12),
        )
        else {
            panic!("parser module-defer must register before graph start");
        };
        let (pending_script_id, _) = start.into_parts();
        store.insert_parser_pending_fetches([401], parser_continuation(script.clone()));

        assert!(
            scheduler.has_load_blocking_document_script_work(owner),
            "prestarted module tree is already owned by the parser-deferred queue"
        );
        assert!(
            !store.has_pending_fetch(&scheduler),
            "parser acceptance owns the PendingScript without watching a non-head fetch"
        );

        assert_eq!(scheduler.seal_parser_deferred_scripts(owner), Ok(1));
        assert!(scheduler.module_script_is_watching_for_test(pending_script_id));
        assert!(
            scheduler.has_load_blocking_document_script_work(owner),
            "sealed parser module remains lifecycle-blocking until its ordered slot executes"
        );
        assert!(
            store.has_pending_fetch(&scheduler),
            "EOF must watch the pending parser-order head"
        );
    }

    #[test]
    fn parser_fetch_lookup_returns_the_exact_pending_script_without_consuming_it() {
        let script = prepared_module_script(13);
        let continuation = parser_continuation(script.clone());
        let expected = continuation
            .parser_pending_script_id()
            .expect("parser continuation should retain PendingScript identity");
        let mut store = ModuleScriptContinuationStore::default();
        store.insert_parser_pending_fetches([411, 412], continuation);

        assert_eq!(store.pending_script_id_for_fetch(411), Some(expected));
        assert_eq!(store.pending_script_id_for_fetch(412), Some(expected));
        assert_eq!(store.pending_script_id_for_fetch(413), None);
        assert!(
            store.take_active_tree_for_fetch_completion(411).is_some(),
            "currentness lookup must not consume the active graph continuation"
        );
        assert_eq!(
            store.pending_script_id_for_fetch(412),
            None,
            "taking the shared active graph must make sibling fetch ids non-authoritative until the continuation is restored"
        );
    }

    #[test]
    fn sealed_parser_module_tree_owner_blocks_lifecycle() {
        let script = prepared_module_script(14);
        let mut store = ModuleScriptContinuationStore::default();
        let mut scheduler = MainDocumentScriptSchedulerStore::default();
        let owner = main_parser_document_owner();
        let Some(crate::document_script_scheduler::ParserDeferredScriptStartAction::ModuleGraph(
            start,
        )) = scheduler.claim_parser_deferred_script(
            owner,
            script.clone(),
            None,
            None,
            Default::default(),
            crate::frame_owner_model::DocumentLoadDelayTokenId(14),
        )
        else {
            panic!("parser module-defer must register before graph start");
        };
        let (pending_script_id, _) = start.into_parts();
        assert_eq!(scheduler.seal_parser_deferred_scripts(owner), Ok(1));
        assert!(scheduler.module_script_is_watching_for_test(pending_script_id));
        store.insert_parser_pending_fetches([501], parser_continuation(script.clone()));

        assert!(
            scheduler.has_load_blocking_document_script_work(owner),
            "parser module tree created from an executing page task is already watched"
        );
        assert!(
            store.has_pending_fetch(&scheduler),
            "implicit parser module tree owner should block while its fetch is pending"
        );
        let continuation = store
            .take_active_tree_for_fetch_completion(501)
            .expect("implicit parser module tree owner should route fetch completion")
            .into_continuation();
        assert_eq!(continuation.script.node_id, script.node_id);
    }

    #[test]
    fn parser_pending_module_script_failure_waits_for_runner_watch() {
        let script = prepared_module_script(10);
        let mut scheduler = MainDocumentScriptSchedulerStore::default();
        let owner = main_parser_document_owner();
        let pending_script_id = scheduler.register_module_script(owner, &script);
        let error = crate::module_runtime::ModuleLoadError::new(
            crate::module_runtime::ModuleLoadStage::Fetch,
            "network error",
        );

        queue_parser_ready_failure(
            &mut scheduler,
            owner,
            ParserModuleScriptFailure {
                continuation: parser_continuation(script.clone()),
                error,
            },
        );
        assert!(
            !scheduler.has_ready_work(),
            "unwatched failed pending module script must not enter the scheduler ready lane"
        );

        let watch = scheduler.watch_module_script(pending_script_id);
        assert!(watch.watched());
        assert!(watch.queued_ready_work());
        let failure = scheduler
            .take_next_ready_work()
            .map(DocumentScriptReadyWork::into_module_script_graph_failed)
            .map(DocumentOwnedScriptReadyAction::into_action)
            .expect("failed pending module script should enter the parser failure queue");
        assert_eq!(failure.continuation.script.node_id, script.node_id);
        assert_eq!(failure.error.message(), "network error");
    }

    #[test]
    fn parser_ready_failure_uses_shared_scheduler_ready_lane() {
        let script = prepared_module_script(18);
        let mut scheduler = MainDocumentScriptSchedulerStore::default();
        let owner = main_parser_document_owner();
        let pending_script_id = scheduler.register_module_script(owner, &script);
        assert!(scheduler.watch_module_script(pending_script_id).watched());
        let error = crate::module_runtime::ModuleLoadError::new(
            crate::module_runtime::ModuleLoadStage::Fetch,
            "network error",
        );

        queue_parser_ready_failure(
            &mut scheduler,
            owner,
            ParserModuleScriptFailure {
                continuation: parser_continuation(script.clone()),
                error: error.clone(),
            },
        );

        assert!(scheduler.has_ready_work());
        let failure = scheduler
            .take_next_ready_work()
            .map(DocumentScriptReadyWork::into_module_script_graph_failed)
            .map(DocumentOwnedScriptReadyAction::into_action)
            .expect("ready parser failure should drain from the shared scheduler ready lane");
        assert_eq!(failure.continuation.script.node_id, script.node_id);
        assert_eq!(failure.error, error);
        assert!(!scheduler.has_ready_work());
    }

    #[test]
    fn parser_module_evaluation_is_not_lifecycle_pending_until_reaction_finishes() {
        let script = prepared_module_script(11);
        let mut scheduler = MainDocumentScriptSchedulerStore::default();
        let owner = main_parser_document_owner();

        queue_parser_evaluation(
            &mut scheduler,
            owner,
            ModuleScriptEvaluationContinuation {
                script_continuation: parser_continuation(script),
                root_entry: crate::module_runtime::ModuleEntryId::for_test(1),
                reaction_id: 42,
                reaction_state: ModuleScriptEvaluationReactionState::Pending,
                completion_applied_at_evaluation_start: true,
            },
        );

        assert!(
            !scheduler.has_load_blocking_document_script_work(owner),
            "parser-owned evaluation promise should not keep DCL/load blocked until TLA completion"
        );
        assert!(
            !scheduler.has_ready_work(),
            "pending evaluation must not enter the scheduler ready lane until the module reaction fires"
        );

        assert!(
            scheduler
                .mark_parser_module_evaluation_fulfilled(
                    42,
                    parser_module_evaluation_continuation_into_ready_action
                )
                .is_some(),
            "reaction should mark the existing parser-owned evaluation"
        );
        assert!(scheduler.has_load_blocking_document_script_work(owner));
        let Some(DocumentScriptReadyWork::ModuleScriptEvaluationCompleted(evaluation)) =
            scheduler.take_next_ready_work()
        else {
            panic!("ready evaluation should route through the shared scheduler ready lane");
        };
        let evaluation = evaluation.into_action();
        assert_eq!(evaluation.reaction_id, 42);
        assert!(matches!(
            evaluation.reaction_state,
            ModuleScriptEvaluationReactionState::Fulfilled
        ));
        assert!(
            !scheduler.has_load_blocking_document_script_work(owner),
            "draining the final evaluation should release the parser-owned module pending state"
        );
    }

    #[test]
    fn parser_pending_fetches_can_share_one_active_graph_continuation() {
        let script = prepared_module_script(13);
        let mut store = ModuleScriptContinuationStore::default();

        let mut continuation = parser_continuation(script.clone());
        continuation.active_fetch_load_id = Some(101);
        store.insert_parser_pending_fetches([101, 102], continuation);

        let continuation = store
            .take_active_tree_for_fetch_completion(102)
            .expect("second load id should find the shared active continuation")
            .into_continuation();
        assert_eq!(continuation.script.node_id, script.node_id);
        store.restore_pending_continuation(continuation);
        let continuation = store
            .take_active_tree_for_fetch_completion(101)
            .expect("first load id should still route to the restored active continuation")
            .into_continuation();
        assert_eq!(continuation.script.node_id, script.node_id);
    }

    #[test]
    fn parser_pending_fetch_clear_removes_all_load_ids_for_script() {
        let script = prepared_module_script(15);
        let mut store = ModuleScriptContinuationStore::default();

        store.insert_parser_pending_fetches([201, 202], parser_continuation(script.clone()));
        let (mut removed, removed_joined_clients) =
            store.clear_pending_fetches_for_script(script.node_id);
        removed.sort_unstable();
        assert_eq!(removed, vec![201, 202]);
        assert!(removed_joined_clients.is_empty());
        assert!(
            store.take_active_tree_for_fetch_completion(201).is_none(),
            "cleared load id should no longer route to the script"
        );
        assert!(
            store.take_active_tree_for_fetch_completion(202).is_none(),
            "cleared load id should no longer route to the script"
        );
    }

    #[test]
    fn parser_pending_fetch_clear_returns_joined_clients_for_owner_detach() {
        let script = prepared_module_script(16);
        let joined_client = module_tree::SingleModuleClientToken {
            tree_id: module_tree::ModuleTreeId(9),
            sequence: 4,
        };
        let mut store = ModuleScriptContinuationStore::default();

        store.insert_parser_pending_waits(
            std::iter::empty::<u64>(),
            [joined_client],
            parser_continuation(script.clone()),
        );

        let (removed_load_ids, removed_joined_clients) =
            store.clear_pending_fetches_for_script(script.node_id);
        assert!(removed_load_ids.is_empty());
        assert_eq!(removed_joined_clients, vec![joined_client]);
        assert!(
            store
                .take_active_tree_for_joined_client(joined_client)
                .is_none(),
            "cleared joined client should no longer route to the parser owner"
        );
    }

    #[test]
    fn document_replacement_clear_drops_all_parser_owned_continuations() {
        let fetching_script = prepared_module_script(21);
        let ready_script = prepared_module_script(22);
        let evaluation_script = prepared_module_script(23);
        let pending_script = prepared_module_script(24);
        let joined_client = module_tree::SingleModuleClientToken {
            tree_id: module_tree::ModuleTreeId(9),
            sequence: 3,
        };
        let mut store = ModuleScriptContinuationStore::default();
        let mut scheduler = MainDocumentScriptSchedulerStore::default();

        let owner = main_parser_document_owner();
        let pending_script_id = scheduler.register_module_script(owner, &pending_script);
        assert!(scheduler.watch_module_script(pending_script_id).watched());
        let ready_pending_script_id = scheduler.register_module_script(owner, &ready_script);
        assert!(
            scheduler
                .watch_module_script(ready_pending_script_id)
                .watched()
        );
        store.insert_parser_pending_waits(
            [301],
            [joined_client],
            parser_continuation(fetching_script.clone()),
        );
        queue_parser_ready_completed(&mut scheduler, owner, parser_continuation(ready_script));
        queue_parser_evaluation(
            &mut scheduler,
            main_parser_document_owner(),
            ModuleScriptEvaluationContinuation {
                script_continuation: parser_continuation(evaluation_script),
                root_entry: crate::module_runtime::ModuleEntryId::for_test(1),
                reaction_id: 77,
                reaction_state: ModuleScriptEvaluationReactionState::Fulfilled,
                completion_applied_at_evaluation_start: true,
            },
        );

        store.clear_for_document_replacement();
        scheduler.clear();

        assert!(!scheduler.has_load_blocking_document_script_work(owner));
        assert!(!store.has_pending_fetch(&scheduler));
        assert!(!scheduler.has_ready_work());
        assert!(store.take_active_tree_for_fetch_completion(301).is_none());
        assert!(
            store
                .take_active_tree_for_joined_client(joined_client)
                .is_none()
        );
        assert!(!scheduler.watch_module_script(pending_script_id).watched());
    }
}
