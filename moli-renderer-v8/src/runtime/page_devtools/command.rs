use super::super::*;

pub(crate) type RendererInspectorPageCommand =
    moli_command_types::devtools::InspectorCommand<RendererInspectorCommandHandles>;

/// Execution capabilities stay in the renderer rather than the command DTO crate.
pub(crate) struct RendererInspectorCommandHandles;

impl moli_command_types::devtools::InspectorCommandHandles for RendererInspectorCommandHandles {
    type ResponseSender = RendererRuntimeInspectorResponseSender;
    type RuntimeOutput = RendererRuntimeCommandOutput;
    type SessionDetachGuard = RendererRuntimeInspectorSessionDetachGuard;
}

#[non_exhaustive]
pub enum RendererDevToolsPageCommand {
    WebMcp {
        session: DevToolsSessionKey,
        command: moli_page_types::RendererWebMcpCommand,
    },
    // The call fits inline; its operation boxes the recursively contained page command.
    Native(RendererCdpCall),
    Inspector(RendererInspectorCommandEnvelope),
    DispatchPreparedElementClick(RendererPreparedPointerClick),
    DispatchMouseEventAtPoint {
        x: f64,
        y: f64,
        event_name: String,
        button: i32,
        buttons: Option<i32>,
        click_count: i32,
        delta_x: f64,
        delta_y: f64,
        pointer: RendererPointerEventProperties,
        modifiers: u8,
        /// `Input.setInterceptDrags` state of the issuing DevTools session.
        intercept_drags: bool,
    },
    ValidateSyntheticWheelAnchor {
        x: f64,
        y: f64,
        gesture: crate::runtime::RendererSyntheticWheelGesture,
    },
    DispatchSyntheticWheel {
        gesture: crate::runtime::RendererSyntheticWheelGesture,
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
        end: bool,
        intercept_drags: bool,
    },
    DispatchTouchEvent {
        points: Vec<RendererTouchPoint>,
        event_name: String,
        activate: bool,
    },
    DispatchDragEventAtPoint {
        x: f64,
        y: f64,
        event_name: String,
        data: RendererDragData,
        modifiers: u8,
    },
    ClearActiveDragDataTransfer,
    CancelDragging,
    InsertTextIntoActiveControl(String),
    DispatchKeyEvent {
        event_name: String,
        key: String,
        code: String,
        text: String,
        modifiers: u8,
        location: u32,
        auto_repeat: bool,
        should_insert_text: bool,
    },
    DomDebuggerConfigureEventListenerBreakpoint {
        inspector_session_id: Option<String>,
        breakpoint: RendererDomDebuggerEventListenerBreakpoint,
        enabled: bool,
    },
    DomDebuggerConfigureXhrBreakpoint {
        inspector_session_id: Option<String>,
        breakpoint: RendererDomDebuggerXhrBreakpoint,
        enabled: bool,
    },
    DomDebuggerConfigureDomBreakpoint {
        inspector_session_id: Option<String>,
        frontend_node_id: u32,
        breakpoint_type: String,
        enabled: bool,
    },
    CreateIsolatedWorld {
        name: String,
        grant_universal_access: bool,
        frame_id: Option<String>,
    },
    CreateIsolatedWorldRuntimeActivity {
        inspector_session_id: Option<String>,
        frame_id: Option<String>,
        name: String,
        grant_universal_access: bool,
    },
    InstallRuntimeBinding {
        name: String,
        execution_context_name: Option<String>,
        execution_context_id: Option<i64>,
    },
    RemoveRuntimeBinding(String),
    RemoveDefaultRuntimeBinding(String),
    AddDocumentStartScriptRuntimeActivity {
        inspector_session_id: Option<String>,
        script: DocumentStartScript,
        run_immediately: bool,
    },
    RemoveDocumentStartScriptByRegistryKey(String),
    ConnectRuntimeInspectorSession {
        inspector_session_id: Option<String>,
        connection: crate::devtools::ingress::session::RendererInspectorSessionConnect,
    },
    SetSessionRuntimeBindingState {
        agent: crate::runtime::RendererDevToolsAgentToken,
        inspector_session_id: Option<String>,
        session_runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
    },
    SetRuntimeBindingState {
        inspector_session_id: Option<String>,
        stored_runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
        session_runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
    },
    EnsureIsolatedWorldsAttachedToInspector,
    InspectorExecutionContextIdForIsolatedContext(i64),
    IsolatedExecutionContextIdForInspectorContext(i64),
    PerformanceMetricSnapshot,
    RuntimeCollectGarbage,
    ComputedStyleProperties {
        reference: RendererDomNodeReference,
    },
    SetInlineStyleSheetTextForStyleSheetId {
        inspector_session_id: Option<String>,
        style_sheet_id: String,
        text: String,
    },
    ScrollNodeIntoViewIfNeeded {
        reference: RendererDomNodeReference,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    },
    ClientRectForBackendNodeId {
        backend_node_id: u32,
    },
    DocumentGeometryForNode {
        reference: RendererDomNodeReference,
    },
    SetOverlayHighlight {
        inspector_session_id: Option<String>,
        highlight: Result<Option<RendererOverlayHighlight>, String>,
    },
    DocumentHitTest {
        inspector_session_id: Option<String>,
        x: f64,
        y: f64,
        include_user_agent_shadow_dom: bool,
        ignore_pointer_events_none: bool,
    },
    NodeHasGeometryForBackendNodeId {
        backend_node_id: u32,
    },
    RemoveDocumentNode {
        reference: RendererDomNodeReference,
    },
    MutateDocumentNodeAttribute {
        reference: RendererDomNodeReference,
        mutation: RendererDomAttributeMutation,
    },
    EditDocumentNode {
        inspector_session_id: Option<String>,
        edit: RendererDomEdit,
    },
    FocusDocumentNode {
        reference: RendererDomNodeReference,
    },
    TriggerAutofill(RendererAutofillTriggerRequest),
    SetFileInputFilesForBackendNodeId {
        backend_node_id: u32,
        files: Vec<crate::dom::native::SelectedFile>,
        append: bool,
    },
    DocumentNodeExistsForBackendNodeId {
        backend_node_id: u32,
    },
    DocumentNodeSnapshotForBackendNodeId {
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    },
    DocumentNodeSnapshotForNodeInInspectorSession {
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        reference: RendererDomNodeReference,
        depth: i32,
        pierce: bool,
    },
    DocumentNodeSnapshotForDocument {
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        depth: i32,
        pierce: bool,
    },
    DiscardDomAgentFrontendBindings {
        inspector_session_id: Option<String>,
    },
    DomSnapshotCapture {
        top_frame_id: String,
        options: RendererDomSnapshotCaptureOptions,
    },
    DocumentChildNodeSnapshotEventsForNode {
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        reference: RendererDomNodeReference,
        depth: i32,
        pierce: bool,
    },
    DocumentQuerySelectorForDocument {
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        selector: String,
        multiple: bool,
    },
    DocumentQuerySelectorForChildFrameNode {
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        frame_id: String,
        root: RendererDomNodeReference,
        selector: String,
        multiple: bool,
    },
    DocumentQuerySelectorForNode {
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        root: RendererDomNodeReference,
        selector: String,
        multiple: bool,
    },
    DocumentQuerySelectorWithChildNodeSnapshotEventsForNode {
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        root: RendererDomNodeReference,
        selector: String,
        multiple: bool,
    },
    DocumentPerformSearch {
        inspector_session_id: Option<String>,
        query: String,
        include_user_agent_shadow_dom: bool,
        include_whitespace: bool,
    },
    DocumentGetSearchResults {
        inspector_session_id: Option<String>,
        search_id: String,
        from_index: usize,
        to_index: usize,
    },
    DocumentDiscardSearchResults {
        inspector_session_id: Option<String>,
        search_id: String,
    },
    DocumentSetNodeStackTracesEnabled {
        inspector_session_id: Option<String>,
        enabled: bool,
    },
    DocumentNodeStackTrace {
        inspector_session_id: Option<String>,
        frontend_node_id: u32,
    },
    DocumentFrontendNodeBinding {
        reference: RendererDomNodeReference,
    },
    RegisterDocumentBidiNodeBinding {
        inspector_session_id: Option<String>,
        shared_id: String,
        backend_node_id: u32,
    },
    DocumentBidiNodeBinding {
        inspector_session_id: Option<String>,
        shared_id: String,
    },
    DocumentBidiNodeSharedIdForBackendNodeId {
        inspector_session_id: Option<String>,
        backend_node_id: u32,
    },
    DocumentNodeAttributes {
        reference: RendererDomNodeReference,
    },
    DocumentNodeText {
        reference: RendererDomNodeReference,
    },
    DocumentNodeProperty {
        reference: RendererDomNodeReference,
        name: String,
    },
    AccessibilityTreePayloadsForDocument {
        max_depth: Option<i32>,
    },
    AccessibilityNodePayloadForDocument,
    AccessibilityTreePayloadsForNode {
        reference: RendererDomNodeReference,
        max_depth: Option<i32>,
    },
    AccessibilityNodePayloadForNode {
        reference: RendererDomNodeReference,
    },
    AccessibilityNodeAndAncestorPayloadsForNode {
        reference: RendererDomNodeReference,
    },
    AccessibilityChildNodePayloadsForNode {
        reference: RendererDomNodeReference,
    },
    AccessibilityPartialTreePayloadsForNode {
        reference: RendererDomNodeReference,
        fetch_relatives: bool,
    },
    AccessibilityTreePayloadsForChildFrame {
        frame_id: String,
        max_depth: Option<i32>,
    },
    AccessibilityNodePayloadForChildFrame {
        frame_id: String,
    },
    StyleSheetPayloadForStyleSheetId {
        inspector_session_id: Option<String>,
        style_sheet_id: String,
    },
    StyleSheetInventoryForDocument {
        inspector_session_id: Option<String>,
    },
    ResetCssAgentSession {
        inspector_session_id: Option<String>,
    },
    CaptureScreencastFrame(RendererCaptureScreencastFrameRequest),
    DocumentFrontendNodeIdsForBackendNodeIds {
        inspector_session_id: Option<String>,
        backend_node_ids: Vec<u32>,
    },
    SearchResourceByLines(Box<RendererResourceSearchRequest>),
    ChildFrameOwnerNodeReference {
        inspector_session_id: Option<String>,
        frame_id: String,
    },
    ChildFrameDocumentRootNodeReference {
        inspector_session_id: Option<String>,
        frame_id: String,
    },
    ContinuePendingSubresourceFetch {
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<bytes::Bytes>>,
        headers: Option<moli_fetch::RequestHeaderOverride>,
        intercept_response: bool,
        handle_auth_requests: bool,
    },
    ContinuePendingSubresourceAuth {
        internal_id: u64,
        auth: crate::SubresourceAuthCredentials,
    },
    CancelPendingSubresourceAuth {
        internal_id: u64,
    },
    FailPendingSubresourceAuth {
        internal_id: u64,
        error_text: String,
    },
    FailPendingSubresourceFetch {
        internal_id: u64,
        error_text: String,
    },
    FulfillPendingSubresourceFetch {
        internal_id: u64,
        request_overrides: Option<Box<moli_fetch::RequestOverrides>>,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    },
    ContinuePendingSubresourceResponse {
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
    },
    FailPendingSubresourceResponse {
        internal_id: u64,
        error_text: String,
    },
    FulfillPendingSubresourceResponse {
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    },
    ReceiveSyntheticWebSocketText {
        socket_id: u64,
        data: String,
    },
    ReceiveSyntheticWebSocketBinary {
        socket_id: u64,
        data: Vec<u8>,
    },
    CloseSyntheticWebSocketFromServer {
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    },
    SetFetchSubresourceInterception {
        classes: crate::SubresourceInterceptionClasses,
    },
    PrepareNetworkResourceLoad {
        frame_id: String,
        url: Url,
        disable_cache: bool,
        include_credentials: bool,
    },
}

