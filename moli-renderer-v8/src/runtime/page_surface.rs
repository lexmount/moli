use super::*;
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
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::sync::oneshot;

mod javascript_dialog;
mod popup_activation;
mod window_document_source;
pub use javascript_dialog::{
    RendererJavaScriptDialogId, RendererJavaScriptDialogSource, RendererPendingJavaScriptDialog,
};
pub use popup_activation::{RendererPendingPopupActivation, RendererPopupActivationSource};
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
/// producer order even though response and renderer output use separate
/// transports.
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
        })
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
        (self.completion, self.renderer_output_predecessor)
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
    lease_id: u64,
    channel: RendererRuntimeInspectorResponseChannel,
}

impl RendererRuntimeInspectorResponsePublication {
    pub(crate) fn commit(
        self,
        renderer_output_predecessor: Option<RendererOutputFence>,
    ) -> Result<(), RendererRuntimeInspectorAsyncCompletion> {
        self.channel.send(
            self.lease_id,
            RendererRuntimeInspectorAsyncCompletion::from_renderer_response_edge(
                self.call_id,
                self.output,
                renderer_output_predecessor,
            ),
        )
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
        debug_assert!(
            state.response.is_none(),
            "one runtime command output scope cannot own multiple responses"
        );
        state.response = Some((sender, message));
    }

    pub(crate) fn finish(self) -> RendererRuntimeCommandOutput {
        self.finish_with_response_override(None)
    }

    pub(crate) fn finish_with_error(self, message: String) -> RendererRuntimeCommandOutput {
        self.finish_with_response_override(Some(message))
    }

    fn finish_with_response_override(
        self,
        error_message: Option<String>,
    ) -> RendererRuntimeCommandOutput {
        let mut state = self.0.borrow_mut();
        let mut output = std::mem::take(&mut state.output);
        let response = state.response.take();
        drop(state);
        let Some((sender, message)) = response else {
            return output;
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
    tx: Option<oneshot::Sender<RendererRuntimeInspectorAsyncCompletion>>,
}

#[derive(Clone)]
pub struct RendererRuntimeInspectorResponseChannel {
    state: Arc<Mutex<RendererRuntimeInspectorResponseChannelState>>,
}

impl std::fmt::Debug for RendererRuntimeInspectorResponseChannel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.lock();
        formatter
            .debug_struct("RendererRuntimeInspectorResponseChannel")
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
                state: Arc::new(Mutex::new(RendererRuntimeInspectorResponseChannelState {
                    next_lease_id: 1,
                    active_lease_id: None,
                    tx: Some(tx),
                })),
            },
            rx,
        )
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
            state.tx.as_ref()?;
            let lease_id = state.next_lease_id;
            state.next_lease_id = lease_id
                .checked_add(1)
                .expect("renderer response lease id exhausted");
            state.active_lease_id = Some(lease_id);
            lease_id
        };
        Some(RendererRuntimeInspectorResponseSender {
            call_id,
            attachment_id,
            lease_id,
            channel: self.clone(),
            publication_boundary: RendererRuntimeInspectorResponsePublicationBoundary::Immediate,
        })
    }

    pub fn cancel(&self) {
        let mut state = self.state.lock();
        state.active_lease_id = None;
        state.tx.take();
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
            state.active_lease_id = None;
            state.tx.take()
        };
        let Some(tx) = tx else {
            return Err(completion);
        };
        tx.send(completion)
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
    lease_id: u64,
    channel: RendererRuntimeInspectorResponseChannel,
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
            state: Arc::new(Mutex::new(RendererRuntimeInspectorResponseChannelState {
                next_lease_id: 2,
                active_lease_id: Some(1),
                tx: Some(tx),
            })),
        };
        Self {
            call_id,
            attachment_id: None,
            lease_id: 1,
            channel,
            publication_boundary: RendererRuntimeInspectorResponsePublicationBoundary::Immediate,
        }
    }

    pub fn call_id(&self) -> i32 {
        self.call_id
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
            lease_id,
            channel,
            publication_boundary,
        } = self;
        if let Some(attachment_id) = attachment_id {
            output.bind_renderer_agent_attachment(attachment_id);
        }
        let publication = RendererRuntimeInspectorResponsePublication {
            call_id,
            output,
            lease_id,
            channel,
        };
        match publication_boundary {
            RendererRuntimeInspectorResponsePublicationBoundary::Immediate => {
                publication.commit(None)
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
                        publication.commit(None)
                    }
                }
            }
        }
    }

    fn finalize_output(
        self,
        mut output: RendererRuntimeCommandOutput,
    ) -> RendererRuntimeCommandOutput {
        if let Some(attachment_id) = self.attachment_id {
            output.bind_renderer_agent_attachment(attachment_id);
        }
        output
    }
}

