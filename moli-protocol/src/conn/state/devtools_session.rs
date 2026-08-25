use std::collections::{BTreeSet, HashMap, HashSet};

use super::{
    page_slot::RuntimeBindingDefinition,
    parking::{PendingInspectorAwait, TargetPendingInspectorAwaitRegistry},
    pending_renderer_command::{
        DuplicatePendingRendererCommand, PreparedRendererCallDispatch, PreparedRendererCallReplay,
        PreparedRendererCallTermination, RegisterRendererCallError, RendererCallIdExhausted,
        RendererCommandCorrelation, RendererCommandDescriptor,
    },
    session::{InspectorSessionState, TargetPageSessionState, TargetRuntimeSessionState},
};
use moli_core::network::WebStorageMutationSubscription;
use moli_page_types::{
    RendererDomDebuggerEventListenerBreakpoint, RendererDomDebuggerXhrBreakpoint,
};

/// CDP session-owned state for DevTools domains that are backed by a renderer
/// inspector session.
///
/// This is the protocol-side state object we can share across page and worker
/// targets while the renderer-side V8 inspector backend is still target-shaped.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DevToolsSessionState {
    pub(crate) dom_session_state: DevToolsDomSessionState,
    pub(crate) dom_debugger_event_listener_breakpoints:
        BTreeSet<RendererDomDebuggerEventListenerBreakpoint>,
    pub(crate) dom_debugger_xhr_breakpoints: BTreeSet<RendererDomDebuggerXhrBreakpoint>,
    pub(crate) page_session_state: TargetPageSessionState,
    pub(crate) runtime_session_state: TargetRuntimeSessionState,
    pub(crate) console_output_session_state: DevToolsConsoleOutputSessionState,
    pub(crate) dom_storage_session_state: DevToolsDomStorageSessionState,
    pub(crate) network_output_session_state: DevToolsNetworkOutputSessionState,
    pub(crate) runtime_bindings: Vec<RuntimeBindingDefinition>,
    pub(crate) runtime_binding_replay_pending: BTreeSet<(String, Option<String>)>,
    pub(crate) runtime_remote_object_ids: HashSet<String>,
    pub(crate) runtime_remote_object_groups: HashMap<String, String>,
    pub(crate) runtime_remote_object_realms: HashMap<String, String>,
    pub(crate) runtime_remote_object_aliases: HashMap<String, String>,
    pub(crate) emitted_child_default_execution_context_ids: HashSet<i64>,
    pub(crate) inspector_session_state: InspectorSessionState,
    pub(crate) pending_inspector_awaits: TargetPendingInspectorAwaitRegistry,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DevToolsDomSessionState {
    pub(crate) enabled: bool,
    pub(crate) include_whitespace: bool,
}

#[derive(Debug)]
pub(crate) struct SessionRendererCallReplay {
    frontend_session_id: Option<String>,
    renderer_inspector_session_id: Option<String>,
    replay: PreparedRendererCallReplay,
}

#[derive(Debug)]
pub(crate) struct SessionRendererCallTermination {
    frontend_session_id: Option<String>,
    termination: PreparedRendererCallTermination,
}

#[derive(Debug, Default)]
pub(crate) struct PreparedRendererCallReplacements {
    new_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
    terminations: Vec<SessionRendererCallTermination>,
    replays: Vec<SessionRendererCallReplay>,
}

impl PreparedRendererCallReplacements {
    fn new(
        new_attachment_id: moli_page_types::RendererAgentAttachmentId,
        terminations: Vec<SessionRendererCallTermination>,
        replays: Vec<SessionRendererCallReplay>,
    ) -> Self {
        Self {
            new_attachment_id: Some(new_attachment_id),
            terminations,
            replays,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.terminations.is_empty() && self.replays.is_empty()
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        moli_page_types::RendererAgentAttachmentId,
        Vec<SessionRendererCallTermination>,
        Vec<SessionRendererCallReplay>,
    ) {
        (
            self.new_attachment_id
                .expect("prepared renderer replacements must have an attachment"),
            self.terminations,
            self.replays,
        )
    }
}

impl SessionRendererCallTermination {
    pub(crate) fn into_parts(self) -> (Option<String>, PreparedRendererCallTermination) {
        (self.frontend_session_id, self.termination)
    }
}

impl SessionRendererCallReplay {
    pub(crate) fn frontend_session_id(&self) -> Option<&str> {
        self.frontend_session_id.as_deref()
    }

