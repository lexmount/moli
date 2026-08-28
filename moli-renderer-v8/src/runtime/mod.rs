#[cfg(debug_assertions)]
use std::thread::ThreadId;
use std::{
    cell::RefCell,
    collections::HashMap,
    fmt,
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use super::native_bridge::element::ClientRect;
use crate::DocumentStartScript;
#[cfg(test)]
use crate::dom::native::NativeDom;
use crate::{
    browsing_context_model::BrowsingContextId,
    dom::NodeId,
    network::{ResourceRequestClient, context::DocumentResourceLoader},
};
use anyhow::{Result, anyhow, ensure};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tracing::debug;
use url::Url;

use super::{
    document_runtime::DocumentProcessingAction,
    document_script_scheduler::DocumentScriptScheduler,
    local_executor::{JsLocalExecutor, is_on_named_owner_execution_lane_for},
    native_bridge::PendingRuntimeBindingCall,
    page_task_queue::PageTaskQueue,
    planning::PreparedScript,
    script_vm::ScriptVm,
    types::{ScriptExecutionReport, ScriptRun, SubresourceResponseWaitCriteria},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererPageStateCapturePolicy {
    FullReport,
    ProtocolTurn,
}

mod access;
mod browser_context_runtime;
mod document_lifecycle;
mod document_lifecycle_turn;
mod javascript_dialog;
mod lifecycle_decision;
mod main_document_ready_gate;
mod navigation;
mod nested_main;
mod owner;
mod owner_deadline_index;
mod owner_local;
mod owner_local_store;
mod owner_maintenance;
mod page;
mod page_commands;
mod page_context_cancel;
mod page_creation_progress;
mod page_css;
pub(crate) mod page_dom;
mod page_dom_snapshot;
mod page_dump;
mod page_entry_residence;
mod page_generated_dom;
mod page_geometry;
mod page_network;
mod page_screenshot;
mod page_state;
mod page_surface;
mod page_turn_scheduler;
mod page_vm;
pub(crate) use page_vm::dom_agent_state::RendererDomAgentState;
mod phase_one;
mod protocol_output;
mod script_preloads;
mod service_worker_run;

pub(crate) use self::script_preloads::{
    BufferedScriptPreloadKey, BufferedScriptPreloadRequest, DocumentScriptPreloadStore,
    IncrementalBufferedScriptPreloadScanner,
};

pub use self::page_creation_progress::{RendererPageCreationPhase, RendererPageCreationProgress};

pub(crate) use self::browser_context_runtime::RendererOutputTransportSenderSlot;
pub(in crate::runtime) use self::document_lifecycle_turn::PendingDocumentLifecycleTurn;
pub(crate) use self::page_turn_scheduler::{
    PageOwnerBlockedReason, PageOwnerTurnOutcome, PageOwnerTurnReadiness,
};
pub(crate) use self::page_vm::AuthorizedCurrentBroadcastChannelDelivery;
pub(crate) use self::page_vm::AuthorizedCurrentPageChildClassicScriptSourceLoad;
pub(crate) use self::page_vm::AuthorizedCurrentPageChildDocumentLifecycle;
pub(crate) use self::page_vm::AuthorizedCurrentPageChildDocumentScriptReady;
pub(crate) use self::page_vm::AuthorizedCurrentPageChildHostLoad;
pub(crate) use self::page_vm::AuthorizedCurrentPageChildNavigationCommit;
pub(crate) use self::page_vm::AuthorizedCurrentPageChildParserModuleRootStart;
pub(crate) use self::page_vm::AuthorizedCurrentPageChildRealmMaterialization;
pub(crate) use self::page_vm::AuthorizedCurrentPageDedicatedWorkerClientEvent;
pub(crate) use self::page_vm::AuthorizedCurrentPageElementToggleEvent;
pub(crate) use self::page_vm::AuthorizedCurrentPageFileEntryFileCallback;
pub(crate) use self::page_vm::AuthorizedCurrentPageFileReadingTask;
pub(crate) use self::page_vm::AuthorizedCurrentPageHashChangeDelivery;
pub(crate) use self::page_vm::AuthorizedCurrentPageHistoryTraversal;
pub(crate) use self::page_vm::AuthorizedCurrentPageImageLoadEvent;
pub(crate) use self::page_vm::AuthorizedCurrentPageIndexedDbTask;
pub(crate) use self::page_vm::AuthorizedCurrentPageMediaElementEvent;
pub(crate) use self::page_vm::AuthorizedCurrentPageMessagePortDelivery;
pub(crate) use self::page_vm::AuthorizedCurrentPageMiscPlatformApiTask;
pub(crate) use self::page_vm::AuthorizedCurrentPageModuleReaction;
pub(crate) use self::page_vm::AuthorizedCurrentPageNavigationApiTask;
pub(crate) use self::page_vm::AuthorizedCurrentPageOpfsTask;
pub(crate) use self::page_vm::AuthorizedCurrentPageRenderingUpdate;
pub(crate) use self::page_vm::AuthorizedCurrentPageServiceWorkerClientMessage;
pub(crate) use self::page_vm::AuthorizedCurrentPageServiceWorkerInternalTask;
pub(crate) use self::page_vm::AuthorizedCurrentPageSharedWorkerClientEvent;
pub(crate) use self::page_vm::AuthorizedCurrentPageStorageEventDelivery;
pub(crate) use self::page_vm::AuthorizedCurrentPageTextTrackDefaultMode;
pub(crate) use self::page_vm::AuthorizedCurrentPageTextTrackLoad;
pub(crate) use self::page_vm::AuthorizedCurrentPageUserInteractionTask;
pub(crate) use self::page_vm::AuthorizedCurrentPageViewTransitionUpdate;
pub(crate) use self::page_vm::AuthorizedCurrentPageWebCryptoTask;
pub(crate) use self::page_vm::AuthorizedCurrentPageWindowMessage;
#[cfg(test)]
pub(crate) use self::page_vm::PageDomManipulationTestFamily;
pub(crate) use self::page_vm::backend_node_registry::SharedRendererBackendNodeRegistry;
#[cfg(test)]
pub(crate) use self::page_vm::backend_node_registry::new_shared_renderer_backend_node_registry;
#[cfg(test)]
pub(crate) use self::page_vm::test_support::PageVmTaskExecutorTestHarness;
#[cfg(test)]
pub(crate) use self::page_vm::{IntoPageTaskCompletion, PageTaskCompletion};
pub(super) const MAX_PENDING_LOCATION_NAVIGATION_TURNS: usize = 32;

/// Opaque identity of the exact frontend Runtime command that produced one
/// renderer-side effect.
///
/// Protocol creates the command and can compare this value with its own
/// one-shot response barrier. Renderer only propagates it across the bounded
/// owner transition that surfaced the effect; it must never infer command
/// ownership from a Page, Document, wake source, or time window.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RendererRuntimeCommandCausalIdentity {
    inspector_session_id: Option<String>,
    call_id: i32,
}

impl RendererRuntimeCommandCausalIdentity {
    pub fn new(inspector_session_id: Option<String>, call_id: i32) -> Self {
        Self {
            inspector_session_id,
            call_id,
        }
    }

    pub fn inspector_session_id(&self) -> Option<&str> {
        self.inspector_session_id.as_deref()
    }

    pub fn call_id(&self) -> i32 {
        self.call_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererOwnerResourceActivitySource {
    AsyncSubresource,
    /// Parser-blocking classic script source fetches can pause page creation
    /// before the page is installed. CDP must surface their Fetch pause before
    /// deferred main-document load completion can make progress.
    ParserBlockingScriptFetchInterception,
    WebSocket,
    /// User-visible worker lifecycle/message completions.
    Worker,
    /// Worker fetch/XHR subresource records that bridge back into CDP-visible Network state.
    WorkerSubresource,
    /// Worker fetch/XHR request-stage pauses that bridge back into Fetch.requestPaused.
    WorkerFetchInterception,
    /// Worker fetch/XHR cancellations that produce terminal Network and continue output.
    WorkerFetchCancellation,
    /// Worker fetch/XHR continue results that can produce response/auth/completion follow-up.
    WorkerContinueEvent,
    /// Worker WebSocket lifecycle/frame records that bridge back into CDP-visible Network state.
    WorkerWebSocket,
    ChildDocument,
    ChildClassicScript,
    ChildBlockingStylesheet,
    Stylesheet,
    ModuleGraphFetch,
    ServiceWorker,
    WebCryptoTask,
    StorageIo,
    DocumentWriteExternalScript,
    MainParserDeferredClassicSource,
    MessagePort,
    BroadcastChannel,
    StorageEvent,
    SharedWorker,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererOwnerRuntimeActivitySource {
    /// Broad output attribution for an already-selected runtime action whose
    /// concrete task identity stays inside the renderer. This is not a Page
    /// task source and grants protocol code no execution authority.
    SelectedTaskOutput,
    /// A host timer callback already selected and settled by the Page owner.
    Timer,
    /// One task from the HTML navigation-and-traversal task source.
    NavigationAndTraversal,
    /// One task from the HTML rendering task source.
    RenderingUpdate,
    /// One task from the HTML media-element event task source.
    MediaElementEvent,
    /// One runtime-visible task from the HTML DOM-manipulation task source.
    DomManipulation,
    /// One runtime-visible callback from the HTML networking task source.
    Networking,
    /// One selection/select/dialog event from the HTML user-interaction task source.
    UserInteraction,
    /// One directory-reader callback from the HTML file-reading task source.
    FileReading,
    /// One callback from the HTML miscellaneous-platform API task source.
    MiscPlatformApi,
    /// A Window.postMessage delivery already settled by the Page owner.
    WindowMessage,
    /// An IndexedDB request/transaction task already settled by the Page owner.
    IndexedDb,
    /// A document-owned task from the HTML internal-loading task source.
    InternalLoading,
    DocumentReplacement,
    /// Module promise/reaction records that were produced by V8 callbacks.
    ModuleReaction,
    /// A foreground continuation posted by V8 for this page's isolate.
    V8ForegroundTask,
    /// The post-DOMContentLoaded lifecycle driver advanced load-stage work.
    DocumentLifecycleTurn,
    /// A child LocalWindow default realm is ready to be materialized.
    ChildRealmMaterialization,
}

#[cfg(test)]
use self::access::{
    OwnerLocalRuntimeAccessPath, OwnerLocalRuntimeEntryPath, ScriptExecutionDomainPath,
    ScriptExecutionLanePath, is_on_parse_time_scaffold_lane, is_on_script_execution_domain_for,
    owner_local_runtime_access_path, owner_local_runtime_entry_path, script_execution_domain_path,
    script_execution_lane_path,
};
pub(crate) use self::browser_context_runtime::ServiceWorkerControlState;
pub use self::browser_context_runtime::{
    DetachedParserScriptFetchContinuation, RendererBrowserContextRuntime,
    RendererBrowserContextRuntimeOwner, RendererBrowserContextRuntimeOwnerAccess,
    RendererPopupBlockerPolicy, RendererReservedServiceWorkerClient,
    RendererServiceWorkerClientsOpenWindowContinuation, RendererServiceWorkerMainResourceFetch,
};
pub(crate) use self::browser_context_runtime::{
    RendererStoragePartitionIdentity, RendererWorkerContextRuntime,
};
pub(crate) use self::document_lifecycle::{
    RendererDocumentLifecycleDriveAdmission, RendererDocumentLifecycleJournalHandle,
    RendererDocumentLifecycleTransition,
};
pub use self::document_lifecycle::{
    RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
    RendererDocumentLifecycleIdentity, RendererDocumentLifecycleMilestone,
    RendererDocumentLifecycleSnapshot, RendererDocumentLifecycleWaitOutcome,
    RendererDocumentLifecycleWaiter, RendererDocumentTerminationReason, RendererDocumentToken,
    RendererFrameToken, RendererLifecycleEpoch, RendererLifecycleEventStamp,
    RendererLifecycleStartReason, RendererLifecycleTerminationStamp, RendererPageCreationArtifacts,
};
pub(crate) use self::javascript_dialog::{
    RendererJavaScriptDialogBroker, RendererJavaScriptDialogRuntime, RendererJavaScriptDialogWatch,
};
pub use self::javascript_dialog::{
    RendererJavaScriptDialogCompletion, RendererJavaScriptDialogResult,
    RendererPageCommandInterruptedByJavaScriptDialog,
};
pub use self::lifecycle_decision::{
    RendererLifecycleDecider, RendererLifecycleDecision, RendererLifecycleSnapshot,
};
use self::owner::RendererOwnerState;
pub use self::owner::{
    RendererOwnerCommand, RendererOwnerHandle, RendererOwnerReply,
    RendererPreparedDocumentCommitConfiguration,
};
pub use self::owner_local::RendererPageTestingHandle;
pub use self::owner_local::{
    RendererPageCommandPending, RendererPageHandle, RendererPageReplacementReservationPending,
    RendererRuntimeInspectorSessionDetachGuard,
};
pub(crate) use self::owner_local_store::RendererPageToken;
pub use self::page::{JsRuntime, JsRuntimeOwner, PendingHtmlPage, PreparedRendererDocument};
use self::page::{PageVmNavigationResponse, PageVmStateCapture};
pub(crate) use self::page_context_cancel::{
    RendererPageContextCancelReason, RendererPageContextCancelReceiver,
    RendererPageContextCancelSender, renderer_page_context_cancel_channel,
};
pub use self::page_screenshot::{
    RendererCaptureScreencastFrameRequest, RendererCaptureScreenshotRequest,
    RendererScreenshotClip, RendererScreenshotFormat, RendererScreenshotPurpose,
    RendererScreenshotRegion, RendererVisualStateToken,
};
pub(super) use self::page_state::RendererPageEntry;
pub use self::page_state::RendererPageRecord;
pub(crate) use self::page_state::RendererPageSlotHandle;
pub use self::page_state::RendererPageState;
use self::page_surface::RendererPageTable;
pub(crate) use self::page_surface::RendererPopupCreationUserActivation;
pub use self::page_surface::RendererRuntimeInspectorMessageResponseOrder;
pub use self::page_surface::{
    DevToolsSessionKey, RendererAccessibilityPayloadsForObjectId, RendererActivityDiagnostics,
    RendererAgentAttachmentId, RendererAutofillAddressField, RendererAutofillCreditCard,
    RendererAutofillTriggerOutcome, RendererAutofillTriggerRequest,
    RendererAuxiliaryBrowsingContextPolicy, RendererCaptureScreencastFrameReply,
    RendererCaptureScreenshotReply, RendererCapturedScreencastFrame, RendererCapturedScreenshot,
    RendererCommandTurnCompletion, RendererCommandTurnOutput, RendererCountEntry,
    RendererDedicatedWorkerTargetEvent, RendererDedicatedWorkerTargetInfo,
    RendererDevToolsAgentToken, RendererDocumentBoxModel, RendererDocumentChildNodeSnapshotEvent,
    RendererDocumentChildNodeSnapshotEvents, RendererDocumentChildNodeSnapshots,
    RendererDocumentFrontendNodeIdsResolution, RendererDocumentHitTestResult,
    RendererDocumentIsolateAccountingDiagnostics, RendererDocumentNodeAttributesResolution,
    RendererDocumentNodeClientRect, RendererDocumentNodeGeometry,
    RendererDocumentNodePropertyResolution, RendererDocumentNodeReference,
    RendererDocumentNodeTextResolution, RendererDocumentQuerySelectorNode,
    RendererDocumentQuerySelectorResolution,
    RendererDocumentQuerySelectorWithChildNodeSnapshotEvents,
    RendererDocumentSourcedSameDocumentNavigation,
    RendererDocumentSourcedTopLevelLocationNavigation, RendererDomAttributeMutation,
    RendererDomAttributeMutationOutcome, RendererDomBidiNodeBindingResolution,
    RendererDomBidiNodeSharedIdResolution, RendererDomDebuggerDomBreakpointResolution,
    RendererDomDebuggerEventListener, RendererDomDebuggerEventListenerBreakpoint,
    RendererDomDebuggerEventListenersResolution, RendererDomDebuggerXhrBreakpoint, RendererDomEdit,
    RendererDomEditOutcome, RendererDomFocusOutcome, RendererDomFrontendNodeBindingResolution,
    RendererDomMutationEvent, RendererDomMutationEventBatch, RendererDomNodeCreationStackFrame,
    RendererDomNodeCreationStackTrace, RendererDomNodeStackTraceResolution,
    RendererDomSearchRegistration, RendererDomSearchResultNode, RendererDomSearchResultsResolution,
    RendererDomSnapshotCaptureOptions, RendererDomSnapshotCapturePayload, RendererDragData,
    RendererDragDataItem, RendererDraggedDirectory, RendererDraggedFile, RendererGeometryQuad,
    RendererInputDispatchOutcome, RendererInspectorProtocolConfiguration,
    RendererInspectorProtocolConfigurationCommand, RendererInspectorSessionRestoreSnapshot,
    RendererJavaScriptDialogId, RendererJavaScriptDialogSource, RendererLayoutMetrics,
    RendererMainDocumentCommit, RendererMainDocumentResponseBlock,
    RendererMoliDomMemoryDiagnostics, RendererMoliMemoryDiagnostics,
    RendererMoliMemoryScopeDiagnostics, RendererMoliRuntimeMemoryDiagnostics, RendererPageCommand,
    RendererPageCommandPostResponseContinuation, RendererPageCookieFacadeSnapshotReply,
    RendererPageCreationDiagnostics, RendererPageDiagnosticsSnapshot, RendererPageDumpFormat,
    RendererPageDumpOptions, RendererPageDumpStripOptions, RendererPageReply, RendererPageView,
    RendererPendingDownloadActivation, RendererPendingDownloadResponse,
    RendererPendingFileChooserActivation, RendererPendingJavaScriptDialog,
    RendererPendingPopupActivation, RendererPendingSameDocumentNavigation,
    RendererPendingTopLevelHistoryTraversal, RendererPendingWindowOpenEvent,
    RendererPerformanceMetricSnapshot, RendererPointerEventProperties,
    RendererPopupActivationSource, RendererPopupDisposition, RendererPopupNewTargetDisposition,
    RendererRemoteWindowProxyCommand, RendererRemoteWindowProxySource, RendererResolvedPopupTarget,
    RendererResourceTextSearchOutcome, RendererRuntimeCommandOutput,
    RendererRuntimeEvaluationResult, RendererRuntimeHeapSpaceUsage, RendererRuntimeHeapUsage,
    RendererRuntimeInspectorAsyncCompletion, RendererRuntimeInspectorMessage,
    RendererRuntimeInspectorMessageBatch, RendererRuntimeInspectorProtocolMessage,
    RendererRuntimeInspectorProtocolMessageValueMut, RendererRuntimeInspectorResponseChannel,
    RendererRuntimeInspectorResponseSender, RendererRuntimeObservableSourceItem,
    RendererRuntimeObservableSourceSummary, RendererRuntimeRealmInfo, RendererRuntimeRemoteObject,
    RendererRuntimeRemoteObjectResolution, RendererScriptExecutionMemoryDiagnostics,
    RendererScriptSourceMemoryDiagnostics, RendererScrollIntoViewResult,
    RendererServiceWorkerConsoleMessage, RendererServiceWorkerExceptionMessage,
    RendererServiceWorkerFetchDiagnostic, RendererServiceWorkerFetchDiagnosticResult,
    RendererServiceWorkerTargetEvent, RendererServiceWorkerTargetInfo,
    RendererServiceWorkerVersionStatus, RendererSetDocumentContentResult,
    RendererSharedWorkerConsoleMessage, RendererSharedWorkerTargetEvent,
    RendererSharedWorkerTargetInfo, RendererStyleSheetHeader, RendererStyleSheetInventoryUpdate,
    RendererStyleSheetPayload, RendererSyntheticResponseBody, RendererTextSearchMatch,
    RendererTopLevelNavigationRequest, RendererTopLevelNavigationSource, RendererTouchPoint,
    RendererWindowDocumentSource, RuntimeConsoleMessageSnapshot,
};
pub(crate) use self::page_surface::{
    RendererCommandTurnOutputRecorder, RendererDevToolsSessionOutputHost,
    RendererInspectorPageCommand, RendererRuntimeCommandOutputRecorder,
    RendererRuntimeCommandOutputSettlement, RendererRuntimeInspectorResponsePublication,
    RendererRuntimeInspectorSessionResponseSettlement, RendererRuntimeObservableSourceQueue,
};
pub(crate) use self::page_surface::{
    RendererRemoteFrameNavigationId, RendererRemoteJavaScriptUrlSource,
    RendererRemoteJavaScriptUrlSourceWorld, RendererRemoteWindowProxyChannel,
    RendererRemoteWindowProxyCommandKind, RendererRemoteWindowProxyMessage,
    RendererRemoteWindowProxyNavigationKind,
};
pub(crate) use self::page_vm::PageVm;
use self::page_vm::PageVmDropTracker;
pub(crate) use self::page_vm::PageVmEnvConfig;
pub(crate) use self::page_vm::PageVmRuntimeHooks;
#[cfg(test)]
pub(crate) use self::page_vm::deferred_page_vm_drop_pending_count_for_testing;
pub(crate) use self::page_vm::{
    AuthorizedCurrentChildDocumentLoadCompletion, AuthorizedCurrentChildDynamicImportOwnerAction,
    AuthorizedCurrentChildModuleDependencyFetchStart, AuthorizedCurrentChildModuleFetchCompletion,
    AuthorizedCurrentChildModuleScriptTerminal, AuthorizedCurrentChildModulepreloadEventAction,
    AuthorizedCurrentChildModulepreloadStartTask,
    AuthorizedCurrentDocumentWriteExternalScriptLoadCompletion,
    AuthorizedCurrentMainDynamicImportGraphFetchCompletion,
    AuthorizedCurrentMainParserModuleGraphFetchCompletion,
    AuthorizedCurrentMainRuntimeModuleGraphFetchCompletion,
    AuthorizedLiveMainModulepreloadFetchCompletion, CurrentChildDocumentLoadApplication,
};
pub(in crate::runtime) use self::page_vm::{
    PageVmCommittedNavigationBootstrap, PageVmDocumentCommitPreparation,
    PageVmPreparedFollowedNavigationCommit,
};
pub use self::phase_one::ExternalRawDocumentBodyStream;
use self::phase_one::PendingPhaseOneResidence;
pub(in crate::runtime) use self::phase_one::{
    PhaseOneResidenceAdmission, PhaseOneRestoreRequirement,
};
pub(crate) use self::protocol_output::{PendingRendererOutputRecord, RendererTurnOutputJournal};
pub use self::protocol_output::{
    RendererDocumentTitleChanged, RendererOutputCursor, RendererOutputFence,
    RendererOutputFenceLeaseId, RendererOutputItem, RendererOutputPublication,
    RendererOutputPublicationOrdering, RendererOutputRecord, RendererOutputResidenceIdentity,
    RendererOutputStreamCloseReason, RendererOutputStreamControl, RendererOutputStreamEpoch,
    RendererOutputStreamIdentity, RendererOutputTransportDiagnostics,
    RendererOutputTransportMessage, RendererOutputTransportReceiver,
    RendererOutputTransportSendError, RendererOutputTransportSender, RendererOwnerAction,
    RendererPageOutputOwnerReservationId, RendererProtocolObservation, RendererTopLevelCloseSource,
    renderer_output_transport_channel,
};
pub use self::service_worker_run::RendererServiceWorkerRunIdentity;
pub use crate::devtools::command::{
    RendererDevToolsIoCommandEnvelope, RendererDevToolsMainCommandEnvelope,
    RendererInspectorCommandEnvelope, RendererInspectorCommandRoute,
    RendererInspectorIngressTicket,
};
pub(crate) use crate::devtools::command::{
    RendererDevToolsIoCommandKind, RendererDevToolsIoCommandPayload,
    RendererDevToolsMainNestedDispatch, RendererInspectorPauseCommandEffect,
};
pub use crate::devtools::ingress::io::{
    RendererRuntimeInspectorIoCommandClaim, RendererRuntimeInspectorIoCommandRoute,
};
pub use crate::devtools::ingress::main::{
    RendererRuntimeInspectorMainCommandCompletion, RendererRuntimeInspectorMainCommandRoute,
};
pub(crate) use crate::renderer::PageVmInitStage;
pub(crate) use crate::service_worker_runtime::{
    MaterializedServiceWorkerFetchResponseHead, ServiceWorkerClientFocus,
    ServiceWorkerClientFocusError, ServiceWorkerClientFocusResult, ServiceWorkerClientId,
    ServiceWorkerClientMessage, ServiceWorkerClientNavigate, ServiceWorkerClientNavigateError,
    ServiceWorkerClientNavigateResult, ServiceWorkerClientQuery, ServiceWorkerClientQueryKind,
    ServiceWorkerClientQueryOptions, ServiceWorkerClientQueryResult, ServiceWorkerClientQueryType,
    ServiceWorkerClientSnapshot, ServiceWorkerClientsOpenWindow,
    ServiceWorkerClientsOpenWindowError, ServiceWorkerClientsOpenWindowResult,
    ServiceWorkerCloseNotification, ServiceWorkerEventId, ServiceWorkerFetchCompletion,
    ServiceWorkerFetchEvent, ServiceWorkerFetchResponse, ServiceWorkerFetchResult,
    ServiceWorkerFetchStreamChunk, ServiceWorkerFetchStreamStarted, ServiceWorkerGetNotifications,
    ServiceWorkerGetNotificationsResult, ServiceWorkerLifecycleCompletion,
    ServiceWorkerLifecycleEvent, ServiceWorkerLifecycleEventKind, ServiceWorkerMessageCompletion,
    ServiceWorkerMessageEvent, ServiceWorkerNavigationPreloadFailure,
    ServiceWorkerNavigationPreloadResponseStarted, ServiceWorkerNavigationPreloadState,
    ServiceWorkerNavigationPreloadStateError, ServiceWorkerNavigationPreloadStreamChunk,
    ServiceWorkerNavigationPreloadStreamFinished, ServiceWorkerNotificationCompletion,
    ServiceWorkerNotificationEvent, ServiceWorkerNotificationMetadata,
    ServiceWorkerNotificationSnapshot, ServiceWorkerPeriodicSyncCompletion,
    ServiceWorkerPeriodicSyncEvent, ServiceWorkerPeriodicSyncGetTags,
    ServiceWorkerPeriodicSyncGetTagsResult, ServiceWorkerPeriodicSyncRegistration,
    ServiceWorkerPeriodicSyncRegistrationResult, ServiceWorkerPeriodicSyncUnregistration,
    ServiceWorkerPeriodicSyncUnregistrationResult, ServiceWorkerPushCompletion,
    ServiceWorkerPushEvent, ServiceWorkerPushGetSubscription,
    ServiceWorkerPushGetSubscriptionResult, ServiceWorkerPushSubscribe,
    ServiceWorkerPushSubscribeResult, ServiceWorkerPushSubscriptionSnapshot,
    ServiceWorkerPushUnsubscribe, ServiceWorkerPushUnsubscribeResult, ServiceWorkerRegistrationId,
    ServiceWorkerShowNotification, ServiceWorkerShowNotificationResult,
    ServiceWorkerSyncCompletion, ServiceWorkerSyncEvent, ServiceWorkerSyncGetTags,
    ServiceWorkerSyncGetTagsResult, ServiceWorkerSyncRegistration,
    ServiceWorkerSyncRegistrationResult, ServiceWorkerVersionId, ServiceWorkerWorkerMessage,
    service_worker_exposed_client_id,
};
pub(crate) use nested_main::dispatch_nested_main_page_command;

static NEXT_RENDERER_OWNER_LOCAL_HOST_ID: AtomicU64 = AtomicU64::new(1);

/// Stable process-local identity of one browser-context renderer runtime.
///
/// Page owners are replaceable execution lanes, so their
/// [`RendererOwnerLocalHostId`] cannot identify browser-context-scoped
/// SharedWorker and ServiceWorker output. Worker output streams use this
/// identity to route directly to the owning BrowserContext without pretending
/// to belong to an arbitrary live Page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RendererBrowserContextRuntimeId(u64);

impl RendererBrowserContextRuntimeId {
    pub(crate) fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub fn new_for_testing(raw: u64) -> Self {
        Self(raw)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RendererOwnerLocalHostId(u64);

impl RendererOwnerLocalHostId {
    fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub fn new_for_testing(raw: u64) -> Self {
        Self(raw)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub(crate) const fn from_wire(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }
}

thread_local! {
    static PAGE_VM_DROP_TRACKER: RefCell<PageVmDropTracker> =
        RefCell::new(PageVmDropTracker::default());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageId(u64);

impl PageId {
    fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn new_for_testing(raw: u64) -> Self {
        Self(raw)
    }

    pub(crate) const fn from_wire(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }
}

/// Selects the script environment admitted when a Page reservation is
/// consumed by document bootstrap.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RendererScriptAgentAdmission {
    Fresh,
    RelatedAuxiliaryPage {
        opener_page_id: PageId,
    },
    ExistingPageReplacement {
        expected_vm_creation_id: u64,
        reservation_nonce: u64,
    },
}

/// Opaque owner-local reservation for one renderer Page admission.
///
/// Initial creation reserves a future Page identity. A live replacement
/// reserves one exact committed `PageVm` generation of an existing identity.
/// Both are allocated before a queued full-body build or prepared document can
/// enter parser or author-script execution, so external observers never infer
/// ownership from whichever Page happens to be installed later.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RendererPageReservationToken {
    local_host_id: RendererOwnerLocalHostId,
    page_id: PageId,
    output_owner_reservation_id: RendererPageOutputOwnerReservationId,
    script_agent_admission: RendererScriptAgentAdmission,
    /// Whether this top-level browsing context was created by a DOM Window
    /// operation. This is Page metadata: it survives every Document/realm
    /// replacement and is consumed by the script-closable check.
    opened_by_dom: bool,
    /// Initial browser-owned Page focus at the instant this reservation is
    /// admitted. Existing-Page replacement ignores this snapshot and keeps
    /// the stable Page environment's current focus state.
    initially_active: bool,
    initially_focused: bool,
}

impl RendererPageReservationToken {
    fn new(local_host_id: RendererOwnerLocalHostId, page_id: PageId) -> Self {
        Self {
            local_host_id,
            page_id,
            output_owner_reservation_id: RendererPageOutputOwnerReservationId::allocate(),
            script_agent_admission: RendererScriptAgentAdmission::Fresh,
            opened_by_dom: false,
            initially_active: true,
            initially_focused: true,
        }
    }

    fn new_dom_auxiliary_page(local_host_id: RendererOwnerLocalHostId, page_id: PageId) -> Self {
        Self {
            local_host_id,
            page_id,
            output_owner_reservation_id: RendererPageOutputOwnerReservationId::allocate(),
            script_agent_admission: RendererScriptAgentAdmission::Fresh,
            opened_by_dom: true,
            initially_active: false,
            initially_focused: false,
        }
    }

    pub(crate) fn new_related_auxiliary_page(
        local_host_id: RendererOwnerLocalHostId,
        page_id: PageId,
        opener_page_id: PageId,
    ) -> Self {
        Self {
            local_host_id,
            page_id,
            output_owner_reservation_id: RendererPageOutputOwnerReservationId::allocate(),
            script_agent_admission: RendererScriptAgentAdmission::RelatedAuxiliaryPage {
                opener_page_id,
            },
            opened_by_dom: true,
            initially_active: false,
            initially_focused: false,
        }
    }

    pub(crate) fn new_existing_page_replacement(
        local_host_id: RendererOwnerLocalHostId,
        page_id: PageId,
        expected_vm_creation_id: u64,
        reservation_nonce: u64,
        output_owner_reservation_id: RendererPageOutputOwnerReservationId,
    ) -> Self {
        Self {
            local_host_id,
            page_id,
            output_owner_reservation_id,
            script_agent_admission: RendererScriptAgentAdmission::ExistingPageReplacement {
                expected_vm_creation_id,
                reservation_nonce,
            },
            // Replacement consumes the already-resident Page environment. The
            // value is intentionally not re-derived from the navigation source.
            opened_by_dom: false,
            initially_active: true,
            initially_focused: true,
        }
    }

    pub fn local_host_id(self) -> RendererOwnerLocalHostId {
        self.local_host_id
    }

    pub fn page_id(self) -> PageId {
        self.page_id
    }

    pub fn output_owner_reservation_id(self) -> RendererPageOutputOwnerReservationId {
        self.output_owner_reservation_id
    }

    pub(crate) fn script_agent_admission(self) -> RendererScriptAgentAdmission {
        self.script_agent_admission
    }

    pub(crate) fn opened_by_dom(self) -> bool {
        self.opened_by_dom
    }

    /// Rebinds initial focus before parser or author script can observe the
    /// reserved Page. Protocol uses this when an ordinary fresh reservation
    /// is assigned to an active or background target slot.
    pub fn with_initial_page_activation(mut self, active: bool, focused: bool) -> Self {
        self.initially_active = active;
        self.initially_focused = focused;
        self
    }

    pub(crate) fn initially_active(self) -> bool {
        self.initially_active
    }

    pub(crate) fn initially_focused(self) -> bool {
        self.initially_focused
    }
}

/// Renderer-owned identity reserved synchronously for a newly accepted
/// auxiliary top-level browsing context.
///
/// The typed browsing-context identity remains stable across Document
/// replacements, while the Page reservation is consumed exactly once by the
/// protocol target's initial empty-Document build. Reserving here prevents
/// protocol code from manufacturing a second Page identity for a popup the
/// renderer has already accepted.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RendererPendingAuxiliaryPage {
    browsing_context_id: BrowsingContextId,
    page_reservation: RendererPageReservationToken,
}

impl RendererPendingAuxiliaryPage {
    pub fn browsing_context_id(self) -> u64 {
        self.browsing_context_id.value()
    }

    pub fn page_reservation(self) -> RendererPageReservationToken {
        self.page_reservation
    }
}

/// Owner-local V8 state staged between synchronous popup acceptance and the
/// related auxiliary Page's initial realm bootstrap.
pub(crate) struct RendererStagedAuxiliaryWindowProxy {
    window_proxy: v8::Global<v8::Object>,
    facade_context: v8::Global<v8::Context>,
    inherited_security_token: Option<v8::Global<v8::Value>>,
}

/// Inputs captured from the creator Document for one real synchronous
/// auxiliary initial-empty Page realm.
pub(crate) struct RendererRelatedInitialEmptyPageRealmInit {
    pub(crate) dom_host: crate::dom::native::DomHost,
    pub(crate) loader: ResourceRequestClient,
    pub(crate) env: PageVmEnvConfig,
    pub(crate) inherited_origin: String,
    pub(crate) policy_container: crate::document_runtime::DocumentPolicyContainer,
    pub(crate) auxiliary_popup_id: u64,
    pub(crate) staged_window_proxy: RendererStagedAuxiliaryWindowProxy,
    pub(crate) opener: Option<v8::Global<v8::Object>>,
    pub(crate) window_name: String,
}

impl RendererStagedAuxiliaryWindowProxy {
    pub(crate) fn new(
        window_proxy: v8::Global<v8::Object>,
        facade_context: v8::Global<v8::Context>,
        inherited_security_token: Option<v8::Global<v8::Value>>,
    ) -> Self {
        Self {
            window_proxy,
            facade_context,
            inherited_security_token,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        v8::Global<v8::Object>,
        v8::Global<v8::Context>,
        Option<v8::Global<v8::Value>>,
    ) {
        (
            self.window_proxy,
            self.facade_context,
            self.inherited_security_token,
        )
    }
}

/// Page-local allocation and handoff capability for auxiliary browsing
/// contexts created synchronously by one opener realm.
///
/// Related Pages are now staged as a complete initial Page/Document/realm in
/// the opener's owner turn. The allocator therefore carries only typed Page
/// identity and the owner capability; it no longer keeps a second loose
/// WindowProxy registry for a later protocol-created Page to consume.
#[derive(Clone)]
pub(crate) struct RendererAuxiliaryPageReservationAllocator {
    owner: Option<owner_local_store::RendererOwnerLocalContext>,
    local_host_id: RendererOwnerLocalHostId,
    opener_page_id: PageId,
    next_page_id: Arc<AtomicU64>,
}

impl fmt::Debug for RendererAuxiliaryPageReservationAllocator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RendererAuxiliaryPageReservationAllocator")
            .field("local_host_id", &self.local_host_id)
            .field("opener_page_id", &self.opener_page_id)
            .finish_non_exhaustive()
    }
}

impl RendererAuxiliaryPageReservationAllocator {
    pub(in crate::runtime) fn new_for_owner(
        owner: owner_local_store::RendererOwnerLocalContext,
        opener_page_id: PageId,
    ) -> Self {
        let local_host_id = owner.local_host_id;
        let next_page_id = owner.owner_state.next_page_id.clone();
        Self {
            owner: Some(owner),
            local_host_id,
            opener_page_id,
            next_page_id,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(
        local_host_id: RendererOwnerLocalHostId,
        opener_page_id: PageId,
        next_page_id: Arc<AtomicU64>,
    ) -> Self {
        Self {
            owner: None,
            local_host_id,
            opener_page_id,
            next_page_id,
        }
    }

    pub(crate) fn reserve(
        &self,
        exposes_opener: bool,
        opened_by_dom: bool,
    ) -> RendererPendingAuxiliaryPage {
        let page_id = PageId::new(self.next_page_id.fetch_add(1, Ordering::Relaxed));
        let page_reservation = if exposes_opener {
            debug_assert!(
                opened_by_dom,
                "only a DOM-created auxiliary Page may expose an opener"
            );
            RendererPageReservationToken::new_related_auxiliary_page(
                self.local_host_id,
                page_id,
                self.opener_page_id,
            )
        } else if opened_by_dom {
            RendererPageReservationToken::new_dom_auxiliary_page(self.local_host_id, page_id)
        } else {
            RendererPageReservationToken::new(self.local_host_id, page_id)
        };
        RendererPendingAuxiliaryPage {
            browsing_context_id: BrowsingContextId::auxiliary_top_level(page_id.as_u64()),
            page_reservation,
        }
    }

    pub(crate) fn stage_related_initial_empty_page_in_scope(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        pending: RendererPendingAuxiliaryPage,
        source_environment: &crate::script_vm::RendererPageScriptEnvironment,
        source_bridge_bindings: &crate::native_bridge::bindings::NativeBridgeBindings,
        init: RendererRelatedInitialEmptyPageRealmInit,
    ) -> Result<()> {
        let owner = self.owner.as_ref().ok_or_else(|| {
            anyhow!("standalone auxiliary allocator cannot stage a production Page realm")
        })?;
        owner_local_store::stage_related_initial_empty_page_on_bound_owner_local_store(
            owner,
            scope,
            pending,
            source_environment,
            source_bridge_bindings,
            init,
        )
    }
}

/// Typed authority to consume one matching prepared document and enter its
/// renderer bootstrap.
#[derive(Debug)]
pub struct RendererDocumentCommitPermit {
    prepared_document: RendererPageReservationToken,
}

impl RendererDocumentCommitPermit {
    fn new(prepared_document: RendererPageReservationToken) -> Self {
        Self { prepared_document }
    }

    fn prepared_document(&self) -> RendererPageReservationToken {
        self.prepared_document
    }
}

/// Result of committing a prepared Document into an already-owned renderer
/// Page.
///
/// Unlike initial Page creation, this value deliberately carries no
/// [`RendererPageHandle`]. The existing handle remains the sole Page close and
/// command authority; callers only adopt the replacement Document's state and
/// DevTools agent identity.
pub struct RendererPageReplacementCommit {
    pub(crate) local_host_id: RendererOwnerLocalHostId,
    pub(crate) page_id: PageId,
    pub(crate) renderer_devtools_agent_token: RendererDevToolsAgentToken,
    pub(crate) javascript_dialog_broker: RendererJavaScriptDialogBroker,
    pub(crate) devtools_target: crate::devtools::target::RendererDevToolsTargetHandle,
    pub(crate) page_state: Arc<RendererPageState>,
    pub(crate) creation_diagnostics: RendererPageCreationDiagnostics,
    pub(crate) creation_artifacts: RendererPageCreationArtifacts,
    pub(crate) pending_download: Option<RendererPendingDownloadActivation>,
}

#[derive(Clone, Debug)]
enum RendererDocumentContinuationState {
    Pending,
    Settled(RendererDocumentContinuationCompletion),
}

/// State captured by the exact owner turn that settles a committed Document's
/// requested continuation target.
///
/// The output predecessor and PageState belong to the same owner turn. This
/// lets protocol adapters first project every lifecycle/Inspector record up to
/// the target and then replace their cached Page view without issuing a later
/// renderer command that could observe a different turn.
#[derive(Clone, Debug)]
pub struct RendererDocumentContinuationCompletion {
    renderer_output_predecessor: Option<RendererOutputFence>,
    page_state: Option<Arc<RendererPageState>>,
}

impl RendererDocumentContinuationCompletion {
    fn settled(
        renderer_output_predecessor: Option<RendererOutputFence>,
        page_state: Option<Arc<RendererPageState>>,
    ) -> Self {
        Self {
            renderer_output_predecessor,
            page_state,
        }
    }

    fn canceled() -> Self {
        Self::settled(None, None)
    }

    pub fn into_parts(self) -> (Option<RendererOutputFence>, Option<Arc<RendererPageState>>) {
        (self.renderer_output_predecessor, self.page_state)
    }
}

/// Exact observer for the owner turn that finishes a DocumentCommit
/// continuation at its requested lifecycle target.
///
/// The lifecycle journal may publish DOMContentLoaded while that owner turn is
/// still committing PageState and settling Inspector output. This observer is
/// resolved only after that concrete output publication has been sent, and
/// carries its fence when one exists.
#[derive(Clone)]
pub struct RendererDocumentContinuationObserver {
    receiver: tokio::sync::watch::Receiver<RendererDocumentContinuationState>,
}

impl std::fmt::Debug for RendererDocumentContinuationObserver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RendererDocumentContinuationObserver")
            .finish_non_exhaustive()
    }
}

impl PartialEq for RendererDocumentContinuationObserver {
    fn eq(&self, other: &Self) -> bool {
        self.receiver.same_channel(&other.receiver)
    }
}

impl RendererDocumentContinuationObserver {
    pub async fn wait(mut self) -> RendererDocumentContinuationCompletion {
        loop {
            match self.receiver.borrow_and_update().clone() {
                RendererDocumentContinuationState::Pending => {}
                RendererDocumentContinuationState::Settled(completion) => return completion,
            }
            if self.receiver.changed().await.is_err() {
                return RendererDocumentContinuationCompletion::canceled();
            }
        }
    }
}

pub(in crate::runtime) struct RendererDocumentContinuationPublisher {
    sender: Option<tokio::sync::watch::Sender<RendererDocumentContinuationState>>,
}

impl RendererDocumentContinuationPublisher {
    fn channel() -> (Self, RendererDocumentContinuationObserver) {
        let (sender, receiver) =
            tokio::sync::watch::channel(RendererDocumentContinuationState::Pending);
        (
            Self {
                sender: Some(sender),
            },
            RendererDocumentContinuationObserver { receiver },
        )
    }

    pub(in crate::runtime) fn settle(
        mut self,
        renderer_output_predecessor: Option<RendererOutputFence>,
        page_state: Option<Arc<RendererPageState>>,
    ) {
        if let Some(sender) = self.sender.take() {
            sender.send_replace(RendererDocumentContinuationState::Settled(
                RendererDocumentContinuationCompletion::settled(
                    renderer_output_predecessor,
                    page_state,
                ),
            ));
        }
    }
}

impl Drop for RendererDocumentContinuationPublisher {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            sender.send_replace(RendererDocumentContinuationState::Settled(
                RendererDocumentContinuationCompletion::canceled(),
            ));
        }
    }
}

pub(in crate::runtime) fn renderer_document_continuation_channel() -> (
    RendererDocumentContinuationPublisher,
    RendererDocumentContinuationObserver,
) {
    RendererDocumentContinuationPublisher::channel()
}

/// Whether a failed prepared replacement left the stable Page's old Document
/// usable or crossed the renderer's irreversible retirement boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererPageReplacementCommitFailureDisposition {
    PagePreserved,
    PageRetired,
}

#[derive(Debug)]
pub struct RendererPageReplacementCommitError {
    disposition: RendererPageReplacementCommitFailureDisposition,
    source: anyhow::Error,
}

impl RendererPageReplacementCommitError {
    #[doc(hidden)]
    pub fn page_preserved(source: anyhow::Error) -> Self {
        Self {
            disposition: RendererPageReplacementCommitFailureDisposition::PagePreserved,
            source,
        }
    }

    #[doc(hidden)]
    pub fn page_retired(source: anyhow::Error) -> Self {
        Self {
            disposition: RendererPageReplacementCommitFailureDisposition::PageRetired,
            source,
        }
    }

    pub fn disposition(&self) -> RendererPageReplacementCommitFailureDisposition {
        self.disposition
    }
}

impl std::fmt::Display for RendererPageReplacementCommitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.source, formatter)
    }
}

