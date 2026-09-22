//! Protocol-neutral DevTools owner and dispatch layer for Moli.
//!
//! CDP, BiDi and Classic share this dispatch layer. Browser state is owned by
//! `moli-core::browser::BrowserService`; this crate owns DevTools sessions and
//! event projection. CDP parsing and protocol metadata belong in `moli-protocol-cdp`.

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
    CdpConnection, CdpInitialStoragePartition, CdpRendererCommandReplacement,
    CdpRendererCommandReplayDispatch, CdpRendererOwnerTurnOutcome, CdpSchedulerEvent,
    CdpTargetHostLifecycleDelta, CdpTargetHostLifecycleObserver, CdpTurnOutcome,
    CommandDispatchContext, CommandResponseFlushContext, CommandResponseFlushPermit,
    CompletedCdpCommandDispatch, CompletedRuntimeProtocolMessageDispatch,
    DEFAULT_CDP_PAGE_TARGET_ID, DEFAULT_CDP_TAB_TARGET_ID, DevToolsCommandDispatchOutcome,
    DevToolsDocumentLifecycleWaitKey, DevToolsDocumentLifecycleWaitState,
    DevToolsDocumentNavigationState, DevToolsPageResidenceIdentity, ParsedCdpCommand,
    PendingCdpCommandDispatch, PendingRuntimeProtocolMessageDispatch, RendererDispatch,
    RendererDispatchBinding, RendererDispatchLane, RendererPageDispatchBinding,
};
pub use domains::activity::{
    ProtocolSchedulerWork, ProtocolSchedulerWorkKind, ProtocolWorkPublishSequence,
    RendererCommandResponseCompletion, RendererCommandResponseOrder, RendererCommandResponsePermit,
    RendererCommandResponseTerminal,
};
pub use domains::page::{
    CompletedDevToolsNavigationCommandDispatch, CompletedPageScreencastCapture,
    DevToolsNavigationCommandTaskStep, PageScreencastCaptureCompletion, PageScreencastCaptureStart,
    PageScreencastRegistration, PageScreencastSubscriptionStatus,
    PendingDevToolsNavigationCommandDispatch, PendingPageScreencastCapture,
    build_default_raster_pdf,
};
pub use domains::runtime::{
    CompletedDevToolsRuntimeCommandDispatch, DevToolsRuntimeCommandTaskStep,
    PendingDevToolsRuntimeCommandDispatch,
};
