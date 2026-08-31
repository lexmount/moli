use std::time::Instant;

use anyhow::Result;

use super::{
    ConcurrentParseTimeRuntime, OwnerStepProgress, PageVm, PageVmInitStage,
    PendingPhaseOneResidence, PostParsePageOwnedWork,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParseTimePhaseTransitionReason {
    /// The parser reached its ordinary phase-one completion boundary.
    ParserCompleted,
    /// Script reentrancy installed another Document while phase one was
    /// running; phase-two reconciliation must use the replacement.
    DocumentReplaced,
}

/// Transient result of one owner-local phase-one pump.
///
/// This enum never becomes stable Page state. `finish_*` converts a pending
/// result into exactly one [`PendingPhaseOneResidence`] before returning across
/// the owner boundary.
pub(super) enum ParseTimeOwnerCompletion {
    /// The streaming parser consumed all currently buffered input.
    NeedMoreInput(ConcurrentParseTimeRuntime),

    /// A parser-blocking classic script is waiting for its source terminal.
    PendingParserBlockingSourceLoad(ConcurrentParseTimeRuntime),

    /// A concrete task is already owned by a stable Page source. The runtime
    /// must be restored before the common scheduler may claim that task.
    PendingPageTask(ConcurrentParseTimeRuntime),

    /// Phase one has no remaining parser work and can enter phase two.
    AdvancePhase {
        runtime: ConcurrentParseTimeRuntime,
        reason: ParseTimePhaseTransitionReason,
    },

    /// Script execution requested a top-level navigation instead of completing
    /// the current parse.
    TriggeredNavigation {
        page_vm: Box<PageVm>,
        stage: PageVmInitStage,
    },
}

/// Result returned from phase-one creation to the Page owner.
///
/// All suspended states use one stable residence variant. This avoids
/// repeating Networking/style/open-stream variants through every protocol
/// layer and ensures the Page owner installs the complete state before waking
/// any producer.
pub(in crate::runtime) enum ParseTimePageVmCreationOutcome {
    /// Phase one yielded and supplied the complete state that must be installed
    /// in the Page slot before it can resume.
    PendingPhaseOne(PendingPhaseOneResidence),

    /// Parsing triggered a navigation; the owner must reconcile that
    /// navigation rather than enter phase two for this Document.
    TriggeredNavigation {
        page_vm: PageVm,
        stage: PageVmInitStage,
    },

    /// Phase one completed and hands the PageVm plus already-published Page
    /// tasks to the post-parse lifecycle.
    ContinuePhaseTwo {
        page_vm: PageVm,
        page_tasks: Vec<PostParsePageOwnedWork>,
        stage: PageVmInitStage,
        started: Instant,
    },
}

/// Result of building the initial parser/runtime around an open response.
pub(super) enum ParseTimePageVmStreamingBootstrapOutcome {
    /// Bootstrap script execution replaced the ordinary parse with navigation.
    TriggeredNavigation {
        page_vm: Box<PageVm>,
        stage: PageVmInitStage,
    },

    /// Bootstrap produced a runtime that can consume buffered body input.
    Runtime(Box<ConcurrentParseTimeRuntime>),
}

/// Transient progress result while an open response drives phase one.
///
/// `NeedMoreInput` and `PendingPageTask` are intentionally transient. The
/// streaming coordinator decides whether the response is still open and then
/// constructs one of the source-neutral stable residence variants.
pub(super) enum ParseTimePageVmStreamingProgress {
    /// All currently buffered body input was consumed.
    NeedMoreInput(Box<ConcurrentParseTimeRuntime>),

    /// A stable Page source owns a concrete ready task that must run outside
    /// the streaming parser loop.
    PendingPageTask(Box<ConcurrentParseTimeRuntime>),

    /// Script execution triggered a navigation while consuming streamed input.
    TriggeredNavigation {
        page_vm: PageVm,
        stage: PageVmInitStage,
    },

    /// EOF was consumed and phase one completed normally.
    ContinuePhaseTwo {
        page_vm: PageVm,
        page_tasks: Vec<PostParsePageOwnedWork>,
        stage: PageVmInitStage,
        started: Instant,
    },
}

enum ParseTimeOwnerPumpAction {
    Continue(ConcurrentParseTimeRuntime),
    Complete(Result<ParseTimeOwnerCompletion>),
}

pub(super) struct ParseTimePhaseOnePump {
    pub(super) runtime: Option<ConcurrentParseTimeRuntime>,
}

impl ParseTimePhaseOnePump {
    pub(super) fn new(runtime: ConcurrentParseTimeRuntime) -> Self {
        Self {
            runtime: Some(runtime),
        }
    }

    pub(super) async fn run_to_completion(mut self) -> Result<ParseTimeOwnerCompletion> {
        loop {
            match self.run_current_owner().await {
                ParseTimeOwnerPumpAction::Continue(next_runtime) => {
                    self.runtime = Some(next_runtime);
                }
                ParseTimeOwnerPumpAction::Complete(completion) => return completion,
            }
        }
    }

    async fn run_current_owner(&mut self) -> ParseTimeOwnerPumpAction {
        let runtime = self
            .runtime
            .take()
            .expect("parse-time pump must hold a runtime while stepping an owner");
        let mut runtime = runtime;
        // Keep the large owner-step future off the native stack. In debug
        // builds its nested parser/module variants otherwise consume most of
        // V8's stack budget before synchronous inline-module compilation even
        // begins. Boxing changes storage only; it does not add a task turn or
        // alter parser ordering.
        let progress = Box::pin(runtime.drive_owner_step()).await;
        match progress {
            Ok(OwnerStepProgress::Continue) => ParseTimeOwnerPumpAction::Continue(runtime),
            Ok(OwnerStepProgress::BlockedOnParserScriptSourceLoad) => {
                ParseTimeOwnerPumpAction::Complete(Ok(
                    ParseTimeOwnerCompletion::PendingParserBlockingSourceLoad(runtime),
                ))
            }
            Ok(OwnerStepProgress::BlockedOnPageTask) => ParseTimeOwnerPumpAction::Complete(Ok(
                ParseTimeOwnerCompletion::PendingPageTask(runtime),
            )),
            Ok(OwnerStepProgress::NeedMoreInput) => ParseTimeOwnerPumpAction::Complete(Ok(
                ParseTimeOwnerCompletion::NeedMoreInput(runtime),
            )),
            Ok(OwnerStepProgress::AdvancePhase) => {
                ParseTimeOwnerPumpAction::Complete(Ok(ParseTimeOwnerCompletion::AdvancePhase {
                    runtime,
                    reason: ParseTimePhaseTransitionReason::ParserCompleted,
                }))
            }
            Ok(OwnerStepProgress::DocumentReplaced) => {
                ParseTimeOwnerPumpAction::Complete(Ok(ParseTimeOwnerCompletion::AdvancePhase {
                    runtime,
                    reason: ParseTimePhaseTransitionReason::DocumentReplaced,
                }))
            }
            Ok(OwnerStepProgress::TriggeredNavigation) => {
                let stage = runtime.stage;
                ParseTimeOwnerPumpAction::Complete(Ok(
                    ParseTimeOwnerCompletion::TriggeredNavigation {
                        page_vm: Box::new(runtime.into_navigation_triggered_page_vm()),
                        stage,
                    },
                ))
            }
            Err(error) => ParseTimeOwnerPumpAction::Complete(Err(error)),
        }
    }
}