impl std::error::Error for RendererPageReplacementCommitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

impl RendererPageReplacementCommit {
    pub fn owner_local_host_id(&self) -> RendererOwnerLocalHostId {
        self.local_host_id
    }

    pub fn page_id(&self) -> PageId {
        self.page_id
    }

    pub fn renderer_devtools_agent_token(&self) -> RendererDevToolsAgentToken {
        self.renderer_devtools_agent_token
    }

    pub fn into_parts(
        self,
    ) -> (
        RendererDevToolsAgentToken,
        Arc<RendererPageState>,
        RendererPageCreationDiagnostics,
        RendererPageCreationArtifacts,
        Option<RendererPendingDownloadActivation>,
    ) {
        (
            self.renderer_devtools_agent_token,
            self.page_state,
            self.creation_diagnostics,
            self.creation_artifacts,
            self.pending_download,
        )
    }
}

#[cfg(test)]
pub(in crate::runtime) enum PageVmNavigationTurnOutcome {
    Completed(Box<PageVm>),
    TriggeredNavigation,
}

pub(in crate::runtime) enum PageVmFollowedNavigationBuildOutcome {
    ContinuePostParseLifecycle {
        page_vm: PageVm,
        page_tasks: Vec<crate::page_task_queue::PostParsePageOwnedWork>,
        stage: PageVmInitStage,
        started: Instant,
    },
    Download(RendererPendingDownloadActivation),
    PendingPhaseOne(PageVmPendingPhaseOneNavigation),
    TriggeredNavigation {
        page_vm: PageVm,
        stage: PageVmInitStage,
    },
}

