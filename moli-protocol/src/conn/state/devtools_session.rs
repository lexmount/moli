use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    ops::{Index, IndexMut},
};

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
use moli_core::{
    network::WebStorageMutationSubscription,
    page::{
        RendererInspectorProtocolConfiguration, RendererInspectorSessionRestoreSnapshot,
        V8InspectorSessionAttach,
    },
};
use moli_page_types::{
    DevToolsSessionKey, RendererDomDebuggerEventListenerBreakpoint,
    RendererDomDebuggerXhrBreakpoint,
};

/// CDP domain-handler state owned by one DevTools session.
///
/// Browser-side policy and renderer-inspector state live together here so
/// attachment, replay, and disposal all address the same session object.
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
    pub(crate) network_session_state: DevToolsNetworkSessionState,
    pub(crate) emulation_session_state: DevToolsEmulationSessionState,
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

/// Every DevTools session attached to one Page target.
///
/// The renderer's implicit root session is represented by `Primary`; flattened
/// target sessions use `Attached(session_id)`. Keeping both in one ordered map
/// gives attachment, disposal, replay, and effective-domain aggregation one
/// source of truth.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DevToolsSessionRegistry {
    states: BTreeMap<DevToolsSessionKey, DevToolsSessionState>,
    attached_order: Vec<String>,
    browser_identity_activation_order: Vec<DevToolsSessionKey>,
}

impl Default for DevToolsSessionRegistry {
    fn default() -> Self {
        Self {
            states: BTreeMap::from([(
                DevToolsSessionKey::Primary,
                DevToolsSessionState::default(),
            )]),
            attached_order: Vec::new(),
            browser_identity_activation_order: Vec::new(),
        }
    }
}

impl DevToolsSessionRegistry {
    pub(crate) fn primary(&self) -> &DevToolsSessionState {
        self.states
            .get(&DevToolsSessionKey::Primary)
            .expect("DevTools session registry must retain its primary session")
    }

    pub(crate) fn primary_mut(&mut self) -> &mut DevToolsSessionState {
        self.states
            .get_mut(&DevToolsSessionKey::Primary)
            .expect("DevTools session registry must retain its primary session")
    }

    pub(crate) fn routed(
        &self,
        is_attached_session: bool,
        session_id: Option<&str>,
    ) -> Option<&DevToolsSessionState> {
        if is_attached_session {
            return self.attached(session_id?);
        }
        Some(self.primary())
    }

    pub(crate) fn routed_mut_or_insert(
        &mut self,
        is_attached_session: bool,
        session_id: Option<&str>,
    ) -> &mut DevToolsSessionState {
        if is_attached_session && let Some(session_id) = session_id {
            return self.ensure_attached(session_id);
        }
        self.primary_mut()
    }

    pub(crate) fn attached(&self, session_id: &str) -> Option<&DevToolsSessionState> {
        self.states
            .get(&DevToolsSessionKey::Attached(session_id.to_owned()))
    }

    pub(crate) fn attached_mut(&mut self, session_id: &str) -> Option<&mut DevToolsSessionState> {
        self.states
            .get_mut(&DevToolsSessionKey::Attached(session_id.to_owned()))
    }

    pub(crate) fn ensure_attached(&mut self, session_id: &str) -> &mut DevToolsSessionState {
        if !self
            .states
            .contains_key(&DevToolsSessionKey::Attached(session_id.to_owned()))
        {
            self.attached_order.push(session_id.to_owned());
        }
        self.states
            .entry(DevToolsSessionKey::Attached(session_id.to_owned()))
            .or_default()
    }

    pub(crate) fn remove_attached(&mut self, session_id: &str) -> Option<DevToolsSessionState> {
        let key = DevToolsSessionKey::Attached(session_id.to_owned());
        let removed = self.states.remove(&key);
        if removed.is_some() {
            self.attached_order
                .retain(|attached| attached != session_id);
            self.browser_identity_activation_order
                .retain(|candidate| candidate != &key);
        }
        removed
    }

