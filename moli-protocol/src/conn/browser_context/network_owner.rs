use super::target_session_owner::{TargetSessionOwnerMut, TargetSessionOwnerRef};
use super::*;
use crate::conn::CdpSessionRoute;
use crate::conn::{CapturedBody, TargetRuntimeSlot};
use crate::conn::{
    DocumentPolicyUpdate, PendingDocumentPolicyUpdate, PendingDocumentResourceRuntimeUpdate,
};
use crate::devtools_runtime::DevToolsNetworkDataType;
use crate::domains::network::{
    CapturedRequestBody, CapturedResponseBody, CollectedNetworkDataArtifact,
    NetworkBacklogPreferredRequestId, PendingNetworkBacklogDeliverySnapshot,
    TargetNetworkBacklogPreparedDelivery,
};
use moli_page_types::DevToolsSessionKey;

struct TargetNetworkListenerOwnerMut<'a> {
    target: &'a mut crate::conn::PageAgentHost,
    session_key: DevToolsSessionKey,
}

impl<'a> TargetSessionOwnerMut<'a> {
    fn into_network_listener_owner(self) -> TargetNetworkListenerOwnerMut<'a> {
        let target = self
            .browser_context
            .page_target_mut(&self.target_id)
            .expect("resolved Page target owner must remain live");
        TargetNetworkListenerOwnerMut {
            target,
            session_key: self.session_key,
        }
    }

    fn enable_listener(mut self) -> bool {
        self.mutate_network_policy_session_state(|state| state.network_enabled = true);
        self.into_network_listener_owner().enable_listener()
    }

    fn disable_listener(mut self) -> bool {
        self.mutate_network_policy_session_state(|state| *state = Default::default());
        self.into_network_listener_owner().disable_listener()
    }
}

impl TargetSessionOwnerRef<'_> {
    fn network_listener_enabled(&self) -> bool {
        let Some(target) = self.browser_context.page_target(&self.target_id) else {
            return false;
        };
        match &self.session_key {
            DevToolsSessionKey::Primary => target.runtime_slot.primary_network_events_enabled(),
            DevToolsSessionKey::Attached(session_id) => target
                .runtime_slot
                .attached_network_events_enabled_for_session(session_id),
        }
    }
}

impl TargetNetworkListenerOwnerMut<'_> {
    fn network_listener_enabled(&self) -> bool {
        match &self.session_key {
            DevToolsSessionKey::Primary => {
                self.target.runtime_slot.primary_network_events_enabled()
            }
            DevToolsSessionKey::Attached(session_id) => self
                .target
                .runtime_slot()
                .attached_network_events_enabled_for_session(session_id),
        }
    }

    fn set_primary_network_enabled(&mut self, enabled: bool) {
        self.target
            .runtime_slot
            .set_primary_network_events_enabled(enabled);
    }

    fn initialize_network_observation_cursor_at_current_tail(&mut self, session_id: Option<&str>) {
        self.target
            .runtime_slot
            .initialize_network_session_observation_cursor_at_output_tail(session_id);
    }

    fn remove_network_observation_cursor(&mut self, session_id: Option<&str>) {
        self.target
            .runtime_slot
            .remove_network_session_observation_cursor(session_id);
    }

    fn remove_captured_response_body_visibility_for_session(&mut self, session_id: Option<&str>) {
        self.target
            .runtime_slot
            .remove_captured_response_body_visibility_for_session(session_id);
    }

    fn clear_network_observation_artifacts_if_unobserved(&mut self) {
        if !self.target.runtime_slot.has_network_event_listeners() {
            self.target.runtime_slot.clear_captured_response_bodies();
            self.target.runtime_slot.clear_websocket_request_ids();
        }
    }

    fn listener_session_id(&self) -> Option<&str> {
        self.session_key.wire_session_id()
    }

    fn is_attached_session(&self) -> bool {
        matches!(self.session_key, DevToolsSessionKey::Attached(_))
    }

    fn enable_listener(mut self) -> bool {
        let adding_network_event_listener = !self.network_listener_enabled();
        let listener_session_id = self.listener_session_id().map(str::to_owned);
        if self.is_attached_session() {
            if let Some(session_id) = listener_session_id.as_deref() {
                self.runtime_slot_mut()
                    .enable_attached_network_events(session_id);
            }
        } else {
            self.set_primary_network_enabled(true);
        }
        if adding_network_event_listener {
            self.initialize_network_observation_cursor_at_current_tail(
                listener_session_id.as_deref(),
            );
        }
        true
    }

    fn disable_listener(mut self) -> bool {
        let listener_session_id = self.listener_session_id().map(str::to_owned);
        if self.is_attached_session() {
            if let Some(session_id) = listener_session_id.as_deref() {
                self.runtime_slot_mut()
                    .disable_attached_network_events(session_id);
            }
        } else {
            self.set_primary_network_enabled(false);
        }
        self.remove_network_observation_cursor(listener_session_id.as_deref());
        self.remove_captured_response_body_visibility_for_session(listener_session_id.as_deref());
        self.clear_network_observation_artifacts_if_unobserved();
        true
    }

    fn runtime_slot_mut(&mut self) -> &mut TargetRuntimeSlot {
        &mut self.target.runtime_slot
    }
}

