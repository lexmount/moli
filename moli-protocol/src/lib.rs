//! Protocol-neutral DevTools owner and dispatch layer for Moli.
//!
//! This crate is being split away from the Chrome DevTools Protocol wire shape.
//! It still contains transitional CDP-named owner types, but protocol-specific
//! parsing and Chrome protocol metadata belong in
//! `moli-protocol-cdp`.

mod cdp_projection;
pub mod conn;
pub mod devtools_runtime;
pub mod domains;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
pub mod testing;
pub mod version;

pub use devtools_runtime::*;

pub use conn::{
    AgentHostDispatchResult, BackgroundCommandResponsePayload, BackgroundProtocolEvent,
    CdpCommandTaskStep, CdpConnection, CdpInitialStoragePartition, CdpRendererCommandReplacement,
    CdpRendererCommandReplayDispatch, CdpRendererOwnerTurnOutcome, CdpSchedulerEvent,
    CdpTargetHostLifecycleDelta, CdpTargetHostLifecycleObserver, CdpTurnOutcome,
    CommandDispatchContext, CommandResponseFlushContext, CommandResponseFlushPermit,
    CompletedCdpCommandDispatch, CompletedDeferredMainDocumentLoadCompletion,
    CompletedRuntimeProtocolMessageDispatch, DEFAULT_CDP_PAGE_TARGET_ID, DEFAULT_CDP_TAB_TARGET_ID,
    DeferredMainDocumentLoadCompletionOutputAction,
    DeferredMainDocumentLoadCompletionOutputInterest, DeferredMainDocumentLoadObservationId,
    DeferredMainDocumentLoadPredecessorCandidate, DevToolsCommandDispatchOutcome,
    DevToolsDocumentLifecycleWaitKey, DevToolsDocumentLifecycleWaitState,
    DevToolsDocumentNavigationState, DevToolsPageResidenceIdentity, ParsedCdpCommand,
    PendingCdpCommandDispatch, PendingDeferredMainDocumentLoadCompletion,
    PendingRuntimeProtocolMessageDispatch, RendererDispatch, RendererDispatchBinding,
    RendererDispatchLane, RendererPageDispatchBinding,
};
pub use domains::activity::{
    ProtocolSchedulerWork, ProtocolSchedulerWorkKind, ProtocolWorkPublishSequence,
    RendererCommandResponseCompletion, RendererCommandResponseOrder, RendererCommandResponsePermit,
    RendererCommandResponseTerminal,
};
pub use domains::page::{
    BackgroundNavigationCompletion, CompletedPageScreencastCapture,
    PageScreencastCaptureCompletion, PageScreencastCaptureStart, PageScreencastRegistration,
    PageScreencastSubscriptionStatus, PendingPageScreencastCapture, build_default_raster_pdf,
};
pub use domains::runtime::{
    CompletedDevToolsRuntimeCommandDispatch, DevToolsRuntimeCommandTaskStep,
    PendingDevToolsRuntimeCommandDispatch,
};
