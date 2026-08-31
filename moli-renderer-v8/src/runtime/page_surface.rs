use super::*;
use crate::devtools::ingress::main::RendererInspectorMainFirstDispatchGuard;
use crate::native_bridge::{
    PendingRuntimeObservableConsoleSourceEvent, RuntimeObservableContextToken,
};
use crate::page_task_queue::RendererOwnerWakeSender;
use crate::protocol_types::{
    ChildFrameDocumentOpenedSnapshot, ChildFrameTreeEventSnapshot, ChildFrameTreeSnapshot,
};
use crate::types::{ScriptObservableOutput, ScriptObservableOutputItem};
use anyhow::bail;
pub use moli_page_types::{
    DevToolsSessionKey, DocumentNodeObjectSnapshot, DocumentNodeSnapshot,
    RendererAgentAttachmentId, RendererDevToolsAgentToken,
    RendererDomDebuggerEventListenerBreakpoint, RendererDomDebuggerXhrBreakpoint,
    RendererInspectorProtocolConfiguration, RendererInspectorProtocolConfigurationCommand,
    RendererInspectorSessionRestoreSnapshot, SameDocumentHistoryUpdate, V8InspectorSessionState,
};
use moli_shared_worker::SharedWorkerInstanceId;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};
use tokio::sync::oneshot;

mod javascript_dialog;
mod popup_activation;
mod window_document_source;
pub use javascript_dialog::{
    RendererJavaScriptDialogId, RendererJavaScriptDialogSource, RendererPendingJavaScriptDialog,
};
pub use popup_activation::{
    RendererPendingPopupActivation, RendererPopupActivationSource, RendererPopupDisposition,
};
pub use window_document_source::RendererWindowDocumentSource;