    pub(crate) fn renderer_inspector_session_id(&self) -> Option<&str> {
        self.renderer_inspector_session_id.as_deref()
    }

    pub(crate) fn into_replay(self) -> PreparedRendererCallReplay {
        self.replay
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DevToolsDomStorageSessionState {
    mutation_subscription: Option<WebStorageMutationSubscription>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DevToolsNetworkOutputSessionState {
    pub(crate) network_enabled: bool,
    pub(crate) service_worker_fetch_diagnostic_entries: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DevToolsConsoleOutputSessionState {
    pub(crate) console_enabled: bool,
    pub(crate) console_domain_entries: usize,
    pub(crate) log_output_generation: u64,
    pub(crate) log_lifecycle_entries: usize,
    pub(crate) log_network_entries: usize,
    pub(crate) log_violation_thresholds: Vec<DevToolsLogViolationThreshold>,
    pub(crate) runtime_console_entries: usize,
    pub(crate) runtime_exception_entries: usize,
    pub(crate) renderer_console_agent_owns_page_console_api_events: bool,
    pub(crate) renderer_runtime_agent_owns_page_console_api_events: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DevToolsLogViolationThreshold {
    pub(crate) name: String,
    pub(crate) threshold: f64,
}

impl DevToolsSessionState {
    pub(crate) fn upsert_runtime_binding_definition(
        &mut self,
        name: String,
        execution_context_name: Option<String>,
    ) {
        if !self.runtime_bindings.iter().any(|binding| {
            binding.name == name && binding.execution_context_name == execution_context_name
        }) {
            self.runtime_bindings.push(RuntimeBindingDefinition {
                name,
                execution_context_name,
            });
        }
    }

    pub(crate) fn remove_runtime_binding_definitions(&mut self, name: &str) {
        self.runtime_bindings.retain(|binding| binding.name != name);
        self.runtime_binding_replay_pending
            .retain(|(binding_name, _)| binding_name != name);
    }

    pub(crate) fn clear_runtime_binding_definitions(&mut self) {
        self.runtime_bindings.clear();
        self.runtime_binding_replay_pending.clear();
    }

    #[cfg(test)]
    pub(crate) fn register_pending_inspector_await(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
        object_group: Option<&str>,
    ) {
        self.try_register_pending_inspector_await(cdp_request_id, session_id, object_group)
            .expect("pending Inspector await frontend command id must be unique per session");
    }

    pub(crate) fn try_register_pending_inspector_await(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
        object_group: Option<&str>,
    ) -> Result<(), DuplicatePendingRendererCommand> {
        self.pending_inspector_awaits
            .try_insert(cdp_request_id, session_id, object_group)
    }

    pub(crate) fn register_pending_bidi_channel_listener(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
        object_group: Option<&str>,
        listener: crate::conn::BidiChannelListenerResidence,
    ) {
        self.pending_inspector_awaits.insert_bidi_channel_listener(
            cdp_request_id,
            session_id,
            object_group,
            listener,
        );
    }

    pub(crate) fn try_register_renderer_call(
        &mut self,
        cdp_request_id: u64,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
        descriptor: RendererCommandDescriptor,
    ) -> Result<PreparedRendererCallDispatch, RegisterRendererCallError> {
        self.pending_inspector_awaits.try_register_renderer_call(
            cdp_request_id,
            dispatched_attachment_id,
            descriptor,
        )
    }

    pub(crate) fn take_renderer_call_for_frontend(
        &mut self,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        self.pending_inspector_awaits
            .take_renderer_call_for_frontend(cdp_request_id)
    }

    pub(crate) fn renderer_call_for_frontend(
        &self,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        self.pending_inspector_awaits
            .renderer_call_for_frontend(cdp_request_id)
    }

    pub(crate) fn renderer_command_descriptor_for_renderer_if_attachment_matches(
        &self,
        renderer_call_id: moli_page_types::RendererCallId,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
    ) -> Option<RendererCommandDescriptor> {
        self.pending_inspector_awaits
            .renderer_command_descriptor_for_renderer_if_attachment_matches(
                renderer_call_id,
                dispatched_attachment_id,
            )
    }

    pub(crate) fn prepare_renderer_call_replays(
        &mut self,
        old_attachment_id: moli_page_types::RendererAgentAttachmentId,
        new_attachment_id: moli_page_types::RendererAgentAttachmentId,
    ) -> Result<Vec<PreparedRendererCallReplay>, RendererCallIdExhausted> {
        self.pending_inspector_awaits
            .prepare_renderer_call_replays(old_attachment_id, new_attachment_id)
    }

    pub(crate) fn prepare_renderer_call_terminations(
        &mut self,
        old_attachment_id: moli_page_types::RendererAgentAttachmentId,
        terminal_attachment_id: moli_page_types::RendererAgentAttachmentId,
    ) -> Result<Vec<PreparedRendererCallTermination>, RendererCallIdExhausted> {
        self.pending_inspector_awaits
            .prepare_renderer_call_terminations(old_attachment_id, terminal_attachment_id)
    }

    pub(crate) fn terminate_all_renderer_calls(
        &mut self,
        reason: &str,
    ) -> Vec<RendererCommandCorrelation> {
        self.pending_inspector_awaits
            .terminate_all_renderer_calls(reason)
    }

    pub(crate) fn take_renderer_call_for_frontend_if_matches(
        &mut self,
        cdp_request_id: u64,
        renderer_call_id: moli_page_types::RendererCallId,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        self.pending_inspector_awaits
            .take_renderer_call_for_frontend_if_matches(
                cdp_request_id,
                renderer_call_id,
                dispatched_attachment_id,
            )
    }

    pub(crate) fn take_frontend_command_for_renderer_if_attachment_matches(
        &mut self,
        renderer_call_id: moli_page_types::RendererCallId,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        self.pending_inspector_awaits
            .take_frontend_command_for_renderer_if_attachment_matches(
                renderer_call_id,
                dispatched_attachment_id,
            )
    }

    pub(crate) fn remove_pending_inspector_await(
        &mut self,
        cdp_request_id: u64,
    ) -> Option<PendingInspectorAwait> {
        self.pending_inspector_awaits.remove(cdp_request_id)
    }

    pub(crate) fn has_pending_inspector_awaits(&self) -> bool {
        !self.pending_inspector_awaits.is_empty()
    }

    pub(crate) fn pending_inspector_await_count(&self) -> usize {
        self.pending_inspector_awaits.len()
    }

    pub(crate) fn drain_pending_inspector_awaits(&mut self) -> Vec<(u64, PendingInspectorAwait)> {
        self.pending_inspector_awaits.drain_all()
    }

    pub(crate) fn drain_pending_inspector_awaits_for_sessions(
        &mut self,
        session_ids: &[&str],
    ) -> Vec<(u64, PendingInspectorAwait)> {
        self.pending_inspector_awaits
            .drain_for_sessions(session_ids)
    }

    pub(crate) fn register_runtime_remote_object_ids<I>(&mut self, object_ids: I)
    where
        I: IntoIterator<Item = String>,
    {
        self.runtime_remote_object_ids.extend(object_ids);
    }

    pub(crate) fn register_runtime_remote_object_ids_with_realm<I>(
        &mut self,
        object_ids: I,
        realm_id: &str,
    ) where
        I: IntoIterator<Item = String>,
    {
        for object_id in object_ids {
            self.runtime_remote_object_ids.insert(object_id.clone());
            self.runtime_remote_object_realms
                .insert(object_id, realm_id.to_owned());
        }
    }

    pub(crate) fn register_runtime_remote_object_alias_with_realm(
        &mut self,
        alias_id: String,
        object_id: String,
        realm_id: &str,
    ) {
        self.runtime_remote_object_ids.insert(object_id.clone());
        self.runtime_remote_object_realms
            .insert(object_id.clone(), realm_id.to_owned());
        self.runtime_remote_object_aliases
            .insert(alias_id.clone(), object_id);
        self.runtime_remote_object_realms
            .insert(alias_id, realm_id.to_owned());
    }

    pub(crate) fn register_runtime_remote_object_ids_with_group<I>(
        &mut self,
        object_ids: I,
        object_group: &str,
    ) where
        I: IntoIterator<Item = String>,
    {
        for object_id in object_ids {
            self.runtime_remote_object_ids.insert(object_id.clone());
            self.runtime_remote_object_groups
                .insert(object_id, object_group.to_owned());
        }
    }

    pub(crate) fn unregister_runtime_remote_object_ids(&mut self, object_ids: &[String]) {
        let object_ids = object_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let alias_ids_to_remove = self
            .runtime_remote_object_aliases
            .iter()
            .filter_map(|(alias_id, object_id)| {
                (object_ids.contains(alias_id.as_str()) || object_ids.contains(object_id.as_str()))
                    .then_some(alias_id.clone())
            })
            .collect::<Vec<_>>();

        for object_id in object_ids {
            self.runtime_remote_object_ids.remove(object_id);
            self.runtime_remote_object_groups.remove(object_id);
            self.runtime_remote_object_realms.remove(object_id);
        }

        for alias_id in alias_ids_to_remove {
            self.runtime_remote_object_aliases.remove(&alias_id);
            self.runtime_remote_object_realms.remove(&alias_id);
        }
    }

    pub(crate) fn unregister_runtime_remote_object_group(&mut self, object_group: &str) {
        let object_ids = self
            .runtime_remote_object_groups
            .iter()
            .filter_map(|(object_id, group)| {
                (group == object_group).then_some(object_id.to_owned())
            })
            .collect::<Vec<_>>();
        self.unregister_runtime_remote_object_ids(&object_ids);
    }

    pub(crate) fn clear_runtime_remote_object_tracking(&mut self) {
        self.runtime_remote_object_ids.clear();
        self.runtime_remote_object_groups.clear();
        self.runtime_remote_object_realms.clear();
        self.runtime_remote_object_aliases.clear();
        self.clear_child_default_context_emission_state();
    }

    pub(crate) fn record_runtime_contexts_reported_to_frontend(&mut self) {
        self.runtime_session_state
            .runtime_contexts_reported_to_frontend = true;
    }

    pub(crate) fn record_runtime_contexts_cleared_for_frontend(&mut self) {
        self.runtime_session_state
            .runtime_contexts_reported_to_frontend = false;
    }

    pub(crate) fn has_emitted_child_default_execution_context_id(
        &self,
        execution_context_id: i64,
    ) -> bool {
        self.emitted_child_default_execution_context_ids
            .contains(&execution_context_id)
    }

    pub(crate) fn mark_child_default_execution_context_id_emitted(
        &mut self,
        execution_context_id: i64,
    ) -> bool {
        self.emitted_child_default_execution_context_ids
            .insert(execution_context_id)
    }

    pub(crate) fn clear_child_default_context_emission_state(&mut self) {
        self.emitted_child_default_execution_context_ids.clear();
    }

    pub(crate) fn clear_runtime_remote_objects_for_realm(&mut self, realm_id: &str) {
        let object_ids = self
            .runtime_remote_object_realms
            .iter()
            .filter_map(|(object_id, realm)| (realm == realm_id).then_some(object_id.clone()))
            .collect::<Vec<_>>();
        self.unregister_runtime_remote_object_ids(&object_ids);
    }

    pub(crate) fn runtime_remote_object_group(&self, object_id: &str) -> Option<&str> {
        self.runtime_remote_object_groups
            .get(object_id)
            .or_else(|| {
                self.runtime_remote_object_aliases
                    .get(object_id)
                    .and_then(|object_id| self.runtime_remote_object_groups.get(object_id))
            })
            .map(String::as_str)
    }

    pub(crate) fn has_runtime_remote_object_id(&self, object_id: &str) -> bool {
        self.runtime_remote_object_ids.contains(object_id)
            || self.runtime_remote_object_aliases.contains_key(object_id)
    }

    pub(crate) fn runtime_remote_object_realm(&self, object_id: &str) -> Option<&str> {
        self.runtime_remote_object_realms
            .get(object_id)
            .or_else(|| {
                self.runtime_remote_object_aliases
                    .get(object_id)
                    .and_then(|object_id| self.runtime_remote_object_realms.get(object_id))
            })
            .map(String::as_str)
    }

    pub(crate) fn runtime_remote_object_alias(&self, object_id: &str) -> Option<&str> {
        self.runtime_remote_object_aliases
            .get(object_id)
            .map(String::as_str)
    }

    pub(crate) fn take_runtime_remote_object_cleanup_plan(&mut self) -> (Vec<String>, Vec<String>) {
        let grouped_object_ids = self
            .runtime_remote_object_groups
            .keys()
            .cloned()
            .collect::<HashSet<_>>();
        let mut object_groups = self
            .runtime_remote_object_groups
            .values()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut ungrouped_object_ids = self
            .runtime_remote_object_ids
            .iter()
            .filter(|object_id| !grouped_object_ids.contains(*object_id))
            .cloned()
            .collect::<Vec<_>>();
        object_groups.sort();
        ungrouped_object_ids.sort();
        self.clear_runtime_remote_object_tracking();
        (object_groups, ungrouped_object_ids)
    }
}

impl DevToolsDomStorageSessionState {
    pub(crate) fn is_enabled(&self) -> bool {
        self.mutation_subscription.is_some()
    }

    pub(crate) fn enable(&mut self, subscription: WebStorageMutationSubscription) {
        if self.mutation_subscription.is_none() {
            self.mutation_subscription = Some(subscription);
        }
    }

    pub(crate) fn disable(&mut self) {
        self.mutation_subscription = None;
    }

    pub(crate) fn mutation_subscription(&self) -> Option<&WebStorageMutationSubscription> {
        self.mutation_subscription.as_ref()
    }
}

pub(crate) fn devtools_sessions_have_pending_inspector_awaits(
    primary: &DevToolsSessionState,
    auxiliary: &HashMap<String, DevToolsSessionState>,
) -> bool {
    primary.has_pending_inspector_awaits()
        || auxiliary
            .values()
            .any(DevToolsSessionState::has_pending_inspector_awaits)
}

pub(crate) fn devtools_sessions_pending_inspector_await_count(
    primary: &DevToolsSessionState,
    auxiliary: &HashMap<String, DevToolsSessionState>,
) -> usize {
    primary.pending_inspector_await_count()
        + auxiliary
            .values()
            .map(DevToolsSessionState::pending_inspector_await_count)
            .sum::<usize>()
}

pub(crate) fn drain_pending_inspector_awaits_for_devtools_sessions(
    primary: &mut DevToolsSessionState,
    auxiliary: &mut HashMap<String, DevToolsSessionState>,
    session_ids: &[&str],
) -> Vec<(u64, PendingInspectorAwait)> {
    let mut drained = primary.drain_pending_inspector_awaits_for_sessions(session_ids);
    for state in auxiliary.values_mut() {
        drained.extend(state.drain_pending_inspector_awaits_for_sessions(session_ids));
    }
    drained
}

pub(crate) fn prepare_renderer_call_replays_for_devtools_sessions(
    primary_session_id: Option<&str>,
    primary: &mut DevToolsSessionState,
    auxiliary: &mut HashMap<String, DevToolsSessionState>,
    old_attachment_id: moli_page_types::RendererAgentAttachmentId,
    new_attachment_id: moli_page_types::RendererAgentAttachmentId,
) -> Result<Vec<SessionRendererCallReplay>, RendererCallIdExhausted> {
    let mut replays = primary
        .prepare_renderer_call_replays(old_attachment_id, new_attachment_id)?
        .into_iter()
        .map(|replay| SessionRendererCallReplay {
            frontend_session_id: primary_session_id.map(str::to_owned),
            renderer_inspector_session_id: None,
            replay,
        })
        .collect::<Vec<_>>();
    let mut auxiliary_session_ids = auxiliary.keys().cloned().collect::<Vec<_>>();
    auxiliary_session_ids.sort();
    for session_id in auxiliary_session_ids {
        let state = auxiliary
            .get_mut(&session_id)
            .expect("selected auxiliary session must remain registered");
        replays.extend(
            state
                .prepare_renderer_call_replays(old_attachment_id, new_attachment_id)?
                .into_iter()
                .map(|replay| SessionRendererCallReplay {
                    frontend_session_id: Some(session_id.clone()),
                    renderer_inspector_session_id: Some(session_id.clone()),
                    replay,
                }),
        );
    }
    Ok(replays)
}

pub(crate) fn prepare_renderer_call_terminations_for_devtools_sessions(
    primary_session_id: Option<&str>,
    primary: &mut DevToolsSessionState,
    auxiliary: &mut HashMap<String, DevToolsSessionState>,
    old_attachment_id: moli_page_types::RendererAgentAttachmentId,
    terminal_attachment_id: moli_page_types::RendererAgentAttachmentId,
) -> Result<Vec<SessionRendererCallTermination>, RendererCallIdExhausted> {
    let mut terminations = primary
        .prepare_renderer_call_terminations(old_attachment_id, terminal_attachment_id)?
        .into_iter()
        .map(|termination| SessionRendererCallTermination {
            frontend_session_id: primary_session_id.map(str::to_owned),
            termination,
        })
        .collect::<Vec<_>>();
    let mut auxiliary_session_ids = auxiliary.keys().cloned().collect::<Vec<_>>();
    auxiliary_session_ids.sort();
    for session_id in auxiliary_session_ids {
        let state = auxiliary
            .get_mut(&session_id)
            .expect("selected auxiliary session must remain registered");
        terminations.extend(
            state
                .prepare_renderer_call_terminations(old_attachment_id, terminal_attachment_id)?
                .into_iter()
                .map(|termination| SessionRendererCallTermination {
                    frontend_session_id: Some(session_id.clone()),
                    termination,
                }),
        );
    }
    Ok(terminations)
}

pub(crate) fn prepare_renderer_call_replacements_for_devtools_sessions(
    primary_session_id: Option<&str>,
    primary: &mut DevToolsSessionState,
    auxiliary: &mut HashMap<String, DevToolsSessionState>,
    old_attachment_id: moli_page_types::RendererAgentAttachmentId,
    new_attachment_id: moli_page_types::RendererAgentAttachmentId,
) -> Result<PreparedRendererCallReplacements, RendererCallIdExhausted> {
    let terminations = prepare_renderer_call_terminations_for_devtools_sessions(
        primary_session_id,
        primary,
        auxiliary,
        old_attachment_id,
        new_attachment_id,
    )?;
    let replays = prepare_renderer_call_replays_for_devtools_sessions(
        primary_session_id,
        primary,
        auxiliary,
        old_attachment_id,
        new_attachment_id,
    )?;
    Ok(PreparedRendererCallReplacements::new(
        new_attachment_id,
        terminations,
        replays,
    ))
}

pub(crate) fn runtime_bindings_for_renderer(
    primary: &DevToolsSessionState,
    auxiliary: &HashMap<String, DevToolsSessionState>,
) -> Vec<RuntimeBindingDefinition> {
    fn push_unique(
        bindings: &mut Vec<RuntimeBindingDefinition>,
        binding: &RuntimeBindingDefinition,
    ) {
        if !bindings.iter().any(|existing| existing == binding) {
            bindings.push(binding.clone());
        }
    }

    let mut bindings = Vec::new();
    for binding in &primary.runtime_bindings {
        push_unique(&mut bindings, binding);
    }
    let mut auxiliary = auxiliary.iter().collect::<Vec<_>>();
    auxiliary.sort_by_key(|(session_id, _)| *session_id);
    for (_, state) in auxiliary {
        for binding in &state.runtime_bindings {
            push_unique(&mut bindings, binding);
        }
    }
    bindings
}

pub(crate) fn page_bypass_csp_enabled_for_devtools_sessions(
    primary: &DevToolsSessionState,
    auxiliary: &HashMap<String, DevToolsSessionState>,
) -> bool {
    primary.page_session_state.page_bypass_csp_enabled
        || auxiliary
            .values()
            .any(|state| state.page_session_state.page_bypass_csp_enabled)
}