impl RendererDevToolsPageCommand {
    pub(in crate::runtime) fn cdp_nav_timing_label(&self) -> Option<&'static str> {
        match self {
            Self::Native(command) => command.operation.command.cdp_nav_timing_label(),
            Self::Inspector(envelope) => envelope.cdp_nav_timing_label(),
            Self::SearchResourceByLines(..) => Some("SearchResourceByLines"),
            Self::CreateIsolatedWorldRuntimeActivity { .. } => {
                Some("CreateIsolatedWorldRuntimeActivity")
            }
            Self::AddDocumentStartScriptRuntimeActivity { .. } => {
                Some("AddDocumentStartScriptRuntimeActivity")
            }
            Self::RemoveDocumentStartScriptByRegistryKey(_) => {
                Some("RemoveDocumentStartScriptByRegistryKey")
            }
            Self::ConnectRuntimeInspectorSession { .. } => Some("ConnectRuntimeInspectorSession"),
            Self::SetSessionRuntimeBindingState { .. } => Some("SetSessionRuntimeBindingState"),
            Self::SetRuntimeBindingState { .. } => Some("SetRuntimeBindingState"),
            Self::ChildFrameOwnerNodeReference { .. } => Some("ChildFrameOwnerNodeReference"),
            Self::ChildFrameDocumentRootNodeReference { .. } => {
                Some("ChildFrameDocumentRootNodeReference")
            }
            Self::DocumentNodeExistsForBackendNodeId { .. } => {
                Some("DocumentNodeExistsForBackendNodeId")
            }
            Self::DocumentNodeSnapshotForBackendNodeId { .. } => {
                Some("DocumentNodeSnapshotForBackendNodeId")
            }
            Self::DocumentNodeSnapshotForNodeInInspectorSession { .. } => {
                Some("DocumentNodeSnapshotForNodeInInspectorSession")
            }
            Self::DocumentNodeSnapshotForDocument { .. } => Some("DocumentNodeSnapshotForDocument"),
            Self::DiscardDomAgentFrontendBindings { .. } => Some("DiscardDomAgentFrontendBindings"),
            Self::DomSnapshotCapture { .. } => Some("DomSnapshotCapture"),
            Self::DocumentChildNodeSnapshotEventsForNode { .. } => {
                Some("DocumentChildNodeSnapshotEventsForNode")
            }
            Self::DocumentQuerySelectorForDocument { .. } => {
                Some("DocumentQuerySelectorForDocument")
            }
            Self::DocumentQuerySelectorForChildFrameNode { .. } => {
                Some("DocumentQuerySelectorForChildFrameNode")
            }
            Self::DocumentQuerySelectorForNode { .. } => Some("DocumentQuerySelectorForNode"),
            Self::DocumentQuerySelectorWithChildNodeSnapshotEventsForNode { .. } => {
                Some("DocumentQuerySelectorWithChildNodeSnapshotEventsForNode")
            }
            Self::DocumentPerformSearch { .. } => Some("DocumentPerformSearch"),
            Self::DocumentGetSearchResults { .. } => Some("DocumentGetSearchResults"),
            Self::DocumentSetNodeStackTracesEnabled { .. } => {
                Some("DocumentSetNodeStackTracesEnabled")
            }
            Self::DocumentNodeStackTrace { .. } => Some("DocumentNodeStackTrace"),
            Self::DocumentDiscardSearchResults { .. } => Some("DocumentDiscardSearchResults"),
            Self::DocumentFrontendNodeBinding { .. } => Some("DocumentFrontendNodeBinding"),
            Self::RegisterDocumentBidiNodeBinding { .. } => Some("RegisterDocumentBidiNodeBinding"),
            Self::DocumentBidiNodeBinding { .. } => Some("DocumentBidiNodeBinding"),
            Self::DocumentBidiNodeSharedIdForBackendNodeId { .. } => {
                Some("DocumentBidiNodeSharedIdForBackendNodeId")
            }
            Self::DocumentNodeAttributes { .. } => Some("DocumentNodeAttributes"),
            Self::DocumentNodeText { .. } => Some("DocumentNodeText"),
            Self::DocumentNodeProperty { .. } => Some("DocumentNodeProperty"),
            Self::AccessibilityTreePayloadsForDocument { .. } => {
                Some("AccessibilityTreePayloadsForDocument")
            }
            Self::AccessibilityNodePayloadForDocument => {
                Some("AccessibilityNodePayloadForDocument")
            }
            Self::AccessibilityTreePayloadsForNode { .. } => {
                Some("AccessibilityTreePayloadsForNode")
            }
            Self::AccessibilityNodePayloadForNode { .. } => Some("AccessibilityNodePayloadForNode"),
            Self::AccessibilityNodeAndAncestorPayloadsForNode { .. } => {
                Some("AccessibilityNodeAndAncestorPayloadsForNode")
            }
            Self::AccessibilityChildNodePayloadsForNode { .. } => {
                Some("AccessibilityChildNodePayloadsForNode")
            }
            Self::AccessibilityPartialTreePayloadsForNode { .. } => {
                Some("AccessibilityPartialTreePayloadsForNode")
            }
            Self::AccessibilityTreePayloadsForChildFrame { .. } => {
                Some("AccessibilityTreePayloadsForChildFrame")
            }
            Self::AccessibilityNodePayloadForChildFrame { .. } => {
                Some("AccessibilityNodePayloadForChildFrame")
            }
            Self::CaptureScreencastFrame(_) => Some("CaptureScreencastFrame"),
            Self::ScrollNodeIntoViewIfNeeded { .. } => Some("ScrollNodeIntoViewIfNeeded"),
            Self::ClientRectForBackendNodeId { .. } => Some("ClientRectForBackendNodeId"),
            Self::DocumentGeometryForNode { .. } => Some("DocumentGeometryForNode"),
            Self::SetOverlayHighlight { .. } => Some("SetOverlayHighlight"),
            Self::DocumentHitTest { .. } => Some("DocumentHitTest"),
            Self::NodeHasGeometryForBackendNodeId { .. } => Some("NodeHasGeometryForBackendNodeId"),
            Self::RemoveDocumentNode { .. } => Some("RemoveDocumentNode"),
            Self::EditDocumentNode { .. } => Some("EditDocumentNode"),
            Self::TriggerAutofill(_) => Some("TriggerAutofill"),
            Self::WebMcp { .. } => Some("WebMCP"),
            Self::ComputedStyleProperties { .. } => Some("ComputedStyleProperties"),
            Self::SetFileInputFilesForBackendNodeId { .. } => {
                Some("SetFileInputFilesForBackendNodeId")
            }
            Self::DocumentFrontendNodeIdsForBackendNodeIds { .. } => {
                Some("DocumentFrontendNodeIdsForBackendNodeIds")
            }
            Self::EnsureIsolatedWorldsAttachedToInspector => {
                Some("EnsureIsolatedWorldsAttachedToInspector")
            }
            _ => None,
        }
    }
}