    pub(crate) fn clear_attached(&mut self) {
        self.states
            .retain(|key, _state| matches!(key, DevToolsSessionKey::Primary));
        self.attached_order.clear();
        self.browser_identity_activation_order
            .retain(|key| matches!(key, DevToolsSessionKey::Primary));
    }

    pub(crate) fn attached_len(&self) -> usize {
        self.states.len().saturating_sub(1)
    }

    pub(crate) fn attached_is_empty(&self) -> bool {
        self.attached_len() == 0
    }

    pub(crate) fn attached_entries(&self) -> impl Iterator<Item = (&str, &DevToolsSessionState)> {
        self.states.iter().filter_map(|(key, state)| match key {
            DevToolsSessionKey::Primary => None,
            DevToolsSessionKey::Attached(session_id) => Some((session_id.as_str(), state)),
        })
    }

    pub(crate) fn attached_states(&self) -> impl Iterator<Item = &DevToolsSessionState> {
        self.attached_entries().map(|(_session_id, state)| state)
    }

    pub(crate) fn states(&self) -> impl Iterator<Item = &DevToolsSessionState> {
        self.states.values()
    }

    /// Iterates browser-side domain handlers in attachment order: the implicit
    /// primary session first, then flattened sessions in attach order. Network
    /// header merging and Emulation identity precedence expose this order.
    pub(crate) fn states_in_attachment_order(&self) -> impl Iterator<Item = &DevToolsSessionState> {
        std::iter::once(self.primary()).chain(
            self.attached_order
                .iter()
                .filter_map(|session_id| self.attached(session_id)),
        )
    }

    pub(crate) fn effective_network_policy(&self) -> DevToolsNetworkPolicyAggregate {
        let mut aggregate = DevToolsNetworkPolicyAggregate::default();
        for state in self.states_in_attachment_order() {
            let network = &state.network_session_state;
            if !network.network_enabled {
                continue;
            }
            aggregate.cache_disabled |= network.cache_disabled;
            aggregate.bypass_service_worker |= network.bypass_service_worker;
            for (name, value) in &network.extra_headers {
                if let Some(index) = aggregate
                    .extra_headers
                    .iter()
                    .position(|(existing, _)| existing.eq_ignore_ascii_case(name))
                {
                    aggregate.extra_headers[index] = (name.clone(), value.clone());
                } else {
                    aggregate.extra_headers.push((name.clone(), value.clone()));
                }
            }
        }
        aggregate
    }

    pub(crate) fn effective_network_browser_identity_override(
        &self,
    ) -> Option<moli_browser_profile::BrowserIdentityProfile> {
        Self::aggregate_browser_identity_overrides(self.states_in_attachment_order().filter_map(
            |state| {
                state
                    .emulation_session_state
                    .browser_identity_override
                    .as_ref()
            },
        ))
    }

    /// Resolves the identity exposed by the live renderer Document.
    ///
    /// Chromium's renderer agents enter the instrumenting-agent list when a
    /// session first enables a non-empty UA override. Updating that session
    /// does not move it in the list, so this order intentionally differs from
    /// the browser-side attachment order used for navigation request headers.
    pub(crate) fn effective_renderer_browser_identity_override(
        &self,
    ) -> Option<moli_browser_profile::BrowserIdentityProfile> {
        Self::aggregate_browser_identity_overrides(
            self.browser_identity_activation_order
                .iter()
                .filter_map(|key| self.states.get(key))
                .filter_map(|state| {
                    state
                        .emulation_session_state
                        .browser_identity_override
                        .as_ref()
                }),
        )
    }

