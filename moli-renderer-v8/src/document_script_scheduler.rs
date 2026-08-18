mod async_queues;
mod completion_port;
mod execution_lane;
mod frame_classic_owner;
mod frame_document_ready;
mod frame_document_script_owner;
mod frame_module_script_owner;
mod frame_parser_classic;
mod frame_parser_deferred_order;
mod frame_stylesheet;
mod main_document_ready;
mod main_parser_async_module_admission;
mod module_ready;
mod owner_execution;
mod owner_hooks;
mod owner_runner;
mod owner_work;
mod page_task_adapter;
mod parse_time_task;
mod parse_visible_async;
mod parser_runner;
mod post_parse;
mod post_parse_task;
mod ready_work;
mod runner;
mod source_load_adapter;
mod source_load_port;
mod store;

#[cfg(test)]
use crate::document_task_lane::DocumentTaskQueue;

#[cfg(test)]
use super::planning::SharedScriptSourceLoad;
use super::{page_task_queue::RendererOwnerWakeSender, planning::PreparedScript};

#[cfg(test)]
use self::async_queues::{
    AsyncFallbackEntry, AsyncFallbackQueue, AsyncLoadCompletion, AsyncParseTimeQueue,
    ParseTimeAsyncEntry,
};
pub(crate) use self::execution_lane::{
    DocumentScriptExecutionLane, DocumentScriptSourceFailureLane,
};
pub(crate) use self::frame_classic_owner::{
    FrameClassicDocumentScriptExecutionOwner, ParserClassicDocumentScriptCompletionPlan,
    ParserClassicDocumentScriptContinuation, ParserClassicDocumentScriptExecutionHooks,
    ParserClassicDocumentScriptExecutionOwner, ParserClassicDocumentScriptExecutionResult,
    ParserClassicDocumentScriptExecutionStartReport,
    ParserClassicDocumentScriptSourceFailureReport,
};
pub(crate) use self::frame_document_ready::{
    FrameDocumentClassicReadyWork, FrameDocumentClassicScriptSchedulerWork,
    FrameDocumentClassicSourceFailureWork, FrameDocumentModuleScriptReadyWork,
    FrameDocumentScriptReadyWork, FrameDocumentScriptSchedulerStore,
};
pub(crate) use self::frame_document_script_owner::{
    DocumentScriptExecutionHooks, DocumentScriptExecutionRunner,
    DocumentScriptExecutionStartReport, FrameDocumentScriptExecutionOwner,
    FrameDocumentScriptExecutionStartReport,
};
pub(crate) use self::frame_module_script_owner::{
    FrameModuleScriptDocumentScriptHooks, FrameModuleScriptDocumentScriptRunner,
    FrameModuleScriptEvaluationStart, FrameModuleScriptRunOutcome, FrameModuleScriptTaskActivity,
};
pub(crate) use self::frame_parser_classic::{
    FrameParserClassicScriptItem, FrameParserClassicScriptRunnerStore,
    external_pending_frame_parser_classic_script_item_with_blocking_signatures,
    inline_frame_parser_classic_script_item_with_blocking_signatures,
};
pub(crate) use self::frame_parser_deferred_order::{
    FrameParserDeferredScriptKind, FrameParserDeferredScriptOrderEntry,
    FrameParserDeferredScriptOrderStore,
};
pub(crate) use self::frame_stylesheet::FrameDocumentBlockingStylesheetStore;
pub(crate) use self::main_document_ready::{
    MainDocumentClassicReadyWork, MainDocumentClassicScriptTarget,
    MainDocumentClassicSourceFailureWork, MainDocumentReadyActionRoute,
};
pub(crate) use self::main_parser_async_module_admission::MainParserAsyncModuleAdmission;
pub(crate) use self::module_ready::ParserModuleGraphTerminalWork;
use self::module_ready::ParserModulePendingScriptWatchResult;
pub(crate) use self::module_ready::{
    DocumentModuleGraphFailedWork, DocumentModuleGraphReadyWork, DocumentModuleScriptReadyWork,
    FrameDocumentModuleGraphReadyTarget, FrameDocumentReadyActionRoute, ModuleScriptGraphReadyWork,
    ParserOrderedModuleTerminalState, ParserPendingScriptId, ParserPendingScriptKey,
    ParserPendingScriptRoute,
};
pub(crate) use self::owner_execution::{
    PageOwnedDocumentScriptBodyActivity, PageOwnedDocumentScriptBodyExecution,
    PageOwnedDocumentScriptBodyKind, PageOwnedDocumentScriptExecution,
};
pub(crate) use self::owner_hooks::PageOwnedDocumentScriptHooks;
pub(crate) use self::owner_runner::PageOwnedDocumentScriptRunner;
pub(crate) use self::owner_work::{
    PageOwnedDocumentScriptSourceFailure, PageOwnedDocumentScriptWork,
};
pub(crate) use self::parse_time_task::{ParseTimeDocumentScriptEvent, ParseTimeDocumentScriptTask};
#[cfg(test)]
use self::parse_visible_async::ParseVisibleReevaluationCreditReason;
#[cfg(test)]
pub(super) use self::parse_visible_async::PostParseScriptClaimDisposition;
pub(super) use self::parse_visible_async::{
    ParseTimeTurn, ParseTimeTurnTrigger, ParseVisibleReadyTurnDisposition,
    ParseVisibleReadyTurnPhase,
};
#[cfg(test)]
use self::parse_visible_async::{ParseVisibleAsyncLaneState, ParseVisibleAsyncReevaluationCredit};
#[cfg(test)]
use self::parse_visible_async::{
    ParseVisibleReevaluationCreditGrant, ParseVisibleReevaluationCreditGrantRefusalReason,
};
use self::parser_runner::ParserScriptRunner;
pub(crate) use self::parser_runner::{
    ParserDeferredClassicSourceLoadCompletion, ParserDeferredModuleGraphStart,
    ParserDeferredScriptReady, ParserDeferredScriptStartAction,
};
pub(crate) use self::post_parse::ParserDeferredClassicReady;
#[cfg(test)]
use self::post_parse::ResolvedDeferPhaseScript;
pub(crate) use self::ready_work::{
    DocumentOwnedScriptReadyAction, DocumentScriptExecutionOutcome,
    DocumentScriptReadyActionDispatchRoute, DocumentScriptReadyActionRoute,
    DocumentScriptReadyDispatch, DocumentScriptReadyDispatchOwnerMismatch, DocumentScriptReadyWork,
    DocumentScriptReadyWorkOwner, ParserClassicDocumentScriptReadyOwner,
};
use self::runner::DocumentScriptRunner;
pub(crate) use self::store::{
    DocumentScriptSchedulerStore, ParserDeferredClassicSourceLoadApplyResult,
    ParserModuleEvaluationReactionUpdate,
};

/// Document-level script scheduler with readiness-driven async ownership.
///
/// This scheduler implements the design described in the Blink/Servo gap analysis:
/// completion is a first-class wake source, and the parser does not participate
/// in async readiness observation at all.
///
/// ## Ownership model
///
/// Scripts are classified at parser discovery time into document-owned queues:
/// - `defer_like_scripts`: classic defer + modules/importmaps (run post-parse, pre-DCL)
/// - `async_parse_time_queue`: classic external async eligible for parse-time execution
/// - `async_fallback_queue`: async scripts that cannot participate in parse-time execution
///
/// ## Readiness-driven async execution
///
/// When an async script's network fetch completes, the completion is delivered
/// through an owner-provided completion port. The main adapter currently maps
/// that port to `PageTaskQueue` channel injection, and the parse-time
/// coordination loop (`feed_html`) watches that queue with `tokio::select!`.
/// Completion arrival immediately wakes the coordinator — no parser-adjacent
/// compat bridges, no wall-clock timeouts, no local yield loops.
///
/// This matches the shared Blink/Servo pattern:
/// - Blink: completion → `PendingScriptFinished()` → `PostTask(...)`
/// - Servo: network completion → `task_source.queue(...)` → script thread callback
/// - Moli main adapter: completion → owner port → `PageTaskQueue` injection →
///   `select!` wakes coordinator
pub(super) struct DocumentScriptScheduler<
    Target = FrameDocumentModuleGraphReadyTarget,
    ParserModuleEvaluation = std::convert::Infallible,
    ParserModuleGraphFailure = std::convert::Infallible,
    ParserClassicReady = std::convert::Infallible,
    ParserClassicSourceFailure = std::convert::Infallible,
> {
    parser_runner: ParserScriptRunner<Target, ParserModuleGraphFailure>,
    runner: DocumentScriptRunner<
        Target,
        ParserModuleEvaluation,
        ParserModuleGraphFailure,
        ParserClassicReady,
        ParserClassicSourceFailure,
    >,
}

impl<
    Target,
    ParserModuleEvaluation,
    ParserModuleGraphFailure,
    ParserClassicReady,
    ParserClassicSourceFailure,
> std::fmt::Debug
    for DocumentScriptScheduler<
        Target,
        ParserModuleEvaluation,
        ParserModuleGraphFailure,
        ParserClassicReady,
        ParserClassicSourceFailure,
    >
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentScriptScheduler")
            .field(
                "defer_like_scripts",
                &self.parser_runner.defer_script_count(),
            )
            .field(
                "parse_time_async_scripts",
                &self.runner.async_parse_time_queue.parse_time_entries.len(),
            )
            .field(
                "fallback_async_scripts",
                &self.runner.async_fallback_queue.entries.len(),
            )
            .field(
                "module_graph_ready_work",
                &self.runner.pending_module_graph_ready_count(),
            )
            .field(
                "module_graph_failed_work",
                &self.runner.pending_parser_module_failure_count(),
            )
            .field(
                "module_evaluation_ready_work",
                &self.runner.pending_parser_module_evaluation_count(),
            )
            .field(
                "classic_ready_work",
                &self.runner.pending_parser_classic_ready_count(),
            )
            .field(
                "classic_source_failure_work",
                &self.runner.pending_parser_classic_source_failure_count(),
            )
            .finish()
    }
}

