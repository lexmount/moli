macro_rules! expect_page_reply {
    ($reply:expr, $operation:expr, $expected:expr, $pattern:pat => $body:expr $(,)?) => {{
        match $reply {
            $pattern => $body,
            other => Page::unexpected_page_reply($operation, $expected, other),
        }
    }};
}

mod accessibility_support;
mod app_manifest_support;
mod child_frame_navigation_support;
mod client_geometry;
mod command_dispatch;
#[cfg(feature = "devtools")]
mod devtools;
mod document_content_support;
mod document_cookie_diagnostics;
mod dom_protocol_support;
mod history_support;
mod js_evaluation;
mod navigation_diagnostics;
mod page_state_cache;
mod protocol_support;
mod renderer_command_support;
mod same_document_navigation_support;
mod settings_support;
mod subresource_support;
mod testing_support;
mod wait_support;

use std::fmt;
use std::sync::Arc;

use crate::renderer::{
    RendererPageCommand, RendererPageCommandPending, RendererPageHandle, RendererPageReply,
    RendererPageState,
};
use anyhow::Result;
pub use command_dispatch::{CompletedPageCommand, PageDocumentCommandTarget, PendingPageCommand};
#[cfg(feature = "devtools")]
pub use devtools::{
    CompletedDevToolsIoCommandDispatch, CompletedRuntimeInspectorCommandDispatch, PageInputExt,
    PendingDevToolsIoCommandDispatch, PendingRuntimeInspectorCommandDispatch, PreparedNativeCall,
    decode_element_click_dispatch_completion, decode_element_click_preparation_completion,
    decode_input_dispatch_outcome_completion, decode_insert_text_completion,
};
pub use moli_renderer_v8::RendererPageRequestPolicyUpdate;

#[cfg(all(test, debug_assertions))]
use crate::network::ResourceRequestClient;
use page_state_cache::PageStateCache;
#[cfg(all(test, debug_assertions))]
use std::time::Duration;