    fn aggregate_browser_identity_overrides<'a>(
        contributions: impl Iterator<Item = &'a DevToolsBrowserIdentityOverride>,
    ) -> Option<moli_browser_profile::BrowserIdentityProfile> {
        let mut identity_base = None;
        let mut user_agent = None;
        let mut accept_language = None;
        let mut navigator_platform = None;
        for contribution in contributions {
            identity_base = Some(&contribution.base);
            if contribution.user_agent.is_some() {
                user_agent = Some(contribution);
            }
            if contribution.accept_language.is_some() {
                accept_language = Some(contribution);
            }
            if contribution.navigator_platform.is_some() {
                navigator_platform = Some(contribution);
            }
        }
        identity_base.map(|base| {
            moli_browser_profile::BrowserIdentityProfile::from_devtools_override(
                base,
                user_agent
                    .and_then(|contribution| contribution.user_agent.clone())
                    .unwrap_or_default(),
                accept_language.and_then(|contribution| contribution.accept_language.clone()),
                navigator_platform.and_then(|contribution| contribution.navigator_platform.clone()),
                user_agent.and_then(|contribution| contribution.user_agent_metadata.clone()),
            )
        })
    }

    fn routed_key(is_attached_session: bool, session_id: Option<&str>) -> DevToolsSessionKey {
        if is_attached_session && let Some(session_id) = session_id {
            return DevToolsSessionKey::Attached(session_id.to_owned());
        }
        DevToolsSessionKey::Primary
    }

    pub(crate) fn set_browser_identity_override(
        &mut self,
        is_attached_session: bool,
        session_id: Option<&str>,
        browser_identity_override: Option<DevToolsBrowserIdentityOverride>,
    ) {
        let key = Self::routed_key(is_attached_session, session_id);
        if browser_identity_override.is_some()
            && !self.browser_identity_activation_order.contains(&key)
        {
            self.browser_identity_activation_order.push(key);
        }
        let state = self.routed_mut_or_insert(is_attached_session, session_id);
        state.emulation_session_state.browser_identity_override = browser_identity_override;
    }

    pub(crate) fn set_locale_override(
        &mut self,
        is_attached_session: bool,
        session_id: Option<&str>,
        locale_override: Option<String>,
    ) -> Result<(), &'static str> {
        let key = Self::routed_key(is_attached_session, session_id);
        let current_session_owns_override = self
            .states
            .get(&key)
            .is_some_and(|state| state.emulation_session_state.locale_override.is_some());
        let another_session_owns_override = self.states.iter().any(|(candidate, state)| {
            candidate != &key && state.emulation_session_state.locale_override.is_some()
        });
        if !current_session_owns_override && another_session_owns_override {
            return Err("Another locale override is already in effect");
        }
        self.routed_mut_or_insert(is_attached_session, session_id)
            .emulation_session_state
            .locale_override = locale_override;
        Ok(())
    }

    pub(crate) fn set_timezone_override(
        &mut self,
        is_attached_session: bool,
        session_id: Option<&str>,
        timezone_override: Option<String>,
    ) -> Result<(), &'static str> {
        let key = Self::routed_key(is_attached_session, session_id);
        let current_session_owns_override = self
            .states
            .get(&key)
            .is_some_and(|state| state.emulation_session_state.timezone_override.is_some());
        let another_session_owns_override = self.states.iter().any(|(candidate, state)| {
            candidate != &key && state.emulation_session_state.timezone_override.is_some()
        });
        if timezone_override.is_some()
            && !current_session_owns_override
            && another_session_owns_override
        {
            return Err("Timezone override is already in effect");
        }
        self.routed_mut_or_insert(is_attached_session, session_id)
            .emulation_session_state
            .timezone_override = timezone_override;
        Ok(())
    }

    pub(crate) fn effective_locale_override(&self) -> Option<&str> {
        self.states
            .values()
            .find_map(|state| state.emulation_session_state.locale_override.as_deref())
    }

    pub(crate) fn effective_timezone_override(&self) -> Option<&str> {
        self.states
            .values()
            .find_map(|state| state.emulation_session_state.timezone_override.as_deref())
    }

    pub(crate) fn clear_routed_network_state(
        &mut self,
        is_attached_session: bool,
        session_id: Option<&str>,
    ) {
        let key = Self::routed_key(is_attached_session, session_id);
        if let Some(state) = self.states.get_mut(&key) {
            state.network_session_state = DevToolsNetworkSessionState::default();
        }
    }

    pub(crate) fn clear_routed_emulation_state(
        &mut self,
        is_attached_session: bool,
        session_id: Option<&str>,
    ) {
        let key = Self::routed_key(is_attached_session, session_id);
        if let Some(state) = self.states.get_mut(&key) {
            state.emulation_session_state = DevToolsEmulationSessionState::default();
        }
        self.browser_identity_activation_order
            .retain(|candidate| candidate != &key);
    }

    pub(crate) fn states_mut(&mut self) -> impl Iterator<Item = &mut DevToolsSessionState> {
        self.states.values_mut()
    }

    pub(crate) fn reset(&mut self, preserve_attached_sessions: bool) {
        *self.primary_mut() = DevToolsSessionState::default();
        self.browser_identity_activation_order
            .retain(|key| !matches!(key, DevToolsSessionKey::Primary));
        if !preserve_attached_sessions {
            self.clear_attached();
        }
    }

    pub(crate) fn has_non_default_state(&self) -> bool {
        self.primary() != &DevToolsSessionState::default() || !self.attached_is_empty()
    }

    pub(crate) fn has_pending_inspector_awaits(&self) -> bool {
        self.states()
            .any(DevToolsSessionState::has_pending_inspector_awaits)
    }

    pub(crate) fn pending_inspector_await_count(&self) -> usize {
        self.states()
            .map(DevToolsSessionState::pending_inspector_await_count)
            .sum()
    }

    pub(crate) fn drain_pending_inspector_awaits_for_sessions(
        &mut self,
        session_ids: &[&str],
    ) -> Vec<(u64, PendingInspectorAwait)> {
        self.states_mut()
            .flat_map(|state| state.drain_pending_inspector_awaits_for_sessions(session_ids))
            .collect()
    }

    pub(crate) fn prepare_renderer_call_replacements(
        &mut self,
        primary_session_id: Option<&str>,
        old_attachment_id: moli_page_types::RendererAgentAttachmentId,
        new_attachment_id: moli_page_types::RendererAgentAttachmentId,
    ) -> Result<PreparedRendererCallReplacements, RendererCallIdExhausted> {
        let terminations = self.prepare_renderer_call_terminations(
            primary_session_id,
            old_attachment_id,
            new_attachment_id,
        )?;
        let replays = self.prepare_renderer_call_replays(
            primary_session_id,
            old_attachment_id,
            new_attachment_id,
        )?;
        Ok(PreparedRendererCallReplacements::new(
            new_attachment_id,
            terminations,
            replays,
        ))
    }

    fn prepare_renderer_call_replays(
        &mut self,
        primary_session_id: Option<&str>,
        old_attachment_id: moli_page_types::RendererAgentAttachmentId,
        new_attachment_id: moli_page_types::RendererAgentAttachmentId,
    ) -> Result<Vec<SessionRendererCallReplay>, RendererCallIdExhausted> {
        let mut replays = Vec::new();
        for (key, state) in &mut self.states {
            let (frontend_session_id, renderer_inspector_session_id) = match key {
                DevToolsSessionKey::Primary => (primary_session_id.map(str::to_owned), None),
                DevToolsSessionKey::Attached(session_id) => {
                    (Some(session_id.clone()), Some(session_id.clone()))
                }
            };
            replays.extend(
                state
                    .prepare_renderer_call_replays(old_attachment_id, new_attachment_id)?
                    .into_iter()
                    .map(|replay| SessionRendererCallReplay {
                        frontend_session_id: frontend_session_id.clone(),
                        renderer_inspector_session_id: renderer_inspector_session_id.clone(),
                        replay,
                    }),
            );
        }
        Ok(replays)
    }

    fn prepare_renderer_call_terminations(
        &mut self,
        primary_session_id: Option<&str>,
        old_attachment_id: moli_page_types::RendererAgentAttachmentId,
        terminal_attachment_id: moli_page_types::RendererAgentAttachmentId,
    ) -> Result<Vec<SessionRendererCallTermination>, RendererCallIdExhausted> {
        let mut terminations = Vec::new();
        for (key, state) in &mut self.states {
            let frontend_session_id = match key {
                DevToolsSessionKey::Primary => primary_session_id.map(str::to_owned),
                DevToolsSessionKey::Attached(session_id) => Some(session_id.clone()),
            };
            terminations.extend(
                state
                    .prepare_renderer_call_terminations(old_attachment_id, terminal_attachment_id)?
                    .into_iter()
                    .map(|termination| SessionRendererCallTermination {
                        frontend_session_id: frontend_session_id.clone(),
                        termination,
                    }),
            );
        }
        Ok(terminations)
    }

    pub(crate) fn runtime_bindings_for_renderer(&self) -> Vec<RuntimeBindingDefinition> {
        let mut bindings = Vec::new();
        for (key, state) in &self.states {
            for binding in &state.runtime_bindings {
                let binding = binding.with_devtools_session(key.clone());
                if !bindings.iter().any(|existing| existing == &binding) {
                    bindings.push(binding);
                }
            }
        }
        bindings
    }

    pub(crate) fn runtime_inspector_restore_snapshots(
        &self,
    ) -> Vec<RendererInspectorSessionRestoreSnapshot> {
        self.states
            .iter()
            .filter_map(|(key, state)| {
                let requires_restore = state.runtime_session_state.runtime_frontend_enabled
                    || state.console_output_session_state.console_enabled
                    || !state.runtime_bindings.is_empty()
                    || !state.dom_debugger_event_listener_breakpoints.is_empty()
                    || !state.dom_debugger_xhr_breakpoints.is_empty()
                    || state.inspector_session_state.v8_state.is_some();
                requires_restore.then(|| RendererInspectorSessionRestoreSnapshot {
                    inspector_session_id: key.wire_session_id().map(str::to_owned),
                    v8_attach: V8InspectorSessionAttach::from_optional_state(
                        state.inspector_session_state.v8_state.clone(),
                    ),
                    protocol_configuration: RendererInspectorProtocolConfiguration {
                        runtime_bindings: state.runtime_bindings.clone(),
                        runtime_frontend_enabled: state
                            .runtime_session_state
                            .runtime_frontend_enabled,
                        console_frontend_enabled: state
                            .console_output_session_state
                            .console_enabled,
                        dom_debugger_event_listener_breakpoints: state
                            .dom_debugger_event_listener_breakpoints
                            .clone(),
                        dom_debugger_xhr_breakpoints: state.dom_debugger_xhr_breakpoints.clone(),
                    },
                })
            })
            .collect()
    }

    pub(crate) fn page_bypass_csp_enabled(&self) -> bool {
        self.states()
            .any(|state| state.page_session_state.page_bypass_csp_enabled)
    }
}