impl<
    Target,
    ParserModuleEvaluation,
    ParserModuleGraphFailure,
    ParserClassicReady,
    ParserClassicSourceFailure,
> Default
    for DocumentScriptScheduler<
        Target,
        ParserModuleEvaluation,
        ParserModuleGraphFailure,
        ParserClassicReady,
        ParserClassicSourceFailure,
    >
{
    fn default() -> Self {
        Self {
            parser_runner: ParserScriptRunner::default(),
            runner: DocumentScriptRunner::<
                Target,
                ParserModuleEvaluation,
                ParserModuleGraphFailure,
                ParserClassicReady,
                ParserClassicSourceFailure,
            >::new(),
        }
    }
}

impl DocumentScriptScheduler<FrameDocumentModuleGraphReadyTarget> {
    pub(super) fn new() -> Self {
        Self::default()
    }
}

impl<
    Target,
    ParserModuleEvaluation,
    ParserModuleGraphFailure,
    ParserClassicReady,
    ParserClassicSourceFailure,
>
    DocumentScriptScheduler<
        Target,
        ParserModuleEvaluation,
        ParserModuleGraphFailure,
        ParserClassicReady,
        ParserClassicSourceFailure,
    >
{
    pub(super) fn bind_owner_wake(&mut self, owner_wake: Option<RendererOwnerWakeSender>) {
        self.runner.bind_owner_wake(owner_wake);
    }

    pub(super) fn register_module_script(
        &mut self,
        script: &PreparedScript,
    ) -> ParserPendingScriptKey {
        self.parser_runner.register_module_script(script)
    }

    pub(super) fn accept_parser_ordered_module_script(
        &mut self,
        script: &PreparedScript,
        blocking_stylesheet_signatures: std::collections::HashSet<
            crate::stylesheet_blocking::DocumentBlockingStylesheetSignature,
        >,
    ) -> Option<ParserPendingScriptKey> {
        self.parser_runner
            .accept_parser_ordered_module_script(script, blocking_stylesheet_signatures)
    }

    pub(super) fn module_script_blocking_stylesheet_signatures(
        &self,
        key: ParserPendingScriptKey,
    ) -> Option<
        &std::collections::HashSet<crate::stylesheet_blocking::DocumentBlockingStylesheetSignature>,
    > {
        self.parser_runner
            .module_script_blocking_stylesheet_signatures(key)
    }

    pub(super) fn watch_module_script(&mut self, key: ParserPendingScriptKey) -> bool {
        match self.parser_runner.watch_module_script(key) {
            ParserModulePendingScriptWatchResult::Missing => false,
            ParserModulePendingScriptWatchResult::WaitingForTree => true,
            ParserModulePendingScriptWatchResult::Ready(terminals) => {
                self.enqueue_parser_module_terminals(terminals);
                true
            }
        }
    }

    pub(super) fn notify_module_script_graph_ready_work(
        &mut self,
        key: ParserPendingScriptKey,
        work: ModuleScriptGraphReadyWork<Target>,
    ) -> bool {
        let Some(terminals) = self
            .parser_runner
            .notify_module_tree_load_finished(key, work)
        else {
            return false;
        };
        self.enqueue_parser_module_terminals(terminals);
        true
    }

    pub(super) fn notify_module_script_graph_failed_action(
        &mut self,
        key: ParserPendingScriptKey,
        failure: ParserModuleGraphFailure,
    ) -> bool {
        let Some(terminals) = self
            .parser_runner
            .notify_module_tree_load_failed(key, failure)
        else {
            return false;
        };
        self.enqueue_parser_module_terminals(terminals);
        true
    }

    pub(super) fn notify_module_script_evaluation_completed(
        &mut self,
        evaluation: ParserModuleEvaluation,
    ) {
        self.runner
            .notify_module_script_evaluation_completed(evaluation);
    }

    pub(super) fn notify_parser_classic_ready_work(&mut self, ready: ParserClassicReady) {
        self.runner.notify_parser_classic_ready_work(ready);
    }

    pub(super) fn notify_parser_classic_source_failure_work(
        &mut self,
        failure: ParserClassicSourceFailure,
    ) {
        self.runner
            .notify_parser_classic_source_failure_work(failure);
    }

    pub(super) fn take_next_ready_work(
        &mut self,
    ) -> Option<
        DocumentScriptReadyWork<
            Target,
            ParserModuleEvaluation,
            ParserModuleGraphFailure,
            ParserClassicReady,
            ParserClassicSourceFailure,
        >,
    > {
        self.runner.take_next_ready_work()
    }

    #[cfg(test)]
    pub(super) fn pending_module_graph_ready_count(&self) -> usize {
        self.runner.pending_module_graph_ready_count()
    }

    #[cfg(test)]
    pub(super) fn pending_parser_module_evaluation_count(&self) -> usize {
        self.runner.pending_parser_module_evaluation_count()
    }

    #[cfg(test)]
    pub(super) fn pending_parser_module_failure_count(&self) -> usize {
        self.runner.pending_parser_module_failure_count()
    }

    pub(super) fn pending_ready_work_count(&self) -> usize {
        self.runner.pending_ready_work_count()
    }

    pub(super) fn seal_parser_deferred_scripts(&mut self) -> Result<usize, ParserPendingScriptKey> {
        self.parser_runner.seal_defer_phase()
    }

    pub(super) fn complete_parser_deferred_classic_source_load(
        &mut self,
        key: ParserPendingScriptKey,
        outcome: crate::planning::PreparedScriptSourceLoadOutcome,
    ) -> bool {
        self.parser_runner
            .complete_classic_source_load(key, outcome)
    }

    pub(super) fn cancel_parser_deferred_script(
        &mut self,
        key: ParserPendingScriptKey,
    ) -> Option<crate::frame_owner_model::DocumentLoadDelayTokenId> {
        self.parser_runner.cancel_deferred_script(key)
    }

    pub(super) fn has_after_parsing_script(&self) -> bool {
        self.parser_runner.has_after_parsing_script()
    }

    pub(super) fn next_after_parsing_blocking_signatures(
        &self,
    ) -> Option<&std::collections::HashSet<crate::DocumentBlockingStylesheetSignature>> {
        self.parser_runner.next_after_parsing_blocking_signatures()
    }

    pub(super) fn next_after_parsing_script_is_ready(&self) -> bool {
        self.parser_runner.next_after_parsing_script_is_ready()
    }

    pub(super) fn take_next_after_parsing_ready_script(
        &mut self,
    ) -> Option<ParserDeferredScriptReady<Target, ParserModuleGraphFailure>> {
        self.parser_runner.take_next_after_parsing_ready_script()
    }

    #[cfg(test)]
    pub(super) fn parser_ordered_module_terminal_is_ready(
        &self,
        key: ParserPendingScriptKey,
    ) -> bool {
        self.parser_runner
            .parser_ordered_module_terminal_is_ready(key)
    }

    pub(super) fn prepare_parser_ordered_module_terminal(
        &mut self,
        key: ParserPendingScriptKey,
    ) -> ParserOrderedModuleTerminalState {
        self.parser_runner
            .prepare_parser_ordered_module_terminal(key)
    }

    pub(super) fn promote_parser_ordered_module_terminal(
        &mut self,
        key: ParserPendingScriptKey,
    ) -> bool {
        let Some(terminal) = self.parser_runner.take_parser_ordered_module_terminal(key) else {
            return false;
        };
        self.runner.enqueue_parser_module_terminal_work(terminal);
        true
    }

    #[cfg(test)]
    pub(super) fn has_load_blocking_document_script_work(&self) -> bool {
        self.parser_runner.has_lifecycle_blocking_pending_script()
            || self.runner.has_load_blocking_document_script_work()
    }

    pub(super) fn has_module_script(&self, key: ParserPendingScriptKey) -> bool {
        self.parser_runner.has_module_script(key)
    }

    #[cfg(test)]
    pub(super) fn module_script_is_watching_for_test(&self, key: ParserPendingScriptKey) -> bool {
        self.parser_runner.module_script_is_watching_for_test(key)
    }

    pub(super) fn discard_module_script(&mut self, key: ParserPendingScriptKey) -> bool {
        self.parser_runner.discard_module_script(key)
    }

    #[cfg(test)]
    pub(super) fn pending_parser_module_script_count_for_test(&self) -> usize {
        self.parser_runner.pending_module_script_count()
    }

    #[cfg(test)]
    pub(super) fn parser_deferred_scripts_for_test(
        &self,
    ) -> &std::collections::VecDeque<ResolvedDeferPhaseScript> {
        self.parser_runner.after_parsing_scripts()
    }

    fn enqueue_parser_module_terminals(
        &mut self,
        terminals: Vec<ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>>,
    ) {
        for terminal in terminals {
            self.runner.enqueue_parser_module_terminal_work(terminal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::NodeId;
    use crate::{
        planning::{PreparedScriptSourceLoadOutcome, ScriptSource},
        types::{ScriptKind, ScriptMode, ScriptSourceKind},
    };
    use url::Url;

    fn prepared_script(
        position: usize,
        mode: ScriptMode,
        kind: ScriptKind,
        source_kind: ScriptSourceKind,
    ) -> PreparedScript {
        PreparedScript {
            position,
            node_id: NodeId::new(position + 1),
            kind,
            mode,
            source_kind,
            fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
            source: match source_kind {
                ScriptSourceKind::Inline => ScriptSource::Inline(format!("code-{position}")),
                ScriptSourceKind::External => ScriptSource::External,
            },
            url: Url::parse(&format!("https://example.com/{position}.js")).unwrap(),
            base_url: Url::parse(&format!("https://example.com/{position}.js")).unwrap(),
            initiator_url: Url::parse("https://example.com/index.html").unwrap(),
            host_script_handle: None,
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct TestModuleGraphFailure {
        script_node_id: NodeId,
        message: &'static str,
    }

    impl TestModuleGraphFailure {
        fn new(script_node_id: NodeId, message: &'static str) -> Self {
            Self {
                script_node_id,
                message,
            }
        }
    }

    fn prepared_external_data_script(position: usize, mode: ScriptMode) -> PreparedScript {
        let mut script = prepared_script(
            position,
            mode,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        script.url =
            Url::parse("data:text/javascript,window.fallbackCounter%20%3D%201%3B").unwrap();
        script
    }

    fn source_load_outcome_ok(source: impl Into<String>) -> PreparedScriptSourceLoadOutcome {
        PreparedScriptSourceLoadOutcome {
            source_result: Ok(source.into()),
            source_bytes: None,
            network_result: None,
        }
    }

    fn source_load_outcome_err(error: impl Into<String>) -> PreparedScriptSourceLoadOutcome {
        PreparedScriptSourceLoadOutcome {
            source_result: Err(error.into()),
            source_bytes: None,
            network_result: None,
        }
    }

    fn async_load_completion_ok(node_id: NodeId, source: impl Into<String>) -> AsyncLoadCompletion {
        AsyncLoadCompletion {
            node_id,
            outcome: source_load_outcome_ok(source),
        }
    }

    fn async_load_completion_err(node_id: NodeId, error: impl Into<String>) -> AsyncLoadCompletion {
        AsyncLoadCompletion {
            node_id,
            outcome: source_load_outcome_err(error),
        }
    }

    fn parse_time_async_entry(original: PreparedScript) -> ParseTimeAsyncEntry {
        ParseTimeAsyncEntry {
            original,
            load_delay_binding: None,
            claimed_at_handoff: true,
            completion: None,
            source_load: None,
        }
    }

    fn discovered_async_entry(original: PreparedScript) -> ParseTimeAsyncEntry {
        ParseTimeAsyncEntry {
            original,
            load_delay_binding: None,
            claimed_at_handoff: false,
            completion: None,
            source_load: None,
        }
    }

    fn discovered_async_entry_with_source_load(
        original: PreparedScript,
        source_load: SharedScriptSourceLoad,
    ) -> ParseTimeAsyncEntry {
        ParseTimeAsyncEntry {
            original,
            load_delay_binding: None,
            claimed_at_handoff: false,
            completion: None,
            source_load: Some(source_load),
        }
    }

    /// Helper to build a scheduler with pre-populated async state.
    ///
    /// In the readiness-driven model there is no `AsyncCompatTail` — completion
    /// delivery goes exclusively through the page task queue injection channel.
    fn scheduler_with_async_state(
        parse_time_entries: Vec<ParseTimeAsyncEntry>,
        fallback_entries: Vec<PreparedScript>,
    ) -> DocumentScriptScheduler {
        DocumentScriptScheduler {
            parser_runner: ParserScriptRunner::default(),
            runner: DocumentScriptRunner {
                async_parse_time_queue: AsyncParseTimeQueue {
                    parse_time_entries,
                    ready_tasks: DocumentTaskQueue::default(),
                    parse_time_completion_port: None,
                },
                async_fallback_queue: AsyncFallbackQueue {
                    entries: fallback_entries
                        .into_iter()
                        .map(|script| AsyncFallbackEntry {
                            script,
                            load_delay_binding: None,
                            awaiting_completion: false,
                            source_load: None,
                            load_failure: None,
                        })
                        .collect(),
                },
                ready_work: DocumentTaskQueue::default(),
                parse_visible_async_lane_state: ParseVisibleAsyncLaneState::Open,
                parse_visible_async_reevaluation_credit: ParseVisibleAsyncReevaluationCredit::None,
                owner_wake: None,
            },
        }
    }

    // -----------------------------------------------------------------------
    // Basic claim routing
    // -----------------------------------------------------------------------

    #[test]
    fn module_graph_ready_work_is_claimed_by_document_script_runner() {
        let mut scheduler = DocumentScriptScheduler::new();
        let owner = crate::frame_owner_model::FrameDocumentTaskOwner::new(
            crate::frame_owner_model::FrameSchedulerLaneId(1),
            crate::frame_owner_model::LocalWindowId(2),
            crate::frame_owner_model::DocumentId(3),
        );
        let realm_id = crate::frame_owner_model::FrameRealmId(4);
        let request_key = crate::module_runtime::ModuleMapKey::java_script(
            Url::parse("https://scheduler.test/child-module.js").expect("module url"),
        );
        let script = prepared_script(
            5,
            ScriptMode::ModuleDefer,
            ScriptKind::Module,
            ScriptSourceKind::External,
        );
        let pending_script_id = ParserPendingScriptId::new(owner.document_owner(), &script);
        let work = DocumentModuleGraphReadyWork::new(
            owner,
            realm_id,
            pending_script_id,
            script,
            crate::document_runtime::DomHandle::new(5),
            request_key.clone(),
            moli_module_script_tree::ModuleTreeId(6),
            crate::frame_owner_model::DocumentLoadDelayTokenId(10),
            crate::module_runtime::ModuleGraphHandle {
                root_entry: crate::module_runtime::ModuleEntryId::from_raw(7),
                entries: vec![
                    crate::module_runtime::ModuleEntryId::from_raw(7),
                    crate::module_runtime::ModuleEntryId::from_raw(8),
                    crate::module_runtime::ModuleEntryId::from_raw(9),
                ],
            },
        );

        let pending_script_key = scheduler.register_module_script(work.script());
        assert!(scheduler.watch_module_script(pending_script_key));
        assert!(scheduler.notify_module_script_graph_ready_work(pending_script_key, work.clone()));

        assert_eq!(scheduler.pending_module_graph_ready_count(), 1);
        assert_eq!(
            work.entry_id(),
            crate::module_runtime::ModuleEntryId::from_raw(7)
        );
        assert_eq!(work.dependency_count(), 2);
        assert_eq!(work.graph().entries.len(), 3);
        let claimed = scheduler
            .take_next_ready_work()
            .expect("claimed module graph-ready work should be queued")
            .into_module_script_graph_ready();
        assert_eq!(claimed.owner(), work.owner());
        assert_eq!(claimed.realm_id(), work.realm_id());
        assert_eq!(claimed.script().node_id, work.script().node_id);
        assert_eq!(claimed.script_handle(), work.script_handle());
        assert_eq!(claimed.request_key(), work.request_key());
        assert_eq!(claimed.tree_id(), work.tree_id());
        assert_eq!(claimed.entry_id(), work.entry_id());
        assert_eq!(claimed.graph().entries, work.graph().entries);
        assert_eq!(scheduler.pending_module_graph_ready_count(), 0);
    }

    #[test]
    fn module_graph_failed_work_is_claimed_by_document_script_runner() {
        let mut scheduler: DocumentScriptScheduler<
            FrameDocumentModuleGraphReadyTarget,
            std::convert::Infallible,
            TestModuleGraphFailure,
        > = DocumentScriptScheduler::default();

        let script = prepared_script(
            9,
            ScriptMode::ModuleDefer,
            ScriptKind::Module,
            ScriptSourceKind::External,
        );
        let pending_script_key = scheduler.register_module_script(&script);
        assert!(scheduler.watch_module_script(pending_script_key));
        assert!(scheduler.notify_module_script_graph_failed_action(
            pending_script_key,
            TestModuleGraphFailure::new(NodeId::new(9), "fetch failed"),
        ));

        assert_eq!(scheduler.pending_parser_module_failure_count(), 1);
        let claimed = scheduler
            .take_next_ready_work()
            .expect("claimed module graph-failed work should be queued");
        match claimed {
            DocumentScriptReadyWork::ModuleScriptGraphFailed(failure) => {
                assert_eq!(failure.message, "fetch failed");
            }
            other => panic!("expected graph-failed work, got {other:?}"),
        }
        assert_eq!(scheduler.pending_parser_module_failure_count(), 0);
    }

    #[test]
    fn parser_module_terminal_ready_work_uses_one_fifo_lane() {
        let mut scheduler: DocumentScriptScheduler<
            FrameDocumentModuleGraphReadyTarget,
            u64,
            TestModuleGraphFailure,
        > = DocumentScriptScheduler::default();
        let owner = crate::frame_owner_model::FrameDocumentTaskOwner::new(
            crate::frame_owner_model::FrameSchedulerLaneId(1),
            crate::frame_owner_model::LocalWindowId(2),
            crate::frame_owner_model::DocumentId(3),
        );
        let realm_id = crate::frame_owner_model::FrameRealmId(4);
        let request_key = crate::module_runtime::ModuleMapKey::java_script(
            Url::parse("https://scheduler.test/ordered-module.js").expect("module url"),
        );
        let script = prepared_script(
            5,
            ScriptMode::ModuleDefer,
            ScriptKind::Module,
            ScriptSourceKind::External,
        );
        let pending_script_id = ParserPendingScriptId::new(owner.document_owner(), &script);
        let work = DocumentModuleGraphReadyWork::new(
            owner,
            realm_id,
            pending_script_id,
            script,
            crate::document_runtime::DomHandle::new(5),
            request_key,
            moli_module_script_tree::ModuleTreeId(6),
            crate::frame_owner_model::DocumentLoadDelayTokenId(10),
            crate::module_runtime::ModuleGraphHandle {
                root_entry: crate::module_runtime::ModuleEntryId::from_raw(7),
                entries: vec![crate::module_runtime::ModuleEntryId::from_raw(7)],
            },
        );

        let ready_pending_script_key = scheduler.register_module_script(work.script());
        assert!(scheduler.watch_module_script(ready_pending_script_key));
        let failed_script = prepared_script(
            9,
            ScriptMode::ModuleDefer,
            ScriptKind::Module,
            ScriptSourceKind::External,
        );
        let failed_pending_script_key = scheduler.register_module_script(&failed_script);
        assert!(scheduler.watch_module_script(failed_pending_script_key));

        scheduler.notify_module_script_evaluation_completed(42);
        assert!(scheduler.notify_module_script_graph_ready_work(ready_pending_script_key, work));
        assert!(scheduler.notify_module_script_graph_failed_action(
            failed_pending_script_key,
            TestModuleGraphFailure::new(NodeId::new(9), "fetch failed"),
        ));

        assert_eq!(scheduler.pending_parser_module_evaluation_count(), 1);
        assert_eq!(scheduler.pending_module_graph_ready_count(), 1);
        assert_eq!(scheduler.pending_parser_module_failure_count(), 1);

        match scheduler
            .take_next_ready_work()
            .expect("evaluation completion should be first")
        {
            DocumentScriptReadyWork::ModuleScriptEvaluationCompleted(reaction_id) => {
                assert_eq!(*reaction_id, 42);
            }
            other => panic!("expected evaluation completion first, got {other:?}"),
        }
        assert!(
            scheduler
                .take_next_ready_work()
                .expect("graph ready should be second")
                .into_module_script_graph_ready()
                .graph()
                .entries
                .contains(&crate::module_runtime::ModuleEntryId::from_raw(7))
        );
        match scheduler
            .take_next_ready_work()
            .expect("graph failure should be third")
        {
            DocumentScriptReadyWork::ModuleScriptGraphFailed(failure) => {
                assert_eq!(failure.message, "fetch failed");
            }
            other => panic!("expected graph failure third, got {other:?}"),
        }
        assert!(scheduler.take_next_ready_work().is_none());
    }

    #[test]
    fn module_graph_failed_work_waits_for_module_script_watch() {
        let mut scheduler: DocumentScriptScheduler<
            FrameDocumentModuleGraphReadyTarget,
            std::convert::Infallible,
            TestModuleGraphFailure,
        > = DocumentScriptScheduler::default();
        let script = prepared_script(
            9,
            ScriptMode::ModuleDefer,
            ScriptKind::Module,
            ScriptSourceKind::External,
        );

        let pending_script_key = scheduler.register_module_script(&script);
        assert!(scheduler.notify_module_script_graph_failed_action(
            pending_script_key,
            TestModuleGraphFailure::new(script.node_id, "fetch failed"),
        ));

        assert_eq!(scheduler.pending_parser_module_script_count_for_test(), 1);
        assert_eq!(scheduler.pending_parser_module_failure_count(), 0);
        assert!(
            scheduler.take_next_ready_work().is_none(),
            "graph-failed terminal result should stay on the module-script pending entry until watched"
        );

        assert!(scheduler.watch_module_script(pending_script_key));
        assert_eq!(scheduler.pending_parser_module_script_count_for_test(), 0);
        assert_eq!(scheduler.pending_parser_module_failure_count(), 1);
        let claimed = scheduler
            .take_next_ready_work()
            .expect("watching a graph-failed pending script should enqueue failure work");
        match claimed {
            DocumentScriptReadyWork::ModuleScriptGraphFailed(failure) => {
                assert_eq!(failure.message, "fetch failed");
            }
            other => panic!("expected graph-failed work, got {other:?}"),
        }
        assert_eq!(scheduler.pending_parser_module_failure_count(), 0);
    }

    #[tokio::test]
    async fn claim_parser_post_parse_script_buckets_by_mode() {
        let mut scheduler = DocumentScriptScheduler::new();
        scheduler.claim_parser_non_async_post_parse_script(prepared_script(
            3,
            ScriptMode::Normal,
            ScriptKind::ImportMap,
            ScriptSourceKind::Inline,
        ));
        scheduler.claim_parser_non_async_post_parse_script(prepared_script(
            1,
            ScriptMode::Defer,
            ScriptKind::Module,
            ScriptSourceKind::Inline,
        ));
        let async_script = prepared_script(
            2,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        assert!(
            scheduler.on_parser_discovered_async_candidate_with_shared_load(
                async_script.clone(),
                Some(SharedScriptSourceLoad::ready_err(
                    "synthetic async prefetch miss",
                )),
            )
        );
        scheduler.claim_parse_time_async_handoff_with_shared_load(
            async_script,
            Some(SharedScriptSourceLoad::ready_err(
                "synthetic async handoff reuse",
            )),
        );

        assert_eq!(scheduler.seal_parser_deferred_scripts(), Ok(1));
        let defer_scripts = scheduler.parser_deferred_scripts_for_test();
        assert_eq!(defer_scripts.len(), 1);
        assert_eq!(defer_scripts[0].position, 1);
        let async_scripts = scheduler
            .finalize_owned_script_work()
            .await
            .into_async_tasks();
        assert_eq!(async_scripts.len(), 1);
        assert_eq!(async_scripts[0].position(), 2);
    }

    #[tokio::test]
    async fn claim_parser_post_parse_script_reports_parse_time_async_handoff_only_for_eligible_async()
     {
        let mut scheduler = DocumentScriptScheduler::new();
        let async_script = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        assert!(
            scheduler.on_parser_discovered_async_candidate_with_shared_load(
                async_script.clone(),
                Some(SharedScriptSourceLoad::ready_err(
                    "synthetic async discovery miss",
                )),
            )
        );
        let async_claim = scheduler.claim_parse_time_async_handoff_with_shared_load(
            async_script,
            Some(SharedScriptSourceLoad::ready_err(
                "synthetic async handoff reuse",
            )),
        );
        scheduler.claim_parser_non_async_post_parse_script(prepared_script(
            2,
            ScriptMode::Defer,
            ScriptKind::Module,
            ScriptSourceKind::Inline,
        ));

        assert!(async_claim);
    }

    #[tokio::test]
    async fn parser_discovered_async_can_reuse_ready_shared_preload() {
        let mut scheduler = DocumentScriptScheduler::new();
        let async_script = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let shared_load = SharedScriptSourceLoad::ready_ok("window.sharedAsync = 1;");

        assert!(
            scheduler.on_parser_discovered_async_candidate_with_shared_load(
                async_script.clone(),
                Some(shared_load.clone()),
            )
        );
        assert!(
            scheduler
                .claim_parse_time_async_handoff_with_shared_load(async_script, Some(shared_load),)
        );

        let turn = scheduler.parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
            default_chunk_bytes: 32,
        });
        assert!(matches!(
            turn.ready_task,
            Some(ParseTimeDocumentScriptTask::ClassicAsyncScript(script))
                if matches!(&script.source, ScriptSource::Loaded(source) if source == "window.sharedAsync = 1;")
        ));
    }

    #[tokio::test]
    async fn classic_defer_can_reuse_ready_shared_preload() {
        let mut scheduler = DocumentScriptScheduler::new();
        let defer_script = prepared_script(
            1,
            ScriptMode::Defer,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let shared_load = SharedScriptSourceLoad::ready_ok("window.sharedDefer = 1;");

        scheduler.claim_parser_non_async_post_parse_script_with_shared_load(
            defer_script,
            Some(shared_load),
        );
        assert_eq!(scheduler.seal_parser_deferred_scripts(), Ok(1));
        let defer_scripts = scheduler.parser_deferred_scripts_for_test();
        assert_eq!(defer_scripts.len(), 1);
        assert!(matches!(
            &defer_scripts[0].source,
            ScriptSource::Loaded(source) if source == "window.sharedDefer = 1;"
        ));
    }

    #[tokio::test]
    async fn classic_defer_source_failure_is_a_terminal_pending_script_result() {
        let mut scheduler = DocumentScriptScheduler::new();
        let defer_script = prepared_script(
            1,
            ScriptMode::Defer,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );

        scheduler.claim_parser_non_async_post_parse_script_with_shared_load(
            defer_script.clone(),
            Some(SharedScriptSourceLoad::ready_err("source failed once")),
        );
        assert_eq!(scheduler.seal_parser_deferred_scripts(), Ok(1));

        let ready = scheduler
            .take_next_after_parsing_ready_script()
            .expect("failed source must make its original PendingScript terminal");
        match ready {
            ParserDeferredScriptReady::Classic(ParserDeferredClassicReady::SourceFailure {
                script,
                error,
                source_network_result,
                ..
            }) => {
                assert_eq!(script.node_id, defer_script.node_id);
                assert_eq!(error, "source failed once");
                assert!(source_network_result.is_none());
                assert!(matches!(script.source, ScriptSource::External));
            }
            other => panic!("expected typed classic-defer source failure, got {other:?}"),
        }
        assert!(
            scheduler.take_next_after_parsing_ready_script().is_none(),
            "source failure must consume the original PendingScript exactly once"
        );
    }

    #[tokio::test]
    async fn external_module_defer_keeps_external_source_with_ready_shared_preload() {
        let mut scheduler = DocumentScriptScheduler::new();
        let module_script = prepared_script(
            1,
            ScriptMode::ModuleDefer,
            ScriptKind::Module,
            ScriptSourceKind::External,
        );
        let response = crate::types::NavigationResponse::from_head_and_text_body(
            moli_fetch::ResponseHead {
                final_url: module_script.url.clone(),
                status: 200,
                headers: vec![("content-type".to_owned(), "text/javascript".to_owned())],
                request_cookie_report: None,
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: true,
                negotiated_http_version: None,
            },
            "export default 1;".to_owned(),
        );
        let shared_load = SharedScriptSourceLoad::ready_outcome(
            Ok("export default 1;".to_owned()),
            Some(std::sync::Arc::new(Ok(response))),
        );

        scheduler.claim_parser_non_async_post_parse_script_with_shared_load(
            module_script,
            Some(shared_load),
        );
        assert_eq!(scheduler.seal_parser_deferred_scripts(), Ok(1));
        let defer_scripts = scheduler.parser_deferred_scripts_for_test();
        assert_eq!(defer_scripts.len(), 1);
        assert!(matches!(&defer_scripts[0].source, ScriptSource::External));
        assert!(
            defer_scripts[0].source_network_result().is_none(),
            "module graph fetch owns external module network provenance"
        );
    }

    #[tokio::test]
    async fn external_module_defer_does_not_wait_for_pending_shared_preload() {
        let mut scheduler = DocumentScriptScheduler::new();
        let module_script = prepared_script(
            1,
            ScriptMode::ModuleDefer,
            ScriptKind::Module,
            ScriptSourceKind::External,
        );
        let shared_load = SharedScriptSourceLoad::spawn_for_test(std::future::pending());

        scheduler.claim_parser_non_async_post_parse_script_with_shared_load(
            module_script,
            Some(shared_load),
        );
        assert_eq!(scheduler.seal_parser_deferred_scripts(), Ok(1));
        let defer_scripts = scheduler.parser_deferred_scripts_for_test();
        assert_eq!(defer_scripts.len(), 1);
        assert!(matches!(&defer_scripts[0].source, ScriptSource::External));
        assert!(
            defer_scripts[0].source_network_result().is_none(),
            "pending script text preload must not own module graph network provenance"
        );
    }

    #[tokio::test]
    async fn classic_defer_without_shared_preload_returns_source_start_work() {
        let mut scheduler = DocumentScriptScheduler::new();
        let claim = scheduler
            .claim_parser_non_async_post_parse_script_with_shared_load_and_document_character_set(
                prepared_external_data_script(1, ScriptMode::Defer),
                None,
                None,
                Default::default(),
                crate::frame_owner_model::DocumentLoadDelayTokenId(1),
            )
            .expect("classic defer should be accepted");

        assert!(
            claim.into_classic_source_load().is_some(),
            "the runtime adapter, not the scheduler unit test, owns starting missing source work"
        );
    }

    #[tokio::test]
    async fn external_module_defer_without_shared_preload_does_not_spawn_source_load() {
        let mut scheduler = DocumentScriptScheduler::new();
        let mut script = prepared_script(
            1,
            ScriptMode::ModuleDefer,
            ScriptKind::Module,
            ScriptSourceKind::External,
        );
        script.url =
            Url::parse("data:text/javascript,export%20default%201%3B").expect("data module url");

        scheduler.claim_parser_non_async_post_parse_script_with_shared_load(script, None);

        assert_eq!(scheduler.seal_parser_deferred_scripts(), Ok(1));
        let defer_scripts = scheduler.parser_deferred_scripts_for_test();
        assert_eq!(defer_scripts.len(), 1);
        assert!(matches!(&defer_scripts[0].source, ScriptSource::External));
    }

    #[tokio::test]
    async fn loaded_external_module_defer_does_not_spawn_fallback_shared_load() {
        let mut scheduler = DocumentScriptScheduler::new();
        let mut script = prepared_script(
            1,
            ScriptMode::ModuleDefer,
            ScriptKind::Module,
            ScriptSourceKind::External,
        );
        script.source = ScriptSource::Loaded("export default 1;".to_owned());

        scheduler.claim_parser_non_async_post_parse_script_with_shared_load(script, None);

        assert_eq!(scheduler.seal_parser_deferred_scripts(), Ok(1));
        let defer_scripts = scheduler.parser_deferred_scripts_for_test();
        assert_eq!(defer_scripts.len(), 1);
        assert!(matches!(
            &defer_scripts[0].source,
            ScriptSource::Loaded(source) if source == "export default 1;"
        ));
        assert!(
            defer_scripts[0].source_network_result().is_none(),
            "parser turn owns the network event for preload-applied loaded sources"
        );
    }

    #[tokio::test]
    async fn parser_discovered_async_without_shared_preload_starts_through_source_load_port() {
        let mut scheduler = DocumentScriptScheduler::new();
        let starts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_starts = starts.clone();
        let source_load_port = source_load_port::DocumentScriptSourceLoadPort::new(move |_, _| {
            observed_starts.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            SharedScriptSourceLoad::ready_err("synthetic source-load port terminal")
        });

        assert!(
            scheduler
                .runner
                .on_parser_discovered_async_candidate_with_source_load_port(
                    prepared_external_data_script(1, ScriptMode::Async),
                    &source_load_port,
                    None,
                    None,
                    |_| None,
                )
        );
        assert_eq!(
            starts.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "a missing preload should start exactly once through the injected source-load port"
        );
    }

    // -----------------------------------------------------------------------
    // Finalize plan ordering
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn finalize_plan_preserves_parser_owned_defer_order() {
        let mut scheduler = DocumentScriptScheduler::new();
        scheduler.claim_parser_post_parse_script(prepared_script(
            5,
            ScriptMode::Defer,
            ScriptKind::Module,
            ScriptSourceKind::Inline,
        ));
        scheduler.claim_parser_post_parse_script(prepared_script(
            1,
            ScriptMode::Defer,
            ScriptKind::Module,
            ScriptSourceKind::Inline,
        ));
        scheduler.claim_parser_post_parse_script(prepared_script(
            3,
            ScriptMode::Defer,
            ScriptKind::Module,
            ScriptSourceKind::Inline,
        ));

        assert_eq!(scheduler.seal_parser_deferred_scripts(), Ok(3));
        let defer_scripts = scheduler.parser_deferred_scripts_for_test();

        let positions = defer_scripts
            .iter()
            .map(|script| script.position)
            .collect::<Vec<_>>();
        assert_eq!(positions, vec![1, 3, 5]);
    }

    #[tokio::test]
    async fn finalize_plan_keeps_direct_classic_defer_and_fallback_defer_in_document_order() {
        let mut scheduler = DocumentScriptScheduler::new();
        scheduler.claim_parser_non_async_post_parse_script_with_shared_load(
            prepared_script(
                5,
                ScriptMode::Defer,
                ScriptKind::Classic,
                ScriptSourceKind::External,
            ),
            Some(SharedScriptSourceLoad::ready_ok("window.deferA = 1;")),
        );
        scheduler.claim_parser_post_parse_script(prepared_script(
            1,
            ScriptMode::Defer,
            ScriptKind::Module,
            ScriptSourceKind::Inline,
        ));
        scheduler.claim_parser_post_parse_script(prepared_script(
            3,
            ScriptMode::Defer,
            ScriptKind::Module,
            ScriptSourceKind::Inline,
        ));

        assert_eq!(scheduler.seal_parser_deferred_scripts(), Ok(3));
        let defer_scripts = scheduler.parser_deferred_scripts_for_test();

        let positions = defer_scripts
            .iter()
            .map(|script| script.position)
            .collect::<Vec<_>>();
        assert_eq!(positions, vec![1, 3, 5]);
        assert!(matches!(
            &defer_scripts[2].source,
            ScriptSource::Loaded(source) if source == "window.deferA = 1;"
        ));
    }

    #[tokio::test]
    async fn finalize_plan_merges_classic_defer_and_module_in_one_document_ordered_phase() {
        let mut scheduler = DocumentScriptScheduler::new();
        scheduler.claim_parser_non_async_post_parse_script_with_shared_load(
            prepared_script(
                1,
                ScriptMode::Defer,
                ScriptKind::Classic,
                ScriptSourceKind::External,
            ),
            Some(SharedScriptSourceLoad::ready_ok(
                "window.deferClassicA = 1;",
            )),
        );
        scheduler.claim_parser_post_parse_script(prepared_script(
            2,
            ScriptMode::Defer,
            ScriptKind::Module,
            ScriptSourceKind::Inline,
        ));
        scheduler.claim_parser_non_async_post_parse_script_with_shared_load(
            prepared_script(
                3,
                ScriptMode::Defer,
                ScriptKind::Classic,
                ScriptSourceKind::External,
            ),
            Some(SharedScriptSourceLoad::ready_err(
                "synthetic defer preload failure",
            )),
        );

        assert_eq!(scheduler.seal_parser_deferred_scripts(), Ok(3));
        let defer_scripts = scheduler.parser_deferred_scripts_for_test();
        let positions = defer_scripts
            .iter()
            .map(|script| script.position)
            .collect::<Vec<_>>();
        assert_eq!(positions, vec![1, 2, 3]);
        assert!(matches!(
            &defer_scripts[0].source,
            ScriptSource::Loaded(source) if source == "window.deferClassicA = 1;"
        ));
        assert!(matches!(&defer_scripts[2].source, ScriptSource::External));
    }

    // -----------------------------------------------------------------------
    // Synchronous readiness: parse_time_turn is now a simple check
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn parse_time_turn_returns_ready_task_synchronously() {
        let mut scheduler = scheduler_with_async_state(Vec::new(), Vec::new());
        scheduler
            .runner
            .async_parse_time_queue
            .ready_tasks
            .push_back(ParseTimeDocumentScriptTask::classic_async_script_for_test(
                prepared_script(
                    1,
                    ScriptMode::Async,
                    ScriptKind::Classic,
                    ScriptSourceKind::External,
                )
                .with_loaded_source("ready-source".to_owned()),
            ));

        let turn = scheduler.parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
            default_chunk_bytes: 4096,
        });

        assert_eq!(turn.parser_step_bytes, Some(4096));
        assert!(matches!(
            &turn.ready_task,
            Some(ParseTimeDocumentScriptTask::ClassicAsyncScript(script))
                if matches!(&script.source, ScriptSource::Loaded(source) if source == "ready-source")
        ));
    }

    #[tokio::test]
    async fn parse_time_turn_returns_none_when_nothing_ready() {
        let mut scheduler = scheduler_with_async_state(
            vec![parse_time_async_entry(prepared_script(
                1,
                ScriptMode::Async,
                ScriptKind::Classic,
                ScriptSourceKind::External,
            ))],
            Vec::new(),
        );

        let turn = scheduler.parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
            default_chunk_bytes: 4096,
        });

        assert_eq!(turn.parser_step_bytes, Some(4096));
        assert!(turn.ready_task.is_none());
    }

    #[tokio::test]
    async fn parse_time_turn_without_pending_async_returns_none() {
        let mut scheduler = DocumentScriptScheduler::new();
        let turn = scheduler.parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
            default_chunk_bytes: 4096,
        });
        assert_eq!(turn.parser_step_bytes, Some(4096));
        assert!(turn.ready_task.is_none());
    }

    // -----------------------------------------------------------------------
    // Readiness via apply_completion
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn apply_completion_makes_entry_ready() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let node_id = original.node_id;
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        scheduler
            .runner
            .async_parse_time_queue
            .apply_completion(async_load_completion_ok(node_id, "ready-source"));

        let turn = scheduler.parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
            default_chunk_bytes: 4096,
        });
        assert_eq!(
            scheduler
                .runner
                .async_parse_time_queue
                .parse_time_entries
                .len(),
            0
        );
        assert!(matches!(
            &turn.ready_task,
            Some(ParseTimeDocumentScriptTask::ClassicAsyncScript(script))
                if matches!(&script.source, ScriptSource::Loaded(source) if source == "ready-source")
        ));
    }

    #[tokio::test]
    async fn accept_injected_completion_returns_ready_task() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let node_id = original.node_id;
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        let (task, ready_task_enqueued) = scheduler
            .runner
            .async_parse_time_queue
            .accept_injected_completion(async_load_completion_ok(node_id, "injected-ready"));

        assert!(ready_task_enqueued);
        assert!(matches!(
            &task,
            Some(ParseTimeDocumentScriptTask::ClassicAsyncScript(script))
                if matches!(&script.source, ScriptSource::Loaded(source) if source == "injected-ready")
        ));
    }

    #[tokio::test]
    async fn completion_after_parse_time_cutoff_falls_back_instead_of_reopening_parse_visible_turn()
    {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let node_id = original.node_id;
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        scheduler.seal_parse_time_async_cutoff();
        let task = scheduler.accept_injected_parse_time_async_completion(
            node_id,
            source_load_outcome_ok("late-ready"),
        );

        assert!(task.is_none());
        assert!(scheduler.parse_time_async_cutoff_sealed());
        assert!(!scheduler.has_pending_parse_time_async_entries());
        assert_eq!(scheduler.runner.async_fallback_queue.entries.len(), 1);
        assert!(matches!(
            &scheduler.runner.async_fallback_queue.entries[0].script.source,
            ScriptSource::Loaded(source) if source == "late-ready"
        ));
    }

    #[tokio::test]
    async fn closing_parse_visible_lane_moves_pending_entries_to_fallback_owner() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        scheduler.seal_parse_visible_async_cutoff();

        assert!(!scheduler.has_pending_parse_time_async_entries());
        assert_eq!(scheduler.runner.async_fallback_queue.entries.len(), 1);
        assert!(scheduler.runner.async_fallback_queue.entries[0].awaiting_completion);
    }

    #[tokio::test]
    async fn closing_parse_visible_lane_moves_ready_tasks_to_fallback_owner() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let mut scheduler = scheduler_with_async_state(Vec::new(), Vec::new());
        scheduler
            .runner
            .async_parse_time_queue
            .enqueue_ready_task_in_completion_order(
                ParseTimeDocumentScriptTask::classic_async_script_for_test(
                    original
                        .clone()
                        .with_loaded_source("ready-before-seal".to_owned()),
                ),
            );

        scheduler.seal_parse_visible_async_cutoff();

        assert!(
            scheduler
                .runner
                .async_parse_time_queue
                .ready_tasks
                .is_empty()
        );
        assert_eq!(scheduler.runner.async_fallback_queue.entries.len(), 1);
        assert!(matches!(
            &scheduler.runner.async_fallback_queue.entries[0].script.source,
            ScriptSource::Loaded(source) if source == "ready-before-seal"
        ));
    }

    #[tokio::test]
    async fn late_completion_after_lane_close_updates_fallback_owner_entry() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let node_id = original.node_id;
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        scheduler.seal_parse_visible_async_cutoff();
        let task = scheduler.accept_injected_parse_time_async_completion(
            node_id,
            source_load_outcome_ok("late-ready"),
        );

        assert!(task.is_none());
        assert_eq!(scheduler.runner.async_fallback_queue.entries.len(), 1);
        assert!(!scheduler.runner.async_fallback_queue.entries[0].awaiting_completion);
        assert!(matches!(
            &scheduler.runner.async_fallback_queue.entries[0].script.source,
            ScriptSource::Loaded(source) if source == "late-ready"
        ));
    }

    // -----------------------------------------------------------------------
    // Completion-order readiness queue
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn ready_async_tasks_are_enqueued_in_completion_order() {
        let mut scheduler = scheduler_with_async_state(
            vec![
                parse_time_async_entry(prepared_script(
                    1,
                    ScriptMode::Async,
                    ScriptKind::Classic,
                    ScriptSourceKind::External,
                )),
                parse_time_async_entry(prepared_script(
                    2,
                    ScriptMode::Async,
                    ScriptKind::Classic,
                    ScriptSourceKind::External,
                )),
            ],
            Vec::new(),
        );

        // Async scripts are not defer scripts: once the parser has handed them
        // off, their execution order follows readiness/completion, not DOM
        // position. This is observable in WPT execution-timing/086.html.
        scheduler
            .runner
            .async_parse_time_queue
            .apply_completion(async_load_completion_ok(
                NodeId::new(3),
                "second-ready-first",
            ));
        scheduler
            .runner
            .async_parse_time_queue
            .apply_completion(async_load_completion_ok(
                NodeId::new(2),
                "first-ready-second",
            ));

        let first = scheduler
            .parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
                default_chunk_bytes: 4096,
            })
            .ready_task;
        let second = scheduler
            .parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
                default_chunk_bytes: 4096,
            })
            .ready_task;
        let third = scheduler
            .parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
                default_chunk_bytes: 4096,
            })
            .ready_task;

        assert!(matches!(
            &first,
            Some(ParseTimeDocumentScriptTask::ClassicAsyncScript(script))
                if script.position == 2
                    && matches!(&script.source, ScriptSource::Loaded(source) if source == "second-ready-first")
        ));
        assert!(matches!(
            &second,
            Some(ParseTimeDocumentScriptTask::ClassicAsyncScript(script))
                if script.position == 1
                    && matches!(&script.source, ScriptSource::Loaded(source) if source == "first-ready-second")
        ));
        assert!(third.is_none());
    }

    // -----------------------------------------------------------------------
    // Trigger chaining: AfterClassicAsyncTaskExecuted can chain ready tasks
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn after_task_executed_can_chain_next_ready_async_task() {
        let mut scheduler = scheduler_with_async_state(
            vec![
                parse_time_async_entry(prepared_script(
                    1,
                    ScriptMode::Async,
                    ScriptKind::Classic,
                    ScriptSourceKind::External,
                )),
                parse_time_async_entry(prepared_script(
                    2,
                    ScriptMode::Async,
                    ScriptKind::Classic,
                    ScriptSourceKind::External,
                )),
            ],
            Vec::new(),
        );

        scheduler
            .runner
            .async_parse_time_queue
            .apply_completion(async_load_completion_ok(NodeId::new(2), "first-ready"));
        let first = scheduler
            .parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
                default_chunk_bytes: 4096,
            })
            .ready_task;
        assert!(matches!(
            &first,
            Some(ParseTimeDocumentScriptTask::ClassicAsyncScript(script))
                if script.position == 1
                    && matches!(&script.source, ScriptSource::Loaded(source) if source == "first-ready")
        ));

        // Second completion arrives, follow-up turn picks it up without parser progress
        scheduler
            .runner
            .async_parse_time_queue
            .apply_completion(async_load_completion_ok(NodeId::new(3), "second-ready"));
        let second = scheduler
            .parse_time_turn(ParseTimeTurnTrigger::AfterClassicAsyncTaskExecuted)
            .ready_task;

        assert!(matches!(
            &second,
            Some(ParseTimeDocumentScriptTask::ClassicAsyncScript(script))
                if script.position == 2
                    && matches!(&script.source, ScriptSource::Loaded(source) if source == "second-ready")
        ));
    }

    #[tokio::test]
    async fn buffered_ready_tasks_drain_one_per_turn() {
        let mut scheduler = scheduler_with_async_state(
            vec![
                parse_time_async_entry(prepared_script(
                    1,
                    ScriptMode::Async,
                    ScriptKind::Classic,
                    ScriptSourceKind::External,
                )),
                parse_time_async_entry(prepared_script(
                    2,
                    ScriptMode::Async,
                    ScriptKind::Classic,
                    ScriptSourceKind::External,
                )),
            ],
            Vec::new(),
        );

        // Both complete before any turn
        scheduler
            .runner
            .async_parse_time_queue
            .apply_completion(async_load_completion_ok(
                NodeId::new(3),
                "second-ready-first",
            ));
        scheduler
            .runner
            .async_parse_time_queue
            .apply_completion(async_load_completion_ok(
                NodeId::new(2),
                "first-ready-second",
            ));

        let first = scheduler
            .parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
                default_chunk_bytes: 4096,
            })
            .ready_task;
        assert!(matches!(
            &first,
            Some(ParseTimeDocumentScriptTask::ClassicAsyncScript(script)) if script.position == 2
        ));

        let second = scheduler
            .parse_time_turn(ParseTimeTurnTrigger::AfterClassicAsyncTaskExecuted)
            .ready_task;
        assert!(matches!(
            &second,
            Some(ParseTimeDocumentScriptTask::ClassicAsyncScript(script)) if script.position == 1
        ));

        let third = scheduler
            .parse_time_turn(ParseTimeTurnTrigger::AfterClassicAsyncTaskExecuted)
            .ready_task;
        assert!(third.is_none());
    }

    // -----------------------------------------------------------------------
    // Discovery-before-handoff completion adoption
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn claim_parser_discovered_async_adopts_completion_recorded_before_handoff() {
        let mut scheduler = DocumentScriptScheduler::new();
        let script = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        assert!(
            scheduler.on_parser_discovered_async_candidate_with_shared_load(
                script.clone(),
                Some(SharedScriptSourceLoad::ready_err(
                    "synthetic async discovery miss",
                )),
            )
        );
        scheduler
            .runner
            .async_parse_time_queue
            .apply_completion(async_load_completion_ok(script.node_id, "spec-ready"));

        let disposition = scheduler.claim_parser_post_parse_script(script);
        assert_eq!(
            disposition,
            PostParseScriptClaimDisposition::ParseTimeAsyncClaimedAtHandoff
        );

        let turn = scheduler.parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
            default_chunk_bytes: 4096,
        });
        assert_eq!(turn.parser_step_bytes, Some(4096));
        assert!(matches!(
            turn.ready_task,
            Some(ParseTimeDocumentScriptTask::ClassicAsyncScript(script))
                if matches!(&script.source, ScriptSource::Loaded(source) if source == "spec-ready")
        ));
    }
    #[tokio::test]
    async fn before_parser_step_preserves_default_chunk_bytes() {
        let mut scheduler = scheduler_with_async_state(
            vec![parse_time_async_entry(prepared_script(
                1,
                ScriptMode::Async,
                ScriptKind::Classic,
                ScriptSourceKind::External,
            ))],
            Vec::new(),
        );

        let turn = scheduler.parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
            default_chunk_bytes: 8192,
        });
        assert_eq!(turn.parser_step_bytes, Some(8192));
        assert!(turn.ready_task.is_none());
    }

    #[tokio::test]
    async fn non_before_parser_step_triggers_have_no_parser_step_bytes() {
        let mut scheduler = scheduler_with_async_state(Vec::new(), Vec::new());

        let executed_turn =
            scheduler.parse_time_turn(ParseTimeTurnTrigger::AfterClassicAsyncTaskExecuted);
        assert_eq!(executed_turn.parser_step_bytes, None);
    }

    #[tokio::test]
    async fn ready_task_at_before_parser_step_still_reports_full_chunk() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let node_id = original.node_id;
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        scheduler
            .runner
            .async_parse_time_queue
            .apply_completion(async_load_completion_ok(node_id, "ready-before-next-step"));

        let turn = scheduler.parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
            default_chunk_bytes: 4096,
        });
        assert_eq!(turn.parser_step_bytes, Some(4096));
        assert!(turn.ready_task.is_some());
    }

    // -----------------------------------------------------------------------
    // Fallback queue isolation
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fallback_entries_do_not_appear_as_parse_time_ready() {
        let pending = prepared_script(
            2,
            ScriptMode::Async,
            ScriptKind::Module,
            ScriptSourceKind::Inline,
        );

        let mut scheduler = scheduler_with_async_state(Vec::new(), vec![pending]);

        let turn = scheduler.parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
            default_chunk_bytes: 4096,
        });
        assert_eq!(turn.parser_step_bytes, Some(4096));
        assert!(turn.ready_task.is_none());
    }

    #[tokio::test]
    async fn ready_task_moves_out_while_fallback_entries_remain() {
        let pending = prepared_script(
            2,
            ScriptMode::Async,
            ScriptKind::Module,
            ScriptSourceKind::Inline,
        );

        let mut scheduler = scheduler_with_async_state(Vec::new(), vec![pending]);
        scheduler
            .runner
            .async_parse_time_queue
            .ready_tasks
            .push_back(ParseTimeDocumentScriptTask::classic_async_script_for_test(
                prepared_script(
                    1,
                    ScriptMode::Async,
                    ScriptKind::Classic,
                    ScriptSourceKind::External,
                )
                .with_loaded_source("ready-source".to_owned()),
            ));

        let turn = scheduler.parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
            default_chunk_bytes: 4096,
        });

        assert_eq!(
            scheduler
                .runner
                .async_parse_time_queue
                .parse_time_entries
                .len(),
            0
        );
        assert_eq!(scheduler.runner.async_fallback_queue.entries.len(), 1);
        assert!(matches!(
            &turn.ready_task,
            Some(ParseTimeDocumentScriptTask::ClassicAsyncScript(script))
                if matches!(&script.source, ScriptSource::Loaded(source) if source == "ready-source")
        ));
    }

    // -----------------------------------------------------------------------
    // Finalize: remaining async handed back correctly
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn finalize_plan_hands_back_ready_completions_before_post_dcl_fallback() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let mut scheduler =
            scheduler_with_async_state(vec![discovered_async_entry(original.clone())], Vec::new());

        // Simulate completion arriving before finalize
        scheduler
            .runner
            .async_parse_time_queue
            .apply_completion(async_load_completion_ok(
                original.node_id,
                "ready-before-finalize",
            ));

        let async_scripts = scheduler
            .finalize_owned_script_work()
            .await
            .into_async_tasks();
        assert_eq!(async_scripts.len(), 1);
        assert!(matches!(
            &async_scripts[0].as_script().expect("async task script").source,
            ScriptSource::Loaded(source) if source == "ready-before-finalize"
        ));
    }

    #[tokio::test]
    async fn finalize_plan_hands_back_failed_completion_as_terminal_async_failure() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let mut scheduler =
            scheduler_with_async_state(vec![discovered_async_entry(original.clone())], Vec::new());

        scheduler
            .runner
            .async_parse_time_queue
            .apply_completion(async_load_completion_err(
                original.node_id,
                "synthetic prefetch failure",
            ));

        let async_scripts = scheduler
            .finalize_owned_script_work()
            .await
            .into_async_tasks();
        assert_eq!(async_scripts.len(), 1);
        assert!(async_scripts[0].is_async_script_failure());
        assert!(matches!(
            &async_scripts[0]
                .as_script()
                .expect("async failure task script")
                .source,
            ScriptSource::External
        ));
    }

    #[tokio::test]
    async fn finalize_plan_preserves_incomplete_entries_as_originals() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original.clone())], Vec::new());

        let async_scripts = scheduler
            .finalize_owned_script_work()
            .await
            .into_async_tasks();

        // Incomplete test entries without a retained load handle still fall
        // back to the original external script form.
        assert_eq!(async_scripts.len(), 1);
        assert!(matches!(
            &async_scripts[0]
                .as_script()
                .expect("async task script")
                .source,
            ScriptSource::External
        ));
    }

    #[tokio::test]
    async fn finalize_plan_preserves_pending_async_source_load_as_non_ready_page_task() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let source_load = SharedScriptSourceLoad::spawn_for_test(std::future::pending());
        let scheduler = scheduler_with_async_state(
            vec![discovered_async_entry_with_source_load(
                original.clone(),
                source_load,
            )],
            Vec::new(),
        );

        let async_scripts = scheduler
            .finalize_owned_script_work()
            .await
            .into_async_tasks();
        assert_eq!(async_scripts.len(), 1);
        assert!(
            async_scripts[0].is_waiting_for_source_load(),
            "pending parse-time async fetches must not become immediately executable post-DCL tasks"
        );
    }

    // -----------------------------------------------------------------------
    // Completion for unknown node is ignored
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn completion_for_unknown_entry_is_ignored() {
        let mut scheduler = scheduler_with_async_state(Vec::new(), Vec::new());

        scheduler
            .runner
            .async_parse_time_queue
            .apply_completion(async_load_completion_ok(NodeId::new(99), "orphan"));

        assert!(
            scheduler
                .runner
                .async_parse_time_queue
                .parse_time_entries
                .is_empty()
        );
        assert!(
            scheduler
                .runner
                .async_parse_time_queue
                .ready_tasks
                .is_empty()
        );
        let turn = scheduler.parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
            default_chunk_bytes: 4096,
        });
        assert!(turn.ready_task.is_none());
    }

    #[test]
    fn pending_claimed_parse_time_async_entries_excludes_unclaimed_discovery_entries() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let scheduler =
            scheduler_with_async_state(vec![discovered_async_entry(original)], Vec::new());

        assert!(scheduler.has_pending_parse_time_async_entries());
        assert!(!scheduler.has_parse_visible_pending_claimed_async());
    }

    #[test]
    fn pending_claimed_parse_time_async_entries_tracks_claimed_handoff_entries() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        assert!(scheduler.has_pending_parse_time_async_entries());
        assert!(scheduler.has_parse_visible_pending_claimed_async());
    }

    #[test]
    fn parse_visible_pending_claimed_async_requires_open_lane() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        assert!(scheduler.has_parse_visible_pending_claimed_async());
        scheduler.seal_parse_time_async_cutoff();
        assert!(!scheduler.has_parse_visible_pending_claimed_async());
    }

    #[test]
    fn outstanding_reevaluation_credit_requires_open_lane_and_pending_claimed_async() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        assert!(!scheduler.has_outstanding_parse_visible_reevaluation_credit());
        assert_eq!(
            scheduler.grant_parse_visible_reevaluation_credit(),
            ParseVisibleReevaluationCreditGrant::Granted
        );
        assert!(scheduler.has_outstanding_parse_visible_reevaluation_credit());
        scheduler.seal_parse_time_async_cutoff();
        assert!(!scheduler.has_outstanding_parse_visible_reevaluation_credit());
    }

    #[test]
    fn parse_visible_pending_claimed_async_does_not_imply_outstanding_reevaluation_credit() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        assert!(scheduler.has_parse_visible_pending_claimed_async());
        assert!(!scheduler.has_outstanding_parse_visible_reevaluation_credit());
    }

    #[test]
    fn parse_visible_ready_turn_planner_waits_only_after_first_drain_and_pending_wait() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        assert_eq!(
            scheduler.plan_parse_visible_ready_turn(ParseVisibleReadyTurnPhase::Parsing, false),
            ParseVisibleReadyTurnDisposition::DrainReadyTasks
        );
        assert_eq!(
            scheduler.plan_parse_visible_ready_turn(ParseVisibleReadyTurnPhase::Parsing, true),
            ParseVisibleReadyTurnDisposition::FinishNoTask
        );

        assert_eq!(
            scheduler.grant_parse_visible_reevaluation_credit(),
            ParseVisibleReevaluationCreditGrant::Granted
        );

        assert_eq!(
            scheduler.plan_parse_visible_ready_turn(ParseVisibleReadyTurnPhase::Parsing, true),
            ParseVisibleReadyTurnDisposition::YieldToParserBoundary
        );
    }

    #[test]
    fn claimed_async_does_not_grant_reevaluation_credit_without_handoff_miss() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        assert!(!scheduler.has_outstanding_parse_visible_reevaluation_credit());

        assert!(!scheduler.has_outstanding_parse_visible_reevaluation_credit());
    }

    #[test]
    fn parse_visible_reevaluation_credit_only_arms_for_claimed_entries() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let mut scheduler =
            scheduler_with_async_state(vec![discovered_async_entry(original)], Vec::new());

        assert_eq!(
            scheduler.grant_parse_visible_reevaluation_credit(),
            ParseVisibleReevaluationCreditGrant::NotGranted(
                ParseVisibleReevaluationCreditGrantRefusalReason::NoPendingClaimedAsync,
            )
        );
    }

    #[test]
    fn parse_visible_reevaluation_credit_is_consumed_once() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        assert_eq!(
            scheduler.grant_parse_visible_reevaluation_credit(),
            ParseVisibleReevaluationCreditGrant::Granted
        );

        assert_eq!(
            scheduler.grant_parse_visible_reevaluation_credit(),
            ParseVisibleReevaluationCreditGrant::NotGranted(
                ParseVisibleReevaluationCreditGrantRefusalReason::AlreadyArmed,
            )
        );
        scheduler.consume_parse_visible_reevaluation_credit();
        assert!(!scheduler.has_outstanding_parse_visible_reevaluation_credit());
    }

    #[test]
    fn parse_visible_reevaluation_credit_persists_across_parser_boundaries_until_consumed() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        assert_eq!(
            scheduler.grant_parse_visible_reevaluation_credit(),
            ParseVisibleReevaluationCreditGrant::Granted
        );

        assert!(scheduler.has_outstanding_parse_visible_reevaluation_credit());
        assert_eq!(
            scheduler.plan_parse_visible_ready_turn(ParseVisibleReadyTurnPhase::Parsing, true),
            ParseVisibleReadyTurnDisposition::YieldToParserBoundary
        );
        assert_eq!(
            scheduler.plan_parse_visible_ready_turn(ParseVisibleReadyTurnPhase::Parsing, true),
            ParseVisibleReadyTurnDisposition::YieldToParserBoundary
        );
    }

    #[test]
    fn outstanding_reevaluation_credit_is_consumed_when_claimed_async_completion_enqueues_ready_task()
     {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let node_id = original.node_id;
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        assert_eq!(
            scheduler.grant_parse_visible_reevaluation_credit(),
            ParseVisibleReevaluationCreditGrant::Granted
        );
        assert!(scheduler.has_outstanding_parse_visible_reevaluation_credit());

        let _ = scheduler.accept_injected_parse_time_async_completion(
            node_id,
            source_load_outcome_ok("ready-after-handoff"),
        );

        assert!(!scheduler.has_outstanding_parse_visible_reevaluation_credit());
    }

    #[test]
    fn outstanding_reevaluation_credit_is_dropped_when_parse_time_cutoff_closes() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        assert_eq!(
            scheduler.grant_parse_visible_reevaluation_credit(),
            ParseVisibleReevaluationCreditGrant::Granted
        );
        scheduler.seal_parse_time_async_cutoff();

        assert_eq!(
            scheduler.grant_parse_visible_reevaluation_credit(),
            ParseVisibleReevaluationCreditGrant::NotGranted(
                ParseVisibleReevaluationCreditGrantRefusalReason::LaneClosed,
            )
        );
        assert!(!scheduler.has_outstanding_parse_visible_reevaluation_credit());
    }

    #[test]
    fn parse_visible_reevaluation_credit_reason_is_handoff_claim_without_progress() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        scheduler.grant_parse_visible_reevaluation_credit();

        assert!(matches!(
            scheduler.runner.parse_visible_async_lane_state,
            ParseVisibleAsyncLaneState::Open
        ));
        assert!(matches!(
            scheduler.runner.parse_visible_async_reevaluation_credit,
            ParseVisibleAsyncReevaluationCredit::Outstanding(
                ParseVisibleReevaluationCreditReason::ClaimedParseVisibleAsyncWithoutImmediateProgress
            )
        ));
    }

    // -----------------------------------------------------------------------
    // Failed load still produces a terminal ready task.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn completion_error_enqueues_parse_time_failure_task_without_refetching_external_source()
    {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let node_id = original.node_id;
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        scheduler
            .runner
            .async_parse_time_queue
            .apply_completion(async_load_completion_err(
                node_id,
                "synthetic async load failure",
            ));

        let turn = scheduler.parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
            default_chunk_bytes: 4096,
        });
        assert!(matches!(
            &turn.ready_task,
            Some(ParseTimeDocumentScriptTask::AsyncScriptFailure(task))
                if matches!(&task.script().source, ScriptSource::External)
        ));
    }

    #[tokio::test]
    async fn failed_completion_after_parse_time_cutoff_stays_terminal_in_fallback_queue() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let node_id = original.node_id;
        let mut scheduler =
            scheduler_with_async_state(vec![parse_time_async_entry(original)], Vec::new());

        scheduler.seal_parse_time_async_cutoff();
        let task = scheduler.accept_injected_parse_time_async_completion(
            node_id,
            source_load_outcome_err("late prefetch failure"),
        );

        assert!(task.is_none());
        assert_eq!(scheduler.runner.async_fallback_queue.entries.len(), 1);
        assert!(!scheduler.runner.async_fallback_queue.entries[0].awaiting_completion);
        let async_scripts = scheduler
            .finalize_owned_script_work()
            .await
            .into_async_tasks();
        assert_eq!(async_scripts.len(), 1);
        assert!(async_scripts[0].is_async_script_failure());
        assert!(matches!(
            &async_scripts[0]
                .as_script()
                .expect("async failure task script")
                .source,
            ScriptSource::External
        ));
    }

    #[tokio::test]
    async fn completion_recorded_before_handoff_becomes_ready_when_claimed() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let node_id = original.node_id;
        let mut scheduler =
            scheduler_with_async_state(vec![discovered_async_entry(original.clone())], Vec::new());

        scheduler
            .runner
            .async_parse_time_queue
            .apply_completion(async_load_completion_ok(node_id, "ready-before-handoff"));

        let disposition = scheduler.claim_parser_post_parse_script(original);
        assert_eq!(
            disposition,
            PostParseScriptClaimDisposition::ParseTimeAsyncClaimedAtHandoff
        );

        let turn = scheduler.parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
            default_chunk_bytes: 4096,
        });
        assert!(matches!(
            turn.ready_task,
            Some(ParseTimeDocumentScriptTask::ClassicAsyncScript(script))
                if matches!(&script.source, ScriptSource::Loaded(source) if source == "ready-before-handoff")
        ));
        assert_eq!(turn.parser_step_bytes, Some(4096));
        assert!(
            scheduler
                .runner
                .async_parse_time_queue
                .parse_time_entries
                .is_empty()
        );
    }

    #[tokio::test]
    async fn handoff_activation_reuses_parser_discovered_async_entry() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let mut scheduler =
            scheduler_with_async_state(vec![discovered_async_entry(original.clone())], Vec::new());

        let disposition = scheduler.claim_parser_post_parse_script(original.clone());

        assert_eq!(
            disposition,
            PostParseScriptClaimDisposition::ParseTimeAsyncClaimedAtHandoff
        );
        assert_eq!(
            scheduler
                .runner
                .async_parse_time_queue
                .parse_time_entries
                .len(),
            1
        );
        assert_eq!(
            scheduler.runner.async_parse_time_queue.parse_time_entries[0]
                .original
                .node_id,
            original.node_id
        );
        assert!(
            scheduler.runner.async_parse_time_queue.parse_time_entries[0].claimed_at_handoff,
            "handoff should activate the existing discovery-owned entry rather than duplicating it"
        );
    }

    #[tokio::test]
    async fn handoff_activation_keeps_discovery_owned_original_metadata() {
        let original = prepared_script(
            1,
            ScriptMode::Async,
            ScriptKind::Classic,
            ScriptSourceKind::External,
        );
        let mut scheduler =
            scheduler_with_async_state(vec![discovered_async_entry(original.clone())], Vec::new());

        scheduler
            .runner
            .async_parse_time_queue
            .apply_completion(async_load_completion_ok(
                original.node_id,
                "ready-before-handoff",
            ));

        let mut recovery_script = original.clone();
        recovery_script.url = Url::parse("https://example.com/recovery.js").unwrap();

        let disposition = scheduler.claim_parser_post_parse_script(recovery_script);
        assert_eq!(
            disposition,
            PostParseScriptClaimDisposition::ParseTimeAsyncClaimedAtHandoff
        );

        let turn = scheduler.parse_time_turn(ParseTimeTurnTrigger::BeforeParserStep {
            default_chunk_bytes: 4096,
        });
        match turn.ready_task {
            Some(ParseTimeDocumentScriptTask::ClassicAsyncScript(script)) => {
                assert_eq!(script.url, original.url);
                assert!(matches!(
                    &script.source,
                    ScriptSource::Loaded(source) if source == "ready-before-handoff"
                ));
            }
            other => panic!("expected ready classic async task, got {other:?}"),
        }
        assert_eq!(turn.parser_step_bytes, Some(4096));
    }
}
