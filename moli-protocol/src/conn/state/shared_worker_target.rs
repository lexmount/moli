use std::collections::BTreeMap;

use moli_core::{
    RendererOwnerLocalHostId,
    page::{RendererSharedWorkerConsoleMessage, RuntimeConsoleMessageSnapshot},
};
use moli_shared_worker::SharedWorkerInstanceId;

use crate::devtools_runtime::RuntimeExecutionContextEvent;

#[cfg(test)]
use super::session::InspectorSessionState;
use super::{
    DevToolsSessionState, DuplicatePendingRendererCommand, PreparedRendererCallDispatch,
    RegisterRendererCallError, RendererCommandCorrelation, RendererCommandDescriptor,
    page_slot::RuntimeBindingDefinition,
    shared_worker_attachment::{
        TargetSharedWorkerProtocolAttachmentIdentity,
        TargetSharedWorkerProtocolAttachmentRetirement, TargetSharedWorkerProtocolAttachmentScope,
    },
    target_state::PendingInspectorAwait,
};

// Chromium does not reserve negative Runtime.ExecutionContextId values for
// worker target types. It gets the integer id from V8 inspector, scopes
// inspector sessions by contextGroupId, and exposes uniqueId for cross-process
// context identity. This Moli-only fallback bridges target/log/realm
// projections that can exist before Runtime.executionContextCreated is recorded.
// Keep it in a negative worker-typed range so it cannot be confused with a real
// renderer/V8 context id, then rebind snapshots once the real id arrives.
const SHARED_WORKER_SYNTHETIC_EXECUTION_CONTEXT_ID_BASE: i64 = -10_000_000;

/// CDP-side projection for a renderer-owned `shared_worker` target.
///
/// Renderer `SharedWorkerHost` events are the lifecycle source of truth. This
/// state only tracks protocol attachment, Runtime/Console cursors, bindings,
/// remote-object ownership, and pending inspector replies for a target that has
/// already been created by the renderer event stream.
#[derive(Debug)]
pub(crate) struct SharedWorkerTargetState {
    pub(crate) renderer_owner_local_host_id: RendererOwnerLocalHostId,
    pub(crate) renderer_instance_id: SharedWorkerInstanceId,
    pub(crate) target_id: String,
    owner_target_id: Option<String>,
    sessions: BTreeMap<String, SharedWorkerTargetSessionState>,
    pub(crate) url: String,
    pub(crate) name: String,
    runtime_execution_context_id: Option<i64>,
    console_messages: Vec<RuntimeConsoleMessageSnapshot>,
}

#[derive(Debug)]
struct SharedWorkerTargetSessionState {
    devtools: DevToolsSessionState,
    attachment_scope: TargetSharedWorkerProtocolAttachmentScope,
}

impl Default for SharedWorkerTargetSessionState {
    fn default() -> Self {
        Self {
            devtools: DevToolsSessionState::default(),
            attachment_scope: TargetSharedWorkerProtocolAttachmentScope::new(),
        }
    }
}

impl SharedWorkerTargetState {
    pub(crate) fn new(
        renderer_owner_local_host_id: RendererOwnerLocalHostId,
        renderer_instance_id: SharedWorkerInstanceId,
        target_id: String,
        owner_target_id: Option<String>,
        url: String,
        name: String,
    ) -> Self {
        Self {
            renderer_owner_local_host_id,
            renderer_instance_id,
            target_id,
            owner_target_id,
            sessions: BTreeMap::new(),
            url,
            name,
            runtime_execution_context_id: None,
            console_messages: Vec::new(),
        }
    }

    pub(crate) fn execution_context_id(&self) -> i64 {
        if let Some(id) = self.runtime_execution_context_id {
            return id;
        }
        let instance_id = i64::try_from(self.renderer_instance_id.as_u64()).unwrap_or(i64::MAX);
        SHARED_WORKER_SYNTHETIC_EXECUTION_CONTEXT_ID_BASE.saturating_sub(instance_id)
    }

    pub(crate) fn real_runtime_execution_context_id(&self) -> Option<i64> {
        self.runtime_execution_context_id
    }

    fn rebind_synthetic_runtime_snapshots(&mut self, synthetic_id: i64, real_id: i64) {
        for message in &mut self.console_messages {
            if message.execution_context_id == synthetic_id {
                message.execution_context_id = real_id;
            }
        }
    }

