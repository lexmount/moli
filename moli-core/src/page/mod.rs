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
mod document_content_support;
mod document_cookie_diagnostics;
mod dom_protocol_support;
mod history_support;
mod input_support;
mod js_evaluation;
mod navigation_diagnostics;
mod network_resource_support;
mod page_state_cache;
mod protocol_support;
mod renderer_command_support;
mod resource_search_support;
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
pub use command_dispatch::{
    CompletedDevToolsIoCommandDispatch, CompletedPageCommand,
    CompletedRuntimeInspectorCommandDispatch, PendingDevToolsIoCommandDispatch, PendingPageCommand,
    PendingRuntimeInspectorCommandDispatch,
};

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
pub use input_support::{
    PageInputExt, decode_input_dispatch_outcome_completion, decode_insert_text_completion,
};
pub use moli_page_types::DomScrollIntoViewRect;
pub use moli_page_types::{
    RendererDomDebuggerDomBreakpointType, RendererDomDebuggerEventListenerBreakpoint,
    RendererDomDebuggerXhrBreakpoint, RendererInspectorProtocolConfiguration,
    RendererInspectorProtocolConfigurationCommand, RendererInspectorSessionRestoreSnapshot,
    SameDocumentHistoryUpdate, V8InspectorSessionAttach, V8InspectorSessionState,
    renderer_inspector_protocol_configuration_command_from_message,
    renderer_inspector_protocol_configuration_command_from_method,
};
pub use moli_renderer_v8::RendererRuntimeInspectorMessageResponseOrder;
pub use moli_renderer_v8::dom::native::SelectedFile;
pub use moli_renderer_v8::network::{
    RendererNetworkResourceLoadOutcome, RendererNetworkResourceLoadPreparation,
    RendererNetworkResourceLoadResponse, RendererPreparedNetworkResourceLoad,
};
pub use moli_renderer_v8::{
    DevToolsSessionKey, RendererActivityDiagnostics, RendererAgentAttachmentId,
    RendererAutofillAddressField, RendererAutofillCreditCard, RendererAutofillTriggerOutcome,
    RendererAutofillTriggerRequest, RendererCaptureScreenshotReply,
    RendererCaptureScreenshotRequest, RendererCommandTurnCompletion, RendererCommandTurnOutput,
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
    RendererDomSnapshotCaptureOptions, RendererDomSnapshotCapturePayload, RendererDragData,
    RendererDragDataItem, RendererDraggedDirectory, RendererDraggedFile, RendererFrameToken,
    RendererInputDispatchOutcome, RendererInspectorCommandRoute,
    RendererJavaScriptDialogCompletion, RendererJavaScriptDialogId, RendererJavaScriptDialogResult,
    RendererJavaScriptDialogSource, RendererLayoutMetrics, RendererLifecycleEpoch,
    RendererLifecycleEventStamp, RendererLifecycleStartReason, RendererLifecycleTerminationStamp,
    RendererMainDocumentCommit, RendererPageCommandPostResponseContinuation,
    RendererPageCreationArtifacts, RendererPageCreationDiagnostics,
    RendererPageDiagnosticsSnapshot, RendererPageDumpFormat, RendererPageDumpOptions,
    RendererPageDumpStripOptions, RendererPendingDownloadActivation,
    RendererPendingDownloadResponse, RendererPendingFileChooserActivation,
    RendererPendingJavaScriptDialog, RendererPendingPopupActivation,
    RendererPendingSameDocumentNavigation, RendererPendingTopLevelHistoryTraversal,
    RendererPendingWindowOpenEvent, RendererPerformanceMetricSnapshot,
    RendererPointerEventProperties, RendererPopupActivationSource,
    RendererResourceTextSearchOutcome, RendererRuntimeCommandOutput, RendererRuntimeHeapUsage,
    RendererRuntimeInspectorIoCommandClaim, RendererRuntimeInspectorIoCommandRoute,
    RendererRuntimeInspectorMainCommandCompletion, RendererRuntimeInspectorMainCommandRoute,
    RendererRuntimeInspectorMessage, RendererRuntimeInspectorMessageBatch,
    RendererRuntimeInspectorProtocolMessage, RendererRuntimeInspectorProtocolMessageValueMut,
    RendererRuntimeObservableSourceItem, RendererRuntimeObservableSourceSummary,
    RendererRuntimeRealmInfo, RendererScreenshotClip, RendererScreenshotFormat,
    RendererScreenshotPurpose, RendererScreenshotRegion, RendererScrollIntoViewResult,
    RendererServiceWorkerConsoleMessage, RendererServiceWorkerExceptionMessage,
    RendererServiceWorkerFetchDiagnostic, RendererServiceWorkerFetchDiagnosticResult,
    RendererServiceWorkerRunIdentity, RendererServiceWorkerTargetEvent,
    RendererServiceWorkerTargetInfo, RendererServiceWorkerVersionStatus,
    RendererSetDocumentContentResult, RendererSharedWorkerConsoleMessage,
    RendererSharedWorkerTargetEvent, RendererSharedWorkerTargetInfo, RendererSyntheticResponseBody,
    RendererTextSearchMatch, RendererTouchPoint, RendererWindowDocumentSource,
    RuntimeConsoleMessageSnapshot,
};
pub use moli_renderer_v8::{
    RendererAppManifest, RendererAppManifestDisplayMode, RendererAppManifestError,
    RendererAppManifestImageResource, RendererAppManifestLoadOutcome,
    RendererAppManifestLoadPreparation, RendererAppManifestLoadPublication,
    RendererAppManifestOrientation, RendererAppManifestProtocolHandler,
    RendererAppManifestQueryResult, RendererAppManifestRelatedApplication,
    RendererAppManifestShortcut, RendererPreparedAppManifestLoad,
};
pub use navigation_diagnostics::{NavigationRedirect, NavigationResponse};
pub use protocol_support::{
    BidiPreloadChannelHandoff, ChildFrameAttachmentSnapshot, ChildFrameDetachmentSnapshot,
    ChildFrameDocumentNetworkActivitySnapshot, ChildFrameDocumentNetworkSnapshot,
    ChildFrameDocumentOpenedSnapshot, ChildFrameNavigationSnapshot, ChildFrameTreeEventSnapshot,
    ChildFrameTreeSnapshot, ContentSecurityPolicyIssueSnapshot, ContentSecurityPolicyViolationType,
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
    SubresourceJsonPathEquals, SubresourceJsonPathRegex, SubresourceNetworkOutcome,
    SubresourceNetworkRecord, SubresourceNetworkRequestHandle, SubresourceRequestInitiatorType,
    SubresourceRequestStarted, SubresourceResourceType, SubresourceResponseBody,
    SubresourceResponseStarted, SubresourceResponseWaitCriteria, ViewportSurface,
    WebSocketFrameDirection, WebSocketFrameOpcode, WebSocketLifecycleEvent, WebSocketLifecycleKind,
    WebSocketNetworkEvent, extract_subresource_auth_challenge,
    subresource_auth_credentials_for_challenge,
};
pub use renderer_command_support::DocumentNodeClientRectResolution;
pub use renderer_command_support::{
    DocumentNodeRuntimeObjectResolution, PageNetworkOutputUpdate, PageObservableOutputUpdate,
    TestingOutcome,
};

