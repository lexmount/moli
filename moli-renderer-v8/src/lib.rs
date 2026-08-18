//! V8-backed renderer implementation for Moli.
//!
//! This crate owns the JS/runtime machinery behind page execution: renderer
//! owner/page handles, the native bridge, script and module execution, Web API
//! bootstrap, and the renderer-side `ResourceRequestClient` wrapper used by the facade crate.

#[path = "planning.rs"]
mod script_planning;
mod stylesheet_blocking;
mod subresource_integrity;

pub use moli_dom as dom;
pub use moli_page_types as protocol_types;
pub use moli_parser as parser;
pub use moli_selector as selector;

pub use app_manifest::{
    RendererAppManifest, RendererAppManifestDisplayMode, RendererAppManifestError,
    RendererAppManifestImageResource, RendererAppManifestLoadOutcome,
    RendererAppManifestLoadPreparation, RendererAppManifestLoadPublication,
    RendererAppManifestOrientation, RendererAppManifestProtocolHandler,
    RendererAppManifestQueryResult, RendererAppManifestRelatedApplication,
    RendererAppManifestShortcut, RendererPreparedAppManifestLoad,
};

mod abort_signal_route;
mod app_manifest;
mod blob;
mod broadcast_channel_runtime;
mod callback_invocation;
#[cfg(test)]
mod chromium_property_surface;
mod content_security_policy;
mod context_bootstrap;
mod cross_origin_isolation;
mod css_custom_function;
mod css_resource_urls;
mod css_style;
mod custom_elements;
mod definitions;
mod detached_css_style;
mod detached_dom_surface;
mod detached_event_target;
mod devtools;
mod document_cookie_owner;
mod document_language;
mod document_last_modified;
mod document_module_graph;
mod document_runtime;
mod document_script_scheduler;
mod document_task_lane;
mod dom_parser;
mod dynamic_script_owner;
mod exception_reporting;
mod frame_owner_model;
mod host;
mod host_bindings;
mod inspector_microtasks;
mod layout_renderer;
mod link_as;
mod live_document_parser;
mod live_stylesheet;
pub(crate) mod local_executor;
mod message_port_runtime;
mod module_runtime;
mod module_script_continuation;
mod modulepreload;
mod mutation_coordinator;
pub(crate) mod native_bridge;
pub mod network;
mod network_host;
mod observer_runtime;
mod opfs_owner_tasks;
mod opfs_task_result;
mod page_resource_completion;
mod page_task_queue;
mod parser_module_evaluation;
mod parser_module_pending;
mod parser_script;
mod queue_microtask;
mod range_boundary;
mod referrer_policy;
pub(crate) mod reflector;
mod render_runtime;
mod renderer_resource_scheduler;
mod resource_owner;
mod resource_ready;
mod runtime;
mod runtime_binding_data;
mod script_execution_control;
mod script_provenance;
mod script_vm;
mod service_worker_runtime;
mod shared_worker_runtime;
mod structured_clone;
mod style_engine;
mod text_codec;
mod tokio_blocking_budget;
pub(crate) mod types;
mod util;
mod v8_execution_watchdog;
mod v8_finalizer;
pub(crate) mod v8_platform;
mod v8_traced_webidl_callback;
mod wasm_module_support;
mod web_storage_handles;
pub(crate) mod webidl;
#[cfg(test)]
mod webidl_callback_source_boundary_tests;
mod webidl_iterator;
mod window_document_identity;
mod window_host;
mod window_webidl_callback;
pub(crate) mod worker;
mod xml_serializer;

pub(crate) mod planning {
    pub(crate) use crate::script_planning::*;
}

#[cfg(test)]
pub(crate) fn ensure_v8_for_test() {
    moli_v8_test_util::ensure_v8_with_flags_and_platform(
        Some(v8_platform::initialization_flags()),
        v8_platform::create_platform,
    );
}