impl TargetSessionOwnerMut<'_> {
    fn mutate_network_policy_session_state<T>(
        &mut self,
        f: impl FnOnce(&mut crate::conn::state::DevToolsNetworkSessionState) -> T,
    ) -> T {
        {
            self.browser_context
                .mutate_devtools_network_session_state_for_target(
                    &self.target_id,
                    &self.session_key,
                    f,
                )
        }
    }

    fn start_set_cache_disabled(
        mut self,
        cache_disabled: bool,
        global_extra_headers: &[(String, String)],
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        self.mutate_network_policy_session_state(|state| {
            state.cache_disabled = cache_disabled;
        });
        self.start_replay_effective_network_request_policy(global_extra_headers)
    }

    fn start_set_bypass_service_worker(
        mut self,
        bypass_service_worker: bool,
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        self.mutate_network_policy_session_state(|state| {
            state.bypass_service_worker = bypass_service_worker;
        });
        let effective = self
            .browser_context
            .effective_policy_for_target(&self.target_id);
        self.start_document_policy_update(DocumentPolicyUpdate::BypassServiceWorker(
            effective.bypass_service_worker(),
        ))
    }

    fn start_set_blocked_url_patterns(
        mut self,
        blocked_url_patterns: Vec<String>,
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        self.mutate_network_policy_session_state(|state| {
            state.blocked_url_patterns = blocked_url_patterns;
        });
        let effective = self
            .browser_context
            .effective_policy_for_target(&self.target_id);
        self.start_document_policy_update(DocumentPolicyUpdate::BlockedUrls(
            effective.blocked_url_patterns().to_vec(),
        ))
    }

    fn start_set_extra_http_headers(
        mut self,
        extra_headers: Vec<(String, String)>,
        global_extra_headers: &[(String, String)],
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        self.mutate_network_policy_session_state(|state| {
            state.extra_headers = extra_headers;
        });
        self.start_effective_extra_http_headers_update(global_extra_headers)
    }

    fn start_set_target_extra_http_headers(
        mut self,
        extra_headers: Vec<(String, String)>,
        global_extra_headers: &[(String, String)],
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        self.browser_context
            .set_base_extra_headers_for_target(&self.target_id, extra_headers);
        self.start_effective_extra_http_headers_update(global_extra_headers)
    }

    fn start_effective_extra_http_headers_update(
        &mut self,
        global_extra_headers: &[(String, String)],
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        let headers = self
            .browser_context
            .effective_extra_headers_for_target(&self.target_id, global_extra_headers);
        let Some(document) = self
            .browser_context
            .document_handle_for_target(&self.target_id)
        else {
            return Ok(None);
        };
        self.browser_context
            .start_document_policy_update(document, DocumentPolicyUpdate::ExtraHttpHeaders(headers))
            .map(Some)
    }

    fn start_replay_effective_network_request_policy(
        &mut self,
        global_extra_headers: &[(String, String)],
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        let effective = self
            .browser_context
            .effective_policy_for_target(&self.target_id);
        let headers = self.browser_context.merged_extra_headers_for_target_policy(
            global_extra_headers,
            effective.extra_headers(),
        );
        self.start_document_policy_update(DocumentPolicyUpdate::NetworkRequestPolicy {
            extra_headers: headers,
            bypass_service_worker: effective.bypass_service_worker(),
            cache_disabled: effective.cache_disabled(),
            blocked_url_patterns: effective.blocked_url_patterns().to_vec(),
        })
    }

    fn start_document_policy_update(
        &mut self,
        update: DocumentPolicyUpdate,
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        let Some(document) = self
            .browser_context
            .document_handle_for_target(&self.target_id)
        else {
            return Ok(None);
        };
        self.browser_context
            .start_document_policy_update(document, update)
            .map(Some)
    }

    fn set_devtools_browser_identity_override(
        &mut self,
        browser_identity: Option<crate::conn::DevToolsBrowserIdentityOverride>,
    ) -> bool {
        {
            self.browser_context
                .set_devtools_browser_identity_override_for_target(
                    &self.target_id,
                    &self.session_key,
                    browser_identity,
                );
        };
        true
    }

    fn set_base_user_agent_override(
        &mut self,
        user_agent: Option<String>,
        fallback_identity: &moli_browser_profile::BrowserIdentityProfile,
    ) -> bool {
        {
            self.browser_context
                .set_base_user_agent_override_for_target(
                    &self.target_id,
                    user_agent,
                    fallback_identity,
                );
        };
        true
    }

    fn set_tls_verify_host_override(self, enabled: bool) {
        {
            self.browser_context
                .set_tls_verify_host_override_for_target(&self.target_id, Some(enabled));
        };
    }

    fn start_set_network_offline(
        self,
        offline: bool,
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        let Some(handle) = self
            .browser_context
            .web_contents_handle_for_target(&self.target_id)
        else {
            return Ok(None);
        };
        self.browser_context
            .set_web_contents_network_offline(handle, offline)?;
        let Some(document) = self
            .browser_context
            .document_handle_for_target(&self.target_id)
        else {
            return Ok(None);
        };
        self.browser_context
            .start_document_policy_update(document, DocumentPolicyUpdate::NetworkOffline(offline))
            .map(Some)
    }
}

