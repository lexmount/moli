mod browser_context;
mod browser_identity;
mod dedicated_worker_target;
mod devtools_renderer_channel;
mod devtools_session;
mod document_lifecycle_observer;
mod fetch;
mod identity;
mod inspector;
pub(in crate::conn) use browser_context::javascript_dialog;
mod navigation_outcome;
mod page_resource;
pub(in crate::conn) use browser_context::page_slot;
mod page_agent_host;
mod pending_renderer_command;
mod profiler;
pub(in crate::conn) use browser_context::runtime_slot;
mod service_worker_lifetime;
mod service_worker_target;
pub(in crate::conn) use browser_context::session;
mod shared_worker_attachment;
mod shared_worker_target;
mod target_state;
#[cfg(test)]
mod tests;
pub(in crate::conn) use moli_core::browser::web_contents::InitialDocumentBuildKey;
pub(crate) use moli_core::browser::web_contents::{
    ClaimedNavigationRequest, NavigationInterceptionPermit,
};

// Re-export everything so `use super::state::*` paths continue to work.

pub(crate) use browser_identity::BaseBrowserIdentityOverrideState;
pub use identity::TargetPageResidenceIdentity as DevToolsPageResidenceIdentity;
pub use identity::URL_BASE;
pub(crate) use identity::{
    TargetIdentityState, TargetPageProtocolAttachmentIdentity, TargetPageResidenceIdentity,
    TargetRootDocumentProtocolAttachmentIdentity,
};
pub(crate) use moli_core::browser::{DocumentId, NavigationId, RendererPageResidenceIdentity};

pub(crate) use devtools_renderer_channel::{
    DevToolsRendererChannelError, DocumentProjectionFence, RendererAgentAttachment,
    RendererAgentBinding,
};

pub(crate) use dedicated_worker_target::DedicatedWorkerTargetState;
#[cfg(test)]
pub(crate) use devtools_session::DevToolsEmulationSessionState;
pub(crate) use devtools_session::{
    DevToolsBrowserIdentityOverride, DevToolsConsoleOutputSessionState,
    DevToolsLogViolationThreshold, DevToolsNetworkSessionState, DevToolsSessionState,
    PreparedRendererCallReplacements, SessionRendererCallReplay, SessionRendererCallTermination,
};
pub(crate) use document_lifecycle_observer::{
    RendererDocumentLifecycleObservation, RendererDocumentLifecycleObserver,
};

pub(crate) use page_resource::MainDocumentResourceSnapshot;
#[cfg(test)]
pub(crate) use page_slot::TargetPageSlot;
pub(crate) use page_slot::{
    CommittedRendererDocumentBinding, RendererDocumentLifecycleWaiterId, TargetPageAbsenceReason,
};
pub use page_slot::{DocumentStartScript, IsolatedWorldDefinition, RuntimeBindingDefinition};

pub(crate) use runtime_slot::{DocumentProjectionOutputRelease, TargetRuntimeSlot};

pub use fetch::TargetFetchConfig;
pub(crate) use fetch::{TargetFetchOwner, TargetFetchSubresourceInterceptionSnapshot};