/// Real-layout policy for renderer fixtures whose assertions observe used
/// geometry or paint. Production defaults are configured outside test helpers.
#[cfg(test)]
pub(crate) const fn real_layout_test_policy() -> moli_page_types::LayoutPolicy {
    moli_page_types::LayoutPolicy::OnDemand
}

/// Builds an owned HTML fixture through the same incremental parser frontier
/// used by executable Documents while retaining parser-time discovery output
/// needed by unit tests. This is deliberately test-only: production code must
/// keep the live parser session and deliver each handoff at its source boundary.
#[cfg(test)]
pub(crate) fn parse_html_test_fixture_with_parser_outputs(
    final_url: url::Url,
    html: String,
) -> (
    dom::native::NativeDom,
    Vec<parser::ParserScriptHandoff>,
    Vec<DocumentOwnedBlockingStylesheetDiscoveryInput>,
) {
    let mut stream = parser::HtmlParser.start_document(final_url);
    stream.append_to_end(html);
    let mut scripts = Vec::new();
    let mut blocking_stylesheets = Vec::new();
    while stream.has_pending_input() {
        let outcome = stream.pump_next_parser_step(0);
        blocking_stylesheets.extend(outcome.discovered_blocking_stylesheet_inputs);
        if let parser::ParserPumpStep::Yield(parser::ParserYield::Script(script)) = outcome.result {
            scripts.push(*script);
        }
    }
    (stream.finish(), scripts, blocking_stylesheets)
}

#[allow(unused_imports)]
pub(crate) use crate::script_planning::{
    ParserPlanningReadView, ParserScriptRead, PrepareScriptOutcome, PreparedScript,
    build_prepared_script, classify_parser_script,
};
#[allow(unused_imports)]
pub(crate) use crate::stylesheet_blocking::{
    DocumentBlockingStylesheetSignature, DocumentOwnedBlockingStylesheetDiscoveryInput,
    StylesheetBlockingReadView, StylesheetElementRead,
    collect_document_owned_blocking_stylesheet_discovery_inputs_before_in_view,
    collect_document_owned_blocking_stylesheets_before_in_view,
};

