use super::*;

enum PostParseProgressWaitDisposition {
    ReadyNow,
    Await,
    Idle,
}

pub(super) enum PostParsePageTaskPopBlocker {
    ParserOwnedPreDomContentLoadedTask,
    MainParserDeferredScript,
    WindowLoadWaitingForPostDomContentLoadedRuntimeBacklog,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ParserOwnedPreDomContentLoadedPageTaskFrontGate {
    Empty,
    DrainBeforeCurrentFront,
    DrainBeforeLifecycleBoundary,
    WaitBehindCurrentFront,
}

impl DocumentRuntime {
    fn main_parser_deferred_scripts_precede_post_parse_front(
        &mut self,
        task_queue: &mut PageTaskQueue,
    ) -> bool {
        let Some(owner) = self.main_parser_deferred_scripts_owner() else {
            return false;
        };
        let marker_sort_key =
            PostParsePageOwnedWork::main_parser_deferred_scripts(owner, 0).phase_sort_key();
        task_queue
            .post_parse_front()
            .is_none_or(|front| marker_sort_key <= front.phase_sort_key())
    }

    pub(crate) fn main_parser_deferred_script_continuation_is_ready(
        &mut self,
        task_queue: &mut PageTaskQueue,
    ) -> bool {
        // A document.open()/document.write() live parser can reach the outer
        // post-parse driver while its own input is still paused at a blocking
        // stylesheet or script. Defer-like scripts belong after that parser's
        // EOF, even when the script element appeared before the blocking
        // resource. Keep the deferred owner armed so the stylesheet task can
        // resume the parser, but do not admit its script action yet.
        if self.has_unfinished_root_document_parser_stream()
            || self.has_pending_document_write_parser_blocking_work()
        {
            return false;
        }
        let Some(task_owner) = self.main_parser_deferred_scripts_owner() else {
            return false;
        };
        if !self.main_parser_deferred_scripts_precede_post_parse_front(task_queue) {
            return false;
        }
        let owner = crate::module_script_continuation::MainParserDocumentOwner::new(task_owner);
        if !self
            .parser_module_document_scripts()
            .next_after_parsing_script_is_ready(owner)
        {
            return false;
        }
        !self.parser_deferred_script_is_blocked_by_stylesheets(owner)
    }

    pub(crate) fn parser_deferred_script_is_blocked_by_stylesheets(
        &self,
        owner: crate::module_script_continuation::MainParserDocumentOwner,
    ) -> bool {
        self.parser_module_document_scripts()
            .next_after_parsing_blocking_signatures(owner)
            .is_some_and(|signatures| {
                self.has_pending_parser_script_blocking_stylesheet_signatures(signatures)
            })
    }

    pub(super) fn ready_main_parser_deferred_script_action(
        &mut self,
        task_queue: &mut PageTaskQueue,
    ) -> Option<DocumentProcessingAction> {
        self.main_parser_deferred_script_continuation_is_ready(task_queue)
            .then(|| {
                let owner = self
                    .main_parser_deferred_scripts_owner()
                    .expect("ready parser-deferred action requires an armed owner");
                DocumentProcessingAction::PostParsePageOwnedWork(Box::new(
                    PostParsePageOwnedWork::main_parser_deferred_scripts(owner, 0),
                ))
            })
    }

    fn parser_owned_pre_domcontentloaded_page_task_front_gate(
        &mut self,
        task_queue: &mut PageTaskQueue,
    ) -> ParserOwnedPreDomContentLoadedPageTaskFrontGate {
        if let Some(work) = task_queue.post_parse_front() {
            if work.is_domcontentloaded_task() || work.is_window_load_task() {
                return ParserOwnedPreDomContentLoadedPageTaskFrontGate::DrainBeforeLifecycleBoundary;
            }
            if work.is_document_script_run_record_task()
                || work.is_defer_like_document_script()
                || matches!(
                    work.as_lifecycle_work(),
                    Some(crate::page_task_queue::PostParseLifecycleWork::DispatchScriptEvent(_))
                        | Some(
                            crate::page_task_queue::PostParseLifecycleWork::ReportWindowScriptFailure(
                                _
                            )
                        )
                )
            {
                return ParserOwnedPreDomContentLoadedPageTaskFrontGate::WaitBehindCurrentFront;
            }
            return ParserOwnedPreDomContentLoadedPageTaskFrontGate::DrainBeforeCurrentFront;
        }
        ParserOwnedPreDomContentLoadedPageTaskFrontGate::Empty
    }