    pub(crate) fn record_runtime_execution_context_created_event(
        &mut self,
        event: &RuntimeExecutionContextEvent,
    ) {
        if event.context_type.as_deref() == Some("worker")
            && let Some(id) = event.context_id
        {
            let previous_id = self.execution_context_id();
            if self.runtime_execution_context_id.is_some()
                && self.runtime_execution_context_id != Some(id)
            {
                self.mark_all_runtime_bindings_pending_replay();
            }
            self.runtime_execution_context_id = Some(id);
            if previous_id < 0 {
                self.rebind_synthetic_runtime_snapshots(previous_id, id);
            }
        }
    }

    pub(crate) fn record_runtime_execution_context_destroyed_event(
        &mut self,
        event: &RuntimeExecutionContextEvent,
    ) {
        if event.context_id == self.runtime_execution_context_id {
            self.runtime_execution_context_id = None;
            self.mark_all_runtime_bindings_pending_replay();
        }
    }

    pub(crate) fn record_runtime_execution_contexts_cleared_event(&mut self) {
        self.runtime_execution_context_id = None;
        self.mark_all_runtime_bindings_pending_replay();
    }

    #[cfg(test)]
    pub(crate) fn runtime_bindings(&self, session_id: &str) -> &[RuntimeBindingDefinition] {
        self.session_state(session_id)
            .map(|state| state.runtime_bindings.as_slice())
            .unwrap_or(&[])
    }

