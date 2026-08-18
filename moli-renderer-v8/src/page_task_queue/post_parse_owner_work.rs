use std::collections::HashSet;

use crate::{
    document_runtime::ReadyConnectedStyleLoad,
    document_script_scheduler::PageOwnedDocumentScriptWork,
    frame_owner_model::{
        FrameDocumentTaskOwner, MainDocumentInteractiveLifecycleAction,
        MainDocumentScriptLoadDelayLease,
    },
    host::{ScriptEventKind, ScriptEventTask},
    planning::PreparedScript,
    stylesheet_blocking::DocumentBlockingStylesheetSignature,
    stylesheet_blocking::DocumentOwnedBlockingStylesheetDiscoveryInput,
    types::ScriptRun,
};

use super::{ContentSecurityPolicyViolationEventTask, PageTask, WindowScriptFailureReportTask};

#[derive(Debug)]
pub(crate) enum PostParsePageOwnedWork {
    Lifecycle(Box<PostParseLifecycleWork>),
    DocumentScript(Box<PageOwnedDocumentScriptWork>),
    DocumentScriptWithStylesheetSnapshot {
        work: Box<PageOwnedDocumentScriptWork>,
        blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
    },
}

#[derive(Debug)]
pub(crate) enum PostParseLifecycleWork {
    SeedDocumentOwnedBlockingStylesheets(Vec<DocumentOwnedBlockingStylesheetDiscoveryInput>),
    AdvanceMainParserDeferredScripts {
        owner: FrameDocumentTaskOwner,
        initial_count: usize,
    },
    RecordDocumentScriptRun {
        position: usize,
        run: ScriptRun,
    },
    DispatchContentSecurityPolicyViolation(ContentSecurityPolicyViolationEventTask),
    DispatchScriptEvent(ScriptEventTask),
    ReportWindowScriptFailure(WindowScriptFailureReportTask),
    SettleMainDocumentScriptLoadDelay(MainDocumentScriptLoadDelayLease),
    ApplyMainDocumentInteractive(MainDocumentInteractiveLifecycleAction),
    DispatchDomContentLoaded {
        owner: FrameDocumentTaskOwner,
    },
    CheckMainDocumentCompletion {
        owner: FrameDocumentTaskOwner,
    },
    DispatchConnectedStyleLoad(ReadyConnectedStyleLoad),
    RecordDetachedPostParseRuns(Vec<ScriptRun>),
    DispatchWindowLoad {
        owner: FrameDocumentTaskOwner,
    },
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PostParseLifecycleQueueStats {
    pub(crate) defer_count: usize,
    pub(crate) async_count: usize,
    pub(crate) detached_count: usize,
}

impl PostParseLifecycleWork {
    pub(crate) fn matches_main_document_runtime_target(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        if self
            .main_parser_deferred_scripts_owner()
            .or_else(|| self.main_document_lifecycle_owner())
            .is_some_and(|work_owner| work_owner != owner)
        {
            return false;
        }
        !matches!(
            self,
            Self::DispatchContentSecurityPolicyViolation(task) if task.owner() != owner
        )
    }

    #[cfg(test)]
    fn test_main_document_task_owner() -> FrameDocumentTaskOwner {
        use crate::frame_owner_model::{DocumentId, FrameSchedulerLaneId, LocalWindowId};

        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(0), LocalWindowId(0), DocumentId(0))
    }

