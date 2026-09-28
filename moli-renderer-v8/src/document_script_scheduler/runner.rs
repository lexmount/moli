use crate::{
    document_script_scheduler::{DocumentScriptReadyWork, FrameDocumentModuleGraphReadyTarget},
    document_task_lane::DocumentTaskQueue,
    page_task_queue::RendererOwnerWakeSender,
};

use super::{
    async_queues::{AsyncFallbackQueue, AsyncParseTimeQueue},
    completion_port::ParseTimeAsyncCompletionPort,
    module_ready::ParserModuleGraphTerminalWork,
    parse_visible_async::{ParseVisibleAsyncLaneState, ParseVisibleAsyncReevaluationCredit},
    post_parse_task::PostParseDocumentScriptTask,
};

/// Pure document-level script runner state.
///
/// `DocumentScriptScheduler` remains the owner-facing shell: adapter impls
/// bind completion ports and concrete owner outputs around this state core.
/// This type is the state core that can later grow into the shared
/// `DocumentScriptRunner` used by both main and child document owners.
pub(super) struct DocumentScriptRunner<
    Target = FrameDocumentModuleGraphReadyTarget,
    ParserModuleEvaluation = std::convert::Infallible,
    ParserModuleGraphFailure = std::convert::Infallible,
    ParserClassicReady = std::convert::Infallible,
    ParserClassicSourceFailure = std::convert::Infallible,
> {
    pub(super) async_parse_time_queue: AsyncParseTimeQueue,
    pub(super) async_fallback_queue: AsyncFallbackQueue,
    pub(super) ready_work: DocumentTaskQueue<
        DocumentScriptReadyWork<
            Target,
            ParserModuleEvaluation,
            ParserModuleGraphFailure,
            ParserClassicReady,
            ParserClassicSourceFailure,
        >,
    >,
    pub(super) parse_visible_async_lane_state: ParseVisibleAsyncLaneState,
    pub(super) parse_visible_async_reevaluation_credit: ParseVisibleAsyncReevaluationCredit,
    pub(super) owner_wake: Option<RendererOwnerWakeSender>,
}

pub(super) struct DocumentScriptRunnerPostParsePlan {
    async_tasks: Vec<PostParseDocumentScriptTask>,
}

impl DocumentScriptRunnerPostParsePlan {
    pub(super) fn into_async_tasks(self) -> Vec<PostParseDocumentScriptTask> {
        self.async_tasks
    }
}

impl<
    Target,
    ParserModuleEvaluation,
    ParserModuleGraphFailure,
    ParserClassicReady,
    ParserClassicSourceFailure,
>
    DocumentScriptRunner<
        Target,
        ParserModuleEvaluation,
        ParserModuleGraphFailure,
        ParserClassicReady,
        ParserClassicSourceFailure,
    >
{
    pub(super) fn new() -> Self {
        Self {
            async_parse_time_queue: AsyncParseTimeQueue::new(),
            async_fallback_queue: AsyncFallbackQueue::new(),
            ready_work: DocumentTaskQueue::default(),
            parse_visible_async_lane_state: ParseVisibleAsyncLaneState::Open,
            parse_visible_async_reevaluation_credit: ParseVisibleAsyncReevaluationCredit::None,
            owner_wake: None,
        }
    }

    pub(super) fn bind_parse_time_async_completion_port(
        &mut self,
        port: ParseTimeAsyncCompletionPort,
    ) {
        self.async_parse_time_queue
            .bind_parse_time_async_completion_port(port);
    }

    pub(super) fn bind_owner_wake(&mut self, owner_wake: Option<RendererOwnerWakeSender>) {
        self.owner_wake = owner_wake;
    }
    pub(super) fn notify_module_script_evaluation_completed(
        &mut self,
        evaluation: ParserModuleEvaluation,
    ) {
        self.ready_work
            .push_back(DocumentScriptReadyWork::module_script_evaluation_completed(
                evaluation,
            ));
    }

    pub(super) fn notify_parser_classic_ready_work(&mut self, ready: ParserClassicReady) {
        self.ready_work
            .push_back(DocumentScriptReadyWork::parser_classic_ready(ready));
    }

    pub(super) fn notify_parser_classic_source_failure_work(
        &mut self,
        failure: ParserClassicSourceFailure,
    ) {
        self.ready_work
            .push_back(DocumentScriptReadyWork::parser_classic_source_failed(
                failure,
            ));
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
        self.ready_work.pop_front()
    }

    pub(super) fn pending_module_graph_ready_count(&self) -> usize {
        self.ready_work
            .iter()
            .filter(|work| matches!(work, DocumentScriptReadyWork::ModuleScriptGraphReady(_)))
            .count()
    }

    pub(super) fn pending_parser_module_evaluation_count(&self) -> usize {
        self.ready_work
            .iter()
            .filter(|work| {
                matches!(
                    work,
                    DocumentScriptReadyWork::ModuleScriptEvaluationCompleted(_)
                )
            })
            .count()
    }

    pub(super) fn pending_parser_module_failure_count(&self) -> usize {
        self.ready_work
            .iter()
            .filter(|work| matches!(work, DocumentScriptReadyWork::ModuleScriptGraphFailed(_)))
            .count()
    }

    pub(super) fn pending_parser_classic_ready_count(&self) -> usize {
        self.ready_work
            .iter()
            .filter(|work| matches!(work, DocumentScriptReadyWork::ParserClassicReady(_)))
            .count()
    }

    pub(super) fn pending_parser_classic_source_failure_count(&self) -> usize {
        self.ready_work
            .iter()
            .filter(|work| matches!(work, DocumentScriptReadyWork::ParserClassicSourceFailed(_)))
            .count()
    }

    pub(super) fn pending_ready_work_count(&self) -> usize {
        self.ready_work.len()
    }

    #[cfg(test)]
    pub(super) fn has_load_blocking_document_script_work(&self) -> bool {
        !self.ready_work.is_empty()
    }

    pub(super) fn finalize_owned_script_work(mut self) -> DocumentScriptRunnerPostParsePlan {
        self.seal_parse_visible_async_cutoff();
        let Self {
            async_parse_time_queue,
            async_fallback_queue,
            ready_work: _,
            parse_visible_async_lane_state: _,
            parse_visible_async_reevaluation_credit: _,
            owner_wake: _,
            ..
        } = self;
        let mut async_tasks = async_fallback_queue.into_async_phase_tasks();
        async_tasks.extend(async_parse_time_queue.into_remaining_async_phase_tasks());
        // Keep the order of already-observed async completions across parser
        // handoff. Pending sources acquire a ready position when they finish.
        DocumentScriptRunnerPostParsePlan { async_tasks }
    }

    pub(super) fn enqueue_parser_module_terminal_work(
        &mut self,
        terminal: ParserModuleGraphTerminalWork<Target, ParserModuleGraphFailure>,
    ) {
        match terminal {
            ParserModuleGraphTerminalWork::Ready(work) => self
                .ready_work
                .push_back(DocumentScriptReadyWork::module_script_graph_ready(*work)),
            ParserModuleGraphTerminalWork::Failed(failure) => self
                .ready_work
                .push_back(DocumentScriptReadyWork::module_script_graph_failed(failure)),
        }
    }
}
