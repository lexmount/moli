use std::collections::{HashMap, HashSet};

use moli_core::network::SharedWebStorageStore;
use moli_page_types::FrontendCommandId;
use serde_json::{Value, json};

use crate::{
    conn::cookie_manager_surface,
    devtools_runtime::{
        DevToolsBidiChannelProperties, DevToolsRealmId, DevToolsRemoteHandleId, DevToolsTargetId,
    },
    domains::{
        audits_output_state::TargetAuditsStorageState,
        console_output_state::TargetConsoleOutputState,
        log_output_state::TargetLogStorageState,
        network::{CollectedNetworkDataArtifact, TargetNetworkArtifacts},
        observable_output::TargetRuntimeObservableState,
    },
};

use super::{
    DevToolsSessionState, devtools_sessions_have_pending_inspector_awaits,
    devtools_sessions_pending_inspector_await_count,
    emulation::{
        EmulatedDeviceMetrics, EmulatedGeolocationOverrideState, EmulatedMediaOverrides,
        EmulatedNetworkConditions,
    },
    fetch::{ParkedFetchState, TargetFetchConfig},
    identity::TargetIdentityState,
    navigation::{PageNavigationHistoryEntry, TargetNavigationHistoryState},
    page_resource::TargetPageResourceStore,
    page_slot::{DocumentStartScript, IsolatedWorldDefinition, TargetPageSlot},
    pending_renderer_command::{
        DuplicatePendingRendererCommand, PendingRendererCommandRegistry,
        PreparedRendererCallDispatch, PreparedRendererCallReplay, PreparedRendererCallTermination,
        RegisterRendererCallError, RendererCallIdExhausted, RendererCommandCorrelation,
        RendererCommandDescriptor,
    },
    runtime_slot::{TargetNetworkRequestCounters, TargetRuntimeSlot},
    session::TargetNetworkPolicyState,
    session_storage::TargetSessionStorageNamespace,
};

#[derive(Debug)]
pub struct BackgroundTarget {
    pub(in crate::conn) target_id: String,
    pub(in crate::conn) session_id: Option<String>,
    pub(in crate::conn) target_identity: TargetIdentityState,
    pub(in crate::conn) runtime_slot: TargetRuntimeSlot,
    session_storage_namespace: TargetSessionStorageNamespace,
}