impl CdpConnection {
    pub(crate) fn captured_response_body_for_bidi_network_data(
        &self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<&CapturedResponseBody> {
        self.browser_contexts().find_map(|browser_context| {
            browser_context.page_targets.iter().find_map(|target| {
                target
                    .runtime_slot()
                    .captured_response_body(request_id)
                    .filter(|body| {
                        body.is_visible_to_session(session_id) || body.is_visible_to_session(None)
                    })
            })
        })
    }

    pub(crate) fn captured_request_body_for_bidi_network_data(
        &self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<&CapturedRequestBody> {
        self.browser_contexts().find_map(|browser_context| {
            browser_context.page_targets.iter().find_map(|target| {
                target
                    .runtime_slot()
                    .captured_request_body(request_id)
                    .filter(|body| {
                        body.is_visible_to_session(session_id) || body.is_visible_to_session(None)
                    })
            })
        })
    }

    pub(crate) fn network_data_collector_ids_for_session_owner_body(
        &self,
        session_id: Option<&str>,
        data_type: DevToolsNetworkDataType,
        encoded_data_size: usize,
    ) -> Vec<String> {
        let owner = crate::conn::CommandOwnerScope::capture(self, session_id);
        self.network_data_collector_ids_for_owner_body(&owner, data_type, encoded_data_size)
    }

    pub(crate) fn network_data_collector_ids_for_owner_body(
        &self,
        owner: &crate::conn::CommandOwnerScope,
        data_type: DevToolsNetworkDataType,
        encoded_data_size: usize,
    ) -> Vec<String> {
        let Some((browser_context_id, target_id)) = self.target_owner_identity_for_owner(owner)
        else {
            return Vec::new();
        };
        self.network_data_collectors
            .collector_ids_for_body(
                data_type,
                encoded_data_size,
                target_id.as_deref(),
                Some(&browser_context_id),
            )
            .into_iter()
            .collect()
    }

    pub(crate) fn network_data_collection_is_gated_for_body(
        &self,
        data_type: DevToolsNetworkDataType,
    ) -> bool {
        self.network_data_collectors
            .has_collector_for_data_type(data_type)
    }

    pub(crate) fn record_collected_network_data_body(
        &mut self,
        request_id: String,
        data_type: DevToolsNetworkDataType,
        body: CapturedBody,
        collector_ids: impl IntoIterator<Item = String>,
        collection_was_gated: bool,
    ) {
        self.network_data_collectors.record_collected_body(
            request_id,
            data_type,
            body,
            collector_ids,
            collection_was_gated,
        );
    }

    pub(crate) fn record_collected_network_data_artifacts(
        &mut self,
        artifacts: impl IntoIterator<Item = CollectedNetworkDataArtifact>,
    ) {
        for artifact in artifacts {
            self.record_collected_network_data_body(
                artifact.request_id,
                artifact.data_type,
                artifact.body,
                artifact.collector_ids,
                artifact.collection_was_gated,
            );
        }
    }

    pub(crate) fn has_network_event_listeners_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        if let Some(session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session(Some(session_id))
        {
            return target.network_enabled(session_id);
        }
        if let Some(session_id) = session_id
            && let Some(target) = self.dedicated_worker_target_for_session(Some(session_id))
        {
            return target.network_enabled(session_id);
        }
        self.runtime_session_owner_slot(session_id)
            .is_ok_and(|runtime_slot| runtime_slot.has_network_event_listeners())
    }

    pub(crate) fn network_backlog_prepared_delivery_for_owner(
        &mut self,
        owner: &crate::conn::CommandOwnerScope,
        trigger_session_id: Option<&str>,
        primary_session_id: Option<&str>,
        preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
    ) -> Option<TargetNetworkBacklogPreparedDelivery> {
        let mut network_request_id_allocator =
            std::mem::take(&mut self.network_request_id_allocator);
        let result = self
            .runtime_session_owner_slot_mut_for_owner(owner)
            .ok()
            .map(|runtime_slot| {
                runtime_slot.network_backlog_prepared_delivery(
                    trigger_session_id,
                    primary_session_id,
                    preferred_request_id,
                    &mut network_request_id_allocator,
                )
            });
        self.network_request_id_allocator = network_request_id_allocator;
        result
    }

    pub(crate) fn network_request_id_for_subresource_handle_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        handle: moli_core::page::SubresourceNetworkRequestHandle,
    ) -> Option<String> {
        let mut network_request_id_allocator =
            std::mem::take(&mut self.network_request_id_allocator);
        let result = self
            .runtime_session_owner_slot_mut(session_id)
            .ok()
            .map(|runtime_slot| {
                runtime_slot.network_request_id_for_subresource_handle(
                    handle,
                    &mut network_request_id_allocator,
                )
            });
        self.network_request_id_allocator = network_request_id_allocator;
        result
    }

    pub(crate) fn pending_network_backlog_delivery_snapshot_for_owner(
        &mut self,
        owner: &crate::conn::CommandOwnerScope,
        trigger_session_id: Option<&str>,
        primary_session_id: Option<&str>,
        preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
    ) -> Option<PendingNetworkBacklogDeliverySnapshot> {
        let mut network_request_id_allocator =
            std::mem::take(&mut self.network_request_id_allocator);
        let result = self
            .runtime_session_owner_slot_mut_for_owner(owner)
            .ok()
            .and_then(|runtime_slot| {
                runtime_slot.pending_network_backlog_delivery_snapshot(
                    trigger_session_id,
                    primary_session_id,
                    preferred_request_id,
                    &mut network_request_id_allocator,
                )
            });
        self.network_request_id_allocator = network_request_id_allocator;
        result
    }

    pub(crate) fn network_event_session_ids_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Vec<Option<String>> {
        let owner = crate::conn::CommandOwnerScope::capture(self, session_id);
        self.network_event_session_ids_for_owner(&owner)
    }

    pub(crate) fn network_event_session_ids_for_owner(
        &self,
        owner: &crate::conn::CommandOwnerScope,
    ) -> Vec<Option<String>> {
        let Ok(runtime_slot) = self.runtime_session_owner_slot_for_owner(owner) else {
            return vec![owner.session_id().map(str::to_owned)];
        };
        let primary_session_id = self.runtime_session_owner_primary_session_id_for_owner(owner);
        runtime_slot.network_event_session_ids(owner.session_id(), primary_session_id.as_deref())
    }

    pub(crate) fn enable_network_listener_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> bool {
        let owner = crate::conn::CommandOwnerScope::capture(self, session_id);
        self.set_network_listener_enabled_for_owner(&owner, true)
    }

    pub(crate) fn network_listener_enabled_for_session_owner(&self, session_id: &str) -> bool {
        self.target_session_owner_ref(Some(session_id))
            .is_some_and(|owner| owner.network_listener_enabled())
    }

    fn set_network_listener_enabled_for_owner(
        &mut self,
        owner: &crate::conn::CommandOwnerScope,
        enabled: bool,
    ) -> bool {
        if let Some(session_id) = owner.session_id()
            && let Some(target) = self.service_worker_target_for_session_mut(Some(session_id))
        {
            return target.set_network_enabled(session_id, enabled);
        }
        if let Some(session_id) = owner.session_id()
            && let Some(target) = self.dedicated_worker_target_for_session_mut(Some(session_id))
        {
            return target.set_network_enabled(session_id, enabled);
        }
        self.with_target_session_owner_mut_for_owner(owner, |resolved| {
            if enabled {
                resolved.enable_listener()
            } else {
                resolved.disable_listener()
            }
        })
        .unwrap_or(false)
    }

    pub fn enable_network_listener_for_target(&mut self, target_id: &str) -> bool {
        let Some(route) = self.target_session_route_for_target_id(target_id) else {
            return false;
        };
        self.set_network_listener_enabled_for_owner(
            &crate::conn::CommandOwnerScope::for_route(route),
            true,
        )
    }

    pub fn disable_network_listener_for_target(&mut self, target_id: &str) -> bool {
        let Some(route) = self.target_session_route_for_target_id(target_id) else {
            return false;
        };
        self.set_network_listener_enabled_for_owner(
            &crate::conn::CommandOwnerScope::for_route(route),
            false,
        )
    }

    pub(crate) fn set_global_cache_disabled(&mut self, cache_disabled: bool) {
        self.browser_global_overrides.cache_disabled = cache_disabled;
        for browser_context in self
            .browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
        {
            browser_context.apply_browser_cache_disabled(cache_disabled);
        }
    }

    pub(crate) fn set_global_extra_headers(&mut self, extra_headers: Vec<(String, String)>) {
        self.browser_global_overrides.extra_headers = extra_headers;
    }

    pub(crate) fn disable_network_listener_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> bool {
        let owner = crate::conn::CommandOwnerScope::capture(self, session_id);
        self.set_network_listener_enabled_for_owner(&owner, false)
    }

    pub(crate) fn start_set_cache_disabled_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        cache_disabled: bool,
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        let global_extra_headers = self.browser_global_overrides.extra_headers.clone();
        let Some(owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        owner.start_set_cache_disabled(cache_disabled, &global_extra_headers)
    }

    pub(crate) fn set_cache_disabled_for_target(
        &mut self,
        target_id: &str,
        cache_disabled: bool,
    ) -> bool {
        for browser_context in self
            .browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
        {
            if browser_context.page_target(target_id).is_some() {
                browser_context.set_base_cache_disabled_for_target(target_id, cache_disabled);
                return true;
            }
        }
        false
    }

    pub(crate) fn start_set_bypass_service_worker_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        bypass_service_worker: bool,
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        let Some(owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        owner.start_set_bypass_service_worker(bypass_service_worker)
    }

    pub(crate) fn start_set_blocked_url_patterns_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        blocked_url_patterns: Vec<String>,
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        let Some(owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        owner.start_set_blocked_url_patterns(blocked_url_patterns)
    }

    pub(crate) fn start_set_extra_http_headers_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        extra_headers: Vec<(String, String)>,
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        let global_extra_headers = self.browser_global_overrides.extra_headers.clone();
        let Some(owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        owner.start_set_extra_http_headers(extra_headers, &global_extra_headers)
    }

    pub(crate) fn start_set_target_extra_http_headers_for_owner(
        &mut self,
        command_owner: &crate::conn::CommandOwnerScope,
        extra_headers: Vec<(String, String)>,
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        let global_extra_headers = self.browser_global_overrides.extra_headers.clone();
        let Some(owner) = self.target_session_owner_mut_for_owner(command_owner) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        owner.start_set_target_extra_http_headers(extra_headers, &global_extra_headers)
    }

    pub(crate) fn start_replay_effective_network_request_policy_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        if matches!(
            self.session_route(session_id),
            Some(
                CdpSessionRoute::DedicatedWorkerTarget { .. }
                    | CdpSessionRoute::SharedWorkerTarget { .. }
                    | CdpSessionRoute::ServiceWorkerTarget { .. }
            )
        ) {
            return Ok(None);
        }
        let global_extra_headers = self.browser_global_overrides.extra_headers.clone();
        let Some(mut owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        owner.start_replay_effective_network_request_policy(&global_extra_headers)
    }

    pub(crate) fn start_set_base_user_agent_override_for_owner(
        &mut self,
        command_owner: &crate::conn::CommandOwnerScope,
        user_agent: Option<String>,
    ) -> Result<Option<PendingDocumentResourceRuntimeUpdate>, String> {
        let browser_identity = user_agent.as_ref().map(|user_agent| {
            moli_browser_profile::BrowserIdentityProfile::new(
                user_agent.clone(),
                self.base_browser_identity.accept_language(),
            )
        });
        if let Some(result) = self.start_set_non_page_browser_identity_override(
            command_owner.session_id(),
            browser_identity,
        ) {
            return result;
        }

        let fallback_identity = self.base_browser_identity.clone();
        let Some(mut owner) = self.target_session_owner_mut_for_owner(command_owner) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        if !owner.set_base_user_agent_override(user_agent, &fallback_identity) {
            return Err("BrowserContextNotLoaded".to_owned());
        }
        self.start_rebuild_resource_runtime_for_owner(command_owner)
    }

    fn start_set_non_page_browser_identity_override(
        &mut self,
        session_id: Option<&str>,
        browser_identity: Option<moli_browser_profile::BrowserIdentityProfile>,
    ) -> Option<Result<Option<PendingDocumentResourceRuntimeUpdate>, String>> {
        let is_browser_session = matches!(
            self.session_route(session_id),
            Some(CdpSessionRoute::Browser)
        );
        let is_pre_context_root = session_id.is_none() && self.browser_context.is_none();
        if !is_browser_session && !is_pre_context_root {
            return None;
        }

        if is_browser_session && let Some(browser_context) = self.browser_context.as_mut() {
            let target_id = browser_context
                .active_target_id()
                .expect("active target")
                .to_owned();
            browser_context
                .set_base_browser_identity_override_for_target(&target_id, browser_identity);
        } else {
            self.global_browser_identity_override = browser_identity;
        }
        self.apply_active_engine_fetch_overrides();
        Some(self.start_rebuild_resource_runtime_for_session_owner(session_id))
    }

    pub(crate) fn start_set_devtools_browser_identity_override_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        browser_identity: Option<crate::conn::DevToolsBrowserIdentityOverride>,
    ) -> Result<Option<PendingDocumentResourceRuntimeUpdate>, String> {
        let non_page_identity = browser_identity
            .as_ref()
            .map(crate::conn::DevToolsBrowserIdentityOverride::to_browser_identity);
        if let Some(result) =
            self.start_set_non_page_browser_identity_override(session_id, non_page_identity)
        {
            return result;
        }

        let Some(owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        let mut owner = owner;
        if !owner.set_devtools_browser_identity_override(browser_identity) {
            return Err("BrowserContextNotLoaded".to_owned());
        }
        self.start_rebuild_resource_runtime_for_session_owner(session_id)
    }

    pub(crate) fn start_set_tls_verify_host_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
    ) -> Result<Option<PendingDocumentResourceRuntimeUpdate>, String> {
        if session_id.is_none()
            || matches!(
                self.session_route(session_id),
                Some(CdpSessionRoute::Browser)
            )
        {
            if let Some(browser_context) = self.browser_context.as_mut() {
                browser_context.set_tls_verify_host_override(enabled);
            } else {
                self.base_tls_verify_host = enabled;
            }
            self.apply_active_engine_fetch_overrides();
            return self.start_rebuild_resource_runtime_for_session_owner(session_id);
        }

        let Some(owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        owner.set_tls_verify_host_override(enabled);
        self.start_rebuild_resource_runtime_for_session_owner(session_id)
    }

    pub(crate) fn start_set_network_offline_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        offline: bool,
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        let owner = crate::conn::CommandOwnerScope::capture(self, session_id);
        self.start_set_network_offline_for_owner(&owner, offline)
    }

