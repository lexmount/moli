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
mod owner;
mod owner_deadline_index;
mod owner_local;
mod owner_local_store;
mod owner_maintenance;
mod page;
mod page_commands;
mod page_context_cancel;
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
pub(crate) use self::page_vm::AuthorizedCurrentPagePopupLoadEvent;
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
    PopupDocument,
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
    RendererReservedServiceWorkerClient, RendererServiceWorkerMainResourceFetch,
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
    RendererPageCommandPending, RendererPageHandle, RendererRuntimeInspectorSessionDetachGuard,
};
pub(crate) use self::owner_local_store::RendererPageToken;
pub use self::page::{JsRuntime, JsRuntimeOwner, PendingHtmlPage, PreparedRendererDocument};
use self::page::{PageVmNavigationResponse, PageVmStateCapture};
pub(crate) use self::page_context_cancel::{
    RendererPageContextCancelReason, RendererPageContextCancelReceiver,
    RendererPageContextCancelSender, renderer_page_context_cancel_channel,
};
pub use self::page_screenshot::{
    RendererCaptureScreenshotRequest, RendererScreenshotClip, RendererScreenshotFormat,
    RendererScreenshotPurpose, RendererScreenshotRegion,
};
pub(super) use self::page_state::RendererPageEntry;
pub use self::page_state::RendererPageRecord;
pub(crate) use self::page_state::RendererPageSlotHandle;
pub use self::page_state::RendererPageState;
use self::page_surface::RendererPageTable;
pub use self::page_surface::RendererRuntimeInspectorMessageResponseOrder;
pub use self::page_surface::{
    DevToolsSessionKey, RendererAccessibilityPayloadsForObjectId, RendererActivityDiagnostics,
    RendererAgentAttachmentId, RendererAutofillAddressField, RendererAutofillCreditCard,
    RendererAutofillTriggerOutcome, RendererAutofillTriggerRequest, RendererCaptureScreenshotReply,
    RendererCapturedScreenshot, RendererCommandTurnCompletion, RendererCommandTurnOutput,
    RendererCountEntry, RendererDedicatedWorkerTargetEvent, RendererDedicatedWorkerTargetInfo,
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
    RendererInputDispatchOutcome, RendererInspectorCommandEnvelope, RendererInspectorCommandRoute,
    RendererInspectorFirstDispatchLifecycle, RendererInspectorIngressTicket,
    RendererInspectorProtocolConfiguration, RendererInspectorProtocolConfigurationCommand,
    RendererInspectorSessionRestoreSnapshot, RendererJavaScriptDialogId,
    RendererJavaScriptDialogSource, RendererLayoutMetrics, RendererMainDocumentCommit,
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
    RendererPopupActivationSource, RendererResourceTextSearchOutcome, RendererRuntimeCommandOutput,
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
    RendererTouchPoint, RendererWindowDocumentSource, RuntimeConsoleMessageSnapshot,
};
pub(crate) use self::page_surface::{
    RendererCommandTurnOutputRecorder, RendererInspectorPauseCommandEffect,
    RendererRuntimeCommandOutputRecorder, RendererRuntimeInspectorResponsePublication,
    RendererRuntimeObservableSourceQueue,
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
    AuthorizedCurrentPopupClassicScriptLoadCompletion,
    AuthorizedCurrentPopupDocumentLoadCompletion, AuthorizedLiveMainModulepreloadFetchCompletion,
    CurrentChildDocumentLoadApplication,
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
    RendererProtocolObservation, renderer_output_transport_channel,
};
pub use self::service_worker_run::RendererServiceWorkerRunIdentity;
pub(crate) use crate::renderer::PageVmInitStage;
pub use crate::script_vm::inspector_io::{
    RendererRuntimeInspectorIoCommandClaim, RendererRuntimeInspectorIoCommandRoute,
};
pub use crate::script_vm::inspector_main::{
    RendererRuntimeInspectorMainCommandCompletion, RendererRuntimeInspectorMainCommandRoute,
};
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
}

/// Opaque owner-local reservation for one renderer Page identity.
///
/// The reservation is allocated before a queued full-body build or prepared
/// document can enter parser or author-script execution. This lets external
/// observers bind work to the future Page without guessing from the currently
/// installed Page.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RendererPageReservationToken {
    local_host_id: RendererOwnerLocalHostId,
    page_id: PageId,
}

impl RendererPageReservationToken {
    fn new(local_host_id: RendererOwnerLocalHostId, page_id: PageId) -> Self {
        Self {
            local_host_id,
            page_id,
        }
    }

    pub fn local_host_id(self) -> RendererOwnerLocalHostId {
        self.local_host_id
    }

    pub fn page_id(self) -> PageId {
        self.page_id
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
    PendingPhaseOne(PageVmPendingPhaseOneNavigation),
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
    pub(super) followed_navigation_response: Option<(Url, u16, Vec<(String, String)>)>,
    pub(super) service_worker_client_navigate:
        Option<crate::types::ServiceWorkerClientNavigateContinuation>,
    pub(super) abort_reserved_service_worker_client_id: Option<ServiceWorkerClientId>,
    pub(super) abort_navigation_initiator_url: Option<Url>,
}

impl PageVmFollowedNavigationMetadata {
    fn attach_committed_response(&mut self, page_vm: &mut PageVm) {
        if let Some((url, status, headers)) = self.followed_navigation_response.take() {
            page::attach_navigation_response_to_page_vm(page_vm, url, status, headers);
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
                    source_generation: continuation.source_generation,
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
