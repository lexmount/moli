mod agent;
mod agent_sessions;
mod context_registration;
mod context_registry;
mod document_backend;
mod input_state;
mod outbound;
mod v8_backend;

use self::agent::RendererDevToolsAgent;
pub(crate) use self::agent::{
    RendererDomDebuggerPauseScheduler, RendererDomDebuggerScheduledPause,
};
pub(super) use self::agent_sessions::PageInspectorSessionTarget;
use self::agent_sessions::inspector_session_key;
use self::context_registration::DocumentInspectorContextRegistrations;
#[cfg(test)]
use self::context_registry::DocumentInspectorContextGroupId;
pub(in crate::script_vm) use self::context_registry::DocumentInspectorContextRegistrationId;
use self::document_backend::DocumentInspectorBackendState;
pub(super) use self::input_state::{
    current_selection_range, current_selection_state, is_space_key, key_target_info,
    option_is_disabled, radio_group_members,
};
pub(in crate::script_vm) use self::outbound::InspectorOutbound;
pub(super) use self::outbound::ScriptVmInspectorCommandTurnOutputScope;
#[cfg(test)]
use self::v8_backend::RendererInspectorClientUniqueIdState;
pub(in crate::script_vm) use self::v8_backend::RendererInspectorIsolateBackend;
pub(crate) use self::v8_backend::{
    RendererInspectorIsolateBackendHandle, dispatch_inspector_io_owner_wake,
    dispatch_inspector_main_owner_wake,
};
use crate::devtools::target::RendererDevToolsTargetHandle;
#[cfg(test)]
use crate::runtime::RendererRuntimeCommandOutputRecorder;
use crate::runtime::RendererRuntimeInspectorResponseSender;
use crate::{
    frame_owner_model::FrameRealmId,
    protocol_types::RuntimeBindingRegistration,
    script_vm::document_isolate::RendererDocumentIsolateHandle,
    {document_runtime::DomHandle, runtime::RendererCommandTurnOutputRecorder},
};
use anyhow::Result;
use moli_page_types::{
    DevToolsSessionKey, RendererDevToolsAgentToken, RendererInspectorResponseDelivery,
    RendererInspectorSessionRestoreSnapshot, V8InspectorSessionState,
};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::HashMap};
use url::Url;

pub(crate) struct DocumentInspectorBinding {
    context_registrations: DocumentInspectorContextRegistrations,
    agent: RendererDevToolsAgent,
    backend: RefCell<DocumentInspectorBackendState>,
    devtools_target: RendererDevToolsTargetHandle,
}

impl std::fmt::Debug for DocumentInspectorBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentInspectorBinding")
            .field("agent", &self.agent)
            .finish_non_exhaustive()
    }
}

impl Drop for DocumentInspectorBinding {
    fn drop(&mut self) {
        self.context_registrations.destroy_all();
        self.agent.deactivate_all_routes();
    }
}

/// Renderer-side default-world realm for one child frame's current local window.
pub(super) struct ChildFrameRealmRecord {
    pub(super) frame_id: String,
    pub(super) child_handle: DomHandle,
    pub(super) local_window_id: crate::frame_owner_model::LocalWindowId,
    pub(super) owner_realm_id: FrameRealmId,
    pub(super) context: v8::Global<v8::Context>,
    pub(super) _bridge_ref: crate::native_bridge::JsContextHostBridgeRef,
    pub(super) runtime_observable_context_token:
        crate::native_bridge::RuntimeObservableContextToken,
    pub(super) inspector_execution_context_id: i64,
    pub(super) inspector_execution_context_realm_id: Option<String>,
    pub(super) inspector_context_registration_id: DocumentInspectorContextRegistrationId,
}

pub(super) struct ReportedExecutionContext {
    pub(super) id: i64,
    pub(super) unique_id: Option<String>,
}

