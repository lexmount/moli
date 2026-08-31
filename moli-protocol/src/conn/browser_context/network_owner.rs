use super::target_session_owner::{TargetSessionOwnerMut, TargetSessionStateMut};
use super::*;
use crate::conn::CdpSessionRoute;
use crate::conn::{CapturedBody, TargetRuntimeSlot};
use crate::devtools_runtime::DevToolsNetworkDataType;
use crate::domains::network::{
    CapturedRequestBody, CapturedResponseBody, CollectedNetworkDataArtifact,
    NetworkBacklogPreferredRequestId, PendingNetworkBacklogDeliverySnapshot,
    TargetNetworkBacklogPreparedDelivery,
};
use moli_core::page::PendingPageCommand;

impl TargetSessionStateMut<'_> {
    fn set_blocked_url_patterns(
        mut self,
        blocked_url_patterns: Vec<String>,
    ) -> Option<Vec<String>> {
        let network_policy = self.network_policy_mut()?;
        Some(network_policy.replace_blocked_url_patterns(blocked_url_patterns))
    }

    fn set_tls_verify_host_override(mut self, enabled: bool) -> bool {
        let should_refresh_active_engine = matches!(self, Self::Active { .. });
        if let Some(tls_verify_host_override) = self.tls_verify_host_override_mut() {
            *tls_verify_host_override = Some(enabled);
        }
        should_refresh_active_engine
    }

    fn set_emulated_network_conditions(
        mut self,
        offline: bool,
        latency: f64,
        download_throughput: f64,
        upload_throughput: f64,
        connection_type: Option<String>,
    ) -> Option<bool> {
        let network_policy = self.network_policy_mut()?;
        Some(network_policy.set_emulated_network_conditions(
            offline,
            latency,
            download_throughput,
            upload_throughput,
            connection_type,
        ))
    }
}

enum TargetNetworkListenerOwnerMut<'a> {
    Active {
        browser_context: &'a mut BrowserContext,
        is_auxiliary_target_session: bool,
    },
    Background {
        browser_context: &'a mut BrowserContext,
        target_id: String,
        is_auxiliary_target_session: bool,
    },
    NoLoadedBrowserContext,
}

impl<'a> TargetSessionOwnerMut<'a> {
    fn into_network_listener_owner(self) -> TargetNetworkListenerOwnerMut<'a> {
        match self {
            Self::ActiveTarget {
                browser_context,
                is_auxiliary_target_session,
                ..
            } => TargetNetworkListenerOwnerMut::Active {
                browser_context,
                is_auxiliary_target_session,
            },
            Self::PageTargetHost {
                browser_context,
                target_id,
                is_auxiliary_target_session,
                ..
            } => TargetNetworkListenerOwnerMut::Background {
                browser_context,
                target_id,
                is_auxiliary_target_session,
            },
            Self::NoLoadedBrowserContext => TargetNetworkListenerOwnerMut::NoLoadedBrowserContext,
        }
    }

    fn enable_listener(mut self, session_id: Option<&str>) -> bool {
        if self
            .mutate_network_policy_session_state(|state| state.network_enabled = true)
            .is_none()
        {
            return false;
        }
        self.into_network_listener_owner()
            .enable_listener(session_id)
    }

    fn disable_listener(mut self, session_id: Option<&str>) -> bool {
        if self
            .mutate_network_policy_session_state(|state| *state = Default::default())
            .is_none()
        {
            return false;
        }
        self.into_network_listener_owner()
            .disable_listener(session_id)
    }
}

