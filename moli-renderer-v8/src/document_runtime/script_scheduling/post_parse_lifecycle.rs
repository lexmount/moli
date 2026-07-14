use super::*;
use crate::StylesheetBlockingReadView;
use crate::page_task_queue::{
    PageTask, PageTaskQueue, PostParseLifecycleWork, PostParsePageOwnedWork,
};
use crate::planning::PreparedScript;
use crate::types::ScriptExecutionReport;

impl DocumentRuntime {
    fn inject_post_parse_lifecycle_boundary_tasks(
        &self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        work: &mut Vec<PostParsePageOwnedWork>,
    ) {
        debug_assert!(
            !work.iter().any(|item| matches!(
                item.as_lifecycle_work(),
                Some(
                    PostParseLifecycleWork::DispatchDomContentLoaded { .. }
                        | PostParseLifecycleWork::DispatchWindowLoad { .. }
                )
            )),
            "post-parse lifecycle boundary tasks should be injected by document runtime, not pre-seeded"
        );
        let domcontentloaded_index = work
            .iter()
            .position(PostParsePageOwnedWork::starts_after_domcontentloaded_boundary)
            .unwrap_or(work.len());
        // This is an owner-queue boundary marker, not yet the HTML event-loop
        // queue point for DOMContentLoaded. When it reaches the front after all
        // defer-like and parser-owned pre-DCL work has drained, the driver first
        // runs ready timer tasks recorded during classic defer execution.
        work.insert(
            domcontentloaded_index,
            PostParsePageOwnedWork::main_document_domcontentloaded(owner),
        );
        work.push(PostParsePageOwnedWork::main_document_window_load(owner));
    }

    pub(crate) fn prepare_post_parse_lifecycle_page_owned_work(
        &self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        mut work: Vec<PostParsePageOwnedWork>,
    ) -> Vec<PostParsePageOwnedWork> {
        self.inject_post_parse_lifecycle_boundary_tasks(owner, &mut work);
        work
    }

    fn defer_like_script_from_page_task(task: &PageTask) -> Option<&PreparedScript> {
        // This is intentionally narrower than `task.as_script()`: post-DCL async
        // and lifecycle tasks still live on the same owner queue, but only the
        // defer-like pre-DCL slice participates in the stylesheet gate below.
        let script = task.as_script()?;
        script.mode.is_defer_like().then_some(script)
    }

    pub(crate) fn enqueue_post_parse_lifecycle_page_owned_work(
        &mut self,
        task_queue: &mut PageTaskQueue,
        work: Vec<PostParsePageOwnedWork>,
        _report: &mut ScriptExecutionReport,
    ) {
        let mut queued_work = Vec::with_capacity(work.len());
        for item in work {
            if let Some(owner) = item.main_parser_deferred_scripts_owner() {
                let parser_owner =
                    crate::module_script_continuation::MainParserDocumentOwner::new(owner);
                if self
                    .parser_module_document_scripts()
                    .has_after_parsing_script(parser_owner)
                {
                    self.arm_main_parser_deferred_scripts(owner);
                } else {
                    tracing::debug!(
                        ?owner,
                        "dropping stale main parser-deferred adapter marker without owned parser work"
                    );
                }
            } else {
                queued_work.push(item);
            }
        }
        task_queue.extend_post_parse_work(queued_work);
    }

    fn defer_like_script_from_post_parse_work(
        work: &PostParsePageOwnedWork,
    ) -> Option<&PreparedScript> {
        work.is_defer_like_document_script()
            .then(|| work.as_script())
            .flatten()
    }

    pub(crate) fn page_task_is_blocked_by_stylesheets_in_document(
        &mut self,
        document: &(impl StylesheetBlockingReadView + ?Sized),
        task: &PageTask,
    ) -> bool {
        let Some(script) = Self::defer_like_script_from_page_task(task) else {
            return false;
        };
        // Chromium's shape here is indirect:
        // - script-blocking stylesheets gate parser-blocking / defer-like script readiness
        // - `DOMContentLoaded` follows after that work drains
        // - `DOMContentLoaded` itself is not separately stylesheet-gated
        //
        // Keep that boundary explicit on the task gate:
        // only defer-like script tasks are checked here; lifecycle tasks such as
        // `DispatchDomContentLoaded` stay unblocked and are delayed only because
        // earlier defer-like work remains pending in the same owner queue.
        self.is_document_script_blocked_by_stylesheets(document, script.node_id)
    }

    pub(crate) fn page_task_is_blocked_by_document_stylesheets(&mut self, task: &PageTask) -> bool {
        let Some(script) = Self::defer_like_script_from_page_task(task) else {
            return false;
        };
        let node_id = script.node_id;
        self.note_discovered_live_blocking_stylesheets();
        self.drain_blocking_stylesheet_completions();
        self.stylesheet_lifecycle
            .fetches
            .blocks_script(&self.dom_host, node_id)
    }

    pub(crate) fn post_parse_work_is_blocked_by_document_stylesheets(
        &mut self,
        work: &PostParsePageOwnedWork,
    ) -> bool {
        if let Some(signatures) = work.post_parse_blocking_signatures_before() {
            return self
                .has_pending_parser_script_blocking_stylesheet_signatures(signatures.iter());
        }
        let Some(script) = Self::defer_like_script_from_post_parse_work(work) else {
            return false;
        };
        let node_id = script.node_id;
        let fetcher = self.stylesheet_fetcher();
        self.stylesheet_lifecycle
            .fetches
            .discover_from_document(&fetcher, &self.dom_host);
        self.drain_blocking_stylesheet_completions();
        self.stylesheet_lifecycle
            .fetches
            .blocks_script(&self.dom_host, node_id)
    }
}
