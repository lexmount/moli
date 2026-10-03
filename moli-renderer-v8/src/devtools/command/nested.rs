//! Backend entry requirements shared by internal queries and native frontend
//! operations. They are independent of who owns the terminal response.

use super::RendererDevToolsMainNestedDispatch;
use crate::runtime::{RendererPageCommand, RendererScreenshotPurpose};

impl RendererPageCommand {
    pub(crate) fn nested_dispatch(&self) -> RendererDevToolsMainNestedDispatch {
        use RendererDevToolsMainNestedDispatch::{InspectorSession, OwnerOnly, PageAgent};
        match self {
            Self::Native(command) => {
                if command.operation.can_dispatch_on_nested_main() {
                    PageAgent
                } else {
                    OwnerOnly
                }
            }
            Self::Inspector(envelope)
                if envelope.can_dispatch_at_nested_inspector_session_boundary() =>
            {
                InspectorSession
            }
            Self::Inspector(_) => OwnerOnly,
            // Printing temporarily switches media and synchronizes document
            // fonts through V8. A regular screenshot only reads native layout.
            Self::CaptureScreenshot(request)
                if matches!(request.purpose, RendererScreenshotPurpose::Screenshot) =>
            {
                PageAgent
            }
            // These handlers read native document/agent state without entering
            // a new V8 scope. They can use the existing pause callback boundary.
            Self::HasPendingLocationNavigation
            | Self::DomDebuggerConfigureEventListenerBreakpoint { .. }
            | Self::DomDebuggerConfigureXhrBreakpoint { .. }
            | Self::DomDebuggerConfigureDomBreakpoint { .. }
            | Self::ComputedStyleProperties { .. }
            | Self::ClientRectForBackendNodeId { .. }
            | Self::DocumentGeometryForNode { .. }
            | Self::DocumentHitTest { .. }
            | Self::NodeHasGeometryForBackendNodeId { .. }
            | Self::DocumentNodeExistsForBackendNodeId { .. }
            | Self::DocumentNodeSnapshotForBackendNodeId { .. }
            | Self::DocumentNodeSnapshotForNodeInInspectorSession { .. }
            | Self::DocumentNodeSnapshotForDocument { .. }
            | Self::DiscardDomAgentFrontendBindings { .. }
            | Self::DomSnapshotCapture { .. }
            | Self::DocumentChildNodeSnapshotEventsForNode { .. }
            | Self::DocumentQuerySelectorForDocument { .. }
            | Self::DocumentQuerySelectorForChildFrameNode { .. }
            | Self::DocumentQuerySelectorForNode { .. }
            | Self::DocumentQuerySelectorWithChildNodeSnapshotEventsForNode { .. }
            | Self::DocumentPerformSearch { .. }
            | Self::DocumentGetSearchResults { .. }
            | Self::DocumentDiscardSearchResults { .. }
            | Self::DocumentSetNodeStackTracesEnabled { .. }
            | Self::DocumentNodeStackTrace { .. }
            | Self::DocumentFrontendNodeBinding { .. }
            | Self::RegisterDocumentBidiNodeBinding { .. }
            | Self::DocumentBidiNodeBinding { .. }
            | Self::DocumentBidiNodeSharedIdForBackendNodeId { .. }
            | Self::DocumentNodeAttributes { .. }
            | Self::DocumentNodeText { .. }
            | Self::DocumentNodeProperty { .. }
            | Self::AccessibilityTreePayloadsForDocument { .. }
            | Self::AccessibilityNodePayloadForDocument
            | Self::AccessibilityTreePayloadsForNode { .. }
            | Self::AccessibilityNodePayloadForNode { .. }
            | Self::AccessibilityNodeAndAncestorPayloadsForNode { .. }
            | Self::AccessibilityChildNodePayloadsForNode { .. }
            | Self::AccessibilityPartialTreePayloadsForNode { .. }
            | Self::AccessibilityTreePayloadsForChildFrame { .. }
            | Self::AccessibilityNodePayloadForChildFrame { .. }
            | Self::StyleSheetPayloadForStyleSheetId { .. }
            | Self::StyleSheetInventoryForDocument { .. }
            | Self::ResetCssAgentSession { .. }
            | Self::OuterHtmlForDocument { .. }
            | Self::OuterHtmlForNode { .. }
            | Self::SerializeDocument
            | Self::LayoutMetrics
            | Self::PublishLayout
            | Self::CaptureScreencastFrame(..)
            | Self::BlobBytesForUuid { .. }
            | Self::DocumentFrontendNodeIdsForBackendNodeIds { .. }
            | Self::DocumentStorageKeySnapshot
            | Self::ChildFrameTreeSnapshot
            | Self::ChildFrameOwnerNodeReference { .. }
            | Self::ChildFrameDocumentRootNodeReference { .. }
            | Self::PendingSubresourceRequestCount
            | Self::SetFetchSubresourceInterception { .. }
            | Self::SetJavaScriptDialogHandlerEnabled(..)
            | Self::SetExtraHttpHeaders(..)
            | Self::SetNetworkRequestPolicy { .. }
            | Self::SetPermissionOverrides(..)
            | Self::SetScriptExecutionDisabled(..)
            | Self::SetBypassContentSecurityPolicy(..)
            | Self::SetNetworkOffline(..)
            | Self::SetBypassServiceWorker(..)
            | Self::SetBlockedUrlPatterns(..)
            | Self::MsToNextTimeout => PageAgent,

            // These handlers enter V8, run script/lifecycle continuations, or
            // own work whose nested execution has no supported entry point.
            // Keep this match exhaustive: a new backend command must declare
            // its entry requirement instead of inheriting PageAgent by default.
            Self::EvaluateExpression { .. }
            | Self::EvaluateExpressionByValue { .. }
            | Self::EvaluateExpressionAndFollowPendingNavigation { .. }
            | Self::EvaluateExpressionInExecutionContext { .. }
            | Self::EvaluateExpressionInExecutionContextAndFollowPendingNavigation { .. }
            | Self::WaitForSelector { .. }
            | Self::WaitForScriptTruthy { .. }
            | Self::WaitForSubresourceResponse { .. }
            | Self::CompleteChildFrameLifecycleWorkBestEffort { .. }
            | Self::SetDocumentContent { .. }
            | Self::NavigateChildFrame { .. }
            | Self::NavigateTopLevelSameDocument { .. }
            | Self::RefreshFullPageState
            | Self::PageDiagnosticsSnapshot
            | Self::DispatchPreparedElementClick(..)
            | Self::DispatchMouseEventAtPoint { .. }
            | Self::DispatchTouchEvent { .. }
            | Self::DispatchDragEventAtPoint { .. }
            | Self::ClearActiveDragDataTransfer
            | Self::InsertTextIntoActiveControl(..)
            | Self::DispatchKeyEvent { .. }
            | Self::CreateIsolatedWorld { .. }
            | Self::CreateIsolatedWorldRuntimeActivity { .. }
            | Self::InstallRuntimeBinding { .. }
            | Self::RemoveRuntimeBinding(..)
            | Self::RemoveDefaultRuntimeBinding(..)
            | Self::QueueTopLevelHistoryTraversalByDelta(..)
            | Self::RunPageSurfaceOverrideScript { .. }
            | Self::AddDocumentStartScriptRuntimeActivity { .. }
            | Self::RemoveDocumentStartScriptByRegistryKey(..)
            | Self::SetRuntimeBindingState { .. }
            | Self::DefaultExecutionContextId
            | Self::DefaultOrInitialExecutionContextId
            | Self::HasIsolatedWorldNamed { .. }
            | Self::HasIsolatedExecutionContextId(..)
            | Self::EnsureIsolatedWorldsAttachedToInspector
            | Self::InspectorExecutionContextIdForIsolatedContext(..)
            | Self::IsolatedExecutionContextIdForInspectorContext(..)
            | Self::RuntimeRealmInventory
            | Self::LiveChildDefaultRuntimeRealmInventory
            | Self::ChildFrameIdForDefaultExecutionContextId(..)
            | Self::ChildDefaultExecutionContextIdForFrameId(..)
            | Self::RuntimeConsoleMessagesWithContext
            | Self::RuntimeHeapUsage
            | Self::PerformanceMetricSnapshot
            | Self::RuntimeCollectGarbage
            | Self::StopDocumentLifecycle
            | Self::SetInlineStyleSheetTextForStyleSheetId { .. }
            | Self::ScrollNodeIntoViewIfNeeded { .. }
            | Self::RemoveDocumentNode { .. }
            | Self::MutateDocumentNodeAttribute { .. }
            | Self::EditDocumentNode { .. }
            | Self::FocusDocumentNode { .. }
            | Self::TriggerAutofill(..)
            | Self::ResetNavigationHistory
            | Self::SetFileInputFilesForBackendNodeId { .. }
            | Self::RenderPageDump { .. }
            | Self::CaptureScreenshot(..)
            | Self::SearchTextByLines { .. }
            | Self::SearchChildFrameResourceByLines { .. }
            | Self::ContinuePendingSubresourceFetch { .. }
            | Self::ContinuePendingSubresourceAuth { .. }
            | Self::CancelPendingSubresourceAuth { .. }
            | Self::FailPendingSubresourceAuth { .. }
            | Self::FailPendingSubresourceFetch { .. }
            | Self::FulfillPendingSubresourceFetch { .. }
            | Self::ContinuePendingSubresourceResponse { .. }
            | Self::FailPendingSubresourceResponse { .. }
            | Self::FulfillPendingSubresourceResponse { .. }
            | Self::ReceiveSyntheticWebSocketText { .. }
            | Self::ReceiveSyntheticWebSocketBinary { .. }
            | Self::CloseSyntheticWebSocketFromServer { .. }
            | Self::ReplaceBrowserResourceRuntime { .. }
            | Self::RetireDocumentResourceAuthorities
            | Self::ApplyDocumentCookieFacadeOverrides(..)
            | Self::ClearDocumentCookieFacadeOverrides
            | Self::DocumentCookieTelemetrySnapshot
            | Self::DocumentCookieOwnerSnapshot
            | Self::PrepareNetworkResourceLoad { .. }
            | Self::PrepareAppManifestLoad
            | Self::PublishAppManifestLoad(..)
            | Self::SetIdleOverride(..)
            | Self::SetNavigatorOverrides(..)
            | Self::SetNavigatorAndDocumentActivity { .. }
            | Self::SetDocumentActivity(..)
            | Self::SetEmulatedMedia(..)
            | Self::SetScrollbarsHidden(..)
            | Self::SetViewportSurface(..) => OwnerOnly,

            #[cfg(test)]
            Self::SetStoredDocumentStartScripts(..) | Self::TakeDocumentLifecycleEvents => {
                OwnerOnly
            }
            #[cfg(debug_assertions)]
            Self::PanicForTesting => OwnerOnly,
        }
    }
}