impl TargetNetworkListenerOwnerMut<'_> {
    fn network_listener_enabled(&self, session_id: Option<&str>) -> bool {
        match self {
            Self::Active {
                browser_context, ..
            } => browser_context.network_enabled_for_session(session_id),
            Self::Background {
                browser_context,
                target_id,
                is_auxiliary_target_session,
            } => {
                if *is_auxiliary_target_session {
                    return session_id.is_some_and(|session_id| {
                        browser_context
                            .background_target(target_id)
                            .is_some_and(|target| {
                                target
                                    .runtime_slot()
                                    .auxiliary_network_events_enabled_for_session(session_id)
                            })
                    });
                }
                browser_context
                    .background_target(target_id)
                    .is_some_and(|target| target.runtime_slot.primary_network_events_enabled())
            }
            Self::NoLoadedBrowserContext => false,
        }
    }

    fn set_primary_network_enabled(&mut self, enabled: bool) {
        match self {
            Self::Active {
                browser_context, ..
            } => {
                browser_context
                    .active_target
                    .runtime_slot
                    .set_primary_network_events_enabled(enabled);
            }
            Self::Background {
                browser_context,
                target_id,
                ..
            } => {
                if let Some(target) = browser_context.background_target_mut(target_id) {
                    target
                        .runtime_slot
                        .set_primary_network_events_enabled(enabled);
                }
            }
            Self::NoLoadedBrowserContext => {}
        }
    }

    fn initialize_network_observation_cursor_at_current_tail(&mut self, session_id: Option<&str>) {
        match self {
            Self::Active {
                browser_context, ..
            } => {
                browser_context.initialize_network_listener_observation_cursor(session_id);
            }
            Self::Background {
                browser_context,
                target_id,
                ..
            } => {
                initialize_parked_network_observation_cursor(browser_context, target_id, session_id)
            }
            Self::NoLoadedBrowserContext => {}
        }
    }

    fn remove_network_observation_cursor(&mut self, session_id: Option<&str>) {
        match self {
            Self::Active {
                browser_context, ..
            } => {
                browser_context.remove_network_listener_observation_cursor(session_id);
            }
            Self::Background {
                browser_context,
                target_id,
                ..
            } => remove_parked_network_observation_cursor(browser_context, target_id, session_id),
            Self::NoLoadedBrowserContext => {}
        }
    }

    fn remove_captured_response_body_visibility_for_session(&mut self, session_id: Option<&str>) {
        match self {
            Self::Active {
                browser_context, ..
            } => {
                browser_context.remove_captured_response_body_visibility_for_session(session_id);
            }
            Self::Background {
                browser_context,
                target_id,
                ..
            } => {
                remove_parked_captured_response_body_visibility(
                    browser_context,
                    target_id,
                    session_id,
                );
            }
            Self::NoLoadedBrowserContext => {}
        }
    }

    fn clear_network_observation_artifacts_if_unobserved(&mut self) {
        match self {
            Self::Active {
                browser_context, ..
            } => {
                browser_context.clear_network_observation_artifacts_if_unobserved();
            }
            Self::Background {
                browser_context,
                target_id,
                ..
            } => {
                clear_parked_network_observation_artifacts_if_unobserved(
                    browser_context,
                    target_id,
                );
            }
            Self::NoLoadedBrowserContext => {}
        }
    }

    fn listener_session_id<'s>(&self, session_id: Option<&'s str>) -> Option<&'s str> {
        self.is_auxiliary_target_session()
            .then_some(session_id)
            .flatten()
    }

    fn is_auxiliary_target_session(&self) -> bool {
        match self {
            Self::Active {
                is_auxiliary_target_session,
                ..
            }
            | Self::Background {
                is_auxiliary_target_session,
                ..
            } => *is_auxiliary_target_session,
            Self::NoLoadedBrowserContext => false,
        }
    }

    fn enable_listener(mut self, session_id: Option<&str>) -> bool {
        match &mut self {
            Self::Active {
                browser_context,
                is_auxiliary_target_session,
            } => {
                browser_context.enable_network_event_listener_for_session(
                    session_id,
                    *is_auxiliary_target_session,
                );
                return true;
            }
            Self::NoLoadedBrowserContext => return false,
            Self::Background { .. } => {}
        }

        let adding_network_event_listener = !self.network_listener_enabled(session_id);
        let listener_session_id = self.listener_session_id(session_id);
        if self.is_auxiliary_target_session() {
            if let Some(session_id) = session_id
                && let Some(runtime_slot) = self.runtime_slot_mut()
            {
                runtime_slot.enable_auxiliary_network_events(session_id);
            }
        } else {
            self.set_primary_network_enabled(true);
        }
        if adding_network_event_listener {
            self.initialize_network_observation_cursor_at_current_tail(listener_session_id);
        }
        true
    }

    fn disable_listener(mut self, session_id: Option<&str>) -> bool {
        match &mut self {
            Self::Active {
                browser_context,
                is_auxiliary_target_session,
            } => {
                browser_context.disable_network_event_listener_for_session(
                    session_id,
                    *is_auxiliary_target_session,
                );
                return true;
            }
            Self::NoLoadedBrowserContext => return false,
            Self::Background { .. } => {}
        }

        let listener_session_id = self.listener_session_id(session_id);
        if self.is_auxiliary_target_session() {
            if let Some(session_id) = session_id
                && let Some(runtime_slot) = self.runtime_slot_mut()
            {
                runtime_slot.disable_auxiliary_network_events(session_id);
            }
        } else {
            self.set_primary_network_enabled(false);
        }
        self.remove_network_observation_cursor(listener_session_id);
        self.remove_captured_response_body_visibility_for_session(listener_session_id);
        self.clear_network_observation_artifacts_if_unobserved();
        true
    }

    fn runtime_slot_mut(&mut self) -> Option<&mut TargetRuntimeSlot> {
        match self {
            Self::Active {
                browser_context, ..
            } => Some(&mut browser_context.active_target.runtime_slot),
            Self::Background {
                browser_context,
                target_id,
                ..
            } => browser_context
                .background_target_mut(target_id)
                .map(|target| &mut target.runtime_slot),
            Self::NoLoadedBrowserContext => None,
        }
    }
}