#[cfg(test)]
use crate::renderer::RendererPageTestingHandle;

pub use crate::renderer::{
    RendererDocumentFrontendNodeIdsResolution, RendererDocumentNodeAttributesResolution,
    RendererDocumentNodePropertyResolution, RendererDocumentNodeReference,
    RendererDocumentNodeTextResolution, RendererDocumentQuerySelectorNode,
    RendererDocumentQuerySelectorResolution,
    RendererDocumentQuerySelectorWithChildNodeSnapshotEvents, RendererDomBidiNodeBindingResolution,
    RendererDomBidiNodeSharedIdResolution, RendererDomFrontendNodeBindingResolution,
    RendererDomNodeCreationStackFrame, RendererDomNodeCreationStackTrace,
    RendererDomNodeStackTraceResolution, RendererDomSearchRegistration,
    RendererDomSearchResultNode, RendererDomSearchResultsResolution, RendererRuntimeRemoteObject,
    RendererStyleSheetHeader, RendererStyleSheetInventoryUpdate, RendererStyleSheetPayload,
};

pub struct Page {
    page_state: PageStateCache,
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
    pub(crate) fn from_attached_handle(
        handle: RendererPageHandle,
        page_state: Arc<RendererPageState>,
    ) -> Self {
        Self {
            page_state: PageStateCache::new(page_state),
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
        Self {
            page_state: PageStateCache::new(page_state),
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

    /// Seals this target's Main/IO DevTools ingress and interrupts active V8.
    ///
    /// `Page.crash` is a terminal renderer IO control in Chromium, not an
    /// ordinary V8 Inspector command. Protocol integrations use this boundary
    /// to retire the Page without waiting behind a session lane.
    #[doc(hidden)]
    pub fn crash_devtools_target_from_io(&self) {
        self.handle.crash_devtools_target_from_io();
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
    pub(crate) async fn panic_wait_for_selector_for_testing(
        &mut self,
        loader: &ResourceRequestClient,
    ) -> Result<()> {
        self.wait_for_selector(
            loader,
            "__moli_panic_wait_for_selector_for_testing__",
            Duration::from_secs(1),
        )
        .await
        .map(|_| ())
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

    // Input dispatch methods (delegated to by the PageInputExt trait in input_support)

    pub(crate) async fn dispatch_mouse_event_at_point_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        delta_x: f64,
        delta_y: f64,
    ) -> Result<bool> {
        Ok(self
            .dispatch_mouse_event_at_point_with_outcome_async(
                x, y, event_name, button, buttons, delta_x, delta_y,
            )
            .await?
            .handled)
    }

    pub(crate) async fn dispatch_mouse_event_at_point_with_outcome_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        button: i32,
        buttons: Option<i32>,
        delta_x: f64,
        delta_y: f64,
    ) -> Result<RendererInputDispatchOutcome> {
        let command = RendererPageCommand::DispatchMouseEventAtPoint {
            x,
            y,
            event_name: event_name.to_owned(),
            button,
            buttons,
            click_count: 0,
            delta_x,
            delta_y,
            pointer: RendererPointerEventProperties::default(),
            modifiers: 0,
        };
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "mouse event page command",
            "an input dispatch outcome reply",
            RendererPageReply::InputDispatchOutcome(value) => Ok(value),
        )
    }

    pub(crate) async fn dispatch_touch_event_at_point_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        activate: bool,
    ) -> Result<bool> {
        Ok(self
            .dispatch_touch_event_at_point_with_outcome_async(x, y, event_name, activate)
            .await?
            .handled)
    }

    pub(crate) async fn dispatch_drag_event_at_point_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        data: RendererDragData,
        modifiers: u8,
    ) -> Result<bool> {
        Ok(self
            .dispatch_drag_event_at_point_with_outcome_async(x, y, event_name, data, modifiers)
            .await?
            .handled)
    }