    pub(crate) fn start_set_network_offline_for_owner(
        &mut self,
        command_owner: &crate::conn::CommandOwnerScope,
        offline: bool,
    ) -> Result<Option<PendingDocumentPolicyUpdate>, String> {
        let Some(owner) = self.target_session_owner_mut_for_owner(command_owner) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        owner.start_set_network_offline(offline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection_with_background_attached_session() -> CdpConnection {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-background".to_owned());
        browser_context.register_page_target_url_fixture(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "https://background.example/".to_owned(),
        );
        assert!(
            browser_context
                .assign_attached_session_to_target("TID-background", "SID-attached".to_owned())
        );
        conn.install_browser_context_fixture_for_test(browser_context);
        conn
    }

    #[test]
    fn browser_globals_apply_to_context_inserted_after_configuration() {
        let mut conn = CdpConnection::default();
        conn.set_global_extra_headers(vec![("X-Browser".into(), "global".into())]);
        conn.set_global_network_conditions(Some(crate::conn::EmulatedNetworkConditions::offline()));
        conn.set_global_geolocation_override(Some(
            crate::conn::EmulatedGeolocationOverrideState::Position(
                crate::conn::EmulatedGeolocationOverride {
                    latitude: 12.5,
                    longitude: 34.5,
                    accuracy: 1.0,
                    altitude: None,
                    altitude_accuracy: None,
                    heading: None,
                    speed: None,
                },
            ),
        ));
        conn.set_global_cache_disabled(true);

        let mut browser_context = BrowserContext::new("BID-late".to_owned());
        browser_context.set_active_target_id("TID-late");
        browser_context.attach_active_session("SID-late");
        conn.insert_browser_context(browser_context);

        let inputs = conn.navigation_load_inputs_for_session_owner(Some("SID-late"));
        assert_eq!(
            inputs.extra_http_headers,
            vec![("X-Browser".into(), "global".into())]
        );
        assert!(inputs.network_offline);
        // This fixture inserts the Context without registering its wire session.
        // Resolve the exact Page route for the native policy, just as the
        // former inspection-preload assertion did.
        let inputs = conn.navigation_load_inputs_for_owner(
            &crate::conn::CommandOwnerScope::for_route(CdpSessionRoute::PageTarget {
                browser_context_id: "BID-late".into(),
                target_id: "TID-late".into(),
                session_key: DevToolsSessionKey::Primary,
            }),
        );
        assert_eq!(inputs.navigator_overrides.online, Some(false));
        assert_eq!(
            inputs.navigator_overrides.geolocation,
            Some(moli_page_types::GeolocationPositionOverride {
                latitude: 12.5,
                longitude: 34.5,
                accuracy: 1.0,
                altitude: None,
                altitude_accuracy: None,
                heading: None,
                speed: None,
            })
        );
        assert!(
            conn.browser_context
                .as_ref()
                .unwrap()
                .effective_policy_for_target("TID-late")
                .cache_disabled()
        );
    }

    #[tokio::test]
    async fn tls_policy_is_shared_by_page_sessions_and_primary_cleanup_reveals_context_default() {
        let mut conn = connection_with_background_attached_session();
        conn.browser_context
            .as_mut()
            .unwrap()
            .set_tls_verify_host_override(true);
        for (session, enabled) in [
            ("SID-attached", false),
            ("SID-background", true),
            ("SID-attached", false),
        ] {
            assert!(
                conn.start_set_tls_verify_host_for_session_owner(Some(session), enabled)
                    .unwrap()
                    .is_none()
            );
            for observer in ["SID-background", "SID-attached"] {
                let inputs = conn.navigation_load_inputs_for_session_owner(Some(observer));
                assert_eq!(inputs.tls_verify_host_override, Some(enabled));
                assert_eq!(
                    conn.ensure_resource_request_client_for_navigation_load_inputs(&inputs)
                        .unwrap()
                        .tls_verify_host(),
                    enabled
                );
            }
        }
        for (session, installed, effective) in [
            ("SID-attached", Some(false), false),
            ("SID-background", None, true),
        ] {
            conn.reset_primary_page_session_target_state_async(
                "BID-background",
                "TID-background",
                session,
            )
            .await
            .unwrap();
            let context = conn.browser_context.as_mut().unwrap();
            let target = context.page_target("TID-background").unwrap();
            assert!(target.is_session("SID-background"));
            assert_eq!(
                context.tls_verify_host_override_for_target(target.target_id()),
                installed
            );
            assert_eq!(context.network_policy().tls_verify_host, Some(true));
            let inputs = conn.navigation_load_inputs_for_session_owner(Some("SID-background"));
            assert_eq!(inputs.tls_verify_host_override, Some(effective));
            assert_eq!(
                conn.ensure_resource_request_client_for_navigation_load_inputs(&inputs)
                    .unwrap()
                    .tls_verify_host(),
                effective
            );
        }
    }

    #[test]
    fn network_offline_updates_are_shared_by_sessions_and_navigation_keeps_emulation_defaults() {
        let mut conn = connection_with_background_attached_session();
        let primary = crate::conn::CommandOwnerScope::for_session("SID-background");
        let attached = crate::conn::CommandOwnerScope::for_session("SID-attached");
        for (session, offline) in [
            ("SID-attached", true),
            ("SID-background", false),
            ("SID-attached", true),
        ] {
            assert!(
                conn.start_set_network_offline_for_session_owner(Some(session), offline)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                conn.navigation_load_inputs_for_owner(&primary)
                    .network_offline,
                offline
            );
            assert_eq!(
                conn.navigation_load_inputs_for_owner(&attached)
                    .network_offline,
                offline
            );
        }
        let context = conn.browser_context.as_mut().unwrap();
        context.clear_devtools_network_state_for_target(
            "TID-background",
            &DevToolsSessionKey::Attached("SID-attached".into()),
        );
        assert!(
            context.network_offline_for_target("TID-background"),
            "Network.disable does not reset traffic state"
        );
        context.reset_primary_session_target_state_fields_for_target("TID-background");
        assert!(!context.network_offline_for_target("TID-background"));
        context.set_default_network_conditions(Some(
            crate::conn::EmulatedNetworkConditions::offline(),
        ));
        assert!(
            conn.navigation_load_inputs_for_owner(&primary)
                .network_offline
        );
        let context = conn.browser_context.as_mut().unwrap();
        context.set_default_network_conditions(None);
        context.apply_target_emulation_policy_change(
            "TID-background",
            crate::conn::EmulationPolicyChange::NetworkConditions(Some(
                crate::conn::EmulatedNetworkConditions::offline(),
            )),
        );
        assert!(
            conn.navigation_load_inputs_for_owner(&primary)
                .network_offline
        );
        conn.browser_context
            .as_mut()
            .unwrap()
            .apply_target_emulation_policy_change(
                "TID-background",
                crate::conn::EmulationPolicyChange::NetworkConditions(None),
            );
        assert!(
            !conn
                .navigation_load_inputs_for_owner(&primary)
                .network_offline
        );
    }

    #[tokio::test]
    async fn only_primary_session_cleanup_resets_installed_network_offline() {
        let mut conn = connection_with_background_attached_session();
        conn.start_set_network_offline_for_session_owner(Some("SID-attached"), true)
            .unwrap();
        conn.reset_primary_page_session_target_state_async(
            "BID-background",
            "TID-background",
            "SID-attached",
        )
        .await
        .unwrap();
        let context = conn.browser_context.as_ref().unwrap();
        assert!(context.network_offline_for_target("TID-background"));
        conn.reset_primary_page_session_target_state_async(
            "BID-background",
            "TID-background",
            "SID-background",
        )
        .await
        .unwrap();
        let context = conn.browser_context.as_ref().unwrap();
        assert!(!context.network_offline_for_target("TID-background"));
    }

    #[test]
    fn subresource_fetch_network_request_ids_are_connection_global_across_target_owners() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-mixed".to_owned());
        browser_context.set_active_target_id("TID-active".to_owned());
        browser_context.attach_active_session("SID-active".to_owned());
        browser_context.register_page_target_url_fixture(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "https://background.example/".to_owned(),
        );
        conn.install_browser_context_fixture_for_test(browser_context);

        let (active_fetch_id, active_network_request_id) = conn
            .allocate_pending_subresource_fetch_request_ids_for_owner(
                &crate::conn::CommandOwnerScope::for_session("SID-active"),
            )
            .expect("active owner should allocate request ids");
        let (background_fetch_id, background_network_request_id) = conn
            .allocate_pending_subresource_fetch_request_ids_for_owner(
                &crate::conn::CommandOwnerScope::for_session("SID-background"),
            )
            .expect("background owner should allocate request ids");
        let (second_active_fetch_id, second_active_network_request_id) = conn
            .allocate_pending_subresource_fetch_request_ids_for_owner(
                &crate::conn::CommandOwnerScope::for_session("SID-active"),
            )
            .expect("active owner should allocate a second request id");

        assert_eq!(active_fetch_id, "INT-SUB-1");
        assert_eq!(
            background_fetch_id, "INT-SUB-1",
            "Fetch interception ids remain target-local"
        );
        assert_eq!(second_active_fetch_id, "INT-SUB-2");
        assert_eq!(active_network_request_id, "REQ-1");
        assert_eq!(
            background_network_request_id, "REQ-2",
            "Network request ids must not restart for a background target"
        );
        assert_eq!(second_active_network_request_id, "REQ-3");
    }

    #[test]
    fn network_updates_preserve_active_and_background_policy_fields() {
        let mut active = BrowserContext::new_with_page_for_test("BID-active", "TID-active");
        {
            let context = &mut active;
            let target_id = context
                .active_target_id_owned()
                .expect("active fixture target");
            context.mutate_devtools_network_session_state_for_target(
                &target_id,
                &DevToolsSessionKey::Primary,
                |network| {
                    network.network_enabled = true;
                    network.cache_disabled = true;
                    network.bypass_service_worker = true;
                    network.blocked_url_patterns = vec!["*://blocked.test/*".to_owned()];
                    network.extra_headers = vec![("X-Test".to_owned(), "active".to_owned())];
                },
            )
        };
        {
            let context = &mut active;
            let target_id = context
                .active_target_id_owned()
                .expect("active fixture target");
            context.set_network_offline_for_target(&target_id, true)
        };

        assert!(
            active
                .effective_policy_for_target(active.active_target_id().unwrap())
                .cache_disabled()
        );
        assert!(
            active
                .effective_policy_for_target(active.active_target_id().unwrap())
                .bypass_service_worker()
        );
        assert_eq!(
            active
                .effective_policy_for_target(active.active_target_id().unwrap())
                .blocked_url_patterns(),
            vec!["*://blocked.test/*"]
        );
        assert_eq!(
            active
                .effective_policy_for_target(active.active_target_id().unwrap())
                .extra_headers(),
            vec![("X-Test".to_owned(), "active".to_owned())]
        );
        assert!(active.network_offline_for_target(active.active_target_id().unwrap()));

        let mut background = BrowserContext::new_with_page_for_test(
            "BID-network-owner-test",
            "TID-network-owner-test",
        );
        background.mutate_devtools_network_session_state_for_target(
            "TID-network-owner-test",
            &DevToolsSessionKey::Primary,
            |network| {
                network.network_enabled = true;
                network.cache_disabled = true;
                network.bypass_service_worker = true;
                network.blocked_url_patterns = vec!["*://background-blocked.test/*".to_owned()];
                network.extra_headers = vec![("X-Test".to_owned(), "background".to_owned())];
            },
        );
        background.set_network_offline_for_target("TID-network-owner-test", true);

        assert!(
            background
                .effective_policy_for_target("TID-network-owner-test")
                .cache_disabled()
        );
        assert!(
            background
                .effective_policy_for_target("TID-network-owner-test")
                .bypass_service_worker()
        );
        assert_eq!(
            background
                .effective_policy_for_target("TID-network-owner-test")
                .blocked_url_patterns(),
            vec!["*://background-blocked.test/*"]
        );
        assert_eq!(
            background
                .effective_policy_for_target("TID-network-owner-test")
                .extra_headers(),
            vec![("X-Test".to_owned(), "background".to_owned())]
        );
        assert!(background.network_offline_for_target("TID-network-owner-test"));
    }

    #[test]
    fn repeated_background_primary_network_enable_preserves_observation_cursor() {
        let mut conn = connection_with_background_attached_session();

        assert!(conn.enable_network_listener_for_session_owner(Some("SID-background")));
        conn.runtime_session_owner_slot_mut(Some("SID-background"))
            .expect("background runtime slot")
            .set_subresource_emitted_record_count_for_test(4);

        assert!(conn.enable_network_listener_for_session_owner(Some("SID-background")));

        assert_eq!(
            conn.runtime_session_owner_slot(Some("SID-background"))
                .expect("background runtime slot")
                .subresource_emitted_record_count_for_test(),
            4,
            "idempotent Network.enable must not rewind the background primary cursor"
        );
    }

    #[test]
    fn repeated_background_attached_network_enable_preserves_observation_cursor() {
        let mut conn = connection_with_background_attached_session();

        assert!(conn.enable_network_listener_for_session_owner(Some("SID-attached")));
        conn.runtime_session_owner_slot_mut(Some("SID-attached"))
            .expect("background attached runtime slot")
            .set_session_observation_cursor_at_counts_for_test(Some("SID-attached"), 7, 0);

        assert!(conn.enable_network_listener_for_session_owner(Some("SID-attached")));

        assert_eq!(
            conn.runtime_session_owner_slot(Some("SID-attached"))
                .expect("background attached runtime slot")
                .emitted_subresource_record_count_for_session_for_test(Some("SID-attached")),
            7,
            "idempotent Network.enable must not rewind the background attached cursor"
        );
    }

    #[test]
    fn background_primary_network_disable_preserves_attached_listener_artifacts() {
        let mut conn = connection_with_background_attached_session();
        assert!(conn.enable_network_listener_for_session_owner(Some("SID-background")));
        assert!(conn.enable_network_listener_for_session_owner(Some("SID-attached")));
        conn.runtime_session_owner_slot_mut(Some("SID-background"))
            .expect("background runtime slot")
            .record_captured_response_body(
                "REQ-shared".to_owned(),
                "shared body".to_owned(),
                [None::<String>, Some("SID-attached".to_owned())],
            );
        conn.runtime_session_owner_slot_mut(Some("SID-background"))
            .expect("background runtime slot")
            .record_captured_response_body(
                "REQ-primary-only".to_owned(),
                "primary body".to_owned(),
                [None::<String>],
            );

        assert!(conn.disable_network_listener_for_session_owner(Some("SID-background")));

        let slot = conn
            .runtime_session_owner_slot(Some("SID-background"))
            .expect("background runtime slot");
        assert!(!slot.primary_network_events_enabled());
        assert!(slot.attached_network_events_enabled_for_session("SID-attached"));
        let shared = slot
            .captured_response_body("REQ-shared")
            .expect("shared body should remain while attached can observe it");
        assert!(!shared.is_visible_to_session(None));
        assert!(shared.is_visible_to_session(Some("SID-attached")));
        assert!(
            slot.captured_response_body("REQ-primary-only").is_none(),
            "primary-only body should be dropped when primary Network is disabled"
        );
    }

    #[test]
    fn background_attached_network_disable_preserves_primary_listener_artifacts() {
        let mut conn = connection_with_background_attached_session();
        assert!(conn.enable_network_listener_for_session_owner(Some("SID-background")));
        assert!(conn.enable_network_listener_for_session_owner(Some("SID-attached")));
        conn.runtime_session_owner_slot_mut(Some("SID-background"))
            .expect("background runtime slot")
            .record_captured_response_body(
                "REQ-shared".to_owned(),
                "shared body".to_owned(),
                [None::<String>, Some("SID-attached".to_owned())],
            );
        conn.runtime_session_owner_slot_mut(Some("SID-background"))
            .expect("background runtime slot")
            .record_captured_response_body(
                "REQ-aux-only".to_owned(),
                "aux body".to_owned(),
                [Some("SID-attached".to_owned())],
            );

        assert!(conn.disable_network_listener_for_session_owner(Some("SID-attached")));

        let slot = conn
            .runtime_session_owner_slot(Some("SID-background"))
            .expect("background runtime slot");
        assert!(slot.primary_network_events_enabled());
        assert!(!slot.attached_network_events_enabled_for_session("SID-attached"));
        let shared = slot
            .captured_response_body("REQ-shared")
            .expect("shared body should remain while primary can observe it");
        assert!(shared.is_visible_to_session(None));
        assert!(!shared.is_visible_to_session(Some("SID-attached")));
        assert!(
            slot.captured_response_body("REQ-aux-only").is_none(),
            "attached-only body should be dropped when attached Network is disabled"
        );
    }

    #[test]
    fn network_target_listener_can_be_disabled_after_enable() {
        let mut conn = CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new("BID-network".to_owned()));
        conn.browser_context
            .as_mut()
            .expect("browser context")
            .set_active_target_id("TID-network");

        assert!(conn.enable_network_listener_for_target("TID-network"));
        assert!(
            conn.browser_context
                .as_ref()
                .expect("browser context")
                .has_network_event_listeners()
        );

        assert!(conn.disable_network_listener_for_target("TID-network"));
        assert!(
            !conn
                .browser_context
                .as_ref()
                .expect("browser context")
                .has_network_event_listeners()
        );
    }
}