impl DocumentInspectorBinding {
    pub(crate) fn new(isolate_backend: RendererInspectorIsolateBackendHandle) -> Self {
        let devtools_target = isolate_backend.devtools_target();
        Self {
            context_registrations: DocumentInspectorContextRegistrations::default(),
            agent: RendererDevToolsAgent::new(isolate_backend),
            backend: RefCell::new(DocumentInspectorBackendState::new()),
            devtools_target,
        }
    }

    pub(crate) fn with_output_journal(
        self,
        output_journal: crate::runtime::RendererTurnOutputJournal,
    ) -> Self {
        self.agent.bind_output_journal(output_journal);
        self
    }

    pub(crate) fn devtools_target(&self) -> RendererDevToolsTargetHandle {
        self.devtools_target.clone()
    }

    pub(crate) fn dom_debugger_pause_scheduler(&self) -> RendererDomDebuggerPauseScheduler {
        self.agent.dom_debugger_pause_scheduler()
    }

    pub(crate) fn deactivate_page_vm_binding_for_teardown(&mut self) {
        self.agent.deactivate_all_routes();
    }

    pub(super) fn with_session_and_outbound<T>(
        &self,
        backend: &mut RendererInspectorIsolateBackend,
        target: PageInspectorSessionTarget<'_>,
        op: impl FnOnce(
            &v8::inspector::V8InspectorSession,
            InspectorOutbound,
            Vec<RuntimeBindingRegistration>,
        ) -> T,
    ) -> T {
        match target {
            PageInspectorSessionTarget::InternalRuntimeEvaluate => {
                self.agent.with_internal_runtime_evaluate(backend, op)
            }
            PageInspectorSessionTarget::Frontend(inspector_session_id) => {
                let session_key = inspector_session_key(inspector_session_id);
                self.agent.with_frontend(backend, session_key, op)
            }
        }
    }

    pub(super) fn route_frontend_response(
        &self,
        inspector_session_id: Option<&str>,
        response: RendererRuntimeInspectorResponseSender,
    ) -> Result<RendererRuntimeInspectorResponseSender> {
        if response.response_delivery() == RendererInspectorResponseDelivery::CommandReply {
            return Ok(response);
        }
        let session_key = inspector_session_key(inspector_session_id);
        let outbound = self
            .agent
            .frontend_outbound(&session_key)
            .ok_or_else(|| anyhow::anyhow!("Inspector frontend session is not available"))?;
        Ok(outbound.route_frontend_response(response))
    }

    pub(super) fn v8_session_state(
        &self,
        inspector_session_id: Option<&str>,
    ) -> Option<V8InspectorSessionState> {
        let session_key = inspector_session_key(inspector_session_id);
        self.agent.v8_state(&session_key)
    }

    pub(super) fn v8_session_states(&self) -> Vec<(DevToolsSessionKey, V8InspectorSessionState)> {
        self.agent.v8_states()
    }

    pub(super) fn reattach_v8_sessions(
        &self,
        backend: &mut RendererInspectorIsolateBackend,
        restores: &[RendererInspectorSessionRestoreSnapshot],
    ) {
        for restore in restores {
            let Some(v8_state) = restore.v8_attach.reattach_state() else {
                continue;
            };
            let session_key = inspector_session_key(restore.inspector_session_id.as_deref());
            self.agent.reattach_frontend(backend, session_key, v8_state);
        }
    }

    pub(super) fn ensure_frontend_session(
        &self,
        backend: &mut RendererInspectorIsolateBackend,
        inspector_session_id: Option<&str>,
    ) {
        self.agent
            .ensure_frontend(backend, inspector_session_key(inspector_session_id));
    }

    pub(super) fn end_runtime_command_output_for_session(
        &self,
        inspector_session_id: Option<&str>,
    ) {
        let session_key = inspector_session_key(inspector_session_id);
        self.agent.end_runtime_command_output(&session_key);
    }

    pub(super) fn begin_command_turn_output(
        &self,
        recorder: RendererCommandTurnOutputRecorder,
    ) -> Result<ScriptVmInspectorCommandTurnOutputScope> {
        ScriptVmInspectorCommandTurnOutputScope::begin(
            self.agent.frontend_routes().into_iter(),
            recorder,
        )
    }