pub use client_geometry::ClientRect;
pub use document_cookie_diagnostics::{
    DocumentCookieBackendConnectionState, DocumentCookieBrowserContextSnapshot,
    DocumentCookieCacheLookupResult, DocumentCookieCacheSnapshot, DocumentCookieCacheStatus,
    DocumentCookieCapabilitySnapshot, DocumentCookieFacadeTelemetrySnapshot,
    DocumentCookieFirstOperation, DocumentCookieGetFreshnessStatus, DocumentCookieOwnerSnapshot,
    DocumentCookieSetReadinessStatus, DocumentCookieWriteCapabilitySnapshot,
};
pub use dom_protocol_support::{
    DocumentNodeAttributeSnapshot, DocumentNodeObjectSnapshot, DocumentNodeSnapshot,
    MAX_DOM_OUTPUT_TREE_DEPTH, MAX_INSPECTOR_PROTOCOL_VALUE_DEPTH, MAX_JSON_OUTPUT_TREE_DEPTH,
    RENDERER_BACKEND_NODE_ID_START, is_renderer_backend_node_id,
};
pub use moli_dom::accessibility::{AccessibilityNode, AccessibilityNodeId, AccessibilityValue};
pub use moli_page_types::DomScrollIntoViewRect;
pub use moli_page_types::{
    RendererDomDebuggerDomBreakpointType, RendererDomDebuggerEventListenerBreakpoint,
    RendererDomDebuggerXhrBreakpoint, RendererInspectorProtocolConfiguration,
    RendererInspectorProtocolConfigurationCommand, RendererInspectorSessionRestoreSnapshot,
    SessionHistoryUpdateKind, V8InspectorSessionAttach, V8InspectorSessionState,
    renderer_inspector_protocol_configuration_command_from_message,
    renderer_inspector_protocol_configuration_command_from_method,
};
pub use moli_renderer_v8::dom::native::SelectedFile;
pub use moli_renderer_v8::network::{
    RendererNetworkResourceLoadOutcome, RendererNetworkResourceLoadPreparation,
    RendererNetworkResourceLoadResponse, RendererPreparedNetworkResourceLoad,
};
pub use moli_renderer_v8::{
    CompletedWorkerRuntimeInspectorCommandDispatch, DevToolsSessionKey,
    PendingWorkerRuntimeInspectorSessionResponse, RendererActivityDiagnostics,
    RendererAgentAttachmentId, RendererAutofillAddressField, RendererAutofillCreditCard,
    RendererAutofillTriggerOutcome, RendererAutofillTriggerRequest,
    RendererAuxiliaryDocumentResponse, RendererAuxiliaryNavigationKind, RendererAuxiliaryWindow,
    RendererBrowsingContextGroup, RendererBrowsingContextName, RendererCaptureScreencastFrameReply,
    RendererCaptureScreencastFrameRequest, RendererCaptureScreenshotReply,
    RendererCaptureScreenshotRequest, RendererCapturedDocumentEnvironment,
    RendererCommandTurnCompletion, RendererCommandTurnOutput, RendererContentStripOptions,
    RendererDedicatedWorkerTargetEvent, RendererDedicatedWorkerTargetInfo,
    RendererDevToolsAgentToken, RendererDocumentHitTestResult,
    RendererDocumentIsolateAccountingDiagnostics, RendererDocumentLifecycleEvent,
    RendererDocumentLifecycleEventKind, RendererDocumentLifecycleIdentity,
    RendererDocumentLifecycleMilestone, RendererDocumentLifecycleSnapshot,
    RendererDocumentLifecycleWaitOutcome, RendererDocumentLifecycleWaiter,
    RendererDocumentNodeGeometry, RendererDocumentSourcedSameDocumentNavigation,
    RendererDocumentSourcedTopLevelLocationNavigation, RendererDocumentTerminationReason,
    RendererDocumentToken, RendererDomAttributeMutation, RendererDomAttributeMutationOutcome,
    RendererDomDebuggerDomBreakpointResolution, RendererDomDebuggerEventListener,
    RendererDomDebuggerEventListenersResolution, RendererDomEdit, RendererDomEditOutcome,
    RendererDomFocusOutcome, RendererDomMutationEvent, RendererDomMutationEventBatch,
    RendererDomSnapshotCaptureOptions, RendererDomSnapshotCapturePayload, RendererDownloadBody,
    RendererDragData, RendererDragDataItem, RendererDraggedDirectory, RendererDraggedFile,
    RendererFrameToken, RendererHtmlSerializeOptions, RendererInputDispatchOutcome,
    RendererInterceptedDrag, RendererJavaScriptDialogCompletion, RendererJavaScriptDialogId,
    RendererJavaScriptDialogResult, RendererJavaScriptDialogSource, RendererLayoutMetrics,
    RendererLifecycleEpoch, RendererLifecycleEventStamp, RendererLifecycleStartReason,
    RendererLifecycleTerminationStamp, RendererMainDocumentCommit, RendererMainDocumentResource,
    RendererMarkdownOptions, RendererNavigationInitiator,
    RendererPageCommandPostResponseContinuation, RendererPageCreationArtifacts,
    RendererPageCreationDiagnostics, RendererPageDiagnosticsSnapshot, RendererPageReplacementError,
    RendererPageReplacementFailureDisposition, RendererPageWindowIdentity, RendererPdfOptions,
    RendererPendingAuxiliaryPage, RendererPendingDownloadActivation,
    RendererPendingDownloadResponse, RendererPendingFileChooserActivation,
    RendererPendingJavaScriptDialog, RendererPendingPopupActivation,
    RendererPendingSameDocumentNavigation, RendererPendingTopLevelHistoryTraversal,
    RendererPendingWindowOpenEvent, RendererPerformanceMetricSnapshot,
    RendererPointerEventProperties, RendererPopupActivationSource, RendererPopupDisposition,
    RendererPrintToPdfReply, RendererPrintToPdfRequest, RendererResourceContentBody,
    RendererResourceSearchRequest, RendererResourceTextSearchOutcome, RendererRuntimeCommandOutput,
    RendererRuntimeHeapUsage, RendererRuntimeInspectorMessage,
    RendererRuntimeInspectorMessageBatch, RendererRuntimeInspectorProtocolMessage,
    RendererRuntimeObservableSourceItem, RendererRuntimeObservableSourceSummary,
    RendererRuntimeRealmInfo, RendererScreenshotClip, RendererScreenshotFormat,
    RendererScreenshotPurpose, RendererScreenshotRegion, RendererScrollIntoViewResult,
    RendererServiceWorkerConsoleMessage, RendererServiceWorkerExceptionMessage,
    RendererServiceWorkerFetchDiagnostic, RendererServiceWorkerFetchDiagnosticResult,
    RendererServiceWorkerRunIdentity, RendererServiceWorkerTargetEvent,
    RendererServiceWorkerTargetInfo, RendererServiceWorkerVersionStatus,
    RendererSetDocumentContentResult, RendererSharedWorkerConsoleMessage,
    RendererSharedWorkerTargetEvent, RendererSharedWorkerTargetInfo, RendererSyntheticResponseBody,
    RendererTextSearchMatch, RendererTouchPoint, RendererVisualStateToken,
    RendererWindowDocumentSource, RuntimeConsoleMessageSnapshot, WorkerInspectorError,
};
pub use moli_renderer_v8::{
    RendererAppManifest, RendererAppManifestDisplayMode, RendererAppManifestError,
    RendererAppManifestImageResource, RendererAppManifestLoadOutcome,
    RendererAppManifestLoadPreparation, RendererAppManifestLoadPublication,
    RendererAppManifestOrientation, RendererAppManifestProtocolHandler,
    RendererAppManifestQueryResult, RendererAppManifestRelatedApplication,
    RendererAppManifestShortcut, RendererPreparedAppManifestLoad,
};
pub use moli_renderer_v8::{
    RendererElementClickError, RendererElementClickTarget, RendererPreparedPointerClick,
};
#[cfg(feature = "devtools")]
pub use moli_renderer_v8::{
    RendererInspectorCommandRoute, RendererRuntimeInspectorIoCommandClaim,
    RendererRuntimeInspectorIoCommandRoute, RendererRuntimeInspectorMainCommandCompletion,
    RendererRuntimeInspectorMainCommandRoute,
};
pub use moli_renderer_v8::{
    RendererPopupActivationParts, RendererRuntimeInspectorMessageResponseOrder,
};
pub use navigation_diagnostics::{NavigationRedirect, NavigationResponse};
pub use protocol_support::{
    BidiPreloadChannelHandoff, ChildDocumentFetchRequest, ChildFrameAttachmentSnapshot,
    ChildFrameDetachmentSnapshot, ChildFrameDocumentNetworkActivitySnapshot,
    ChildFrameDocumentNetworkSnapshot, ChildFrameDocumentOpenedSnapshot,
    ChildFrameNavigationSnapshot, ChildFrameTreeEventSnapshot, ChildFrameTreeSnapshot,
    ContentSecurityPolicyIssueSnapshot, ContentSecurityPolicyViolationType, DocumentActivity,
    DocumentStartScript, EmulatedIdleOverride, EmulatedMediaOverrides, InspectorIssueSnapshot,
    InspectorSourceCodeLocationSnapshot, PendingRuntimeBindingCall, PendingSubresourceAuthInfo,
    PendingSubresourceContinueEvent, PendingSubresourceContinueOutcome,
    PendingSubresourceFetchInfo, PendingSubresourceResponseInfo, PermissionOverrideRegistration,
    QuirksModeIssueSnapshot, RuntimeBindingCallSourceIdentity, RuntimeBindingRegistration,
    RuntimeContextRestoreEvent, RuntimeExecutionContextRestoreEvent,
    RuntimeExecutionContextsClearedRestoreEvent, RuntimeIsolatedWorldDefinition,
    ScriptExecutionReport, ScriptNetworkOutput, ScriptNetworkOutputItem, ScriptObservableOutput,
    ScriptObservableOutputItem, SubresourceAuthChallenge, SubresourceAuthCredentials,
    SubresourceAuthScheme, SubresourceAuthTarget, SubresourceBodyFinished,
    SubresourceBodyFinishedResult, SubresourceDataReceived, SubresourceEventSourceMessageReceived,
    SubresourceInterceptionClasses, SubresourceJsonPathEquals, SubresourceJsonPathRegex,
    SubresourceNetworkOutcome, SubresourceNetworkRecord, SubresourceNetworkRequestHandle,
    SubresourceRequestInitiatorType, SubresourceRequestSnapshot, SubresourceResourceType,
    SubresourceResponseBody, SubresourceResponseStarted, SubresourceResponseWaitCriteria,
    ViewportSurface, WebSocketFrameDirection, WebSocketFrameOpcode, WebSocketLifecycleEvent,
    WebSocketLifecycleKind, WebSocketNetworkEvent, extract_subresource_auth_challenge,
    subresource_auth_credentials_for_challenge,
};
pub use renderer_command_support::DocumentNodeClientRectResolution;
pub use renderer_command_support::{
    DocumentNodeRuntimeObjectResolution, PageObservableOutputUpdate, SerializedHtmlDocument,
    TestingOutcome,
};

