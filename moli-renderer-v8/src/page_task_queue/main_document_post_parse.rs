//! Typed execution boundary for the remaining main-Document post-parse work.
//!
//! `PostParseLifecycleWork` is a broad transport enum shared with parser and
//! lifecycle machinery. Once one of the eight generic callback/state entries
//! is selected, this module narrows it to [`MainDocumentPostParseWork`] and
//! preserves its concrete identity through body execution. The resulting
//! [`MainDocumentPostParseExecution`] is short-lived: it is never queued and
//! can only be consumed by the PageVm task-completion coordinator.

use crate::{
    frame_owner_model::{
        FrameDocumentTaskOwner, MainDocumentLoadCompletionState, MainDocumentScriptLoadDelayKind,
        MainDocumentScriptLoadDelayLease, MainDocumentScriptLoadDelayRelease,
    },
    host::ScriptEventTask,
    stylesheet_blocking::DocumentOwnedBlockingStylesheetDiscoveryInput,
    types::ScriptRun,
};

use super::{
    ContentSecurityPolicyViolationEventTask, PostParseLifecycleWork, PostParsePageOwnedWork,
    WindowScriptFailureReportTask,
};

/// Exact main-Document binding selected by a post-parse carrier.
///
/// The PageVm root token remains the outer scheduler's authority. This value
/// is the ScriptVm-local half and separates `document.open()` replacements.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MainDocumentPostParseOwner {
    document_owner: FrameDocumentTaskOwner,
}

impl MainDocumentPostParseOwner {
    pub(crate) const fn new(document_owner: FrameDocumentTaskOwner) -> Self {
        Self { document_owner }
    }

    pub(crate) const fn document_owner(self) -> FrameDocumentTaskOwner {
        self.document_owner
    }
}

/// The eight post-parse callback/state bodies whose task-end checkpoint was
/// previously hidden behind the generic `BeforeTask` compatibility policy.
///
/// This is an execution-only narrowing of an already-selected queue payload;
/// it is not another task queue or scheduler classification.
#[derive(Debug)]
pub(crate) enum MainDocumentPostParseWork {
    SeedDocumentOwnedBlockingStylesheets(Vec<DocumentOwnedBlockingStylesheetDiscoveryInput>),
    RecordDocumentScriptRun(ScriptRun),
    DispatchContentSecurityPolicyViolation(ContentSecurityPolicyViolationEventTask),
    DispatchScriptEvent(ScriptEventTask),
    ReportWindowScriptFailure(WindowScriptFailureReportTask),
    SettleMainDocumentScriptLoadDelay(MainDocumentScriptLoadDelayLease),
    CheckMainDocumentCompletion { owner: FrameDocumentTaskOwner },
    RecordDetachedPostParseRuns(Vec<ScriptRun>),
}

impl MainDocumentPostParseWork {
    /// Narrow a broad page-owned payload without losing it when another
    /// already-migrated family owns the work.
    pub(crate) fn try_from_page_owned(
        work: PostParsePageOwnedWork,
    ) -> Result<Self, PostParsePageOwnedWork> {
        let PostParsePageOwnedWork::Lifecycle(work) = work else {
            return Err(work);
        };
        Self::try_from_lifecycle(*work)
            .map_err(|work| PostParsePageOwnedWork::lifecycle_work(*work))
    }

    /// Keep the rejected broad transport boxed: its callback variants are
    /// substantially larger than this execution-only narrowing.
    pub(crate) fn try_from_lifecycle(
        work: PostParseLifecycleWork,
    ) -> Result<Self, Box<PostParseLifecycleWork>> {
        match work {
            PostParseLifecycleWork::SeedDocumentOwnedBlockingStylesheets(inputs) => {
                Ok(Self::SeedDocumentOwnedBlockingStylesheets(inputs))
            }
            PostParseLifecycleWork::RecordDocumentScriptRun { run, .. } => {
                Ok(Self::RecordDocumentScriptRun(run))
            }
            PostParseLifecycleWork::DispatchContentSecurityPolicyViolation(task) => {
                Ok(Self::DispatchContentSecurityPolicyViolation(task))
            }
            PostParseLifecycleWork::DispatchScriptEvent(task) => {
                Ok(Self::DispatchScriptEvent(task))
            }
            PostParseLifecycleWork::ReportWindowScriptFailure(task) => {
                Ok(Self::ReportWindowScriptFailure(task))
            }
            PostParseLifecycleWork::SettleMainDocumentScriptLoadDelay(binding) => {
                Ok(Self::SettleMainDocumentScriptLoadDelay(binding))
            }
            PostParseLifecycleWork::CheckMainDocumentCompletion { owner } => {
                Ok(Self::CheckMainDocumentCompletion { owner })
            }
            PostParseLifecycleWork::RecordDetachedPostParseRuns(runs) => {
                Ok(Self::RecordDetachedPostParseRuns(runs))
            }
            other => Err(Box::new(other)),
        }
    }