impl BackgroundTarget {
    pub(crate) fn new(
        target_id: String,
        session_id: Option<String>,
        target_identity: TargetIdentityState,
        target_page_slot: TargetPageSlot,
    ) -> Self {
        Self {
            target_id,
            session_id,
            target_identity,
            runtime_slot: TargetRuntimeSlot::from_page_slot(target_page_slot),
            session_storage_namespace: TargetSessionStorageNamespace::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_url(target_id: String, session_id: Option<String>, url: String) -> Self {
        Self::new(
            target_id,
            session_id,
            TargetIdentityState::with_url(url),
            TargetPageSlot::empty_for_test_fixture(),
        )
    }

    pub(crate) fn with_identity(
        target_id: String,
        session_id: Option<String>,
        target_identity: TargetIdentityState,
    ) -> Self {
        Self::new(
            target_id,
            session_id,
            target_identity,
            TargetPageSlot::empty_for_initial_document_page_build(),
        )
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn is_target(&self, target_id: &str) -> bool {
        self.target_id() == target_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(crate) fn has_session(&self) -> bool {
        self.session_id.is_some()
    }

    #[cfg(test)]
    pub(crate) fn auxiliary_browsing_context_id(&self) -> Option<u64> {
        self.runtime_slot
            .page_slot()
            .auxiliary_browsing_context_id()
    }

    pub(crate) fn is_session(&self, session_id: &str) -> bool {
        self.session_id() == Some(session_id)
    }

    pub(crate) fn attach_session(&mut self, session_id: String) {
        self.session_id = Some(session_id);
    }

    pub(crate) fn detach_session(&mut self) -> Option<String> {
        self.session_id.take()
    }

    pub(crate) fn session_storage_store(&self) -> &SharedWebStorageStore {
        self.session_storage_namespace.store()
    }

    pub(crate) fn deep_clone_session_storage_namespace(&self) -> TargetSessionStorageNamespace {
        self.session_storage_namespace.deep_clone()
    }

    pub(crate) fn replace_session_storage_namespace(
        &mut self,
        namespace: TargetSessionStorageNamespace,
    ) {
        self.session_storage_namespace = namespace;
    }

    pub(crate) fn take_session_storage_namespace(&mut self) -> TargetSessionStorageNamespace {
        std::mem::take(&mut self.session_storage_namespace)
    }
}

#[derive(Debug)]
pub(crate) struct TargetSlotState {
    target: BackgroundTarget,
    aux_state: ParkedTargetAuxState,
}

impl TargetSlotState {
    pub(crate) fn new(target: BackgroundTarget, aux_state: ParkedTargetAuxState) -> Self {
        Self { target, aux_state }
    }

    pub(crate) fn from_active_snapshot(
        target_id: String,
        session_id: Option<String>,
        target_identity: TargetIdentityState,
        runtime_slot: TargetRuntimeSlot,
        session_storage_namespace: TargetSessionStorageNamespace,
        aux_state: ParkedTargetAuxState,
    ) -> Self {
        Self {
            target: BackgroundTarget {
                target_id,
                session_id,
                target_identity,
                runtime_slot,
                session_storage_namespace,
            },
            aux_state,
        }
    }

    #[cfg(test)]
    pub(crate) fn target(&self) -> &BackgroundTarget {
        &self.target
    }

    pub(crate) fn target_id(&self) -> &str {
        self.target.target_id()
    }

    #[cfg(test)]
    pub(crate) fn aux_state(&self) -> &ParkedTargetAuxState {
        &self.aux_state
    }

    pub(crate) fn into_parts(self) -> (BackgroundTarget, ParkedTargetAuxState) {
        (self.target, self.aux_state)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParkedPageSessionState {
    pub(crate) devtools_session_state: DevToolsSessionState,
    pub(crate) auxiliary_devtools_session_states: HashMap<String, DevToolsSessionState>,
    pub network_enabled: bool,
    pub(crate) network_policy: TargetNetworkPolicyState,
    pub http_proxy_override: Option<String>,
    pub http_no_proxy_override: Option<String>,
    pub tls_verify_host_override: Option<bool>,
    pub locale_override: Option<String>,
    pub timezone_override: Option<String>,
    pub(crate) network_conditions: Option<EmulatedNetworkConditions>,
    pub geolocation_override: Option<EmulatedGeolocationOverrideState>,
    pub emulated_media: EmulatedMediaOverrides,
    pub emulated_device_metrics: Option<EmulatedDeviceMetrics>,
    pub cpu_throttling_rate: f64,
    pub touch_emulation_enabled: bool,
    pub emit_touch_events_for_mouse: bool,
    pub focus_emulation_enabled: bool,
    pub script_execution_disabled: bool,
    pub css_enabled: bool,
    pub fetch_config: TargetFetchConfig,
}

impl Default for ParkedPageSessionState {
    fn default() -> Self {
        Self {
            devtools_session_state: DevToolsSessionState::default(),
            auxiliary_devtools_session_states: HashMap::new(),
            network_enabled: false,
            network_policy: TargetNetworkPolicyState::default(),
            http_proxy_override: None,
            http_no_proxy_override: None,
            tls_verify_host_override: None,
            locale_override: None,
            timezone_override: None,
            network_conditions: None,
            geolocation_override: None,
            emulated_media: EmulatedMediaOverrides::default(),
            emulated_device_metrics: None,
            cpu_throttling_rate: 1.0,
            touch_emulation_enabled: false,
            emit_touch_events_for_mouse: false,
            focus_emulation_enabled: false,
            script_execution_disabled: false,
            css_enabled: false,
            fetch_config: TargetFetchConfig::default(),
        }
    }
}

impl ParkedPageSessionState {
    pub(crate) fn has_pending_javascript_dialog(&self) -> bool {
        !self
            .devtools_session_state
            .page_session_state
            .javascript_dialog_state
            .is_empty()
            || self
                .auxiliary_devtools_session_states
                .values()
                .any(|session| {
                    !session
                        .page_session_state
                        .javascript_dialog_state
                        .is_empty()
                })
    }
}

fn parked_page_session_has_page_domain_enabled(state: &ParkedPageSessionState) -> bool {
    state
        .devtools_session_state
        .page_session_state
        .page_domain_enabled
        || state
            .auxiliary_devtools_session_states
            .values()
            .any(|session_state| session_state.page_session_state.page_domain_enabled)
}

fn parked_page_session_has_runtime_remote_object_id(
    state: &ParkedPageSessionState,
    object_id: &str,
) -> bool {
    state
        .devtools_session_state
        .has_runtime_remote_object_id(object_id)
        || state
            .auxiliary_devtools_session_states
            .values()
            .any(|session_state| session_state.has_runtime_remote_object_id(object_id))
}

fn parked_page_session_has_runtime_remote_object_id_for_different_session(
    state: &ParkedPageSessionState,
    devtools_session_id: Option<&str>,
    object_id: &str,
) -> bool {
    if devtools_session_id.is_some()
        && state
            .devtools_session_state
            .has_runtime_remote_object_id(object_id)
    {
        return true;
    }
    state
        .auxiliary_devtools_session_states
        .iter()
        .any(|(session_id, session_state)| {
            Some(session_id.as_str()) != devtools_session_id
                && session_state.has_runtime_remote_object_id(object_id)
        })
}

pub(crate) fn parked_page_session_has_pending_inspector_awaits(
    state: &ParkedPageSessionState,
) -> bool {
    devtools_sessions_have_pending_inspector_awaits(
        &state.devtools_session_state,
        &state.auxiliary_devtools_session_states,
    )
}

pub(crate) fn parked_page_session_pending_inspector_await_count(
    state: &ParkedPageSessionState,
) -> usize {
    devtools_sessions_pending_inspector_await_count(
        &state.devtools_session_state,
        &state.auxiliary_devtools_session_states,
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParkedNetworkArtifacts {
    target_network_artifacts: TargetNetworkArtifacts,
    request_counters: TargetNetworkRequestCounters,
}

impl ParkedNetworkArtifacts {
    pub(crate) fn snapshot_from_runtime_slot(runtime_slot: &TargetRuntimeSlot) -> Self {
        Self {
            target_network_artifacts: runtime_slot.snapshot_network_artifacts(),
            request_counters: runtime_slot.snapshot_network_request_counters(),
        }
    }

    pub(crate) fn restore_into_runtime_slot(self, runtime_slot: &mut TargetRuntimeSlot) {
        runtime_slot.restore_network_artifacts(self.target_network_artifacts);
        runtime_slot.restore_network_request_counters(self.request_counters);
    }

    pub(crate) fn collected_network_data_artifacts(&self) -> Vec<CollectedNetworkDataArtifact> {
        self.target_network_artifacts
            .collected_network_data_artifacts()
    }

    pub(crate) fn drain_from_background_target(&mut self, target: &mut BackgroundTarget) {
        self.target_network_artifacts = target.runtime_slot.take_network_artifacts();
        self.request_counters = target.runtime_slot.take_network_request_counters();
    }

    pub(crate) fn drain_into_background_target(&mut self, target: &mut BackgroundTarget) {
        target
            .runtime_slot
            .restore_network_artifacts(std::mem::take(&mut self.target_network_artifacts));
        target
            .runtime_slot
            .restore_network_request_counters(std::mem::take(&mut self.request_counters));
    }

    pub(crate) fn set_session_observation_cursor_at_counts(
        &mut self,
        session_id: Option<&str>,
        subresource_count: usize,
        websocket_count: usize,
    ) {
        self.target_network_artifacts
            .set_session_observation_cursor_at_counts(
                session_id,
                subresource_count,
                websocket_count,
            );
    }

    pub(crate) fn remove_session_observation_cursor(&mut self, session_id: Option<&str>) {
        self.target_network_artifacts
            .remove_session_observation_cursor(session_id);
    }

    pub(crate) fn remove_captured_response_body_visibility_for_session(
        &mut self,
        session_id: Option<&str>,
    ) {
        self.target_network_artifacts
            .remove_captured_response_body_visibility_for_session(session_id);
    }

    pub(crate) fn clear_captured_response_bodies_and_websocket_request_ids(&mut self) {
        self.target_network_artifacts
            .clear_captured_response_bodies();
        self.target_network_artifacts.clear_websocket_request_ids();
    }

    #[cfg(test)]
    pub(crate) fn next_fetch_request_id_for_test(&self) -> u32 {
        self.request_counters.next_fetch_request_id
    }

    #[cfg(test)]
    pub(crate) fn next_subresource_fetch_request_id_for_test(&self) -> u32 {
        self.request_counters.next_subresource_fetch_request_id
    }

    #[cfg(test)]
    pub(crate) fn emitted_subresource_record_count_for_session(
        &self,
        session_id: Option<&str>,
    ) -> usize {
        self.target_network_artifacts
            .emitted_subresource_record_count_for_session(session_id)
    }

    #[cfg(test)]
    pub(crate) fn emitted_websocket_event_count_for_session(
        &self,
        session_id: Option<&str>,
    ) -> usize {
        self.target_network_artifacts
            .emitted_websocket_event_count_for_session(session_id)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TargetCrashState {
    crashed: bool,
}

impl TargetCrashState {
    pub(crate) fn mark_crashed(&mut self) {
        self.crashed = true;
    }

    pub(crate) fn clear(&mut self) {
        self.crashed = false;
    }

    pub(crate) fn is_crashed(self) -> bool {
        self.crashed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingBidiChannelListener {
    target_id: DevToolsTargetId,
    realm_id: DevToolsRealmId,
    channel_handle: DevToolsRemoteHandleId,
    channel_object_group: String,
    properties: DevToolsBidiChannelProperties,
}

impl PendingBidiChannelListener {
    pub(crate) fn new(
        target_id: Option<DevToolsTargetId>,
        realm_id: Option<DevToolsRealmId>,
        channel_handle: DevToolsRemoteHandleId,
        channel_object_group: String,
        properties: DevToolsBidiChannelProperties,
    ) -> Option<Self> {
        Some(Self {
            target_id: target_id?,
            realm_id: realm_id?,
            channel_handle,
            channel_object_group,
            properties,
        })
    }

    pub(crate) fn target_id(&self) -> &DevToolsTargetId {
        &self.target_id
    }

    pub(crate) fn realm_id(&self) -> &DevToolsRealmId {
        &self.realm_id
    }

    pub(crate) fn channel_handle(&self) -> &DevToolsRemoteHandleId {
        &self.channel_handle
    }

    pub(crate) fn channel_object_group(&self) -> &str {
        &self.channel_object_group
    }

    pub(crate) fn properties(&self) -> &DevToolsBidiChannelProperties {
        &self.properties
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingInspectorAwait {
    session_id: Option<String>,
    object_group: Option<String>,
    bidi_channel_listener: Option<crate::conn::BidiChannelListenerResidence>,
    renderer_correlation: Option<RendererCommandCorrelation>,
}

impl PendingInspectorAwait {
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(crate) fn object_group(&self) -> Option<&str> {
        self.object_group.as_deref()
    }

    pub(crate) fn bidi_channel_listener(
        &self,
    ) -> Option<&crate::conn::BidiChannelListenerResidence> {
        self.bidi_channel_listener.as_ref()
    }

    pub(crate) fn renderer_correlation(&self) -> Option<RendererCommandCorrelation> {
        self.renderer_correlation
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TargetPendingInspectorAwaitRegistry {
    entries: PendingRendererCommandRegistry<PendingInspectorAwait>,
}

impl TargetPendingInspectorAwaitRegistry {
    pub(crate) fn try_insert(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
        object_group: Option<&str>,
    ) -> Result<(), DuplicatePendingRendererCommand> {
        let frontend_command_id = FrontendCommandId::new(cdp_request_id);
        self.entries.try_insert(
            frontend_command_id,
            PendingInspectorAwait {
                session_id: session_id.map(str::to_owned),
                object_group: object_group.map(str::to_owned),
                bidi_channel_listener: None,
                renderer_correlation: None,
            },
        )
    }

    pub(crate) fn try_insert_bidi_channel_listener(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
        object_group: Option<&str>,
        listener: crate::conn::BidiChannelListenerResidence,
    ) -> Result<(), DuplicatePendingRendererCommand> {
        let frontend_command_id = FrontendCommandId::new(cdp_request_id);
        let renderer_correlation = self.entries.renderer_call_for_frontend(frontend_command_id);
        self.entries.try_insert(
            frontend_command_id,
            PendingInspectorAwait {
                session_id: session_id.map(str::to_owned),
                object_group: object_group.map(str::to_owned),
                bidi_channel_listener: Some(listener),
                renderer_correlation,
            },
        )
    }

    pub(crate) fn insert_bidi_channel_listener(
        &mut self,
        cdp_request_id: u64,
        session_id: Option<&str>,
        object_group: Option<&str>,
        listener: crate::conn::BidiChannelListenerResidence,
    ) {
        self.try_insert_bidi_channel_listener(cdp_request_id, session_id, object_group, listener)
            .expect("pending BiDi listener frontend command id must be unique per session");
    }

    pub(crate) fn try_register_renderer_call(
        &mut self,
        cdp_request_id: u64,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
        descriptor: RendererCommandDescriptor,
    ) -> Result<PreparedRendererCallDispatch, RegisterRendererCallError> {
        let dispatch = self.entries.try_register_renderer_call(
            FrontendCommandId::new(cdp_request_id),
            dispatched_attachment_id,
            descriptor,
        )?;
        if let Some(entry) = self.entries.get_mut(FrontendCommandId::new(cdp_request_id)) {
            entry.renderer_correlation = Some(dispatch.correlation());
        }
        Ok(dispatch)
    }

    pub(crate) fn take_renderer_call_for_frontend(
        &mut self,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        self.entries
            .take_renderer_call_for_frontend(FrontendCommandId::new(cdp_request_id))
    }

    pub(crate) fn renderer_call_for_frontend(
        &self,
        cdp_request_id: u64,
    ) -> Option<RendererCommandCorrelation> {
        self.entries
            .renderer_call_for_frontend(FrontendCommandId::new(cdp_request_id))
    }

    pub(crate) fn renderer_command_descriptor_for_renderer_if_attachment_matches(
        &self,
        renderer_call_id: moli_page_types::RendererCallId,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
    ) -> Option<RendererCommandDescriptor> {
        self.entries
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
        self.entries
            .prepare_replays_from_attachment(old_attachment_id, new_attachment_id)
    }

    pub(crate) fn prepare_renderer_call_terminations(
        &mut self,
        old_attachment_id: moli_page_types::RendererAgentAttachmentId,
        terminal_attachment_id: moli_page_types::RendererAgentAttachmentId,
    ) -> Result<Vec<PreparedRendererCallTermination>, RendererCallIdExhausted> {
        self.entries
            .prepare_terminations_from_attachment(old_attachment_id, terminal_attachment_id)
    }

    pub(crate) fn terminate_all_renderer_calls(
        &mut self,
        reason: &str,
    ) -> Vec<RendererCommandCorrelation> {
        self.entries.terminate_all_renderer_calls(reason)
    }

    pub(crate) fn take_renderer_call_for_frontend_if_matches(
        &mut self,
        cdp_request_id: u64,
        renderer_call_id: moli_page_types::RendererCallId,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        self.entries.take_renderer_call_for_frontend_if_matches(
            FrontendCommandId::new(cdp_request_id),
            renderer_call_id,
            dispatched_attachment_id,
        )
    }

    pub(crate) fn take_frontend_command_for_renderer_if_attachment_matches(
        &mut self,
        renderer_call_id: moli_page_types::RendererCallId,
        dispatched_attachment_id: Option<moli_page_types::RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        self.entries
            .take_frontend_command_for_renderer_if_attachment_matches(
                renderer_call_id,
                dispatched_attachment_id,
            )
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn remove(&mut self, cdp_request_id: u64) -> Option<PendingInspectorAwait> {
        self.entries.remove(FrontendCommandId::new(cdp_request_id))
    }

    pub(crate) fn drain_all(&mut self) -> Vec<(u64, PendingInspectorAwait)> {
        let to_remove = self.entries.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        to_remove
            .into_iter()
            .filter_map(|id| {
                self.remove_for_cancellation(id)
                    .map(|entry| (id.get(), entry))
            })
            .collect()
    }

    fn remove_for_cancellation(
        &mut self,
        frontend_command_id: FrontendCommandId,
    ) -> Option<PendingInspectorAwait> {
        let entry = self.entries.remove(frontend_command_id)?;
        if let Some(correlation) = entry.renderer_correlation {
            let removed = self.entries.take_renderer_call_for_frontend_if_matches(
                frontend_command_id,
                correlation.renderer_call_id(),
                correlation.dispatched_attachment_id(),
            );
            debug_assert_eq!(removed, Some(correlation));
        }
        Some(entry)
    }

    pub(crate) fn drain_for_sessions(
        &mut self,
        session_ids: &[&str],
    ) -> Vec<(u64, PendingInspectorAwait)> {
        if self.entries.is_empty() || session_ids.is_empty() {
            return Vec::new();
        }
        let to_remove: Vec<FrontendCommandId> = self
            .entries
            .iter()
            .filter_map(|(id, entry)| {
                entry
                    .session_id
                    .as_deref()
                    .filter(|sid| session_ids.contains(sid))
                    .map(|_| *id)
            })
            .collect();
        to_remove
            .into_iter()
            .filter_map(|id| {
                self.remove_for_cancellation(id)
                    .map(|entry| (id.get(), entry))
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TargetWindowSurfaceState {
    #[default]
    Normal,
    Maximized,
    Minimized,
    Fullscreen,
}

impl TargetWindowSurfaceState {
    pub(crate) fn document_hidden(self) -> bool {
        matches!(self, Self::Minimized)
    }

    pub(crate) fn is_fullscreen(self) -> bool {
        matches!(self, Self::Fullscreen)
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Maximized => "maximized",
            Self::Minimized => "minimized",
            Self::Fullscreen => "fullscreen",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TargetWindowSurfaceGeometry {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) x: i32,
    pub(crate) y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetInitialEmptyDocumentCreator {
    target_id: String,
    security_origin: String,
    secure_context_type: String,
}

impl TargetInitialEmptyDocumentCreator {
    pub(crate) fn new(
        target_id: String,
        security_origin: String,
        secure_context_type: String,
    ) -> Self {
        Self {
            target_id,
            security_origin,
            secure_context_type,
        }
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn security_origin(&self) -> &str {
        &self.security_origin
    }

    pub(crate) fn secure_context_type(&self) -> &str {
        &self.secure_context_type
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetInitialEmptyDocumentState {
    target_id: String,
    loader_id: String,
    initial_url: String,
    creator: Option<TargetInitialEmptyDocumentCreator>,
    storage_key: Option<moli_storage_key::MoliStorageKey>,
    materialized: bool,
    exited: bool,
    pending_cross_document_navigation: bool,
}

impl TargetInitialEmptyDocumentState {
    fn new(
        target_id: String,
        initial_url: String,
        creator: Option<TargetInitialEmptyDocumentCreator>,
        storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) -> Self {
        let loader_id = format!("LID-INITIAL-{target_id}");
        Self {
            target_id,
            loader_id,
            initial_url,
            creator,
            storage_key,
            materialized: false,
            exited: false,
            pending_cross_document_navigation: false,
        }
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn initial_url(&self) -> &str {
        &self.initial_url
    }

    pub(crate) fn loader_id(&self) -> &str {
        &self.loader_id
    }

    pub(crate) fn creator(&self) -> Option<&TargetInitialEmptyDocumentCreator> {
        self.creator.as_ref()
    }

    pub(crate) fn storage_key(&self) -> Option<&moli_storage_key::MoliStorageKey> {
        self.storage_key.as_ref()
    }

    pub(crate) fn materialized(&self) -> bool {
        self.materialized
    }

    pub(crate) fn exited(&self) -> bool {
        self.exited
    }

    pub(crate) fn pending_cross_document_navigation(&self) -> bool {
        self.pending_cross_document_navigation
    }

    pub(crate) fn is_on_initial_empty_document(&self) -> bool {
        !self.exited
    }

    fn mark_materialized(&mut self) {
        if !self.exited {
            self.materialized = true;
        }
    }

    fn mark_pending_cross_document_navigation(&mut self) {
        if !self.exited {
            self.pending_cross_document_navigation = true;
        }
    }

    fn clear_pending_cross_document_navigation(&mut self) {
        self.pending_cross_document_navigation = false;
    }

    fn mark_exited(&mut self) {
        self.exited = true;
        self.pending_cross_document_navigation = false;
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TargetOwnerState {
    pub(crate) initial_empty_document: Option<TargetInitialEmptyDocumentState>,
    pub(crate) committed_document_title: Option<String>,
    pub(crate) next_document_start_script_id: u32,
    pub(crate) document_start_scripts: Vec<(String, DocumentStartScript)>,
    pub(crate) isolated_worlds: Vec<IsolatedWorldDefinition>,
    pub(crate) navigation_history_state: TargetNavigationHistoryState,
    pub(crate) page_resource_store: TargetPageResourceStore,
    pub(crate) runtime_observable_state: TargetRuntimeObservableState,
    pub(crate) console_output_state: TargetConsoleOutputState,
    pub(crate) audits_storage_state: TargetAuditsStorageState,
    pub(crate) log_storage_state: TargetLogStorageState,
    pub(crate) target_crash_state: TargetCrashState,
    pub(crate) window_surface_state: TargetWindowSurfaceState,
    pub(crate) window_surface_geometry: TargetWindowSurfaceGeometry,
    pub(crate) attached_child_frame_ids: HashSet<String>,
}

impl TargetOwnerState {
    pub(crate) fn committed_document_title(&self) -> Option<&str> {
        self.committed_document_title.as_deref()
    }

    pub(crate) fn commit_document_title(&mut self, title: String) -> bool {
        let changed = self.committed_document_title.as_deref().unwrap_or_default() != title;
        self.committed_document_title = Some(title.clone());
        self.navigation_history_state
            .refresh_current_entry_title(title);
        changed
    }

    pub(crate) fn has_bidi_channel_preload_script(&self) -> bool {
        self.document_start_scripts
            .iter()
            .any(|(_, script)| script.has_bidi_channel_argument)
    }

    pub(crate) fn begin_initial_empty_document(
        &mut self,
        target_id: String,
        initial_url: String,
        creator: Option<TargetInitialEmptyDocumentCreator>,
        storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) {
        if !is_initial_empty_document_url(&initial_url) {
            self.initial_empty_document = None;
            return;
        }
        if self.navigation_history_state.is_empty() {
            let entry = PageNavigationHistoryEntry {
                id: self.navigation_history_state.allocate_entry_id(),
                url: initial_url.clone(),
                user_typed_url: initial_url.clone(),
                title: String::new(),
                transition_type: "auto_toplevel".to_owned(),
                document_sequence_number: None,
            };
            self.navigation_history_state.seed_entry(entry);
        }
        self.initial_empty_document = Some(TargetInitialEmptyDocumentState::new(
            target_id,
            initial_url,
            creator,
            storage_key,
        ));
    }

    pub(crate) fn mark_initial_empty_document_materialized(&mut self) {
        if let Some(state) = self.initial_empty_document.as_mut() {
            state.mark_materialized();
        }
    }

    pub(crate) fn mark_initial_empty_document_pending_cross_document_navigation(&mut self) {
        if let Some(state) = self.initial_empty_document.as_mut() {
            state.mark_pending_cross_document_navigation();
        }
    }

    pub(crate) fn clear_initial_empty_document_pending_cross_document_navigation(&mut self) {
        if let Some(state) = self.initial_empty_document.as_mut() {
            state.clear_pending_cross_document_navigation();
        }
    }

    pub(crate) fn mark_initial_empty_document_exited(&mut self) {
        if let Some(state) = self.initial_empty_document.as_mut() {
            state.mark_exited();
        }
    }

    pub(crate) fn initial_empty_document_state(&self) -> Option<&TargetInitialEmptyDocumentState> {
        self.initial_empty_document.as_ref()
    }

    pub(crate) fn initial_empty_document_url_if_current(&self) -> Option<&str> {
        self.initial_empty_document_state()
            .filter(|state| state.is_on_initial_empty_document())
            .map(TargetInitialEmptyDocumentState::initial_url)
    }

    pub(crate) fn initial_empty_document_loader_id_if_current(&self) -> Option<&str> {
        self.initial_empty_document_state()
            .filter(|state| state.is_on_initial_empty_document())
            .map(TargetInitialEmptyDocumentState::loader_id)
    }

    pub(crate) fn initial_empty_document_storage_key_if_current(
        &self,
    ) -> Option<&moli_storage_key::MoliStorageKey> {
        self.initial_empty_document_state()
            .filter(|state| state.is_on_initial_empty_document())
            .and_then(TargetInitialEmptyDocumentState::storage_key)
    }

    pub(crate) fn is_on_initial_empty_document(&self) -> Option<bool> {
        self.initial_empty_document_state()
            .map(TargetInitialEmptyDocumentState::is_on_initial_empty_document)
    }

    pub(crate) fn can_install_current_initial_empty_document_page(&self) -> bool {
        self.initial_empty_document_state()
            .is_none_or(TargetInitialEmptyDocumentState::is_on_initial_empty_document)
    }

    pub(crate) fn initial_empty_document_pending_cross_document_navigation(&self) -> bool {
        self.initial_empty_document_state()
            .is_some_and(TargetInitialEmptyDocumentState::pending_cross_document_navigation)
    }

    pub(crate) fn has_materialized_current_initial_empty_document(&self) -> bool {
        self.initial_empty_document_state()
            .is_some_and(|state| state.is_on_initial_empty_document() && state.materialized())
    }

    pub(crate) fn moli_memory_diagnostics(&self) -> Value {
        let initial_empty_document = self.initial_empty_document.as_ref().map(|state| {
            let creator = state.creator().map(|creator| {
                json!({
                    "targetId": creator.target_id(),
                    "securityOrigin": creator.security_origin(),
                    "secureContextType": creator.secure_context_type(),
                })
            });
            json!({
                "targetId": state.target_id(),
                "initialUrl": state.initial_url(),
                "creator": creator,
                "materialized": state.materialized(),
                "exited": state.exited(),
                "pendingCrossDocumentNavigation": state.pending_cross_document_navigation(),
                "isOnInitialEmptyDocument": state.is_on_initial_empty_document(),
            })
        });
        json!({
            "initialEmptyDocument": initial_empty_document,
            "documentStartScriptCount": self.document_start_scripts.len(),
            "isolatedWorldCount": self.isolated_worlds.len(),
            "retainedPageResourceBodyBytes": self.page_resource_store.retained_body_bytes(),
            "windowSurfaceState": self.window_surface_state.label(),
            "attachedChildFrameIdCount": self.attached_child_frame_ids.len(),
            "targetCrashed": self.target_crash_state.is_crashed(),
            "isDefault": self.is_default(),
        })
    }

    fn navigation_history_entry_for_page_snapshot(
        &mut self,
        page_snapshot: (String, String),
    ) -> PageNavigationHistoryEntry {
        let (url, title) = page_snapshot;
        PageNavigationHistoryEntry {
            id: self.navigation_history_state.allocate_entry_id(),
            user_typed_url: url.clone(),
            url,
            title,
            transition_type: "typed".to_owned(),
            document_sequence_number: None,
        }
    }

    fn reconcile_navigation_history_page_snapshot(
        &mut self,
        page_snapshot: Option<(String, String)>,
    ) {
        let Some(mut page_snapshot) = page_snapshot else {
            return;
        };
        if let Some(title) = self.committed_document_title() {
            page_snapshot.1 = title.to_owned();
        }
        if !self.navigation_history_state.is_empty() {
            let (_, title) = page_snapshot;
            self.navigation_history_state
                .refresh_current_entry_title(title);
            return;
        }
        let entry = self.navigation_history_entry_for_page_snapshot(page_snapshot);
        self.navigation_history_state.seed_entry(entry);
    }

    pub(crate) fn refresh_current_navigation_history_title(&mut self, title: String) -> bool {
        self.navigation_history_state
            .refresh_current_entry_title(title)
    }

    pub(crate) fn mark_next_navigation_history_replace_current(&mut self) {
        self.navigation_history_state.mark_replace_current();
    }

    pub(crate) fn mark_next_navigation_history_replace_initial_empty_document(&mut self) {
        self.navigation_history_state
            .mark_replace_initial_empty_document();
    }

    pub(crate) fn mark_next_navigation_history_traverse_to_entry(&mut self, entry_id: i32) {
        self.navigation_history_state
            .mark_traverse_to_entry(entry_id);
    }

    pub(crate) fn clear_pending_navigation_history_update(&mut self) {
        self.navigation_history_state.clear_pending_update();
    }

    pub(crate) fn resolve_no_commit_response_navigation_history(&mut self) {
        if self.is_on_initial_empty_document() == Some(true) {
            self.navigation_history_state
                .retain_pending_initial_empty_document_replacement();
        } else {
            self.navigation_history_state.clear_pending_update();
        }
    }

    pub(crate) fn navigation_history_entry_url(
        &mut self,
        page_snapshot: Option<(String, String)>,
        entry_id: i32,
    ) -> Option<String> {
        self.reconcile_navigation_history_page_snapshot(page_snapshot);
        self.navigation_history_state.entry_url(entry_id)
    }

    pub(crate) fn navigation_history_snapshot(
        &mut self,
        page_snapshot: Option<(String, String)>,
    ) -> (usize, Vec<PageNavigationHistoryEntry>) {
        self.reconcile_navigation_history_page_snapshot(page_snapshot);
        self.navigation_history_state.snapshot()
    }

    pub(crate) fn reset_navigation_history(
        &mut self,
        page_snapshot: Option<(String, String)>,
    ) -> bool {
        self.reconcile_navigation_history_page_snapshot(page_snapshot);
        self.navigation_history_state.prune_all_but_current()
    }

    pub(crate) fn can_reset_navigation_history(
        &mut self,
        page_snapshot: Option<(String, String)>,
    ) -> bool {
        self.reconcile_navigation_history_page_snapshot(page_snapshot);
        self.navigation_history_state.can_prune_all_but_current()
    }

    pub(crate) fn record_loaded_page_navigation_history(
        &mut self,
        page_snapshot: (String, String),
    ) {
        let entry = self.navigation_history_entry_for_page_snapshot(page_snapshot);
        self.navigation_history_state.record_loaded_entry(entry);
    }

    pub(crate) fn clear_loaded_document_context_state(&mut self) {
        self.clear_attached_child_frame_ids();
    }

    pub(crate) fn clear_committed_document_navigation_state(&mut self) {
        self.committed_document_title = None;
        self.clear_observable_output_state();
        self.clear_loaded_document_context_state();
    }

    pub(crate) fn record_same_document_navigation_history(
        &mut self,
        page_snapshot: Option<(String, String)>,
        url: String,
        mut title: String,
        history_update: moli_core::page::SameDocumentHistoryUpdate,
    ) {
        self.reconcile_navigation_history_page_snapshot(page_snapshot);
        if let Some(committed_title) = self.committed_document_title() {
            title = committed_title.to_owned();
        }
        let _ =
            self.navigation_history_state
                .record_same_document_update(url, title, history_update);
    }

    pub(crate) fn clear_page_local_state(&mut self) {
        self.next_document_start_script_id = 0;
        self.isolated_worlds.clear();
        self.attached_child_frame_ids.clear();
        self.navigation_history_state.clear();
        self.page_resource_store.clear();
        self.target_crash_state.clear();
    }

    pub(crate) fn clear_observable_output_state(&mut self) {
        self.runtime_observable_state.clear();
        self.console_output_state.clear();
        self.audits_storage_state.reset_for_new_document();
        self.log_storage_state.reset_for_new_document();
    }

    pub(crate) fn set_window_surface_state(&mut self, state: TargetWindowSurfaceState) {
        self.window_surface_state = state;
    }

    pub(crate) fn set_window_surface_geometry(
        &mut self,
        width: Option<u32>,
        height: Option<u32>,
        x: Option<i32>,
        y: Option<i32>,
    ) {
        if let Some(width) = width {
            self.window_surface_geometry.width = width;
        }
        if let Some(height) = height {
            self.window_surface_geometry.height = height;
        }
        if let Some(x) = x {
            self.window_surface_geometry.x = x;
        }
        if let Some(y) = y {
            self.window_surface_geometry.y = y;
        }
    }

    pub(crate) fn window_document_hidden(&self) -> bool {
        self.window_surface_state.document_hidden()
    }

    pub(crate) fn window_fullscreen(&self) -> bool {
        self.window_surface_state.is_fullscreen()
    }

    pub(crate) fn is_default(&self) -> bool {
        self.initial_empty_document.is_none()
            && self.committed_document_title.is_none()
            && self.next_document_start_script_id == 0
            && self.document_start_scripts.is_empty()
            && self.isolated_worlds.is_empty()
            && self.navigation_history_state == TargetNavigationHistoryState::default()
            && self.page_resource_store.is_empty()
            && self.runtime_observable_state == TargetRuntimeObservableState::default()
            && self.console_output_state == TargetConsoleOutputState::default()
            && self.log_storage_state.is_empty()
            && !self.target_crash_state.is_crashed()
            && self.window_surface_state == TargetWindowSurfaceState::default()
            && self.window_surface_geometry == TargetWindowSurfaceGeometry::default()
            && self.attached_child_frame_ids.is_empty()
    }

    pub(crate) fn insert_attached_child_frame_id(&mut self, frame_id: String) -> bool {
        self.attached_child_frame_ids.insert(frame_id)
    }

    pub(crate) fn has_attached_child_frame_id(&self, frame_id: &str) -> bool {
        self.attached_child_frame_ids.contains(frame_id)
    }

    pub(crate) fn remove_attached_child_frame_id(&mut self, frame_id: &str) -> bool {
        self.attached_child_frame_ids.remove(frame_id)
    }

    pub(crate) fn clear_attached_child_frame_ids(&mut self) {
        self.attached_child_frame_ids.clear();
    }
}

pub(crate) type ParkedTargetOwnerState = TargetOwnerState;

#[derive(Debug)]
pub(crate) struct ParkedTargetAuxState {
    pub(crate) cookie_manager_surface:
        cookie_manager_surface::BrowserContextCookieManagerSurfaceSnapshot,
    pub(crate) page_session_state: ParkedPageSessionState,
    pub(crate) fetch_state: ParkedFetchState,
    pub(crate) network_artifacts: ParkedNetworkArtifacts,
    pub(crate) target_owner_state: ParkedTargetOwnerState,
}

impl ParkedTargetAuxState {
    pub(crate) fn has_pending_javascript_dialog(&self) -> bool {
        self.page_session_state.has_pending_javascript_dialog()
    }
}

#[derive(Debug, Default)]
pub(crate) struct TargetParkingStateStore {
    cookie_manager_surfaces:
        HashMap<String, cookie_manager_surface::BrowserContextCookieManagerSurfaceSnapshot>,
    page_session_states: HashMap<String, ParkedPageSessionState>,
    fetch_states: HashMap<String, ParkedFetchState>,
    network_artifacts: HashMap<String, ParkedNetworkArtifacts>,
    target_owner_states: HashMap<String, ParkedTargetOwnerState>,
}

impl TargetParkingStateStore {
    pub(crate) fn moli_memory_diagnostics(&self) -> Value {
        let owner_state_summaries = self
            .target_owner_states
            .iter()
            .map(|(target_id, state)| {
                json!({
                    "targetId": target_id,
                    "ownerState": state.moli_memory_diagnostics(),
                })
            })
            .collect::<Vec<_>>();
        json!({
            "cookieManagerSurfaceCount": self.cookie_manager_surfaces.len(),
            "pageSessionStateCount": self.page_session_states.len(),
            "fetchStateCount": self.fetch_states.len(),
            "networkArtifactCount": self.network_artifacts.len(),
            "targetOwnerStateCount": self.target_owner_states.len(),
            "targetOwnerStateWithPendingInspectorAwaitCount": self
                .pending_inspector_await_target_count(),
            "pendingInspectorAwaitCount": self.pending_inspector_await_count(),
            "nonEmptyFetchStateCount": self
                .fetch_states
                .values()
                .filter(|state| !state.is_empty())
                .count(),
            "ownerStates": owner_state_summaries,
        })
    }

    pub(crate) fn mutate_target_owner_state<T>(
        &mut self,
        target_id: &str,
        mutate: impl FnOnce(&mut ParkedTargetOwnerState) -> T,
    ) -> T {
        let mut state = self.take_target_owner_state(target_id);
        let result = mutate(&mut state);
        self.replace_target_owner_state(target_id.to_owned(), state);
        result
    }

    pub(crate) fn take_isolated_worlds(&mut self, target_id: &str) -> Vec<IsolatedWorldDefinition> {
        self.mutate_target_owner_state(target_id, |state| {
            std::mem::take(&mut state.isolated_worlds)
        })
    }

    pub(crate) fn replace_isolated_worlds(
        &mut self,
        target_id: String,
        isolated_worlds: Vec<IsolatedWorldDefinition>,
    ) {
        self.mutate_target_owner_state(&target_id, |state| {
            state.isolated_worlds = isolated_worlds;
        });
    }

    pub(crate) fn take_document_start_script_counter(&mut self, target_id: &str) -> u32 {
        self.mutate_target_owner_state(target_id, |state| {
            let counter = state.next_document_start_script_id;
            state.next_document_start_script_id = 0;
            counter
        })
    }

    pub(crate) fn replace_document_start_script_counter(
        &mut self,
        target_id: String,
        counter: u32,
    ) {
        self.mutate_target_owner_state(&target_id, |state| {
            state.next_document_start_script_id = counter;
        });
    }

    pub(crate) fn take_cookie_manager_surface(
        &mut self,
        target_id: &str,
    ) -> cookie_manager_surface::BrowserContextCookieManagerSurfaceSnapshot {
        self.cookie_manager_surfaces
            .remove(target_id)
            .unwrap_or_default()
    }

    pub(crate) fn replace_cookie_manager_surface(
        &mut self,
        target_id: String,
        snapshot: cookie_manager_surface::BrowserContextCookieManagerSurfaceSnapshot,
    ) {
        if snapshot == cookie_manager_surface::BrowserContextCookieManagerSurfaceSnapshot::default()
        {
            self.cookie_manager_surfaces.remove(&target_id);
            return;
        }
        self.cookie_manager_surfaces.insert(target_id, snapshot);
    }

    pub(crate) fn page_session_state(&self, target_id: &str) -> Option<&ParkedPageSessionState> {
        self.page_session_states.get(target_id)
    }

    pub(crate) fn has_page_domain_enabled_session(&self) -> bool {
        self.page_session_states
            .values()
            .any(parked_page_session_has_page_domain_enabled)
    }

    pub(crate) fn take_page_session_state(&mut self, target_id: &str) -> ParkedPageSessionState {
        self.page_session_states
            .remove(target_id)
            .unwrap_or_default()
    }

    pub(crate) fn replace_page_session_state(
        &mut self,
        target_id: String,
        state: ParkedPageSessionState,
    ) {
        if state == ParkedPageSessionState::default() {
            self.page_session_states.remove(&target_id);
            return;
        }
        self.page_session_states.insert(target_id, state);
    }

    pub(crate) fn take_fetch_state(&mut self, target_id: &str) -> ParkedFetchState {
        self.fetch_states.remove(target_id).unwrap_or_default()
    }

    pub(crate) fn fetch_state(&self, target_id: &str) -> Option<&ParkedFetchState> {
        self.fetch_states.get(target_id)
    }

    pub(crate) fn replace_fetch_state(&mut self, target_id: String, state: ParkedFetchState) {
        if state.is_empty() {
            self.fetch_states.remove(&target_id);
            return;
        }
        self.fetch_states.insert(target_id, state);
    }

    #[cfg(test)]
    pub(crate) fn has_non_empty_fetch_state(&self, target_id: &str) -> bool {
        self.fetch_states
            .get(target_id)
            .is_some_and(|state| !state.is_empty())
    }

    pub(crate) fn take_network_artifacts(&mut self, target_id: &str) -> ParkedNetworkArtifacts {
        self.network_artifacts.remove(target_id).unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn network_artifacts(&self, target_id: &str) -> Option<&ParkedNetworkArtifacts> {
        self.network_artifacts.get(target_id)
    }

    pub(crate) fn replace_network_artifacts(
        &mut self,
        target_id: String,
        artifacts: ParkedNetworkArtifacts,
    ) {
        if artifacts == ParkedNetworkArtifacts::default() {
            self.network_artifacts.remove(&target_id);
            return;
        }
        self.network_artifacts.insert(target_id, artifacts);
    }

    pub(crate) fn take_target_owner_state(&mut self, target_id: &str) -> ParkedTargetOwnerState {
        self.target_owner_states
            .remove(target_id)
            .unwrap_or_default()
    }

    pub(crate) fn target_owner_state(&self, target_id: &str) -> Option<&ParkedTargetOwnerState> {
        self.target_owner_states.get(target_id)
    }

    pub(crate) fn replace_target_owner_state(
        &mut self,
        target_id: String,
        state: ParkedTargetOwnerState,
    ) {
        if state.is_default() {
            self.target_owner_states.remove(&target_id);
            return;
        }
        self.target_owner_states.insert(target_id, state);
    }

    pub(crate) fn has_pending_javascript_dialog(&self) -> bool {
        self.page_session_states.values().any(|state| {
            !state
                .devtools_session_state
                .page_session_state
                .javascript_dialog_state
                .is_empty()
                || state
                    .auxiliary_devtools_session_states
                    .values()
                    .any(|session| {
                        !session
                            .page_session_state
                            .javascript_dialog_state
                            .is_empty()
                    })
        })
    }

    pub(crate) fn has_pending_inspector_awaits(&self) -> bool {
        self.page_session_states
            .values()
            .any(parked_page_session_has_pending_inspector_awaits)
    }

    pub(crate) fn pending_inspector_await_count(&self) -> usize {
        self.page_session_states
            .values()
            .map(parked_page_session_pending_inspector_await_count)
            .sum()
    }

    pub(crate) fn pending_inspector_await_target_count(&self) -> usize {
        self.page_session_states
            .values()
            .filter(|state| parked_page_session_has_pending_inspector_awaits(state))
            .count()
    }

    pub(crate) fn has_runtime_remote_object_id(&self, object_id: &str) -> bool {
        self.page_session_states
            .values()
            .any(|state| parked_page_session_has_runtime_remote_object_id(state, object_id))
    }

    pub(crate) fn runtime_remote_object_id_known_for_different_page_owner(
        &self,
        target_id: &str,
        devtools_session_id: Option<&str>,
        object_id: &str,
    ) -> bool {
        self.page_session_states.iter().any(|(id, state)| {
            if id == target_id {
                return parked_page_session_has_runtime_remote_object_id_for_different_session(
                    state,
                    devtools_session_id,
                    object_id,
                );
            }
            parked_page_session_has_runtime_remote_object_id(state, object_id)
        })
    }
}

fn is_initial_empty_document_url(raw_url: &str) -> bool {
    url::Url::parse(raw_url)
        .ok()
        .as_ref()
        .is_some_and(moli_url::is_about_blank)
}