    pub(crate) async fn dispatch_touch_event_at_point_with_outcome_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        activate: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        let command = RendererPageCommand::DispatchTouchEvent {
            points: vec![RendererTouchPoint { id: 0, x, y }],
            event_name: event_name.to_owned(),
            activate,
        };
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "touch event page command",
            "an input dispatch outcome reply",
            RendererPageReply::InputDispatchOutcome(value) => Ok(value),
        )
    }

    pub(crate) async fn dispatch_drag_event_at_point_with_outcome_async(
        &mut self,
        x: f64,
        y: f64,
        event_name: &str,
        data: RendererDragData,
        modifiers: u8,
    ) -> Result<RendererInputDispatchOutcome> {
        let command = RendererPageCommand::DispatchDragEventAtPoint {
            x,
            y,
            event_name: event_name.to_owned(),
            data,
            modifiers,
        };
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "drag event page command",
            "an input dispatch outcome reply",
            RendererPageReply::InputDispatchOutcome(value) => Ok(value),
        )
    }

    pub(crate) async fn clear_active_drag_data_transfer_async(&mut self) -> Result<()> {
        let reply = self
            .dispatch_page_command_async(RendererPageCommand::ClearActiveDragDataTransfer)
            .await?;
        expect_page_reply!(
            reply,
            "clear active drag data transfer page command",
            "a unit reply",
            RendererPageReply::Unit => Ok(()),
        )
    }

    pub(crate) async fn insert_text_into_active_control_async(
        &mut self,
        text: &str,
    ) -> Result<bool> {
        let command = RendererPageCommand::InsertTextIntoActiveControl(text.to_owned());
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "insert text page command",
            "a bool reply",
            RendererPageReply::Bool(value) => Ok(value),
        )
    }

    pub(crate) async fn dispatch_key_event_async(
        &mut self,
        event_name: &str,
        key: &str,
        code: &str,
        text: &str,
        modifiers: u8,
        auto_repeat: bool,
        should_insert_text: bool,
    ) -> Result<bool> {
        Ok(self
            .dispatch_key_event_with_outcome_async(
                event_name,
                key,
                code,
                text,
                modifiers,
                auto_repeat,
                should_insert_text,
            )
            .await?
            .handled)
    }

    pub(crate) async fn dispatch_key_event_with_outcome_async(
        &mut self,
        event_name: &str,
        key: &str,
        code: &str,
        text: &str,
        modifiers: u8,
        auto_repeat: bool,
        should_insert_text: bool,
    ) -> Result<RendererInputDispatchOutcome> {
        let command = RendererPageCommand::DispatchKeyEvent {
            event_name: event_name.to_owned(),
            key: key.to_owned(),
            code: code.to_owned(),
            text: text.to_owned(),
            modifiers,
            auto_repeat,
            should_insert_text,
        };
        let reply = self.dispatch_page_command_async(command).await?;
        expect_page_reply!(
            reply,
            "key event page command",
            "an input dispatch outcome reply",
            RendererPageReply::InputDispatchOutcome(value) => Ok(value),
        )
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