pub use context_bootstrap::{
    DEFAULT_ORIGIN_STORAGE_QUOTA_BYTES, IndexedDbKey, IndexedDbObjectStoreOptions,
    IndexedDbOpenOptions, IndexedDbTransactionMode, SharedIndexedDbManager, WeakIndexedDbManager,
    clear_indexed_db_origin, clear_indexed_db_origins_with_prefix, downgrade_indexed_db_manager,
    indexed_db_origin_usage_bytes, indexed_db_origins_with_prefix_usage_bytes,
    new_indexed_db_manager,
};
pub use context_bootstrap::{
    SharedStorageBucketStore, StorageBucketIdentity, new_shared_json_storage_bucket_store,
    new_shared_json_storage_bucket_store_with_cache_root,
    new_shared_json_storage_bucket_store_with_cache_root_and_indexed_db_manager,
    new_shared_json_storage_bucket_store_with_storage_service, new_shared_storage_bucket_store,
    new_shared_storage_bucket_store_with_indexed_db_manager,
    new_shared_storage_bucket_store_with_storage_service,
    new_shared_storage_bucket_store_with_storage_service_and_indexed_db_manager,
    storage_bucket_indexed_db_storage_key,
};
pub use context_bootstrap::{
    SharedWebStorageStore, WebStorageAreaKind, WebStorageMutation, WebStorageMutationRecord,
    WebStorageMutationSubscription, WebStorageString, deep_clone_shared_web_storage_store,
    new_shared_json_web_storage_store, new_shared_web_storage_store,
    web_storage_partitioned_area_key,
};
pub(crate) use context_bootstrap::{
    construct_form_data_entries_for_form, form_data_entries_multipart_body_with_prefix,
    form_data_entries_to_string_pairs, form_data_object_from_entries, snapshot_form_data_value,
};
pub use document_cookie_owner::{
    DocumentCookieBackendConnectionState, DocumentCookieBrowserContextSnapshot,
    DocumentCookieGetFreshnessStatus, DocumentCookieOwnerSnapshot,
    DocumentCookieSetReadinessStatus, DocumentCookieWriteCapabilitySnapshot,
};
pub use host::{
    DocumentCookieCacheLookupResult, DocumentCookieCacheSnapshot, DocumentCookieCacheStatus,
    DocumentCookieCapabilitySnapshot, DocumentCookieFacadeTelemetrySnapshot,
    DocumentCookieFirstOperation,
};
pub use local_executor::is_on_js_local_executor;
pub use native_bridge::element::ClientRect as RendererClientRect;
pub use reflector::ReflectorRegistry;
pub use runtime::RendererRuntimeInspectorMessageResponseOrder;
pub use runtime::{
    DetachedParserScriptFetchContinuation, DevToolsSessionKey, ExternalRawDocumentBodyStream,
    JsRuntime, JsRuntimeOwner, PageId, PendingHtmlPage, PreparedRendererDocument,
    RendererAccessibilityPayloadsForObjectId, RendererActivityDiagnostics,
    RendererAgentAttachmentId, RendererAutofillAddressField, RendererAutofillCreditCard,
    RendererAutofillTriggerOutcome, RendererAutofillTriggerRequest, RendererBrowserContextRuntime,
    RendererBrowserContextRuntimeId, RendererBrowserContextRuntimeOwner,
    RendererBrowserContextRuntimeOwnerAccess, RendererCaptureScreenshotReply,
    RendererCaptureScreenshotRequest, RendererCapturedScreenshot, RendererCommandTurnCompletion,
    RendererCommandTurnOutput, RendererCountEntry, RendererDedicatedWorkerTargetEvent,
    RendererDedicatedWorkerTargetInfo, RendererDevToolsAgentToken,
    RendererDevToolsMainCommandEnvelope, RendererDocumentBoxModel,
    RendererDocumentChildNodeSnapshotEvent, RendererDocumentChildNodeSnapshotEvents,
    RendererDocumentChildNodeSnapshots, RendererDocumentCommitPermit,
    RendererDocumentFrontendNodeIdsResolution, RendererDocumentHitTestResult,
    RendererDocumentIsolateAccountingDiagnostics, RendererDocumentLifecycleEvent,
    RendererDocumentLifecycleEventKind, RendererDocumentLifecycleIdentity,
    RendererDocumentLifecycleMilestone, RendererDocumentLifecycleSnapshot,
    RendererDocumentLifecycleWaitOutcome, RendererDocumentLifecycleWaiter,
    RendererDocumentNodeAttributesResolution, RendererDocumentNodeClientRect,
    RendererDocumentNodeGeometry, RendererDocumentNodePropertyResolution,
    RendererDocumentNodeReference, RendererDocumentNodeTextResolution,
    RendererDocumentQuerySelectorNode, RendererDocumentQuerySelectorResolution,
    RendererDocumentQuerySelectorWithChildNodeSnapshotEvents,
    RendererDocumentSourcedSameDocumentNavigation,
    RendererDocumentSourcedTopLevelLocationNavigation, RendererDocumentTerminationReason,
    RendererDocumentTitleChanged, RendererDocumentToken, RendererDomAttributeMutation,
    RendererDomAttributeMutationOutcome, RendererDomBidiNodeBindingResolution,
    RendererDomBidiNodeSharedIdResolution, RendererDomDebuggerDomBreakpointResolution,
    RendererDomDebuggerEventListener, RendererDomDebuggerEventListenerBreakpoint,
    RendererDomDebuggerEventListenersResolution, RendererDomDebuggerXhrBreakpoint, RendererDomEdit,
    RendererDomEditOutcome, RendererDomFocusOutcome, RendererDomFrontendNodeBindingResolution,
    RendererDomMutationEvent, RendererDomMutationEventBatch, RendererDomNodeCreationStackFrame,
    RendererDomNodeCreationStackTrace, RendererDomNodeStackTraceResolution,
    RendererDomSearchRegistration, RendererDomSearchResultNode, RendererDomSearchResultsResolution,
    RendererDomSnapshotCaptureOptions, RendererDomSnapshotCapturePayload, RendererDragData,
    RendererDragDataItem, RendererDraggedDirectory, RendererDraggedFile, RendererFrameToken,
    RendererGeometryQuad, RendererInputDispatchOutcome, RendererInspectorCommandEnvelope,
    RendererInspectorCommandRoute, RendererInspectorFirstDispatchLifecycle,
    RendererInspectorIngressTicket, RendererInspectorProtocolConfiguration,
    RendererInspectorProtocolConfigurationCommand, RendererInspectorSessionRestoreSnapshot,
    RendererJavaScriptDialogCompletion, RendererJavaScriptDialogId, RendererJavaScriptDialogResult,
    RendererJavaScriptDialogSource, RendererLayoutMetrics, RendererLifecycleDecider,
    RendererLifecycleDecision, RendererLifecycleEpoch, RendererLifecycleEventStamp,
    RendererLifecycleSnapshot, RendererLifecycleStartReason, RendererLifecycleTerminationStamp,
    RendererMainDocumentCommit, RendererMoliDomMemoryDiagnostics, RendererMoliMemoryDiagnostics,
    RendererMoliMemoryScopeDiagnostics, RendererMoliRuntimeMemoryDiagnostics, RendererOutputCursor,
    RendererOutputFence, RendererOutputFenceLeaseId, RendererOutputItem, RendererOutputPublication,
    RendererOutputPublicationOrdering, RendererOutputRecord, RendererOutputResidenceIdentity,
    RendererOutputStreamCloseReason, RendererOutputStreamControl, RendererOutputStreamEpoch,
    RendererOutputStreamIdentity, RendererOutputTransportDiagnostics,
    RendererOutputTransportMessage, RendererOutputTransportReceiver,
    RendererOutputTransportSendError, RendererOutputTransportSender, RendererOwnerAction,
    RendererOwnerCommand, RendererOwnerHandle, RendererOwnerLocalHostId, RendererOwnerReply,
    RendererOwnerResourceActivitySource, RendererOwnerRuntimeActivitySource, RendererPageCommand,
    RendererPageCommandPending, RendererPageCommandPostResponseContinuation,
    RendererPageCookieFacadeSnapshotReply, RendererPageCreationArtifacts,
    RendererPageCreationDiagnostics, RendererPageDiagnosticsSnapshot, RendererPageDumpFormat,
    RendererPageDumpOptions, RendererPageDumpStripOptions, RendererPageHandle, RendererPageReply,
    RendererPageReservationToken, RendererPageState, RendererPageTestingHandle, RendererPageView,
    RendererPendingDownloadActivation, RendererPendingDownloadResponse,
    RendererPendingFileChooserActivation, RendererPendingJavaScriptDialog,
    RendererPendingPopupActivation, RendererPendingSameDocumentNavigation,
    RendererPendingTopLevelHistoryTraversal, RendererPendingWindowOpenEvent,
    RendererPerformanceMetricSnapshot, RendererPointerEventProperties,
    RendererPopupActivationSource, RendererPreparedDocumentCommitConfiguration,
    RendererProtocolObservation, RendererReservedServiceWorkerClient,
    RendererResourceTextSearchOutcome, RendererRuntimeCommandCausalIdentity,
    RendererRuntimeCommandOutput, RendererRuntimeEvaluationResult, RendererRuntimeHeapSpaceUsage,
    RendererRuntimeHeapUsage, RendererRuntimeInspectorAsyncCompletion,
    RendererRuntimeInspectorIoCommandClaim, RendererRuntimeInspectorIoCommandRoute,
    RendererRuntimeInspectorMainCommandCompletion, RendererRuntimeInspectorMainCommandRoute,
    RendererRuntimeInspectorMessage, RendererRuntimeInspectorMessageBatch,
    RendererRuntimeInspectorProtocolMessage, RendererRuntimeInspectorProtocolMessageValueMut,
    RendererRuntimeInspectorResponseChannel, RendererRuntimeInspectorResponseSender,
    RendererRuntimeObservableSourceItem, RendererRuntimeObservableSourceSummary,
    RendererRuntimeRealmInfo, RendererRuntimeRemoteObject, RendererRuntimeRemoteObjectResolution,
    RendererScreenshotClip, RendererScreenshotFormat, RendererScreenshotPurpose,
    RendererScreenshotRegion, RendererScriptExecutionMemoryDiagnostics,
    RendererScriptSourceMemoryDiagnostics, RendererScrollIntoViewResult,
    RendererServiceWorkerConsoleMessage, RendererServiceWorkerExceptionMessage,
    RendererServiceWorkerFetchDiagnostic, RendererServiceWorkerFetchDiagnosticResult,
    RendererServiceWorkerMainResourceFetch, RendererServiceWorkerRunIdentity,
    RendererServiceWorkerTargetEvent, RendererServiceWorkerTargetInfo,
    RendererServiceWorkerVersionStatus, RendererSetDocumentContentResult,
    RendererSharedWorkerConsoleMessage, RendererSharedWorkerTargetEvent,
    RendererSharedWorkerTargetInfo, RendererStyleSheetHeader, RendererStyleSheetInventoryUpdate,
    RendererStyleSheetPayload, RendererSyntheticResponseBody, RendererTextSearchMatch,
    RendererTouchPoint, RendererWindowDocumentSource, RuntimeConsoleMessageSnapshot,
    renderer_output_transport_channel,
};
pub use service_worker_runtime::{
    SharedServiceWorkerResourceStore, new_shared_json_service_worker_resource_store,
    new_shared_service_worker_resource_store,
};
pub use shared_worker_runtime::RendererSharedWorkerRuntimeDiagnostics;
pub(crate) use types::{
    DocumentStartScript, PendingSubresourceContinueOutcome, SubresourceAuthCredentials,
    SubresourceResourceType,
};
#[allow(unused_imports)]
pub use types::{ScriptKind, ScriptMode, ScriptRunOutcome, ScriptSourceKind};
pub use web_storage_handles::RendererWebStorageHandles;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageVmInitStage {
    DomContentLoaded,
    Load,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererReplyBoundary {
    /// Resolve page creation only after the requested lifecycle milestone.
    Stage,
    /// Publish the committed Document and continue DCL/load on owner turns.
    DocumentCommit,
}

impl RendererReplyBoundary {
    pub(crate) const fn waits_for_stage(self) -> bool {
        matches!(self, Self::Stage)
    }
}

/// Host integration route for cross-document top-level navigation requests.
///
/// This is not navigation ownership and is not Document lifecycle state. The
/// renderer always produces the request from the exact source Document. A
/// browser-managed page delegates that request to the browser navigation
/// controller; the standalone route is a compatibility adapter for renderer
/// APIs that do not yet have a browser controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererTopLevelNavigationDispatch {
    DelegateToBrowser,
    FollowInStandaloneAdapter,
}

/// One-shot handling for a top-level navigation request observed while the
/// initial page-creation command is still waiting to reply.
///
/// Unlike [`RendererTopLevelNavigationDispatch`], this policy belongs only to
/// that command observer. It is discarded when the command replies or
/// detaches and never becomes stable Page or Document lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererNavigationReplyPolicy {
    FollowBeforeReply,
    ReturnWithPendingNavigation,
}

impl RendererNavigationReplyPolicy {
    pub(crate) const fn returns_with_pending_navigation(self) -> bool {
        matches!(self, Self::ReturnWithPendingNavigation)
    }
}

pub mod renderer {
    pub use super::{
        PageVmInitStage, RendererNavigationReplyPolicy, RendererReplyBoundary,
        RendererTopLevelNavigationDispatch, is_on_js_local_executor,
    };
}
