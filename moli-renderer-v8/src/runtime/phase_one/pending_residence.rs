use std::time::Instant;

use anyhow::Result;

use super::{
    ConcurrentParseTimeRuntime, PageVm, ParseTimePageVmCreationOutcome,
    PendingStreamingPhaseOneContinuation, PhaseOneRestoreRequirement,
};

pub(in crate::runtime) enum PendingPhaseOneResumeOutcome {
    Progress(ParseTimePageVmCreationOutcome),
    MainResourceLoadFailed {
        page_vm: PageVm,
        error: anyhow::Error,
    },
}

impl PendingPhaseOneResumeOutcome {
    pub(super) fn progress(outcome: ParseTimePageVmCreationOutcome) -> Self {
        Self::Progress(outcome)
    }

    pub(super) fn main_resource_load_failed(
        runtime: ConcurrentParseTimeRuntime,
        error: anyhow::Error,
    ) -> Self {
        Self::MainResourceLoadFailed {
            page_vm: runtime.into_main_resource_load_failed_page_vm(),
            error,
        }
    }
}

/// Stable Page-slot residence for a parser that has yielded phase one.
///
/// Closed-input runtimes and open streaming responses have different state
/// requirements, so they are deliberately represented by separate variants.
/// Callers can inspect the PageVm and wake interest, but only this type can
/// resume the stored continuation.
pub(in crate::runtime) enum PendingPhaseOneResidence {
    /// Closed main-document input with a parser-owned external classic-script
    /// source load still in flight.
    ParserBlockingSourceLoad {
        /// Complete parser/runtime state to resume after the source terminal.
        runtime: Box<ConcurrentParseTimeRuntime>,
        /// Original page-creation start time retained across suspension.
        started: Instant,
    },

    /// Closed main-document input whose parser progress depends on Page-owned
    /// work.
    ///
    /// The work may already be resident or its producer may still be in
    /// flight. This variant intentionally stores neither a source
    /// classification nor a readiness bit. After restoration, the production
    /// Page scheduler reads its complete descriptor snapshot; if no descriptor
    /// is ready, the next producer transition owns the wake.
    ClosedInputPageWork {
        runtime: Box<ConcurrentParseTimeRuntime>,
        started: Instant,
    },

    /// The main response is still open and this continuation owns the parser,
    /// decoder, body-input bridge, and start time as one residence.
    ///
    /// New input and ordinary Page tasks may both wake the owner. Ordinary task
    /// identity remains solely in the production Page sources.
    OpenStreaming(Box<PendingStreamingPhaseOneContinuation>),
}

impl PendingPhaseOneResidence {
    pub(in crate::runtime) fn parser_blocking_source_load(
        runtime: Box<ConcurrentParseTimeRuntime>,
        started: Instant,
    ) -> Self {
        runtime.publish_pending_page_creation_phase();
        Self::ParserBlockingSourceLoad { runtime, started }
    }

    pub(in crate::runtime) fn closed_input_page_work(
        runtime: Box<ConcurrentParseTimeRuntime>,
        started: Instant,
    ) -> Self {
        runtime.publish_pending_page_creation_phase();
        Self::ClosedInputPageWork { runtime, started }
    }

    pub(in crate::runtime) fn open_streaming(
        continuation: Box<PendingStreamingPhaseOneContinuation>,
    ) -> Self {
        continuation.publish_pending_page_creation_phase();
        Self::OpenStreaming(continuation)
    }

    pub(in crate::runtime) fn page_vm(&self) -> &PageVm {
        match self {
            Self::ParserBlockingSourceLoad { runtime, .. }
            | Self::ClosedInputPageWork { runtime, .. } => runtime.page_vm(),
            Self::OpenStreaming(continuation) => continuation.page_vm(),
        }
    }

    pub(in crate::runtime) fn page_vm_mut(&mut self) -> &mut PageVm {
        match self {
            Self::ParserBlockingSourceLoad { runtime, .. }
            | Self::ClosedInputPageWork { runtime, .. } => runtime.page_vm_mut(),
            Self::OpenStreaming(continuation) => continuation.page_vm_mut(),
        }
    }

    pub(in crate::runtime) fn owner_wake_token(&self) -> Option<crate::runtime::RendererPageToken> {
        match self {
            Self::ParserBlockingSourceLoad { runtime, .. }
            | Self::ClosedInputPageWork { runtime, .. } => runtime.owner_wake_token(),
            Self::OpenStreaming(continuation) => continuation.owner_wake_token(),
        }
    }

    pub(in crate::runtime) const fn restore_requirement(&self) -> PhaseOneRestoreRequirement {
        match self {
            Self::ParserBlockingSourceLoad { .. } => {
                PhaseOneRestoreRequirement::ParserBlockingSourceLoad
            }
            Self::ClosedInputPageWork { .. } => PhaseOneRestoreRequirement::PageWork,
            Self::OpenStreaming(_) => PhaseOneRestoreRequirement::Producer,
        }
    }

    pub(in crate::runtime) fn has_ready_streaming_input(&mut self) -> bool {
        match self {
            Self::OpenStreaming(continuation) => continuation.has_ready_input(),
            Self::ParserBlockingSourceLoad { .. } | Self::ClosedInputPageWork { .. } => false,
        }
    }

    pub(in crate::runtime) async fn resume(self) -> Result<PendingPhaseOneResumeOutcome> {
        match self {
            Self::ParserBlockingSourceLoad { runtime, started }
            | Self::ClosedInputPageWork { runtime, started } => {
                runtime.publish_processing_main_document_phase();
                let outcome = (*runtime)
                    .continue_creation_from_phase_one_runtime(started)
                    .await?;
                Ok(PendingPhaseOneResumeOutcome::progress(outcome))
            }
            Self::OpenStreaming(continuation) => continuation.resume().await,
        }
    }
}