pub(in crate::runtime) enum PageVmFollowNavigationTurnOutcome {
    Completed,
    PostParseLifecycle {
        target_stage: PageVmInitStage,
        outcome: page_vm::DocumentLifecycleTurnOutcome,
    },
    Download(RendererPendingDownloadActivation),
    TriggeredNavigation {
        stage: PageVmInitStage,
    },
}

pub(in crate::runtime) struct PageVmPendingPhaseOneNavigation {
    pub(super) residence: PendingPhaseOneResidence,
    pub(super) metadata: PageVmFollowedNavigationMetadata,
}

impl PageVmPendingPhaseOneNavigation {
    pub(super) fn new(
        residence: PendingPhaseOneResidence,
        metadata: PageVmFollowedNavigationMetadata,
    ) -> Self {
        Self {
            residence,
            metadata,
        }
    }

    pub(super) fn page_vm(&self) -> &PageVm {
        self.residence.page_vm()
    }

    pub(super) fn page_vm_mut(&mut self) -> &mut PageVm {
        self.residence.page_vm_mut()
    }

    pub(super) fn owner_wake_token(&self) -> Option<RendererPageToken> {
        self.residence.owner_wake_token()
    }

    pub(super) const fn phase_one_restore_requirement(&self) -> PhaseOneRestoreRequirement {
        self.residence.restore_requirement()
    }