#[cfg(test)]
mod renderer_runtime_inspector_response_channel_tests {
    use super::*;
    use crate::page_task_queue::{RendererOwnerWake, RendererOwnerWakeSender};
    use tokio::sync::oneshot::error::TryRecvError;

    fn output(call_id: i32) -> RendererRuntimeCommandOutput {
        RendererRuntimeCommandOutput::from_inspector_message(
            RendererRuntimeInspectorMessage::protocol(json!({
                "id": call_id,
                "result": {},
            })),
        )
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
    async fn retained_channel_prevents_old_sender_drop_from_canceling_frontend_receiver() {
        let (channel, mut rx) = RendererRuntimeInspectorResponseChannel::new();
        let old_sender = channel.activate_sender(1, None);
        drop(old_sender);

        assert!(matches!(
            rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));

        channel
            .activate_sender(2, None)
            .send_output(output(2))
            .expect("replacement sender should retain completion ownership");
        assert_eq!(rx.await.unwrap().call_id, 2);
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
    async fn completed_response_lease_cannot_be_rotated() {
        let (channel, rx) = RendererRuntimeInspectorResponseChannel::new();
        channel
            .activate_sender(1, None)
            .send_output(output(1))
            .expect("active response lease should complete");

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
            lease_id,
            channel,
            publication_boundary: _,
        } = sender;
        let mut output = output(1);
        if let Some(attachment_id) = attachment_id {
            output.bind_renderer_agent_attachment(attachment_id);
        }

        RendererRuntimeInspectorResponsePublication {
            call_id,
            output,
            lease_id,
            channel,
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

/// Chromium routes Page DevTools work through separate main-thread and IO
/// session ingress paths. Ordering is guaranteed within one session and one
/// route, while an IO command may overtake main-thread work that has not
/// reached its first dispatch yet.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RendererInspectorCommandRoute {
    MainThread,
    Io,
}

/// The command owns its `(Page, session, route)` lane until its first access to
/// the frontend V8 Inspector session. Protocol response completion is not part
/// of this lifetime: V8 may complete a response asynchronously, and holding the
/// lane for that response could prevent a later resume command from running.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererInspectorFirstDispatchLifecycle {
    OrderedUntilFirstDispatch,
}

/// Pause-loop transition owned by an Inspector command. This is derived once
/// when the ingress envelope is created so executors do not need to reparse
/// protocol JSON to decide how resumed/paused notifications are attributed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererInspectorPauseCommandEffect {
    None,
    Resume,
    Step,
}

impl RendererInspectorPauseCommandEffect {
    fn from_message(message: Option<&Value>) -> Self {
        match message
            .and_then(|message| message.get("method"))
            .and_then(Value::as_str)
        {
            Some("Debugger.resume") => Self::Resume,
            Some(
                "Debugger.continueToLocation"
                | "Debugger.restartFrame"
                | "Debugger.stepInto"
                | "Debugger.stepOut"
                | "Debugger.stepOver",
            ) => Self::Step,
            _ => Self::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RendererInspectorMainDispatchBoundary {
    InspectorSession,
    PageOwner,
}

static NEXT_RENDERER_INSPECTOR_INGRESS_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererInspectorIngressTicket {
    attachment: Option<RendererAgentAttachmentId>,
    session: DevToolsSessionKey,
    route: RendererInspectorCommandRoute,
    sequence: u64,
}

impl RendererInspectorIngressTicket {
    pub fn new(
        attachment: Option<RendererAgentAttachmentId>,
        inspector_session_id: Option<String>,
        route: RendererInspectorCommandRoute,
    ) -> Self {
        Self {
            attachment,
            session: DevToolsSessionKey::from_wire_session_id(
                inspector_session_id
                    .as_deref()
                    .filter(|session_id| !session_id.is_empty()),
            ),
            route,
            sequence: NEXT_RENDERER_INSPECTOR_INGRESS_SEQUENCE
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |sequence| {
                    sequence.checked_add(1)
                })
                .expect("renderer Inspector ingress sequence overflow"),
        }
    }

    pub fn attachment(&self) -> Option<RendererAgentAttachmentId> {
        self.attachment
    }

    pub fn session(&self) -> &DevToolsSessionKey {
        &self.session
    }

    pub fn route(&self) -> RendererInspectorCommandRoute {
        self.route
    }

    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    fn bind_attachment(&mut self, attachment: RendererAgentAttachmentId) {
        if let Some(bound) = self.attachment {
            assert_eq!(
                bound, attachment,
                "an Inspector ingress ticket cannot be retargeted to another attachment"
            );
        } else {
            self.attachment = Some(attachment);
        }
    }
}

/// Strongly typed DevToolsSession ingress. The payload deliberately does not
/// carry another session id: every operation that accesses a frontend
/// `V8InspectorSession` must obtain its identity and dispatch policy from this
/// envelope.
pub struct RendererInspectorCommandEnvelope {
    ticket: RendererInspectorIngressTicket,
    first_dispatch: RendererInspectorFirstDispatchLifecycle,
    pause_effect: RendererInspectorPauseCommandEffect,
    main_dispatch_boundary: RendererInspectorMainDispatchBoundary,
    payload: RendererInspectorCommandPayload,
}

/// One command delivered by the renderer's Main DevTools receiver.
///
/// Unlike `RendererInspectorCommandEnvelope`, this envelope is deliberately
/// agent-neutral: protocol commands that ultimately need the renderer Page,
/// DOM, CSS, Accessibility, or V8 agents all enter the same Main receiver.
/// The boxed payload keeps that admission boundary structural without adding
/// a second allowlist of `RendererPageCommand` variants.
#[doc(hidden)]
pub struct RendererDevToolsMainCommandEnvelope {
    ticket: RendererInspectorIngressTicket,
    payload: Box<RendererPageCommand>,
    first_dispatch:
        Option<crate::script_vm::inspector_main::RendererInspectorMainFirstDispatchGuard>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererDevToolsMainNestedDispatch {
    InspectorSession,
    PageAgent,
    OwnerOnly,
}

impl RendererDevToolsMainCommandEnvelope {
    pub(crate) fn from_protocol_command(command: RendererPageCommand) -> Self {
        Self::from_protocol_command_in_session(command, None)
    }

    pub(crate) fn from_protocol_command_in_session(
        command: RendererPageCommand,
        inspector_session_id: Option<String>,
    ) -> Self {
        let ticket = match &command {
            RendererPageCommand::Inspector(envelope) => envelope.ticket().clone(),
            RendererPageCommand::DevToolsMain(_) => {
                panic!("a Main DevTools command cannot be enveloped twice")
            }
            _ => RendererInspectorIngressTicket::new(
                None,
                inspector_session_id,
                RendererInspectorCommandRoute::MainThread,
            ),
        };
        Self {
            ticket,
            payload: Box::new(command),
            first_dispatch: None,
        }
    }

    pub(crate) fn ticket(&self) -> &RendererInspectorIngressTicket {
        &self.ticket
    }

    pub(crate) fn payload(&self) -> &RendererPageCommand {
        self.payload.as_ref()
    }

    pub(crate) fn first_dispatch_lifecycle(&self) -> RendererInspectorFirstDispatchLifecycle {
        RendererInspectorFirstDispatchLifecycle::OrderedUntilFirstDispatch
    }

    pub(crate) fn nested_dispatch(&self) -> RendererDevToolsMainNestedDispatch {
        match self.payload.as_ref() {
            RendererPageCommand::Inspector(envelope)
                if envelope.can_dispatch_at_nested_inspector_session_boundary() =>
            {
                RendererDevToolsMainNestedDispatch::InspectorSession
            }
            RendererPageCommand::Inspector(_) => RendererDevToolsMainNestedDispatch::OwnerOnly,
            RendererPageCommand::DevToolsMain(_) => {
                unreachable!("a Main DevTools command cannot contain another Main envelope")
            }
            _ => RendererDevToolsMainNestedDispatch::PageAgent,
        }
    }

    pub(crate) fn inspector_envelope(&self) -> Option<&RendererInspectorCommandEnvelope> {
        match self.payload.as_ref() {
            RendererPageCommand::Inspector(envelope) => Some(envelope),
            _ => None,
        }
    }

    pub(crate) fn into_nested_inspector_envelope(self) -> RendererInspectorCommandEnvelope {
        assert_eq!(
            self.nested_dispatch(),
            RendererDevToolsMainNestedDispatch::InspectorSession,
            "only a session-boundary Inspector command may enter nested V8 dispatch"
        );
        let RendererPageCommand::Inspector(envelope) = *self.payload else {
            unreachable!("nested Inspector dispatch kind requires an Inspector payload")
        };
        envelope
    }

    pub(crate) fn into_inspector_envelope(self) -> Option<RendererInspectorCommandEnvelope> {
        match *self.payload {
            RendererPageCommand::Inspector(envelope) => Some(envelope),
            _ => None,
        }
    }

    pub(crate) fn into_nested_page_command(self) -> RendererPageCommand {
        assert_eq!(
            self.nested_dispatch(),
            RendererDevToolsMainNestedDispatch::PageAgent,
            "only a non-V8 Main agent command may enter nested Page dispatch"
        );
        *self.payload
    }

    pub(crate) fn bind_first_dispatch(
        &mut self,
        first_dispatch: crate::script_vm::inspector_main::RendererInspectorMainFirstDispatchGuard,
    ) {
        assert!(
            self.first_dispatch.is_none(),
            "a Main DevTools envelope can own only one first-dispatch permit"
        );
        self.first_dispatch = Some(first_dispatch);
    }

    pub(crate) fn into_owner_parts(
        mut self,
    ) -> (
        crate::script_vm::inspector_main::RendererInspectorMainFirstDispatchGuard,
        RendererPageCommand,
    ) {
        let first_dispatch = self
            .first_dispatch
            .take()
            .expect("an owner-dispatched Main DevTools command must retain its lane permit");
        (first_dispatch, *self.payload)
    }
}

enum RendererInspectorCommandPayload {
    MainThread(RendererInspectorPageCommand),
    Io {
        raw_json: String,
        response: Option<RendererRuntimeInspectorResponseSender>,
    },
}

impl RendererInspectorCommandEnvelope {
    fn new(inspector_session_id: Option<String>, command: RendererInspectorPageCommand) -> Self {
        Self {
            ticket: RendererInspectorIngressTicket::new(
                None,
                inspector_session_id,
                RendererInspectorCommandRoute::MainThread,
            ),
            first_dispatch: RendererInspectorFirstDispatchLifecycle::OrderedUntilFirstDispatch,
            pause_effect: RendererInspectorPauseCommandEffect::None,
            main_dispatch_boundary: RendererInspectorMainDispatchBoundary::PageOwner,
            payload: RendererInspectorCommandPayload::MainThread(command),
        }
    }

    #[doc(hidden)]
    pub fn new_main_protocol(
        ticket: RendererInspectorIngressTicket,
        owner_context_resolution_action: Option<String>,
        raw_json: String,
        response: RendererRuntimeInspectorResponseSender,
    ) -> Self {
        assert_eq!(
            ticket.route(),
            RendererInspectorCommandRoute::MainThread,
            "a Main Inspector protocol payload must use the MainThread route"
        );
        let message = serde_json::from_str::<Value>(&raw_json).ok();
        let main_dispatch_boundary = if main_protocol_can_dispatch_at_inspector_session_boundary(
            owner_context_resolution_action.as_deref(),
            message.as_ref(),
        ) {
            RendererInspectorMainDispatchBoundary::InspectorSession
        } else {
            RendererInspectorMainDispatchBoundary::PageOwner
        };
        let command = match owner_context_resolution_action {
            Some(action) => RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse {
                action,
                raw_json,
                deferred_response: response,
            },
            None => RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithDeferredResponse {
                raw_json,
                deferred_response: response,
            },
        };
        Self {
            ticket,
            first_dispatch: RendererInspectorFirstDispatchLifecycle::OrderedUntilFirstDispatch,
            pause_effect: RendererInspectorPauseCommandEffect::from_message(message.as_ref()),
            main_dispatch_boundary,
            payload: RendererInspectorCommandPayload::MainThread(command),
        }
    }

    #[doc(hidden)]
    pub fn new_io(
        ticket: RendererInspectorIngressTicket,
        raw_json: String,
        response: Option<RendererRuntimeInspectorResponseSender>,
    ) -> Self {
        assert_eq!(
            ticket.route(),
            RendererInspectorCommandRoute::Io,
            "an Inspector IO payload must use the IO route"
        );
        let message = serde_json::from_str::<Value>(&raw_json).ok();
        Self {
            ticket,
            first_dispatch: RendererInspectorFirstDispatchLifecycle::OrderedUntilFirstDispatch,
            pause_effect: RendererInspectorPauseCommandEffect::from_message(message.as_ref()),
            main_dispatch_boundary: RendererInspectorMainDispatchBoundary::InspectorSession,
            payload: RendererInspectorCommandPayload::Io { raw_json, response },
        }
    }

    pub fn ticket(&self) -> &RendererInspectorIngressTicket {
        &self.ticket
    }

    pub fn first_dispatch_lifecycle(&self) -> RendererInspectorFirstDispatchLifecycle {
        self.first_dispatch
    }

    pub(crate) fn pause_effect(&self) -> RendererInspectorPauseCommandEffect {
        self.pause_effect
    }

    pub(crate) fn can_dispatch_at_nested_inspector_session_boundary(&self) -> bool {
        self.main_dispatch_boundary == RendererInspectorMainDispatchBoundary::InspectorSession
    }

    fn bind_attachment(&mut self, attachment: RendererAgentAttachmentId) {
        self.ticket.bind_attachment(attachment);
    }

    pub(crate) fn into_main_thread_parts(
        self,
    ) -> (RendererInspectorIngressTicket, RendererInspectorPageCommand) {
        let RendererInspectorCommandPayload::MainThread(command) = self.payload else {
            panic!("an Inspector IO envelope cannot enter Page owner dispatch");
        };
        (self.ticket, command)
    }

    fn main_thread_payload(&self) -> &RendererInspectorPageCommand {
        let RendererInspectorCommandPayload::MainThread(command) = &self.payload else {
            panic!("an Inspector IO envelope cannot enter Page owner dispatch");
        };
        command
    }

    pub(crate) fn is_main_protocol_command_with_deferred_response(&self) -> bool {
        matches!(
            self.main_thread_payload(),
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithDeferredResponse {
                ..
            } | RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse {
                ..
            }
        )
    }

    #[cfg(test)]
    pub(crate) fn main_protocol_raw_json(&self) -> &str {
        match self.main_thread_payload() {
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithDeferredResponse {
                raw_json,
                ..
            }
            | RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse {
                raw_json,
                ..
            } => raw_json,
            _ => panic!("only a deferred Main protocol command can enter the nested receiver"),
        }
    }

    pub(crate) fn main_protocol_response(&self) -> &RendererRuntimeInspectorResponseSender {
        match self.main_thread_payload() {
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithDeferredResponse {
                deferred_response,
                ..
            }
            | RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse {
                deferred_response,
                ..
            } => deferred_response,
            _ => panic!("only a deferred Main protocol command can enter the nested receiver"),
        }
    }

    pub(crate) fn into_main_protocol_parts(
        self,
    ) -> (
        RendererInspectorIngressTicket,
        String,
        RendererRuntimeInspectorResponseSender,
    ) {
        assert_eq!(
            self.main_dispatch_boundary,
            RendererInspectorMainDispatchBoundary::InspectorSession,
            "a Page-owner-dependent Main command cannot enter direct nested Inspector dispatch"
        );
        match self.payload {
            RendererInspectorCommandPayload::MainThread(
                RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithDeferredResponse {
                    raw_json,
                    deferred_response,
                }
                | RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse {
                    raw_json,
                    deferred_response,
                    ..
                },
            ) => (self.ticket, raw_json, deferred_response),
            _ => panic!("only a deferred Main protocol command can enter the nested receiver"),
        }
    }

    pub(crate) fn io_raw_json(&self) -> &str {
        let RendererInspectorCommandPayload::Io { raw_json, .. } = &self.payload else {
            panic!("a MainThread Inspector envelope cannot enter IO dispatch");
        };
        raw_json
    }

    pub(crate) fn io_response(&self) -> Option<&RendererRuntimeInspectorResponseSender> {
        let RendererInspectorCommandPayload::Io { response, .. } = &self.payload else {
            panic!("a MainThread Inspector envelope cannot enter IO dispatch");
        };
        response.as_ref()
    }

    pub(crate) fn take_io_response(&mut self) -> Option<RendererRuntimeInspectorResponseSender> {
        let RendererInspectorCommandPayload::Io { response, .. } = &mut self.payload else {
            panic!("a MainThread Inspector envelope cannot enter IO dispatch");
        };
        response.take()
    }

    pub(crate) fn requires_materialized_child_realms(&self) -> bool {
        matches!(
            self.main_thread_payload(),
            RendererInspectorPageCommand::RuntimeEnableEvents
        )
    }

    pub(crate) fn cdp_nav_timing_label(&self) -> Option<&'static str> {
        match self.main_thread_payload() {
            RendererInspectorPageCommand::RuntimeEnableEvents => Some("RuntimeEnableEvents"),
            RendererInspectorPageCommand::ApplyRuntimeProtocolState { .. } => {
                Some("ApplyRuntimeProtocolState")
            }
            RendererInspectorPageCommand::DetachRuntimeInspectorSession { .. } => {
                Some("DetachRuntimeInspectorSession")
            }
            RendererInspectorPageCommand::DocumentNodeSnapshotForObjectId { .. } => {
                Some("DocumentNodeSnapshotForObjectId")
            }
            RendererInspectorPageCommand::AccessibilityTreePayloadsForObjectId { .. } => {
                Some("AccessibilityTreePayloadsForObjectId")
            }
            RendererInspectorPageCommand::AccessibilityNodeAndAncestorPayloadsForObjectId {
                ..
            } => Some("AccessibilityNodeAndAncestorPayloadsForObjectId"),
            RendererInspectorPageCommand::AccessibilityPartialTreePayloadsForObjectId {
                ..
            } => Some("AccessibilityPartialTreePayloadsForObjectId"),
            RendererInspectorPageCommand::OuterHtmlForObjectId { .. } => {
                Some("OuterHtmlForObjectId")
            }
            RendererInspectorPageCommand::ScrollObjectNodeIntoViewIfNeeded { .. } => {
                Some("ScrollObjectNodeIntoViewIfNeeded")
            }
            RendererInspectorPageCommand::ClientRectForObjectId { .. } => {
                Some("ClientRectForObjectId")
            }
            RendererInspectorPageCommand::DocumentGeometryForObjectId { .. } => {
                Some("DocumentGeometryForObjectId")
            }
            RendererInspectorPageCommand::NodeHasGeometryForObjectId { .. } => {
                Some("NodeHasGeometryForObjectId")
            }
            RendererInspectorPageCommand::SetFileInputFilesForObjectId { .. } => {
                Some("SetFileInputFilesForObjectId")
            }
            RendererInspectorPageCommand::ResolveRuntimeObjectForBackendNodeId { .. } => {
                Some("ResolveRuntimeObjectForBackendNodeId")
            }
            RendererInspectorPageCommand::ResolveBlobObject { .. } => Some("ResolveBlobObject"),
            _ => None,
        }
    }

    pub(crate) fn uses_cpu_throttling(&self) -> bool {
        matches!(
            self.main_thread_payload(),
            RendererInspectorPageCommand::DispatchRuntimeProtocolMessage { .. }
                | RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithDeferredResponse { .. }
                | RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolution { .. }
                | RendererInspectorPageCommand::DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse { .. }
                | RendererInspectorPageCommand::DomDebuggerGetEventListeners { .. }
                | RendererInspectorPageCommand::FocusDocumentNodeForObjectId { .. }
        )
    }
}

fn main_protocol_can_dispatch_at_inspector_session_boundary(
    owner_context_resolution_action: Option<&str>,
    message: Option<&Value>,
) -> bool {
    let Some(message) = message else {
        return false;
    };
    let method = message.get("method").and_then(Value::as_str);
    let params = message.get("params").and_then(Value::as_object);

    // These transitions have renderer-owned restore state and must therefore
    // pass through PageVm even though their final sink is V8InspectorSession.
    if matches!(
        method,
        Some("Runtime.enable" | "Runtime.disable" | "Console.enable" | "Console.disable")
    ) {
        return false;
    }

    let has_owner_scoped_runtime_semantics = params.is_some_and(|params| {
        params.get("userGesture").and_then(Value::as_bool) == Some(true)
            || params.contains_key(crate::script_vm::WEBDRIVER_BIDI_FILE_PROMPT_HANDLER_PARAM)
    });
    if has_owner_scoped_runtime_semantics {
        return false;
    }

    match owner_context_resolution_action {
        // Page.createIsolatedWorld and Runtime.executionContextCreated expose
        // V8 Inspector's native context id. Context-targeted commands can
        // therefore enter Chromium's nested Main receiver without borrowing
        // PageVm; an unknown or retired id is rejected by V8 itself.
        Some("evaluate") => true,
        // A target supplied by object, unique context, or numeric Inspector
        // context is complete. Only the target-less compatibility form needs
        // PageVm to insert Moli's default execution context.
        Some("callFunctionOn") => params.is_some_and(|params| {
            params.contains_key("objectId")
                || params.contains_key("uniqueContextId")
                || params.contains_key("executionContextId")
        }),
        Some(_) => false,
        None => true,
    }
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
    #[doc(hidden)]
    DevToolsMain(RendererDevToolsMainCommandEnvelope),
    Inspector(RendererInspectorCommandEnvelope),
    EvaluateExpression {
        expression: String,
        await_promise: bool,
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

    #[cfg(test)]
    pub(crate) fn inspector_first_dispatch_lifecycle(
        &self,
    ) -> Option<RendererInspectorFirstDispatchLifecycle> {
        match self {
            Self::Inspector(envelope) => Some(envelope.first_dispatch_lifecycle()),
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
        assert_eq!(
            command.inspector_first_dispatch_lifecycle(),
            Some(RendererInspectorFirstDispatchLifecycle::OrderedUntilFirstDispatch)
        );
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
                final_url,
                document_title,
                status,
                headers: Vec::new(),
                script_execution: Arc::new(ScriptExecutionReport::default()),
                idle_override: None,
                service_worker_client_id: 0,
                dedicated_worker_running_worker_isolate_count: 0,
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