    pub(crate) fn discarded_stale(
        self,
        current_owner: Option<MainDocumentPostParseOwner>,
    ) -> MainDocumentPostParseExecution {
        let target = MainDocumentPostParseTargetEffect::DiscardedStaleOwner { current_owner };
        match self {
            Self::SeedDocumentOwnedBlockingStylesheets(_) => {
                MainDocumentPostParseExecution::SeedDocumentOwnedBlockingStylesheets(
                    MainDocumentPostParseStateExecution::new(target, 0),
                )
            }
            Self::RecordDocumentScriptRun(run) => {
                MainDocumentPostParseExecution::RecordDocumentScriptRun(
                    MainDocumentPostParseStateExecution::new(target, run),
                )
            }
            Self::DispatchContentSecurityPolicyViolation(_) => {
                MainDocumentPostParseExecution::DispatchContentSecurityPolicyViolation(
                    MainDocumentPostParseCallbackExecution::not_entered(target),
                )
            }
            Self::DispatchScriptEvent(_) => MainDocumentPostParseExecution::DispatchScriptEvent(
                MainDocumentPostParseCallbackExecution::not_entered(target),
            ),
            Self::ReportWindowScriptFailure(_) => {
                MainDocumentPostParseExecution::ReportWindowScriptFailure(
                    MainDocumentPostParseCallbackExecution::not_entered(target),
                )
            }
            Self::SettleMainDocumentScriptLoadDelay(binding) => {
                MainDocumentPostParseExecution::SettleMainDocumentScriptLoadDelay(
                    MainDocumentPostParseStateExecution::new(
                        target,
                        MainDocumentScriptLoadDelayEffect::not_released(&binding),
                    ),
                )
            }
            Self::CheckMainDocumentCompletion { owner } => {
                MainDocumentPostParseExecution::CheckMainDocumentCompletion(
                    MainDocumentPostParseStateExecution::new(
                        target,
                        MainDocumentCompletionRecheckEffect::not_applied(owner),
                    ),
                )
            }
            Self::RecordDetachedPostParseRuns(runs) => {
                MainDocumentPostParseExecution::RecordDetachedPostParseRuns(
                    MainDocumentPostParseStateExecution::new(target, runs),
                )
            }
        }
    }
}

/// Whether the selected work applied to the exact current ScriptVm owner.
///
/// `AppliedToSelectedOwner` remains true if a callback synchronously replaces
/// the Document. That callback still ran and its old-realm microtasks still
/// belong to this task. `DiscardedStaleOwner` means the body was never entered
/// and therefore must not checkpoint the replacement realm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MainDocumentPostParseTargetEffect {
    AppliedToSelectedOwner {
        selected_owner: MainDocumentPostParseOwner,
        current_owner_after_body: Option<MainDocumentPostParseOwner>,
    },
    DiscardedStaleOwner {
        current_owner: Option<MainDocumentPostParseOwner>,
    },
}

impl MainDocumentPostParseTargetEffect {
    pub(crate) const fn applied(
        selected_owner: MainDocumentPostParseOwner,
        current_owner_after_body: Option<MainDocumentPostParseOwner>,
    ) -> Self {
        Self::AppliedToSelectedOwner {
            selected_owner,
            current_owner_after_body,
        }
    }

    pub(crate) const fn applied_to_selected_owner(self) -> bool {
        matches!(self, Self::AppliedToSelectedOwner { .. })
    }
}

