use std::{
    cell::{Ref, RefCell, RefMut},
    collections::{HashSet, VecDeque},
    rc::Rc,
};

use moli_owner_queue::OwnerTaskSource;
use tracing::debug;

use crate::{
    document_script_scheduler::{
        DocumentScriptExecutionLane, PageOwnedDocumentScriptWork, ParserDeferredScriptStartAction,
    },
    dynamic_script_owner::{DynamicScriptOwner, DynamicScriptOwnerEventSource},
    frame_owner_model::{DocumentLoadDelayTokenId, FrameDocumentTaskOwner},
    host::HostScriptScheduler,
    module_script_continuation::{
        MainDocumentScriptSchedulerStore, MainParserDocumentOwner, ModuleScriptContinuationStore,
    },
    page_task_queue::{
        PageTask, PostParseLifecycleWork, PostParsePageOwnedWork, WindowScriptFailureReportTask,
    },
    parser::ParserScriptPreparationFailure,
    planning::PreparedScript,
    stylesheet_blocking::DocumentBlockingStylesheetSignature,
    types::{ScriptMode, ScriptRun},
};

#[derive(Debug)]
pub(crate) struct DocumentScriptLifecycle {
    scripts: HostScriptScheduler,
    parser_module_scripts: ModuleScriptContinuationStore,
    parser_module_document_scripts: MainDocumentScriptSchedulerStore,
    main_parser_deferred_scripts_owner: Option<FrameDocumentTaskOwner>,
    runtime_script_work: Rc<RefCell<RuntimeScriptWorkState>>,
    runtime_script_events: DynamicScriptOwnerEventSource,
    deferred_page_tasks: DeferredPageTaskState,
    parser_owned_pre_domcontentloaded_work: OwnerTaskSource<PostParsePageOwnedWork>,
    parser_boundary_lifecycle_tx: tokio::sync::mpsc::UnboundedSender<PageTask>,
    /// Standalone `DocumentRuntime` unit tests retain the same owner-source
    /// residence used by the parse-time queue instead of constructing a
    /// runtime with no parser-boundary route.
    #[cfg(test)]
    standalone_parser_boundary_lifecycle_source: Option<OwnerTaskSource<PageTask>>,
    pending_main_parser_deferred_starts: VecDeque<PendingMainParserDeferredScriptStart>,
}

pub(crate) type SharedRuntimeScriptWorkState = Rc<RefCell<RuntimeScriptWorkState>>;

impl DocumentScriptLifecycle {
    pub(crate) fn with_scheduler(
        scripts: HostScriptScheduler,
        parser_boundary_lifecycle_tx: tokio::sync::mpsc::UnboundedSender<PageTask>,
    ) -> Self {
        let runtime_script_events = DynamicScriptOwnerEventSource::default();
        let dynamic_scripts = DynamicScriptOwner::with_event_sender(runtime_script_events.sender());
        Self {
            scripts,
            parser_module_scripts: ModuleScriptContinuationStore::default(),
            parser_module_document_scripts: MainDocumentScriptSchedulerStore::default(),
            main_parser_deferred_scripts_owner: None,
            runtime_script_work: Rc::new(RefCell::new(
                RuntimeScriptWorkState::with_dynamic_scripts(dynamic_scripts),
            )),
            runtime_script_events,
            deferred_page_tasks: DeferredPageTaskState::default(),
            parser_owned_pre_domcontentloaded_work: OwnerTaskSource::default(),
            parser_boundary_lifecycle_tx,
            #[cfg(test)]
            standalone_parser_boundary_lifecycle_source: None,
            pending_main_parser_deferred_starts: VecDeque::new(),
        }
    }

    pub(crate) fn enqueue_parser_boundary_lifecycle_work(&self, work: PostParseLifecycleWork) {
        assert!(
            self.parser_boundary_lifecycle_tx
                .send(work.into_page_task())
                .is_ok(),
            "active Document parser-boundary lifecycle route must remain resident"
        );
    }

    #[cfg(test)]
    pub(super) fn retain_standalone_parser_boundary_lifecycle_source(
        &mut self,
        source: OwnerTaskSource<PageTask>,
    ) {
        assert!(
            self.standalone_parser_boundary_lifecycle_source
                .replace(source)
                .is_none(),
            "standalone parser-boundary lifecycle residence must be installed exactly once"
        );
    }

