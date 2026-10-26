//! Protocol-neutral automation owner and dispatch layer for Moli.
//!
//! This crate is being split away from the Chrome DevTools Protocol wire shape.
//! It still contains transitional CDP-named owner types, but protocol-specific
//! parsing and Chrome protocol metadata belong in
//! `moli-protocol-cdp`.

pub mod automation;
mod cdp_projection;
pub mod conn;
pub mod domains;
pub mod frame_index;
#[cfg(feature = "test-support")]
pub mod test_support;
#[cfg(test)]
pub mod testing;
pub mod version;

pub use automation::*;

pub use conn::{
    AutomationCommandDispatchOutcome, BackgroundCommandResponsePayload, BackgroundProtocolEvent,
    CdpCommandTaskStep, CdpConnection, CdpInitialStoragePartition, CdpRendererCommandAccess,
    CdpRendererCommandReplacement, CdpRendererCommandReplayDispatch, CdpRendererOwnerTurnOutcome,
    CdpSchedulerEvent, CdpTargetHostLifecycleDelta, CdpTargetHostLifecycleObserver, CdpTurnOutcome,
    CommandDispatchContext, CommandResponseFlushContext, CommandResponseFlushPermit,
    CommandWaitLifetime, CompletedCdpCommandDispatch, CompletedDeferredMainDocumentLoadCompletion,
    CompletedProtocolSchedulerWork, CompletedRuntimeProtocolMessageDispatch,
    DEFAULT_CDP_PAGE_TARGET_ID, DEFAULT_CDP_TAB_TARGET_ID,
    DeferredMainDocumentLoadCompletionOutputAction,
    DeferredMainDocumentLoadCompletionOutputInterest, DeferredMainDocumentLoadObservationId,
    DeferredMainDocumentLoadPredecessorCandidate, DevToolsDocumentLifecycleWaitKey,
    DevToolsDocumentLifecycleWaitState, DevToolsDocumentNavigationState,
    DevToolsDocumentResidenceIdentity, DevToolsPageResidenceIdentity, OwnedProtocolWaitFailure,
    OwnedProtocolWaitTask, ParsedCdpCommand, PendingCdpCommandDispatch,
    PendingDeferredMainDocumentLoadCompletion, PendingProtocolSchedulerWork,
    PendingRuntimeProtocolMessageDispatch, ProtocolStateHandle, ProtocolStateOwner,
    ProtocolWorkflowState, spawn_owned_protocol_wait,
};
pub use domains::activity::{
    ProtocolNavigationDependency, ProtocolSchedulerWork, ProtocolSchedulerWorkKind,
    ProtocolWorkPublishSequence, RuntimeCommandOutputBarrierCompletion,
    RuntimeCommandOutputBarrierPermit, RuntimeCommandOutputBarrierTerminal,
    RuntimeCommandOutputBarriers,
};
pub use domains::page::{
    BackgroundNavigationCompletion, CompletedPageScreencastCapture,
    PageScreencastCaptureCompletion, PageScreencastCaptureStart, PageScreencastRegistration,
    PageScreencastSubscriptionStatus, PendingPageScreencastCapture,
};
pub use domains::runtime::{
    CompletedDevToolsRuntimeCommandDispatch, DevToolsRuntimeCommandTaskStep,
    PendingDevToolsRuntimeCommandDispatch,
};