pub(crate) use inspector::InspectorCommandDispatch;
#[cfg(test)]
pub(crate) use javascript_dialog::TargetJavaScriptDialog;
pub(crate) use javascript_dialog::{
    TargetJavaScriptDialogScope, TargetJavaScriptDialogScopeObserver,
    TargetPreparedJavaScriptDialog, TargetPreparedJavaScriptDialogRoute,
};
#[cfg(test)]
pub(crate) use moli_core::browser::web_contents::JavaScriptDialogKey;
pub(crate) use moli_core::browser::web_contents::LIVE_DEVICE_METRICS_CLEAR_SCRIPT;
pub(in crate::conn) use moli_core::browser::web_contents::PageSurface;
pub(crate) use moli_core::browser::web_contents::SameDocumentNavigationCommitted;
pub(crate) use moli_core::browser::web_contents::{
    CommittedDocumentLifecycle, DocumentLifecycleEvent,
};
pub(crate) use moli_core::browser::web_contents::{
    EmulationPolicy, EmulationPolicyChange, SessionStorageNamespace, WindowSurface,
    WindowSurfaceState,
};
pub(crate) use moli_core::browser::web_contents::{
    JavaScriptDialogClosed, JavaScriptDialogError, JavaScriptDialogSnapshot,
};
pub(crate) use pending_renderer_command::{
    DuplicatePendingRendererCommand, PreparedRendererCallDispatch, PreparedRendererCallTermination,
    RegisterRendererCallError, RendererCommandCorrelation, RendererCommandDescriptor,
    RendererCommandReplay,
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
pub(crate) use session::{
    PageScreencastConfig, PageScreencastFormat, PerformanceTimeDomain, TargetPageSessionState,
    TargetRuntimeSessionState,
};
pub(crate) use shared_worker_attachment::{
    TargetSharedWorkerProtocolAttachmentIdentity, TargetSharedWorkerProtocolAttachmentRetirement,
};
pub(crate) use shared_worker_target::SharedWorkerTargetState;

pub use browser_context::BrowserContext;
pub(crate) use browser_context::PageInputCommand;
pub(crate) use browser_context::{
    BrowserAppManifestLoadPreparation, CompletedAppManifestLoadPreparation,
    CompletedAppManifestPublication, CompletedCaptureDocumentImage,
    CompletedCaptureDocumentScreencastFrame, CompletedCaptureDocumentSnapshot,
    CompletedChildFrameNavigation, CompletedChildFrameTreeSnapshot,
    CompletedDocumentAutofillTrigger, CompletedDocumentBlobRead,
    CompletedDocumentCookieOwnerSnapshot, CompletedDocumentCspBypassUpdate,
    CompletedDocumentDiagnosticsSnapshot, CompletedDocumentFetchCommand,
    CompletedDocumentInputCommand, CompletedDocumentLifecycleStop, CompletedDocumentPolicyUpdate,
    CompletedDocumentResourceRuntimeUpdate, CompletedDocumentResourceTextSearch,
    CompletedDocumentStorageKeySnapshot, CompletedNavigationHistoryReset,
    CompletedNetworkResourceLoadPreparation, CompletedSetDocumentContent,
    CompletedTopLevelHistoryTraversal, CompletedTopLevelSameDocumentNavigation,
    DocumentFetchCommand, DocumentFetchCommandOutcome, DocumentPolicyUpdate,
    DocumentRuntimePolicyReconciliation, PendingAppManifestLoadPreparation,
    PendingAppManifestPublication, PendingCaptureDocumentImage,
    PendingCaptureDocumentScreencastFrame, PendingCaptureDocumentSnapshot,
    PendingChildFrameLifecycleWork, PendingChildFrameNavigation, PendingChildFrameTreeSnapshot,
    PendingDocumentAutofillTrigger, PendingDocumentBlobRead, PendingDocumentCookieOwnerSnapshot,
    PendingDocumentCspBypassUpdate, PendingDocumentDiagnosticsSnapshot,
    PendingDocumentFetchCommand, PendingDocumentInputCommand, PendingDocumentLifecycleStop,
    PendingDocumentPolicyBatch, PendingDocumentPolicyUpdate, PendingDocumentResourceRuntimeUpdate,
    PendingDocumentResourceTextSearch, PendingDocumentStorageKeySnapshot,
    PendingNavigationHistoryReset, PendingNetworkResourceLoadPreparation,
    PendingSetDocumentContent, PendingTopLevelHistoryTraversal,
    PendingTopLevelSameDocumentNavigation,
};
#[cfg(test)]
pub(crate) use moli_core::browser::BrowserContextResourceStorageHandles;
pub(crate) use moli_core::browser::{
    BrowserContextPageStorageHandles, BrowserContextStoragePartitionHandles, ContextNetworkPolicy,
    SiteDataClearOptions,
};
pub(crate) use moli_core::browser::{
    CompletedContextPermissionUpdate, PendingContextPermissionUpdate,
};

pub use moli_core::browser::web_contents::PageNavigationHistoryEntry;
pub(crate) use moli_core::browser::web_contents::{
    HistoryTraversalDestination, InitialDocumentCreator, InitialDocumentSnapshot,
    ResolvedHistoryTraversal,
};

pub use moli_core::browser::{
    EmulatedDeviceMetrics, EmulatedGeolocationOverride, EmulatedGeolocationOverrideState,
    EmulatedMediaOverrides,
};
pub(crate) use moli_core::browser::{
    EmulatedNetworkConditions, EmulatedViewportSurface, viewport_surface_install_script,
};
pub use page_agent_host::PageAgentHost;
pub(crate) use target_state::{
    PendingBidiChannelListener, PendingInspectorAwait, TargetOwnerState,
};

pub(crate) use navigation_outcome::{NETWORK_ERROR_PAGE_URL, NavigationResultProjection};
pub use navigation_outcome::{NavigationDispatchState, NavigationRequestLoadPolicy, TargetInfo};