    pub(crate) fn clear_for_document_replacement(&mut self) {
        self.scripts.clear_for_document_replacement();
        self.parser_module_scripts.clear_for_document_replacement();
        self.parser_module_document_scripts.clear();
        self.main_parser_deferred_scripts_owner = None;
        let runtime_script_events = DynamicScriptOwnerEventSource::default();
        let dynamic_scripts = DynamicScriptOwner::with_event_sender(runtime_script_events.sender());
        self.runtime_script_events = runtime_script_events;
        self.runtime_script_work
            .borrow_mut()
            .clear_with_dynamic_scripts(dynamic_scripts);
        self.deferred_page_tasks.clear();
        // Retire the source itself, not only its currently materialized
        // queue. Producers from the replaced Document keep senders for the
        // old channel and must never publish into the new Document's owner
        // lane after this reset.
        self.parser_owned_pre_domcontentloaded_work = OwnerTaskSource::default();
        self.pending_main_parser_deferred_starts.clear();
    }

    pub(crate) fn scripts(&self) -> &HostScriptScheduler {
        &self.scripts
    }

    pub(crate) fn scripts_mut(&mut self) -> &mut HostScriptScheduler {
        &mut self.scripts
    }

    pub(crate) fn parser_module_scripts(&self) -> &ModuleScriptContinuationStore {
        &self.parser_module_scripts
    }

    pub(crate) fn parser_module_scripts_mut(&mut self) -> &mut ModuleScriptContinuationStore {
        &mut self.parser_module_scripts
    }

    pub(crate) fn parser_module_document_scripts(&self) -> &MainDocumentScriptSchedulerStore {
        &self.parser_module_document_scripts
    }

    pub(crate) fn parser_module_document_scripts_mut(
        &mut self,
    ) -> &mut MainDocumentScriptSchedulerStore {
        &mut self.parser_module_document_scripts
    }

    pub(crate) fn arm_main_parser_deferred_scripts(&mut self, owner: FrameDocumentTaskOwner) {
        match self.main_parser_deferred_scripts_owner {
            Some(armed_owner) if armed_owner == owner => return,
            Some(armed_owner) => {
                debug_assert_eq!(
                    armed_owner, owner,
                    "one document lifecycle cannot arm parser-deferred work for two owners"
                );
            }
            None => {}
        }
        debug!(?owner, "armed main parser-deferred document owner source");
        self.main_parser_deferred_scripts_owner = Some(owner);
    }

    pub(crate) fn main_parser_deferred_scripts_owner(&self) -> Option<FrameDocumentTaskOwner> {
        self.main_parser_deferred_scripts_owner
    }

    pub(crate) fn disarm_main_parser_deferred_scripts(&mut self, owner: FrameDocumentTaskOwner) {
        if self.main_parser_deferred_scripts_owner == Some(owner) {
            debug!(
                ?owner,
                "disarmed main parser-deferred document owner source"
            );
            self.main_parser_deferred_scripts_owner = None;
        }
    }

    pub(crate) fn runtime_script_work_mut(&self) -> RefMut<'_, RuntimeScriptWorkState> {
        self.runtime_script_work.borrow_mut()
    }

    pub(crate) fn runtime_script_work(&self) -> Ref<'_, RuntimeScriptWorkState> {
        self.runtime_script_work.borrow()
    }

    pub(crate) fn runtime_script_work_handle(&self) -> SharedRuntimeScriptWorkState {
        Rc::clone(&self.runtime_script_work)
    }

    pub(crate) fn record_parser_no_execution_run(&self, run: ScriptRun) {
        self.runtime_script_work
            .borrow_mut()
            .parser_no_execution_runs
            .push(run);
    }

    pub(crate) fn take_parser_no_execution_runs(&self) -> Vec<ScriptRun> {
        std::mem::take(
            &mut self
                .runtime_script_work
                .borrow_mut()
                .parser_no_execution_runs,
        )
    }

    pub(crate) fn accept_ready_runtime_script_events(&mut self) {
        let events = self.runtime_script_events.drain_ready();
        if events.is_empty() {
            return;
        }
        let mut work = self.runtime_script_work.borrow_mut();
        for event in events {
            work.dynamic_scripts.apply_owner_event(event);
        }
    }

    pub(crate) fn deferred_page_tasks_mut(&mut self) -> &mut DeferredPageTaskState {
        &mut self.deferred_page_tasks
    }

    pub(crate) fn parser_owned_pre_domcontentloaded_work_mut(
        &mut self,
    ) -> &mut OwnerTaskSource<PostParsePageOwnedWork> {
        &mut self.parser_owned_pre_domcontentloaded_work
    }

    pub(crate) fn parser_owned_pre_domcontentloaded_work(
        &self,
    ) -> &OwnerTaskSource<PostParsePageOwnedWork> {
        &self.parser_owned_pre_domcontentloaded_work
    }

    pub(crate) fn enqueue_main_parser_deferred_start(
        &mut self,
        start: PendingMainParserDeferredScriptStart,
    ) {
        self.pending_main_parser_deferred_starts.push_back(start);
    }

    pub(crate) fn take_main_parser_deferred_starts(
        &mut self,
    ) -> VecDeque<PendingMainParserDeferredScriptStart> {
        std::mem::take(&mut self.pending_main_parser_deferred_starts)
    }
}

