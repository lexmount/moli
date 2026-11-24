use super::super::*;

#[non_exhaustive]
pub enum RendererDevToolsPageReply {
    WebMcp(std::result::Result<Option<u64>, moli_page_types::RendererWebMcpError>),
    NativeCommandReady(Box<RendererNativeCommandReadyResponse>),
    NativeCommandPublished,
    ElementClickDispatch(Result<RendererInputDispatchOutcome, RendererElementClickError>),
    PerformanceMetricSnapshot(Box<RendererPerformanceMetricSnapshot>),
    ExecutionContextId(i64),
    OptionalDomSnapshotCapturePayload(Option<RendererDomSnapshotCapturePayload>),
    DocumentQuerySelectorResolution(RendererDocumentQuerySelectorResolution),
    DocumentQuerySelectorWithChildNodeSnapshotEvents(
        RendererDocumentQuerySelectorWithChildNodeSnapshotEvents,
    ),
    DocumentPerformSearch(RendererDomSearchRegistration),
    DocumentSearchResults {
        resolution: RendererDomSearchResultsResolution,
        child_node_snapshot_events: Option<RendererDocumentChildNodeSnapshotEvents>,
    },
    DocumentSearchResultsDiscarded,
    DocumentNodeStackTracesEnabled,
    DocumentNodeStackTrace(RendererDomNodeStackTraceResolution),
    DocumentFrontendNodeBinding(RendererDomFrontendNodeBindingResolution),
    DocumentBidiNodeBinding(RendererDomBidiNodeBindingResolution),
    DocumentBidiNodeSharedId(RendererDomBidiNodeSharedIdResolution),
    DocumentBidiNodeBindingRegistered,
    DocumentNodeAttributesResolution(RendererDocumentNodeAttributesResolution),
    DocumentNodeTextResolution(RendererDocumentNodeTextResolution),
    DocumentNodePropertyResolution(RendererDocumentNodePropertyResolution),
    OptionalAccessibilityPayloads(Option<Vec<Value>>),
    OptionalAccessibilityPayload(Option<Value>),
    OptionalAccessibilityPayloadsForObjectId(Option<RendererAccessibilityPayloadsForObjectId>),
    OptionalStyleSheetPayload(Option<RendererStyleSheetPayload>),
    StyleSheetInventory(RendererStyleSheetInventoryUpdate),
    DocumentFrontendNodeIds(RendererDocumentFrontendNodeIdsResolution),
    OptionalDocumentNodeClientRect(Option<RendererDocumentNodeClientRect>),
    OptionalDocumentNodeGeometry(Option<RendererDocumentNodeGeometry>),
    OptionalDocumentHitTest(Option<RendererDocumentHitTestResult>),
    DomDebuggerEventListeners(RendererDomDebuggerEventListenersResolution),
    DomDebuggerDomBreakpoint(RendererDomDebuggerDomBreakpointResolution),
    ComputedStyleProperties(Option<Vec<(String, String)>>),
    DomAttributeMutationOutcome(RendererDomAttributeMutationOutcome),
    DomEditOutcome(RendererDomEditOutcome),
    DomFocusOutcome(RendererDomFocusOutcome),
    AutofillTriggerOutcome(RendererAutofillTriggerOutcome),
    ResourceTextSearchOutcome(RendererResourceTextSearchOutcome),
    NetworkResourceLoadPreparation(crate::network::RendererNetworkResourceLoadPreparation),
    InputDispatchOutcome(RendererInputDispatchOutcome),
    ScrollIntoViewResult(RendererScrollIntoViewResult),
    OptionalBool(Option<bool>),
    CaptureScreencastFrame(RendererCaptureScreencastFrameReply),
    #[doc(hidden)]
    ScreencastFrameRasterJob(Box<super::super::page_screenshot::RendererScreencastFrameRasterJob>),
}

impl RendererDevToolsPageReply {
    pub(in crate::runtime) fn input_dispatch_outcomes(&self) -> &[RendererInputDispatchOutcome] {
        match self {
            Self::InputDispatchOutcome(outcome) | Self::ElementClickDispatch(Ok(outcome)) => {
                std::slice::from_ref(outcome)
            }
            Self::NativeCommandReady(ready) => ready.input_dispatch_outcomes(),
            _ => &[],
        }
    }

    pub(in crate::runtime) fn input_dispatch_outcomes_mut(
        &mut self,
    ) -> &mut [RendererInputDispatchOutcome] {
        match self {
            Self::InputDispatchOutcome(outcome) | Self::ElementClickDispatch(Ok(outcome)) => {
                std::slice::from_mut(outcome)
            }
            Self::NativeCommandReady(ready) => ready.input_dispatch_outcomes_mut(),
            _ => &mut [],
        }
    }
}