impl RendererPageCommand {
    fn inspector_command(
        inspector_session_id: Option<String>,
        command: RendererInspectorPageCommand,
    ) -> Self {
        Self::DevTools(RendererDevToolsPageCommand::Inspector(
            RendererInspectorCommandEnvelope::new(inspector_session_id, command),
        ))
    }

    pub fn dispatch_runtime_protocol_message(
        inspector_session_id: Option<String>,
        raw_json: String,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessage { raw_json },
        )
    }

    pub fn dispatch_runtime_protocol_message_with_deferred_response(
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithDeferredResponse {
                raw_json,
                deferred_response,
            },
        )
    }

    pub fn dispatch_runtime_protocol_message_with_context_resolution(
        inspector_session_id: Option<String>,
        action: String,
        raw_json: String,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolution {
                action,
                raw_json,
            },
        )
    }

    pub fn dispatch_runtime_protocol_message_with_context_resolution_and_deferred_response(
        inspector_session_id: Option<String>,
        action: String,
        raw_json: String,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse {
                action,
                raw_json,
                deferred_response,
            },
        )
    }

    pub fn runtime_enable_events(inspector_session_id: Option<String>) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::RuntimeEnableEvents,
        )
    }

    pub fn runtime_disable_events(inspector_session_id: Option<String>) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::RuntimeDisableEvents,
        )
    }

    pub fn runtime_binding_events(
        inspector_session_id: Option<String>,
        binding: crate::protocol_types::RuntimeBindingRegistration,
        remove: bool,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::RuntimeBindingEvents { binding, remove },
        )
    }

    pub fn apply_runtime_protocol_state(
        inspector_session_id: Option<String>,
        session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        isolated_worlds: Vec<crate::protocol_types::RuntimeIsolatedWorldDefinition>,
        stored_runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
        session_runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::ApplyRuntimeProtocolState {
                session_restore_snapshots,
                isolated_worlds,
                stored_runtime_bindings,
                session_runtime_bindings,
            },
        )
    }

    pub fn detach_runtime_inspector_session(
        inspector_session_id: Option<String>,
        pause_guard: RendererRuntimeInspectorSessionDetachGuard,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::DetachRuntimeInspectorSession { pause_guard },
        )
    }

    pub fn add_runtime_binding(
        inspector_session_id: Option<String>,
        name: String,
        execution_context_name: Option<String>,
        execution_context_id: Option<i64>,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::AddRuntimeBinding {
                name,
                execution_context_name,
                execution_context_id,
            },
        )
    }

    pub fn dom_debugger_get_event_listeners(
        inspector_session_id: Option<String>,
        object_id: String,
        depth: i32,
        pierce: bool,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::DomDebuggerGetEventListeners {
                object_id,
                depth,
                pierce,
            },
        )
    }

    pub fn computed_style_properties_for_object_id(
        inspector_session_id: Option<String>,
        object_id: String,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::ComputedStylePropertiesForObjectId { object_id },
        )
    }

    pub fn prepare_element_click(inspector_session_id: Option<String>, object_id: String) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::PrepareElementClick { object_id },
        )
    }

    pub fn scroll_object_node_into_view_if_needed(
        inspector_session_id: Option<String>,
        object_id: String,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::ScrollObjectNodeIntoViewIfNeeded { object_id, rect },
        )
    }

    pub fn client_rect_for_object_id(
        inspector_session_id: Option<String>,
        object_id: String,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::ClientRectForObjectId { object_id },
        )
    }

    pub fn document_geometry_for_object_id(
        inspector_session_id: Option<String>,
        object_id: String,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::DocumentGeometryForObjectId { object_id },
        )
    }

    pub fn node_has_geometry_for_object_id(
        inspector_session_id: Option<String>,
        object_id: String,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::NodeHasGeometryForObjectId { object_id },
        )
    }

    pub fn focus_document_node_for_object_id(
        inspector_session_id: Option<String>,
        object_id: String,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::FocusDocumentNodeForObjectId { object_id },
        )
    }

    pub fn set_file_input_files_for_object_id(
        inspector_session_id: Option<String>,
        object_id: String,
        files: Vec<crate::dom::native::SelectedFile>,
        append: bool,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::SetFileInputFilesForObjectId {
                object_id,
                files,
                append,
            },
        )
    }

    pub fn document_node_snapshot_for_object_id(
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        object_id: String,
        depth: i32,
        pierce: bool,
        bind_frontend_nodes: bool,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::DocumentNodeSnapshotForObjectId {
                include_whitespace,
                object_id,
                depth,
                pierce,
                bind_frontend_nodes,
            },
        )
    }

    pub fn request_document_node_for_object_id(
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        object_id: String,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::RequestDocumentNodeForObjectId {
                include_whitespace,
                object_id,
            },
        )
    }

    pub fn accessibility_tree_payloads_for_object_id(
        inspector_session_id: Option<String>,
        object_id: String,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::AccessibilityTreePayloadsForObjectId { object_id },
        )
    }

    pub fn accessibility_node_and_ancestor_payloads_for_object_id(
        inspector_session_id: Option<String>,
        object_id: String,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::AccessibilityNodeAndAncestorPayloadsForObjectId {
                object_id,
            },
        )
    }

    pub fn accessibility_partial_tree_payloads_for_object_id(
        inspector_session_id: Option<String>,
        object_id: String,
        fetch_relatives: bool,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::AccessibilityPartialTreePayloadsForObjectId {
                object_id,
                fetch_relatives,
            },
        )
    }

    pub fn outer_html_for_object_id(
        inspector_session_id: Option<String>,
        object_id: String,
        include_shadow_dom: bool,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::OuterHtmlForObjectId {
                object_id,
                include_shadow_dom,
            },
        )
    }

    pub fn resolve_runtime_object_for_backend_node_id(
        inspector_session_id: Option<String>,
        backend_node_id: u32,
        execution_context_id: Option<i64>,
        object_group: Option<String>,
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::ResolveRuntimeObjectForBackendNodeId {
                backend_node_id,
                execution_context_id,
                object_group,
            },
        )
    }

    pub fn resolve_blob_object(inspector_session_id: Option<String>, object_id: String) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::ResolveBlobObject { object_id },
        )
    }

    /// Metadata is structurally present for every command that can access a
    /// frontend V8 Inspector session; no command-variant allowlist is involved.
    #[cfg(test)]
    pub(crate) fn inspector_ticket(&self) -> Option<&RendererInspectorIngressTicket> {
        match self {
            Self::DevTools(RendererDevToolsPageCommand::Inspector(envelope)) => {
                Some(envelope.ticket())
            }
            _ => None,
        }
    }

    pub fn bind_inspector_attachment(&mut self, attachment: RendererAgentAttachmentId) {
        if let Self::DevTools(RendererDevToolsPageCommand::Native(command)) = self {
            command
                .operation
                .command
                .bind_inspector_attachment(attachment);
            return;
        }
        if let Self::DevTools(RendererDevToolsPageCommand::Inspector(envelope)) = self {
            envelope.bind_attachment(attachment);
        }
    }
}