/// A parser PendingScript accepted during nested V8 re-entry whose concrete
/// source/module start waits until that re-entry unwinds.
pub(crate) struct PendingMainParserDeferredScriptStart {
    task_owner: FrameDocumentTaskOwner,
    load_delay_token: DocumentLoadDelayTokenId,
    action: ParserDeferredScriptStartAction<MainParserDocumentOwner>,
}

impl std::fmt::Debug for PendingMainParserDeferredScriptStart {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let action = match &self.action {
            ParserDeferredScriptStartAction::NoFetch => "NoFetch",
            ParserDeferredScriptStartAction::ClassicSource(_) => "ClassicSource",
            ParserDeferredScriptStartAction::ModuleGraph(_) => "ModuleGraph",
        };
        formatter
            .debug_struct("PendingMainParserDeferredScriptStart")
            .field("task_owner", &self.task_owner)
            .field("load_delay_token", &self.load_delay_token)
            .field("action", &action)
            .finish()
    }
}

impl PendingMainParserDeferredScriptStart {
    pub(crate) fn new(
        task_owner: FrameDocumentTaskOwner,
        load_delay_token: DocumentLoadDelayTokenId,
        action: ParserDeferredScriptStartAction<MainParserDocumentOwner>,
    ) -> Self {
        Self {
            task_owner,
            load_delay_token,
            action,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        FrameDocumentTaskOwner,
        DocumentLoadDelayTokenId,
        ParserDeferredScriptStartAction<MainParserDocumentOwner>,
    ) {
        (self.task_owner, self.load_delay_token, self.action)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FollowupPageTaskDisposition {
    Skipped,
    Deferred,
    Enqueued,
}

/// Convert one prepared parser-owned script directly into its sole executable
/// Page residence.
///
/// Parser-blocking/defer lanes preserve the stylesheet snapshot captured at
/// discovery. Async work has no stylesheet gate and enters the shared
/// DocumentScript execution lane without an intermediate written-script
/// queue.
pub(crate) fn parser_prepared_script_page_owned_work(
    script: PreparedScript,
    blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
) -> PostParsePageOwnedWork {
    let lane = match script.mode {
        ScriptMode::Normal => DocumentScriptExecutionLane::ParserBlocking,
        ScriptMode::Defer => DocumentScriptExecutionLane::ClassicDefer,
        ScriptMode::ModuleDefer => DocumentScriptExecutionLane::ModuleDefer,
        ScriptMode::InOrder
        | ScriptMode::ImportMapInOrder
        | ScriptMode::ModuleInOrder
        | ScriptMode::Async => DocumentScriptExecutionLane::AsyncPhase,
    };
    let work = PageOwnedDocumentScriptWork::script(lane, script);
    if matches!(
        lane,
        DocumentScriptExecutionLane::ParserBlocking
            | DocumentScriptExecutionLane::ClassicDefer
            | DocumentScriptExecutionLane::ModuleDefer
    ) {
        PostParsePageOwnedWork::document_script_work_with_blocking_signatures(
            work,
            blocking_signatures_before,
        )
    } else {
        PostParsePageOwnedWork::document_script_work(work)
    }
}

/// Report an internal parser preparation failure. External-source failures
/// instead queue an element error on the DOM-manipulation task source.
pub(crate) fn parser_script_preparation_failure_page_owned_work(
    failure: ParserScriptPreparationFailure,
) -> PostParsePageOwnedWork {
    assert!(!failure.is_external_source_failure());
    let (_, _, message) = failure.into_parts();
    PostParsePageOwnedWork::lifecycle_work(PostParseLifecycleWork::ReportWindowScriptFailure(
        WindowScriptFailureReportTask::new(message, None),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeScriptWorkPauseKind {
    StablePageTurnContinuation,
}

#[derive(Debug)]
pub(crate) struct RuntimeScriptWorkState {
    /// Internal parser diagnostics are not an HTML task source. They are
    /// drained into the page report at an existing task/checkpoint boundary.
    pub(crate) parser_no_execution_runs: Vec<ScriptRun>,
    pub(crate) dynamic_scripts: DynamicScriptOwner,
    pub(crate) pause_kind: Option<RuntimeScriptWorkPauseKind>,
}

#[cfg(test)]
impl Default for RuntimeScriptWorkState {
    fn default() -> Self {
        Self::with_dynamic_scripts(DynamicScriptOwner::default())
    }
}

impl RuntimeScriptWorkState {
    fn with_dynamic_scripts(dynamic_scripts: DynamicScriptOwner) -> Self {
        Self {
            parser_no_execution_runs: Vec::new(),
            dynamic_scripts,
            pause_kind: None,
        }
    }

    pub(crate) fn has_pending_work(&mut self) -> bool {
        !self.is_idle()
    }

    pub(crate) fn is_idle(&mut self) -> bool {
        self.dynamic_scripts.is_idle()
    }

    pub(crate) fn pause_for_deferred_page_tasks(&mut self, kind: RuntimeScriptWorkPauseKind) {
        let has_pending_work = !self.is_idle();
        self.pause_kind = has_pending_work.then_some(kind);
    }

    pub(crate) fn resume_after_deferred_page_tasks(&mut self) {
        self.pause_kind = None;
    }

    pub(crate) fn is_paused_for_deferred_page_tasks(&self) -> bool {
        self.pause_kind.is_some()
    }

    pub(crate) fn pause_kind(&self) -> Option<RuntimeScriptWorkPauseKind> {
        self.pause_kind
    }

    fn clear_with_dynamic_scripts(&mut self, dynamic_scripts: DynamicScriptOwner) {
        self.parser_no_execution_runs.clear();
        self.dynamic_scripts.disable_continuation_enqueue();
        self.dynamic_scripts = dynamic_scripts;
        self.pause_kind = None;
    }

    pub(crate) fn has_immediately_runnable_work(&mut self) -> bool {
        self.dynamic_scripts.has_immediately_runnable_work()
    }
}

#[derive(Debug, Default)]
pub(crate) struct DeferredPageTaskState {
    tasks: VecDeque<DeferredPageTask>,
    depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeferredPageTaskLane {
    ParserBoundary,
    PreDomContentLoaded,
    PostDomContentLoaded,
}

#[derive(Debug)]
pub(crate) struct DeferredPageTask {
    work: PostParsePageOwnedWork,
    pub(crate) lane: DeferredPageTaskLane,
}

impl DeferredPageTask {
    pub(crate) fn page_owned_work(
        work: PostParsePageOwnedWork,
        lane: DeferredPageTaskLane,
    ) -> Option<Self> {
        (!matches!(lane, DeferredPageTaskLane::ParserBoundary)).then_some(Self { work, lane })
    }

    pub(crate) fn phase_label(&self) -> &'static str {
        "page-owned work"
    }

    #[cfg(test)]
    pub(crate) fn page_owned_work_ref(&self) -> &PostParsePageOwnedWork {
        &self.work
    }

    pub(crate) fn into_parts(self) -> (DeferredPageTaskLane, PostParsePageOwnedWork) {
        (self.lane, self.work)
    }
}

impl DeferredPageTaskState {
    pub(crate) fn enter_scope(&mut self) {
        self.depth += 1;
    }

    pub(crate) fn exit_scope(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    pub(crate) fn enqueue_or_defer(
        &mut self,
        task: DeferredPageTask,
        mut enqueue_now: impl FnMut(DeferredPageTask),
    ) -> FollowupPageTaskDisposition {
        if self.depth > 0 {
            debug!(
                task = task.phase_label(),
                deferred_count = self.tasks.len(),
                "deferring page task until parser progress resumes"
            );
            self.tasks.push_back(task);
            return FollowupPageTaskDisposition::Deferred;
        }
        enqueue_now(task);
        FollowupPageTaskDisposition::Enqueued
    }

    pub(crate) fn drain_into(&mut self, mut enqueue_now: impl FnMut(DeferredPageTask)) {
        while let Some(task) = self.tasks.pop_front() {
            debug!(
                task = task.phase_label(),
                remaining = self.tasks.len(),
                "running deferred page task after parser progress"
            );
            enqueue_now(task);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.tasks.clear();
        self.depth = 0;
    }
}

#[cfg(test)]
mod parser_boundary_lifecycle_route_tests {
    use super::*;

    #[test]
    fn standalone_parser_boundary_route_retains_published_lifecycle_work() {
        let source = OwnerTaskSource::<PageTask>::default();
        let sender = source.parser_boundary_sender();
        let mut lifecycle =
            DocumentScriptLifecycle::with_scheduler(HostScriptScheduler::default(), sender);
        lifecycle.retain_standalone_parser_boundary_lifecycle_source(source);

        lifecycle.enqueue_parser_boundary_lifecycle_work(
            PostParseLifecycleWork::test_domcontentloaded(),
        );

        assert!(matches!(
            lifecycle
                .standalone_parser_boundary_lifecycle_source
                .as_mut()
                .expect("standalone source")
                .pop_front(),
            Some(PageTask::DispatchDomContentLoaded)
        ));
    }

    #[test]
    #[should_panic(
        expected = "active Document parser-boundary lifecycle route must remain resident"
    )]
    fn closed_parser_boundary_route_cannot_silently_drop_lifecycle_work() {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        drop(receiver);
        let lifecycle =
            DocumentScriptLifecycle::with_scheduler(HostScriptScheduler::default(), sender);

        lifecycle.enqueue_parser_boundary_lifecycle_work(
            PostParseLifecycleWork::test_domcontentloaded(),
        );
    }
}