/// Whether a callback-capable body actually entered its dispatch algorithm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MainDocumentPostParseCallbackEffect {
    NotEntered,
    DispatchAttempted,
}

/// Best-effort callback settlement, kept separate from callback entry.
///
/// A dispatch can fail after event construction or after some listener-visible
/// work. It still owes callback completion, so failure must not be flattened
/// into `NotEntered` or a checkpoint-only state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MainDocumentPostParseCallbackSettlement {
    NotRun,
    Completed,
    FailedBestEffort,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MainDocumentPostParseCallbackExecution {
    target: MainDocumentPostParseTargetEffect,
    callback: MainDocumentPostParseCallbackEffect,
    settlement: MainDocumentPostParseCallbackSettlement,
}

impl MainDocumentPostParseCallbackExecution {
    pub(crate) const fn not_entered(target: MainDocumentPostParseTargetEffect) -> Self {
        Self {
            target,
            callback: MainDocumentPostParseCallbackEffect::NotEntered,
            settlement: MainDocumentPostParseCallbackSettlement::NotRun,
        }
    }

    pub(crate) const fn dispatch_attempted(
        target: MainDocumentPostParseTargetEffect,
        settlement: MainDocumentPostParseCallbackSettlement,
    ) -> Self {
        Self {
            target,
            callback: MainDocumentPostParseCallbackEffect::DispatchAttempted,
            settlement,
        }
    }

    pub(crate) const fn target(self) -> MainDocumentPostParseTargetEffect {
        self.target
    }

    pub(crate) const fn callback(self) -> MainDocumentPostParseCallbackEffect {
        self.callback
    }

    pub(crate) const fn settlement(self) -> MainDocumentPostParseCallbackSettlement {
        self.settlement
    }
}

#[derive(Debug)]
pub(crate) struct MainDocumentPostParseStateExecution<T> {
    target: MainDocumentPostParseTargetEffect,
    value: T,
}

impl<T> MainDocumentPostParseStateExecution<T> {
    pub(crate) const fn new(target: MainDocumentPostParseTargetEffect, value: T) -> Self {
        Self { target, value }
    }

    pub(crate) const fn target(&self) -> MainDocumentPostParseTargetEffect {
        self.target
    }

