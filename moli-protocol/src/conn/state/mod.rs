mod attachment_identity;
mod bounds;
mod browser_context;
mod dedicated_worker_target;
mod devtools_renderer_channel;
mod devtools_session;
mod document_lifecycle_observer;
mod emulation;
mod fetch;
mod identity;
mod inspector;
mod javascript_dialog;
mod navigation;
mod navigation_outcome;
mod page_residence_token;
mod page_resource;
mod page_slot;
mod parking;
mod pending_renderer_command;
mod profiler;
mod runtime_slot;
mod service_worker_lifetime;
mod service_worker_target;
mod session;
mod session_storage;
mod shared_worker_attachment;
mod shared_worker_target;
#[cfg(test)]
mod tests;

// Re-export everything so `use super::state::*` paths continue to work.

pub(crate) use attachment_identity::{NavigationRequestId, TargetPageAttachmentId};
pub use identity::TargetPageResidenceIdentity as DevToolsPageResidenceIdentity;
pub use identity::URL_BASE;
pub(crate) use identity::{
    RendererPageResidenceIdentity, TargetIdentityState, TargetPageProtocolAttachmentIdentity,
    TargetPageResidenceIdentity, TargetRootDocumentProtocolAttachmentIdentity,
};

pub(crate) use devtools_renderer_channel::{
    CommittedRendererAgentAttachment, DevToolsRendererChannelError,
    PreparedRendererAgentAttachment, RendererAgentAttachment,
};

pub(crate) use dedicated_worker_target::{
    DedicatedWorkerMainScriptOutcome, DedicatedWorkerMainScriptSnapshot, DedicatedWorkerTargetState,
};
pub(crate) use devtools_session::{
    DevToolsConsoleOutputSessionState, DevToolsLogViolationThreshold, DevToolsSessionState,
    PreparedRendererCallReplacements, SessionRendererCallReplay, SessionRendererCallTermination,
    devtools_sessions_have_pending_inspector_awaits,
    devtools_sessions_pending_inspector_await_count,
    drain_pending_inspector_awaits_for_devtools_sessions,
    page_bypass_csp_enabled_for_devtools_sessions,
    prepare_renderer_call_replacements_for_devtools_sessions, runtime_bindings_for_renderer,
};
pub(crate) use document_lifecycle_observer::{
    RendererDocumentLifecycleObservation, RendererDocumentLifecycleObserver,
};
pub(crate) use page_residence_token::{TargetPageResidenceObservation, TargetPageResidenceToken};

pub use bounds::BrowserWindowBounds;

pub(crate) use page_resource::MainDocumentResourceSnapshot;
#[cfg(test)]
pub(crate) use page_slot::TargetPageSlot;
pub(crate) use page_slot::{
    CommittedRendererDocumentBinding, DocumentNavigationToken, InitialDocumentPageBuildWaiter,
    RendererDocumentLifecycleWaiterId, TargetPageAbsenceReason,
};
pub use page_slot::{DocumentStartScript, IsolatedWorldDefinition, RuntimeBindingDefinition};

pub(crate) use runtime_slot::{FinishedRendererDocumentNavigation, TargetRuntimeSlot};

pub use fetch::{ParkedFetchState, TargetFetchConfig};
pub(crate) use fetch::{
    TargetFetchOwner, TargetFetchState, TargetFetchSubresourceInterceptionSnapshot,
};

pub(crate) use inspector::InspectorCommandDispatch;
#[cfg(test)]
pub(crate) use javascript_dialog::TargetJavaScriptDialog;
pub(crate) use javascript_dialog::{
    TargetJavaScriptDialogScope, TargetJavaScriptDialogScopeObserver,
    TargetPreparedJavaScriptDialog, TargetPreparedJavaScriptDialogRoute,
};
pub(crate) use pending_renderer_command::{
    DuplicatePendingRendererCommand, PendingRendererCommandKey, PreparedRendererCallDispatch,
    PreparedRendererCallTermination, RegisterRendererCallError, RendererCommandCorrelation,
    RendererCommandDescriptor, RendererCommandReplay,
};
pub(crate) use profiler::{ProfilerAction, ProfilerInspectorCommand};
pub(crate) use service_worker_lifetime::{
    TargetServiceWorkerProtocolAttachmentIdentity, TargetServiceWorkerProtocolAttachmentRetirement,
    TargetServiceWorkerRunIdentity, TargetServiceWorkerRunRetirement,
    TargetServiceWorkerRuntimeAttachmentIdentity, TargetServiceWorkerVersionIdentity,
    TargetServiceWorkerVersionRetirement,
};
pub(crate) use service_worker_target::{
    ServiceWorkerRuntimeExceptionSnapshot, ServiceWorkerTargetState,
};
#[cfg(test)]
pub(crate) use session::TargetPerformanceSessionState;
pub(crate) use session::{
    PageScreencastConfig, PageScreencastFormat, PerformanceTimeDomain, TargetNetworkPolicyState,
    TargetPageSessionState, TargetRuntimeSessionState,
};
pub(crate) use session_storage::TargetSessionStorageNamespace;
pub(crate) use shared_worker_attachment::{
    TargetSharedWorkerProtocolAttachmentIdentity, TargetSharedWorkerProtocolAttachmentRetirement,
};
pub(crate) use shared_worker_target::SharedWorkerTargetState;

pub use browser_context::BrowserContext;
pub(crate) use browser_context::{
    BrowserContextPageStorageHandles, BrowserContextResourceStorageHandles,
    BrowserContextStoragePartitionHandles, SiteDataClearOptions,
};

pub use navigation::{PageNavigationHistoryEntry, PendingNavigationHistoryUpdate};

pub use parking::{BackgroundTarget, ParkedNetworkArtifacts, ParkedPageSessionState};
pub(crate) use parking::{
    ParkedTargetAuxState, ParkedTargetOwnerState, PendingBidiChannelListener,
    PendingInspectorAwait, TargetInitialEmptyDocumentCreator, TargetOwnerState, TargetSlotState,
    TargetWindowSurfaceState,
};

pub use emulation::{
    EmulatedDeviceMetrics, EmulatedGeolocationOverride, EmulatedGeolocationOverrideState,
    EmulatedMediaOverrides,
};
pub(crate) use emulation::{
    EmulatedNetworkConditions, EmulatedViewportSurface, viewport_surface_install_script,
};

pub(crate) use navigation_outcome::{CompletedDownloadBody, CompletedDownloadBodyArtifact};
pub use navigation_outcome::{
    DownloadNavigation, LoadedNavigation, NavigationDispatchState, NavigationLoadOutcome,
    NavigationRequestLoadPolicy, TargetInfo,
};
pub(crate) use navigation_outcome::{
    NETWORK_ERROR_PAGE_URL, NavigationResultProjection, NavigationSourceDocumentSecurityContext,
    NetworkErrorPageNavigation, RendererMainDocumentCommitSeed,
};