    pub(super) fn has_ready_streaming_input(&mut self) -> bool {
        self.residence.has_ready_streaming_input()
    }

    pub(super) fn attach_committed_response(&mut self) {
        self.metadata
            .attach_committed_response(self.residence.page_vm_mut());
    }

    pub(super) fn into_parts(self) -> (PendingPhaseOneResidence, PageVmFollowedNavigationMetadata) {
        (self.residence, self.metadata)
    }
}

#[derive(Default)]
pub(in crate::runtime) struct PageVmFollowedNavigationMetadata {
    pub(super) committed_navigation_response: Option<PageVmNavigationResponse>,
    pub(super) service_worker_client_navigate:
        Option<crate::types::ServiceWorkerClientNavigateContinuation>,
    pub(super) abort_reserved_service_worker_client_id: Option<ServiceWorkerClientId>,
    pub(super) abort_navigation_initiator_url: Option<Url>,
}

impl PageVmFollowedNavigationMetadata {
    fn attach_committed_response(&mut self, page_vm: &mut PageVm) {
        if let Some(response) = self.committed_navigation_response.take() {
            page_vm::attach_navigation_response_to_page_vm(page_vm, response);
        }
    }

    fn complete_service_worker_follow(&mut self, page_vm: &mut PageVm) {
        if let Some(continuation) = self.service_worker_client_navigate.take() {
            page_vm
                .vm_mut()
                .complete_pending_service_worker_client_navigate_after_follow(continuation);
        }
    }