    fn parser_owned_pre_domcontentloaded_page_task_drain_is_permitted(
        &mut self,
        task_queue: &mut PageTaskQueue,
    ) -> bool {
        matches!(
            self.parser_owned_pre_domcontentloaded_page_task_front_gate(task_queue),
            ParserOwnedPreDomContentLoadedPageTaskFrontGate::Empty
                | ParserOwnedPreDomContentLoadedPageTaskFrontGate::DrainBeforeCurrentFront
                | ParserOwnedPreDomContentLoadedPageTaskFrontGate::DrainBeforeLifecycleBoundary
        )
    }

    fn post_parse_front_is_window_load(&mut self, task_queue: &mut PageTaskQueue) -> bool {
        task_queue
            .post_parse_front()
            .is_some_and(PostParsePageOwnedWork::is_window_load_task)
    }

    pub(super) fn post_parse_page_task_pop_blocker(
        &mut self,
        task_queue: &mut PageTaskQueue,
        has_parser_owned_pre_domcontentloaded_page_tasks: bool,
        has_post_domcontentloaded_runtime_backlog: bool,
    ) -> Option<PostParsePageTaskPopBlocker> {
        if has_parser_owned_pre_domcontentloaded_page_tasks
            && self.parser_owned_pre_domcontentloaded_page_task_drain_is_permitted(task_queue)
        {
            return Some(PostParsePageTaskPopBlocker::ParserOwnedPreDomContentLoadedTask);
        }
        if self.main_parser_deferred_scripts_precede_post_parse_front(task_queue)
            && !self.main_parser_deferred_script_continuation_is_ready(task_queue)
        {
            return Some(PostParsePageTaskPopBlocker::MainParserDeferredScript);
        }
        if has_post_domcontentloaded_runtime_backlog
            && self.post_parse_front_is_window_load(task_queue)
        {
            return Some(
                PostParsePageTaskPopBlocker::WindowLoadWaitingForPostDomContentLoadedRuntimeBacklog,
            );
        }
        None
    }

    pub(crate) fn send_parser_owned_pre_domcontentloaded_page_owned_work(
        &self,
        work: Vec<PostParsePageOwnedWork>,
    ) -> bool {
        self.send_parser_owned_pre_domcontentloaded_work(work)
    }

    fn send_parser_owned_pre_domcontentloaded_work<I>(&self, work: I) -> bool
    where
        I: IntoIterator<Item = PostParsePageOwnedWork>,
    {
        let work: Vec<_> = work.into_iter().collect();
        if work.is_empty() {
            return false;
        }
        let task_tx = self
            .script_lifecycle
            .parser_owned_pre_domcontentloaded_work()
            .sender();
        for item in work {
            let _ = task_tx.send(item);
        }
        true
    }

    pub(crate) fn enqueue_parser_owned_pre_domcontentloaded_page_owned_work(
        &self,
        work: PostParsePageOwnedWork,
    ) {
        let _ = self
            .script_lifecycle
            .parser_owned_pre_domcontentloaded_work()
            .sender()
            .send(work);
    }

    pub(crate) fn has_parser_owned_pre_domcontentloaded_page_tasks(&mut self) -> bool {
        self.script_lifecycle
            .parser_owned_pre_domcontentloaded_work_mut()
            .with_tasks_mut(|tasks| !tasks.is_empty())
    }

    pub(crate) fn pop_parser_owned_pre_domcontentloaded_action(
        &mut self,
    ) -> Option<DocumentProcessingAction> {
        self.script_lifecycle
            .parser_owned_pre_domcontentloaded_work_mut()
            .pop_front()
            .map(|work| DocumentProcessingAction::PostParsePageOwnedWork(Box::new(work)))
    }