    pub(super) fn set_runtime_bindings_for_session(
        &self,
        inspector_session_id: Option<&str>,
        bindings: &[RuntimeBindingRegistration],
    ) {
        let session_key = inspector_session_key(inspector_session_id);
        self.agent.set_runtime_bindings(session_key, bindings);
    }

    pub(super) fn runtime_bindings_for_session(
        &self,
        inspector_session_id: Option<&str>,
    ) -> Vec<RuntimeBindingRegistration> {
        let session_key = inspector_session_key(inspector_session_id);
        self.agent.runtime_bindings(&session_key)
    }

    pub(super) fn detach_session(&self, inspector_session_id: Option<&str>) -> bool {
        let session_key = inspector_session_key(inspector_session_id);
        self.agent.remove_frontend(&session_key)
    }

    pub(super) fn outbound_len(&self) -> usize {
        self.observable_outbound_routes()
            .into_iter()
            .map(|(_, outbound)| outbound.len())
            .sum::<usize>()
    }

    pub(super) fn session_count_for_diagnostics(&self) -> usize {
        self.agent.session_count()
    }

    pub(crate) fn agent_token(&self) -> RendererDevToolsAgentToken {
        self.agent.token()
    }

    pub(super) fn registry_owner_for_diagnostics(&self) -> &'static str {
        "renderer-devtools-agent"
    }

    pub(super) fn registry_lifetime_scope_for_diagnostics(&self) -> &'static str {
        "local-root-agent"
    }

    pub(super) fn context_group_id_for_diagnostics(&self) -> i32 {
        self.agent.context_group_id().get()
    }

    pub(super) fn detach_default_context_from_backend_if_same(
        &self,
        backend: &RendererInspectorIsolateBackend,
    ) {
        self.agent.assert_isolate_backend(backend);
        let Some(registration_id) = self.context_registrations.default_registration_id() else {
            return;
        };
        backend.detach_default_context_if_same(self.agent.context_group_id(), registration_id);
    }

    pub(super) fn destroy_context_registration(
        &mut self,
        registration_id: DocumentInspectorContextRegistrationId,
    ) -> bool {
        self.context_registrations.destroy(registration_id)
    }

    pub(super) fn destroy_all_context_registrations(&mut self) {
        self.context_registrations.destroy_all();
    }

    pub(super) fn context_registration_count_for_diagnostics(&self) -> usize {
        self.context_registrations.len()
    }

    pub(super) fn take_outbound_messages_for_session(
        &self,
        inspector_session_id: Option<&str>,
    ) -> Vec<Value> {
        let session_key = inspector_session_key(inspector_session_id);
        self.observable_outbound_routes()
            .into_iter()
            .find(|(key, _)| key == &session_key)
            .map(|(_, outbound)| outbound.take_pending_messages())
            .unwrap_or_default()
    }

    pub(super) fn cancel_response_callback_for_session(
        &self,
        inspector_session_id: Option<&str>,
        call_id: i32,
    ) {
        let session_key = inspector_session_key(inspector_session_id);
        self.agent.cancel_response_callback(&session_key, call_id);
    }

    pub(super) fn cancel_internal_runtime_evaluate_response(&self, call_id: i32) {
        self.agent
            .cancel_internal_runtime_evaluate_response(call_id);
    }

    pub(super) fn attach_context<'s>(
        &mut self,
        renderer_document_isolate: RendererDocumentIsolateHandle,
        backend: &mut RendererInspectorIsolateBackend,
        context: v8::Local<'s, v8::Context>,
        default_context: v8::Global<v8::Context>,
        registered_context: v8::Global<v8::Context>,
        final_url: &Url,
        root_frame_id: Option<&str>,
    ) {
        self.agent.assert_isolate_backend(backend);
        let context_group_id = self.agent.context_group_id();
        if backend.reset_default_context_group_before_replacement(context_group_id) {
            self.backend.borrow_mut().runtime_realms.clear();
        }
        self.agent
            .ensure_frontend(backend, DevToolsSessionKey::Primary);
        let mut aux_data = serde_json::Map::new();
        aux_data.insert("isDefault".to_owned(), json!(true));
        aux_data.insert("type".to_owned(), json!("default"));
        if let Some(root_frame_id) = root_frame_id {
            aux_data.insert("frameId".to_owned(), json!(root_frame_id));
        }
        let aux_data = serde_json::to_string(&Value::Object(aux_data))
            .expect("default execution context aux data should serialize");
        let origin = inspector_execution_context_origin(final_url);
        let context_unique_id = backend.context_created_with_unique_id(
            context,
            context_group_id,
            final_url.as_str().as_bytes(),
            origin.as_bytes(),
            aux_data.as_bytes(),
        );
        let execution_context_id = v8::inspector::V8Inspector::execution_context_id(context);
        let registration_id = self.context_registrations.register_default(
            renderer_document_isolate,
            self.agent.isolate_backend_handle(),
            context_group_id,
            registered_context,
        );
        backend.context_registry.set_default_context(
            context_group_id,
            default_context,
            registration_id,
        );
        self.backend
            .borrow_mut()
            .runtime_realms
            .record_attached_default_context(i64::from(execution_context_id), context_unique_id);
    }

    pub(super) fn default_execution_context_id(&self) -> Option<i64> {
        self.backend
            .borrow()
            .runtime_realms
            .default_execution_context_id()
    }

    pub(super) fn default_execution_context_realm_id(&self) -> Option<String> {
        self.backend
            .borrow()
            .runtime_realms
            .default_execution_context_realm_id()
    }

    pub(super) fn initial_default_execution_context_id(&self) -> Option<i64> {
        self.backend
            .borrow()
            .runtime_realms
            .initial_default_execution_context_id()
    }

    pub(super) fn initial_default_execution_context_realm_id(&self) -> Option<String> {
        self.backend
            .borrow()
            .runtime_realms
            .initial_default_execution_context_realm_id()
    }

    pub(super) fn attach_isolated_context<'s>(
        &mut self,
        renderer_document_isolate: RendererDocumentIsolateHandle,
        backend: &mut RendererInspectorIsolateBackend,
        context: v8::Local<'s, v8::Context>,
        registered_context: v8::Global<v8::Context>,
        replaced_registration_id: Option<DocumentInspectorContextRegistrationId>,
        name: &str,
        grant_universal_access: bool,
        frame_id: Option<&str>,
    ) -> Result<
        Option<(
            ReportedExecutionContext,
            DocumentInspectorContextRegistrationId,
        )>,
    > {
        self.agent.assert_isolate_backend(backend);
        let snap = self.outbound_snapshots();
        let mut aux_data = serde_json::Map::new();
        aux_data.insert("isDefault".to_owned(), json!(false));
        aux_data.insert("type".to_owned(), json!("isolated"));
        aux_data.insert(
            "grantUniversalAccess".to_owned(),
            json!(grant_universal_access),
        );
        if let Some(frame_id) = frame_id {
            aux_data.insert("frameId".to_owned(), json!(frame_id));
        }
        let aux_data = serde_json::to_string(&Value::Object(aux_data))?;
        let context_group_id = self.agent.context_group_id();
        let context_unique_id = backend.context_created_with_unique_id(
            context,
            context_group_id,
            name.as_bytes(),
            b"",
            aux_data.as_bytes(),
        );
        let v8_context_id = v8::inspector::V8Inspector::execution_context_id(context);
        let messages = self.outbound_after_snapshots(&snap);
        let mut reported_context =
            reported_execution_context_created(&messages).unwrap_or_else(|| {
                ReportedExecutionContext {
                    id: i64::from(v8_context_id),
                    unique_id: None,
                }
            });
        if reported_context.unique_id.is_none() {
            reported_context.unique_id = context_unique_id;
        }
        let registration_id = self.context_registrations.register_non_default(
            renderer_document_isolate,
            self.agent.isolate_backend_handle(),
            registered_context,
            replaced_registration_id,
        );
        Ok(Some((reported_context, registration_id)))
    }

    pub(super) fn attach_child_default_context<'s>(
        &mut self,
        renderer_document_isolate: RendererDocumentIsolateHandle,
        backend: &mut RendererInspectorIsolateBackend,
        context: v8::Local<'s, v8::Context>,
        registered_context: v8::Global<v8::Context>,
        document_url: &Url,
        frame_id: &str,
    ) -> Result<(
        ReportedExecutionContext,
        DocumentInspectorContextRegistrationId,
    )> {
        self.agent.assert_isolate_backend(backend);
        let snap = self.outbound_snapshots();
        let aux_data = serde_json::to_string(&json!({
            "isDefault": true,
            "type": "default",
            "frameId": frame_id,
        }))?;
        let context_group_id = self.agent.context_group_id();
        let origin = inspector_execution_context_origin(document_url);
        let context_unique_id = backend.context_created_with_unique_id(
            context,
            context_group_id,
            document_url.as_str().as_bytes(),
            origin.as_bytes(),
            aux_data.as_bytes(),
        );
        let v8_context_id = v8::inspector::V8Inspector::execution_context_id(context);
        let messages = self.outbound_after_snapshots(&snap);
        let mut reported_context =
            reported_execution_context_created(&messages).unwrap_or_else(|| {
                ReportedExecutionContext {
                    id: i64::from(v8_context_id),
                    unique_id: None,
                }
            });
        if reported_context.unique_id.is_none() {
            reported_context.unique_id = context_unique_id;
        }
        let registration_id = self.context_registrations.register_non_default(
            renderer_document_isolate,
            self.agent.isolate_backend_handle(),
            registered_context,
            None,
        );
        Ok((reported_context, registration_id))
    }

    pub(super) fn record_execution_context_state(
        &mut self,
        messages: &[Value],
        root_frame_id: Option<&str>,
    ) {
        self.backend
            .borrow_mut()
            .runtime_realms
            .record_execution_context_state(messages, root_frame_id);
    }

    fn outbound_snapshots(&self) -> HashMap<DevToolsSessionKey, usize> {
        self.observable_outbound_routes()
            .into_iter()
            .map(|(key, outbound)| (key, outbound.len()))
            .collect()
    }

    fn outbound_after_snapshots(
        &self,
        snapshots: &HashMap<DevToolsSessionKey, usize>,
    ) -> Vec<Value> {
        let mut messages = Vec::new();
        for (key, outbound) in self.observable_outbound_routes() {
            let snapshot_len = snapshots.get(&key).copied().unwrap_or_default();
            messages.extend(outbound.values_after(snapshot_len));
        }
        messages
    }

    /// Returns the session routes owned by this renderer agent.
    ///
    /// Protocol attachment identity, not a renderer target queue, decides
    /// whether these messages are frontend-visible.
    fn observable_outbound_routes(&self) -> Vec<(DevToolsSessionKey, InspectorOutbound)> {
        self.agent.frontend_routes()
    }
}

fn inspector_execution_context_origin(url: &Url) -> String {
    let origin = moli_url::origin_ascii_serialization(url);
    if origin == "null" {
        "://".to_owned()
    } else {
        origin
    }
}

fn reported_execution_context_created(messages: &[Value]) -> Option<ReportedExecutionContext> {
    let context = messages
        .iter()
        .find(|message| message["method"] == json!("Runtime.executionContextCreated"))
        .map(|message| &message["params"]["context"])?;
    Some(ReportedExecutionContext {
        id: context["id"].as_i64()?,
        unique_id: context["uniqueId"].as_str().map(str::to_owned),
    })
}

#[cfg(test)]
mod tests;