    fn reject(
        &mut self,
        live_page_vm: Option<&mut PageVm>,
        browser_context_runtime: &RendererBrowserContextRuntime,
        message: String,
    ) {
        if let Some(client_id) = self.abort_reserved_service_worker_client_id.take() {
            browser_context_runtime.unregister_service_worker_client(client_id);
        }
        if let Some(page_vm) = live_page_vm
            && let Some(url) = self.abort_navigation_initiator_url.as_ref()
        {
            page_vm
                .vm_mut()
                .restore_top_level_location_runtime_state(url);
        }
        if let Some(continuation) = self.service_worker_client_navigate.take() {
            browser_context_runtime
                .service_worker_runtime()
                .enqueue_client_navigate_completed(
                crate::types::ServiceWorkerClientNavigateCompletion {
                    request_id: continuation.request_id,
                    source_version_id: continuation.source_version_id,
                    source_run: continuation.source_run,
                    result: Err(
                        crate::service_worker_runtime::ServiceWorkerClientNavigateError::type_error(
                            message,
                        ),
                    ),
                },
            );
        }
    }
}

pub(super) enum PageVmNetworkIdleWaitAdvance {
    Completed,
    TriggeredNavigation,
    Progressed {
        state: PageVmNetworkIdleWaitState,
    },
    Waiting {
        sleep_for: std::time::Duration,
        state: PageVmNetworkIdleWaitState,
    },
}