    pub(crate) fn upsert_runtime_binding_definition(
        &mut self,
        session_id: &str,
        name: String,
        execution_context_name: Option<String>,
    ) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.upsert_runtime_binding_definition(name.clone(), execution_context_name.clone());
        state
            .runtime_binding_replay_pending
            .insert((name, execution_context_name));
    }

    pub(crate) fn upsert_live_runtime_binding_definition(
        &mut self,
        session_id: &str,
        name: String,
        execution_context_name: Option<String>,
    ) {
        self.upsert_runtime_binding_definition(
            session_id,
            name.clone(),
            execution_context_name.clone(),
        );
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state
            .runtime_binding_replay_pending
            .remove(&(name, execution_context_name));
    }

    pub(crate) fn remove_live_runtime_binding_definitions(&mut self, session_id: &str, name: &str) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.remove_runtime_binding_definitions(name);
    }

    pub(crate) fn clear_runtime_binding_definitions(&mut self, session_id: &str) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.clear_runtime_binding_definitions();
    }

    pub(crate) fn runtime_bindings_requiring_replay(
        &self,
        session_id: &str,
    ) -> Vec<RuntimeBindingDefinition> {
        if self.runtime_execution_context_id.is_none() {
            return Vec::new();
        }
        let Some(state) = self.session_state(session_id) else {
            return Vec::new();
        };
        state
            .runtime_bindings
            .iter()
            .filter(|binding| {
                state
                    .runtime_binding_replay_pending
                    .contains(&(binding.name.clone(), binding.execution_context_name.clone()))
            })
            .cloned()
            .collect()
    }

    pub(crate) fn mark_runtime_binding_replayed(
        &mut self,
        session_id: &str,
        binding: &RuntimeBindingDefinition,
    ) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state
            .runtime_binding_replay_pending
            .remove(&(binding.name.clone(), binding.execution_context_name.clone()));
    }

    fn mark_all_runtime_bindings_pending_replay(&mut self) {
        for session in self.sessions.values_mut() {
            session.devtools.runtime_binding_replay_pending.extend(
                session
                    .devtools
                    .runtime_bindings
                    .iter()
                    .map(|binding| (binding.name.clone(), binding.execution_context_name.clone())),
            );
        }
    }

    pub(crate) fn register_runtime_remote_object_ids_for_session<I>(
        &mut self,
        session_id: &str,
        object_ids: I,
    ) where
        I: IntoIterator<Item = String>,
    {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.register_runtime_remote_object_ids(object_ids);
    }

    pub(crate) fn register_runtime_remote_object_ids_with_realm<I>(
        &mut self,
        session_id: &str,
        object_ids: I,
        realm_id: &str,
    ) where
        I: IntoIterator<Item = String>,
    {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.register_runtime_remote_object_ids_with_realm(object_ids, realm_id);
    }

    pub(crate) fn register_runtime_remote_object_ids_with_group<I>(
        &mut self,
        session_id: &str,
        object_ids: I,
        object_group: &str,
    ) where
        I: IntoIterator<Item = String>,
    {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.register_runtime_remote_object_ids_with_group(object_ids, object_group);
    }

    pub(crate) fn register_runtime_remote_object_alias_with_realm(
        &mut self,
        session_id: &str,
        alias_id: String,
        object_id: String,
        realm_id: &str,
    ) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.register_runtime_remote_object_alias_with_realm(alias_id, object_id, realm_id);
    }

    pub(crate) fn unregister_runtime_remote_object_ids(
        &mut self,
        session_id: &str,
        object_ids: &[String],
    ) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.unregister_runtime_remote_object_ids(object_ids);
    }

    pub(crate) fn unregister_runtime_remote_object_group(
        &mut self,
        session_id: &str,
        object_group: &str,
    ) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.unregister_runtime_remote_object_group(object_group);
    }

    pub(crate) fn clear_runtime_remote_object_tracking(&mut self, session_id: &str) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.clear_runtime_remote_object_tracking();
    }

    pub(crate) fn record_runtime_contexts_reported_to_frontend(&mut self, session_id: &str) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.record_runtime_contexts_reported_to_frontend();
    }

    pub(crate) fn record_runtime_contexts_cleared_for_frontend(&mut self, session_id: &str) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.record_runtime_contexts_cleared_for_frontend();
    }

    pub(crate) fn clear_runtime_remote_objects_for_realm(
        &mut self,
        session_id: &str,
        realm_id: &str,
    ) {
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.clear_runtime_remote_objects_for_realm(realm_id);
    }

    pub(crate) fn runtime_remote_object_group(
        &self,
        session_id: &str,
        object_id: &str,
    ) -> Option<&str> {
        self.session_state(session_id)?
            .runtime_remote_object_group(object_id)
    }

    pub(crate) fn has_runtime_remote_object_id(&self, session_id: &str, object_id: &str) -> bool {
        self.session_state(session_id)
            .is_some_and(|state| state.has_runtime_remote_object_id(object_id))
    }

    pub(crate) fn any_session_has_runtime_remote_object_id(&self, object_id: &str) -> bool {
        self.sessions
            .values()
            .any(|session| session.devtools.has_runtime_remote_object_id(object_id))
    }

    pub(crate) fn runtime_remote_object_realm(
        &self,
        session_id: &str,
        object_id: &str,
    ) -> Option<&str> {
        self.session_state(session_id)?
            .runtime_remote_object_realm(object_id)
    }

    pub(crate) fn runtime_remote_object_alias(
        &self,
        session_id: &str,
        object_id: &str,
    ) -> Option<&str> {
        self.session_state(session_id)?
            .runtime_remote_object_alias(object_id)
    }

    pub(crate) fn take_runtime_remote_object_cleanup_plan(
        &mut self,
        session_id: &str,
    ) -> (Vec<String>, Vec<String>) {
        let Some(state) = self.session_state_mut(session_id) else {
            return (Vec::new(), Vec::new());
        };
        state.take_runtime_remote_object_cleanup_plan()
    }

    #[cfg(test)]
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.sessions.keys().next().map(String::as_str)
    }

    pub(crate) fn session_ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    pub(crate) fn has_session(&self) -> bool {
        !self.sessions.is_empty()
    }

    pub(crate) fn is_session(&self, session_id: &str) -> bool {
        self.sessions.contains_key(session_id)
    }

    pub(crate) fn attach_session(&mut self, session_id: String) {
        self.sessions.entry(session_id).or_default();
    }

    pub(crate) fn detach_session(&mut self, session_id: &str) -> Option<String> {
        self.sessions
            .remove(session_id)
            .map(|_| session_id.to_owned())
    }

    pub(crate) fn protocol_attachment_identity(
        &self,
        browser_context_id: &str,
        session_id: &str,
    ) -> Option<TargetSharedWorkerProtocolAttachmentIdentity> {
        let session = self.sessions.get(session_id)?;
        Some(session.attachment_scope.bind(
            browser_context_id,
            self.renderer_owner_local_host_id,
            self.renderer_instance_id,
            self.owner_target_id.clone(),
            self.target_id.clone(),
            session_id,
        ))
    }

    pub(crate) fn owner_target_id(&self) -> Option<&str> {
        self.owner_target_id.as_deref()
    }

    pub(crate) fn take_protocol_attachment_retirement(
        &mut self,
        browser_context_id: &str,
        session_id: &str,
    ) -> Option<TargetSharedWorkerProtocolAttachmentRetirement> {
        let identity = self.protocol_attachment_identity(browser_context_id, session_id)?;
        let session = self.sessions.remove(session_id)?;
        Some(session.attachment_scope.into_retirement(identity))
    }

    #[cfg(test)]
    pub(crate) fn inspector_session_state(
        &self,
        session_id: &str,
    ) -> Option<&InspectorSessionState> {
        self.session_state(session_id)
            .map(|state| &state.inspector_session_state)
    }

    pub(crate) fn set_runtime_frontend_enabled(&mut self, session_id: &str, enabled: bool) {
        let console_len = self.console_messages.len();
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.runtime_session_state.runtime_frontend_enabled = enabled;
        if !enabled {
            state
                .runtime_session_state
                .runtime_contexts_reported_to_frontend = false;
        }
        if !enabled {
            state.console_output_session_state.runtime_console_entries = console_len;
        }
    }

    pub(crate) fn runtime_frontend_enabled(&self, session_id: &str) -> bool {
        self.session_state(session_id)
            .is_some_and(|state| state.runtime_session_state.runtime_frontend_enabled)
    }

    pub(crate) fn set_network_enabled(&mut self, session_id: &str, enabled: bool) -> bool {
        let Some(state) = self.session_state_mut(session_id) else {
            return false;
        };
        if enabled {
            state.network_session_state.network_enabled = true;
        } else {
            state.network_session_state = Default::default();
        }
        true
    }

    pub(crate) fn network_enabled(&self, session_id: &str) -> bool {
        self.session_state(session_id)
            .is_some_and(|state| state.network_session_state.network_enabled)
    }

    pub(crate) fn set_console_enabled(&mut self, session_id: &str, enabled: bool) {
        let console_len = self.console_messages.len();
        let Some(state) = self.session_state_mut(session_id) else {
            return;
        };
        state.console_output_session_state.console_enabled = enabled;
        state.console_output_session_state.console_domain_entries = console_len;
    }

    pub(crate) fn clear_console_messages(&mut self, session_id: &str) {
        let console_len = self.console_messages.len();
        if let Some(state) = self.session_state_mut(session_id) {
            state.console_output_session_state.console_domain_entries = console_len;
        }
    }

    pub(crate) fn discard_runtime_console_entries(&mut self, session_id: &str) {
        let console_len = self.console_messages.len();
        if let Some(state) = self.session_state_mut(session_id) {
            state.console_output_session_state.runtime_console_entries = console_len;
        }
    }

    pub(crate) fn record_console_message(&mut self, message: RendererSharedWorkerConsoleMessage) {
        self.console_messages.push(RuntimeConsoleMessageSnapshot {
            execution_context_id: self.execution_context_id(),
            message: message.message,
            args: message.args,
            stack: message.stack,
        });
    }

    pub(crate) fn pending_console_domain_messages(
        &self,
        session_id: &str,
    ) -> &[RuntimeConsoleMessageSnapshot] {
        let Some(state) = self.session_state(session_id) else {
            return &[];
        };
        if !state.console_output_session_state.console_enabled {
            return &[];
        }
        &self.console_messages[state
            .console_output_session_state
            .console_domain_entries
            .min(self.console_messages.len())..]
    }

    pub(crate) fn pending_runtime_console_messages(
        &self,
        session_id: &str,
    ) -> &[RuntimeConsoleMessageSnapshot] {
        let Some(state) = self.session_state(session_id) else {
            return &[];
        };
        if !state.runtime_session_state.runtime_frontend_enabled {
            return &[];
        }
        if self.runtime_execution_context_id.is_none() {
            return &[];
        }
        &self.console_messages[state
            .console_output_session_state
            .runtime_console_entries
            .min(self.console_messages.len())..]
    }

    pub(crate) fn mark_console_domain_emitted(&mut self, session_id: &str, console_end: usize) {
        let console_len = self.console_messages.len();
        if let Some(state) = self.session_state_mut(session_id) {
            state.console_output_session_state.console_domain_entries =
                console_end.min(console_len);
        }
    }

    pub(crate) fn mark_runtime_console_emitted(&mut self, session_id: &str, console_end: usize) {
        let console_len = self.console_messages.len();
        if let Some(state) = self.session_state_mut(session_id) {
            state.console_output_session_state.runtime_console_entries =
                console_end.min(console_len);
        }
    }

    pub(crate) fn console_message_count(&self) -> usize {
        self.console_messages.len()
    }

    #[cfg(test)]
    pub(crate) fn register_pending_inspector_await(
        &mut self,
        owner_session_id: &str,
        cdp_request_id: u64,
        session_id: Option<&str>,
        object_group: Option<&str>,
    ) {
        self.try_register_pending_inspector_await(
            owner_session_id,
            cdp_request_id,
            session_id,
            object_group,
        )
        .expect("pending Inspector await frontend command id must be unique per session");
    }

    pub(crate) fn try_register_pending_inspector_await(
        &mut self,
        owner_session_id: &str,
        cdp_request_id: u64,
        session_id: Option<&str>,
        object_group: Option<&str>,
    ) -> Result<(), DuplicatePendingRendererCommand> {
        let Some(state) = self.session_state_mut(owner_session_id) else {
            return Ok(());
        };
        state
            .pending_inspector_awaits
            .try_insert(cdp_request_id, session_id, object_group)
    }

    pub(crate) fn try_register_renderer_call(
        &mut self,
        owner_session_id: &str,
        cdp_request_id: u64,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
        descriptor: RendererCommandDescriptor,
    ) -> Option<Result<PreparedRendererCallDispatch, RegisterRendererCallError>> {
        Some(
            self.session_state_mut(owner_session_id)?
                .try_register_renderer_call(cdp_request_id, dispatched_attachment_id, descriptor),
        )
    }

    pub(crate) fn take_renderer_call_for_frontend(
        &mut self,
        owner_session_id: &str,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        self.session_state_mut(owner_session_id)?
            .take_renderer_call_for_frontend(cdp_request_id)
    }

    pub(crate) fn renderer_call_for_frontend(
        &self,
        owner_session_id: &str,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        self.session_state(owner_session_id)?
            .renderer_call_for_frontend(cdp_request_id)
    }

    pub(crate) fn take_renderer_call_for_frontend_if_matches(
        &mut self,
        owner_session_id: &str,
        cdp_request_id: u64,
        renderer_call_id: moli_page_types::RendererCallId,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        self.session_state_mut(owner_session_id)?
            .take_renderer_call_for_frontend_if_matches(
                cdp_request_id,
                renderer_call_id,
                dispatched_attachment_id,
            )
    }

    pub(crate) fn take_frontend_command_for_renderer_if_attachment_matches(
        &mut self,
        owner_session_id: &str,
        renderer_call_id: moli_page_types::RendererCallId,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        self.session_state_mut(owner_session_id)?
            .take_frontend_command_for_renderer_if_attachment_matches(
                renderer_call_id,
                dispatched_attachment_id,
            )
    }

    pub(crate) fn renderer_command_descriptor_for_renderer_if_attachment_matches(
        &self,
        owner_session_id: &str,
        renderer_call_id: moli_page_types::RendererCallId,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
    ) -> Option<RendererCommandDescriptor> {
        self.session_state(owner_session_id)?
            .renderer_command_descriptor_for_renderer_if_attachment_matches(
                renderer_call_id,
                dispatched_attachment_id,
            )
    }

    pub(crate) fn remove_pending_inspector_await(
        &mut self,
        owner_session_id: &str,
        cdp_request_id: u64,
    ) -> Option<PendingInspectorAwait> {
        self.session_state_mut(owner_session_id)?
            .pending_inspector_awaits
            .remove(cdp_request_id)
    }

    pub(crate) fn has_pending_inspector_awaits(&self) -> bool {
        self.sessions
            .values()
            .any(|session| !session.devtools.pending_inspector_awaits.is_empty())
    }

    pub(crate) fn has_pending_inspector_awaits_for_session(&self, session_id: &str) -> bool {
        self.session_state(session_id)
            .is_some_and(|state| !state.pending_inspector_awaits.is_empty())
    }

    pub(crate) fn pending_inspector_await_count_all_sessions(&self) -> usize {
        self.sessions
            .values()
            .map(|session| session.devtools.pending_inspector_awaits.len())
            .sum()
    }

    pub(crate) fn drain_pending_inspector_awaits_for_session(
        &mut self,
        session_id: &str,
    ) -> Vec<(u64, PendingInspectorAwait)> {
        self.session_state_mut(session_id)
            .map(|state| state.pending_inspector_awaits.drain_all())
            .unwrap_or_default()
    }

    pub(crate) fn terminate_renderer_calls_for_session(
        &mut self,
        session_id: &str,
        reason: &str,
    ) -> Vec<RendererCommandCorrelation> {
        self.session_state_mut(session_id)
            .map(|state| state.terminate_all_renderer_calls(reason))
            .unwrap_or_default()
    }

    fn session_state(&self, session_id: &str) -> Option<&DevToolsSessionState> {
        self.sessions.get(session_id).map(|state| &state.devtools)
    }

    fn session_state_mut(&mut self, session_id: &str) -> Option<&mut DevToolsSessionState> {
        self.sessions
            .get_mut(session_id)
            .map(|state| &mut state.devtools)
    }
}