#[cfg(test)]
use crate::renderer::RendererPageTestingHandle;

pub use crate::renderer::{
    RendererDocumentChildNodeSnapshotEvents, RendererDocumentFrontendNodeIdsResolution,
    RendererDocumentNodeAttributesResolution, RendererDocumentNodePropertyResolution,
    RendererDocumentNodeReference, RendererDocumentNodeTextResolution,
    RendererDocumentQuerySelectorNode, RendererDocumentQuerySelectorResolution,
    RendererDocumentQuerySelectorWithChildNodeSnapshotEvents, RendererDomBidiNodeBindingResolution,
    RendererDomBidiNodeSharedIdResolution, RendererDomFrontendNodeBindingResolution,
    RendererDomNodeCreationStackFrame, RendererDomNodeCreationStackTrace, RendererDomNodeReference,
    RendererDomNodeStackTraceResolution, RendererDomSearchRegistration,
    RendererDomSearchResultNode, RendererDomSearchResultsResolution, RendererRuntimeRemoteObject,
    RendererStyleSheetHeader, RendererStyleSheetInventoryUpdate, RendererStyleSheetPayload,
};

pub struct Page {
    page_state: PageStateCache,
    requested_document_activity: std::cell::Cell<moli_page_types::DocumentActivity>,
    // Chromium owns this on RenderFrameHostImpl::IdleManager. Keep it on the
    // current document handle instead of deriving browser-side navigation
    // policy from whichever renderer snapshot the protocol actor consumed
    // most recently.
    idle_override: Option<EmulatedIdleOverride>,
    handle: RendererPageHandle,
    renderer_agent_attachment_id: Option<RendererAgentAttachmentId>,
    renderer_devtools_command_session_id: Option<String>,
    page_creation_artifacts: Option<Box<RendererPageCreationArtifacts>>,
}