#[cfg(test)]
mod renderer_inspector_command_envelope_tests {
    use super::*;

    fn assert_ticket(
        command: &RendererPageCommand,
        session: DevToolsSessionKey,
        route: RendererInspectorCommandRoute,
    ) {
        let ticket = command
            .inspector_ticket()
            .expect("every frontend V8 Inspector operation must carry an ingress ticket");
        assert_eq!(ticket.session(), &session);
        assert_eq!(ticket.route(), route);
        assert!(ticket.sequence() > 0);
    }

    #[test]
    fn raw_special_and_runtime_object_commands_share_one_inspector_boundary() {
        let attached = Some("SID-envelope".to_owned());
        let main_thread_commands = [
            RendererPageCommand::runtime_enable_events(attached.clone()),
            RendererPageCommand::apply_runtime_protocol_state(
                attached.clone(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            RendererPageCommand::add_runtime_binding(
                attached.clone(),
                "binding".to_owned(),
                None,
                None,
            ),
            RendererPageCommand::computed_style_properties_for_object_id(
                attached,
                "runtime-object".to_owned(),
            ),
        ];
        for command in &main_thread_commands {
            assert_ticket(
                command,
                DevToolsSessionKey::Attached("SID-envelope".to_owned()),
                RendererInspectorCommandRoute::MainThread,
            );
        }

        assert!(
            RendererPageCommand::PageDiagnosticsSnapshot
                .inspector_ticket()
                .is_none(),
            "ordinary Page commands must remain outside Inspector lanes"
        );
    }

    #[test]
    fn empty_wire_session_id_normalizes_to_the_primary_session() {
        let command = RendererPageCommand::runtime_enable_events(Some(String::new()));
        assert_ticket(
            &command,
            DevToolsSessionKey::Primary,
            RendererInspectorCommandRoute::MainThread,
        );
    }

    #[test]
    fn ingress_tickets_bind_one_attachment_and_allocate_monotonic_sequences() {
        let mut first = RendererPageCommand::runtime_enable_events(Some("SID-first".to_owned()));
        let second = RendererPageCommand::runtime_enable_events(Some("SID-second".to_owned()));
        let first_sequence = first
            .inspector_ticket()
            .expect("first Inspector ticket")
            .sequence();
        let second_sequence = second
            .inspector_ticket()
            .expect("second Inspector ticket")
            .sequence();
        assert!(second_sequence > first_sequence);

        let attachment = RendererAgentAttachmentId::allocate();
        first.bind_inspector_attachment(attachment);
        first.bind_inspector_attachment(attachment);
        assert_eq!(
            first
                .inspector_ticket()
                .expect("bound Inspector ticket")
                .attachment(),
            Some(attachment)
        );
    }

    #[test]
    #[should_panic(expected = "cannot be retargeted to another attachment")]
    fn ingress_ticket_cannot_be_retargeted_during_navigation() {
        let mut command = RendererPageCommand::runtime_enable_events(None);
        command.bind_inspector_attachment(RendererAgentAttachmentId::allocate());
        command.bind_inspector_attachment(RendererAgentAttachmentId::allocate());
    }
}