#[derive(Debug, Clone)]
pub struct RendererPageView {
    pub page_id: PageId,
    pub vm_creation_id: u64,
    pub view_generation: u64,
    pub page_state: Arc<RendererPageState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererPendingDownloadResponse {
    pub final_url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererSyntheticResponseBody {
    text: String,
    bytes: Vec<u8>,
}

impl RendererSyntheticResponseBody {
    pub fn empty() -> Self {
        Self {
            text: String::new(),
            bytes: Vec::new(),
        }
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        Self { text, bytes }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Clones the exact synthetic payload for protocol/body-capture boundaries.
    pub fn clone_body_bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    /// Consumes the synthetic payload when the caller owns the body and needs
    /// exact bytes for a protocol or replay boundary.
    pub fn into_body_bytes(self) -> Vec<u8> {
        self.bytes
    }

    fn into_response_body(self) -> moli_fetch::ResponseBody {
        moli_fetch::ResponseBody::materialized_text(self.text, self.bytes)
    }

    fn clone_response_body(&self) -> moli_fetch::ResponseBody {
        moli_fetch::ResponseBody::materialized_text(self.text.clone(), self.bytes.clone())
    }

    /// Converts a synthetic fulfill body into the renderer-neutral subresource
    /// record body without reopening a loose text/byte pair at each call site.
    pub fn into_subresource_response_body(self) -> crate::protocol_types::SubresourceResponseBody {
        crate::protocol_types::SubresourceResponseBody::from_materialized_body(
            self.into_response_body(),
        )
    }

    /// Builds a materialized fetch response for worker/Web compatibility paths
    /// that still expose a complete `Response` value.
    pub fn into_fetch_response(self, head: moli_fetch::ResponseHead) -> moli_fetch::Response {
        moli_fetch::Response::from_head_and_materialized_body(head, self.into_response_body())
            .expect("synthetic fetch response body should remain materialized")
    }

    /// Builds a materialized navigation response for compatibility paths that
    /// still need complete text and bytes before the next renderer step.
    pub fn into_navigation_response(
        self,
        head: moli_fetch::ResponseHead,
    ) -> crate::protocol_types::NavigationResponse {
        crate::protocol_types::NavigationResponse::from_head_and_materialized_body(
            head,
            self.into_response_body(),
        )
    }

    /// Clones a materialized navigation response while retaining the synthetic
    /// body for another owner, such as a worker fulfill path.
    pub fn clone_as_navigation_response(
        &self,
        head: moli_fetch::ResponseHead,
    ) -> crate::protocol_types::NavigationResponse {
        crate::protocol_types::NavigationResponse::from_head_and_materialized_body(
            head,
            self.clone_response_body(),
        )
    }

    /// Compatibility alias for callers that still use the older materializing
    /// name. Prefer `clone_as_navigation_response` when the synthetic body is
    /// intentionally retained for a second owner.
    pub fn to_navigation_response(
        &self,
        head: moli_fetch::ResponseHead,
    ) -> crate::protocol_types::NavigationResponse {
        self.clone_as_navigation_response(head)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererPendingDownloadActivation {
    pub url: String,
    pub suggested_filename: Option<String>,
    pub response: Option<RendererPendingDownloadResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererPendingFileChooserActivation {
    source_document: RendererDocumentLifecycleIdentity,
    pub frame_id: Option<String>,
    pub backend_node_id: u32,
    pub node_id: Option<moli_dom::NodeId>,
    pub allow_multiple: bool,
    source_node: Option<RendererPendingFileChooserNodeSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RendererPendingFileChooserNodeSource {
    handle: crate::document_runtime::DomHandle,
    document_id: crate::frame_owner_model::DocumentId,
}

impl RendererPendingFileChooserActivation {
    /// Creates a file-chooser handoff at the element activation boundary.
    ///
    /// `source_document` is the exact root Document residence that initiated
    /// activation. It remains causal metadata when a listener synchronously
    /// calls `document.open()`. `frame_id` is `None` for the root frame and a
    /// concrete renderer frame id for a child activation.
    pub fn new(
        source_document: RendererDocumentLifecycleIdentity,
        frame_id: Option<String>,
        backend_node_id: u32,
        allow_multiple: bool,
    ) -> Self {
        Self {
            source_document,
            frame_id,
            backend_node_id,
            node_id: None,
            allow_multiple,
            source_node: None,
        }
    }

    pub(crate) fn from_live_node(
        source_document: RendererDocumentLifecycleIdentity,
        frame_id: Option<String>,
        allow_multiple: bool,
        handle: crate::document_runtime::DomHandle,
        document_id: crate::frame_owner_model::DocumentId,
    ) -> Self {
        Self {
            source_document,
            frame_id,
            backend_node_id: 0,
            node_id: Some(handle),
            allow_multiple,
            source_node: Some(RendererPendingFileChooserNodeSource {
                handle,
                document_id,
            }),
        }
    }

    pub(crate) fn live_node_source(
        &self,
    ) -> Option<(
        crate::document_runtime::DomHandle,
        crate::frame_owner_model::DocumentId,
    )> {
        self.source_node
            .map(|source| (source.handle, source.document_id))
    }

    pub fn source_document(&self) -> RendererDocumentLifecycleIdentity {
        self.source_document
    }

    pub fn source_frame_id(&self) -> Option<&str> {
        self.frame_id.as_deref()
    }

    pub fn backend_node_id(&self) -> u32 {
        self.backend_node_id
    }

    pub fn allow_multiple(&self) -> bool {
        self.allow_multiple
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererPendingSameDocumentNavigation {
    pub url: String,
    pub navigation_type: String,
    pub history_update: SameDocumentHistoryUpdate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RendererPendingTopLevelHistoryTraversal {
    pub delta: i64,
}

/// A same-Document navigation captured together with the exact Document that
/// produced it.
///
/// `RendererPendingSameDocumentNavigation` is the DocumentRuntime-local body
/// record. The source identity is attached at mutation time and remains causal
/// metadata if `document.open()` replaces the Document before protocol capture.
/// Protocol application must bind the payload to the target's Page residence;
/// source-Document currentness is not an authorization rule because
/// same-Document history mutation survives `document.open()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererDocumentSourcedSameDocumentNavigation {
    source_document: RendererDocumentLifecycleIdentity,
    navigation: RendererPendingSameDocumentNavigation,
}

impl RendererDocumentSourcedSameDocumentNavigation {
    pub fn new(
        source_document: RendererDocumentLifecycleIdentity,
        navigation: RendererPendingSameDocumentNavigation,
    ) -> Self {
        Self {
            source_document,
            navigation,
        }
    }

    pub fn source_document(&self) -> RendererDocumentLifecycleIdentity {
        self.source_document
    }

    pub fn navigation(&self) -> &RendererPendingSameDocumentNavigation {
        &self.navigation
    }

    pub fn into_navigation(self) -> RendererPendingSameDocumentNavigation {
        self.navigation
    }
}

/// A non-JavaScript top-level location navigation captured with its exact
/// source Document.
///
/// The URL, method, raw body bytes, and explicit headers describe the requested
/// navigation, while `source_document` records which Document requested it.
/// Keeping the request whole is required for top-level form POSTs; reducing the
/// handoff to a URL would silently turn them into GETs. The request survives a
/// `document.open()` in the same Page, so protocol application authorizes it
/// against the target-local Page residence rather than requiring this Document
/// to remain current.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererDocumentSourcedTopLevelLocationNavigation {
    source_document: RendererDocumentLifecycleIdentity,
    request: Box<RendererTopLevelNavigationRequest>,
    runtime_command_cause: Option<RendererRuntimeCommandCausalIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RendererTopLevelNavigationRequest {
    url: String,
    request_method: String,
    request_body: Option<Vec<u8>>,
    request_headers: Vec<(String, String)>,
    browser_navigation_kind: moli_fetch::BrowserNavigationRequestKind,
}

impl RendererDocumentSourcedTopLevelLocationNavigation {
    pub fn new(source_document: RendererDocumentLifecycleIdentity, url: String) -> Self {
        Self::new_with_runtime_command_cause(source_document, url, None)
    }

    pub fn new_with_runtime_command_cause(
        source_document: RendererDocumentLifecycleIdentity,
        url: String,
        runtime_command_cause: Option<RendererRuntimeCommandCausalIdentity>,
    ) -> Self {
        Self::new_with_request_and_runtime_command_cause(
            source_document,
            url,
            "GET".to_owned(),
            None,
            Vec::new(),
            moli_fetch::BrowserNavigationRequestKind::Navigate,
            runtime_command_cause,
        )
    }

    pub fn new_with_request_and_runtime_command_cause(
        source_document: RendererDocumentLifecycleIdentity,
        url: String,
        request_method: String,
        request_body: Option<Vec<u8>>,
        request_headers: Vec<(String, String)>,
        browser_navigation_kind: moli_fetch::BrowserNavigationRequestKind,
        runtime_command_cause: Option<RendererRuntimeCommandCausalIdentity>,
    ) -> Self {
        Self {
            source_document,
            request: Box::new(RendererTopLevelNavigationRequest {
                url,
                request_method,
                request_body,
                request_headers,
                browser_navigation_kind,
            }),
            runtime_command_cause,
        }
    }

    pub fn source_document(&self) -> RendererDocumentLifecycleIdentity {
        self.source_document
    }

    pub fn url(&self) -> &str {
        &self.request.url
    }

    pub fn request_method(&self) -> &str {
        &self.request.request_method
    }

    pub fn request_body(&self) -> Option<&[u8]> {
        self.request.request_body.as_deref()
    }

    pub fn request_headers(&self) -> &[(String, String)] {
        &self.request.request_headers
    }

    pub fn browser_navigation_kind(&self) -> moli_fetch::BrowserNavigationRequestKind {
        self.request.browser_navigation_kind
    }

    pub fn runtime_command_cause(&self) -> Option<&RendererRuntimeCommandCausalIdentity> {
        self.runtime_command_cause.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererDragDataItem {
    pub mime_type: String,
    pub data: String,
    pub title: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RendererDraggedFile {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    pub name: String,
    pub last_modified: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RendererDraggedDirectory {
    pub name: String,
    pub files: Vec<RendererDraggedFile>,
    pub directories: Vec<RendererDraggedDirectory>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RendererDragData {
    pub items: Vec<RendererDragDataItem>,
    pub files: Vec<RendererDraggedFile>,
    pub directories: Vec<RendererDraggedDirectory>,
    pub drag_operations_mask: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererInputDispatchOutcome {
    pub handled: bool,
    pub triggered_top_level_navigation: bool,
    pub pending_download: Option<RendererPendingDownloadActivation>,
    pub pending_file_chooser: Option<RendererPendingFileChooserActivation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RendererPointerEventProperties {
    pub pointer_id: i32,
    pub pointer_type: String,
    pub pressure: f64,
    pub tangential_pressure: f64,
    pub tilt_x: f64,
    pub tilt_y: f64,
    pub twist: f64,
}

impl Default for RendererPointerEventProperties {
    fn default() -> Self {
        Self {
            pointer_id: 1,
            pointer_type: "mouse".to_owned(),
            pressure: 0.0,
            tangential_pressure: 0.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            twist: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RendererTouchPoint {
    pub id: i32,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererPendingWindowOpenEvent {
    pub url: String,
    pub window_name: String,
    pub window_features: Vec<String>,
    pub user_gesture: bool,
}

impl RendererPendingWindowOpenEvent {
    pub fn browser_window(url: &str, window_name: &str, user_gesture: bool) -> Self {
        Self {
            url: url.to_owned(),
            window_name: if window_name.is_empty() {
                "_blank".to_owned()
            } else {
                window_name.to_owned()
            },
            window_features: ["menubar", "toolbar", "status", "scrollbars", "resizable"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            user_gesture,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererSharedWorkerTargetInfo {
    pub owner_local_host_id: super::RendererOwnerLocalHostId,
    pub instance_id: SharedWorkerInstanceId,
    pub url: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererSharedWorkerConsoleMessage {
    pub message: String,
    pub args: Vec<Value>,
    pub stack: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RendererSharedWorkerTargetEvent {
    Created(RendererSharedWorkerTargetInfo),
    Destroyed {
        instance_id: SharedWorkerInstanceId,
    },
    Console {
        instance_id: SharedWorkerInstanceId,
        message: RendererSharedWorkerConsoleMessage,
    },
    RuntimeInspectorMessages {
        instance_id: SharedWorkerInstanceId,
        inspector_session_id: Option<String>,
        messages: Vec<RendererRuntimeInspectorMessage>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererDedicatedWorkerTargetInfo {
    pub owner_local_host_id: super::RendererOwnerLocalHostId,
    pub page_id: super::PageId,
    pub instance_id: u64,
    pub request_url: String,
    pub document_url: String,
    pub name: String,
}

/// Page-owned lifecycle facts for one DedicatedWorker target.
///
/// Chromium publishes the initial main-script request through the creator
/// Page's Network agent, but publishes the response and terminal event through
/// the Worker target's Network agent. Keeping the response on this target
/// event stream lets protocol preserve that split without manufacturing a
/// complete Page subresource record.
#[derive(Debug, Clone)]
pub enum RendererDedicatedWorkerTargetEvent {
    Created(RendererDedicatedWorkerTargetInfo),
    ScriptLoaded {
        instance_id: u64,
        script_url: String,
        response: Box<crate::protocol_types::NavigationResponse>,
    },
    ScriptLoadFailed {
        instance_id: u64,
        script_url: String,
        error_message: String,
        response: Option<Box<crate::protocol_types::NavigationResponse>>,
    },
    Console {
        instance_id: u64,
        message: RendererSharedWorkerConsoleMessage,
    },
    RuntimeInspectorMessages {
        instance_id: u64,
        inspector_session_id: Option<String>,
        messages: Vec<RendererRuntimeInspectorMessage>,
    },
    Destroyed {
        instance_id: u64,
    },
}

impl PartialEq for RendererDedicatedWorkerTargetEvent {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Created(left), Self::Created(right)) => left == right,
            (
                Self::ScriptLoaded {
                    instance_id: left_instance,
                    script_url: left_url,
                    response: left_response,
                },
                Self::ScriptLoaded {
                    instance_id: right_instance,
                    script_url: right_url,
                    response: right_response,
                },
            ) => {
                left_instance == right_instance
                    && left_url == right_url
                    && dedicated_worker_navigation_response_eq(left_response, right_response)
            }
            (
                Self::ScriptLoadFailed {
                    instance_id: left_instance,
                    script_url: left_url,
                    error_message: left_error,
                    response: left_response,
                },
                Self::ScriptLoadFailed {
                    instance_id: right_instance,
                    script_url: right_url,
                    error_message: right_error,
                    response: right_response,
                },
            ) => {
                left_instance == right_instance
                    && left_url == right_url
                    && left_error == right_error
                    && match (left_response, right_response) {
                        (Some(left), Some(right)) => {
                            dedicated_worker_navigation_response_eq(left, right)
                        }
                        (None, None) => true,
                        _ => false,
                    }
            }
            (
                Self::Console {
                    instance_id: left_instance,
                    message: left_message,
                },
                Self::Console {
                    instance_id: right_instance,
                    message: right_message,
                },
            ) => left_instance == right_instance && left_message == right_message,
            (
                Self::RuntimeInspectorMessages {
                    instance_id: left_instance,
                    inspector_session_id: left_session,
                    messages: left_messages,
                },
                Self::RuntimeInspectorMessages {
                    instance_id: right_instance,
                    inspector_session_id: right_session,
                    messages: right_messages,
                },
            ) => {
                left_instance == right_instance
                    && left_session == right_session
                    && left_messages == right_messages
            }
            (
                Self::Destroyed {
                    instance_id: left_instance,
                },
                Self::Destroyed {
                    instance_id: right_instance,
                },
            ) => left_instance == right_instance,
            _ => false,
        }
    }
}

fn dedicated_worker_navigation_response_eq(
    left: &crate::protocol_types::NavigationResponse,
    right: &crate::protocol_types::NavigationResponse,
) -> bool {
    left.final_url == right.final_url
        && left.status == right.status
        && left.headers == right.headers
        && left.body_bytes() == right.body_bytes()
        && left.request_cookie_report == right.request_cookie_report
        && left.cookie_set_reports == right.cookie_set_reports
        && left.redirected == right.redirected
        && left.redirect_chain == right.redirect_chain
        && left.from_cache == right.from_cache
        && left.negotiated_http_version == right.negotiated_http_version
        && left.network_request_headers() == right.network_request_headers()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererServiceWorkerTargetInfo {
    pub registration_id: u64,
    pub version_id: u64,
    pub script_url: String,
    pub scope_url: String,
    pub status: RendererServiceWorkerVersionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererServiceWorkerVersionStatus {
    New,
    Installing,
    Installed,
    Activating,
    Activated,
    Redundant,
}

impl RendererServiceWorkerVersionStatus {
    pub fn as_cdp_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Installing => "installing",
            Self::Installed => "installed",
            Self::Activating => "activating",
            Self::Activated => "activated",
            Self::Redundant => "redundant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererServiceWorkerConsoleMessage {
    pub message: String,
    pub args: Vec<Value>,
    pub stack: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererServiceWorkerExceptionMessage {
    pub message: String,
    pub filename: String,
    pub lineno: u32,
    pub colno: u32,
    pub event_kind: String,
    pub phase: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererServiceWorkerFetchDiagnostic {
    pub internal_id: u64,
    pub document_url: String,
    pub request_url: String,
    pub method: String,
    pub request_headers: Vec<(String, String)>,
    pub request_body: Option<String>,
    pub destination: String,
    pub result: RendererServiceWorkerFetchDiagnosticResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererServiceWorkerFetchDiagnosticResult {
    Fallback,
    Response {
        final_url: String,
        status: u16,
        status_text: String,
        response_headers: Vec<(String, String)>,
        body_len: usize,
    },
    Failure {
        message: String,
    },
}

/// Renderer facts for one stable ServiceWorker version target.
///
/// `version_id` survives worker restarts. Run-specific facts carry the opaque
/// identity created by the renderer's ServiceWorker runtime authority;
/// protocol consumers must never reconstruct a run from a scalar generation.
/// `VersionUpdated` never manufactures a run. `Destroyed` is a version-level
/// terminal but snapshots the exact active run, when one exists, so a delayed
/// terminal cannot retire a restarted worker beneath the same version id.
#[derive(Debug, Clone, PartialEq)]
pub enum RendererServiceWorkerTargetEvent {
    Created {
        info: RendererServiceWorkerTargetInfo,
        /// Exact run already owned by a live worker host when this stable
        /// version target is first exposed. Restored stopped versions carry
        /// `None`; target creation alone must never manufacture a run.
        active_run: Option<super::RendererServiceWorkerRunIdentity>,
    },
    Started {
        version_id: u64,
        run: super::RendererServiceWorkerRunIdentity,
    },
    Stopped {
        version_id: u64,
        run: super::RendererServiceWorkerRunIdentity,
        reason: String,
    },
    Destroyed {
        version_id: u64,
        active_run: Option<super::RendererServiceWorkerRunIdentity>,
    },
    VersionUpdated {
        version_id: u64,
        status: RendererServiceWorkerVersionStatus,
    },
    Console {
        version_id: u64,
        run: super::RendererServiceWorkerRunIdentity,
        message: RendererServiceWorkerConsoleMessage,
    },
    Exception {
        version_id: u64,
        run: super::RendererServiceWorkerRunIdentity,
        message: RendererServiceWorkerExceptionMessage,
    },
    FetchDiagnostic {
        version_id: u64,
        run: super::RendererServiceWorkerRunIdentity,
        diagnostic: RendererServiceWorkerFetchDiagnostic,
    },
    RuntimeInspectorMessages {
        version_id: u64,
        run: super::RendererServiceWorkerRunIdentity,
        inspector_session_id: Option<String>,
        messages: Vec<RendererRuntimeInspectorMessage>,
    },
}

/// Frozen main-Document commit fact inserted into the Page output FIFO.
///
/// Chromium resets the old V8 contexts, commits the replacement LocalFrame,
/// and only then reports the new default context. Moli constructs the
/// replacement Page off the protocol lane, so this record carries the
/// protocol-neutral navigation identity needed to preserve the same boundary
/// without making protocol splice events between two renderer cursors.
#[derive(Clone, Debug, PartialEq)]
pub struct RendererMainDocumentCommit {
    pub frame_id: String,
    pub loader_id: String,
    pub url: String,
    pub unreachable_url: Option<String>,
    pub security_origin: String,
    pub secure_context_type: String,
    pub timestamp: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RendererPageCreationDiagnostics {
    pub initial_runtime_realms: Vec<RendererRuntimeRealmInfo>,
    pub renderer_output_predecessor: Option<RendererOutputFence>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RendererRuntimeInspectorMessage {
    RuntimeContext(crate::protocol_types::RuntimeContextRestoreEvent),
    Protocol(RendererRuntimeInspectorProtocolMessage),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RendererRuntimeInspectorProtocolMessage {
    renderer_call_id: Option<moli_page_types::RendererCallId>,
    value: Value,
}

pub struct RendererRuntimeInspectorProtocolMessageValueMut<'a> {
    message: &'a mut RendererRuntimeInspectorProtocolMessage,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RendererRuntimeCommandOutput {
    renderer_agent_attachment_id: Option<RendererAgentAttachmentId>,
    v8_state_update: Option<V8InspectorSessionState>,
    messages: Vec<RendererRuntimeInspectorMessage>,
}

impl RendererRuntimeCommandOutput {
    pub fn from_messages(messages: Vec<RendererRuntimeInspectorMessage>) -> Self {
        Self {
            renderer_agent_attachment_id: None,
            v8_state_update: None,
            messages,
        }
    }

    pub fn from_inspector_message(message: RendererRuntimeInspectorMessage) -> Self {
        Self {
            renderer_agent_attachment_id: None,
            v8_state_update: None,
            messages: vec![message],
        }
    }

    pub fn from_parts(
        renderer_agent_attachment_id: Option<RendererAgentAttachmentId>,
        v8_state_update: Option<V8InspectorSessionState>,
        messages: Vec<RendererRuntimeInspectorMessage>,
    ) -> Self {
        Self {
            renderer_agent_attachment_id,
            v8_state_update,
            messages,
        }
    }

    pub fn messages(&self) -> &[RendererRuntimeInspectorMessage] {
        &self.messages
    }

    pub fn messages_mut(&mut self) -> &mut [RendererRuntimeInspectorMessage] {
        &mut self.messages
    }

    pub fn into_messages(self) -> Vec<RendererRuntimeInspectorMessage> {
        self.messages
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn into_parts(
        self,
    ) -> (
        Option<RendererAgentAttachmentId>,
        Option<V8InspectorSessionState>,
        Vec<RendererRuntimeInspectorMessage>,
    ) {
        (
            self.renderer_agent_attachment_id,
            self.v8_state_update,
            self.messages,
        )
    }

    pub fn renderer_agent_attachment_id(&self) -> Option<RendererAgentAttachmentId> {
        self.renderer_agent_attachment_id
    }

    pub fn v8_state_update(&self) -> Option<&V8InspectorSessionState> {
        self.v8_state_update.as_ref()
    }

    pub(crate) fn set_v8_state_update(&mut self, state: V8InspectorSessionState) {
        self.v8_state_update = Some(state);
    }

    #[doc(hidden)]
    pub fn bind_renderer_agent_attachment(&mut self, id: RendererAgentAttachmentId) {
        match self.renderer_agent_attachment_id {
            Some(current) => assert_eq!(
                current, id,
                "renderer command output cannot change attachment identity"
            ),
            None => self.renderer_agent_attachment_id = Some(id),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn protocol_response(&self, call_id: i32) -> Option<&Value> {
        self.messages.iter().find_map(|message| match message {
            RendererRuntimeInspectorMessage::Protocol(message)
                if message
                    .renderer_call_id()
                    .is_some_and(|id| id.get() == call_id) =>
            {
                Some(message.value())
            }
            _ => None,
        })
    }

    pub fn into_protocol_response(self, call_id: i32) -> Option<Value> {
        self.messages.into_iter().find_map(|message| match message {
            RendererRuntimeInspectorMessage::Protocol(message)
                if message
                    .renderer_call_id()
                    .is_some_and(|id| id.get() == call_id) =>
            {
                Some(message.into_value())
            }
            _ => None,
        })
    }

    pub fn extend_messages(
        &mut self,
        messages: impl IntoIterator<Item = RendererRuntimeInspectorMessage>,
    ) {
        self.messages.extend(messages);
    }

    pub(crate) fn push_inspector_message(&mut self, message: Value) {
        self.messages
            .push(RendererRuntimeInspectorMessage::from_v8_inspector_message(
                message,
            ));
    }

    pub(crate) fn append(&mut self, mut other: Self) {
        if let Some(id) = other.renderer_agent_attachment_id {
            self.bind_renderer_agent_attachment(id);
        }
        if let Some(state) = other.v8_state_update.take() {
            self.set_v8_state_update(state);
        }
        self.messages.append(&mut other.messages);
    }
}

impl RendererRuntimeInspectorMessage {
    pub fn from_v8_inspector_message(message: Value) -> Self {
        if let Some(event) =
            crate::protocol_types::RuntimeContextRestoreEvent::from_v8_inspector_message(
                message.clone(),
            )
        {
            return Self::RuntimeContext(event);
        }
        Self::Protocol(RendererRuntimeInspectorProtocolMessage::new(message))
    }

    pub fn protocol(message: Value) -> Self {
        Self::Protocol(RendererRuntimeInspectorProtocolMessage::new(message))
    }

    pub(crate) fn renderer_transport_charge_bytes_with(
        &self,
        json_charge: impl Fn(&Value) -> usize,
        string_charge: impl Fn(&str) -> usize,
    ) -> usize {
        match self {
            Self::Protocol(message) => json_charge(message.value()),
            Self::RuntimeContext(event) => {
                event.renderer_transport_charge_bytes_with(string_charge)
            }
        }
    }

    pub(crate) fn has_resolved_source_identity(&self) -> bool {
        let Self::RuntimeContext(crate::protocol_types::RuntimeContextRestoreEvent::Created(event)) =
            self
        else {
            return true;
        };
        // The embedder must bind a default context's origin directly through
        // V8ContextInfo before calling contextCreated. Keep the publication
        // boundary strict so a missing creation-time identity cannot be hidden
        // by reconstructing it from mutable renderer state. Non-default worlds
        // legitimately use an empty origin.
        event.is_default != Some(true)
            || event
                .origin
                .as_deref()
                .is_some_and(|origin| !origin.is_empty())
    }

    pub fn into_v8_inspector_message(self) -> Value {
        match self {
            Self::Protocol(message) => message.into_value(),
            Self::RuntimeContext(event) => {
                runtime_context_restore_event_v8_inspector_message(event)
            }
        }
    }

    pub fn has_v8_inspector_method(&self) -> bool {
        match self {
            Self::Protocol(message) => message.value().get("method").is_some(),
            Self::RuntimeContext(_) => true,
        }
    }
}

impl RendererRuntimeInspectorProtocolMessage {
    pub fn new(value: Value) -> Self {
        let renderer_call_id = renderer_call_id_from_protocol_message(&value);
        Self {
            renderer_call_id,
            value,
        }
    }

    pub fn renderer_call_id(&self) -> Option<moli_page_types::RendererCallId> {
        self.renderer_call_id
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn value_mut(&mut self) -> RendererRuntimeInspectorProtocolMessageValueMut<'_> {
        RendererRuntimeInspectorProtocolMessageValueMut { message: self }
    }

    pub fn into_value(self) -> Value {
        self.value
    }
}

impl std::ops::Deref for RendererRuntimeInspectorProtocolMessageValueMut<'_> {
    type Target = Value;

    fn deref(&self) -> &Self::Target {
        &self.message.value
    }
}

impl std::ops::DerefMut for RendererRuntimeInspectorProtocolMessageValueMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.message.value
    }
}

impl Drop for RendererRuntimeInspectorProtocolMessageValueMut<'_> {
    fn drop(&mut self) {
        self.message.renderer_call_id = renderer_call_id_from_protocol_message(&self.message.value);
    }
}

impl std::ops::Deref for RendererRuntimeInspectorProtocolMessage {
    type Target = Value;

    fn deref(&self) -> &Self::Target {
        self.value()
    }
}

fn renderer_call_id_from_protocol_message(
    value: &Value,
) -> Option<moli_page_types::RendererCallId> {
    value
        .get("id")
        .and_then(Value::as_i64)
        .and_then(|id| i32::try_from(id).ok())
        .map(moli_page_types::RendererCallId::new)
}

fn runtime_context_restore_event_v8_inspector_message(
    event: crate::protocol_types::RuntimeContextRestoreEvent,
) -> Value {
    match event {
        crate::protocol_types::RuntimeContextRestoreEvent::Created(event) => {
            let crate::protocol_types::RuntimeExecutionContextRestoreEvent {
                context_id,
                realm_id,
                frame_id,
                origin,
                name,
                is_default,
                context_type,
                grant_universal_access,
            } = event;
            let mut aux_data = serde_json::Map::new();
            if let Some(frame_id) = frame_id {
                aux_data.insert("frameId".to_owned(), json!(frame_id));
            }
            aux_data.insert("isDefault".to_owned(), json!(is_default));
            aux_data.insert("type".to_owned(), json!(context_type));
            if let Some(grant_universal_access) = grant_universal_access {
                aux_data.insert(
                    "grantUniversalAccess".to_owned(),
                    json!(grant_universal_access),
                );
            }
            json!({
                "method": "Runtime.executionContextCreated",
                "params": {
                    "context": {
                        "id": context_id,
                        "uniqueId": realm_id,
                        "origin": origin,
                        "name": name,
                        "auxData": Value::Object(aux_data),
                    },
                },
            })
        }
        crate::protocol_types::RuntimeContextRestoreEvent::Destroyed(event) => {
            let crate::protocol_types::RuntimeExecutionContextRestoreEvent {
                context_id,
                realm_id,
                ..
            } = event;
            json!({
                "method": "Runtime.executionContextDestroyed",
                "params": {
                    "executionContextId": context_id,
                    "executionContextUniqueId": realm_id,
                },
            })
        }
        crate::protocol_types::RuntimeContextRestoreEvent::Cleared(_event) => {
            json!({
                "method": "Runtime.executionContextsCleared",
                "params": {},
            })
        }
    }
}

/// Source ordering of one Inspector notification batch relative to the exact
/// frontend command that caused its renderer record.
///
/// Most Inspector notifications are emitted before the matching command
/// response. Debugger resume/step transitions are the important inverse: V8
/// sends the response first, then reports `Debugger.resumed` and any following
/// step pause. Keeping that edge on the frozen batch lets protocol preserve the
/// producer order even though response and surrounding notifications cross
/// separate publication boundaries.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RendererRuntimeInspectorMessageResponseOrder {
    #[default]
    BeforeCommandResponse,
    AfterCommandResponse,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RendererRuntimeInspectorMessageBatch {
    pub agent_token: RendererDevToolsAgentToken,
    pub session: DevToolsSessionKey,
    pub v8_state_update: Option<V8InspectorSessionState>,
    pub messages: Vec<RendererRuntimeInspectorMessage>,
    command_response_order: RendererRuntimeInspectorMessageResponseOrder,
    renderer_agent_attachment_id: Option<RendererAgentAttachmentId>,
}

impl RendererRuntimeInspectorMessageBatch {
    pub fn new(
        agent_token: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
        messages: Vec<RendererRuntimeInspectorMessage>,
    ) -> Self {
        Self {
            agent_token,
            session,
            v8_state_update: None,
            messages,
            command_response_order:
                RendererRuntimeInspectorMessageResponseOrder::BeforeCommandResponse,
            renderer_agent_attachment_id: None,
        }
    }

    pub fn command_response_order(&self) -> RendererRuntimeInspectorMessageResponseOrder {
        self.command_response_order
    }

    pub fn new_after_command_response(
        agent_token: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
        messages: Vec<RendererRuntimeInspectorMessage>,
    ) -> Self {
        let mut batch = Self::new(agent_token, session, messages);
        batch.command_response_order =
            RendererRuntimeInspectorMessageResponseOrder::AfterCommandResponse;
        batch
    }

    pub fn renderer_agent_attachment_id(&self) -> Option<RendererAgentAttachmentId> {
        self.renderer_agent_attachment_id
    }

    /// Whether this batch contains a concrete Inspector command response.
    ///
    /// The response is identified from the protocol message itself; no
    /// parallel completion marker or inferred scheduler state is involved.
    pub fn has_renderer_protocol_response(&self) -> bool {
        self.messages.iter().any(|message| {
            matches!(
                message,
                RendererRuntimeInspectorMessage::Protocol(message)
                    if message.renderer_call_id().is_some()
            )
        })
    }

    pub(crate) fn has_resolved_source_identities(&self) -> bool {
        self.messages
            .iter()
            .all(RendererRuntimeInspectorMessage::has_resolved_source_identity)
    }

    #[doc(hidden)]
    pub fn bind_renderer_agent_attachment(&mut self, id: RendererAgentAttachmentId) {
        match self.renderer_agent_attachment_id {
            Some(existing) => assert_eq!(
                existing, id,
                "renderer Inspector batch cannot move between attachment leases"
            ),
            None => self.renderer_agent_attachment_id = Some(id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererDomMutationEventBatch {
    pub session: DevToolsSessionKey,
    pub events: Vec<RendererDomMutationEvent>,
    renderer_agent_attachment_id: Option<RendererAgentAttachmentId>,
}

impl RendererDomMutationEventBatch {
    pub fn new(session: DevToolsSessionKey, events: Vec<RendererDomMutationEvent>) -> Self {
        Self {
            session,
            events,
            renderer_agent_attachment_id: None,
        }
    }

    pub fn renderer_agent_attachment_id(&self) -> Option<RendererAgentAttachmentId> {
        self.renderer_agent_attachment_id
    }

    #[doc(hidden)]
    pub fn bind_renderer_agent_attachment(&mut self, id: RendererAgentAttachmentId) {
        match self.renderer_agent_attachment_id {
            Some(existing) => assert_eq!(
                existing, id,
                "renderer DOM mutation batch cannot move between attachment leases"
            ),
            None => self.renderer_agent_attachment_id = Some(id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererDomMutationEvent {
    AttributeModified {
        node_id: u32,
        name: String,
        value: String,
    },
    AttributeRemoved {
        node_id: u32,
        name: String,
    },
    CharacterDataModified {
        node_id: u32,
        character_data: String,
    },
    ChildNodeCountUpdated {
        node_id: u32,
        child_node_count: usize,
    },
    SetChildNodes {
        parent_node_id: u32,
        nodes: Vec<DocumentNodeSnapshot>,
    },
    ChildNodeInserted {
        parent_node_id: u32,
        previous_node_id: u32,
        node: Box<DocumentNodeSnapshot>,
    },
    ChildNodeRemoved {
        parent_node_id: u32,
        node_id: u32,
    },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RendererCommandTurnOutputRecorder(Rc<RefCell<Vec<PendingRendererOutputRecord>>>);

impl RendererCommandTurnOutputRecorder {
    pub(crate) fn records_into_same_sink(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub(crate) fn push_record(&self, record: PendingRendererOutputRecord) {
        self.0.borrow_mut().push(record);
    }

    pub(crate) fn push_observation(
        &self,
        causal_command: Option<RendererRuntimeCommandCausalIdentity>,
        observation: RendererProtocolObservation,
    ) {
        self.push_record(PendingRendererOutputRecord::observation(
            causal_command,
            observation,
        ));
    }

    pub(crate) fn drain_records(&self) -> Vec<PendingRendererOutputRecord> {
        std::mem::take(&mut *self.0.borrow_mut())
    }

    pub(crate) fn push_owner_action(
        &self,
        causal_command: Option<RendererRuntimeCommandCausalIdentity>,
        action: RendererOwnerAction,
    ) {
        self.push_record(PendingRendererOutputRecord::owner_action(
            causal_command,
            action,
        ));
    }

    pub(crate) fn push_runtime_inspector_message(
        &self,
        agent_token: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
        message: Value,
    ) {
        self.push_record(PendingRendererOutputRecord::observation(
            None,
            RendererProtocolObservation::RuntimeInspector(
                RendererRuntimeInspectorMessageBatch::new(
                    agent_token,
                    session,
                    vec![RendererRuntimeInspectorMessage::from_v8_inspector_message(
                        message,
                    )],
                ),
            ),
        ));
    }

    pub(crate) fn push_document_lifecycle_event(&self, event: RendererDocumentLifecycleEvent) {
        self.push_record(PendingRendererOutputRecord::observation(
            None,
            RendererProtocolObservation::DocumentLifecycle(event),
        ));
    }

    pub(crate) fn push_runtime_binding_call(&self, call: PendingRuntimeBindingCall) {
        self.push_record(PendingRendererOutputRecord::observation(
            None,
            RendererProtocolObservation::RuntimeBinding(call),
        ));
    }

    pub(crate) fn push_child_frame_tree_event(
        &self,
        source_document: RendererDocumentLifecycleIdentity,
        event: ChildFrameTreeEventSnapshot,
    ) {
        self.push_owner_action(
            None,
            RendererOwnerAction::ChildFrameTree {
                source_document,
                event,
            },
        );
    }

    pub(crate) fn push_child_frame_document_opened(
        &self,
        source_document: RendererDocumentLifecycleIdentity,
        event: ChildFrameDocumentOpenedSnapshot,
    ) {
        self.push_owner_action(
            None,
            RendererOwnerAction::ChildFrameDocumentOpened {
                source_document,
                event,
            },
        );
    }

    pub(crate) fn finish(self) -> Vec<PendingRendererOutputRecord> {
        std::mem::take(&mut *self.0.borrow_mut())
    }
}

/// One renderer continuation whose causal work must not be admitted until the
/// matching protocol response has crossed its frontend flush boundary.
///
/// Dropping the capability releases it. This fail-open behavior prevents a
/// disconnected frontend or abandoned command from permanently parking page
/// work.
#[must_use = "carry the capability to the matching response boundary; dropping it releases early"]
pub struct RendererPageCommandPostResponseContinuation {
    release: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl RendererPageCommandPostResponseContinuation {
    pub(crate) fn new(release: impl FnOnce() + Send + 'static) -> Self {
        Self {
            release: Some(Box::new(release)),
        }
    }

    pub fn release(mut self) {
        self.release_inner();
    }

    fn release_inner(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

impl Drop for RendererPageCommandPostResponseContinuation {
    fn drop(&mut self) {
        self.release_inner();
    }
}

#[cfg(test)]
mod post_response_continuation_tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::RendererPageCommandPostResponseContinuation;

    #[test]
    fn explicit_release_consumes_the_capability_exactly_once() {
        let releases = Arc::new(AtomicUsize::new(0));
        let release_counter = releases.clone();
        RendererPageCommandPostResponseContinuation::new(move || {
            release_counter.fetch_add(1, Ordering::SeqCst);
        })
        .release();

        assert_eq!(releases.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dropping_the_capability_releases_fail_open() {
        let releases = Arc::new(AtomicUsize::new(0));
        let release_counter = releases.clone();
        drop(RendererPageCommandPostResponseContinuation::new(
            move || {
                release_counter.fetch_add(1, Ordering::SeqCst);
            },
        ));

        assert_eq!(releases.load(Ordering::SeqCst), 1);
    }
}

/// The unique completion boundary at the end of one renderer command turn.
pub struct RendererCommandTurnCompletion {
    reply: RendererPageReply,
    page_state: Arc<RendererPageState>,
    post_response_continuation: Option<RendererPageCommandPostResponseContinuation>,
}

impl RendererCommandTurnCompletion {
    pub fn reply(&self) -> &RendererPageReply {
        &self.reply
    }

    pub fn page_state(&self) -> &Arc<RendererPageState> {
        &self.page_state
    }

    pub fn runtime_inspector_output(&self) -> Option<&RendererRuntimeCommandOutput> {
        match &self.reply {
            RendererPageReply::RuntimeInspectorProtocolMessages(messages) => Some(messages),
            _ => None,
        }
    }

    fn runtime_inspector_output_mut(&mut self) -> Option<&mut RendererRuntimeCommandOutput> {
        match &mut self.reply {
            RendererPageReply::RuntimeInspectorProtocolMessages(messages) => Some(messages),
            _ => None,
        }
    }

    pub fn into_runtime_inspector_output(self) -> Option<RendererRuntimeCommandOutput> {
        match self.reply {
            RendererPageReply::RuntimeInspectorProtocolMessages(messages) => Some(messages),
            _ => None,
        }
    }

    pub fn has_post_response_continuation(&self) -> bool {
        self.post_response_continuation.is_some()
    }

    pub fn take_post_response_continuation(
        &mut self,
    ) -> Option<RendererPageCommandPostResponseContinuation> {
        self.post_response_continuation.take()
    }

    pub fn into_parts(
        self,
    ) -> (
        RendererPageReply,
        Arc<RendererPageState>,
        Option<RendererPageCommandPostResponseContinuation>,
    ) {
        (self.reply, self.page_state, self.post_response_continuation)
    }
}

/// Canonical completion of one renderer command.
///
/// Records produced by the command are settled into the Page's concrete
/// output stream before this value is returned. The exact predecessor cursor
/// therefore carries the ordering contract; duplicating records beside the
/// completion would create a second, competing transport path.
pub struct RendererCommandTurnOutput {
    completion: RendererCommandTurnCompletion,
    renderer_output_predecessor: Option<RendererOutputFence>,
    // A nested synchronous DevTools agent call must keep its per-session Main
    // receiver slot until the protocol actor owns the settled result. This is
    // deliberately distinct from a post-response continuation: dropping the
    // result at the protocol handoff is enough to preserve receiver order,
    // while waiting for the frontend response would deadlock multi-stage
    // commands that need to enter the same receiver again.
    protocol_handoff: Option<Box<RendererInspectorMainFirstDispatchGuard>>,
}

impl RendererCommandTurnOutput {
    pub(crate) fn new(
        reply: RendererPageReply,
        page_state: Arc<RendererPageState>,
        runtime_command_output: RendererRuntimeCommandOutput,
        post_response_continuation: Option<RendererPageCommandPostResponseContinuation>,
        renderer_output_predecessor: Option<RendererOutputFence>,
    ) -> Result<Self> {
        let reply = match reply {
            RendererPageReply::RuntimeInspectorProtocolMessages(reply_messages) => {
                let mut messages = runtime_command_output;
                messages.append(reply_messages);
                RendererPageReply::RuntimeInspectorProtocolMessages(messages)
            }
            reply => {
                anyhow::ensure!(
                    runtime_command_output.is_empty(),
                    "non-Runtime renderer command produced Runtime command output"
                );
                reply
            }
        };
        Ok(Self {
            completion: RendererCommandTurnCompletion {
                reply,
                page_state,
                post_response_continuation,
            },
            renderer_output_predecessor,
            protocol_handoff: None,
        })
    }

    pub(crate) fn hold_until_protocol_handoff(
        mut self,
        handoff: RendererInspectorMainFirstDispatchGuard,
    ) -> Self {
        debug_assert!(
            self.protocol_handoff.is_none(),
            "a renderer command turn may hold only one protocol handoff"
        );
        self.protocol_handoff = Some(Box::new(handoff));
        self
    }

    #[doc(hidden)]
    pub fn bind_renderer_agent_attachment(&mut self, id: RendererAgentAttachmentId) {
        if let RendererPageReply::RuntimeInspectorProtocolMessages(output) =
            &mut self.completion.reply
        {
            output.bind_renderer_agent_attachment(id);
        }
    }

    pub fn completion(&self) -> &RendererCommandTurnCompletion {
        &self.completion
    }

    /// Exact concrete Page-stream position that must cross protocol ingress
    /// before the matching protocol response is exposed.
    ///
    /// This is deliberately a stream cursor, not a process-global watermark.
    /// It covers the Page-stream prefix that preceded command admission plus
    /// any records settled by the command itself. Later Page work receives a
    /// later cursor and cannot delay this response.
    pub fn renderer_output_predecessor(&self) -> Option<RendererOutputFence> {
        self.renderer_output_predecessor.clone()
    }

    pub(crate) fn merge_renderer_output_predecessor(&mut self, predecessor: RendererOutputFence) {
        predecessor.merge_into_same_stream_tail(&mut self.renderer_output_predecessor);
    }

    pub fn runtime_inspector_output(&self) -> Option<&RendererRuntimeCommandOutput> {
        self.completion.runtime_inspector_output()
    }

    pub fn runtime_inspector_output_mut(&mut self) -> Option<&mut RendererRuntimeCommandOutput> {
        self.completion.runtime_inspector_output_mut()
    }

    pub fn into_completion_and_predecessor(
        self,
    ) -> (RendererCommandTurnCompletion, Option<RendererOutputFence>) {
        let Self {
            completion,
            renderer_output_predecessor,
            protocol_handoff: _,
        } = self;
        (completion, renderer_output_predecessor)
    }

    pub fn into_reply_and_state(self) -> (RendererPageReply, Arc<RendererPageState>) {
        let (completion, _renderer_output_predecessor) = self.into_completion_and_predecessor();
        let (reply, page_state, _) = completion.into_parts();
        (reply, page_state)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RendererDocumentIsolateAccountingDiagnostics {
    pub created: u64,
    pub destroyed: u64,
    pub live: u64,
    pub reserved: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RendererActivityDiagnostics {
    pub document_context_count: usize,
    pub isolated_world_context_count: usize,
    pub child_default_context_count: usize,
    pub pending_subresource_requests: usize,
    pub pending_subresource_fetch_infos: usize,
    pub pending_subresource_continue_events: usize,
    pub pending_file_chooser_activations: usize,
    pub pending_download_activations: usize,
    pub pending_javascript_dialogs: usize,
    pub pending_runtime_binding_calls: usize,
    pub pending_inspector_messages: usize,
    pub runtime_console_messages_with_context: usize,
    pub runtime_console_messages_by_context: BTreeMap<i64, usize>,
    pub runtime_lifecycle_errors: usize,
    pub completed_child_frame_navigation_loads: usize,
    pub pending_popup_activations: usize,
    pub dedicated_worker_loading_count: usize,
    pub dedicated_worker_running_worker_isolate_count: usize,
    pub pending_webcrypto_tasks: usize,
    pub pending_opfs_tasks: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RendererRuntimeObservableSourceSummary {
    default_execution_context_id: Option<i64>,
    source_items: Vec<RendererRuntimeObservableSourceItem>,
    inspector_issues: Vec<moli_page_types::InspectorIssueSnapshot>,
}

#[derive(Debug, Clone, serde::Deserialize, PartialEq, Eq)]
pub struct RuntimeConsoleMessageSnapshot {
    #[serde(rename = "executionContextId")]
    pub execution_context_id: i64,
    pub message: String,
    #[serde(default)]
    pub args: Vec<Value>,
    #[serde(default)]
    pub stack: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RendererCountEntry {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RendererRuntimeHeapSpaceUsage {
    pub name: String,
    pub size: usize,
    pub used_size: usize,
    pub available_size: usize,
    pub physical_size: usize,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RendererMoliMemoryScopeDiagnostics {
    pub v8_heap: &'static str,
    pub v8_heap_is_target_local: bool,
    pub counters: &'static str,
    pub garbage_collection: &'static str,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RendererMoliDomMemoryDiagnostics {
    pub node_count: usize,
    pub connected_node_count: usize,
    pub in_document_tree_node_count: usize,
    pub parser_created_node_count: usize,
    pub node_counts_by_kind: BTreeMap<String, usize>,
    pub top_element_tags: Vec<RendererCountEntry>,
    pub attribute_count: usize,
    pub attribute_name_bytes: usize,
    pub attribute_value_bytes: usize,
    pub element_name_bytes: usize,
    pub text_node_count: usize,
    pub text_bytes: usize,
    pub comment_bytes: usize,
    pub cdata_bytes: usize,
    pub processing_instruction_bytes: usize,
    pub string_payload_bytes: usize,
    pub script_element_count: usize,
    pub external_script_count: usize,
    pub external_script_src_bytes: usize,
    pub inline_script_text_bytes: usize,
    pub image_element_count: usize,
    pub iframe_element_count: usize,
    pub style_element_count: usize,
    pub link_stylesheet_count: usize,
    pub template_content_count: usize,
    pub parse_error_count: usize,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RendererMoliRuntimeMemoryDiagnostics {
    pub runtime_observable_context_count: usize,
    pub isolated_context_count: usize,
    pub child_default_context_count: usize,
    pub child_browsing_context_count: usize,
    pub pending_subresource_requests: usize,
    pub pending_subresource_fetch_infos: usize,
    pub pending_subresource_continue_events: usize,
    pub pending_runtime_binding_calls: usize,
    pub completed_child_frame_navigation_loads: usize,
    pub pending_inspector_messages: usize,
    pub inspector_session_registry_owner: &'static str,
    pub inspector_session_registry_lifetime_scope: &'static str,
    pub inspector_session_count: usize,
    pub inspector_context_group_id: i32,
    pub inspector_context_group_scope: &'static str,
    pub inspector_context_registration_count: usize,
    pub main_window_proxy_identity_hash: Option<i32>,
    pub inspector_default_context_registry_count: usize,
    pub inspector_default_context_registry_scope: &'static str,
    pub v8_foreground_task_wake_scope: &'static str,
    pub v8_foreground_task_wake_context_group_id_available: bool,
    pub v8_foreground_task_wake_internal_policy: &'static str,
    pub v8_foreground_task_wake_external_policy: &'static str,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RendererScriptSourceMemoryDiagnostics {
    pub url: String,
    pub source_bytes: usize,
    pub kind: String,
    pub mode: String,
    pub source_kind: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RendererScriptExecutionMemoryDiagnostics {
    pub execution_count: usize,
    pub total_source_bytes: usize,
    pub inline_source_bytes: usize,
    pub external_source_bytes: usize,
    pub classic_source_bytes: usize,
    pub module_source_bytes: usize,
    pub import_map_source_bytes: usize,
    pub data_block_source_bytes: usize,
    pub inline_execution_count: usize,
    pub external_execution_count: usize,
    pub classic_execution_count: usize,
    pub module_execution_count: usize,
    pub import_map_execution_count: usize,
    pub data_block_execution_count: usize,
    pub largest_sources: Vec<RendererScriptSourceMemoryDiagnostics>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RendererMoliMemoryDiagnostics {
    pub scope: RendererMoliMemoryScopeDiagnostics,
    pub dom: RendererMoliDomMemoryDiagnostics,
    pub runtime: RendererMoliRuntimeMemoryDiagnostics,
    pub script_execution: RendererScriptExecutionMemoryDiagnostics,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RendererRuntimeHeapUsage {
    pub used_size: usize,
    pub total_size: usize,
    pub total_heap_size_executable: usize,
    pub total_physical_size: usize,
    pub total_available_size: usize,
    pub heap_size_limit: usize,
    pub malloced_memory: usize,
    pub peak_malloced_memory: usize,
    pub external_memory: usize,
    pub number_of_native_contexts: usize,
    pub number_of_detached_contexts: usize,
    pub total_allocated_bytes: u64,
    pub total_global_handles_size: usize,
    pub used_global_handles_size: usize,
    pub heap_spaces: Vec<RendererRuntimeHeapSpaceUsage>,
    pub moli: RendererMoliMemoryDiagnostics,
}

impl RendererRuntimeHeapUsage {
    pub fn to_diagnostics_json(&self) -> Value {
        serde_json::to_value(self).expect("renderer runtime heap usage should serialize")
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RendererPerformanceMetricSnapshot {
    pub time_origin_ms: Option<f64>,
    pub now_ms: Option<f64>,
    pub navigation_start_ms: Option<f64>,
    pub dom_content_loaded_ms: Option<f64>,
    pub load_event_ms: Option<f64>,
    pub document_count: Option<f64>,
    pub frame_count: Option<f64>,
    pub node_count: Option<f64>,
    pub resource_count: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererRuntimeObservableSourceItem {
    ConsoleMessage {
        message: RuntimeConsoleMessageSnapshot,
        context_count_end: usize,
    },
    LifecycleError {
        text: String,
        execution_context_id: Option<i64>,
        exception_index: usize,
    },
}

impl RendererRuntimeObservableSourceItem {
    fn console_message(message: RuntimeConsoleMessageSnapshot, context_count_end: usize) -> Self {
        Self::ConsoleMessage {
            message,
            context_count_end,
        }
    }

    fn lifecycle_error(
        text: String,
        execution_context_id: Option<i64>,
        exception_index: usize,
    ) -> Self {
        Self::LifecycleError {
            text,
            execution_context_id,
            exception_index,
        }
    }
}

impl RendererRuntimeObservableSourceSummary {
    pub fn from_source_messages(
        default_execution_context_id: Option<i64>,
        console_messages: Vec<RuntimeConsoleMessageSnapshot>,
        lifecycle_error_messages: Vec<String>,
    ) -> Self {
        let mut source_items = console_messages
            .into_iter()
            .scan(BTreeMap::<i64, usize>::new(), |counts, message| {
                let context_count = counts.entry(message.execution_context_id).or_default();
                *context_count = context_count
                    .checked_add(1)
                    .expect("runtime observable source item context count overflow");
                Some(RendererRuntimeObservableSourceItem::console_message(
                    message,
                    *context_count,
                ))
            })
            .collect::<Vec<_>>();
        source_items.extend(lifecycle_error_messages.into_iter().enumerate().map(
            |(exception_index, error)| {
                RendererRuntimeObservableSourceItem::lifecycle_error(
                    error,
                    default_execution_context_id,
                    exception_index,
                )
            },
        ));
        Self::from_source_items(default_execution_context_id, source_items)
    }

    pub fn is_empty(&self) -> bool {
        self.source_items.is_empty() && self.inspector_issues.is_empty()
    }

    pub fn add_lifecycle_error_messages(&mut self, lifecycle_errors: Vec<String>) {
        let lifecycle_error_start = self.lifecycle_errors();
        self.source_items
            .extend(lifecycle_errors.iter().enumerate().map(|(offset, error)| {
                RendererRuntimeObservableSourceItem::lifecycle_error(
                    error.clone(),
                    self.default_execution_context_id,
                    lifecycle_error_start
                        .checked_add(offset)
                        .expect("runtime observable lifecycle error index overflow"),
                )
            }));
    }

    pub fn default_execution_context_id(&self) -> Option<i64> {
        self.default_execution_context_id
    }

    pub fn inspector_issues(&self) -> &[moli_page_types::InspectorIssueSnapshot] {
        &self.inspector_issues
    }

    pub fn set_inspector_issues(&mut self, issues: Vec<moli_page_types::InspectorIssueSnapshot>) {
        self.inspector_issues = issues;
    }

    pub fn console_messages_with_context(&self) -> usize {
        self.source_items
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    RendererRuntimeObservableSourceItem::ConsoleMessage { .. }
                )
            })
            .count()
    }

    pub fn console_messages_by_context(&self) -> BTreeMap<i64, usize> {
        let mut counts = BTreeMap::<i64, usize>::new();
        for item in &self.source_items {
            if let RendererRuntimeObservableSourceItem::ConsoleMessage { message, .. } = item {
                let count = counts.entry(message.execution_context_id).or_default();
                *count = (*count)
                    .checked_add(1)
                    .expect("runtime observable console message context count overflow");
            }
        }
        counts
    }

    pub fn lifecycle_errors(&self) -> usize {
        self.source_items
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    RendererRuntimeObservableSourceItem::LifecycleError { .. }
                )
            })
            .count()
    }

    pub fn source_items(&self) -> &[RendererRuntimeObservableSourceItem] {
        &self.source_items
    }

    pub fn from_source_items(
        default_execution_context_id: Option<i64>,
        source_items: Vec<RendererRuntimeObservableSourceItem>,
    ) -> Self {
        Self {
            default_execution_context_id,
            source_items,
            inspector_issues: Vec::new(),
        }
    }
}

fn source_items_for_snapshot(
    default_execution_context_id: Option<i64>,
    source_items: &[RendererRuntimeObservableSourceItem],
) -> Vec<RendererRuntimeObservableSourceItem> {
    source_items
        .iter()
        .cloned()
        .map(|item| match item {
            RendererRuntimeObservableSourceItem::ConsoleMessage {
                message,
                context_count_end,
            } => RendererRuntimeObservableSourceItem::console_message(message, context_count_end),
            RendererRuntimeObservableSourceItem::LifecycleError {
                text,
                exception_index,
                ..
            } => RendererRuntimeObservableSourceItem::lifecycle_error(
                text,
                default_execution_context_id,
                exception_index,
            ),
        })
        .collect()
}

fn source_item_next_context_count(
    source_items: &[RendererRuntimeObservableSourceItem],
    execution_context_id: i64,
) -> usize {
    source_items
        .iter()
        .filter(|item| {
            matches!(
                item,
                RendererRuntimeObservableSourceItem::ConsoleMessage { message, .. }
                    if message.execution_context_id == execution_context_id
            )
        })
        .count()
        .checked_add(1)
        .expect("runtime observable source item context count overflow")
}

fn source_item_next_lifecycle_error_index(
    source_items: &[RendererRuntimeObservableSourceItem],
) -> usize {
    source_items
        .iter()
        .filter(|item| {
            matches!(
                item,
                RendererRuntimeObservableSourceItem::LifecycleError { .. }
            )
        })
        .count()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RendererRuntimeObservableSourceQueue {
    source_items: Vec<RendererRuntimeObservableSourceItem>,
    pending_console_events: Vec<PendingRuntimeObservableConsoleSourceEvent>,
    report_default_console_message_count: usize,
}

impl RendererRuntimeObservableSourceQueue {
    pub(crate) fn record_lifecycle_error(&mut self, message: String) {
        let exception_index = source_item_next_lifecycle_error_index(&self.source_items);
        self.source_items
            .push(RendererRuntimeObservableSourceItem::lifecycle_error(
                message,
                None,
                exception_index,
            ));
    }

    pub(crate) fn record_pending_console_event(
        &mut self,
        event: PendingRuntimeObservableConsoleSourceEvent,
    ) {
        self.pending_console_events.push(event);
    }

    pub(crate) fn record_console_message(&mut self, message: RuntimeConsoleMessageSnapshot) {
        let context_count_end =
            source_item_next_context_count(&self.source_items, message.execution_context_id);
        self.source_items
            .push(RendererRuntimeObservableSourceItem::console_message(
                message,
                context_count_end,
            ));
    }

    #[cfg(test)]
    pub(crate) fn sync_console_events(
        &mut self,
        active_contexts: &BTreeSet<i64>,
        active_tokens: &BTreeSet<RuntimeObservableContextToken>,
        token_to_execution_context_id: &BTreeMap<RuntimeObservableContextToken, i64>,
        pending_console_events: Vec<PendingRuntimeObservableConsoleSourceEvent>,
    ) {
        self.sync_source_events(
            active_contexts,
            active_tokens,
            token_to_execution_context_id,
            pending_console_events,
        );
    }

    pub(crate) fn sync_source_events(
        &mut self,
        active_contexts: &BTreeSet<i64>,
        active_tokens: &BTreeSet<RuntimeObservableContextToken>,
        token_to_execution_context_id: &BTreeMap<RuntimeObservableContextToken, i64>,
        pending_console_events: Vec<PendingRuntimeObservableConsoleSourceEvent>,
    ) {
        self.source_items.retain(|item| match item {
            RendererRuntimeObservableSourceItem::ConsoleMessage { message, .. } => {
                active_contexts.contains(&message.execution_context_id)
            }
            RendererRuntimeObservableSourceItem::LifecycleError { .. } => true,
        });

        self.pending_console_events.extend(pending_console_events);
        let pending_events = std::mem::take(&mut self.pending_console_events);
        for event in pending_events {
            if !active_tokens.contains(&event.context_token()) {
                continue;
            }
            if let Some(execution_context_id) =
                token_to_execution_context_id.get(&event.context_token())
            {
                let context_count_end =
                    source_item_next_context_count(&self.source_items, *execution_context_id);
                let message = event.into_runtime_console_message_snapshot(*execution_context_id);
                self.source_items
                    .push(RendererRuntimeObservableSourceItem::console_message(
                        message,
                        context_count_end,
                    ));
            } else {
                self.pending_console_events.push(event);
            }
        }
    }

    pub(crate) fn snapshot(
        &self,
        default_execution_context_id: Option<i64>,
    ) -> Option<RendererRuntimeObservableSourceSummary> {
        let source = RendererRuntimeObservableSourceSummary::from_source_items(
            default_execution_context_id,
            source_items_for_snapshot(default_execution_context_id, &self.source_items),
        );
        (!source.is_empty()).then_some(source)
    }

    pub(crate) fn take_report_observable_output(
        &mut self,
        default_execution_context_id: Option<i64>,
        default_context_token: RuntimeObservableContextToken,
    ) -> ScriptObservableOutput {
        let mut output = ScriptObservableOutput::default();
        let mut default_console_message_count = 0usize;
        for item in &self.source_items {
            match item {
                RendererRuntimeObservableSourceItem::ConsoleMessage { message, .. }
                    if Some(message.execution_context_id) == default_execution_context_id =>
                {
                    if default_console_message_count >= self.report_default_console_message_count {
                        output.push_item(ScriptObservableOutputItem::ConsoleMessage(
                            message.message.clone(),
                        ));
                    }
                    default_console_message_count = default_console_message_count
                        .checked_add(1)
                        .expect("report observable console message count overflow");
                }
                RendererRuntimeObservableSourceItem::LifecycleError { text, .. } => {
                    output.push_item(ScriptObservableOutputItem::LifecycleError(text.clone()));
                }
                RendererRuntimeObservableSourceItem::ConsoleMessage { .. } => {}
            }
        }
        for event in &self.pending_console_events {
            if event.context_token() != default_context_token {
                continue;
            }
            if default_console_message_count >= self.report_default_console_message_count {
                output.push_item(ScriptObservableOutputItem::ConsoleMessage(
                    event.message().to_owned(),
                ));
            }
            default_console_message_count = default_console_message_count
                .checked_add(1)
                .expect("report observable pending console message count overflow");
        }
        self.report_default_console_message_count = default_console_message_count;
        self.source_items.retain(|item| {
            matches!(
                item,
                RendererRuntimeObservableSourceItem::ConsoleMessage { .. }
            )
        });
        output
    }

    pub(crate) fn append_lifecycle_error_messages_to_snapshot(
        snapshot: &mut Option<RendererRuntimeObservableSourceSummary>,
        default_execution_context_id: Option<i64>,
        lifecycle_error_messages: Vec<String>,
    ) {
        if lifecycle_error_messages.is_empty() {
            return;
        }
        if let Some(source) = snapshot.as_mut() {
            source.add_lifecycle_error_messages(lifecycle_error_messages);
        } else {
            *snapshot = Some(
                RendererRuntimeObservableSourceSummary::from_source_messages(
                    default_execution_context_id,
                    Vec::new(),
                    lifecycle_error_messages,
                ),
            );
        }
    }

    pub(crate) fn append_report_observable_items_to_snapshot(
        snapshot: &mut Option<RendererRuntimeObservableSourceSummary>,
        default_execution_context_id: Option<i64>,
        items: &[ScriptObservableOutputItem],
    ) -> usize {
        let lifecycle_error_messages = items
            .iter()
            .filter_map(|item| match item {
                ScriptObservableOutputItem::ConsoleMessage(_)
                | ScriptObservableOutputItem::InspectorIssue(_) => None,
                ScriptObservableOutputItem::LifecycleError(error) => Some(error.clone()),
            })
            .collect::<Vec<_>>();
        let lifecycle_error_count = lifecycle_error_messages.len();
        Self::append_lifecycle_error_messages_to_snapshot(
            snapshot,
            default_execution_context_id,
            lifecycle_error_messages,
        );
        let inspector_issues = items
            .iter()
            .filter_map(|item| match item {
                ScriptObservableOutputItem::InspectorIssue(issue) => Some((**issue).clone()),
                ScriptObservableOutputItem::ConsoleMessage(_)
                | ScriptObservableOutputItem::LifecycleError(_) => None,
            })
            .collect::<Vec<_>>();
        if !inspector_issues.is_empty() {
            if snapshot.is_none() {
                *snapshot = Some(RendererRuntimeObservableSourceSummary::from_source_items(
                    default_execution_context_id,
                    Vec::new(),
                ));
            }
            if let Some(source) = snapshot.as_mut() {
                source.set_inspector_issues(inspector_issues);
            }
        }
        lifecycle_error_count
    }

    #[cfg(test)]
    pub(crate) fn lifecycle_error_messages_for_testing(&self) -> Vec<String> {
        self.source_items
            .iter()
            .filter_map(|item| match item {
                RendererRuntimeObservableSourceItem::ConsoleMessage { .. } => None,
                RendererRuntimeObservableSourceItem::LifecycleError { text, .. } => {
                    Some(text.clone())
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RendererPageDiagnosticsSnapshot {
    document_lifecycle_identity: Option<RendererDocumentLifecycleIdentity>,
    document_input_stream_opened: bool,
    runtime_observable_source: Option<RendererRuntimeObservableSourceSummary>,
    pub diagnostics: RendererActivityDiagnostics,
}

impl RendererPageDiagnosticsSnapshot {
    pub fn from_diagnostics(diagnostics: RendererActivityDiagnostics) -> Self {
        Self {
            diagnostics,
            ..Default::default()
        }
    }

    pub fn from_runtime_observable_source(source: RendererRuntimeObservableSourceSummary) -> Self {
        Self {
            runtime_observable_source: Some(source),
            ..Default::default()
        }
    }

    pub fn document_lifecycle_identity(&self) -> Option<RendererDocumentLifecycleIdentity> {
        self.document_lifecycle_identity
    }

    pub(crate) fn set_document_lifecycle_identity(
        &mut self,
        identity: RendererDocumentLifecycleIdentity,
    ) {
        self.document_lifecycle_identity = Some(identity);
    }

    pub fn document_input_stream_opened(&self) -> bool {
        self.document_input_stream_opened
    }

    pub fn set_document_input_stream_opened(&mut self, opened: bool) {
        self.document_input_stream_opened = opened;
    }

    pub fn runtime_observable_source(&self) -> Option<&RendererRuntimeObservableSourceSummary> {
        self.runtime_observable_source.as_ref()
    }

    pub fn set_runtime_observable_source(
        &mut self,
        source: Option<RendererRuntimeObservableSourceSummary>,
    ) {
        self.runtime_observable_source = source;
    }

    pub fn has_runtime_observable_source(&self) -> bool {
        self.runtime_observable_source.is_some()
    }

    pub fn append_report_observable_items(
        &mut self,
        default_execution_context_id: Option<i64>,
        items: &[ScriptObservableOutputItem],
    ) -> usize {
        RendererRuntimeObservableSourceQueue::append_report_observable_items_to_snapshot(
            &mut self.runtime_observable_source,
            default_execution_context_id,
            items,
        )
    }
}

#[cfg(test)]
mod page_diagnostics_snapshot_tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::{
        PendingRuntimeObservableConsoleSourceEvent, RendererRuntimeObservableSourceItem,
        RendererRuntimeObservableSourceQueue, RendererRuntimeObservableSourceSummary,
        RuntimeConsoleMessageSnapshot, RuntimeObservableContextToken,
    };
    use crate::types::ScriptObservableOutputItem;

    #[test]
    fn runtime_observable_source_summary_counts_source_items_by_context() {
        let source = RendererRuntimeObservableSourceSummary::from_source_messages(
            Some(11),
            vec![
                runtime_console_message(3, "first"),
                runtime_console_message(7, "second"),
                runtime_console_message(3, "third"),
            ],
            vec!["failure".to_owned()],
        );

        assert_eq!(source.default_execution_context_id(), Some(11));
        assert_eq!(source.console_messages_with_context(), 3);
        assert_eq!(source.console_messages_by_context().get(&3), Some(&2));
        assert_eq!(source.console_messages_by_context().get(&7), Some(&1));
        assert_eq!(source.lifecycle_errors(), 1);
        assert!(
            matches!(
                &source.source_items()[0],
                RendererRuntimeObservableSourceItem::ConsoleMessage {
                    message,
                    context_count_end: 1,
                } if message.execution_context_id == 3 && message.message == "first"
            ),
            "renderer source summary should expose append-time console cursor item"
        );
        assert!(
            matches!(
                &source.source_items()[2],
                RendererRuntimeObservableSourceItem::ConsoleMessage {
                    message,
                    context_count_end: 2,
                } if message.execution_context_id == 3 && message.message == "third"
            ),
            "renderer source item should carry the per-context end cursor"
        );
        assert!(
            matches!(
                &source.source_items()[3],
                RendererRuntimeObservableSourceItem::LifecycleError {
                    text,
                    execution_context_id: Some(11),
                    exception_index: 0,
                } if text == "failure"
            ),
            "renderer source summary should expose append-time lifecycle cursor item"
        );
    }

    #[test]
    fn runtime_observable_source_queue_owns_lifecycle_errors() {
        let mut queue = RendererRuntimeObservableSourceQueue::default();
        queue.record_lifecycle_error("first failure".to_owned());
        queue.record_lifecycle_error("second failure".to_owned());

        let source = queue
            .snapshot(Some(5))
            .expect("lifecycle errors should produce a source snapshot");

        assert_eq!(source.default_execution_context_id(), Some(5));
        assert_eq!(source.lifecycle_errors(), 2);
        assert_eq!(
            source.source_items(),
            &[
                RendererRuntimeObservableSourceItem::LifecycleError {
                    text: "first failure".to_owned(),
                    execution_context_id: Some(5),
                    exception_index: 0,
                },
                RendererRuntimeObservableSourceItem::LifecycleError {
                    text: "second failure".to_owned(),
                    execution_context_id: Some(5),
                    exception_index: 1,
                },
            ],
        );

        let report_output = queue
            .take_report_observable_output(Some(5), RuntimeObservableContextToken::from_raw(50));
        let report_items: Vec<_> = report_output.into_items().collect();
        assert_eq!(
            report_items,
            vec![
                ScriptObservableOutputItem::LifecycleError("first failure".to_owned()),
                ScriptObservableOutputItem::LifecycleError("second failure".to_owned()),
            ]
        );
        assert!(
            queue.snapshot(Some(5)).is_none(),
            "draining lifecycle errors for the page report should remove queue-owned lifecycle source items"
        );
    }

    #[test]
    fn runtime_observable_source_queue_projects_report_output_by_producer_cursor() {
        let mut queue = RendererRuntimeObservableSourceQueue::default();
        queue
            .source_items
            .push(RendererRuntimeObservableSourceItem::console_message(
                runtime_console_message(5, "log: first"),
                1,
            ));
        queue
            .source_items
            .push(RendererRuntimeObservableSourceItem::console_message(
                runtime_console_message(9, "log: isolated"),
                1,
            ));
        queue.record_lifecycle_error("first failure".to_owned());

        let report_output = queue
            .take_report_observable_output(Some(5), RuntimeObservableContextToken::from_raw(50));
        let report_items: Vec<_> = report_output.into_items().collect();
        assert_eq!(
            report_items,
            vec![
                ScriptObservableOutputItem::ConsoleMessage("log: first".to_owned()),
                ScriptObservableOutputItem::LifecycleError("first failure".to_owned()),
            ]
        );
        assert_eq!(
            queue
                .snapshot(Some(5))
                .expect("console source items should remain available")
                .console_messages_with_context(),
            2,
            "projecting report output must not drain RuntimeObservable console source items"
        );

        queue
            .source_items
            .push(RendererRuntimeObservableSourceItem::console_message(
                runtime_console_message(5, "log: second"),
                2,
            ));
        queue.record_lifecycle_error("second failure".to_owned());

        let report_output = queue
            .take_report_observable_output(Some(5), RuntimeObservableContextToken::from_raw(50));
        let report_items: Vec<_> = report_output.into_items().collect();
        assert_eq!(
            report_items,
            vec![
                ScriptObservableOutputItem::ConsoleMessage("log: second".to_owned()),
                ScriptObservableOutputItem::LifecycleError("second failure".to_owned()),
            ],
            "report output should only include default-context console events after the report cursor"
        );
    }

    #[test]
    fn runtime_observable_source_queue_projects_pending_default_console_to_report() {
        let mut queue = RendererRuntimeObservableSourceQueue::default();
        queue.pending_console_events.push(
            PendingRuntimeObservableConsoleSourceEvent::new_for_testing(50, "log: pending default"),
        );
        queue.pending_console_events.push(
            PendingRuntimeObservableConsoleSourceEvent::new_for_testing(
                90,
                "log: pending isolated",
            ),
        );

        let report_output = queue
            .take_report_observable_output(Some(5), RuntimeObservableContextToken::from_raw(50));
        let report_items: Vec<_> = report_output.into_items().collect();
        assert_eq!(
            report_items,
            vec![ScriptObservableOutputItem::ConsoleMessage(
                "log: pending default".to_owned()
            )],
            "default-token console output should reach the page report before Runtime source resolution"
        );

        queue.sync_console_events(
            &BTreeSet::from([5, 9]),
            &BTreeSet::from([
                RuntimeObservableContextToken::from_raw(50),
                RuntimeObservableContextToken::from_raw(90),
            ]),
            &BTreeMap::from([
                (RuntimeObservableContextToken::from_raw(50), 5),
                (RuntimeObservableContextToken::from_raw(90), 9),
            ]),
            Vec::new(),
        );
        let report_output = queue
            .take_report_observable_output(Some(5), RuntimeObservableContextToken::from_raw(50));
        assert!(
            report_output.is_empty(),
            "pending default console output must not be reported again after it resolves into source items"
        );
    }

    #[test]
    fn runtime_observable_source_queue_projects_report_lifecycle_items() {
        let mut snapshot = None;
        let count =
            RendererRuntimeObservableSourceQueue::append_report_observable_items_to_snapshot(
                &mut snapshot,
                Some(9),
                &[
                    ScriptObservableOutputItem::ConsoleMessage("console ignored".to_owned()),
                    ScriptObservableOutputItem::LifecycleError("first report failure".to_owned()),
                    ScriptObservableOutputItem::LifecycleError("second report failure".to_owned()),
                ],
            );

        assert_eq!(count, 2);
        let source = snapshot.expect("report lifecycle items should produce source summary");
        assert_eq!(source.default_execution_context_id(), Some(9));
        assert_eq!(source.lifecycle_errors(), 2);
        assert_eq!(
            source.source_items(),
            &[
                RendererRuntimeObservableSourceItem::LifecycleError {
                    text: "first report failure".to_owned(),
                    execution_context_id: Some(9),
                    exception_index: 0,
                },
                RendererRuntimeObservableSourceItem::LifecycleError {
                    text: "second report failure".to_owned(),
                    execution_context_id: Some(9),
                    exception_index: 1,
                },
            ],
            "report-side lifecycle merge should preserve renderer producer source items"
        );
    }

    fn runtime_console_message(
        execution_context_id: i64,
        message: &str,
    ) -> RuntimeConsoleMessageSnapshot {
        RuntimeConsoleMessageSnapshot {
            execution_context_id,
            message: message.to_owned(),
            args: Vec::new(),
            stack: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RendererRuntimeInspectorAsyncCompletion {
    pub call_id: i32,
    pub output: RendererRuntimeCommandOutput,
    renderer_output_predecessor: Option<RendererOutputFence>,
}

impl RendererRuntimeInspectorAsyncCompletion {
    pub fn from_protocol_message(call_id: i32, message: Value) -> Self {
        Self {
            call_id,
            output: RendererRuntimeCommandOutput::from_inspector_message(
                RendererRuntimeInspectorMessage::from_v8_inspector_message(message),
            ),
            renderer_output_predecessor: None,
        }
    }

    pub fn from_command_output(call_id: i32, output: RendererRuntimeCommandOutput) -> Self {
        Self {
            call_id,
            output,
            renderer_output_predecessor: None,
        }
    }

    fn from_renderer_response_edge(
        call_id: i32,
        output: RendererRuntimeCommandOutput,
        renderer_output_predecessor: Option<RendererOutputFence>,
    ) -> Self {
        Self {
            call_id,
            output,
            renderer_output_predecessor,
        }
    }

    pub fn renderer_agent_attachment_id(&self) -> Option<RendererAgentAttachmentId> {
        self.output.renderer_agent_attachment_id()
    }

    /// Exact Page-output cursor committed before this response crossed the
    /// renderer owner boundary.
    ///
    /// The response is already correlated to one Runtime command. This cursor
    /// therefore needs to express only the concrete stream position that the
    /// protocol ingress must admit before exposing the response. It does not
    /// impose process-global ordering on unrelated Pages.
    pub fn renderer_output_predecessor(&self) -> Option<RendererOutputFence> {
        self.renderer_output_predecessor.clone()
    }
}

/// Concrete late Inspector response waiting for the renderer owner to finish
/// the Page turn that produced it.
///
/// A Promise may settle while an unrelated HTML task is still inside V8. That
/// task can also have produced protocol-visible Page facts, but their
/// publications are committed only when the task returns to the renderer
/// owner. Keeping the response as a one-shot value until that owner boundary
/// prevents the response watermark from overtaking those publications.
#[derive(Debug)]
pub(crate) struct RendererRuntimeInspectorResponsePublication {
    call_id: i32,
    output: RendererRuntimeCommandOutput,
    destination: RendererRuntimeInspectorResponseDestination,
}

#[derive(Clone, Debug)]
pub(crate) struct RendererRuntimeInspectorSessionResponseSettlement {
    predecessor: RendererOutputFence,
    response_succeeded: bool,
}

impl RendererRuntimeInspectorSessionResponseSettlement {
    fn new(predecessor: RendererOutputFence, response_succeeded: bool) -> Self {
        Self {
            predecessor,
            response_succeeded,
        }
    }

    pub(crate) fn into_parts(self) -> (RendererOutputFence, bool) {
        (self.predecessor, self.response_succeeded)
    }
}

impl RendererRuntimeInspectorResponsePublication {
    pub(crate) fn commit(
        self,
        renderer_output_predecessor: Option<RendererOutputFence>,
    ) -> Result<
        Option<RendererRuntimeInspectorSessionResponseSettlement>,
        RendererRuntimeInspectorAsyncCompletion,
    > {
        self.destination.send(
            RendererRuntimeInspectorAsyncCompletion::from_renderer_response_edge(
                self.call_id,
                self.output,
                renderer_output_predecessor,
            ),
        )
    }

    pub(crate) fn commit_error(
        mut self,
        message: String,
        renderer_output_predecessor: Option<RendererOutputFence>,
    ) -> Result<
        Option<RendererRuntimeInspectorSessionResponseSettlement>,
        RendererRuntimeInspectorAsyncCompletion,
    > {
        let error = RendererRuntimeInspectorMessage::from_v8_inspector_message(json!({
            "id": self.call_id,
            "error": {
                "code": -32000,
                "message": message,
            }
        }));
        let mut response_indices =
            self.output
                .messages
                .iter()
                .enumerate()
                .filter_map(|(index, candidate)| {
                    matches!(
                    candidate,
                    RendererRuntimeInspectorMessage::Protocol(protocol)
                        if protocol.renderer_call_id().is_some_and(|id| id.get() == self.call_id)
                )
                .then_some(index)
                });
        let response_index = response_indices
            .next()
            .expect("a parked session response must remain exactly once");
        assert!(
            response_indices.next().is_none(),
            "a parked session response must remain exactly once"
        );
        self.output.messages[response_index] = error;
        self.commit(renderer_output_predecessor)
    }
}

#[derive(Debug, Default)]
pub(crate) struct RendererRuntimeCommandOutputSettlement {
    output: RendererRuntimeCommandOutput,
    session_response: Option<RendererRuntimeInspectorResponsePublication>,
}

impl RendererRuntimeCommandOutputSettlement {
    fn command_reply(output: RendererRuntimeCommandOutput) -> Self {
        Self {
            output,
            session_response: None,
        }
    }

    fn session_response(publication: RendererRuntimeInspectorResponsePublication) -> Self {
        Self {
            output: RendererRuntimeCommandOutput::default(),
            session_response: Some(publication),
        }
    }

    pub(crate) fn append(&mut self, other: Self) {
        self.output.append(other.output);
        if let Some(session_response) = other.session_response {
            assert!(
                self.session_response.replace(session_response).is_none(),
                "one Page command cannot settle multiple terminal session responses"
            );
        }
    }

    pub(crate) fn has_session_response(&self) -> bool {
        self.session_response.is_some()
    }

    pub(crate) fn set_v8_state_update(&mut self, state: V8InspectorSessionState) {
        self.output.set_v8_state_update(state);
    }

    fn into_command_reply_output(self) -> RendererRuntimeCommandOutput {
        assert!(
            self.session_response.is_none(),
            "a DevTools session response cannot become a typed command reply"
        );
        self.output
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        RendererRuntimeCommandOutput,
        Option<RendererRuntimeInspectorResponsePublication>,
    ) {
        (self.output, self.session_response)
    }
}

#[derive(Clone)]
pub(crate) struct RendererRuntimeCommandOutputRecorder(
    Rc<RefCell<RendererRuntimeCommandOutputRecorderState>>,
);

struct RendererRuntimeCommandOutputRecorderState {
    causal_identity: RendererRuntimeCommandCausalIdentity,
    output: RendererRuntimeCommandOutput,
    response: Option<(RendererRuntimeInspectorResponseSender, Value)>,
}

impl std::fmt::Debug for RendererRuntimeCommandOutputRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.0.borrow();
        f.debug_struct("RendererRuntimeCommandOutputRecorder")
            .field("messages", &state.output.messages.len())
            .field("has_response", &state.response.is_some())
            .finish()
    }
}

impl RendererRuntimeCommandOutputRecorder {
    pub(crate) fn new(inspector_session_id: Option<String>, expected_call_id: i32) -> Self {
        Self(Rc::new(RefCell::new(
            RendererRuntimeCommandOutputRecorderState {
                causal_identity: RendererRuntimeCommandCausalIdentity::new(
                    inspector_session_id,
                    expected_call_id,
                ),
                output: RendererRuntimeCommandOutput::default(),
                response: None,
            },
        )))
    }

    pub(crate) fn owns_response(&self, call_id: i32) -> bool {
        self.0.borrow().causal_identity.call_id() == call_id
    }

    pub(crate) fn call_id(&self) -> i32 {
        self.0.borrow().causal_identity.call_id()
    }

    pub(crate) fn causal_identity(&self) -> RendererRuntimeCommandCausalIdentity {
        self.0.borrow().causal_identity.clone()
    }

    pub(crate) fn has_response(&self) -> bool {
        self.0.borrow().response.is_some()
    }

    pub(crate) fn response_succeeded(&self) -> bool {
        self.0
            .borrow()
            .response
            .as_ref()
            .is_some_and(|(_, message)| message.get("error").is_none())
    }

    pub(crate) fn push_inspector_message(&self, message: Value) {
        self.0.borrow_mut().output.push_inspector_message(message);
    }

    pub(crate) fn set_v8_state_update(&self, state: V8InspectorSessionState) {
        self.0.borrow_mut().output.set_v8_state_update(state);
    }

    pub(crate) fn park_response(
        &self,
        sender: RendererRuntimeInspectorResponseSender,
        message: Value,
    ) {
        let mut state = self.0.borrow_mut();
        debug_assert_eq!(state.causal_identity.call_id(), sender.call_id());
        assert!(
            state.response.is_none(),
            "one runtime command output scope cannot own multiple responses"
        );
        state.response = Some((sender, message));
    }

    pub(crate) fn finish(self) -> RendererRuntimeCommandOutput {
        self.finish_settlement_with_response_override(None)
            .into_command_reply_output()
    }

    pub(crate) fn finish_for_page(self) -> RendererRuntimeCommandOutputSettlement {
        self.finish_settlement_with_response_override(None)
    }

    pub(crate) fn finish_with_error_for_page(
        self,
        message: String,
    ) -> RendererRuntimeCommandOutputSettlement {
        self.finish_settlement_with_response_override(Some(message))
    }

    fn finish_settlement_with_response_override(
        self,
        error_message: Option<String>,
    ) -> RendererRuntimeCommandOutputSettlement {
        let mut state = self.0.borrow_mut();
        let mut output = std::mem::take(&mut state.output);
        let response = state.response.take();
        drop(state);
        let Some((sender, message)) = response else {
            return RendererRuntimeCommandOutputSettlement::command_reply(output);
        };
        let message = error_message.map_or(message, |message| {
            json!({
                "id": sender.call_id(),
                "error": {
                    "code": -32000,
                    "message": message,
                }
            })
        });
        output.push_inspector_message(message);
        sender.finalize_output(output)
    }
}

struct RendererRuntimeInspectorResponseChannelState {
    next_lease_id: u64,
    active_lease_id: Option<u64>,
    open: bool,
    tx: Option<oneshot::Sender<RendererRuntimeInspectorAsyncCompletion>>,
    session_response_settlement_tx:
        Option<oneshot::Sender<RendererRuntimeInspectorSessionResponseSettlement>>,
}

#[derive(Clone)]
pub struct RendererRuntimeInspectorResponseChannel {
    delivery: moli_page_types::RendererInspectorResponseDelivery,
    state: Arc<Mutex<RendererRuntimeInspectorResponseChannelState>>,
}

impl std::fmt::Debug for RendererRuntimeInspectorResponseChannel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.lock();
        formatter
            .debug_struct("RendererRuntimeInspectorResponseChannel")
            .field("delivery", &self.delivery)
            .field("active_lease_id", &state.active_lease_id)
            .field("has_receiver", &state.tx.is_some())
            .finish()
    }
}

impl PartialEq for RendererRuntimeInspectorResponseChannel {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

impl Eq for RendererRuntimeInspectorResponseChannel {}

impl RendererRuntimeInspectorResponseChannel {
    pub fn new() -> (
        Self,
        oneshot::Receiver<RendererRuntimeInspectorAsyncCompletion>,
    ) {
        let (tx, rx) = oneshot::channel();
        (
            Self {
                delivery: moli_page_types::RendererInspectorResponseDelivery::CommandReply,
                state: Arc::new(Mutex::new(RendererRuntimeInspectorResponseChannelState {
                    next_lease_id: 1,
                    active_lease_id: None,
                    open: true,
                    tx: Some(tx),
                    session_response_settlement_tx: None,
                })),
            },
            rx,
        )
    }

    pub fn new_for_delivery(
        delivery: moli_page_types::RendererInspectorResponseDelivery,
    ) -> (
        Self,
        Option<oneshot::Receiver<RendererRuntimeInspectorAsyncCompletion>>,
    ) {
        match delivery {
            moli_page_types::RendererInspectorResponseDelivery::CommandReply => {
                let (channel, receiver) = Self::new();
                (channel, Some(receiver))
            }
            moli_page_types::RendererInspectorResponseDelivery::DevToolsSession => (
                Self {
                    delivery,
                    state: Arc::new(Mutex::new(RendererRuntimeInspectorResponseChannelState {
                        next_lease_id: 1,
                        active_lease_id: None,
                        open: true,
                        tx: None,
                        session_response_settlement_tx: None,
                    })),
                },
                None,
            ),
        }
    }

    pub fn activate_sender(
        &self,
        call_id: i32,
        attachment_id: Option<RendererAgentAttachmentId>,
    ) -> RendererRuntimeInspectorResponseSender {
        self.try_activate_sender(call_id, attachment_id)
            .expect("cannot activate a renderer response lease after terminal completion")
    }

    pub fn try_activate_sender(
        &self,
        call_id: i32,
        attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Option<RendererRuntimeInspectorResponseSender> {
        let lease_id = {
            let mut state = self.state.lock();
            if !state.open {
                return None;
            }
            let lease_id = state.next_lease_id;
            state.next_lease_id = lease_id
                .checked_add(1)
                .expect("renderer response lease id exhausted");
            state.active_lease_id = Some(lease_id);
            lease_id
        };
        let lease = RendererRuntimeInspectorResponseLease::new(lease_id, self.clone());
        let destination = match self.delivery {
            moli_page_types::RendererInspectorResponseDelivery::CommandReply => {
                RendererRuntimeInspectorResponseDestination::CommandReply(lease)
            }
            moli_page_types::RendererInspectorResponseDelivery::DevToolsSession => {
                RendererRuntimeInspectorResponseDestination::DevToolsSessionPending(lease)
            }
        };
        Some(RendererRuntimeInspectorResponseSender {
            call_id,
            attachment_id,
            destination,
            publication_boundary: RendererRuntimeInspectorResponsePublicationBoundary::Immediate,
        })
    }

    /// Revokes the current renderer sender without settling the frontend call.
    ///
    /// Attachment replacement uses this to prevent the retired renderer from
    /// publishing while the protocol session still owns terminal completion.
    /// A response that already claimed the channel wins and returns `false`.
    pub fn try_revoke_active_lease(&self) -> bool {
        let mut state = self.state.lock();
        if !state.open || state.active_lease_id.is_none() {
            return false;
        }
        state.active_lease_id = None;
        true
    }

    pub fn cancel(&self) {
        let mut state = self.state.lock();
        state.open = false;
        state.active_lease_id = None;
        state.tx.take();
        state.session_response_settlement_tx.take();
    }

    fn send(
        &self,
        lease_id: u64,
        completion: RendererRuntimeInspectorAsyncCompletion,
    ) -> Result<(), RendererRuntimeInspectorAsyncCompletion> {
        let tx = {
            let mut state = self.state.lock();
            if state.active_lease_id != Some(lease_id) {
                return Err(completion);
            }
            state.open = false;
            state.active_lease_id = None;
            state.tx.take()
        };
        let Some(tx) = tx else {
            return Err(completion);
        };
        tx.send(completion)
    }

    fn take_session_response_settlement_receiver(
        &self,
        lease_id: u64,
    ) -> Option<oneshot::Receiver<RendererRuntimeInspectorSessionResponseSettlement>> {
        let mut state = self.state.lock();
        if !state.open
            || state.active_lease_id != Some(lease_id)
            || state.session_response_settlement_tx.is_some()
        {
            return None;
        }
        let (tx, rx) = oneshot::channel();
        state.session_response_settlement_tx = Some(tx);
        Some(rx)
    }

    fn cancel_lease(&self, lease_id: u64) -> bool {
        let mut state = self.state.lock();
        if !state.open || state.active_lease_id != Some(lease_id) {
            return false;
        }
        state.open = false;
        state.active_lease_id = None;
        state.tx.take();
        state.session_response_settlement_tx.take();
        true
    }

    fn claim_session_lease(
        &self,
        lease_id: u64,
    ) -> Result<Option<oneshot::Sender<RendererRuntimeInspectorSessionResponseSettlement>>, ()>
    {
        let mut state = self.state.lock();
        if !state.open || state.active_lease_id != Some(lease_id) {
            return Err(());
        }
        state.open = false;
        state.active_lease_id = None;
        Ok(state.session_response_settlement_tx.take())
    }
}

#[derive(Debug)]
struct RendererRuntimeInspectorResponseLeaseLifetime {
    lease_id: u64,
    channel: RendererRuntimeInspectorResponseChannel,
}

impl Drop for RendererRuntimeInspectorResponseLeaseLifetime {
    fn drop(&mut self) {
        self.channel.cancel_lease(self.lease_id);
    }
}

#[derive(Clone, Debug)]
struct RendererRuntimeInspectorResponseLease {
    lifetime: Arc<RendererRuntimeInspectorResponseLeaseLifetime>,
}

impl RendererRuntimeInspectorResponseLease {
    fn new(lease_id: u64, channel: RendererRuntimeInspectorResponseChannel) -> Self {
        Self {
            lifetime: Arc::new(RendererRuntimeInspectorResponseLeaseLifetime { lease_id, channel }),
        }
    }

    fn send_command_reply(
        self,
        completion: RendererRuntimeInspectorAsyncCompletion,
    ) -> Result<(), RendererRuntimeInspectorAsyncCompletion> {
        self.lifetime
            .channel
            .send(self.lifetime.lease_id, completion)
    }

    fn take_session_response_settlement_receiver(
        &self,
    ) -> Option<oneshot::Receiver<RendererRuntimeInspectorSessionResponseSettlement>> {
        self.lifetime
            .channel
            .take_session_response_settlement_receiver(self.lifetime.lease_id)
    }

    fn claim_session(
        &self,
    ) -> Result<Option<oneshot::Sender<RendererRuntimeInspectorSessionResponseSettlement>>, ()>
    {
        self.lifetime
            .channel
            .claim_session_lease(self.lifetime.lease_id)
    }
}

/// Output capability for one concrete renderer attachment and DevTools
/// session.
///
/// Navigation creates another Page journal and attachment identity; closing
/// the old journal structurally prevents an old session response from entering
/// the replacement stream.
#[derive(Clone, Debug)]
pub(crate) struct RendererDevToolsSessionOutputHost {
    agent_token: RendererDevToolsAgentToken,
    session: DevToolsSessionKey,
    attachment_id: RendererAgentAttachmentId,
    output_journal: RendererTurnOutputJournal,
}

impl RendererDevToolsSessionOutputHost {
    pub(crate) fn new(
        agent_token: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
        attachment_id: RendererAgentAttachmentId,
        output_journal: RendererTurnOutputJournal,
    ) -> Self {
        Self {
            agent_token,
            session,
            attachment_id,
            output_journal,
        }
    }

    fn publish(
        &self,
        completion: RendererRuntimeInspectorAsyncCompletion,
    ) -> Result<RendererOutputFence, RendererRuntimeInspectorAsyncCompletion> {
        if completion.renderer_agent_attachment_id() != Some(self.attachment_id) {
            return Err(completion);
        }
        if completion
            .renderer_output_predecessor()
            .is_some_and(|predecessor| {
                predecessor.cursor().stream() != self.output_journal.stream()
            })
        {
            return Err(completion);
        }

        // Keep the completion intact until the exact stream accepts
        // ownership. If the capability already closed, protocol attachment
        // teardown owns terminal completion for the pending frontend call.
        let (_, v8_state_update, messages) = completion.output.clone().into_parts();
        let mut batch = RendererRuntimeInspectorMessageBatch::new(
            self.agent_token,
            self.session.clone(),
            messages,
        );
        batch.v8_state_update = v8_state_update;
        batch.bind_renderer_agent_attachment(self.attachment_id);
        if !batch.has_resolved_source_identities() {
            return Err(completion);
        }
        let record = PendingRendererOutputRecord::observation(
            None,
            RendererProtocolObservation::RuntimeInspector(batch),
        );
        self.output_journal
            .try_publish_terminal_records_and_declare_fence([record])
            .ok_or(completion)
    }
}

#[derive(Clone, Debug)]
enum RendererRuntimeInspectorResponseDestination {
    CommandReply(RendererRuntimeInspectorResponseLease),
    DevToolsSessionPending(RendererRuntimeInspectorResponseLease),
    DevToolsSession {
        lease: RendererRuntimeInspectorResponseLease,
        host: RendererDevToolsSessionOutputHost,
    },
}

impl RendererRuntimeInspectorResponseDestination {
    fn send(
        self,
        completion: RendererRuntimeInspectorAsyncCompletion,
    ) -> Result<
        Option<RendererRuntimeInspectorSessionResponseSettlement>,
        RendererRuntimeInspectorAsyncCompletion,
    > {
        match self {
            Self::CommandReply(lease) => lease.send_command_reply(completion).map(|()| None),
            Self::DevToolsSessionPending(_) => Err(completion),
            Self::DevToolsSession { lease, host } => {
                let settlement_tx = match lease.claim_session() {
                    Ok(settlement_tx) => settlement_tx,
                    Err(()) => return Err(completion),
                };
                let mut responses =
                    completion
                        .output
                        .messages
                        .iter()
                        .filter_map(|message| match message {
                            RendererRuntimeInspectorMessage::Protocol(response)
                                if response.renderer_call_id().is_some_and(
                                    |renderer_call_id| renderer_call_id.get() == completion.call_id,
                                ) =>
                            {
                                Some(response.value())
                            }
                            _ => None,
                        });
                let Some(response) = responses.next() else {
                    return Err(completion);
                };
                if responses.next().is_some() {
                    return Err(completion);
                }
                let response_succeeded = response.get("error").is_none();
                let fence = host.publish(completion)?;
                let settlement = RendererRuntimeInspectorSessionResponseSettlement::new(
                    fence,
                    response_succeeded,
                );
                if let Some(settlement_tx) = settlement_tx {
                    let _ = settlement_tx.send(settlement.clone());
                }
                Ok(Some(settlement))
            }
        }
    }

    fn take_session_response_settlement_receiver(
        &self,
    ) -> Option<oneshot::Receiver<RendererRuntimeInspectorSessionResponseSettlement>> {
        match self {
            Self::CommandReply(_) => None,
            Self::DevToolsSessionPending(lease) | Self::DevToolsSession { lease, .. } => {
                lease.take_session_response_settlement_receiver()
            }
        }
    }
}

#[derive(Clone)]
enum RendererRuntimeInspectorResponsePublicationBoundary {
    Immediate,
    PageOwner(RendererOwnerWakeSender),
    SharedWorkerParent(tokio::sync::mpsc::UnboundedSender<crate::worker::WorkerToParentMessage>),
}

#[derive(Clone)]
pub struct RendererRuntimeInspectorResponseSender {
    call_id: i32,
    attachment_id: Option<RendererAgentAttachmentId>,
    destination: RendererRuntimeInspectorResponseDestination,
    publication_boundary: RendererRuntimeInspectorResponsePublicationBoundary,
}

impl std::fmt::Debug for RendererRuntimeInspectorResponseSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererRuntimeInspectorResponseSender")
            .field("call_id", &self.call_id)
            .finish_non_exhaustive()
    }
}

impl RendererRuntimeInspectorResponseSender {
    pub fn new(call_id: i32, tx: oneshot::Sender<RendererRuntimeInspectorAsyncCompletion>) -> Self {
        let channel = RendererRuntimeInspectorResponseChannel {
            delivery: moli_page_types::RendererInspectorResponseDelivery::CommandReply,
            state: Arc::new(Mutex::new(RendererRuntimeInspectorResponseChannelState {
                next_lease_id: 2,
                active_lease_id: Some(1),
                open: true,
                tx: Some(tx),
                session_response_settlement_tx: None,
            })),
        };
        Self {
            call_id,
            attachment_id: None,
            destination: RendererRuntimeInspectorResponseDestination::CommandReply(
                RendererRuntimeInspectorResponseLease::new(1, channel),
            ),
            publication_boundary: RendererRuntimeInspectorResponsePublicationBoundary::Immediate,
        }
    }

    pub fn call_id(&self) -> i32 {
        self.call_id
    }

    pub(crate) fn response_delivery(&self) -> moli_page_types::RendererInspectorResponseDelivery {
        match self.destination {
            RendererRuntimeInspectorResponseDestination::CommandReply(_) => {
                moli_page_types::RendererInspectorResponseDelivery::CommandReply
            }
            RendererRuntimeInspectorResponseDestination::DevToolsSessionPending(_)
            | RendererRuntimeInspectorResponseDestination::DevToolsSession { .. } => {
                moli_page_types::RendererInspectorResponseDelivery::DevToolsSession
            }
        }
    }

    pub(crate) fn take_session_response_settlement_receiver(
        &self,
    ) -> Option<oneshot::Receiver<RendererRuntimeInspectorSessionResponseSettlement>> {
        self.destination.take_session_response_settlement_receiver()
    }

    pub(crate) fn renderer_agent_attachment_id(&self) -> Option<RendererAgentAttachmentId> {
        self.attachment_id
    }

    pub fn with_renderer_agent_attachment(
        mut self,
        attachment_id: RendererAgentAttachmentId,
    ) -> Self {
        self.attachment_id = Some(attachment_id);
        self
    }

    pub(crate) fn defer_publication_to_page_owner(
        mut self,
        owner_wake: RendererOwnerWakeSender,
    ) -> Self {
        self.publication_boundary =
            RendererRuntimeInspectorResponsePublicationBoundary::PageOwner(owner_wake);
        self
    }

    pub(crate) fn defer_publication_to_shared_worker_parent(
        mut self,
        parent_tx: tokio::sync::mpsc::UnboundedSender<crate::worker::WorkerToParentMessage>,
    ) -> Self {
        self.publication_boundary =
            RendererRuntimeInspectorResponsePublicationBoundary::SharedWorkerParent(parent_tx);
        self
    }

    /// Selects the frontend session output capability without changing when
    /// the response is allowed to publish.
    pub(crate) fn route_to_devtools_session_output(
        mut self,
        host: RendererDevToolsSessionOutputHost,
    ) -> Self {
        assert_eq!(
            self.attachment_id,
            Some(host.attachment_id),
            "a DevTools session response host must belong to the command attachment"
        );
        self.destination = match self.destination {
            RendererRuntimeInspectorResponseDestination::DevToolsSessionPending(lease) => {
                RendererRuntimeInspectorResponseDestination::DevToolsSession { lease, host }
            }
            RendererRuntimeInspectorResponseDestination::DevToolsSession { lease, host: _ } => {
                RendererRuntimeInspectorResponseDestination::DevToolsSession { lease, host }
            }
            RendererRuntimeInspectorResponseDestination::CommandReply(_) => {
                panic!("a command-reply response cannot be rebound to DevTools session output")
            }
        };
        self
    }

    pub fn send(self, message: Value) -> Result<(), RendererRuntimeInspectorAsyncCompletion> {
        self.send_output(RendererRuntimeCommandOutput::from_inspector_message(
            RendererRuntimeInspectorMessage::from_v8_inspector_message(message),
        ))
    }

    pub fn send_output(
        self,
        mut output: RendererRuntimeCommandOutput,
    ) -> Result<(), RendererRuntimeInspectorAsyncCompletion> {
        let Self {
            call_id,
            attachment_id,
            destination,
            publication_boundary,
        } = self;
        if let Some(attachment_id) = attachment_id {
            output.bind_renderer_agent_attachment(attachment_id);
        }
        let publication = RendererRuntimeInspectorResponsePublication {
            call_id,
            output,
            destination,
        };
        match publication_boundary {
            RendererRuntimeInspectorResponsePublicationBoundary::Immediate => {
                publication.commit(None).map(|_| ())
            }
            RendererRuntimeInspectorResponsePublicationBoundary::PageOwner(owner_wake) => {
                owner_wake.defer_runtime_inspector_response_publication(publication)
            }
            RendererRuntimeInspectorResponsePublicationBoundary::SharedWorkerParent(parent_tx) => {
                match parent_tx.send(
                    crate::worker::WorkerToParentMessage::SharedWorkerRuntimeInspectorResponse(
                        publication,
                    ),
                ) {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        let crate::worker::WorkerToParentMessage::
                            SharedWorkerRuntimeInspectorResponse(publication) = error.0
                        else {
                            unreachable!("the failed send must return the submitted publication")
                        };
                        publication.commit(None).map(|_| ())
                    }
                }
            }
        }
    }

    pub(crate) fn finalize_output(
        self,
        mut output: RendererRuntimeCommandOutput,
    ) -> RendererRuntimeCommandOutputSettlement {
        if let Some(attachment_id) = self.attachment_id {
            output.bind_renderer_agent_attachment(attachment_id);
        }
        let Self {
            call_id,
            attachment_id: _,
            destination,
            publication_boundary: _,
        } = self;
        match destination {
            RendererRuntimeInspectorResponseDestination::CommandReply(_) => {
                RendererRuntimeCommandOutputSettlement::command_reply(output)
            }
            RendererRuntimeInspectorResponseDestination::DevToolsSessionPending(_) => panic!(
                "a DevTools session response must bind its concrete output host before settlement"
            ),
            RendererRuntimeInspectorResponseDestination::DevToolsSession { lease, host } => {
                RendererRuntimeCommandOutputSettlement::session_response(
                    RendererRuntimeInspectorResponsePublication {
                        call_id,
                        output,
                        destination: RendererRuntimeInspectorResponseDestination::DevToolsSession {
                            lease,
                            host,
                        },
                    },
                )
            }
        }
    }
}

#[cfg(test)]
mod renderer_runtime_inspector_response_channel_tests {
    use super::*;
    use crate::page_task_queue::{RendererOwnerWake, RendererOwnerWakeSender};
    use crate::runtime::protocol_output::{
        RendererOutputTransportTestLimits, renderer_output_transport_channel_with_test_limits,
    };
    use tokio::sync::oneshot::error::TryRecvError;

    fn output(call_id: i32) -> RendererRuntimeCommandOutput {
        RendererRuntimeCommandOutput::from_inspector_message(
            RendererRuntimeInspectorMessage::protocol(json!({
                "id": call_id,
                "result": {},
            })),
        )
    }

    fn output_with_by_value_payload(
        call_id: i32,
        payload_bytes: usize,
    ) -> RendererRuntimeCommandOutput {
        RendererRuntimeCommandOutput::from_inspector_message(
            RendererRuntimeInspectorMessage::protocol(json!({
                "id": call_id,
                "result": {
                    "result": {
                        "type": "string",
                        "value": "x".repeat(payload_bytes),
                    }
                },
            })),
        )
    }

    fn observation_publication(
        stream: RendererOutputStreamIdentity,
        sequence: u64,
    ) -> RendererOutputTransportMessage {
        let record = RendererOutputRecord::new_for_test(RendererOutputItem::Observation(
            RendererProtocolObservation::RuntimeLifecycleError {
                text: String::new(),
                execution_context_id: None,
            },
        ));
        RendererOutputPublication::new_for_test(
            RendererOutputCursor::new_for_test(stream, sequence),
            vec![record],
        )
        .into()
    }

    #[tokio::test]
    async fn rotating_response_lease_rejects_old_sender_and_completes_new_sender_once() {
        let (channel, rx) = RendererRuntimeInspectorResponseChannel::new();
        let old_attachment = RendererAgentAttachmentId::allocate();
        let new_attachment = RendererAgentAttachmentId::allocate();
        let old_sender = channel.activate_sender(1, Some(old_attachment));
        let new_sender = channel.activate_sender(2, Some(new_attachment));

        let stale = old_sender
            .send_output(output(1))
            .expect_err("old response lease must be stale after rotation");
        assert_eq!(stale.call_id, 1);
        new_sender
            .send_output(output(2))
            .expect("current response lease should complete");

        let completion = rx.await.expect("frontend receiver should complete once");
        assert_eq!(completion.call_id, 2);
        assert_eq!(
            completion.output.renderer_agent_attachment_id(),
            Some(new_attachment)
        );
    }

    #[tokio::test]
    async fn dropping_last_active_sender_closes_frontend_receiver() {
        let (channel, rx) = RendererRuntimeInspectorResponseChannel::new();
        drop(channel.activate_sender(1, None));

        assert!(rx.await.is_err());
        assert!(channel.try_activate_sender(2, None).is_none());
    }

    #[tokio::test]
    async fn cancel_closes_frontend_receiver_and_invalidates_active_sender() {
        let (channel, rx) = RendererRuntimeInspectorResponseChannel::new();
        let sender = channel.activate_sender(1, None);

        channel.cancel();

        assert!(sender.send_output(output(1)).is_err());
        assert!(rx.await.is_err());
    }

    #[tokio::test]
    async fn revoking_session_lease_leaves_terminal_settlement_to_protocol_owner() {
        let attachment = RendererAgentAttachmentId::allocate();
        let (channel, receiver) = RendererRuntimeInspectorResponseChannel::new_for_delivery(
            moli_page_types::RendererInspectorResponseDelivery::DevToolsSession,
        );
        assert!(receiver.is_none());
        let sender = channel.activate_sender(1, Some(attachment));
        let mut settlement_rx = sender
            .take_session_response_settlement_receiver()
            .expect("the frontend call should retain a settlement waiter");

        assert!(channel.try_revoke_active_lease());
        assert!(
            !channel.try_revoke_active_lease(),
            "one attachment replacement must revoke the lease only once"
        );
        assert!(sender.send_output(output(1)).is_err());
        assert!(
            matches!(settlement_rx.try_recv(), Err(TryRecvError::Empty)),
            "lease revocation must not settle the frontend call before its protocol error",
        );

        channel.cancel();
        assert!(settlement_rx.await.is_err());
    }

    #[tokio::test]
    async fn completed_response_lease_cannot_be_rotated() {
        let (channel, rx) = RendererRuntimeInspectorResponseChannel::new();
        channel
            .activate_sender(1, None)
            .send_output(output(1))
            .expect("active response lease should complete");

        assert!(
            !channel.try_revoke_active_lease(),
            "attachment replacement must not revoke a response that already won"
        );
        assert!(channel.try_activate_sender(2, None).is_none());
        assert_eq!(rx.await.unwrap().call_id, 1);
    }

    #[tokio::test]
    async fn direct_response_has_no_implicit_process_global_predecessor() {
        let (channel, rx) = RendererRuntimeInspectorResponseChannel::new();

        channel
            .activate_sender(1, None)
            .send_output(output(1))
            .expect("active response lease should complete");

        assert_eq!(
            rx.await
                .expect("frontend receiver should complete")
                .renderer_output_predecessor(),
            None,
            "a response outside a Page owner turn must not infer a predecessor from unrelated process-global traffic"
        );
    }

    #[tokio::test]
    async fn response_preserves_only_the_exact_owner_supplied_cursor() {
        let stream =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(1));
        let expected_predecessor =
            RendererOutputFence::new_for_test(RendererOutputCursor::new_for_test(stream, 7));
        let (channel, rx) = RendererRuntimeInspectorResponseChannel::new();
        let sender = channel.activate_sender(1, None);
        let RendererRuntimeInspectorResponseSender {
            call_id,
            attachment_id,
            destination,
            publication_boundary: _,
        } = sender;
        let mut output = output(1);
        if let Some(attachment_id) = attachment_id {
            output.bind_renderer_agent_attachment(attachment_id);
        }

        RendererRuntimeInspectorResponsePublication {
            call_id,
            output,
            destination,
        }
        .commit(Some(expected_predecessor.clone()))
        .expect("active response lease should complete");

        assert_eq!(
            rx.await
                .expect("frontend receiver should complete")
                .renderer_output_predecessor(),
            Some(expected_predecessor),
            "a response must retain its exact Page-stream predecessor"
        );
    }

    #[tokio::test]
    async fn late_response_waits_for_page_owner_publication() {
        let page_id = PageId::new_for_testing(1);
        let token = RendererPageToken::new_for_testing(page_id);
        let (owner_wake_tx, mut owner_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = RendererOwnerWakeSender::new(owner_wake_tx, token);
        let (channel, mut response_rx) = RendererRuntimeInspectorResponseChannel::new();

        channel
            .activate_sender(1, None)
            .defer_publication_to_page_owner(owner_wake)
            .send_output(output(1))
            .expect("the Page owner should accept the concrete late response");
        assert_eq!(
            response_rx.try_recv(),
            Err(TryRecvError::Empty),
            "a late response must not publish before the Page owner boundary"
        );
        let response_publication = match owner_wake_rx
            .recv()
            .await
            .expect("owner should receive the concrete response residence")
        {
            RendererOwnerWake::RuntimeInspectorResponsePublication {
                token: actual,
                publication,
            } => {
                assert_eq!(actual, token);
                publication
            }
            other => panic!("expected a Runtime response publication, got {other:?}"),
        };

        let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(page_id);
        let predecessor =
            RendererOutputFence::new_for_test(RendererOutputCursor::new_for_test(stream, 3));

        response_publication
            .commit(Some(predecessor.clone()))
            .expect("frontend response receiver should remain live");
        let completion = response_rx
            .await
            .expect("owner-committed response should complete");
        assert!(
            completion.renderer_output_predecessor() == Some(predecessor),
            "the response must carry the exact output cursor supplied at the Page owner boundary"
        );
    }

    #[tokio::test]
    async fn session_output_changes_the_sink_without_bypassing_the_page_owner_boundary() {
        let page_id = PageId::new_for_testing(1);
        let token = RendererPageToken::new_for_testing(page_id);
        let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(page_id);
        let (transport, mut transport_rx) = crate::runtime::renderer_output_transport_channel();
        let journal = RendererTurnOutputJournal::new_with_transport(stream, transport);
        assert!(matches!(
            transport_rx.try_recv(),
            Ok(RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Opened { stream: opened }
            )) if opened == stream
        ));

        let attachment_id = RendererAgentAttachmentId::allocate();
        let host = RendererDevToolsSessionOutputHost::new(
            RendererDevToolsAgentToken::allocate(),
            DevToolsSessionKey::Attached("SID-session-output".to_owned()),
            attachment_id,
            journal,
        );
        let (owner_wake_tx, mut owner_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let owner_wake = RendererOwnerWakeSender::new(owner_wake_tx, token);
        let (channel, response_rx) = RendererRuntimeInspectorResponseChannel::new_for_delivery(
            moli_page_types::RendererInspectorResponseDelivery::DevToolsSession,
        );
        assert!(
            response_rx.is_none(),
            "session delivery must not allocate a legacy command-reply receiver"
        );

        channel
            .activate_sender(7, Some(attachment_id))
            .route_to_devtools_session_output(host)
            .defer_publication_to_page_owner(owner_wake)
            .send_output(output(7))
            .expect("Page owner should accept the terminal session response");

        assert!(
            transport_rx.try_recv().is_err(),
            "selecting session output must not publish before the Page owner boundary"
        );

        let publication = match owner_wake_rx
            .recv()
            .await
            .expect("Page owner response wake")
        {
            RendererOwnerWake::RuntimeInspectorResponsePublication {
                token: actual,
                publication,
            } => {
                assert_eq!(actual, token);
                publication
            }
            other => panic!("expected a Runtime response publication, got {other:?}"),
        };
        publication
            .commit(None)
            .expect("the exact attachment stream should accept the response");

        let RendererOutputTransportMessage::Publication(publication) = transport_rx
            .recv()
            .await
            .expect("session output publication")
        else {
            panic!("expected a session output publication");
        };
        let [record] = publication.records() else {
            panic!("session response should publish exactly one record");
        };
        let RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(batch)) =
            record.item()
        else {
            panic!("session response should use the Runtime Inspector output stream");
        };
        assert_eq!(batch.renderer_agent_attachment_id(), Some(attachment_id));
        assert_eq!(batch.messages.len(), 1);
        let RendererRuntimeInspectorMessage::Protocol(message) = &batch.messages[0] else {
            panic!("session response must remain a protocol message");
        };
        assert_eq!(message.value()["id"], json!(7));
        channel.cancel();
    }

    #[tokio::test]
    async fn session_terminal_response_uses_essential_reserve_after_observation_saturation() {
        let page_id = PageId::new_for_testing(2);
        let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(page_id);
        let limits = RendererOutputTransportTestLimits {
            max_pending_messages: 4,
            max_pending_bytes: 24 * 1024,
            max_observation_messages: 1,
            max_observation_bytes: 4 * 1024,
            max_message_bytes: 16 * 1024,
        };
        let (transport, mut transport_rx) =
            renderer_output_transport_channel_with_test_limits(limits);
        let journal = RendererTurnOutputJournal::new_with_transport(stream, transport.clone());
        assert!(matches!(
            transport_rx.recv().await,
            Some(RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Opened { stream: opened }
            )) if opened == stream
        ));

        let filler_stream = RendererOutputStreamIdentity::new_shared_worker_for_protocol_test(2002);
        transport
            .send(observation_publication(filler_stream, 1))
            .expect("the first observation should fill its reserved quota");
        assert_eq!(transport.diagnostics().pending_observation_messages, 1);
        assert_eq!(
            transport.diagnostics().pending_observation_bytes,
            limits.max_observation_bytes
        );

        let attachment = RendererAgentAttachmentId::allocate();
        let (channel, receiver) = RendererRuntimeInspectorResponseChannel::new_for_delivery(
            moli_page_types::RendererInspectorResponseDelivery::DevToolsSession,
        );
        assert!(receiver.is_none());
        let sender = channel
            .activate_sender(29, Some(attachment))
            .route_to_devtools_session_output(RendererDevToolsSessionOutputHost::new(
                RendererDevToolsAgentToken::allocate(),
                DevToolsSessionKey::Attached("SID-essential-terminal".to_owned()),
                attachment,
                journal,
            ));
        let settlement_rx = sender
            .take_session_response_settlement_receiver()
            .expect("the session owner should await terminal response admission");

        sender
            .send_output(output_with_by_value_payload(29, 3 * 1024))
            .expect("terminal response should use the essential reserve");
        let settlement = settlement_rx
            .await
            .expect("successful transport admission should settle the session call");
        let (fence, response_succeeded) = settlement.into_parts();
        assert!(response_succeeded);
        assert_eq!(transport.diagnostics().pending_observation_messages, 1);
        assert!(
            transport.diagnostics().admitted_essential_messages >= 3,
            "stream open, terminal response, and its cursor lease should use essential admission"
        );
        assert!(!transport.diagnostics().terminal);

        assert!(matches!(
            transport_rx.recv().await,
            Some(RendererOutputTransportMessage::Publication(publication))
                if publication.cursor().stream() == filler_stream
        ));
        let terminal_publication = match transport_rx
            .recv()
            .await
            .expect("terminal response publication")
        {
            RendererOutputTransportMessage::Publication(publication) => publication,
            other => panic!("expected terminal response publication, got {other:?}"),
        };
        assert_eq!(terminal_publication.cursor(), fence.cursor());
        let [record] = terminal_publication.records() else {
            panic!("terminal response should remain one atomic record");
        };
        let RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(batch)) =
            record.item()
        else {
            panic!("terminal response should remain a Runtime Inspector observation");
        };
        let [RendererRuntimeInspectorMessage::Protocol(response)] = batch.messages.as_slice()
        else {
            panic!("terminal publication should contain exactly one protocol response");
        };
        assert_eq!(response.value()["id"], json!(29));
        assert!(matches!(
            transport_rx.recv().await,
            Some(RendererOutputTransportMessage::CursorLeaseDeclared { cursor, .. })
                if cursor == fence.cursor()
        ));
        drop(fence);
    }

    #[tokio::test]
    async fn rejected_session_terminal_response_does_not_settle_the_frontend_call() {
        let page_id = PageId::new_for_testing(3);
        let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(page_id);
        let (transport, mut transport_rx) =
            renderer_output_transport_channel_with_test_limits(RendererOutputTransportTestLimits {
                max_pending_messages: 1,
                max_pending_bytes: 16 * 1024,
                max_observation_messages: 0,
                max_observation_bytes: 4 * 1024,
                max_message_bytes: 16 * 1024,
            });
        // Keep the essential Opened message pending so the total message
        // budget rejects the following terminal response.
        let journal = RendererTurnOutputJournal::new_with_transport(stream, transport.clone());
        let attachment = RendererAgentAttachmentId::allocate();
        let (channel, receiver) = RendererRuntimeInspectorResponseChannel::new_for_delivery(
            moli_page_types::RendererInspectorResponseDelivery::DevToolsSession,
        );
        assert!(receiver.is_none());
        let sender = channel
            .activate_sender(31, Some(attachment))
            .route_to_devtools_session_output(RendererDevToolsSessionOutputHost::new(
                RendererDevToolsAgentToken::allocate(),
                DevToolsSessionKey::Attached("SID-rejected-terminal".to_owned()),
                attachment,
                journal,
            ));
        let settlement_rx = sender
            .take_session_response_settlement_receiver()
            .expect("the session owner should await terminal response admission");

        let rejected = sender
            .send_output(output(31))
            .expect_err("a full total budget must reject the terminal response");
        assert_eq!(rejected.call_id, 31);
        assert!(transport.diagnostics().terminal);
        assert!(
            settlement_rx.await.is_err(),
            "transport rejection must close rather than satisfy the settlement waiter"
        );
        assert!(channel.try_activate_sender(32, Some(attachment)).is_none());
        assert!(matches!(
            transport_rx.recv().await,
            Some(RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Opened { stream: opened }
            )) if opened == stream
        ));
        assert_eq!(transport_rx.recv().await, None);
    }

    #[tokio::test]
    async fn unbound_session_terminal_response_is_not_deferred_or_settled() {
        let page_id = PageId::new_for_testing(4);
        let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(page_id);
        let journal = RendererTurnOutputJournal::new(stream);
        let attachment = RendererAgentAttachmentId::allocate();
        let (channel, receiver) = RendererRuntimeInspectorResponseChannel::new_for_delivery(
            moli_page_types::RendererInspectorResponseDelivery::DevToolsSession,
        );
        assert!(receiver.is_none());
        let sender = channel
            .activate_sender(33, Some(attachment))
            .route_to_devtools_session_output(RendererDevToolsSessionOutputHost::new(
                RendererDevToolsAgentToken::allocate(),
                DevToolsSessionKey::Attached("SID-unbound-terminal".to_owned()),
                attachment,
                journal.clone(),
            ));
        let settlement_rx = sender
            .take_session_response_settlement_receiver()
            .expect("the session owner should await terminal response admission");

        let rejected = sender
            .send_output(output(33))
            .expect_err("an unbound transport cannot admit a terminal response");
        assert_eq!(rejected.call_id, 33);
        assert!(settlement_rx.await.is_err());
        assert_eq!(journal.last_published_cursor(), None);
        assert_eq!(journal.pending_len(), 0);
    }

    #[tokio::test]
    async fn rotating_session_lease_rejects_stale_bound_sender_and_publishes_once() {
        let old_stream =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(11));
        let new_stream =
            RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(12));
        let (old_transport, mut old_rx) = crate::runtime::renderer_output_transport_channel();
        let (new_transport, mut new_rx) = crate::runtime::renderer_output_transport_channel();
        let old_journal = RendererTurnOutputJournal::new_with_transport(old_stream, old_transport);
        let new_journal = RendererTurnOutputJournal::new_with_transport(new_stream, new_transport);
        assert!(matches!(
            old_rx.try_recv(),
            Ok(RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Opened { stream }
            )) if stream == old_stream
        ));
        assert!(matches!(
            new_rx.try_recv(),
            Ok(RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Opened { stream }
            )) if stream == new_stream
        ));

        let agent = RendererDevToolsAgentToken::allocate();
        let session = DevToolsSessionKey::Attached("SID-rotated-session".to_owned());
        let old_attachment = RendererAgentAttachmentId::allocate();
        let new_attachment = RendererAgentAttachmentId::allocate();
        let (channel, receiver) = RendererRuntimeInspectorResponseChannel::new_for_delivery(
            moli_page_types::RendererInspectorResponseDelivery::DevToolsSession,
        );
        assert!(receiver.is_none());
        let old_sender = channel
            .activate_sender(1, Some(old_attachment))
            .route_to_devtools_session_output(RendererDevToolsSessionOutputHost::new(
                agent,
                session.clone(),
                old_attachment,
                old_journal,
            ));
        let settlement_rx = old_sender
            .take_session_response_settlement_receiver()
            .expect("the original route should own the frontend call settlement");
        let new_sender = channel
            .activate_sender(2, Some(new_attachment))
            .route_to_devtools_session_output(RendererDevToolsSessionOutputHost::new(
                agent,
                session,
                new_attachment,
                new_journal,
            ));

        assert!(
            old_sender.send_output(output(1)).is_err(),
            "rotating the response lease must invalidate a fully bound old session sender"
        );
        assert!(old_rx.try_recv().is_err());
        assert!(
            new_sender
                .take_session_response_settlement_receiver()
                .is_none(),
            "a replay route must not replace the original frontend call waiter"
        );
        new_sender
            .send_output(output(2))
            .expect("the current bound session sender should publish");
        let (_, response_succeeded) = settlement_rx
            .await
            .expect("the published session response should settle its waiter")
            .into_parts();
        assert!(response_succeeded);
        assert!(matches!(
            new_rx.recv().await,
            Some(RendererOutputTransportMessage::Publication(_))
        ));
        assert!(
            channel
                .try_activate_sender(3, Some(new_attachment))
                .is_none(),
            "a published session response must close the terminal lease"
        );
        while let Ok(message) = new_rx.try_recv() {
            assert!(
                !matches!(message, RendererOutputTransportMessage::Publication(_)),
                "settling the session lease must not publish a second response"
            );
        }
    }

    #[tokio::test]
    async fn owner_failure_replaces_parked_session_result_with_exactly_one_error() {
        let page_id = PageId::new_for_testing(21);
        let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(page_id);
        let (transport, mut transport_rx) = crate::runtime::renderer_output_transport_channel();
        let journal = RendererTurnOutputJournal::new_with_transport(stream, transport);
        assert!(matches!(
            transport_rx.try_recv(),
            Ok(RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Opened { stream: opened }
            )) if opened == stream
        ));
        let attachment = RendererAgentAttachmentId::allocate();
        let (channel, receiver) = RendererRuntimeInspectorResponseChannel::new_for_delivery(
            moli_page_types::RendererInspectorResponseDelivery::DevToolsSession,
        );
        assert!(receiver.is_none());
        let sender = channel
            .activate_sender(17, Some(attachment))
            .route_to_devtools_session_output(RendererDevToolsSessionOutputHost::new(
                RendererDevToolsAgentToken::allocate(),
                DevToolsSessionKey::Attached("SID-owner-error".to_owned()),
                attachment,
                journal,
            ));
        let stale_sender = sender.clone();
        let (command_output, publication) = sender.finalize_output(output(17)).into_parts();
        assert!(command_output.is_empty());
        let publication =
            publication.expect("session sender must produce a parked session publication");

        let (_, response_succeeded) = publication
            .commit_error("page state capture failed".to_owned(), None)
            .expect("owner failure should publish through the terminal session sink")
            .expect("session output should return its concrete settlement")
            .into_parts();
        assert!(!response_succeeded);
        let RendererOutputTransportMessage::Publication(publication) = transport_rx
            .recv()
            .await
            .expect("one owner error publication")
        else {
            panic!("owner error must publish a protocol observation");
        };
        let [record] = publication.records() else {
            panic!("owner error must contain exactly one response record");
        };
        let RendererOutputItem::Observation(RendererProtocolObservation::RuntimeInspector(batch)) =
            record.item()
        else {
            panic!("owner error must use the Runtime Inspector output stream");
        };
        let [RendererRuntimeInspectorMessage::Protocol(message)] = batch.messages.as_slice() else {
            panic!("owner error must contain exactly one protocol response");
        };
        assert_eq!(message.value()["id"], json!(17));
        assert_eq!(
            message.value()["error"]["message"],
            json!("page state capture failed")
        );
        assert!(
            stale_sender.send_output(output(17)).is_err(),
            "the settled session lease must reject a duplicate terminal response"
        );
        while let Ok(message) = transport_rx.try_recv() {
            assert!(
                !matches!(message, RendererOutputTransportMessage::Publication(_)),
                "rejecting a stale sender must not publish a second response"
            );
        }
    }

    #[tokio::test]
    async fn page_settlement_keeps_nonresponse_output_beside_session_response() {
        let page_id = PageId::new_for_testing(22);
        let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(page_id);
        let (transport, mut transport_rx) = crate::runtime::renderer_output_transport_channel();
        let journal = RendererTurnOutputJournal::new_with_transport(stream, transport);
        assert!(matches!(
            transport_rx.try_recv(),
            Ok(RendererOutputTransportMessage::StreamControl(
                RendererOutputStreamControl::Opened { stream: opened }
            )) if opened == stream
        ));
        let attachment = RendererAgentAttachmentId::allocate();
        let (channel, receiver) = RendererRuntimeInspectorResponseChannel::new_for_delivery(
            moli_page_types::RendererInspectorResponseDelivery::DevToolsSession,
        );
        assert!(receiver.is_none());
        let sender = channel
            .activate_sender(23, Some(attachment))
            .route_to_devtools_session_output(RendererDevToolsSessionOutputHost::new(
                RendererDevToolsAgentToken::allocate(),
                DevToolsSessionKey::Attached("SID-page-settlement".to_owned()),
                attachment,
                journal,
            ));

        let session_response = sender.finalize_output(output(23));
        let nonresponse_output = RendererRuntimeCommandOutput::from_inspector_message(
            RendererRuntimeInspectorMessage::protocol(json!({
                "method": "Debugger.scriptParsed",
                "params": { "scriptId": "before-response" },
            })),
        );
        let mut accumulated = RendererRuntimeCommandOutputSettlement::default();
        accumulated.append(session_response);
        accumulated.append(RendererRuntimeCommandOutputSettlement::command_reply(
            nonresponse_output,
        ));
        assert!(accumulated.has_session_response());

        let (command_output, publication) = accumulated.into_parts();
        let [RendererRuntimeInspectorMessage::Protocol(notification)] = command_output.messages()
        else {
            panic!("the Page settlement must retain its nonresponse output");
        };
        assert_eq!(
            notification.value()["method"],
            json!("Debugger.scriptParsed")
        );
        let (_, response_succeeded) = publication
            .expect("the session response must remain parked beside Page output")
            .commit(None)
            .expect("the parked session response must publish")
            .expect("session publication must report its concrete fence")
            .into_parts();
        assert!(response_succeeded);
        assert!(matches!(
            transport_rx.recv().await,
            Some(RendererOutputTransportMessage::Publication(_))
        ));
    }

    #[test]
    #[should_panic(expected = "one Page command cannot settle multiple terminal session responses")]
    fn page_settlement_rejects_two_terminal_session_responses() {
        fn parked_session_response(
            call_id: i32,
            page_id: u64,
        ) -> RendererRuntimeCommandOutputSettlement {
            let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(
                PageId::new_for_testing(page_id),
            );
            let (transport, _transport_rx) = crate::runtime::renderer_output_transport_channel();
            let attachment = RendererAgentAttachmentId::allocate();
            let (channel, receiver) = RendererRuntimeInspectorResponseChannel::new_for_delivery(
                moli_page_types::RendererInspectorResponseDelivery::DevToolsSession,
            );
            assert!(receiver.is_none());
            channel
                .activate_sender(call_id, Some(attachment))
                .route_to_devtools_session_output(RendererDevToolsSessionOutputHost::new(
                    RendererDevToolsAgentToken::allocate(),
                    DevToolsSessionKey::Attached(format!("SID-duplicate-{call_id}")),
                    attachment,
                    RendererTurnOutputJournal::new_with_transport(stream, transport),
                ))
                .finalize_output(output(call_id))
        }

        let mut accumulated = RendererRuntimeCommandOutputSettlement::default();
        accumulated.append(parked_session_response(24, 24));
        accumulated.append(parked_session_response(25, 25));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererRuntimeRealmInfo {
    pub context_id: i64,
    pub realm_id: Option<String>,
    pub frame_id: Option<String>,
    pub origin: String,
    pub name: String,
    pub is_default: bool,
    pub context_type: String,
    pub grant_universal_access: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererDocumentNodeAttributesResolution {
    Found(Vec<(String, String)>),
    NotElement,
    MissingNode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererDocumentNodeTextResolution {
    Found(String),
    MissingNode,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RendererDocumentNodePropertyResolution {
    Found(Value),
    NotElement,
    MissingNode,
}

pub struct RendererAccessibilityPayloadsForObjectId {
    pub frame_id: Option<String>,
    pub payloads: Option<Vec<Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererDocumentQuerySelectorResolution {
    Found(Vec<RendererDocumentQuerySelectorNode>),
    MissingRoot,
    InvalidSelector(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RendererDocumentQuerySelectorNode {
    pub live_node_id: crate::dom::NodeId,
    pub frontend_node_id: u32,
    pub backend_node_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RendererDocumentNodeReference {
    pub node_id: u32,
    pub backend_node_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RendererDomSearchResultNode {
    pub frontend_node_id: u32,
    pub backend_node_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererDomSearchRegistration {
    pub search_id: String,
    pub result_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererDomNodeCreationStackTrace {
    pub call_frames: Vec<RendererDomNodeCreationStackFrame>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererDomNodeCreationStackFrame {
    pub function_name: String,
    pub script_id: String,
    pub url: String,
    pub line_number: u64,
    pub column_number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererDomNodeStackTraceResolution {
    Found(Option<RendererDomNodeCreationStackTrace>),
    MissingNode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererDomSearchResultsResolution {
    Found(Vec<RendererDomSearchResultNode>),
    SearchResultNotFound,
    BadIndices,
    BadFromIndex,
    BadToIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererDomFrontendNodeBindingResolution {
    BackendNodeId(u32),
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererDocumentFrontendNodeIdsResolution {
    Found(Vec<Option<u32>>),
    DocumentNotBound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererDomBidiNodeBindingResolution {
    BackendNodeId(u32),
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererDomBidiNodeSharedIdResolution {
    SharedId(String),
    NotFound,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RendererDocumentQuerySelectorWithChildNodeSnapshotEvents {
    pub child_node_snapshot_events: Option<RendererDocumentChildNodeSnapshotEvents>,
    pub query_selector_resolution: RendererDocumentQuerySelectorResolution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererStyleSheetPayload {
    pub text: String,
    pub title: String,
    pub disabled: bool,
    pub source_url: String,
    pub is_inline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererStyleSheetHeader {
    pub style_sheet_id: String,
    pub title: String,
    pub disabled: bool,
    pub source_url: String,
    pub is_inline: bool,
    pub length: usize,
    pub end_line: u32,
    pub end_column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererStyleSheetInventoryUpdate {
    pub added: Vec<RendererStyleSheetHeader>,
    pub removed: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RendererDomAttributeMutation {
    Set { name: String, value: String },
    Remove { name: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RendererDomAttributeMutationOutcome {
    Applied {
        name: String,
        old_value: Option<String>,
        new_value: Option<String>,
    },
    NodeNotFound,
    NodeNotElement,
    InvalidName {
        name: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RendererDomEdit {
    MoveTo {
        node_id: u32,
        target_node_id: u32,
        insert_before_node_id: Option<u32>,
    },
    SetAttributesAsText {
        node_id: u32,
        text: String,
        name: Option<String>,
    },
    SetNodeName {
        node_id: u32,
        name: String,
    },
    SetNodeValue {
        node_id: u32,
        value: String,
    },
    SetOuterHtml {
        node_id: u32,
        outer_html: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RendererDomEditOutcome {
    Applied {
        result_frontend_node_id: Option<u32>,
    },
    NodeNotFound,
    NodeNotElement,
    NodeValueUnsupported,
    MoveIntoSelfOrDescendant,
    AnchorNotChildOfTarget,
    DetachedNode,
    InvalidName {
        name: String,
    },
    CouldNotParseAttributes,
    MutationFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererAutofillCreditCard {
    pub number: String,
    pub name: String,
    pub expiry_month: String,
    pub expiry_year: String,
    pub cvc: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererAutofillAddressField {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererAutofillTriggerRequest {
    pub frame_id: Option<String>,
    pub field_id: u32,
    pub card: Option<RendererAutofillCreditCard>,
    pub address: Option<Vec<RendererAutofillAddressField>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RendererAutofillTriggerOutcome {
    Applied { filled_field_count: usize },
    FieldNotFound,
    FrameNotFound,
    CardAndAddressProvided,
    MissingCardOrAddress,
    AddressNotSupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererDomFocusOutcome {
    Focused,
    NodeNotFound,
    NodeNotElement,
    ElementNotFocusable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RendererDomDebuggerEventListener {
    pub event_type: String,
    pub use_capture: bool,
    pub passive: bool,
    pub once: bool,
    pub script_id: String,
    pub line_number: i32,
    pub column_number: i32,
    pub handler: Option<Value>,
    pub original_handler: Option<Value>,
    pub backend_node_id: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RendererDomDebuggerEventListenersResolution {
    Found(Vec<RendererDomDebuggerEventListener>),
    InvalidRemoteObjectId(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererDomDebuggerDomBreakpointResolution {
    Configured,
    NodeNotFound,
    UnknownType(String),
}

pub(crate) enum RendererInspectorPageCommand {
    DispatchRuntimeProtocolMessage {
        raw_json: String,
    },
    DispatchRuntimeProtocolMessageWithDeferredResponse {
        raw_json: String,
        deferred_response: RendererRuntimeInspectorResponseSender,
    },
    DispatchRuntimeProtocolMessageWithContextResolution {
        action: String,
        raw_json: String,
    },
    DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse {
        action: String,
        raw_json: String,
        deferred_response: RendererRuntimeInspectorResponseSender,
    },
    RuntimeEnableEvents,
    ApplyRuntimeProtocolState {
        session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        isolated_worlds: Vec<crate::protocol_types::RuntimeIsolatedWorldDefinition>,
        stored_runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
        session_runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
    },
    DetachRuntimeInspectorSession {
        pause_guard: RendererRuntimeInspectorSessionDetachGuard,
    },
    AddRuntimeBinding {
        name: String,
        execution_context_name: Option<String>,
        execution_context_id: Option<i64>,
    },
    DomDebuggerGetEventListeners {
        object_id: String,
        depth: i32,
        pierce: bool,
    },
    ComputedStylePropertiesForObjectId {
        object_id: String,
    },
    ScrollObjectNodeIntoViewIfNeeded {
        object_id: String,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    },
    ClientRectForObjectId {
        object_id: String,
    },
    DocumentGeometryForObjectId {
        object_id: String,
    },
    NodeHasGeometryForObjectId {
        object_id: String,
    },
    FocusDocumentNodeForObjectId {
        object_id: String,
    },
    SetFileInputFilesForObjectId {
        object_id: String,
        files: Vec<crate::dom::native::SelectedFile>,
        append: bool,
    },
    DocumentNodeSnapshotForObjectId {
        include_whitespace: bool,
        object_id: String,
        depth: i32,
        pierce: bool,
    },
    AccessibilityTreePayloadsForObjectId {
        object_id: String,
    },
    AccessibilityNodeAndAncestorPayloadsForObjectId {
        object_id: String,
    },
    AccessibilityPartialTreePayloadsForObjectId {
        object_id: String,
        fetch_relatives: bool,
    },
    OuterHtmlForObjectId {
        object_id: String,
        include_shadow_dom: bool,
    },
    ResolveRuntimeObjectForBackendNodeId {
        backend_node_id: u32,
        execution_context_id: Option<i64>,
        object_group: Option<String>,
    },
    ResolveBlobObject {
        object_id: String,
    },
}

#[non_exhaustive]
pub enum RendererPageCommand {
    Inspector(RendererInspectorCommandEnvelope),
    EvaluateExpression {
        expression: String,
        await_promise: bool,
    },
    EvaluateExpressionByValue {
        expression: String,
    },
    EvaluateExpressionAndFollowPendingNavigation {
        expression: String,
        await_promise: bool,
    },
    EvaluateExpressionInExecutionContext {
        execution_context_id: i64,
        expression: String,
        await_promise: bool,
    },
    EvaluateExpressionInExecutionContextAndFollowPendingNavigation {
        execution_context_id: i64,
        expression: String,
        await_promise: bool,
    },
    WaitForSelector {
        selector: String,
        timeout_ms: u64,
        loader: ResourceRequestClient,
    },
    WaitForScriptTruthy {
        expression: String,
        timeout_ms: u64,
        loader: ResourceRequestClient,
    },
    WaitForSubresourceResponse {
        criteria: SubresourceResponseWaitCriteria,
        timeout_ms: u64,
        loader: ResourceRequestClient,
    },
    CompleteChildFrameLifecycleWorkBestEffort {
        timeout_ms: u64,
        loader: ResourceRequestClient,
    },
    SetDocumentContent {
        frame_id: String,
        html: String,
    },
    NavigateChildFrame {
        frame_id: String,
        url: String,
    },
    NavigateTopLevelSameDocument {
        url: String,
    },
    RefreshFullPageState,
    PageDiagnosticsSnapshot,
    HasPendingLocationNavigation,
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
    InsertTextIntoActiveControl(String),
    DispatchKeyEvent {
        event_name: String,
        key: String,
        code: String,
        text: String,
        modifiers: u8,
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
    QueueTopLevelHistoryTraversalByDelta(i64),
    RunPageSurfaceOverrideScript {
        source: String,
    },
    AddDocumentStartScriptRuntimeActivity {
        inspector_session_id: Option<String>,
        script: DocumentStartScript,
        run_immediately: bool,
    },
    RemoveDocumentStartScriptByRegistryKey(String),
    #[cfg(test)]
    SetStoredDocumentStartScripts(Vec<DocumentStartScript>),
    SetRuntimeBindingState {
        inspector_session_id: Option<String>,
        stored_runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
        session_runtime_bindings: Vec<crate::protocol_types::RuntimeBindingRegistration>,
    },
    DefaultExecutionContextId,
    DefaultOrInitialExecutionContextId,
    HasIsolatedWorldNamed {
        name: String,
        frame_id: Option<String>,
    },
    HasIsolatedExecutionContextId(i64),
    EnsureIsolatedWorldsAttachedToInspector,
    InspectorExecutionContextIdForIsolatedContext(i64),
    IsolatedExecutionContextIdForInspectorContext(i64),
    RuntimeRealmInventory,
    LiveChildDefaultRuntimeRealmInventory,
    ChildFrameIdForDefaultExecutionContextId(i64),
    ChildDefaultExecutionContextIdForFrameId(String),
    RuntimeConsoleMessagesWithContext,
    RuntimeHeapUsage,
    PerformanceMetricSnapshot,
    RuntimeCollectGarbage,
    #[cfg(test)]
    TakeDocumentLifecycleEvents,
    StopDocumentLifecycle,
    ComputedStylePropertiesForBackendNodeId {
        backend_node_id: u32,
    },
    SetInlineStyleSheetTextForStyleSheetId {
        inspector_session_id: Option<String>,
        style_sheet_id: String,
        text: String,
    },
    ScrollBackendNodeIntoViewIfNeeded {
        backend_node_id: u32,
        rect: Option<moli_page_types::DomScrollIntoViewRect>,
    },
    ClientRectForBackendNodeId {
        backend_node_id: u32,
    },
    DocumentGeometryForBackendNodeId {
        backend_node_id: u32,
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
    RemoveDocumentBackendNodeId {
        backend_node_id: u32,
    },
    MutateDocumentBackendNodeAttribute {
        backend_node_id: u32,
        mutation: RendererDomAttributeMutation,
    },
    EditDocumentNode {
        inspector_session_id: Option<String>,
        edit: RendererDomEdit,
    },
    FocusDocumentBackendNode {
        backend_node_id: u32,
    },
    TriggerAutofill(RendererAutofillTriggerRequest),
    ResetNavigationHistory,
    SetFileInputFilesForBackendNodeId {
        backend_node_id: u32,
        files: Vec<crate::dom::native::SelectedFile>,
        append: bool,
    },
    DocumentNodeSnapshotForBackendNodeId {
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    },
    DocumentNodeSnapshotForBackendNodeIdInInspectorSession {
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        backend_node_id: u32,
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
    DocumentChildNodeSnapshotEventsForBackendNodeId {
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        backend_node_id: u32,
        depth: i32,
        pierce: bool,
    },
    DocumentQuerySelectorForDocument {
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        selector: String,
        multiple: bool,
    },
    DocumentQuerySelectorForChildFrameBackendNodeId {
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        frame_id: String,
        root_backend_node_id: u32,
        selector: String,
        multiple: bool,
    },
    DocumentQuerySelectorForBackendNodeId {
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        root_backend_node_id: u32,
        selector: String,
        multiple: bool,
    },
    DocumentQuerySelectorWithChildNodeSnapshotEventsForBackendNodeId {
        inspector_session_id: Option<String>,
        include_whitespace: bool,
        root_backend_node_id: u32,
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
        inspector_session_id: Option<String>,
        frontend_node_id: u32,
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
    DocumentNodeAttributesForBackendNodeId {
        backend_node_id: u32,
    },
    DocumentNodeTextForBackendNodeId {
        backend_node_id: u32,
    },
    DocumentNodePropertyForBackendNodeId {
        backend_node_id: u32,
        name: String,
    },
    AccessibilityTreePayloadsForDocument {
        max_depth: Option<i32>,
    },
    AccessibilityNodePayloadForDocument,
    AccessibilityTreePayloadsForBackendNodeId {
        backend_node_id: u32,
        max_depth: Option<i32>,
    },
    AccessibilityNodePayloadForBackendNodeId {
        backend_node_id: u32,
    },
    AccessibilityNodeAndAncestorPayloadsForBackendNodeId {
        backend_node_id: u32,
    },
    AccessibilityChildNodePayloadsForBackendNodeId {
        backend_node_id: u32,
    },
    AccessibilityPartialTreePayloadsForBackendNodeId {
        backend_node_id: u32,
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
    OuterHtmlForDocument {
        include_shadow_dom: bool,
    },
    OuterHtmlForBackendNodeId {
        backend_node_id: u32,
        include_shadow_dom: bool,
    },
    RenderPageDump {
        options: RendererPageDumpOptions,
    },
    SerializeHtml,
    LayoutMetrics,
    CaptureScreenshot(RendererCaptureScreenshotRequest),
    CaptureScreencastFrame(RendererCaptureScreencastFrameRequest),
    BlobBytesForUuid {
        uuid: String,
    },
    DocumentFrontendNodeIdsForBackendNodeIds {
        inspector_session_id: Option<String>,
        backend_node_ids: Vec<u32>,
    },
    DocumentStorageKeySnapshot,
    SearchTextByLines {
        text: String,
        query: String,
        case_sensitive: bool,
        is_regex: bool,
    },
    SearchChildFrameResourceByLines {
        frame_id: String,
        url: String,
        query: String,
        case_sensitive: bool,
        is_regex: bool,
    },
    ChildFrameTreeSnapshot,
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
        body: Option<Option<String>>,
        headers: Option<Vec<(String, String)>>,
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
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    },
    ContinuePendingSubresourceResponse {
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    },
    FailPendingSubresourceResponse {
        internal_id: u64,
        error_text: String,
    },
    FulfillPendingSubresourceResponse {
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
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
    PendingSubresourceRequestCount,
    SetFetchSubresourceInterception {
        enabled: bool,
        resource_type: Option<crate::SubresourceResourceType>,
    },
    SetJavaScriptDialogHandlerEnabled(bool),
    ReplaceBrowserResourceRuntime(crate::network::BrowserResourceRuntime),
    RetireDocumentResourceAuthorities,
    ApplyDocumentCookieFacadeOverrides(moli_cookie_jar::BrowserCookieFacadeOverrides),
    ClearDocumentCookieFacadeOverrides,
    DocumentCookieTelemetrySnapshot,
    DocumentCookieOwnerSnapshot,
    PrepareNetworkResourceLoad {
        frame_id: String,
        url: Url,
        disable_cache: bool,
        include_credentials: bool,
    },
    PrepareAppManifestLoad,
    PublishAppManifestLoad(Box<crate::RendererAppManifestLoadPublication>),
    SetExtraHttpHeaders(Vec<(String, String)>),
    SetNetworkRequestPolicy {
        extra_http_headers: Vec<(String, String)>,
        bypass_service_worker: bool,
        cache_disabled: bool,
    },
    SetPermissionOverrides(Vec<crate::protocol_types::PermissionOverrideRegistration>),
    SetIdleOverride(Option<crate::protocol_types::EmulatedIdleOverride>),
    SetLocaleOverride(Option<String>),
    SetTimezoneOverride(Option<String>),
    SetScriptExecutionDisabled(bool),
    SetBypassContentSecurityPolicy(bool),
    SetCpuThrottlingRate(f64),
    SetEmulatedMedia(crate::protocol_types::EmulatedMediaOverrides),
    SetViewportSurface(Option<crate::protocol_types::ViewportSurface>),
    SetNetworkOffline(bool),
    SetBypassServiceWorker(bool),
    SetBlockedUrlPatterns(Vec<String>),
    MsToNextTimeout,
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    PanicForTesting,
}

pub enum RendererPageCookieFacadeSnapshotReply {
    Telemetry(crate::DocumentCookieFacadeTelemetrySnapshot),
    Owner(Box<crate::DocumentCookieOwnerSnapshot>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererSetDocumentContentResult {
    Updated,
    FrameNotFound,
    DocumentNotFound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RendererScrollIntoViewResult {
    ScrolledOrAlreadyVisible,
    NodeNotFound,
    NodeDetached,
    NodeDoesNotHaveLayoutObject,
}

pub enum RendererRuntimeRemoteObjectResolution {
    Found(RendererRuntimeRemoteObject),
    MissingContext,
    MissingNode,
}

impl RendererPageCommand {
    fn inspector_command(
        inspector_session_id: Option<String>,
        command: RendererInspectorPageCommand,
    ) -> Self {
        Self::Inspector(RendererInspectorCommandEnvelope::new(
            inspector_session_id,
            command,
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
    ) -> Self {
        Self::inspector_command(
            inspector_session_id,
            RendererInspectorPageCommand::DocumentNodeSnapshotForObjectId {
                include_whitespace,
                object_id,
                depth,
                pierce,
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
            Self::Inspector(envelope) => Some(envelope.ticket()),
            _ => None,
        }
    }

    pub fn bind_inspector_attachment(&mut self, attachment: RendererAgentAttachmentId) {
        if let Self::Inspector(envelope) = self {
            envelope.bind_attachment(attachment);
        }
    }

    pub(crate) fn interruptible_by_javascript_dialog(&self) -> bool {
        #[cfg(test)]
        if matches!(self, Self::TakeDocumentLifecycleEvents) {
            return true;
        }
        matches!(
            self,
            Self::PageDiagnosticsSnapshot
                | Self::HasPendingLocationNavigation
                | Self::LiveChildDefaultRuntimeRealmInventory
                | Self::RuntimeConsoleMessagesWithContext
                | Self::ChildFrameTreeSnapshot
                | Self::PendingSubresourceRequestCount
        )
    }

    pub(crate) fn cdp_nav_timing_label(&self) -> Option<&'static str> {
        match self {
            Self::Inspector(envelope) => envelope.cdp_nav_timing_label(),
            Self::DocumentStorageKeySnapshot => Some("DocumentStorageKeySnapshot"),
            Self::CreateIsolatedWorldRuntimeActivity { .. } => {
                Some("CreateIsolatedWorldRuntimeActivity")
            }
            Self::AddDocumentStartScriptRuntimeActivity { .. } => {
                Some("AddDocumentStartScriptRuntimeActivity")
            }
            Self::RemoveDocumentStartScriptByRegistryKey(_) => {
                Some("RemoveDocumentStartScriptByRegistryKey")
            }
            Self::SetRuntimeBindingState { .. } => Some("SetRuntimeBindingState"),
            Self::ChildFrameTreeSnapshot => Some("ChildFrameTreeSnapshot"),
            Self::ChildFrameOwnerNodeReference { .. } => Some("ChildFrameOwnerNodeReference"),
            Self::ChildFrameDocumentRootNodeReference { .. } => {
                Some("ChildFrameDocumentRootNodeReference")
            }
            Self::DocumentNodeSnapshotForBackendNodeId { .. } => {
                Some("DocumentNodeSnapshotForBackendNodeId")
            }
            Self::DocumentNodeSnapshotForBackendNodeIdInInspectorSession { .. } => {
                Some("DocumentNodeSnapshotForBackendNodeIdInInspectorSession")
            }
            Self::DocumentNodeSnapshotForDocument { .. } => Some("DocumentNodeSnapshotForDocument"),
            Self::DiscardDomAgentFrontendBindings { .. } => Some("DiscardDomAgentFrontendBindings"),
            Self::DomSnapshotCapture { .. } => Some("DomSnapshotCapture"),
            Self::DocumentChildNodeSnapshotEventsForBackendNodeId { .. } => {
                Some("DocumentChildNodeSnapshotEventsForBackendNodeId")
            }
            Self::DocumentQuerySelectorForDocument { .. } => {
                Some("DocumentQuerySelectorForDocument")
            }
            Self::DocumentQuerySelectorForChildFrameBackendNodeId { .. } => {
                Some("DocumentQuerySelectorForChildFrameBackendNodeId")
            }
            Self::DocumentQuerySelectorForBackendNodeId { .. } => {
                Some("DocumentQuerySelectorForBackendNodeId")
            }
            Self::DocumentQuerySelectorWithChildNodeSnapshotEventsForBackendNodeId { .. } => {
                Some("DocumentQuerySelectorWithChildNodeSnapshotEventsForBackendNodeId")
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
            Self::DocumentNodeAttributesForBackendNodeId { .. } => {
                Some("DocumentNodeAttributesForBackendNodeId")
            }
            Self::DocumentNodeTextForBackendNodeId { .. } => {
                Some("DocumentNodeTextForBackendNodeId")
            }
            Self::DocumentNodePropertyForBackendNodeId { .. } => {
                Some("DocumentNodePropertyForBackendNodeId")
            }
            Self::AccessibilityTreePayloadsForDocument { .. } => {
                Some("AccessibilityTreePayloadsForDocument")
            }
            Self::AccessibilityNodePayloadForDocument => {
                Some("AccessibilityNodePayloadForDocument")
            }
            Self::AccessibilityTreePayloadsForBackendNodeId { .. } => {
                Some("AccessibilityTreePayloadsForBackendNodeId")
            }
            Self::AccessibilityNodePayloadForBackendNodeId { .. } => {
                Some("AccessibilityNodePayloadForBackendNodeId")
            }
            Self::AccessibilityNodeAndAncestorPayloadsForBackendNodeId { .. } => {
                Some("AccessibilityNodeAndAncestorPayloadsForBackendNodeId")
            }
            Self::AccessibilityChildNodePayloadsForBackendNodeId { .. } => {
                Some("AccessibilityChildNodePayloadsForBackendNodeId")
            }
            Self::AccessibilityPartialTreePayloadsForBackendNodeId { .. } => {
                Some("AccessibilityPartialTreePayloadsForBackendNodeId")
            }
            Self::AccessibilityTreePayloadsForChildFrame { .. } => {
                Some("AccessibilityTreePayloadsForChildFrame")
            }
            Self::AccessibilityNodePayloadForChildFrame { .. } => {
                Some("AccessibilityNodePayloadForChildFrame")
            }
            Self::OuterHtmlForDocument { .. } => Some("OuterHtmlForDocument"),
            Self::OuterHtmlForBackendNodeId { .. } => Some("OuterHtmlForBackendNodeId"),
            Self::RenderPageDump { .. } => Some("RenderPageDump"),
            Self::SerializeHtml => Some("SerializeHtml"),
            Self::LayoutMetrics => Some("LayoutMetrics"),
            Self::CaptureScreenshot(_) => Some("CaptureScreenshot"),
            Self::CaptureScreencastFrame(_) => Some("CaptureScreencastFrame"),
            Self::ScrollBackendNodeIntoViewIfNeeded { .. } => {
                Some("ScrollBackendNodeIntoViewIfNeeded")
            }
            Self::ClientRectForBackendNodeId { .. } => Some("ClientRectForBackendNodeId"),
            Self::DocumentGeometryForBackendNodeId { .. } => {
                Some("DocumentGeometryForBackendNodeId")
            }
            Self::DocumentHitTest { .. } => Some("DocumentHitTest"),
            Self::NodeHasGeometryForBackendNodeId { .. } => Some("NodeHasGeometryForBackendNodeId"),
            Self::RemoveDocumentBackendNodeId { .. } => Some("RemoveDocumentBackendNodeId"),
            Self::EditDocumentNode { .. } => Some("EditDocumentNode"),
            Self::TriggerAutofill(_) => Some("TriggerAutofill"),
            Self::ComputedStylePropertiesForBackendNodeId { .. } => {
                Some("ComputedStylePropertiesForBackendNodeId")
            }
            Self::SetFileInputFilesForBackendNodeId { .. } => {
                Some("SetFileInputFilesForBackendNodeId")
            }
            Self::BlobBytesForUuid { .. } => Some("BlobBytesForUuid"),
            Self::DocumentFrontendNodeIdsForBackendNodeIds { .. } => {
                Some("DocumentFrontendNodeIdsForBackendNodeIds")
            }
            Self::EnsureIsolatedWorldsAttachedToInspector => {
                Some("EnsureIsolatedWorldsAttachedToInspector")
            }
            Self::RuntimeRealmInventory => Some("RuntimeRealmInventory"),
            Self::LiveChildDefaultRuntimeRealmInventory => {
                Some("LiveChildDefaultRuntimeRealmInventory")
            }
            _ => None,
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

#[derive(Debug, Clone, PartialEq)]
pub struct RendererRuntimeRemoteObject {
    protocol_value: Value,
}

impl RendererRuntimeRemoteObject {
    pub(crate) fn from_protocol_value(protocol_value: Value) -> Option<Self> {
        let is_object = protocol_value.get("type").and_then(Value::as_str) == Some("object");
        let is_node = protocol_value.get("subtype").and_then(Value::as_str) == Some("node");
        (is_object || is_node).then_some(Self { protocol_value })
    }

    pub fn as_protocol_value(&self) -> &Value {
        &self.protocol_value
    }

    pub fn into_protocol_value(self) -> Value {
        self.protocol_value
    }
}

pub enum RendererDocumentNodeClientRect {
    Found(ClientRect),
    FoundNonElement(ClientRect),
    NotElement,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RendererGeometryQuad {
    pub points: [f64; 8],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RendererDocumentBoxModel {
    pub content: RendererGeometryQuad,
    pub padding: RendererGeometryQuad,
    pub border: RendererGeometryQuad,
    pub margin: RendererGeometryQuad,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RendererDocumentNodeGeometry {
    FoundElement {
        box_model: Box<RendererDocumentBoxModel>,
        content_quads: Vec<RendererGeometryQuad>,
    },
    FoundNonElement {
        content_quads: Vec<RendererGeometryQuad>,
    },
    NoLayoutObject,
    NotElement,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererDocumentHitTestResult {
    pub node: RendererDocumentNodeReference,
    pub frame_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RendererLayoutMetrics {
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub page_x: f64,
    pub page_y: f64,
    pub content_width: f64,
    pub content_height: f64,
    pub device_pixel_ratio: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RendererDocumentChildNodeSnapshots {
    pub top_snapshot_node_id: crate::dom::NodeId,
    pub snapshots: Vec<DocumentNodeSnapshot>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RendererDocumentChildNodeSnapshotEvent {
    pub parent_frontend_node_id: u32,
    pub snapshots: Vec<DocumentNodeSnapshot>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RendererDocumentChildNodeSnapshotEvents {
    pub top_snapshot_node_id: crate::dom::NodeId,
    pub events: Vec<RendererDocumentChildNodeSnapshotEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererPageDumpFormat {
    Html,
    Markdown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RendererPageDumpStripOptions {
    pub js: bool,
    pub ui: bool,
    pub css: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RendererPageDumpOptions {
    pub format: RendererPageDumpFormat,
    pub strip: RendererPageDumpStripOptions,
    pub with_base: bool,
    pub with_frames: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RendererDomSnapshotCaptureOptions {
    pub computed_styles: Vec<String>,
    pub include_paint_order: bool,
    pub include_dom_rects: bool,
    pub include_blended_background_colors: bool,
    pub include_text_color_opacities: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RendererDomSnapshotCapturePayload {
    protocol_payload: Value,
}

impl RendererDomSnapshotCapturePayload {
    pub(crate) fn from_protocol_payload(protocol_payload: Value) -> Self {
        Self { protocol_payload }
    }

    pub fn into_protocol_payload(self) -> Value {
        self.protocol_payload
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RendererRuntimeEvaluationResult {
    protocol_payload: Value,
}

impl RendererRuntimeEvaluationResult {
    pub(crate) fn from_protocol_payload(protocol_payload: Value) -> Self {
        Self { protocol_payload }
    }

    pub fn as_protocol_payload(&self) -> &Value {
        &self.protocol_payload
    }

    pub fn into_protocol_payload(self) -> Value {
        self.protocol_payload
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererCapturedScreenshot {
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub bytes: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererCaptureScreenshotReply {
    Captured(RendererCapturedScreenshot),
    LayoutDisabled,
    NoDocument,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererCapturedScreencastFrame {
    pub image: RendererCapturedScreenshot,
    pub visual_state: RendererVisualStateToken,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererCaptureScreencastFrameReply {
    Captured(RendererCapturedScreencastFrame),
    Unchanged,
    LayoutDisabled,
    NoDocument,
}

pub enum RendererPageReply {
    RuntimeEvaluationResult(RendererRuntimeEvaluationResult),
    RuntimeInspectorProtocolMessages(RendererRuntimeCommandOutput),
    RuntimeConsoleMessageSnapshots(Vec<RuntimeConsoleMessageSnapshot>),
    RuntimeHeapUsage(Box<RendererRuntimeHeapUsage>),
    PerformanceMetricSnapshot(Box<RendererPerformanceMetricSnapshot>),
    RuntimeRealmInventory(Vec<RendererRuntimeRealmInfo>),
    ExecutionContextId(i64),
    ExecutionContextIds(Vec<i64>),
    OptionalExecutionContextId(Option<i64>),
    OptionalDocumentNodeObjectSnapshot(Box<Option<DocumentNodeObjectSnapshot>>),
    OptionalDomSnapshotCapturePayload(Option<RendererDomSnapshotCapturePayload>),
    OptionalDocumentChildNodeSnapshots(Option<RendererDocumentChildNodeSnapshots>),
    OptionalDocumentChildNodeSnapshotEvents(Option<RendererDocumentChildNodeSnapshotEvents>),
    DocumentQuerySelectorResolution(RendererDocumentQuerySelectorResolution),
    DocumentQuerySelectorNode(RendererDocumentQuerySelectorNode),
    DocumentQuerySelectorWithChildNodeSnapshotEvents(
        RendererDocumentQuerySelectorWithChildNodeSnapshotEvents,
    ),
    DocumentPerformSearch(RendererDomSearchRegistration),
    DocumentSearchResults(RendererDomSearchResultsResolution),
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
    OptionalDocumentNodeReference(Option<RendererDocumentNodeReference>),
    OptionalClientRect(Option<ClientRect>),
    OptionalDocumentNodeClientRect(Option<RendererDocumentNodeClientRect>),
    OptionalDocumentNodeGeometry(Option<RendererDocumentNodeGeometry>),
    OptionalDocumentHitTest(Option<RendererDocumentHitTestResult>),
    RuntimeRemoteObjectResolution(RendererRuntimeRemoteObjectResolution),
    DomDebuggerEventListeners(RendererDomDebuggerEventListenersResolution),
    DomDebuggerDomBreakpoint(RendererDomDebuggerDomBreakpointResolution),
    BlobUuid(String),
    OptionalBlobBytes(Option<Arc<[u8]>>),
    ComputedStyleProperties(Option<Vec<(String, String)>>),
    DomAttributeMutationOutcome(RendererDomAttributeMutationOutcome),
    DomEditOutcome(RendererDomEditOutcome),
    DomFocusOutcome(RendererDomFocusOutcome),
    AutofillTriggerOutcome(RendererAutofillTriggerOutcome),
    DocumentStorageKey(String),
    ResourceTextSearchOutcome(RendererResourceTextSearchOutcome),
    NetworkResourceLoadPreparation(crate::network::RendererNetworkResourceLoadPreparation),
    AppManifestLoadPreparation(crate::RendererAppManifestLoadPreparation),
    ChildFrameTreeSnapshots(Vec<ChildFrameTreeSnapshot>),
    DocumentStartScriptResult(Option<(i64, bool)>),
    #[cfg(test)]
    DocumentLifecycleEvents(Vec<RendererDocumentLifecycleEvent>),
    PendingSubresourceContinueOutcome(crate::PendingSubresourceContinueOutcome),
    PageDiagnosticsSnapshot(RendererPageDiagnosticsSnapshot),
    InputDispatchOutcome(RendererInputDispatchOutcome),
    SetDocumentContentResult(RendererSetDocumentContentResult),
    ScrollIntoViewResult(RendererScrollIntoViewResult),
    Bool(bool),
    OptionalBool(Option<bool>),
    OptionalString(Option<String>),
    OptionalU64(Option<u64>),
    Usize(usize),
    CookieFacadeSnapshot(Box<RendererPageCookieFacadeSnapshotReply>),
    LayoutMetrics(RendererLayoutMetrics),
    CaptureScreenshot(RendererCaptureScreenshotReply),
    CaptureScreencastFrame(RendererCaptureScreencastFrameReply),
    Unit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererTextSearchMatch {
    pub line_number: usize,
    pub line_content: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererResourceTextSearchOutcome {
    Matches(Vec<RendererTextSearchMatch>),
    FrameNotFound,
    ResourceNotFound,
    ContentUnavailable,
}

#[derive(Debug, Default)]
struct RendererPageTableState {
    terminal: bool,
    pages: HashMap<PageId, RendererPageSlotHandle>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct RendererPageTable {
    inner: Arc<Mutex<RendererPageTableState>>,
}

impl RendererPageTable {
    pub(super) fn contains_page(&self, page_id: PageId) -> bool {
        self.inner.lock().pages.contains_key(&page_id)
    }

    pub(super) fn insert_new_slot(
        &self,
        page_id: PageId,
        slot: RendererPageSlotHandle,
    ) -> Result<RendererPageSlotHandle> {
        let mut state = self.inner.lock();
        if state.terminal {
            drop(state);
            slot.cancel_page_context(RendererPageContextCancelReason::ContextDropped);
            bail!("renderer browser context owner was dropped before Page attachment");
        }
        let previous = state.pages.insert(page_id, slot.clone());
        assert!(
            previous.is_none(),
            "renderer owner page-slot attach must not replace page {}",
            page_id.as_u64()
        );
        Ok(slot)
    }

    pub(crate) fn refresh(
        &self,
        page_id: PageId,
        vm_creation_id: u64,
        view_generation: u64,
        requested_url: Url,
        final_url: Url,
        document_title: String,
        status: u16,
    ) -> Result<()> {
        let Some(slot) = self.slot(page_id) else {
            bail!(
                "renderer owner has never tracked page {} for refresh",
                page_id.as_u64()
            );
        };
        slot.refresh(RendererPageView {
            page_id,
            vm_creation_id,
            view_generation,
            page_state: Arc::new(RendererPageState {
                requested_url,
                navigation_initiator_url: None,
                navigation_redirected: false,
                navigation_redirect_count: 0,
                navigation_redirect_chain: Vec::new(),
                final_url,
                document_title,
                status,
                headers: Vec::new(),
                script_execution: Arc::new(ScriptExecutionReport::default()),
                idle_override: None,
                service_worker_client_id: 0,
                dedicated_worker_running_worker_isolate_count: 0,
                performance_metric_snapshot: RendererPerformanceMetricSnapshot::default(),
            }),
        })
    }

    pub(crate) fn remove(&self, page_id: PageId) {
        let Some(slot) = self.slot(page_id) else {
            return;
        };
        slot.remove();
    }

    /// Permanently closes Page admission before cancelling the current slot
    /// snapshot. The terminal bit and slot map share one mutex, so an attach
    /// either commits before this boundary and is present in the snapshot, or
    /// observes terminal and rejects itself.
    pub(crate) fn terminate_and_cancel_all_contexts(
        &self,
        reason: RendererPageContextCancelReason,
    ) {
        let slots = {
            let mut state = self.inner.lock();
            state.terminal = true;
            state.pages.values().cloned().collect::<Vec<_>>()
        };
        for slot in slots {
            slot.cancel_page_context(reason);
        }
    }

    pub(super) fn is_terminal(&self) -> bool {
        self.inner.lock().terminal
    }

    fn slot(&self, page_id: PageId) -> Option<RendererPageSlotHandle> {
        let state = self.inner.lock();
        state.pages.get(&page_id).cloned()
    }

    pub(super) fn owns_slot(&self, slot: &RendererPageSlotHandle) -> bool {
        let page_id = slot.page_id();
        self.slot(page_id)
            .map(|tracked| tracked.same_slot(slot))
            .unwrap_or(false)
    }

    fn entry(&self, page_id: PageId) -> Option<RendererPageEntry> {
        self.slot(page_id).map(|slot| slot.entry())
    }

    pub(crate) fn record(&self, page_id: PageId) -> Option<RendererPageRecord> {
        self.entry(page_id).and_then(|entry| entry.active_record())
    }

    pub(crate) fn len(&self) -> usize {
        let state = self.inner.lock();
        state
            .pages
            .values()
            .filter(|slot| slot.entry().is_active())
            .count()
    }

    pub(crate) fn command_epoch(&self, page_id: PageId) -> Option<u64> {
        self.entry(page_id)
            .filter(|entry| entry.is_active())
            .map(|entry| entry.command_epoch())
    }

    pub(crate) fn in_flight_command_epoch(&self, page_id: PageId) -> Option<u64> {
        self.entry(page_id)
            .filter(|entry| entry.is_active())
            .and_then(|entry| entry.in_flight_command_epoch)
    }
}