impl fmt::Debug for Page {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Page")
            .field("page_id", &self.handle.page_id())
            .field("requested_url", &self.page_state.requested_url())
            .field("status", &self.page_state.status())
            .field("headers", &self.page_state.headers())
            .field("script_execution", &self.page_state.script_execution())
            .finish_non_exhaustive()
    }
}

impl Page {
    pub(crate) fn adopt_renderer_document_replacement(
        &mut self,
        replacement: moli_renderer_v8::RendererPageReplacementCommit,
    ) -> Result<(
        RendererPageCreationDiagnostics,
        RendererPageCreationArtifacts,
    )> {
        let (state, diagnostics, artifacts) =
            self.handle.adopt_document_replacement(replacement)?;
        self.idle_override = state.idle_override();
        self.requested_document_activity
            .set(state.document_activity);
        self.page_state.replace(state);
        self.page_creation_artifacts = None;
        Ok((diagnostics, artifacts))
    }

    pub(crate) fn from_attached_handle(
        handle: RendererPageHandle,
        page_state: Arc<RendererPageState>,
    ) -> Self {
        let idle_override = page_state.idle_override();
        Self {
            requested_document_activity: std::cell::Cell::new(page_state.document_activity),
            page_state: PageStateCache::new(page_state),
            idle_override,
            handle,
            renderer_agent_attachment_id: None,
            renderer_devtools_command_session_id: None,
            page_creation_artifacts: None,
        }
    }