#[cfg(test)]
mod tests {
    use super::SharedWorkerTargetState;
    use crate::devtools_runtime::RuntimeExecutionContextEvent;
    use moli_core::{RendererOwnerLocalHostId, page::RendererSharedWorkerConsoleMessage};
    use moli_shared_worker::SharedWorkerInstanceId;

    fn shared_worker_target() -> SharedWorkerTargetState {
        SharedWorkerTargetState::new(
            RendererOwnerLocalHostId::new_for_testing(1),
            SharedWorkerInstanceId::from_u64(91),
            "TID-shared-worker".to_owned(),
            None,
            "https://example.test/shared-worker.js".to_owned(),
            "shared-worker".to_owned(),
        )
    }

    fn worker_context_created_event(context_id: i64) -> RuntimeExecutionContextEvent {
        RuntimeExecutionContextEvent {
            target_id: None,
            context_id: Some(context_id),
            realm_id: None,
            frame_id: None,
            origin: None,
            name: None,
            is_default: None,
            context_type: Some("worker".to_owned()),
            grant_universal_access: None,
        }
    }

    #[test]
    fn shared_worker_target_detach_clears_only_that_session_state() {
        let mut target = shared_worker_target();
        target.attach_session("SID-shared-worker".to_owned());
        target.attach_session("SID-peer".to_owned());
        assert!(
            target
                .inspector_session_state("SID-shared-worker")
                .is_some()
        );
        assert!(target.inspector_session_state("SID-peer").is_some());

        assert_eq!(
            target.detach_session("SID-shared-worker").as_deref(),
            Some("SID-shared-worker")
        );
        assert!(
            target
                .inspector_session_state("SID-shared-worker")
                .is_none()
        );
        assert!(target.inspector_session_state("SID-peer").is_some());
    }