impl Index<DevToolsSessionKey> for DevToolsSessionRegistry {
    type Output = DevToolsSessionState;

    fn index(&self, key: DevToolsSessionKey) -> &Self::Output {
        self.states
            .get(&key)
            .expect("indexed DevTools session must be registered")
    }
}

impl IndexMut<DevToolsSessionKey> for DevToolsSessionRegistry {
    fn index_mut(&mut self, key: DevToolsSessionKey) -> &mut Self::Output {
        self.states
            .get_mut(&key)
            .expect("indexed DevTools session must be registered")
    }
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
pub(crate) struct DevToolsNetworkSessionState {
    pub(crate) network_enabled: bool,
    pub(crate) cache_disabled: bool,
    pub(crate) bypass_service_worker: bool,
    pub(crate) extra_headers: Vec<(String, String)>,
    pub(crate) service_worker_fetch_diagnostic_entries: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DevToolsEmulationSessionState {
    // UA, Accept-Language, and platform are independent handler contributions.
    pub(crate) browser_identity_override: Option<DevToolsBrowserIdentityOverride>,
    // Locale and timezone are exclusive controller claims, unlike UA fields.
    pub(crate) locale_override: Option<String>,
    pub(crate) timezone_override: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DevToolsBrowserIdentityOverride {
    // Keep individual command contributions instead of a flattened profile:
    // Chromium lets different sessions win UA, Accept-Language, and
    // navigator.platform independently in attachment order.
    base: moli_browser_profile::BrowserIdentityProfile,
    user_agent: Option<String>,
    accept_language: Option<String>,
    navigator_platform: Option<String>,
    user_agent_metadata: Option<moli_browser_profile::BrowserUserAgentMetadataOverride>,
}

impl DevToolsBrowserIdentityOverride {
    pub(crate) fn from_command(
        base: &moli_browser_profile::BrowserIdentityProfile,
        user_agent: String,
        accept_language: Option<String>,
        navigator_platform: Option<String>,
        user_agent_metadata: Option<moli_browser_profile::BrowserUserAgentMetadataOverride>,
    ) -> Option<Self> {
        let user_agent = (!user_agent.is_empty()).then_some(user_agent);
        let accept_language = accept_language.filter(|value| !value.is_empty());
        let navigator_platform = navigator_platform.filter(|value| !value.is_empty());
        (user_agent.is_some() || accept_language.is_some() || navigator_platform.is_some()).then(
            || Self {
                base: base.clone(),
                user_agent,
                accept_language,
                navigator_platform,
                user_agent_metadata,
            },
        )
    }

    pub(crate) fn to_browser_identity(&self) -> moli_browser_profile::BrowserIdentityProfile {
        moli_browser_profile::BrowserIdentityProfile::from_devtools_override(
            &self.base,
            self.user_agent.clone().unwrap_or_default(),
            self.accept_language.clone(),
            self.navigator_platform.clone(),
            self.user_agent_metadata.clone(),
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DevToolsNetworkPolicyAggregate {
    pub(crate) cache_disabled: bool,
    pub(crate) bypass_service_worker: bool,
    pub(crate) extra_headers: Vec<(String, String)>,
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
                devtools_session: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(name: &str) -> RuntimeBindingDefinition {
        RuntimeBindingDefinition {
            devtools_session: None,
            name: name.to_owned(),
            execution_context_name: None,
        }
    }

    fn identity_override(
        user_agent: &str,
        accept_language: Option<&str>,
        navigator_platform: Option<&str>,
    ) -> Option<DevToolsBrowserIdentityOverride> {
        DevToolsBrowserIdentityOverride::from_command(
            &moli_browser_profile::BrowserIdentityProfile::default(),
            user_agent.to_owned(),
            accept_language.map(str::to_owned),
            navigator_platform.map(str::to_owned),
            None,
        )
    }

    fn effective_identity(
        sessions: &DevToolsSessionRegistry,
    ) -> moli_browser_profile::BrowserIdentityProfile {
        sessions
            .effective_network_browser_identity_override()
            .expect("browser identity contribution should be effective")
    }

    fn effective_renderer_identity(
        sessions: &DevToolsSessionRegistry,
    ) -> moli_browser_profile::BrowserIdentityProfile {
        sessions
            .effective_renderer_browser_identity_override()
            .expect("renderer browser identity contribution should be effective")
    }

    #[test]
    fn registry_owns_primary_and_attached_sessions_in_stable_order() {
        let mut sessions = DevToolsSessionRegistry::default();
        sessions.ensure_attached("SID-b");
        sessions.ensure_attached("SID-a");

        assert_eq!(sessions.attached_len(), 2);
        assert_eq!(
            sessions
                .attached_entries()
                .map(|(session_id, _state)| session_id)
                .collect::<Vec<_>>(),
            ["SID-a", "SID-b"]
        );

        sessions.remove_attached("SID-a");
        assert!(sessions.attached("SID-a").is_none());
        assert!(sessions.attached("SID-b").is_some());
        assert_eq!(sessions.primary(), &DevToolsSessionState::default());
    }

    #[test]
    fn network_policy_uses_attachment_order_and_domain_specific_aggregation() {
        let mut sessions = DevToolsSessionRegistry::default();
        let primary = &mut sessions.primary_mut().network_session_state;
        primary.network_enabled = true;
        primary.cache_disabled = true;
        primary.extra_headers = vec![
            ("X-Primary".to_owned(), "primary".to_owned()),
            ("X-Shared".to_owned(), "primary".to_owned()),
        ];

        let attached_b = &mut sessions.ensure_attached("SID-b").network_session_state;
        attached_b.network_enabled = true;
        attached_b.bypass_service_worker = true;
        attached_b.extra_headers = vec![
            ("X-B".to_owned(), "b".to_owned()),
            ("x-shared".to_owned(), "b".to_owned()),
        ];

        let attached_a = &mut sessions.ensure_attached("SID-a").network_session_state;
        attached_a.network_enabled = false;
        attached_a.cache_disabled = true;
        attached_a.extra_headers = vec![("X-Ignored".to_owned(), "a".to_owned())];

        let policy = sessions.effective_network_policy();
        assert!(policy.cache_disabled);
        assert!(policy.bypass_service_worker);
        assert_eq!(
            policy.extra_headers,
            vec![
                ("X-Primary".to_owned(), "primary".to_owned()),
                ("x-shared".to_owned(), "b".to_owned()),
                ("X-B".to_owned(), "b".to_owned()),
            ]
        );

        sessions.remove_attached("SID-b");
        let policy = sessions.effective_network_policy();
        assert!(policy.cache_disabled);
        assert!(!policy.bypass_service_worker);
        assert_eq!(
            policy.extra_headers,
            vec![
                ("X-Primary".to_owned(), "primary".to_owned()),
                ("X-Shared".to_owned(), "primary".to_owned()),
            ]
        );
    }

    #[test]
    fn browser_identity_uses_distinct_network_and_renderer_agent_order() {
        let mut sessions = DevToolsSessionRegistry::default();
        sessions.ensure_attached("SID-later");
        sessions.set_browser_identity_override(
            true,
            Some("SID-later"),
            identity_override("Moli/Later-1", None, None),
        );
        sessions.set_browser_identity_override(
            false,
            None,
            identity_override("Moli/Primary-1", None, None),
        );
        assert_eq!(effective_identity(&sessions).user_agent(), "Moli/Later-1");
        assert_eq!(
            effective_renderer_identity(&sessions).user_agent(),
            "Moli/Primary-1",
            "the renderer follows agent activation order, not attachment order"
        );

        sessions.set_browser_identity_override(
            true,
            Some("SID-later"),
            identity_override("Moli/Later-2", None, None),
        );
        assert_eq!(
            effective_identity(&sessions).user_agent(),
            "Moli/Later-2",
            "the browser-side winner remains the later-attached session"
        );
        assert_eq!(
            effective_renderer_identity(&sessions).user_agent(),
            "Moli/Primary-1",
            "updating an enabled renderer agent must not reorder it"
        );

        sessions.set_browser_identity_override(
            false,
            None,
            identity_override("Moli/Primary-2", None, None),
        );
        assert_eq!(effective_identity(&sessions).user_agent(), "Moli/Later-2");
        assert_eq!(
            effective_renderer_identity(&sessions).user_agent(),
            "Moli/Primary-2"
        );

        sessions.set_browser_identity_override(false, None, None);
        assert_eq!(effective_identity(&sessions).user_agent(), "Moli/Later-2");
        assert_eq!(
            effective_renderer_identity(&sessions).user_agent(),
            "Moli/Later-2"
        );

        sessions.set_browser_identity_override(
            false,
            None,
            identity_override("Moli/Primary-3", Some("fr-FR"), Some("PrimaryPlatform")),
        );
        assert_eq!(effective_identity(&sessions).user_agent(), "Moli/Later-2");
        let renderer_identity = effective_renderer_identity(&sessions);
        assert_eq!(renderer_identity.user_agent(), "Moli/Primary-3");
        assert_eq!(renderer_identity.accept_language(), "fr-FR");
        assert_eq!(renderer_identity.navigator_platform(), "PrimaryPlatform");

        sessions.remove_attached("SID-later");
        assert_eq!(effective_identity(&sessions).user_agent(), "Moli/Primary-3");
        assert_eq!(
            effective_renderer_identity(&sessions).user_agent(),
            "Moli/Primary-3"
        );
    }

    #[test]
    fn locale_and_timezone_follow_their_distinct_exclusive_claim_rules() {
        let mut sessions = DevToolsSessionRegistry::default();
        sessions.ensure_attached("SID-a");
        sessions.ensure_attached("SID-b");

        sessions
            .set_locale_override(true, Some("SID-a"), Some("fr-FR".to_owned()))
            .unwrap();
        assert_eq!(
            sessions
                .set_locale_override(true, Some("SID-b"), Some("de-DE".to_owned()))
                .unwrap_err(),
            "Another locale override is already in effect"
        );
        assert_eq!(
            sessions
                .set_locale_override(true, Some("SID-b"), None)
                .unwrap_err(),
            "Another locale override is already in effect",
            "Chromium treats a non-owner locale clear as a new claim"
        );
        sessions
            .set_locale_override(true, Some("SID-a"), Some("it-IT".to_owned()))
            .unwrap();
        assert_eq!(sessions.effective_locale_override(), Some("it-IT"));

        sessions
            .set_timezone_override(true, Some("SID-a"), Some("Europe/Paris".to_owned()))
            .unwrap();
        assert_eq!(
            sessions
                .set_timezone_override(true, Some("SID-b"), Some("America/New_York".to_owned()),)
                .unwrap_err(),
            "Timezone override is already in effect"
        );
        sessions
            .set_timezone_override(true, Some("SID-b"), None)
            .expect("Chromium accepts a non-owner timezone clear as a no-op");
        assert_eq!(sessions.effective_timezone_override(), Some("Europe/Paris"));

        sessions.remove_attached("SID-a");
        assert_eq!(sessions.effective_locale_override(), None);
        assert_eq!(sessions.effective_timezone_override(), None);
        sessions
            .set_locale_override(true, Some("SID-b"), Some("de-DE".to_owned()))
            .unwrap();
        sessions
            .set_timezone_override(true, Some("SID-b"), Some("America/New_York".to_owned()))
            .unwrap();
    }

    #[test]
    fn registry_aggregates_renderer_configuration_by_session_identity() {
        let mut sessions = DevToolsSessionRegistry::default();
        sessions
            .primary_mut()
            .runtime_bindings
            .push(binding("primary"));
        sessions
            .ensure_attached("SID-b")
            .runtime_bindings
            .push(binding("attached-b"));
        let attached_a = sessions.ensure_attached("SID-a");
        attached_a.runtime_bindings.push(binding("attached-a"));
        attached_a.runtime_session_state.runtime_frontend_enabled = true;
        attached_a.page_session_state.page_bypass_csp_enabled = true;

        let bindings = sessions.runtime_bindings_for_renderer();
        assert_eq!(
            bindings
                .iter()
                .map(|binding| (binding.name.as_str(), binding.devtools_session.clone()))
                .collect::<Vec<_>>(),
            [
                ("primary", Some(DevToolsSessionKey::Primary)),
                (
                    "attached-a",
                    Some(DevToolsSessionKey::Attached("SID-a".to_owned())),
                ),
                (
                    "attached-b",
                    Some(DevToolsSessionKey::Attached("SID-b".to_owned())),
                ),
            ]
        );
        assert!(sessions.page_bypass_csp_enabled());

        let restores = sessions.runtime_inspector_restore_snapshots();
        assert_eq!(
            restores
                .iter()
                .map(|restore| restore.inspector_session_id.as_deref())
                .collect::<Vec<_>>(),
            [None, Some("SID-a"), Some("SID-b")]
        );

        sessions.remove_attached("SID-a");
        assert!(!sessions.page_bypass_csp_enabled());
    }

    #[test]
    fn registry_pending_await_aggregation_tracks_attached_disposal() {
        let mut sessions = DevToolsSessionRegistry::default();
        sessions
            .primary_mut()
            .register_pending_inspector_await(1, Some("SID-primary"), None);
        sessions
            .ensure_attached("SID-attached")
            .register_pending_inspector_await(2, Some("SID-attached"), Some("group"));

        assert!(sessions.has_pending_inspector_awaits());
        assert_eq!(sessions.pending_inspector_await_count(), 2);
        let drained = sessions.drain_pending_inspector_awaits_for_sessions(&["SID-attached"]);
        assert_eq!(drained.len(), 1);
        assert_eq!(sessions.pending_inspector_await_count(), 1);

        sessions.remove_attached("SID-attached");
        assert_eq!(sessions.pending_inspector_await_count(), 1);
        sessions.reset(false);
        assert!(!sessions.has_pending_inspector_awaits());
    }
}