    pub(crate) fn from_attached_handle_with_creation_artifacts(
        handle: RendererPageHandle,
        page_state: Arc<RendererPageState>,
        page_creation_artifacts: RendererPageCreationArtifacts,
    ) -> Self {
        let idle_override = page_state.idle_override();
        Self {
            requested_document_activity: std::cell::Cell::new(page_state.document_activity),
            page_state: PageStateCache::new(page_state),
            idle_override,
            handle,
            renderer_agent_attachment_id: None,
            renderer_devtools_command_session_id: None,
            page_creation_artifacts: Some(Box::new(page_creation_artifacts)),
        }
    }

    /// Takes the renderer lifecycle facts captured while this page was created.
    ///
    /// Direct-fetch callers can retain these facts until the page is handed to
    /// a protocol owner instead of losing the initial lifecycle journal batch.
    pub fn take_page_creation_artifacts(&mut self) -> Option<RendererPageCreationArtifacts> {
        self.page_creation_artifacts
            .take()
            .map(|artifacts| *artifacts)
    }

    #[doc(hidden)]
    pub fn renderer_devtools_agent_token(&self) -> RendererDevToolsAgentToken {
        self.handle.devtools_agent_token()
    }

    #[doc(hidden)]
    pub fn set_renderer_devtools_command_session_id(&mut self, session_id: Option<String>) {
        self.renderer_devtools_command_session_id = session_id;
    }

    #[doc(hidden)]
    pub fn renderer_agent_attachment_id(&self) -> Option<RendererAgentAttachmentId> {
        self.renderer_agent_attachment_id
    }

    #[doc(hidden)]
    pub fn bind_renderer_agent_attachment(&mut self, id: RendererAgentAttachmentId) {
        self.renderer_agent_attachment_id = Some(id);
    }

    #[doc(hidden)]
    pub fn take_committed_document_post_response_continuation(
        &mut self,
    ) -> Option<RendererPageCommandPostResponseContinuation> {
        self.handle
            .take_committed_document_post_response_continuation()
    }

    /// Deterministically releases this page from the renderer owner.
    ///
    /// Dropping `Page` also schedules a detached best-effort cleanup, but code
    /// that needs teardown acknowledgement should await `close_async()` instead.
    pub async fn close_async(mut self) -> Result<()> {
        self.handle.close_async().await
    }

    // Synchronous Page methods below must stay local: they read the cached
    // snapshot or stable identifiers only. Anything that talks to the renderer
    // owner goes through an explicit async method.
    #[cfg(test)]
    pub(crate) fn handle_for_testing(&self) -> RendererPageTestingHandle {
        RendererPageTestingHandle::new_for_testing(&self.handle)
    }

    #[cfg(all(test, debug_assertions))]
    pub(crate) async fn panic_renderer_command_for_testing(&mut self) -> Result<()> {
        self.dispatch_unit_page_command_async(
            RendererPageCommand::PanicForTesting,
            "panic renderer command for testing",
        )
        .await
    }

    #[cfg(all(test, debug_assertions))]
    pub(crate) async fn panic_wait_for_script_truthy_for_testing(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> Result<()> {
        self.wait_for_script_truthy(
            loader,
            "__moli_panic_wait_for_script_truthy_for_testing__",
            Duration::from_secs(1),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_observable_output_update_exposes_valid_producer_items() {
        let items = vec![
            ScriptObservableOutputItem::ConsoleMessage("console-a".to_owned()),
            ScriptObservableOutputItem::LifecycleError("error-a".to_owned()),
            ScriptObservableOutputItem::ConsoleMessage("console-b".to_owned()),
        ];
        let update = PageObservableOutputUpdate::append(&items);

        assert_eq!(
            update.observable_output_items(),
            items.as_slice(),
            "observable update should carry the report-level producer item sequence as its only output view"
        );
    }
}

mod resource_command;
pub use resource_command::{CompletedResourceCommand, PendingResourceCommand};

#[cfg(feature = "devtools")]
pub use moli_renderer_v8::{
    RendererPageInspectorEndpoint, RendererRuntimeInspectorSessionRetirement,
};