    #[cfg(test)]
    pub(crate) fn test_domcontentloaded() -> Self {
        Self::DispatchDomContentLoaded {
            owner: Self::test_main_document_task_owner(),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_window_load() -> Self {
        Self::DispatchWindowLoad {
            owner: Self::test_main_document_task_owner(),
        }
    }

    pub(crate) fn from_parse_time_page_task(task: PageTask) -> Self {
        match task {
            PageTask::SeedDocumentOwnedBlockingStylesheets(inputs) => {
                Self::SeedDocumentOwnedBlockingStylesheets(inputs)
            }
            PageTask::RecordDocumentScriptRun { position, run } => {
                Self::RecordDocumentScriptRun { position, run }
            }
            PageTask::DispatchContentSecurityPolicyViolation(task) => {
                Self::DispatchContentSecurityPolicyViolation(task)
            }
            PageTask::DispatchScriptEvent(task) => Self::DispatchScriptEvent(task),
            PageTask::ReportWindowScriptFailure(task) => Self::ReportWindowScriptFailure(task),
            PageTask::DispatchDomContentLoaded => {
                #[cfg(test)]
                {
                    Self::test_domcontentloaded()
                }
                #[cfg(not(test))]
                unreachable!("production main DOMContentLoaded work must carry its document owner")
            }
            PageTask::CheckMainDocumentCompletion { owner } => {
                Self::CheckMainDocumentCompletion { owner }
            }
            PageTask::DispatchConnectedStyleLoad(ready) => {
                #[cfg(test)]
                {
                    Self::DispatchConnectedStyleLoad(ready)
                }
                #[cfg(not(test))]
                unreachable!(
                    "production connected style work must carry document owner binding: {ready:?}"
                )
            }
            PageTask::RecordDetachedPostParseRuns(runs) => Self::RecordDetachedPostParseRuns(runs),
            PageTask::DispatchWindowLoad => {
                #[cfg(test)]
                {
                    Self::test_window_load()
                }
                #[cfg(not(test))]
                unreachable!("production main load work must carry its document owner")
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn from_page_task(task: PageTask) -> Option<Self> {
        Some(Self::from_parse_time_page_task(task))
    }

    pub(crate) fn into_page_task(self) -> PageTask {
        match self {
            Self::SeedDocumentOwnedBlockingStylesheets(inputs) => {
                PageTask::SeedDocumentOwnedBlockingStylesheets(inputs)
            }
            Self::AdvanceMainParserDeferredScripts { .. } => {
                panic!("main parser-deferred work has no generic PageTask projection")
            }
            Self::RecordDocumentScriptRun { position, run } => {
                PageTask::RecordDocumentScriptRun { position, run }
            }
            Self::DispatchContentSecurityPolicyViolation(task) => {
                PageTask::DispatchContentSecurityPolicyViolation(task)
            }
            Self::DispatchScriptEvent(task) => PageTask::DispatchScriptEvent(task),
            Self::ReportWindowScriptFailure(task) => PageTask::ReportWindowScriptFailure(task),
            Self::SettleMainDocumentScriptLoadDelay(_) => {
                panic!(
                    "main document script load-delay settlement has no legacy PageTask projection"
                )
            }
            Self::ApplyMainDocumentInteractive(_) => {
                panic!("main interactive owner action has no legacy PageTask projection")
            }
            Self::DispatchDomContentLoaded { .. } => PageTask::DispatchDomContentLoaded,
            Self::CheckMainDocumentCompletion { owner } => {
                PageTask::CheckMainDocumentCompletion { owner }
            }
            Self::DispatchConnectedStyleLoad(ready) => PageTask::DispatchConnectedStyleLoad(ready),
            Self::RecordDetachedPostParseRuns(runs) => PageTask::RecordDetachedPostParseRuns(runs),
            Self::DispatchWindowLoad { .. } => PageTask::DispatchWindowLoad,
        }
    }

    pub(crate) fn phase_label(&self) -> &'static str {
        match self {
            Self::SeedDocumentOwnedBlockingStylesheets(_) => "stylesheet seed task",
            Self::AdvanceMainParserDeferredScripts { .. } => "main parser-deferred script task",
            Self::RecordDocumentScriptRun { .. } => "document script run record task",
            Self::DispatchContentSecurityPolicyViolation(_) => {
                "security policy violation event task"
            }
            Self::DispatchScriptEvent(task) => match task.kind {
                ScriptEventKind::Load => "script load task",
                ScriptEventKind::Error => "script error task",
            },
            Self::ReportWindowScriptFailure(_) => "window script failure report task",
            Self::SettleMainDocumentScriptLoadDelay(_) => {
                "main document script load-delay settlement task"
            }
            Self::ApplyMainDocumentInteractive(_) => "main document interactive task",
            Self::DispatchDomContentLoaded { .. } => "domcontentloaded task",
            Self::CheckMainDocumentCompletion { .. } => "main document completion recheck task",
            Self::DispatchConnectedStyleLoad(_) => "connected style load task",
            Self::RecordDetachedPostParseRuns(_) => "detached post-parse runs task",
            Self::DispatchWindowLoad { .. } => "window load task",
        }
    }

    pub(crate) fn phase_sort_key(&self) -> (u8, usize) {
        match self {
            Self::SeedDocumentOwnedBlockingStylesheets(_)
            | Self::ApplyMainDocumentInteractive(_) => (0, self.position()),
            Self::AdvanceMainParserDeferredScripts { .. } => (1, self.position()),
            Self::RecordDocumentScriptRun { .. } => (1, self.position()),
            Self::DispatchContentSecurityPolicyViolation(_)
            | Self::DispatchScriptEvent(_)
            | Self::ReportWindowScriptFailure(_) => (3, self.position()),
            Self::SettleMainDocumentScriptLoadDelay(_) => (3, self.position()),
            Self::DispatchDomContentLoaded { .. } => (4, self.position()),
            Self::CheckMainDocumentCompletion { .. } => (7, self.position()),
            Self::DispatchConnectedStyleLoad(_) => (6, self.position()),
            Self::RecordDetachedPostParseRuns(_) => (6, self.position()),
            Self::DispatchWindowLoad { .. } => (7, self.position()),
        }
    }

    pub(crate) fn position(&self) -> usize {
        match self {
            Self::SeedDocumentOwnedBlockingStylesheets(_)
            | Self::ApplyMainDocumentInteractive(_) => usize::MIN,
            Self::AdvanceMainParserDeferredScripts { .. } => 1,
            Self::RecordDocumentScriptRun { position, .. } => *position,
            Self::DispatchContentSecurityPolicyViolation(_) => usize::MAX - 7,
            Self::DispatchScriptEvent(_) => usize::MAX - 6,
            Self::ReportWindowScriptFailure(_) => usize::MAX - 5,
            Self::SettleMainDocumentScriptLoadDelay(_) => usize::MAX - 4,
            Self::DispatchDomContentLoaded { .. } => usize::MAX - 3,
            Self::CheckMainDocumentCompletion { .. } => usize::MAX - 1,
            Self::DispatchConnectedStyleLoad(_) => usize::MAX - 2,
            Self::RecordDetachedPostParseRuns(_) => usize::MAX - 1,
            Self::DispatchWindowLoad { .. } => usize::MAX,
        }
    }

    pub(crate) fn is_domcontentloaded_task(&self) -> bool {
        matches!(self, Self::DispatchDomContentLoaded { .. })
    }

    #[cfg(test)]
    pub(crate) fn is_main_document_interactive_task(&self) -> bool {
        matches!(self, Self::ApplyMainDocumentInteractive(_))
    }

    pub(crate) fn is_window_load_task(&self) -> bool {
        matches!(self, Self::DispatchWindowLoad { .. })
    }

    pub(crate) fn is_document_script_run_record_task(&self) -> bool {
        matches!(self, Self::RecordDocumentScriptRun { .. })
    }

    pub(crate) fn main_parser_deferred_scripts_owner(&self) -> Option<FrameDocumentTaskOwner> {
        match self {
            Self::AdvanceMainParserDeferredScripts { owner, .. } => Some(*owner),
            _ => None,
        }
    }

    pub(crate) fn main_document_lifecycle_owner(&self) -> Option<FrameDocumentTaskOwner> {
        match self {
            Self::ApplyMainDocumentInteractive(action) => Some(action.owner()),
            Self::SettleMainDocumentScriptLoadDelay(binding) => Some(binding.owner()),
            Self::DispatchConnectedStyleLoad(ready) => {
                ready.load_event_binding().map(|binding| binding.owner())
            }
            Self::DispatchDomContentLoaded { owner } | Self::DispatchWindowLoad { owner } => {
                Some(*owner)
            }
            Self::CheckMainDocumentCompletion { owner } => Some(*owner),
            _ => None,
        }
    }

    pub(crate) fn main_parser_deferred_script_count(&self) -> usize {
        match self {
            Self::AdvanceMainParserDeferredScripts { initial_count, .. } => *initial_count,
            _ => 0,
        }
    }

    pub(crate) fn starts_after_domcontentloaded_boundary(&self) -> bool {
        matches!(self, Self::RecordDetachedPostParseRuns(_))
    }

    pub(crate) fn detached_run_count(&self) -> usize {
        match self {
            Self::RecordDetachedPostParseRuns(runs) => runs.len(),
            _ => 0,
        }
    }

    pub(crate) fn requires_runtime_followup_publication(&self) -> bool {
        matches!(
            self,
            Self::DispatchConnectedStyleLoad(_)
                | Self::DispatchContentSecurityPolicyViolation(_)
                | Self::DispatchScriptEvent(_)
                | Self::ReportWindowScriptFailure(_)
                | Self::ApplyMainDocumentInteractive(_)
                | Self::DispatchDomContentLoaded { .. }
                | Self::CheckMainDocumentCompletion { .. }
                | Self::DispatchWindowLoad { .. }
        )
    }
}

impl PostParsePageOwnedWork {
    pub(crate) fn matches_main_document_runtime_target(
        &self,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        match self {
            Self::Lifecycle(work) => work.matches_main_document_runtime_target(owner),
            Self::DocumentScript(work)
            | Self::DocumentScriptWithStylesheetSnapshot { work, .. } => {
                work.matches_main_document_runtime_target(owner)
            }
        }
    }

    pub(crate) fn lifecycle_work(work: PostParseLifecycleWork) -> Self {
        Self::Lifecycle(Box::new(work))
    }

    pub(crate) fn main_parser_deferred_scripts(
        owner: FrameDocumentTaskOwner,
        initial_count: usize,
    ) -> Self {
        Self::lifecycle_work(PostParseLifecycleWork::AdvanceMainParserDeferredScripts {
            owner,
            initial_count,
        })
    }

    pub(crate) fn main_document_interactive(
        action: MainDocumentInteractiveLifecycleAction,
    ) -> Self {
        Self::lifecycle_work(PostParseLifecycleWork::ApplyMainDocumentInteractive(action))
    }

    pub(crate) fn main_document_domcontentloaded(owner: FrameDocumentTaskOwner) -> Self {
        Self::lifecycle_work(PostParseLifecycleWork::DispatchDomContentLoaded { owner })
    }

    pub(crate) fn main_document_window_load(owner: FrameDocumentTaskOwner) -> Self {
        Self::lifecycle_work(PostParseLifecycleWork::DispatchWindowLoad { owner })
    }

    pub(crate) fn main_document_lifecycle_owner(&self) -> Option<FrameDocumentTaskOwner> {
        self.as_lifecycle_work()
            .and_then(PostParseLifecycleWork::main_document_lifecycle_owner)
    }

    #[cfg(test)]
    pub(crate) fn is_main_document_interactive_task(&self) -> bool {
        self.as_lifecycle_work()
            .is_some_and(PostParseLifecycleWork::is_main_document_interactive_task)
    }

    pub(crate) fn document_script_work(work: PageOwnedDocumentScriptWork) -> Self {
        Self::DocumentScript(Box::new(work))
    }

    pub(crate) fn document_script_work_with_blocking_signatures(
        work: PageOwnedDocumentScriptWork,
        blocking_signatures_before: HashSet<DocumentBlockingStylesheetSignature>,
    ) -> Self {
        Self::DocumentScriptWithStylesheetSnapshot {
            work: Box::new(work),
            blocking_signatures_before,
        }
    }

    pub(crate) fn phase_sort_key(&self) -> (u8, usize) {
        match self {
            Self::Lifecycle(work) => work.phase_sort_key(),
            Self::DocumentScript(work)
            | Self::DocumentScriptWithStylesheetSnapshot { work, .. } => work.phase_sort_key(),
        }
    }

    pub(crate) fn as_script_mut(&mut self) -> Option<&mut PreparedScript> {
        match self {
            Self::Lifecycle(_) => None,
            Self::DocumentScript(work)
            | Self::DocumentScriptWithStylesheetSnapshot { work, .. } => Some(work.as_script_mut()),
        }
    }

    pub(crate) fn as_script(&self) -> Option<&PreparedScript> {
        match self {
            Self::Lifecycle(_) => None,
            Self::DocumentScript(work)
            | Self::DocumentScriptWithStylesheetSnapshot { work, .. } => Some(work.as_script()),
        }
    }

    pub(crate) fn post_parse_blocking_signatures_before(
        &self,
    ) -> Option<&HashSet<DocumentBlockingStylesheetSignature>> {
        match self {
            Self::Lifecycle(_) => None,
            Self::DocumentScript(_) => None,
            Self::DocumentScriptWithStylesheetSnapshot {
                blocking_signatures_before,
                ..
            } => Some(blocking_signatures_before),
        }
    }

    pub(crate) fn is_waiting_for_source_load(&self) -> bool {
        match self {
            Self::Lifecycle(_) => false,
            Self::DocumentScript(work)
            | Self::DocumentScriptWithStylesheetSnapshot { work, .. } => {
                work.is_waiting_for_source_load()
            }
        }
    }

    pub(crate) fn pending_source_load(&self) -> Option<crate::planning::SharedScriptSourceLoad> {
        match self {
            Self::Lifecycle(_) => None,
            Self::DocumentScript(work)
            | Self::DocumentScriptWithStylesheetSnapshot { work, .. } => work.pending_source_load(),
        }
    }

    pub(crate) fn complete_source_load_if_ready(&mut self) -> bool {
        match self {
            Self::Lifecycle(_) => false,
            Self::DocumentScript(work)
            | Self::DocumentScriptWithStylesheetSnapshot { work, .. } => {
                work.complete_source_load_if_ready()
            }
        }
    }

    pub(crate) fn claim_source_load_completion_wake(
        &mut self,
    ) -> Option<crate::planning::SharedScriptSourceLoad> {
        match self {
            Self::Lifecycle(_) => None,
            Self::DocumentScript(work)
            | Self::DocumentScriptWithStylesheetSnapshot { work, .. } => {
                work.claim_source_load_completion_wake()
            }
        }
    }

    pub(crate) fn is_domcontentloaded_task(&self) -> bool {
        matches!(self, Self::Lifecycle(work) if work.is_domcontentloaded_task())
    }

    pub(crate) fn is_window_load_task(&self) -> bool {
        matches!(self, Self::Lifecycle(work) if work.is_window_load_task())
    }

    pub(crate) fn is_document_script_run_record_task(&self) -> bool {
        matches!(self, Self::Lifecycle(work) if work.is_document_script_run_record_task())
    }

    pub(crate) fn starts_after_domcontentloaded_boundary(&self) -> bool {
        match self {
            Self::Lifecycle(work) => work.starts_after_domcontentloaded_boundary(),
            Self::DocumentScript(work)
            | Self::DocumentScriptWithStylesheetSnapshot { work, .. } => {
                work.starts_after_domcontentloaded_boundary()
            }
        }
    }

    pub(crate) fn is_defer_like_document_script(&self) -> bool {
        match self {
            Self::Lifecycle(work) => work.main_parser_deferred_scripts_owner().is_some(),
            Self::DocumentScript(work)
            | Self::DocumentScriptWithStylesheetSnapshot { work, .. } => work.is_defer_like(),
        }
    }

    pub(crate) fn main_parser_deferred_scripts_owner(&self) -> Option<FrameDocumentTaskOwner> {
        self.as_lifecycle_work()?
            .main_parser_deferred_scripts_owner()
    }

    pub(crate) fn is_async_phase_document_script(&self) -> bool {
        match self {
            Self::Lifecycle(_) => false,
            Self::DocumentScript(work)
            | Self::DocumentScriptWithStylesheetSnapshot { work, .. } => work.is_async_phase(),
        }
    }

    pub(crate) fn detached_run_count(&self) -> usize {
        match self {
            Self::Lifecycle(work) => work.detached_run_count(),
            Self::DocumentScript(_) | Self::DocumentScriptWithStylesheetSnapshot { .. } => 0,
        }
    }

    pub(crate) fn requires_runtime_followup_publication(&self) -> bool {
        matches!(self, Self::Lifecycle(work) if work.requires_runtime_followup_publication())
    }

    pub(crate) fn as_lifecycle_work(&self) -> Option<&PostParseLifecycleWork> {
        match self {
            Self::Lifecycle(work) => Some(work.as_ref()),
            Self::DocumentScript(_) | Self::DocumentScriptWithStylesheetSnapshot { .. } => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn into_page_task(self) -> Option<PageTask> {
        match self {
            Self::Lifecycle(work)
                if work.is_main_document_interactive_task()
                    || matches!(
                        work.as_ref(),
                        PostParseLifecycleWork::SettleMainDocumentScriptLoadDelay(_)
                    ) =>
            {
                None
            }
            Self::Lifecycle(work) => Some(work.into_page_task()),
            Self::DocumentScript(_) | Self::DocumentScriptWithStylesheetSnapshot { .. } => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn as_page_task(&self) -> Option<PageTask> {
        match self {
            Self::Lifecycle(work) => Some(match work.as_ref() {
                PostParseLifecycleWork::SeedDocumentOwnedBlockingStylesheets(_) => return None,
                PostParseLifecycleWork::AdvanceMainParserDeferredScripts { .. } => return None,
                PostParseLifecycleWork::RecordDocumentScriptRun { position, run } => {
                    PageTask::RecordDocumentScriptRun {
                        position: *position,
                        run: run.clone(),
                    }
                }
                PostParseLifecycleWork::DispatchContentSecurityPolicyViolation(task) => {
                    PageTask::DispatchContentSecurityPolicyViolation(task.clone())
                }
                PostParseLifecycleWork::DispatchScriptEvent(task) => {
                    PageTask::DispatchScriptEvent(task.clone())
                }
                PostParseLifecycleWork::ReportWindowScriptFailure(task) => {
                    PageTask::ReportWindowScriptFailure(task.clone())
                }
                PostParseLifecycleWork::SettleMainDocumentScriptLoadDelay(_) => return None,
                PostParseLifecycleWork::ApplyMainDocumentInteractive(_) => return None,
                PostParseLifecycleWork::DispatchDomContentLoaded { .. } => {
                    PageTask::DispatchDomContentLoaded
                }
                PostParseLifecycleWork::CheckMainDocumentCompletion { owner } => {
                    PageTask::CheckMainDocumentCompletion { owner: *owner }
                }
                PostParseLifecycleWork::DispatchConnectedStyleLoad(ready) => {
                    PageTask::DispatchConnectedStyleLoad(ready.clone())
                }
                PostParseLifecycleWork::RecordDetachedPostParseRuns(runs) => {
                    PageTask::RecordDetachedPostParseRuns(runs.clone())
                }
                PostParseLifecycleWork::DispatchWindowLoad { .. } => PageTask::DispatchWindowLoad,
            }),
            Self::DocumentScript(_) | Self::DocumentScriptWithStylesheetSnapshot { .. } => None,
        }
    }
}

pub(crate) fn post_parse_lifecycle_queue_stats(
    work: &[PostParsePageOwnedWork],
) -> PostParseLifecycleQueueStats {
    let mut stats = PostParseLifecycleQueueStats::default();
    for item in work {
        if item.is_defer_like_document_script() {
            stats.defer_count += item
                .as_lifecycle_work()
                .map(PostParseLifecycleWork::main_parser_deferred_script_count)
                .filter(|count| *count != 0)
                .unwrap_or(1);
        }
        if item.is_async_phase_document_script() {
            stats.async_count += 1;
        }
        stats.detached_count += item.detached_run_count();
    }
    stats
}