    pub(crate) fn post_parse_owner_readiness(
        &mut self,
        task_queue: &mut PageTaskQueue,
        has_post_domcontentloaded_runtime_backlog: bool,
    ) -> PostParseOwnerReadiness {
        let has_parser_owned_pre_domcontentloaded_page_tasks =
            self.has_parser_owned_pre_domcontentloaded_page_tasks();
        let blocks_page_task_pop = self
            .post_parse_page_task_pop_blocker(
                task_queue,
                has_parser_owned_pre_domcontentloaded_page_tasks,
                has_post_domcontentloaded_runtime_backlog,
            )
            .is_some();
        let has_ready_connected_style_loads = self.has_pending_ready_connected_style_loads();
        PostParseOwnerReadiness {
            should_poll_document_processing: has_ready_connected_style_loads
                || !blocks_page_task_pop,
            blocks_page_task_pop,
            has_pending_progress_source: has_parser_owned_pre_domcontentloaded_page_tasks
                || self.main_parser_deferred_scripts_owner().is_some()
                || has_post_domcontentloaded_runtime_backlog,
        }
    }

    pub(crate) fn poll_next_ready_post_parse_owner_action(
        &mut self,
        task_queue: &mut PageTaskQueue,
        has_post_domcontentloaded_runtime_backlog: bool,
    ) -> Option<DocumentProcessingAction> {
        if let Some(action) = self.poll_ready_parser_owned_pre_domcontentloaded_action(task_queue) {
            return Some(action);
        }
        let readiness =
            self.post_parse_owner_readiness(task_queue, has_post_domcontentloaded_runtime_backlog);
        if readiness.should_poll_document_processing {
            return self.poll_document_processing_action(task_queue, Option::<&NativeDom>::None);
        }
        None
    }

    fn poll_ready_parser_owned_pre_domcontentloaded_action(
        &mut self,
        task_queue: &mut PageTaskQueue,
    ) -> Option<DocumentProcessingAction> {
        self.parser_owned_pre_domcontentloaded_page_task_drain_is_permitted(task_queue)
            .then(|| self.pop_parser_owned_pre_domcontentloaded_action())
            .flatten()
    }

    pub(crate) fn poll_next_post_parse_owner_driver_step(
        &mut self,
        task_queue: &mut PageTaskQueue,
        has_post_domcontentloaded_runtime_backlog: bool,
    ) -> PostParseOwnerDriverStep {
        if let Some(action) = self.poll_next_ready_post_parse_owner_action(
            task_queue,
            has_post_domcontentloaded_runtime_backlog,
        ) {
            return PostParseOwnerDriverStep::Ready(Box::new(action));
        }
        match self.post_parse_progress_wait_disposition(
            task_queue,
            has_post_domcontentloaded_runtime_backlog,
        ) {
            PostParseProgressWaitDisposition::ReadyNow => {
                PostParseOwnerDriverStep::NeedsContinuation
            }
            PostParseProgressWaitDisposition::Await => PostParseOwnerDriverStep::AwaitProgress,
            PostParseProgressWaitDisposition::Idle => PostParseOwnerDriverStep::Idle,
        }
    }

    fn post_parse_progress_wait_disposition(
        &mut self,
        task_queue: &mut PageTaskQueue,
        has_post_domcontentloaded_runtime_backlog: bool,
    ) -> PostParseProgressWaitDisposition {
        self.drain_document_processing_wakes();
        let readiness =
            self.post_parse_owner_readiness(task_queue, has_post_domcontentloaded_runtime_backlog);
        let ready_document_processing_wake =
            !readiness.blocks_page_task_pop && self.has_ready_document_processing_wake(task_queue);
        if moli_trace::defer_wait_probe_enabled() {
            tracing::info!(
                target: "moli_defer_wait_probe",
                ready_document_processing_wake,
                should_poll_document_processing = readiness.should_poll_document_processing,
                blocks_page_task_pop = readiness.blocks_page_task_pop,
                has_pending_progress_source = readiness.has_pending_progress_source,
                front = task_queue.front().map(PageTask::phase_label).unwrap_or("empty"),
                front_waiting_for_source = task_queue
                    .front()
                    .is_some_and(PageTask::is_waiting_for_source_load),
                stage = "post_parse_wait_disposition",
            );
        }
        if ready_document_processing_wake {
            PostParseProgressWaitDisposition::ReadyNow
        } else if readiness.has_pending_progress_source
            || self.has_pending_document_processing(task_queue)
        {
            PostParseProgressWaitDisposition::Await
        } else {
            PostParseProgressWaitDisposition::Idle
        }
    }
}
