use crate::page::Page;
use anyhow::{Result, anyhow};
pub use moli_renderer_v8::ExternalRawDocumentBodyStream;
pub(crate) use moli_renderer_v8::RendererClientRect;
pub use moli_renderer_v8::RendererRuntimeRemoteObject;
pub(crate) use moli_renderer_v8::{
    DocumentCookieBackendConnectionState, DocumentCookieBrowserContextSnapshot,
    DocumentCookieCacheLookupResult, DocumentCookieCacheSnapshot, DocumentCookieCacheStatus,
    DocumentCookieCapabilitySnapshot, DocumentCookieFacadeTelemetrySnapshot,
    DocumentCookieFirstOperation, DocumentCookieGetFreshnessStatus, DocumentCookieOwnerSnapshot,
    DocumentCookieSetReadinessStatus, DocumentCookieWriteCapabilitySnapshot, JsRuntime,
    RendererCaptureScreenshotReply, RendererCaptureScreenshotRequest, RendererHtmlSerializeOptions,
    RendererLayoutMetrics, RendererMarkdownOptions, RendererOwnerCommand, RendererOwnerHandle,
    RendererOwnerReply, RendererPageCommand, RendererPageCommandPending, RendererPageHandle,
    RendererPageReply, RendererPageState, RendererPendingDownloadActivation,
    RendererPerformanceMetricSnapshot, RendererRuntimeRemoteObjectResolution, ScriptRunOutcome,
};
#[cfg(feature = "devtools")]
pub(crate) use moli_renderer_v8::{
    RendererAccessibilityPayloadsForObjectId, RendererDocumentHitTestResult,
    RendererDomDebuggerDomBreakpointResolution, RendererDomDebuggerEventListenerBreakpoint,
    RendererDomDebuggerEventListenersResolution, RendererDomDebuggerXhrBreakpoint,
    RendererDomSnapshotCaptureOptions, RendererDomSnapshotCapturePayload,
    RendererRuntimeInspectorResponseSender,
};
#[cfg(feature = "devtools")]
pub(crate) use moli_renderer_v8::{
    RendererAutofillTriggerOutcome, RendererAutofillTriggerRequest,
    RendererCaptureScreencastFrameReply, RendererCaptureScreencastFrameRequest,
    RendererDocumentNodeClientRect, RendererDocumentNodeGeometry, RendererDomAttributeMutation,
    RendererDomAttributeMutationOutcome, RendererDomFocusOutcome,
    RendererPageCookieFacadeSnapshotReply, RendererRuntimeHeapUsage,
};
pub use moli_renderer_v8::{
    RendererDocumentChildNodeSnapshotEvents, RendererDocumentFrontendNodeIdsResolution,
    RendererDocumentNodeAttributesResolution, RendererDocumentNodePropertyResolution,
    RendererDocumentNodeReference, RendererDocumentNodeTextResolution,
    RendererDocumentQuerySelectorNode, RendererDocumentQuerySelectorResolution,
    RendererDocumentQuerySelectorWithChildNodeSnapshotEvents, RendererDomBidiNodeBindingResolution,
    RendererDomBidiNodeSharedIdResolution, RendererDomFrontendNodeBindingResolution,
    RendererDomNodeCreationStackFrame, RendererDomNodeCreationStackTrace, RendererDomNodeReference,
    RendererDomNodeStackTraceResolution, RendererDomSearchRegistration,
    RendererDomSearchResultNode, RendererDomSearchResultsResolution, RendererStyleSheetHeader,
    RendererStyleSheetInventoryUpdate, RendererStyleSheetPayload,
};
#[cfg(feature = "devtools")]
pub use moli_renderer_v8::{RendererDomEdit, RendererDomEditOutcome};
#[cfg(feature = "devtools")]
pub(crate) use moli_renderer_v8::{
    RendererInspectorCommandEnvelope, RendererInspectorCommandRoute, RendererInspectorIngressTicket,
};

pub use moli_renderer_v8::{PageVmInitStage, RendererReplyBoundary};

pub(crate) struct MaterializedPageCreatedReply {
    pub(crate) page: Page,
    pub(crate) pending_download: Option<RendererPendingDownloadActivation>,
}

pub(crate) fn materialize_page_created_reply(
    renderer_owner: &RendererOwnerHandle,
    reply: RendererOwnerReply,
) -> Result<Page> {
    let materialized = materialize_page_created_reply_with_side_effect(renderer_owner, reply)?;
    if materialized.pending_download.is_some() {
        return Err(anyhow!(
            "page creation produced a pending download side-effect for a caller that expects a pure page"
        ));
    }
    Ok(materialized.page)
}

pub(crate) fn materialize_page_created_reply_with_side_effect(
    renderer_owner: &RendererOwnerHandle,
    reply: RendererOwnerReply,
) -> Result<MaterializedPageCreatedReply> {
    let (handle, page_state, _page_creation_diagnostics, page_creation_artifacts, pending_download) =
        renderer_owner.materialize_page_created_reply_parts(reply)?;
    Ok(MaterializedPageCreatedReply {
        page: Page::from_attached_handle_with_creation_artifacts(
            handle,
            page_state,
            page_creation_artifacts,
        ),
        pending_download,
    })
}

#[cfg(test)]
pub(crate) use moli_renderer_v8::{PageId, RendererPageTestingHandle, RendererPageView};
