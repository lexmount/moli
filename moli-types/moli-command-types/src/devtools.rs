//! External Inspector command parameters and immutable frontend reply data.
//!
//! Callback execution, response publication and session lifetime ownership are
//! intentionally provided by the renderer, never implemented in this module.

use moli_page_types::{
    DevToolsSessionKey, FrontendCommandId, RendererAgentAttachmentId,
    RendererInspectorSessionRestoreSnapshot,
};

/// Opaque execution-owned values carried by a command. Defining these types
/// here does not transfer execution or resource ownership to the data layer.
pub trait InspectorCommandHandles {
    type ResponseSender;
    type RuntimeOutput;
    type SessionDetachGuard;
}

pub enum InspectorCommand<Handles: InspectorCommandHandles> {
    DispatchRuntimeProtocolMessage {
        raw_json: String,
    },
    DispatchRuntimeProtocolMessageWithDeferredResponse {
        raw_json: String,
        deferred_response: Handles::ResponseSender,
    },
    DispatchRuntimeProtocolMessageWithContextResolution {
        action: String,
        raw_json: String,
    },
    DispatchRuntimeProtocolMessageWithContextResolutionAndDeferredResponse {
        action: String,
        raw_json: String,
        deferred_response: Handles::ResponseSender,
    },
    RuntimeEnableEvents,
    RuntimeDisableEvents,
    RuntimeBindingEvents {
        binding: moli_page_types::RuntimeBindingRegistration,
        remove: bool,
    },
    /// The V8 session already executed removal. Only native restore metadata
    /// remains to be committed before publishing the original native terminal.
    CommitRuntimeBindingRemoval {
        name: String,
        output: Result<Handles::RuntimeOutput, String>,
    },
    ApplyRuntimeProtocolState {
        session_restore_snapshots: Vec<RendererInspectorSessionRestoreSnapshot>,
        isolated_worlds: Vec<moli_page_types::RuntimeIsolatedWorldDefinition>,
        stored_runtime_bindings: Vec<moli_page_types::RuntimeBindingRegistration>,
        session_runtime_bindings: Vec<moli_page_types::RuntimeBindingRegistration>,
    },
    DetachRuntimeInspectorSession {
        pause_guard: Handles::SessionDetachGuard,
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
    PrepareElementClick {
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
        files: Vec<moli_dom::native::SelectedFile>,
        append: bool,
    },
    DocumentNodeSnapshotForObjectId {
        include_whitespace: bool,
        object_id: String,
        depth: i32,
        pierce: bool,
        bind_frontend_nodes: bool,
    },
    RequestDocumentNodeForObjectId {
        include_whitespace: bool,
        object_id: String,
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

/// A resolved native frontend reply. The projection captures only immutable
/// protocol metadata at admission; it cannot borrow a Browser or live Page.
/// Both success and backend errors are published at the renderer boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeProtocolResponse<Output> {
    pub result: Result<serde_json::Value, NativeProtocolError>,
    pub notifications: Vec<NativeProtocolNotification>,
    pub state_updates: Vec<NativeProtocolStateUpdate<Output>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeProtocolError {
    pub code: i32,
    pub message: String,
}

/// Native notifications keep the semantic sidecars needed by non-CDP
/// observers. JSON-only events and DOM automation events share one ordered
/// prefix; neither can be reconstructed from mutable Page state at drain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeProtocolNotification {
    Json(serde_json::Value),
    DomSetChildNodes {
        parent_node_id: u32,
        nodes: Vec<serde_json::Value>,
    },
}

impl From<serde_json::Value> for NativeProtocolNotification {
    fn from(value: serde_json::Value) -> Self {
        Self::Json(value)
    }
}

impl<Output> NativeProtocolResponse<Output> {
    pub fn success(result: serde_json::Value) -> Self {
        Self {
            result: Ok(result),
            notifications: Vec::new(),
            state_updates: Vec::new(),
        }
    }

    pub fn error(code: i32, message: impl Into<String>) -> Self {
        Self {
            result: Err(NativeProtocolError {
                code,
                message: message.into(),
            }),
            notifications: Vec::new(),
            state_updates: Vec::new(),
        }
    }
}

/// Browser mirrors committed with a native terminal. These are frozen data,
/// never callbacks into a live Page. Attachment retirement discards document
/// updates; command-local replay can instead fail the terminal at projection.
#[derive(Clone, Debug, PartialEq)]
pub enum NativeProtocolStateUpdate<Output> {
    /// Frozen V8 replay and state, projected by the ordered protocol consumer
    /// before this terminal. Retirement can still turn enable into an error;
    /// neither the adapter waiter nor a replacement attachment owns that choice.
    RuntimeSubscription {
        enabled: bool,
        output: Box<Output>,
    },
    /// Persistent binding mutations share the subscription's ordered ingress;
    /// adapter completion cannot restore bindings cleared by a later disable.
    RuntimeBinding {
        binding: moli_page_types::RuntimeBindingRegistration,
        remove: bool,
        output: Box<Output>,
    },
    WebMcpEnabled(bool),
    /// A DOM-domain fallback snapshot, scoped to the producing session.
    DomRemoteObjectNode {
        object_id: String,
        node: serde_json::Value,
    },
}

/// A complete reply to the original frontend call, not a live Page observation.
/// Its metadata remains valid even if the Page is replaced before projection.
/// The frontend router uses its globally allocated call id to deliver it once.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeCommandTerminal<Output> {
    pub command_id: FrontendCommandId,
    pub session: DevToolsSessionKey,
    pub attachment_id: RendererAgentAttachmentId,
    pub reply: NativeProtocolResponse<Output>,
}

impl crate::InspectorPauseEffect {
    pub fn from_message(message: Option<&serde_json::Value>) -> Self {
        match message
            .and_then(|message| message.get("method"))
            .and_then(serde_json::Value::as_str)
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

/// Opaque capabilities retained by the worker runtime.
pub trait WorkerCommandHandles {
    type SessionLifetime;
    type ResponseSender;
    type ProtocolResponseSender;
}

pub enum WorkerCommand<Handles: WorkerCommandHandles> {
    DispatchProtocolMessage {
        lifetime: Handles::SessionLifetime,
        inspector_session_id: Option<String>,
        raw_json: String,
        deferred_response: Option<Handles::ResponseSender>,
        response_tx: Handles::ProtocolResponseSender,
    },
    AttachSession {
        lifetime: Handles::SessionLifetime,
        inspector_session_id: Option<String>,
    },
    DetachSession {
        lifetime: Handles::SessionLifetime,
        inspector_session_id: Option<String>,
    },
    RunIfWaitingForDebugger,
}
