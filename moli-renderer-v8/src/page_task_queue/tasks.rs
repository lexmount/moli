use crate::host::ScriptEventKind;
use crate::{
    content_security_policy::ContentSecurityPolicyUrlViolation,
    document_runtime::{DomHandle, ReadyConnectedStyleLoad},
    frame_owner_model::FrameDocumentTaskOwner,
    host::ScriptEventTask,
    planning::PreparedScript,
    stylesheet_blocking::DocumentOwnedBlockingStylesheetDiscoveryInput,
    types::ScriptErrorConstructorKind,
    types::ScriptRun,
};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MainDocumentMetaRefreshNavigationTask {
    owner: FrameDocumentTaskOwner,
    delay_ms: u32,
    url: Url,
}

impl MainDocumentMetaRefreshNavigationTask {
    pub(crate) fn new(owner: FrameDocumentTaskOwner, delay_ms: u32, url: Url) -> Self {
        Self {
            owner,
            delay_ms,
            url,
        }
    }

    pub(crate) const fn owner(&self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) const fn delay_ms(&self) -> u32 {
        self.delay_ms
    }

    pub(crate) fn into_url(self) -> Url {
        self.url
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PageOwnedInternalLoadingTask {
    MetaRefreshNavigation(MainDocumentMetaRefreshNavigationTask),
}

impl PageOwnedInternalLoadingTask {
    pub(crate) const fn document_owner(&self) -> FrameDocumentTaskOwner {
        match self {
            Self::MetaRefreshNavigation(task) => task.owner(),
        }
    }
}

/// State transition produced by executing one current-Document internal-loading body.
///
/// Both variants mean that a concrete Moli Page task was selected and ran. The
/// distinction controls navigation output capture, not task-end checkpoint
/// policy: even a refresh superseded by a competing navigation remains a
/// completed no-op task. Stale-Document authorization is deliberately kept in
/// the Page-level target effect instead of being flattened into this enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageOwnedInternalLoadingTaskEffect {
    MetaRefreshNavigationActivated,
    MetaRefreshNavigationNotActivated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WindowScriptFailureReportTask {
    pub(crate) message: String,
    pub(crate) filename: Option<String>,
    pub(crate) error_constructor: Option<ScriptErrorConstructorKind>,
}

impl WindowScriptFailureReportTask {
    pub(crate) fn new(message: impl Into<String>, filename: Option<String>) -> Self {
        Self::new_with_error_constructor(message, filename, None)
    }

    pub(crate) fn new_with_error_constructor(
        message: impl Into<String>,
        filename: Option<String>,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) -> Self {
        Self {
            message: message.into(),
            filename,
            error_constructor,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContentSecurityPolicyViolationEventTask {
    owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    target: Option<DomHandle>,
    violation: ContentSecurityPolicyUrlViolation,
}

impl ContentSecurityPolicyViolationEventTask {
    pub(crate) fn new(
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        violation: ContentSecurityPolicyUrlViolation,
    ) -> Self {
        Self {
            owner,
            target: None,
            violation,
        }
    }

    pub(crate) fn for_element(
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
        target: DomHandle,
        violation: ContentSecurityPolicyUrlViolation,
    ) -> Self {
        Self {
            owner,
            target: Some(target),
            violation,
        }
    }

    pub(crate) fn owner(&self) -> crate::frame_owner_model::FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) fn violation(&self) -> &ContentSecurityPolicyUrlViolation {
        &self.violation
    }

    pub(crate) fn target(&self) -> Option<DomHandle> {
        self.target
    }
}

/// Page/document-level JS tasks that must execute on the page owner chain.
///
/// The queue models browser-observable turn boundaries explicitly: background
/// work can notify readiness, document-level scheduling can enqueue page-owned
/// tasks, and JS still runs only when the page owner drains this queue.
// The frame-owner rollout is shrinking post-parse production use of raw
// `PageTask`. Some lifecycle variants remain as parse-time/test compatibility
// surface and can be unconstructed in focused test binaries.
#[derive(Debug)]
pub(crate) enum PageTask {
    SeedDocumentOwnedBlockingStylesheets(Vec<DocumentOwnedBlockingStylesheetDiscoveryInput>),
    RecordDocumentScriptRun { position: usize, run: ScriptRun },
    DispatchContentSecurityPolicyViolation(ContentSecurityPolicyViolationEventTask),
    DispatchScriptEvent(ScriptEventTask),
    ReportWindowScriptFailure(WindowScriptFailureReportTask),
    // Model DOMContentLoaded as its own page-owned lifecycle task instead of an
    // ad-hoc direct dispatch from runtime glue.
    //
    // Why keep it in the same queue as scripts?
    // - Browsers expose DOMContentLoaded as a distinct turn boundary: deferred
    //   work finishes, then DOMContentLoaded fires, then later work (for us,
    //   post-DCL async / load) may continue.
    // - Once defer-like scripts are already flowing through page-owned turns,
    //   calling DOMContentLoaded directly would collapse that boundary back
    //   into "script loop followed by immediate host callback".
    // - Keeping it in the queue means the owner lane sees one explicit task:
    //   pre-task checkpoint -> dispatch DOMContentLoaded -> post-task effects.
    //
    DispatchDomContentLoaded,
    DispatchConnectedStyleLoad(ReadyConnectedStyleLoad),
    // `load` is the next lifecycle boundary after post-DCL async fallback.
    // Like DOMContentLoaded above, keep it explicit so:
    // - lifecycle dispatch stays on the same owner lane as script turns
    // - `waitUntil=Load` can stop on a real queued task boundary
    // - microtask/checkpoint behavior remains attached to one owner-lane turn
    RecordDetachedPostParseRuns(Vec<ScriptRun>),
    CheckMainDocumentCompletion { owner: FrameDocumentTaskOwner },
    DispatchWindowLoad,
}

impl PageTask {
    pub(crate) fn phase_label(&self) -> &'static str {
        match self {
            Self::SeedDocumentOwnedBlockingStylesheets(_) => "stylesheet seed task",
            Self::RecordDocumentScriptRun { .. } => "document script run record task",
            Self::DispatchContentSecurityPolicyViolation(_) => {
                "security policy violation event task"
            }
            Self::DispatchScriptEvent(task) => match task.kind {
                ScriptEventKind::Load => "script load task",
                ScriptEventKind::Error => "script error task",
            },
            Self::ReportWindowScriptFailure(_) => "window script failure report task",
            Self::DispatchDomContentLoaded => "domcontentloaded task",
            Self::DispatchConnectedStyleLoad(_) => "connected style load task",
            Self::RecordDetachedPostParseRuns(_) => "detached post-parse runs task",
            Self::CheckMainDocumentCompletion { .. } => "main document completion recheck task",
            Self::DispatchWindowLoad => "window load task",
        }
    }

    pub(crate) fn as_script(&self) -> Option<&PreparedScript> {
        match self {
            Self::SeedDocumentOwnedBlockingStylesheets(_) => None,
            Self::RecordDocumentScriptRun { .. } => None,
            Self::DispatchContentSecurityPolicyViolation(_) => None,
            Self::DispatchScriptEvent(_) => None,
            Self::ReportWindowScriptFailure(_) => None,
            Self::DispatchDomContentLoaded
            | Self::DispatchConnectedStyleLoad(_)
            | Self::RecordDetachedPostParseRuns(_)
            | Self::CheckMainDocumentCompletion { .. }
            | Self::DispatchWindowLoad => None,
        }
    }

    pub(crate) fn is_window_load_task(&self) -> bool {
        matches!(self, Self::DispatchWindowLoad)
    }

    #[cfg(test)]
    pub(crate) fn allows_parse_time_scheduler_followup_turn(&self) -> bool {
        // Only parse-time classic async tasks are allowed to reopen the
        // "ask the document scheduler for the next parse-time async turn"
        // checkpoint.
        //
        // This is narrower than "any script task just finished":
        // - defer-like tasks belong to the post-parse lifecycle queue, not the
        //   parse-time async chain
        // - post-DCL async fallback tasks are already outside the parse-time
        //   interleave model
        // - lifecycle tasks (DOMContentLoaded/load) must not feed back into the
        //   parse-time scheduler at all
        //
        // Keeping this contract on `PageTask` makes the boundary explicit at the
        // task source itself instead of leaving runtime to infer it from a
        // specific task variant name at one call site.
        false
    }

    pub(crate) fn is_waiting_for_source_load(&self) -> bool {
        false
    }

    #[cfg(test)]
    pub(crate) fn belongs_to_parse_time_async_lane(&self) -> bool {
        false
    }

    #[cfg(test)]
    pub(crate) fn is_script_event_task(&self) -> bool {
        matches!(self, Self::DispatchScriptEvent(_))
    }

    #[cfg(test)]
    pub(crate) fn script_event(task: ScriptEventTask) -> Self {
        Self::DispatchScriptEvent(task)
    }

    #[cfg(test)]
    pub(crate) fn window_script_failure_report(task: WindowScriptFailureReportTask) -> Self {
        Self::ReportWindowScriptFailure(task)
    }
}