pub(super) enum PageVmSubresourceResponseWaitAdvance {
    Completed,
    TriggeredNavigation,
    Progressed,
    Waiting { sleep_for: std::time::Duration },
}

#[derive(Default)]
pub(super) struct PageVmNetworkIdleWaitState {
    quiet_since: Option<Instant>,
    observed_activity_epoch: Option<u64>,
}

#[derive(Default)]
pub(super) struct PageVmDomStableWaitState {
    last_snapshot: Option<String>,
    stable_since: Option<Instant>,
    saw_post_domcontentloaded_runtime_work: bool,
    saw_long_pending_timeout_for_observation: bool,
}

pub(super) enum PageVmDomStableWaitAdvance {
    Completed,
    TriggeredNavigation,
    Progressed {
        state: PageVmDomStableWaitState,
    },
    Waiting {
        sleep_for: std::time::Duration,
        state: PageVmDomStableWaitState,
    },
}

pub(super) enum PageVmCommandWaitAdvance {
    Completed {
        node: crate::runtime::page_surface::RendererDocumentQuerySelectorNode,
    },
    Progressed,
    Waiting {
        sleep_for: std::time::Duration,
    },
}

pub(super) enum PageVmScriptTruthyWaitAdvance {
    Completed,
    Progressed {
        pending_call: Option<crate::script_vm::PendingRuntimeEvaluateCall>,
    },
    Waiting {
        sleep_for: std::time::Duration,
        pending_call: Option<crate::script_vm::PendingRuntimeEvaluateCall>,
    },
}

pub(super) enum PageVmRuntimeExpressionAwaitAdvance {
    Completed {
        payload: RendererRuntimeEvaluationResult,
    },
    Progressed {
        pending_call: Option<crate::script_vm::PendingRuntimeEvaluateCall>,
    },
    Waiting {
        sleep_for: std::time::Duration,
        pending_call: Option<crate::script_vm::PendingRuntimeEvaluateCall>,
    },
}

#[cfg(test)]
mod tests;