impl TargetSessionOwnerMut<'_> {
    fn mutate_network_policy_session_state<T>(
        &mut self,
        f: impl FnOnce(&mut crate::conn::state::DevToolsNetworkSessionState) -> T,
    ) -> Option<T> {
        fn mutate<T>(
            page_state: &mut crate::conn::state::TargetPageState,
            session_id: Option<&str>,
            is_auxiliary_target_session: bool,
            f: impl FnOnce(&mut crate::conn::state::DevToolsNetworkSessionState) -> T,
        ) -> T {
            page_state.mutate_devtools_network_session_state(
                is_auxiliary_target_session,
                session_id,
                f,
            )
        }

        match self {
            Self::ActiveTarget {
                browser_context,
                session_id,
                is_auxiliary_target_session,
                ..
            } => Some(mutate(
                browser_context.active_page_state_mut(),
                session_id.as_deref(),
                *is_auxiliary_target_session,
                f,
            )),
            Self::PageTargetHost {
                browser_context,
                target_id,
                session_id,
                is_auxiliary_target_session,
            } => Some(
                browser_context.mutate_parked_page_session_state(target_id, |page_state| {
                    mutate(
                        page_state,
                        session_id.as_deref(),
                        *is_auxiliary_target_session,
                        f,
                    )
                }),
            ),
            Self::NoLoadedBrowserContext => None,
        }
    }

    fn start_set_cache_disabled(
        mut self,
        cache_disabled: bool,
    ) -> Result<Option<PendingPageCommand>, String> {
        if self
            .mutate_network_policy_session_state(|state| {
                state.cache_disabled = cache_disabled;
            })
            .is_none()
        {
            return Err("BrowserContextNotLoaded".to_owned());
        }
        self.start_replay_effective_network_request_policy()
    }

    fn start_set_bypass_service_worker(
        mut self,
        bypass_service_worker: bool,
    ) -> Result<Option<PendingPageCommand>, String> {
        if matches!(self, Self::NoLoadedBrowserContext) {
            return Err("BrowserContextNotLoaded".to_owned());
        }
        self.mutate_network_policy_session_state(|state| {
            state.bypass_service_worker = bypass_service_worker;
        });
        let effective_bypass = self
            .mutate_session_state_ref(|mut state| {
                state
                    .network_policy_mut()
                    .map(|policy| policy.bypass_service_worker())
            })
            .unwrap_or(bypass_service_worker);
        let Some(page) = self
            .runtime_slot_mut()
            .and_then(|runtime_slot| runtime_slot.loaded_page_mut())
        else {
            return Ok(None);
        };
        page.start_set_bypass_service_worker(effective_bypass)
            .map(Some)
            .map_err(|error| format!("failed to update page service worker bypass: {error}"))
    }

    fn start_set_blocked_url_patterns(
        mut self,
        blocked_url_patterns: Vec<String>,
    ) -> Result<Option<PendingPageCommand>, String> {
        if matches!(self, Self::NoLoadedBrowserContext) {
            return Err("BrowserContextNotLoaded".to_owned());
        }
        let Some(effective_patterns) = self
            .mutate_session_state_ref(|state| state.set_blocked_url_patterns(blocked_url_patterns))
        else {
            return Ok(None);
        };
        let Some(page) = self
            .runtime_slot_mut()
            .and_then(|runtime_slot| runtime_slot.loaded_page_mut())
        else {
            return Ok(None);
        };
        page.start_set_blocked_url_patterns(&effective_patterns)
            .map(Some)
            .map_err(|error| format!("failed to update page blocked URLs: {error}"))
    }

    fn start_set_extra_http_headers(
        mut self,
        extra_headers: Vec<(String, String)>,
    ) -> Result<Option<PendingPageCommand>, String> {
        if matches!(self, Self::NoLoadedBrowserContext) {
            return Err("BrowserContextNotLoaded".to_owned());
        }
        if self
            .mutate_network_policy_session_state(|state| {
                state.extra_headers = extra_headers;
            })
            .is_none()
        {
            return Ok(None);
        }
        let headers = self
            .mutate_session_state_ref(|mut state| {
                state
                    .network_policy_mut()
                    .map(|policy| policy.extra_headers().to_vec())
            })
            .unwrap_or_default();
        let effective_headers = self.effective_extra_headers_for_target_policy(headers);
        let Some(page) = self
            .runtime_slot_mut()
            .and_then(|runtime_slot| runtime_slot.loaded_page_mut())
        else {
            return Ok(None);
        };
        page.start_set_extra_http_headers(&effective_headers)
            .map(Some)
            .map_err(|error| format!("failed to update page extra HTTP headers: {error}"))
    }

    fn start_set_target_extra_http_headers(
        mut self,
        extra_headers: Vec<(String, String)>,
    ) -> Result<Option<PendingPageCommand>, String> {
        if matches!(self, Self::NoLoadedBrowserContext) {
            return Err("BrowserContextNotLoaded".to_owned());
        }
        let headers = self
            .mutate_session_state_ref(|mut state| {
                state
                    .network_policy_mut()
                    .map(|policy| policy.replace_base_extra_headers(extra_headers))
            })
            .unwrap_or_default();
        let effective_headers = self.effective_extra_headers_for_target_policy(headers);
        let Some(page) = self
            .runtime_slot_mut()
            .and_then(|runtime_slot| runtime_slot.loaded_page_mut())
        else {
            return Ok(None);
        };
        page.start_set_extra_http_headers(&effective_headers)
            .map(Some)
            .map_err(|error| format!("failed to update page extra HTTP headers: {error}"))
    }

    fn start_replay_effective_network_request_policy(
        &mut self,
    ) -> Result<Option<PendingPageCommand>, String> {
        if matches!(self, Self::NoLoadedBrowserContext) {
            return Err("BrowserContextNotLoaded".to_owned());
        }
        let (headers, bypass_service_worker, cache_disabled) = self
            .mutate_session_state_ref(|mut state| {
                state.network_policy_mut().map(|policy| {
                    (
                        policy.extra_headers().to_vec(),
                        policy.bypass_service_worker(),
                        policy.cache_disabled(),
                    )
                })
            })
            .unwrap_or_default();
        let effective_headers = self.effective_extra_headers_for_target_policy(headers);
        let Some(page) = self
            .runtime_slot_mut()
            .and_then(|runtime_slot| runtime_slot.loaded_page_mut())
        else {
            return Ok(None);
        };
        page.start_set_network_request_policy(
            &effective_headers,
            bypass_service_worker,
            cache_disabled,
        )
        .map(Some)
        .map_err(|error| format!("failed to replay page network request policy: {error}"))
    }

    fn set_devtools_browser_identity_override(
        &mut self,
        browser_identity: Option<crate::conn::DevToolsBrowserIdentityOverride>,
    ) -> Option<bool> {
        let refresh_active_engine = matches!(
            self,
            Self::ActiveTarget {
                is_current_active_browser_context: true,
                ..
            }
        );
        self.mutate_page_state(|state, is_auxiliary_target_session, session_id| {
            state.set_devtools_browser_identity_override(
                is_auxiliary_target_session,
                session_id,
                browser_identity,
            );
        })?;
        Some(refresh_active_engine)
    }

    fn set_base_browser_identity_override(
        &mut self,
        browser_identity: Option<moli_browser_profile::BrowserIdentityProfile>,
    ) -> Option<bool> {
        let refresh_active_engine = matches!(
            self,
            Self::ActiveTarget {
                is_current_active_browser_context: true,
                ..
            }
        );
        self.mutate_page_state(|state, _is_auxiliary_target_session, _session_id| {
            if let Some(browser_identity) = browser_identity {
                state
                    .network_policy
                    .set_browser_identity_override(browser_identity);
            } else {
                state.network_policy.clear_browser_identity_override();
            }
        })?;
        Some(refresh_active_engine)
    }

    fn set_tls_verify_host_override(mut self, enabled: bool) -> Option<bool> {
        self.mutate_session_state_ref(|state| {
            state.set_tls_verify_host_override(enabled);
        });
        match self {
            Self::ActiveTarget {
                is_current_active_browser_context,
                ..
            } => Some(is_current_active_browser_context),
            Self::PageTargetHost { .. } => Some(false),
            Self::NoLoadedBrowserContext => None,
        }
    }

    fn start_set_emulated_network_conditions(
        mut self,
        offline: bool,
        latency: f64,
        download_throughput: f64,
        upload_throughput: f64,
        connection_type: Option<String>,
    ) -> Result<Option<PendingPageCommand>, String> {
        if matches!(self, Self::NoLoadedBrowserContext) {
            return Err("BrowserContextNotLoaded".to_owned());
        }
        let Some(effective_offline) = self.mutate_session_state_ref(|state| {
            state.set_emulated_network_conditions(
                offline,
                latency,
                download_throughput,
                upload_throughput,
                connection_type,
            )
        }) else {
            return Ok(None);
        };
        let Some(page) = self
            .runtime_slot_mut()
            .and_then(|runtime_slot| runtime_slot.loaded_page_mut())
        else {
            return Ok(None);
        };
        page.start_set_network_offline(effective_offline)
            .map(Some)
            .map_err(|error| format!("set emulated network conditions failed: {error}"))
    }
}