    #[test]
    fn shared_worker_target_runtime_bindings_are_session_local() {
        let mut target = shared_worker_target();
        target.attach_session("SID-a".to_owned());
        target.attach_session("SID-b".to_owned());

        target.upsert_runtime_binding_definition(
            "SID-a",
            "bindingForA".to_owned(),
            Some("world-a".to_owned()),
        );

        assert_eq!(target.runtime_bindings("SID-a").len(), 1);
        assert!(target.runtime_bindings("SID-b").is_empty());
        assert_eq!(target.runtime_bindings("SID-a")[0].name, "bindingForA");
    }

    #[test]
    fn real_runtime_context_rebinds_synthetic_console_snapshots() {
        let mut target = shared_worker_target();
        target.attach_session("SID-shared-worker".to_owned());
        target.set_runtime_frontend_enabled("SID-shared-worker", true);
        target.record_console_message(RendererSharedWorkerConsoleMessage {
            message: "log: before context".to_owned(),
            args: Vec::new(),
            stack: None,
        });

        assert!(
            target
                .pending_runtime_console_messages("SID-shared-worker")
                .is_empty(),
            "Runtime console messages must wait for a real renderer context id"
        );

        target.record_runtime_execution_context_created_event(&worker_context_created_event(2901));

        assert_eq!(
            target.pending_runtime_console_messages("SID-shared-worker")[0].execution_context_id,
            2901
        );
    }
}
