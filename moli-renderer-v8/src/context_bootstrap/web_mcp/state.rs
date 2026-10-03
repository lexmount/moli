//! Document registrations and invocation state retained by the renderer.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use moli_page_types::DevToolsSessionKey;

use crate::{
    document_runtime::DomHandle, frame_owner_model::FrameDocumentNavigationLoadBinding,
    native_bridge::WindowDocumentOwner, script_vm::RendererInspectorObjectWrapper,
    window_webidl_callback::WindowWebIdlCallbackFunction,
};

use super::conversion::ToolAnnotations;

#[derive(Default)]
pub(crate) struct ModelContextStore {
    pub(super) documents: HashMap<DomHandle, DocumentTools>,
    pub(super) next_registration: u64,
    pub(super) pending: HashMap<u64, PendingInvocation>,
    pub(super) deliveries: HashMap<u64, PendingDelivery>,
    pub(super) retired_deliveries: Vec<PendingDelivery>,
    pub(super) enabled_sessions: BTreeSet<DevToolsSessionKey>,
    pub(super) object_wrapper: Option<RendererInspectorObjectWrapper>,
    pub(super) retired_resolvers: Vec<v8::Global<v8::PromiseResolver>>,
    pub(super) retired_aborts: Vec<AbortRegistration>,
    pub(super) retired_invocations: Vec<PendingInvocation>,
    pub(super) retirement_task_queued: bool,
    pub(super) dirty_form_documents: HashSet<DomHandle>,
    pub(super) form_registration_task_queued: bool,
    pub(super) initialized_form_owner: Option<WindowDocumentOwner>,
    pub(super) child_navigations: HashMap<
        DomHandle,
        (
            FrameDocumentNavigationLoadBinding,
            moli_page_types::RendererWebMcpNavigation,
        ),
    >,
    pub(super) navigation_results:
        HashMap<WindowDocumentOwner, moli_page_types::RendererWebMcpNavigation>,
}

pub(super) struct DocumentTools {
    pub(super) owner: WindowDocumentOwner,
    pub(super) target: v8::Global<v8::Object>,
    pub(super) window: v8::Global<v8::Object>,
    pub(super) origin: url::Origin,
    pub(super) frame_id: Option<String>,
    pub(super) frame_tree: Option<u64>,
    pub(super) tools: BTreeMap<String, RegisteredTool>,
}

pub(super) struct RegisteredTool {
    pub(super) registration: u64,
    pub(super) metadata: ToolMetadata,
    pub(super) stack_trace: Option<serde_json::Value>,
    pub(super) exposed_to: Vec<url::Origin>,
    pub(super) executor: ToolExecutor,
    pub(super) abort: Option<AbortRegistration>,
    pub(super) registration_resolver: Option<v8::Global<v8::PromiseResolver>>,
}

#[derive(PartialEq, Eq)]
pub(super) struct ToolMetadata {
    pub(super) description: String,
    pub(super) title: String,
    pub(super) input_schema: Option<String>,
    pub(super) annotations: Option<ToolAnnotations>,
}

pub(super) enum ToolExecutor {
    Callback(WindowWebIdlCallbackFunction),
    Form {
        handle: DomHandle,
        autosubmit: bool,
        backend_node_id: Option<u32>,
    },
}

pub(super) struct AbortRegistration {
    pub(super) signal: v8::Global<v8::Object>,
    pub(super) algorithm: v8::Global<v8::Function>,
}

pub(super) struct PendingInvocation {
    pub(super) document: DomHandle,
    pub(super) owner: WindowDocumentOwner,
    pub(super) caller_owner: WindowDocumentOwner,
    pub(super) caller_document: DomHandle,
    pub(super) frame_tree: Option<u64>,
    pub(super) name: String,
    pub(super) target: v8::Global<v8::Object>,
    pub(super) resolver: Option<v8::Global<v8::PromiseResolver>>,
    pub(super) signal: v8::Global<v8::Object>,
    pub(super) caller_abort: Option<AbortRegistration>,
    pub(super) input: String,
    pub(super) caller_cancel_requested: bool,
    pub(super) form: Option<FormInvocation>,
}

pub(super) struct FormInvocation {
    pub(super) handle: DomHandle,
    pub(super) submitter: Option<DomHandle>,
    pub(super) state: FormInvocationState,
}

pub(super) enum FormInvocationState {
    Filling,
    Ready,
    Submitting,
    Responding(v8::Global<v8::Promise>),
    Invalidated,
    Navigating,
}

impl ModelContextStore {
    pub(crate) fn set_object_wrapper(&mut self, wrapper: RendererInspectorObjectWrapper) {
        self.object_wrapper = Some(wrapper);
    }
    pub(crate) fn disable_session(&mut self, session: &DevToolsSessionKey) {
        self.enabled_sessions.remove(session);
    }
}

// Target execution has ended; the caller may still cancel until its task runs.
pub(super) struct PendingDelivery {
    pub(super) caller_owner: WindowDocumentOwner,
    pub(super) caller_document: DomHandle,
    pub(super) resolver: v8::Global<v8::PromiseResolver>,
    pub(super) caller_abort: Option<AbortRegistration>,
    pub(super) result: CallerResult,
}

pub(super) enum CallerResult {
    Completed(String),
    Navigated,
    Error(String),
}