impl CdpConnection {
    pub(crate) fn captured_response_body_for_bidi_network_data(
        &self,
        request_id: &str,
        session_id: Option<&str>,
    ) -> Option<&CapturedResponseBody> {
        self.browser_contexts().find_map(|browser_context| {
            browser_context
                .active_target
                .runtime_slot
                .captured_response_body(request_id)
                .filter(|body| {
                    body.is_visible_to_session(session_id) || body.is_visible_to_session(None)
                })
                .or_else(|| {
                    browser_context.background_targets().find_map(|target| {
                        target
                            .runtime_slot()
                            .captured_response_body(request_id)
                            .filter(|body| {
                                body.is_visible_to_session(session_id)
                                    || body.is_visible_to_session(None)
                            })
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
            browser_context
                .active_target
                .runtime_slot
                .captured_request_body(request_id)
                .filter(|body| {
                    body.is_visible_to_session(session_id) || body.is_visible_to_session(None)
                })
                .or_else(|| {
                    browser_context.background_targets().find_map(|target| {
                        target
                            .runtime_slot()
                            .captured_request_body(request_id)
                            .filter(|body| {
                                body.is_visible_to_session(session_id)
                                    || body.is_visible_to_session(None)
                            })
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
        let Some((browser_context_id, target_id)) =
            self.target_owner_identity_for_session(session_id)
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

    pub(crate) fn network_backlog_prepared_delivery_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        trigger_session_id: Option<&str>,
        primary_session_id: Option<&str>,
        preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
    ) -> Option<TargetNetworkBacklogPreparedDelivery> {
        let mut network_request_id_allocator =
            std::mem::take(&mut self.network_request_id_allocator);
        let result = self
            .runtime_session_owner_slot_mut(session_id)
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

    pub(crate) fn pending_network_backlog_delivery_snapshot_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        trigger_session_id: Option<&str>,
        primary_session_id: Option<&str>,
        preferred_request_id: Option<NetworkBacklogPreferredRequestId<'_>>,
    ) -> Option<PendingNetworkBacklogDeliverySnapshot> {
        let mut network_request_id_allocator =
            std::mem::take(&mut self.network_request_id_allocator);
        let result = self
            .runtime_session_owner_slot_mut(session_id)
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
        let Ok(runtime_slot) = self.runtime_session_owner_slot(session_id) else {
            return vec![session_id.map(str::to_owned)];
        };
        let primary_session_id = self.runtime_session_owner_primary_session_id(session_id);
        runtime_slot.network_event_session_ids(session_id, primary_session_id.as_deref())
    }

    pub(crate) fn enable_network_listener_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> bool {
        if let Some(session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(Some(session_id))
        {
            return target.set_network_enabled(session_id, true);
        }
        if let Some(session_id) = session_id
            && let Some(target) = self.dedicated_worker_target_for_session_mut(Some(session_id))
        {
            return target.set_network_enabled(session_id, true);
        }
        self.with_target_session_owner_mut(session_id, |owner| owner.enable_listener(session_id))
            .unwrap_or(false)
    }

    pub fn enable_network_listener_for_target(&mut self, target_id: &str) -> bool {
        let Some(route) = self.target_session_route_for_target_id(target_id) else {
            return false;
        };
        let mut route_scope = self.scoped_none_session_owner_route_override(route);
        route_scope
            .conn_mut()
            .enable_network_listener_for_session_owner(None)
    }

    pub fn disable_network_listener_for_target(&mut self, target_id: &str) -> bool {
        let Some(route) = self.target_session_route_for_target_id(target_id) else {
            return false;
        };
        let mut route_scope = self.scoped_none_session_owner_route_override(route);
        route_scope
            .conn_mut()
            .disable_network_listener_for_session_owner(None)
    }

    pub(crate) fn set_global_cache_disabled(&mut self, cache_disabled: bool) {
        self.global_cache_disabled = cache_disabled;
        for browser_context in self
            .browser_context
            .iter_mut()
            .chain(self.inactive_browser_contexts.iter_mut())
        {
            browser_context.global_cache_disabled = cache_disabled;
            for target in browser_context.page_targets.iter_mut() {
                target.set_base_cache_disabled(cache_disabled);
            }
        }
    }

    pub(crate) fn set_global_extra_headers(&mut self, extra_headers: Vec<(String, String)>) {
        self.global_extra_headers = extra_headers.clone();
        if let Some(browser_context) = self.browser_context.as_mut() {
            browser_context.global_extra_headers = extra_headers.clone();
        }
        for browser_context in &mut self.inactive_browser_contexts {
            browser_context.global_extra_headers = extra_headers.clone();
        }
    }

    pub(crate) fn disable_network_listener_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> bool {
        if let Some(session_id) = session_id
            && let Some(target) = self.service_worker_target_for_session_mut(Some(session_id))
        {
            return target.set_network_enabled(session_id, false);
        }
        if let Some(session_id) = session_id
            && let Some(target) = self.dedicated_worker_target_for_session_mut(Some(session_id))
        {
            return target.set_network_enabled(session_id, false);
        }
        self.with_target_session_owner_mut(session_id, |owner| owner.disable_listener(session_id))
            .unwrap_or(false)
    }

    pub(crate) fn start_set_cache_disabled_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        cache_disabled: bool,
    ) -> Result<Option<PendingPageCommand>, String> {
        let Some(owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        owner.start_set_cache_disabled(cache_disabled)
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
            if let Some(target) = browser_context.page_target_mut(target_id) {
                target.set_base_cache_disabled(cache_disabled);
                return true;
            }
        }
        false
    }

    pub(crate) fn start_set_bypass_service_worker_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        bypass_service_worker: bool,
    ) -> Result<Option<PendingPageCommand>, String> {
        let Some(owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        owner.start_set_bypass_service_worker(bypass_service_worker)
    }

    pub(crate) fn start_set_blocked_url_patterns_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        blocked_url_patterns: Vec<String>,
    ) -> Result<Option<PendingPageCommand>, String> {
        let Some(owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        owner.start_set_blocked_url_patterns(blocked_url_patterns)
    }

    pub(crate) fn start_set_extra_http_headers_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        extra_headers: Vec<(String, String)>,
    ) -> Result<Option<PendingPageCommand>, String> {
        let Some(owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        owner.start_set_extra_http_headers(extra_headers)
    }

    pub(crate) fn start_set_target_extra_http_headers_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        extra_headers: Vec<(String, String)>,
    ) -> Result<Option<PendingPageCommand>, String> {
        let Some(owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        owner.start_set_target_extra_http_headers(extra_headers)
    }

    pub(crate) fn start_replay_effective_network_request_policy_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<Option<PendingPageCommand>, String> {
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
        let Some(mut owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        owner.start_replay_effective_network_request_policy()
    }

    pub(crate) fn start_set_base_user_agent_override_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        user_agent: Option<String>,
    ) -> Result<Option<PendingPageCommand>, String> {
        let browser_identity = user_agent.as_ref().map(|user_agent| {
            moli_browser_profile::BrowserIdentityProfile::new(
                user_agent.clone(),
                self.base_browser_identity.accept_language(),
            )
        });
        self.start_set_base_browser_identity_override_for_session_owner(
            session_id,
            browser_identity,
        )
    }

    fn start_set_non_page_browser_identity_override(
        &mut self,
        session_id: Option<&str>,
        browser_identity: Option<moli_browser_profile::BrowserIdentityProfile>,
    ) -> Option<Result<Option<PendingPageCommand>, String>> {
        let is_browser_session = matches!(
            self.session_route(session_id),
            Some(CdpSessionRoute::Browser)
        );
        let is_pre_context_root = session_id.is_none()
            && self.none_session_owner_route_override().is_none()
            && self.browser_context.is_none();
        if !is_browser_session && !is_pre_context_root {
            return None;
        }

        if is_browser_session && let Some(browser_context) = self.browser_context.as_mut() {
            if let Some(browser_identity) = browser_identity {
                browser_context
                    .network_policy
                    .set_browser_identity_override(browser_identity);
            } else {
                browser_context
                    .network_policy
                    .clear_browser_identity_override();
            }
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
    ) -> Result<Option<PendingPageCommand>, String> {
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
        let Some(refresh_active_engine) =
            owner.set_devtools_browser_identity_override(browser_identity)
        else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        if refresh_active_engine {
            self.apply_active_engine_fetch_overrides();
            return self.start_rebuild_resource_runtime_for_session_owner(session_id);
        }
        Ok(None)
    }

    pub(crate) fn start_set_base_browser_identity_override_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        browser_identity: Option<moli_browser_profile::BrowserIdentityProfile>,
    ) -> Result<Option<PendingPageCommand>, String> {
        if let Some(result) =
            self.start_set_non_page_browser_identity_override(session_id, browser_identity.clone())
        {
            return result;
        }

        let Some(mut owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        let Some(refresh_active_engine) =
            owner.set_base_browser_identity_override(browser_identity)
        else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        if refresh_active_engine {
            self.apply_active_engine_fetch_overrides();
            return self.start_rebuild_resource_runtime_for_session_owner(session_id);
        }
        Ok(None)
    }

    pub(crate) fn start_set_tls_verify_host_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
    ) -> Result<Option<PendingPageCommand>, String> {
        if session_id.is_none()
            || matches!(
                self.session_route(session_id),
                Some(CdpSessionRoute::Browser)
            )
        {
            if let Some(browser_context) = self.browser_context.as_mut() {
                browser_context.default_tls_verify_host_override = Some(enabled);
            } else {
                self.base_tls_verify_host = enabled;
            }
            self.apply_active_engine_fetch_overrides();
            return self.start_rebuild_resource_runtime_for_session_owner(session_id);
        }

        let Some(owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        let Some(refresh_active_engine) = owner.set_tls_verify_host_override(enabled) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        if refresh_active_engine {
            self.apply_active_engine_fetch_overrides();
            return self.start_rebuild_resource_runtime_for_session_owner(session_id);
        }
        Ok(None)
    }

    pub(crate) fn start_set_emulated_network_conditions_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        offline: bool,
        latency: f64,
        download_throughput: f64,
        upload_throughput: f64,
        connection_type: Option<String>,
    ) -> Result<Option<PendingPageCommand>, String> {
        let Some(owner) = self.target_session_owner_mut(session_id) else {
            return Err("BrowserContextNotLoaded".to_owned());
        };
        owner.start_set_emulated_network_conditions(
            offline,
            latency,
            download_throughput,
            upload_throughput,
            connection_type,
        )
    }
}

fn initialize_parked_network_observation_cursor(
    browser_context: &mut BrowserContext,
    target_id: &str,
    session_id: Option<&str>,
) {
    browser_context
        .background_target_mut(target_id)
        .expect("background target route must retain its target")
        .runtime_slot
        .initialize_network_session_observation_cursor_at_output_tail(session_id);
}

fn remove_parked_network_observation_cursor(
    browser_context: &mut BrowserContext,
    target_id: &str,
    session_id: Option<&str>,
) {
    browser_context
        .background_target_mut(target_id)
        .expect("background target route must retain its target")
        .runtime_slot
        .remove_network_session_observation_cursor(session_id);
}

fn remove_parked_captured_response_body_visibility(
    browser_context: &mut BrowserContext,
    target_id: &str,
    session_id: Option<&str>,
) {
    browser_context
        .background_target_mut(target_id)
        .expect("background target route must retain its target")
        .runtime_slot
        .remove_captured_response_body_visibility_for_session(session_id);
}

fn clear_parked_network_observation_artifacts_if_unobserved(
    browser_context: &mut BrowserContext,
    target_id: &str,
) {
    if let Some(target) = browser_context.background_target_mut(target_id) {
        if target.runtime_slot.has_network_event_listeners() {
            return;
        }
        target.runtime_slot.clear_captured_response_bodies();
        target.runtime_slot.clear_websocket_request_ids();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::{PageTargetHost, TargetPageState};

    fn active_session_state_mut(browser_context: &mut BrowserContext) -> TargetSessionStateMut<'_> {
        let state = browser_context.active_page_state_mut();
        TargetSessionStateMut::Active {
            devtools_session_state: &mut state.devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary],
            network_policy: &mut state.network_policy,
            tls_verify_host_override: &mut state.tls_verify_host_override,
        }
    }

    fn parked_session_state_mut(state: &mut TargetPageState) -> TargetSessionStateMut<'_> {
        TargetSessionStateMut::Parked {
            devtools_session_state: &mut state.devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary],
            network_policy: &mut state.network_policy,
            tls_verify_host_override: &mut state.tls_verify_host_override,
        }
    }

    fn connection_with_background_auxiliary_session() -> CdpConnection {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-background".to_owned());
        browser_context.insert_page_target_host(PageTargetHost::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "https://background.example/".to_owned(),
        ));
        assert!(
            browser_context
                .assign_auxiliary_session_to_target("TID-background", "SID-aux".to_owned())
        );
        conn.browser_context = Some(browser_context);
        conn
    }

    #[test]
    fn subresource_fetch_network_request_ids_are_connection_global_across_target_owners() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-mixed".to_owned());
        browser_context.set_active_target_id("TID-active".to_owned());
        browser_context.attach_active_session("SID-active".to_owned());
        browser_context.insert_page_target_host(PageTargetHost::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "https://background.example/".to_owned(),
        ));
        conn.browser_context = Some(browser_context);

        let (active_fetch_id, active_network_request_id) = conn
            .allocate_pending_subresource_fetch_request_ids_for_session_owner(Some("SID-active"))
            .expect("active owner should allocate request ids");
        let (background_fetch_id, background_network_request_id) = conn
            .allocate_pending_subresource_fetch_request_ids_for_session_owner(Some(
                "SID-background",
            ))
            .expect("background owner should allocate request ids");
        let (second_active_fetch_id, second_active_network_request_id) = conn
            .allocate_pending_subresource_fetch_request_ids_for_session_owner(Some("SID-active"))
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
    fn target_session_state_mut_applies_active_and_parked_network_fields() {
        let mut active = BrowserContext::new("BID-active".to_owned());
        {
            let network = &mut active.devtools_sessions.primary_mut().network_session_state;
            network.network_enabled = true;
            network.cache_disabled = true;
            network.bypass_service_worker = true;
            network.extra_headers = vec![("X-Test".to_owned(), "active".to_owned())];
        }
        active.refresh_devtools_network_policy();
        let active_blocked = active_session_state_mut(&mut active)
            .set_blocked_url_patterns(vec!["*://blocked.test/*".to_owned()]);
        let active_offline = active_session_state_mut(&mut active).set_emulated_network_conditions(
            true,
            25.0,
            1024.0,
            256.0,
            Some("cellular3g".to_owned()),
        );

        assert!(active.network_policy.cache_disabled());
        assert!(active.network_policy.bypass_service_worker());
        assert_eq!(active_blocked, Some(vec!["*://blocked.test/*".to_owned()]));
        assert_eq!(
            active.network_policy.blocked_url_patterns(),
            vec!["*://blocked.test/*"]
        );
        assert_eq!(
            active.network_policy.extra_headers(),
            vec![("X-Test".to_owned(), "active".to_owned())]
        );
        assert_eq!(active_offline, Some(true));
        assert!(active.network_policy.network_offline());
        assert_eq!(active.network_policy.emulated_network_latency(), 25.0);
        assert_eq!(active.network_policy.emulated_download_throughput(), 1024.0);
        assert_eq!(active.network_policy.emulated_upload_throughput(), 256.0);
        assert_eq!(
            active.network_policy.emulated_connection_type(),
            Some("cellular3g")
        );

        let mut parked = TargetPageState::default();
        {
            let network = &mut parked.devtools_sessions.primary_mut().network_session_state;
            network.network_enabled = true;
            network.cache_disabled = true;
            network.bypass_service_worker = true;
            network.extra_headers = vec![("X-Test".to_owned(), "parked".to_owned())];
        }
        parked.refresh_devtools_network_policy();
        let parked_blocked = parked_session_state_mut(&mut parked)
            .set_blocked_url_patterns(vec!["*://parked-blocked.test/*".to_owned()]);
        let parked_offline = parked_session_state_mut(&mut parked).set_emulated_network_conditions(
            true,
            50.0,
            2048.0,
            512.0,
            Some("cellular4g".to_owned()),
        );

        assert!(parked.network_policy.cache_disabled());
        assert!(parked.network_policy.bypass_service_worker());
        assert_eq!(
            parked_blocked,
            Some(vec!["*://parked-blocked.test/*".to_owned()])
        );
        assert_eq!(
            parked.network_policy.blocked_url_patterns(),
            vec!["*://parked-blocked.test/*"]
        );
        assert_eq!(
            parked.network_policy.extra_headers(),
            vec![("X-Test".to_owned(), "parked".to_owned())]
        );
        assert_eq!(parked_offline, Some(true));
        assert!(parked.network_policy.network_offline());
        assert_eq!(parked.network_policy.emulated_network_latency(), 50.0);
        assert_eq!(parked.network_policy.emulated_download_throughput(), 2048.0);
        assert_eq!(parked.network_policy.emulated_upload_throughput(), 512.0);
        assert_eq!(
            parked.network_policy.emulated_connection_type(),
            Some("cellular4g")
        );
    }

    #[test]
    fn repeated_background_primary_network_enable_preserves_observation_cursor() {
        let mut conn = connection_with_background_auxiliary_session();

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
    fn repeated_background_auxiliary_network_enable_preserves_observation_cursor() {
        let mut conn = connection_with_background_auxiliary_session();

        assert!(conn.enable_network_listener_for_session_owner(Some("SID-aux")));
        conn.runtime_session_owner_slot_mut(Some("SID-aux"))
            .expect("background auxiliary runtime slot")
            .set_session_observation_cursor_at_counts_for_test(Some("SID-aux"), 7, 0);

        assert!(conn.enable_network_listener_for_session_owner(Some("SID-aux")));

        assert_eq!(
            conn.runtime_session_owner_slot(Some("SID-aux"))
                .expect("background auxiliary runtime slot")
                .emitted_subresource_record_count_for_session_for_test(Some("SID-aux")),
            7,
            "idempotent Network.enable must not rewind the background auxiliary cursor"
        );
    }

    #[test]
    fn background_primary_network_disable_preserves_auxiliary_listener_artifacts() {
        let mut conn = connection_with_background_auxiliary_session();
        assert!(conn.enable_network_listener_for_session_owner(Some("SID-background")));
        assert!(conn.enable_network_listener_for_session_owner(Some("SID-aux")));
        conn.runtime_session_owner_slot_mut(Some("SID-background"))
            .expect("background runtime slot")
            .record_captured_response_body(
                "REQ-shared".to_owned(),
                "shared body".to_owned(),
                [None::<String>, Some("SID-aux".to_owned())],
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
        assert!(slot.auxiliary_network_events_enabled_for_session("SID-aux"));
        let shared = slot
            .captured_response_body("REQ-shared")
            .expect("shared body should remain while auxiliary can observe it");
        assert!(!shared.is_visible_to_session(None));
        assert!(shared.is_visible_to_session(Some("SID-aux")));
        assert!(
            slot.captured_response_body("REQ-primary-only").is_none(),
            "primary-only body should be dropped when primary Network is disabled"
        );
    }

    #[test]
    fn background_auxiliary_network_disable_preserves_primary_listener_artifacts() {
        let mut conn = connection_with_background_auxiliary_session();
        assert!(conn.enable_network_listener_for_session_owner(Some("SID-background")));
        assert!(conn.enable_network_listener_for_session_owner(Some("SID-aux")));
        conn.runtime_session_owner_slot_mut(Some("SID-background"))
            .expect("background runtime slot")
            .record_captured_response_body(
                "REQ-shared".to_owned(),
                "shared body".to_owned(),
                [None::<String>, Some("SID-aux".to_owned())],
            );
        conn.runtime_session_owner_slot_mut(Some("SID-background"))
            .expect("background runtime slot")
            .record_captured_response_body(
                "REQ-aux-only".to_owned(),
                "aux body".to_owned(),
                [Some("SID-aux".to_owned())],
            );

        assert!(conn.disable_network_listener_for_session_owner(Some("SID-aux")));

        let slot = conn
            .runtime_session_owner_slot(Some("SID-background"))
            .expect("background runtime slot");
        assert!(slot.primary_network_events_enabled());
        assert!(!slot.auxiliary_network_events_enabled_for_session("SID-aux"));
        let shared = slot
            .captured_response_body("REQ-shared")
            .expect("shared body should remain while primary can observe it");
        assert!(shared.is_visible_to_session(None));
        assert!(!shared.is_visible_to_session(Some("SID-aux")));
        assert!(
            slot.captured_response_body("REQ-aux-only").is_none(),
            "auxiliary-only body should be dropped when auxiliary Network is disabled"
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