    pub(crate) fn into_parts(self) -> (MainDocumentPostParseTargetEffect, T) {
        (self.target, self.value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MainDocumentScriptLoadDelayEffect {
    owner: FrameDocumentTaskOwner,
    kind: MainDocumentScriptLoadDelayKind,
    release: Option<MainDocumentScriptLoadDelayRelease>,
}

impl MainDocumentScriptLoadDelayEffect {
    pub(crate) fn not_released(binding: &MainDocumentScriptLoadDelayLease) -> Self {
        Self {
            owner: binding.owner(),
            kind: binding.kind(),
            release: None,
        }
    }

    pub(crate) const fn released(
        owner: FrameDocumentTaskOwner,
        kind: MainDocumentScriptLoadDelayKind,
        release: MainDocumentScriptLoadDelayRelease,
    ) -> Self {
        Self {
            owner,
            kind,
            release: Some(release),
        }
    }

    pub(crate) const fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) const fn kind(self) -> MainDocumentScriptLoadDelayKind {
        self.kind
    }

    pub(crate) const fn release(self) -> Option<MainDocumentScriptLoadDelayRelease> {
        self.release
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MainDocumentCompletionRecheckEffect {
    owner: FrameDocumentTaskOwner,
    load_completion: Option<MainDocumentLoadCompletionState>,
    readiness: Option<bool>,
}

impl MainDocumentCompletionRecheckEffect {
    pub(crate) const fn not_applied(owner: FrameDocumentTaskOwner) -> Self {
        Self {
            owner,
            load_completion: None,
            readiness: None,
        }
    }

    pub(crate) const fn applied(
        owner: FrameDocumentTaskOwner,
        load_completion: Option<MainDocumentLoadCompletionState>,
        readiness: Option<bool>,
    ) -> Self {
        Self {
            owner,
            load_completion,
            readiness,
        }
    }

    pub(crate) const fn owner(self) -> FrameDocumentTaskOwner {
        self.owner
    }

    pub(crate) const fn load_completion(self) -> Option<MainDocumentLoadCompletionState> {
        self.load_completion
    }

    pub(crate) const fn readiness(self) -> Option<bool> {
        self.readiness
    }
}

/// Task-end class proven from an already-executed concrete post-parse body.
///
/// This value cannot be stored on queued work. It only tells the shared PageVm
/// coordinator how to discharge the selected task that produced it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MainDocumentPostParseTaskEnd {
    NoCompletion,
    CheckpointOnly,
    CallbackCheckpoint,
}

/// Concrete execution fact retained until the shared PageVm coordinator has
/// submitted task completion and post-checkpoint reconciliation.
#[must_use = "post-parse execution must be consumed by the PageVm coordinator"]
#[derive(Debug)]
pub(crate) enum MainDocumentPostParseExecution {
    SeedDocumentOwnedBlockingStylesheets(MainDocumentPostParseStateExecution<usize>),
    RecordDocumentScriptRun(MainDocumentPostParseStateExecution<ScriptRun>),
    DispatchContentSecurityPolicyViolation(MainDocumentPostParseCallbackExecution),
    DispatchScriptEvent(MainDocumentPostParseCallbackExecution),
    ReportWindowScriptFailure(MainDocumentPostParseCallbackExecution),
    SettleMainDocumentScriptLoadDelay(
        MainDocumentPostParseStateExecution<MainDocumentScriptLoadDelayEffect>,
    ),
    CheckMainDocumentCompletion(
        MainDocumentPostParseStateExecution<MainDocumentCompletionRecheckEffect>,
    ),
    RecordDetachedPostParseRuns(MainDocumentPostParseStateExecution<Vec<ScriptRun>>),
}

impl MainDocumentPostParseExecution {
    pub(crate) const fn kind(&self) -> &'static str {
        match self {
            Self::SeedDocumentOwnedBlockingStylesheets(_) => "stylesheet-seed",
            Self::RecordDocumentScriptRun(_) => "document-script-run-record",
            Self::DispatchContentSecurityPolicyViolation(_) => "csp-violation",
            Self::DispatchScriptEvent(_) => "script-event",
            Self::ReportWindowScriptFailure(_) => "window-script-failure",
            Self::SettleMainDocumentScriptLoadDelay(_) => "script-load-delay-settlement",
            Self::CheckMainDocumentCompletion(_) => "main-document-completion-recheck",
            Self::RecordDetachedPostParseRuns(_) => "detached-run-record",
        }
    }

    pub(crate) const fn target(&self) -> MainDocumentPostParseTargetEffect {
        match self {
            Self::SeedDocumentOwnedBlockingStylesheets(execution) => execution.target(),
            Self::RecordDocumentScriptRun(execution) => execution.target(),
            Self::DispatchContentSecurityPolicyViolation(execution)
            | Self::DispatchScriptEvent(execution)
            | Self::ReportWindowScriptFailure(execution) => execution.target(),
            Self::SettleMainDocumentScriptLoadDelay(execution) => execution.target(),
            Self::CheckMainDocumentCompletion(execution) => execution.target(),
            Self::RecordDetachedPostParseRuns(execution) => execution.target(),
        }
    }

    pub(crate) const fn callback(&self) -> Option<MainDocumentPostParseCallbackEffect> {
        match self {
            Self::DispatchContentSecurityPolicyViolation(execution)
            | Self::DispatchScriptEvent(execution)
            | Self::ReportWindowScriptFailure(execution) => Some(execution.callback()),
            _ => None,
        }
    }

    pub(crate) const fn task_end(&self) -> MainDocumentPostParseTaskEnd {
        if !self.target().applied_to_selected_owner() {
            return MainDocumentPostParseTaskEnd::NoCompletion;
        }
        match self.callback() {
            Some(MainDocumentPostParseCallbackEffect::DispatchAttempted) => {
                MainDocumentPostParseTaskEnd::CallbackCheckpoint
            }
            Some(MainDocumentPostParseCallbackEffect::NotEntered) | None => {
                MainDocumentPostParseTaskEnd::CheckpointOnly
            }
        }
    }
}
