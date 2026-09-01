use super::super::state::{TargetPageAbsenceReason, TargetSessionStorageNamespace};
use super::super::{
    BrowserContext, DedicatedWorkerTargetState, PageTargetHost, ServiceWorkerTargetState,
    SharedWorkerTargetState, TargetIdentityState, TargetInitialEmptyDocumentCreator,
    TargetOwnerState,
};
use crate::devtools_runtime::{
    DevToolsBrowserContextId, DevToolsTargetId, DevToolsTargetInfo, DevToolsTargetKind,
};
use moli_core::network::SharedWebStorageStore;

impl BrowserContext {
    pub fn remove_background_target(&mut self, target_id: &str) -> Option<PageTargetHost> {
        if self.is_active_target(target_id) {
            return None;
        }
        self.page_targets.remove(target_id)
    }

    pub(crate) fn stage_background_target(
        &mut self,
        target_id: String,
        session_id: Option<String>,
        url: String,
        initial_empty_document_url: Option<String>,
        creator: Option<TargetInitialEmptyDocumentCreator>,
    ) {
        let session_storage_namespace =
            self.deep_cloned_session_storage_namespace_for_creator(creator.as_ref());
        self.stage_background_target_with_session_storage_namespace(
            target_id,
            session_id,
            url,
            initial_empty_document_url,
            creator,
            None,
            session_storage_namespace,
        );
    }

    pub(crate) fn stage_popup_background_target(
        &mut self,
        target_id: String,
        session_id: Option<String>,
        url: String,
        initial_empty_document_url: Option<String>,
        creator: Option<TargetInitialEmptyDocumentCreator>,
        session_storage_store: Option<SharedWebStorageStore>,
        initial_empty_document_storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) {
        let session_storage_namespace = session_storage_store
            .map(TargetSessionStorageNamespace::from_store)
            .or_else(|| self.deep_cloned_session_storage_namespace_for_creator(creator.as_ref()));
        self.stage_background_target_with_session_storage_namespace(
            target_id,
            session_id,
            url,
            initial_empty_document_url,
            creator,
            initial_empty_document_storage_key,
            session_storage_namespace,
        );
    }

    fn deep_cloned_session_storage_namespace_for_creator(
        &self,
        creator: Option<&TargetInitialEmptyDocumentCreator>,
    ) -> Option<TargetSessionStorageNamespace> {
        creator.and_then(|creator| {
            if self.is_active_target(creator.target_id()) {
                return Some(
                    self.active_page_target()
                        .session_storage_namespace
                        .deep_clone(),
                );
            }
            self.background_target(creator.target_id())
                .map(PageTargetHost::deep_clone_session_storage_namespace)
        })
    }

    fn stage_background_target_with_session_storage_namespace(
        &mut self,
        target_id: String,
        session_id: Option<String>,
        url: String,
        initial_empty_document_url: Option<String>,
        creator: Option<TargetInitialEmptyDocumentCreator>,
        initial_empty_document_storage_key: Option<moli_storage_key::MoliStorageKey>,
        session_storage_namespace: Option<TargetSessionStorageNamespace>,
    ) {
        let target_identity = background_target_identity_for_initial_url(&url, creator.as_ref());
        let mut target = PageTargetHost::with_identity(target_id, session_id, target_identity);
        let target_id = target.target_id().to_owned();
        target.owner_state.begin_initial_empty_document(
            target_id,
            initial_empty_document_url.unwrap_or_else(|| url.clone()),
            creator,
            initial_empty_document_storage_key,
        );
        if let Some(namespace) = session_storage_namespace {
            target.replace_session_storage_namespace(namespace);
        }
        let inserted = self.insert_page_target_host(target);
        debug_assert!(inserted, "staged page target id must be unique");
    }

    pub(crate) fn stage_active_target_demoting_current(
        &mut self,
        target_id: String,
        session_id: Option<String>,
        url: String,
        initial_empty_document_url: Option<String>,
    ) {
        let mut host = PageTargetHost::with_identity(
            target_id.clone(),
            session_id,
            TargetIdentityState::with_url(url.clone()),
        );
        host.owner_state.begin_initial_empty_document(
            target_id.clone(),
            initial_empty_document_url.unwrap_or(url),
            None,
            None,
        );
        let inserted = self.insert_page_target_host(host);
        debug_assert!(inserted, "new active page target id must be unique");
        let selected = self.page_targets.select(&target_id);
        debug_assert!(selected, "newly inserted page target must be selectable");
    }

    pub(crate) fn reusable_window_open_target_name(target_name: &str) -> Option<String> {
        if target_name.is_empty() || target_name.eq_ignore_ascii_case("_blank") {
            return None;
        }
        Some(target_name.to_owned())
    }

    pub(crate) fn target_id_for_window_name(&self, target_name: &str) -> Option<&str> {
        let name = Self::reusable_window_open_target_name(target_name)?;
        self.target_window_names.get(&name).map(String::as_str)
    }

    pub(crate) fn has_attached_child_frame_id(&self, frame_id: &str) -> bool {
        self.active_page_target()
            .owner_state
            .has_attached_child_frame_id(frame_id)
            || self.background_targets().any(|target| {
                self.parked_target_owner_state(target.target_id())
                    .is_some_and(|owner_state| owner_state.has_attached_child_frame_id(frame_id))
            })
    }

    pub(crate) fn remember_target_window_name(&mut self, target_name: &str, target_id: &str) {
        if let Some(name) = Self::reusable_window_open_target_name(target_name) {
            self.target_window_names.insert(name, target_id.to_owned());
        }
    }

    pub(crate) fn remember_target_popup_id(&mut self, popup_id: Option<u64>, target_id: &str) {
        if let Some(popup_id) = popup_id
            && let Some(replaced_popup_id) =
                self.target_popup_ids.insert(target_id.to_owned(), popup_id)
            && replaced_popup_id != popup_id
        {
            self.dismiss_pending_popup_javascript_dialogs(replaced_popup_id);
        }
    }

    pub(crate) fn forget_target_window_names_for_target(&mut self, target_id: &str) {
        self.target_window_names
            .retain(|_, mapped_target_id| mapped_target_id != target_id);
    }

    pub(crate) fn forget_target_popup_id_for_target(&mut self, target_id: &str) {
        if let Some(popup_id) = self.target_popup_ids.remove(target_id) {
            self.dismiss_pending_popup_javascript_dialogs(popup_id);
        }
    }

    pub(crate) fn target_popup_id(&self, target_id: &str) -> Option<u64> {
        self.target_popup_ids.get(target_id).copied()
    }

    pub(crate) fn target_id_for_popup_id(&self, popup_id: u64) -> Option<&str> {
        self.target_popup_ids
            .iter()
            .find_map(|(target_id, candidate)| {
                (*candidate == popup_id && self.devtools_target_info(target_id).is_some())
                    .then_some(target_id.as_str())
            })
    }

    pub(crate) fn remember_target_opener(
        &mut self,
        target_id: &str,
        opener_target_id: String,
        opener_frame_id: String,
        can_access_opener: bool,
    ) {
        self.target_opener_ids
            .insert(target_id.to_owned(), opener_target_id);
        self.target_opener_frame_ids
            .insert(target_id.to_owned(), opener_frame_id);
        if can_access_opener {
            self.target_can_access_opener.insert(target_id.to_owned());
        } else {
            self.target_can_access_opener.remove(target_id);
        }
    }

    pub(crate) fn forget_target_opener_references_for_target(&mut self, target_id: &str) {
        let targets_with_removed_opener = self
            .target_opener_ids
            .iter()
            .filter_map(|(candidate_target_id, opener_target_id)| {
                (opener_target_id == target_id).then_some(candidate_target_id.clone())
            })
            .collect::<Vec<_>>();
        self.target_opener_ids.remove(target_id);
        self.target_can_access_opener.remove(target_id);
        self.target_opener_ids
            .retain(|_, opener_target_id| opener_target_id != target_id);
        self.target_opener_frame_ids.remove(target_id);
        for candidate_target_id in targets_with_removed_opener {
            self.target_can_access_opener.remove(&candidate_target_id);
            // Chromium keeps openerFrameId as immutable DevTools attribution
            // after the opener target closes, while openerId and script access
            // disappear. Drop the frame id only when the attributed target is
            // itself no longer live.
            if self.devtools_target_info(&candidate_target_id).is_none() {
                self.target_opener_frame_ids.remove(&candidate_target_id);
            }
        }
    }

    pub(crate) fn update_target_url(&mut self, target_id: &str, url: String) -> bool {
        if self.is_active_target(target_id) {
            self.set_target_url(url);
            self.active_page_target_mut()
                .owner_state
                .target_crash_state
                .clear();
            return true;
        }
        if let Some(target) = self.background_target_mut(target_id) {
            target.set_target_url(url);
            return true;
        }
        false
    }

    pub(crate) fn assign_session_to_target(&mut self, target_id: &str, session_id: String) -> bool {
        if self.is_active_target(target_id) {
            self.attach_active_session(session_id);
            true
        } else if let Some(target) = self.background_target_mut(target_id) {
            target.attach_session(session_id);
            true
        } else {
            false
        }
    }

    pub(crate) fn assign_auto_attached_session_to_target(
        &mut self,
        target_id: &str,
        session_id: String,
    ) -> bool {
        if self.is_active_target(target_id) {
            if self.has_active_session() {
                self.assign_auxiliary_session_to_target(target_id, session_id)
            } else {
                self.attach_active_session(session_id);
                true
            }
        } else if let Some(target) = self.background_target_mut(target_id) {
            if target.has_session() {
                self.assign_auxiliary_session_to_target(target_id, session_id)
            } else {
                target.attach_session(session_id);
                true
            }
        } else {
            false
        }
    }

    pub(crate) fn assign_auxiliary_session_to_target(
        &mut self,
        target_id: &str,
        session_id: String,
    ) -> bool {
        let Some(target) = self.page_target_mut(target_id) else {
            return false;
        };
        target.devtools_sessions.ensure_attached(&session_id);
        true
    }

    pub(crate) fn auxiliary_target_id_for_session(&self, session_id: &str) -> Option<&str> {
        self.page_targets
            .iter()
            .find(|target| target.devtools_sessions.attached(session_id).is_some())
            .map(PageTargetHost::target_id)
    }

    pub(crate) fn auxiliary_session_ids_for_target(&self, target_id: &str) -> Vec<String> {
        let mut session_ids = self
            .page_target(target_id)
            .into_iter()
            .flat_map(|target| target.devtools_sessions.attached_session_ids())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        session_ids.sort();
        session_ids
    }

    pub(crate) fn devtools_session_ids_for_target(&self, target_id: &str) -> Vec<String> {
        let mut session_ids = if self.is_active_target(target_id) {
            self.active_session_id_owned().into_iter().collect()
        } else if let Some(target) = self.background_target(target_id) {
            target.session_id().map(str::to_owned).into_iter().collect()
        } else if let Some(target) = self.shared_worker_target(target_id) {
            target.session_ids()
        } else if let Some(target) = self.service_worker_target(target_id) {
            target.session_ids()
        } else {
            Vec::new()
        };
        session_ids.extend(self.auxiliary_session_ids_for_target(target_id));
        session_ids.sort();
        session_ids.dedup();
        session_ids
    }

    pub(crate) fn remove_auxiliary_session(&mut self, session_id: &str) -> Option<String> {
        let target_id = self.auxiliary_target_id_for_session(session_id)?.to_owned();
        if let Some(target) = self.page_target_mut(&target_id) {
            target
                .runtime_slot
                .remove_auxiliary_network_session(session_id);
            target.fetch_owner.remove_fetch_session(Some(session_id));
            target
                .runtime_slot
                .remove_network_session_observation_cursor(Some(session_id));
            target.devtools_sessions.remove_attached(session_id);
            target.refresh_devtools_network_policy();
            target.refresh_devtools_emulation_policy();
        }
        Some(target_id)
    }

    pub(crate) async fn clear_target_session_overrides_async(
        &mut self,
        session_id: &str,
    ) -> Result<bool, String> {
        let Some(target_id) = (self.active_session_id() == Some(session_id))
            .then(|| self.active_target_id_owned())
            .flatten()
            .or_else(|| {
                self.background_targets()
                    .find(|target| target.is_session(session_id))
                    .map(|target| target.target_id().to_owned())
            })
            .or_else(|| {
                self.auxiliary_target_id_for_session(session_id)
                    .map(str::to_owned)
            })
        else {
            return Ok(false);
        };

        let is_auxiliary = self.auxiliary_target_id_for_session(session_id) == Some(&target_id);
        let Some(target) = self.page_target_mut(&target_id) else {
            return Ok(false);
        };
        let state = target;
        let previous_headers = state.network_policy.extra_headers().to_vec();
        let previous_bypass = state.network_policy.bypass_service_worker();
        let previous_cache_disabled = state.network_policy.cache_disabled();
        let previous_blocked_url_patterns = state.network_policy.blocked_url_patterns().to_vec();
        let previous_browser_identity = state.network_policy.browser_identity_override_owned();
        let previous_renderer_browser_identity =
            state.effective_renderer_browser_identity_override_owned();
        let previous_locale = state.locale_override.clone();
        let previous_timezone = state.timezone_override.clone();
        let routed_session_id = is_auxiliary.then_some(session_id);
        state.clear_devtools_network_state(is_auxiliary, routed_session_id);
        state.clear_devtools_emulation_state(is_auxiliary, routed_session_id);
        let extra_headers_changed = previous_headers != state.network_policy.extra_headers();
        let bypass_service_worker_changed =
            previous_bypass != state.network_policy.bypass_service_worker();
        let cache_disabled_changed =
            previous_cache_disabled != state.network_policy.cache_disabled();
        let blocked_url_patterns_changed =
            previous_blocked_url_patterns != state.network_policy.blocked_url_patterns();
        let browser_identity_changed =
            previous_browser_identity != state.network_policy.browser_identity_override_owned();
        let renderer_browser_identity_changed = previous_renderer_browser_identity
            != state.effective_renderer_browser_identity_override_owned();
        let any_browser_identity_changed =
            browser_identity_changed || renderer_browser_identity_changed;
        let locale_changed = previous_locale != state.locale_override;
        let timezone_changed = previous_timezone != state.timezone_override;

        if !extra_headers_changed
            && !bypass_service_worker_changed
            && !cache_disabled_changed
            && !blocked_url_patterns_changed
            && !locale_changed
            && !timezone_changed
        {
            return Ok(any_browser_identity_changed);
        }

        let (
            effective_headers,
            effective_bypass,
            effective_cache_disabled,
            effective_blocked_url_patterns,
            effective_locale,
            effective_timezone,
        ) = if self.is_active_target(&target_id) {
            (
                self.effective_extra_headers(),
                self.active_page_target()
                    .network_policy
                    .bypass_service_worker(),
                self.active_page_target().network_policy.cache_disabled(),
                self.active_page_target()
                    .network_policy
                    .blocked_url_patterns()
                    .to_vec(),
                self.effective_active_locale_override_owned(),
                self.effective_active_timezone_override_owned(),
            )
        } else {
            let page_state = self.parked_page_session_state(&target_id);
            (
                self.effective_parked_extra_headers(&target_id),
                page_state.is_some_and(|state| state.network_policy.bypass_service_worker()),
                page_state.is_some_and(|state| state.network_policy.cache_disabled()),
                page_state
                    .map(|state| state.network_policy.blocked_url_patterns().to_vec())
                    .unwrap_or_default(),
                page_state
                    .and_then(|state| state.locale_override.clone())
                    .or_else(|| self.default_locale_override.clone()),
                page_state
                    .and_then(|state| state.timezone_override.clone())
                    .or_else(|| self.default_timezone_override.clone()),
            )
        };
        let page = if self.is_active_target(&target_id) {
            self.active_page_target_mut().runtime_slot.loaded_page_mut()
        } else {
            self.background_target_mut(&target_id)
                .and_then(|target| target.runtime_slot.loaded_page_mut())
        };
        let Some(page) = page else {
            return Ok(any_browser_identity_changed);
        };
        if extra_headers_changed
            || bypass_service_worker_changed
            || cache_disabled_changed
            || blocked_url_patterns_changed
        {
            page.set_network_request_policy_async(
                &effective_headers,
                effective_bypass,
                effective_cache_disabled,
                &effective_blocked_url_patterns,
            )
            .await
            .map_err(|error| {
                format!("failed to restore detached session network request policy: {error}")
            })?;
        }
        if locale_changed {
            page.set_locale_override_async(effective_locale.as_deref())
                .await
                .map_err(|error| format!("failed to restore detached session locale: {error}"))?;
        }
        if timezone_changed {
            page.set_timezone_override_async(effective_timezone.as_deref())
                .await
                .map_err(|error| format!("failed to restore detached session timezone: {error}"))?;
        }
        Ok(any_browser_identity_changed)
    }

    pub(crate) fn remove_auxiliary_sessions_for_target(&mut self, target_id: &str) -> Vec<String> {
        let session_ids = self.auxiliary_session_ids_for_target(target_id);
        for session_id in &session_ids {
            let _ = self.remove_auxiliary_session(session_id);
        }
        session_ids
    }

    pub(crate) async fn clear_background_target_session_binding_and_scoped_state_async(
        &mut self,
        session_id: &str,
    ) -> Result<Option<String>, String> {
        let Some(target_id) = self
            .background_targets_mut()
            .find(|target| target.is_session(session_id))
            .map(|target| {
                target.detach_session();
                let target_id = target.target_id().to_owned();
                target.runtime_slot.disable_primary_network_events();
                target
                    .runtime_slot
                    .clear_session_scoped_network_observation_artifacts();
                target
                    .runtime_slot
                    .request_id_allocator()
                    .reset_fetch_navigation_request_counter();
                target
                    .runtime_slot
                    .request_id_allocator()
                    .reset_subresource_fetch_request_counter();
                target_id
            })
        else {
            return Ok(None);
        };

        let cleared_emulated_media = moli_core::page::EmulatedMediaOverrides::default();
        self.background_target_mut(&target_id)
            .expect("background target must exist")
            .clear_session_scoped_state_fields(true);
        self.background_target_mut(&target_id)
            .expect("background target must exist")
            .runtime_slot
            .remove_network_session_observation_cursor(Some(session_id));

        let effective_headers = self.effective_parked_extra_headers(&target_id);
        let effective_bypass = self
            .parked_page_session_state(&target_id)
            .is_some_and(|state| state.network_policy.bypass_service_worker());
        let cache_disabled = self
            .parked_page_session_state(&target_id)
            .is_some_and(|state| state.network_policy.cache_disabled());
        let blocked_url_patterns = self
            .parked_page_session_state(&target_id)
            .map(|state| state.network_policy.blocked_url_patterns().to_vec())
            .unwrap_or_default();

        if let Some(page) = self
            .background_target_mut(&target_id)
            .and_then(|target| target.runtime_slot.loaded_page_mut())
        {
            page.set_network_request_policy_async(
                &effective_headers,
                effective_bypass,
                cache_disabled,
                &blocked_url_patterns,
            )
            .await
            .map_err(|error| format!("failed to clear page network request policy: {error}"))?;
            page.set_network_offline_async(false)
                .await
                .map_err(|error| format!("failed to clear page offline state: {error}"))?;
            page.set_script_execution_disabled_async(false)
                .await
                .map_err(|error| {
                    format!("failed to clear page script execution disabled state: {error}")
                })?;
            page.set_emulated_media_async(&cleared_emulated_media)
                .await
                .map_err(|error| format!("failed to clear page emulated media: {error}"))?;
        }

        Ok(Some(target_id))
    }

    pub(crate) fn clear_background_target_primary_auto_attached_session(
        &mut self,
        session_id: &str,
    ) -> Option<String> {
        let target_id = self
            .background_targets_mut()
            .find(|target| target.is_session(session_id))
            .map(|target| {
                target.detach_session();
                let target_id = target.target_id().to_owned();
                target.runtime_slot.disable_primary_network_events();
                target
                    .runtime_slot
                    .clear_session_scoped_network_observation_artifacts();
                target
                    .runtime_slot
                    .request_id_allocator()
                    .reset_fetch_navigation_request_counter();
                target
                    .runtime_slot
                    .request_id_allocator()
                    .reset_subresource_fetch_request_counter();
                target_id
            })?;
        self.mutate_parked_page_session_state(&target_id, |state| {
            state.devtools_sessions.reset(true);
            state.runtime_slot.disable_primary_network_events();
            state.network_policy.clear_session_scoped_state();
            state.refresh_devtools_network_policy();
            state.refresh_devtools_emulation_policy();
            state.fetch_owner.reset_config();
        });
        self.background_target_mut(&target_id)
            .expect("background target must exist")
            .runtime_slot
            .remove_network_session_observation_cursor(Some(session_id));
        Some(target_id)
    }

    #[cfg(test)]
    pub(crate) fn enable_auxiliary_network_events(&mut self, session_id: &str) {
        if self.auxiliary_target_id_for_session(session_id).is_some() {
            self.active_page_state_mut()
                .runtime_slot
                .enable_auxiliary_network_events(session_id);
        }
    }

    #[cfg(test)]
    pub(crate) fn has_network_event_listeners(&self) -> bool {
        self.active_page_state()
            .runtime_slot
            .has_network_event_listeners()
    }

    #[cfg(test)]
    pub(crate) fn network_event_session_ids(
        &self,
        trigger_session_id: Option<&str>,
    ) -> Vec<Option<String>> {
        self.active_page_state()
            .runtime_slot
            .network_event_session_ids(trigger_session_id, self.active_session_id())
    }

    pub(crate) fn active_target_identity(&self) -> Option<(String, Option<String>)> {
        Some((
            self.active_target_id_owned()?,
            self.active_session_id_owned(),
        ))
    }

    pub(crate) fn initial_empty_document_creator_for_target(
        &self,
        target_id: &str,
    ) -> Option<TargetInitialEmptyDocumentCreator> {
        if self.is_active_target(target_id) {
            return Some(TargetInitialEmptyDocumentCreator::new(
                target_id.to_owned(),
                self.target_security_origin().to_owned(),
                self.target_secure_context_type().to_owned(),
            ));
        }
        let target = self.background_target(target_id)?;
        Some(TargetInitialEmptyDocumentCreator::new(
            target.target_id().to_owned(),
            target.target_identity().security_origin().to_owned(),
            target.target_identity().secure_context_type().to_owned(),
        ))
    }

    pub(crate) async fn clear_active_target_session_binding_and_scoped_state_async(
        &mut self,
    ) -> Result<(), String> {
        let target_id = self.active_target_id_owned();
        let auxiliary_inspector_session_ids = target_id
            .as_deref()
            .map(|target_id| self.auxiliary_session_ids_for_target(target_id))
            .unwrap_or_default();
        if let Some(page) = self.active_page_target_mut().runtime_slot.loaded_page_mut() {
            for session_id in &auxiliary_inspector_session_ids {
                page.detach_runtime_inspector_session_async(Some(session_id))
                    .await
                    .map_err(|error| {
                        format!("failed to detach auxiliary renderer inspector session: {error}")
                    })?;
            }
            page.detach_runtime_inspector_session_async(None)
                .await
                .map_err(|error| {
                    format!("failed to detach primary renderer inspector session: {error}")
                })?;
        }
        self.clear_active_target_session_scoped_state_async()
            .await?;
        self.detach_active_session();
        for session_id in auxiliary_inspector_session_ids {
            self.active_page_target_mut()
                .runtime_slot
                .remove_auxiliary_network_session(&session_id);
        }
        Ok(())
    }

    pub(crate) async fn clear_active_target_primary_auto_attached_session_async(
        &mut self,
    ) -> Result<Option<String>, String> {
        let target_id = self.active_target_id_owned();
        if self.active_session_id().is_none() {
            return Ok(None);
        }
        self.clear_active_target_primary_session_scoped_state_async()
            .await?;
        self.detach_active_session();
        Ok(target_id)
    }

    pub(crate) fn release_primary_session_binding_preserving_frontend_state(
        &mut self,
        session_id: &str,
    ) -> bool {
        if self.active_session_id() == Some(session_id) {
            self.active_page_target_mut()
                .runtime_slot
                .disable_primary_network_events();
            self.detach_active_session();
            return true;
        }
        let Some(target) = self
            .background_targets_mut()
            .find(|target| target.is_session(session_id))
        else {
            return false;
        };
        target.runtime_slot.disable_primary_network_events();
        target.detach_session();
        true
    }

    #[cfg(test)]
    pub(crate) async fn promote_first_background_target_to_active_async(
        &mut self,
    ) -> Option<String> {
        let promoted = self
            .background_targets()
            .find(|target| target.has_loaded_page())
            .map(|target| target.target_id().to_owned())
            .or_else(|| {
                self.background_targets()
                    .next()
                    .map(|target| target.target_id().to_owned())
            })?;
        self.promote_selected_background_target_to_active_async(promoted)
            .await
    }

    pub(crate) async fn promote_last_background_target_to_active_async(
        &mut self,
    ) -> Option<String> {
        let promoted = self.last_promotable_background_target_id()?;
        self.promote_selected_background_target_to_active_async(promoted)
            .await
    }

    pub(crate) fn last_promotable_background_target_id(&self) -> Option<String> {
        self.background_targets()
            .rev()
            .find(|target| target.has_loaded_page())
            .map(|target| target.target_id().to_owned())
            .or_else(|| {
                self.background_targets()
                    .next_back()
                    .map(|target| target.target_id().to_owned())
            })
    }

    async fn promote_selected_background_target_to_active_async(
        &mut self,
        promoted: String,
    ) -> Option<String> {
        self.promote_background_target_to_active_slot_async(&promoted)
            .await
            .expect("applying promoted target visibility should succeed")
            .then_some(promoted)
    }

    pub(crate) async fn promote_background_target_to_active_slot_async(
        &mut self,
        target_id: &str,
    ) -> Result<bool, String> {
        if self.is_active_target(target_id) {
            return Ok(true);
        }
        if self.background_target(target_id).is_none() {
            return Ok(false);
        }
        let synchronize_loaded_page = self
            .page_targets
            .active()
            .is_none_or(|host| !host.has_pending_javascript_dialog())
            && self
                .page_target(target_id)
                .is_none_or(|host| !host.has_pending_javascript_dialog());
        let previous_active_target_id = self.active_target_id_owned();
        let demoted_surface_script = if synchronize_loaded_page {
            previous_active_target_id.as_deref().and_then(|target_id| {
                let host = self.page_target(target_id)?;
                self.generated_surface_override_script_for_parked_state(host)
            })
        } else {
            None
        };
        let selected = self.page_targets.select(target_id);
        debug_assert!(selected, "existing page target must be selectable");
        if synchronize_loaded_page {
            self.apply_surface_overrides_to_loaded_page_async().await?;
        }
        if let (Some(previous_active_target_id), Some(script)) =
            (previous_active_target_id, demoted_surface_script)
            && let Some(page) = self
                .page_target_mut(&previous_active_target_id)
                .and_then(|host| host.runtime_slot.loaded_page_mut())
            && let Err(error) = page
                .run_page_surface_override_script_async(&script.source)
                .await
        {
            tracing::warn!(target_id = previous_active_target_id, %error, "failed to update demoted page visibility");
        }
        Ok(true)
    }

    #[cfg(test)]
    pub(crate) async fn demote_active_target_to_background_slot_async(
        &mut self,
    ) -> Result<bool, String> {
        let Some(active_target_id) = self.active_target_id_owned() else {
            return Ok(false);
        };
        let surface_script = self
            .page_target(&active_target_id)
            .and_then(|host| self.generated_surface_override_script_for_parked_state(host));
        if let Some(script) = surface_script
            && let Some(page) = self
                .page_target_mut(&active_target_id)
                .and_then(|host| host.runtime_slot.loaded_page_mut())
        {
            page.run_page_surface_override_script_async(&script.source)
                .await
                .map_err(|error| error.to_string())?;
        }
        self.clear_active_target_id();
        Ok(true)
    }

    pub fn take_parked_document_start_script_counter(&mut self, target_id: &str) -> u32 {
        let owner_state = &mut self
            .background_target_mut(target_id)
            .expect("parked target must exist")
            .owner_state;
        let counter = owner_state.next_document_start_script_id;
        owner_state.next_document_start_script_id = 0;
        counter
    }

    pub fn replace_parked_document_start_script_counter(
        &mut self,
        target_id: String,
        counter: u32,
    ) {
        self.background_target_mut(&target_id)
            .expect("parked target must exist")
            .owner_state
            .next_document_start_script_id = counter;
    }

    pub(crate) fn parked_page_session_state(
        &self,
        target_id: &str,
    ) -> Option<&crate::conn::state::PageTargetHost> {
        let state = self.background_target(target_id)?;
        state.has_non_default_session_state().then_some(state)
    }

    pub fn mutate_parked_page_session_state<T>(
        &mut self,
        target_id: &str,
        mutate: impl FnOnce(&mut crate::conn::state::PageTargetHost) -> T,
    ) -> T {
        mutate(
            self.background_target_mut(target_id)
                .expect("parked target must exist"),
        )
    }

    pub(crate) fn begin_active_target_initial_empty_document(&mut self, initial_url: String) {
        self.begin_active_target_initial_empty_document_with_storage_key(initial_url, None);
    }

    pub(crate) fn begin_active_target_initial_empty_document_with_storage_key(
        &mut self,
        initial_url: String,
        storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) {
        let Some(target_id) = self.active_target_id_owned() else {
            return;
        };
        self.active_page_target_mut()
            .runtime_slot
            .mark_loaded_page_absent(TargetPageAbsenceReason::InitialDocumentPageBuildPending);
        self.active_page_target_mut()
            .owner_state
            .begin_initial_empty_document(target_id, initial_url, None, storage_key);
    }

    #[cfg(test)]
    pub(crate) fn mark_target_initial_empty_document_materialized(&mut self, target_id: &str) {
        self.mutate_target_owner_state_by_target_id(target_id, |owner_state| {
            owner_state.mark_initial_empty_document_materialized();
        });
    }

    pub(crate) fn mark_target_initial_url_replaces_empty_document(&mut self, target_id: &str) {
        self.mutate_target_owner_state_by_target_id(target_id, |owner_state| {
            owner_state.mark_next_navigation_history_replace_initial_empty_document();
        });
    }

    pub(crate) fn mark_target_initial_empty_document_pending_cross_document_navigation(
        &mut self,
        target_id: &str,
    ) {
        self.mutate_target_owner_state_by_target_id(target_id, |owner_state| {
            owner_state.mark_initial_empty_document_pending_cross_document_navigation();
        });
    }

    pub(crate) fn clear_target_initial_empty_document_pending_cross_document_navigation(
        &mut self,
        target_id: &str,
    ) {
        self.mutate_target_owner_state_by_target_id(target_id, |owner_state| {
            owner_state.clear_initial_empty_document_pending_cross_document_navigation();
        });
    }

    pub(crate) fn mark_target_initial_empty_document_exited(&mut self, target_id: &str) {
        self.mutate_target_owner_state_by_target_id(target_id, |owner_state| {
            owner_state.mark_initial_empty_document_exited();
        });
    }

    fn mutate_target_owner_state_by_target_id<T>(
        &mut self,
        target_id: &str,
        mutate: impl FnOnce(&mut TargetOwnerState) -> T,
    ) -> Option<T> {
        if self.is_active_target(target_id) {
            return Some(mutate(&mut self.active_page_target_mut().owner_state));
        }
        self.background_target(target_id)?;
        Some(self.mutate_parked_target_owner_state(target_id, mutate))
    }

    pub(crate) fn mutate_parked_target_owner_state<T>(
        &mut self,
        target_id: &str,
        mutate: impl FnOnce(&mut TargetOwnerState) -> T,
    ) -> T {
        mutate(
            &mut self
                .background_target_mut(target_id)
                .expect("parked target must exist")
                .owner_state,
        )
    }

    pub(crate) fn parked_target_owner_state(&self, target_id: &str) -> Option<&TargetOwnerState> {
        let state = &self.background_target(target_id)?.owner_state;
        (!state.is_default()).then_some(state)
    }

    #[cfg(test)]
    pub(crate) fn parked_target_owner_state_or_default(&self, target_id: &str) -> TargetOwnerState {
        self.background_target(target_id)
            .map(|target| target.owner_state.clone())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn parked_fetch_state(
        &self,
        target_id: &str,
    ) -> Option<&crate::conn::state::TargetFetchState> {
        let state = self
            .background_target(target_id)?
            .fetch_owner
            .pending_state();
        (!state.is_empty()).then_some(state)
    }

    #[cfg(test)]
    pub(crate) fn target_info(&self, target_id: &str) -> Option<serde_json::Value> {
        self.devtools_target_info(target_id)
            .map(DevToolsTargetInfo::into_cdp_value)
    }

    pub(crate) fn devtools_target_info(&self, target_id: &str) -> Option<DevToolsTargetInfo> {
        if self.is_active_target(target_id) {
            let attached = self.has_active_session()
                || !self.auxiliary_session_ids_for_target(target_id).is_empty();
            return Some(DevToolsTargetInfo {
                target_id: Some(DevToolsTargetId::from(target_id)),
                kind: DevToolsTargetKind::Page,
                title: self
                    .active_page_target()
                    .owner_state
                    .committed_document_title()
                    .map(str::to_owned)
                    .or_else(|| self.loaded_page().map(|page| page.document_title()))
                    .unwrap_or_default(),
                url: self.target_url().to_owned(),
                attached,
                opener_id: self
                    .target_opener_ids
                    .get(target_id)
                    .map(|id| DevToolsTargetId::from(id.as_str())),
                opener_frame_id: self
                    .target_opener_frame_ids
                    .get(target_id)
                    .map(|id| crate::devtools_runtime::DevToolsFrameId::from(id.as_str())),
                can_access_opener: self.target_can_access_opener.contains(target_id),
                browser_context_id: Some(DevToolsBrowserContextId::from(self.id.as_str())),
                moli_popup_id: None,
            });
        }

        if let Some(target) = self.shared_worker_target(target_id) {
            return Some(self.shared_worker_devtools_target_info(target));
        }

        if let Some(target) = self.dedicated_worker_target(target_id) {
            return Some(self.dedicated_worker_devtools_target_info(target));
        }

        if let Some(target) = self.service_worker_target(target_id) {
            return Some(self.service_worker_devtools_target_info(target));
        }

        let target = self.background_target(target_id)?;
        let attached = target.has_session()
            || !self
                .auxiliary_session_ids_for_target(target.target_id())
                .is_empty();
        Some(DevToolsTargetInfo {
            target_id: Some(DevToolsTargetId::from(target.target_id())),
            kind: DevToolsTargetKind::Page,
            title: self
                .parked_target_owner_state(target.target_id())
                .and_then(|owner_state| owner_state.committed_document_title())
                .map(str::to_owned)
                .or_else(|| target.loaded_page().map(|page| page.document_title()))
                .unwrap_or_default(),
            url: target.target_url().to_owned(),
            attached,
            opener_id: self
                .target_opener_ids
                .get(target.target_id())
                .map(|id| DevToolsTargetId::from(id.as_str())),
            opener_frame_id: self
                .target_opener_frame_ids
                .get(target.target_id())
                .map(|id| crate::devtools_runtime::DevToolsFrameId::from(id.as_str())),
            can_access_opener: self.target_can_access_opener.contains(target.target_id()),
            browser_context_id: Some(DevToolsBrowserContextId::from(self.id.as_str())),
            moli_popup_id: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn target_infos(&self) -> Vec<serde_json::Value> {
        self.devtools_target_infos()
            .into_iter()
            .map(DevToolsTargetInfo::into_cdp_value)
            .collect()
    }

    pub(crate) fn devtools_target_infos(&self) -> Vec<DevToolsTargetInfo> {
        let mut infos = Vec::new();
        if let Some(target_id) = self.active_target_id() {
            infos.push(
                self.devtools_target_info(target_id)
                    .expect("active target must remain addressable"),
            );
        }
        infos.extend(
            self.background_targets()
                .filter_map(|target| self.devtools_target_info(target.target_id())),
        );
        infos.extend(
            self.shared_worker_targets
                .values()
                .map(|target| self.shared_worker_devtools_target_info(target)),
        );
        infos.extend(
            self.dedicated_worker_targets
                .values()
                .map(|target| self.dedicated_worker_devtools_target_info(target)),
        );
        infos.extend(
            self.service_worker_targets
                .values()
                .map(|target| self.service_worker_devtools_target_info(target)),
        );
        infos
    }

    pub(crate) fn shared_worker_target(&self, target_id: &str) -> Option<&SharedWorkerTargetState> {
        self.shared_worker_targets
            .values()
            .find(|target| target.target_id == target_id)
    }

    pub(crate) fn shared_worker_target_mut(
        &mut self,
        target_id: &str,
    ) -> Option<&mut SharedWorkerTargetState> {
        self.shared_worker_targets
            .values_mut()
            .find(|target| target.target_id == target_id)
    }

    pub(crate) fn shared_worker_target_id_for_session(&self, session_id: &str) -> Option<&str> {
        self.shared_worker_targets
            .values()
            .find(|target| target.is_session(session_id))
            .map(|target| target.target_id.as_str())
    }

    pub(crate) fn has_shared_worker_target(&self, target_id: &str) -> bool {
        self.shared_worker_target(target_id).is_some()
    }

    pub(crate) fn has_any_shared_worker_targets(&self) -> bool {
        !self.shared_worker_targets.is_empty()
    }

    pub(crate) fn shared_worker_target_id_for_renderer_instance(
        &self,
        renderer_instance_id: moli_shared_worker::SharedWorkerInstanceId,
    ) -> Option<&str> {
        self.shared_worker_targets
            .get(&renderer_instance_id)
            .map(|target| target.target_id.as_str())
    }

    pub(crate) fn insert_shared_worker_target(
        &mut self,
        target: SharedWorkerTargetState,
    ) -> serde_json::Value {
        let target_info = self.shared_worker_target_info(&target);
        self.shared_worker_targets
            .insert(target.renderer_instance_id, target);
        target_info
    }

    pub(crate) fn remove_shared_worker_target_by_renderer_instance(
        &mut self,
        renderer_instance_id: moli_shared_worker::SharedWorkerInstanceId,
    ) -> Option<SharedWorkerTargetState> {
        self.shared_worker_targets.remove(&renderer_instance_id)
    }

    pub(crate) fn assign_session_to_shared_worker_target(
        &mut self,
        target_id: &str,
        session_id: String,
    ) -> bool {
        let Some(target) = self.shared_worker_target_mut(target_id) else {
            return false;
        };
        target.attach_session(session_id);
        true
    }

    pub(crate) fn detach_shared_worker_target_session(
        &mut self,
        session_id: &str,
    ) -> Option<String> {
        let target = self
            .shared_worker_targets
            .values_mut()
            .find(|target| target.is_session(session_id))?;
        let target_id = target.target_id.clone();
        target.detach_session(session_id);
        Some(target_id)
    }

    pub(crate) fn dedicated_worker_target(
        &self,
        target_id: &str,
    ) -> Option<&DedicatedWorkerTargetState> {
        self.dedicated_worker_targets
            .values()
            .find(|target| target.target_id == target_id)
    }

    pub(crate) fn dedicated_worker_target_mut(
        &mut self,
        target_id: &str,
    ) -> Option<&mut DedicatedWorkerTargetState> {
        self.dedicated_worker_targets
            .values_mut()
            .find(|target| target.target_id == target_id)
    }

    pub(crate) fn dedicated_worker_target_id_for_session(&self, session_id: &str) -> Option<&str> {
        self.dedicated_worker_targets
            .values()
            .find(|target| target.is_session(session_id))
            .map(|target| target.target_id.as_str())
    }

    pub(crate) fn has_dedicated_worker_target(&self, target_id: &str) -> bool {
        self.dedicated_worker_target(target_id).is_some()
    }

    pub(crate) fn has_any_dedicated_worker_targets(&self) -> bool {
        !self.dedicated_worker_targets.is_empty()
    }

    pub(crate) fn target_page_residence_is_current(
        &self,
        expected: &crate::conn::TargetPageResidenceIdentity,
    ) -> bool {
        if expected.browser_context_id() != self.id {
            return false;
        }
        let current_attachment = match expected.target_id() {
            Some(target_id) if self.is_active_target(target_id) => {
                self.active_page_target().runtime_slot.page_attachment_id()
            }
            Some(target_id) => self
                .background_target(target_id)
                .and_then(|target| target.runtime_slot.page_attachment_id()),
            None if self.active_target_id().is_none() => {
                self.active_page_target().runtime_slot.page_attachment_id()
            }
            None => None,
        };
        current_attachment == Some(expected.page_attachment_id())
    }

    pub(crate) fn dedicated_worker_target_id_for_renderer_instance(
        &self,
        renderer_instance_id: u64,
    ) -> Option<&str> {
        self.dedicated_worker_targets
            .get(&renderer_instance_id)
            .map(|target| target.target_id.as_str())
    }

    pub(crate) fn insert_dedicated_worker_target(
        &mut self,
        target: DedicatedWorkerTargetState,
    ) -> serde_json::Value {
        let target_info = self.dedicated_worker_target_info(&target);
        self.dedicated_worker_targets
            .insert(target.renderer_instance_id, target);
        target_info
    }

    pub(crate) fn remove_dedicated_worker_target_by_renderer_instance(
        &mut self,
        renderer_instance_id: u64,
    ) -> Option<DedicatedWorkerTargetState> {
        self.dedicated_worker_targets.remove(&renderer_instance_id)
    }

    pub(crate) fn assign_session_to_dedicated_worker_target(
        &mut self,
        target_id: &str,
        session_id: String,
    ) -> bool {
        let Some(renderer_instance_id) = self
            .dedicated_worker_target(target_id)
            .map(|target| target.renderer_instance_id)
        else {
            return false;
        };
        self.dedicated_worker_target_mut(target_id)
            .expect("dedicated worker target must remain registered while attaching")
            .attach_session(session_id.clone());
        // The target may close between discovery and attachment. Keep the CDP
        // binding observable so normal target retirement can detach it, while
        // best-effort registering the live renderer session before the attach
        // event is published.
        let _ = self
            .renderer_runtime()
            .attach_dedicated_worker_runtime_inspector_session(
                renderer_instance_id,
                Some(session_id),
            );
        true
    }

    pub(crate) fn detach_dedicated_worker_target_session(
        &mut self,
        session_id: &str,
    ) -> Option<String> {
        let target = self
            .dedicated_worker_targets
            .values_mut()
            .find(|target| target.is_session(session_id))?;
        let target_id = target.target_id.clone();
        target.detach_session(session_id);
        Some(target_id)
    }

    pub(crate) fn service_worker_target(
        &self,
        target_id: &str,
    ) -> Option<&ServiceWorkerTargetState> {
        self.service_worker_targets
            .values()
            .find(|target| target.target_id == target_id)
    }

    pub(crate) fn service_worker_target_mut(
        &mut self,
        target_id: &str,
    ) -> Option<&mut ServiceWorkerTargetState> {
        self.service_worker_targets
            .values_mut()
            .find(|target| target.target_id == target_id)
    }

    pub(crate) fn service_worker_target_id_for_session(&self, session_id: &str) -> Option<&str> {
        self.service_worker_targets
            .values()
            .find(|target| target.is_session(session_id))
            .map(|target| target.target_id.as_str())
    }

    pub(crate) fn has_service_worker_target(&self, target_id: &str) -> bool {
        self.service_worker_target(target_id).is_some()
    }

    pub(crate) fn has_any_service_worker_targets(&self) -> bool {
        !self.service_worker_targets.is_empty()
    }

    pub(crate) fn set_service_worker_domain_enabled(
        &mut self,
        session_id: Option<&str>,
        enabled: bool,
    ) {
        let key = session_id.map(str::to_owned);
        if enabled {
            self.service_worker_domain_sessions.insert(key);
        } else {
            self.service_worker_domain_sessions.remove(&key);
        }
    }

    pub(crate) fn service_worker_domain_enabled_sessions(&self) -> Vec<Option<String>> {
        self.service_worker_domain_sessions
            .iter()
            .cloned()
            .collect()
    }

    pub(crate) fn service_worker_target_id_for_renderer_version(
        &self,
        renderer_version_id: u64,
    ) -> Option<&str> {
        self.service_worker_targets
            .get(&renderer_version_id)
            .map(|target| target.target_id.as_str())
    }

    pub(crate) fn insert_service_worker_target(
        &mut self,
        target: ServiceWorkerTargetState,
    ) -> serde_json::Value {
        let target_info = self.service_worker_target_info(&target);
        self.service_worker_targets
            .insert(target.renderer_version_id, target);
        target_info
    }

    pub(crate) fn remove_service_worker_target_by_renderer_version(
        &mut self,
        renderer_version_id: u64,
    ) -> Option<ServiceWorkerTargetState> {
        self.service_worker_targets.remove(&renderer_version_id)
    }

    pub(crate) fn assign_session_to_service_worker_target(
        &mut self,
        target_id: &str,
        session_id: String,
    ) -> bool {
        let attached_version_id = {
            let Some(target) = self.service_worker_target_mut(target_id) else {
                return false;
            };
            let was_attached = target.has_session();
            target.attach_session(session_id);
            (!was_attached).then_some(target.renderer_version_id)
        };
        if let Some(version_id) = attached_version_id {
            self.renderer_runtime()
                .set_service_worker_devtools_attached(version_id, true);
        };
        true
    }

    pub(crate) fn detach_service_worker_target_session(
        &mut self,
        session_id: &str,
    ) -> Option<String> {
        let (target_id, detached_version_id) = {
            let target = self
                .service_worker_targets
                .values_mut()
                .find(|target| target.is_session(session_id))?;
            let target_id = target.target_id.clone();
            let version_id = target.renderer_version_id;
            target.detach_session(session_id);
            let detached_version_id = (!target.has_session()).then_some(version_id);
            (target_id, detached_version_id)
        };
        if let Some(version_id) = detached_version_id {
            self.renderer_runtime()
                .set_service_worker_devtools_attached(version_id, false);
        }
        Some(target_id)
    }

    fn shared_worker_target_info(&self, target: &SharedWorkerTargetState) -> serde_json::Value {
        self.shared_worker_devtools_target_info(target)
            .into_cdp_value()
    }

    fn dedicated_worker_target_info(
        &self,
        target: &DedicatedWorkerTargetState,
    ) -> serde_json::Value {
        self.dedicated_worker_devtools_target_info(target)
            .into_cdp_value()
    }

    fn service_worker_target_info(&self, target: &ServiceWorkerTargetState) -> serde_json::Value {
        self.service_worker_devtools_target_info(target)
            .into_cdp_value()
    }

    fn shared_worker_devtools_target_info(
        &self,
        target: &SharedWorkerTargetState,
    ) -> DevToolsTargetInfo {
        DevToolsTargetInfo {
            target_id: Some(DevToolsTargetId::from(target.target_id.as_str())),
            kind: DevToolsTargetKind::SharedWorker,
            title: target.name.clone(),
            url: target.url.clone(),
            attached: target.has_session(),
            opener_id: None,
            opener_frame_id: None,
            can_access_opener: false,
            browser_context_id: Some(DevToolsBrowserContextId::from(self.id.as_str())),
            moli_popup_id: None,
        }
    }

    fn dedicated_worker_devtools_target_info(
        &self,
        target: &DedicatedWorkerTargetState,
    ) -> DevToolsTargetInfo {
        let title = if target.main_script().is_none() {
            String::new()
        } else if target.name.is_empty() {
            target.url.clone()
        } else {
            target.name.clone()
        };
        DevToolsTargetInfo {
            target_id: Some(DevToolsTargetId::from(target.target_id.as_str())),
            kind: DevToolsTargetKind::Worker,
            title,
            url: target.url.clone(),
            attached: target.has_session(),
            opener_id: target.owner_page.target_id().map(DevToolsTargetId::from),
            opener_frame_id: None,
            can_access_opener: false,
            browser_context_id: Some(DevToolsBrowserContextId::from(self.id.as_str())),
            moli_popup_id: None,
        }
    }

    fn service_worker_devtools_target_info(
        &self,
        target: &ServiceWorkerTargetState,
    ) -> DevToolsTargetInfo {
        DevToolsTargetInfo {
            target_id: Some(DevToolsTargetId::from(target.target_id.as_str())),
            kind: DevToolsTargetKind::ServiceWorker,
            title: format!("Service Worker {}", target.script_url),
            url: target.script_url.clone(),
            attached: target.has_session(),
            opener_id: None,
            opener_frame_id: None,
            can_access_opener: false,
            browser_context_id: Some(DevToolsBrowserContextId::from(self.id.as_str())),
            moli_popup_id: None,
        }
    }
}

fn background_target_identity_for_initial_url(
    url: &str,
    creator: Option<&TargetInitialEmptyDocumentCreator>,
) -> TargetIdentityState {
    let Some(creator) = creator else {
        return TargetIdentityState::with_url(url.to_owned());
    };
    if url::Url::parse(url)
        .ok()
        .as_ref()
        .is_some_and(moli_url::is_about_blank)
    {
        return TargetIdentityState::new(
            url.to_owned(),
            creator.security_origin().to_owned(),
            creator.secure_context_type().to_owned(),
        );
    }
    TargetIdentityState::with_url(url.to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::conn::state::{PerformanceTimeDomain, TargetPerformanceSessionState};
    use crate::conn::{
        DevToolsSessionState, DocumentStartScript, FetchInterceptionPattern, FetchRequestStage,
        TargetPageSessionState, TargetRuntimeSessionState,
    };
    use crate::testing::TestContext;
    use serde_json::json;

    #[test]
    fn independent_top_level_targets_isolate_session_storage_but_share_local_storage() {
        let mut context = BrowserContext::new("BC-storage".to_owned());
        context.set_active_target_id("TID-first");
        let first_storage = context.page_storage_handles();
        {
            let mut local_storage = first_storage.web_storage_store.lock();
            assert!(local_storage.set_item("https://same.test", "local", "shared"));
        }
        {
            let mut session_storage = first_storage.session_storage_store.lock();
            assert!(session_storage.set_item("https://same.test", "session", "first"));
        }

        context.stage_background_target(
            "TID-second".to_owned(),
            None,
            "https://same.test/".to_owned(),
            None,
            None,
        );
        let second_storage = context
            .page_storage_handles_for_target("TID-second")
            .expect("staged target should own storage");

        assert!(Arc::ptr_eq(
            &first_storage.web_storage_store,
            &second_storage.web_storage_store
        ));
        assert!(!Arc::ptr_eq(
            &first_storage.session_storage_store,
            &second_storage.session_storage_store
        ));
        assert_eq!(
            second_storage
                .web_storage_store
                .lock()
                .get_item("https://same.test", "local"),
            Some("shared".to_owned())
        );
        assert_eq!(
            second_storage
                .session_storage_store
                .lock()
                .get_item("https://same.test", "session"),
            None
        );
    }

    #[test]
    fn popup_clones_opener_session_storage_without_sharing_later_mutations() {
        let mut context = BrowserContext::new("BC-popup-storage".to_owned());
        context.set_active_target_id("TID-opener");
        let opener_storage = context.page_storage_handles();
        assert!(opener_storage.session_storage_store.lock().set_item(
            "https://same.test",
            "session",
            "opener"
        ));
        let creator = context
            .initial_empty_document_creator_for_target("TID-opener")
            .expect("active target should describe popup creator");

        context.stage_background_target(
            "TID-popup".to_owned(),
            None,
            "about:blank".to_owned(),
            None,
            Some(creator),
        );
        let popup_storage = context
            .page_storage_handles_for_target("TID-popup")
            .expect("popup target should own storage");

        assert!(!Arc::ptr_eq(
            &opener_storage.session_storage_store,
            &popup_storage.session_storage_store
        ));
        assert_eq!(
            popup_storage
                .session_storage_store
                .lock()
                .get_item("https://same.test", "session"),
            Some("opener".to_owned())
        );
        assert!(popup_storage.session_storage_store.lock().set_item(
            "https://same.test",
            "session",
            "popup"
        ));
        assert_eq!(
            opener_storage
                .session_storage_store
                .lock()
                .get_item("https://same.test", "session"),
            Some("opener".to_owned())
        );
    }

    #[test]
    fn demoted_target_retains_its_session_storage_namespace() {
        let mut context = BrowserContext::new("BC-demoted-storage".to_owned());
        context.set_active_target_id("TID-first");
        let first_session_storage = context.page_storage_handles().session_storage_store.clone();
        assert!(
            first_session_storage
                .lock()
                .set_item("https://same.test", "session", "first")
        );

        context.stage_active_target_demoting_current(
            "TID-second".to_owned(),
            None,
            "about:blank".to_owned(),
            None,
        );
        let parked_first_storage = context
            .page_storage_handles_for_target("TID-first")
            .expect("demoted target should retain storage");
        let second_storage = context.page_storage_handles();

        assert!(Arc::ptr_eq(
            &first_session_storage,
            &parked_first_storage.session_storage_store
        ));
        assert!(!Arc::ptr_eq(
            &first_session_storage,
            &second_storage.session_storage_store
        ));
        assert_eq!(
            parked_first_storage
                .session_storage_store
                .lock()
                .get_item("https://same.test", "session"),
            Some("first".to_owned())
        );
    }

    #[test]
    fn replacing_active_target_releases_its_session_storage_namespace() {
        let mut context = BrowserContext::new("BC-replaced-storage".to_owned());
        context.set_active_target_id("TID-first");
        let first_session_storage = context.page_storage_handles().session_storage_store.clone();
        assert!(
            first_session_storage
                .lock()
                .set_item("https://same.test", "session", "first")
        );

        context.clear_active_target_id();
        context.set_active_target_id("TID-second");
        let second_session_storage = context.page_storage_handles().session_storage_store.clone();

        assert!(!Arc::ptr_eq(
            &first_session_storage,
            &second_session_storage
        ));
        assert_eq!(
            second_session_storage
                .lock()
                .get_item("https://same.test", "session"),
            None
        );
    }

    #[test]
    fn window_open_target_registry_preserves_named_target_bytes() {
        assert_eq!(
            BrowserContext::reusable_window_open_target_name("_BlAnK"),
            None
        );
        assert_eq!(
            BrowserContext::reusable_window_open_target_name(" _blank "),
            Some(" _blank ".to_owned())
        );
        assert_eq!(
            BrowserContext::reusable_window_open_target_name("ReportWindow"),
            Some("ReportWindow".to_owned())
        );

        let mut context = BrowserContext::new("BC-window-name".to_owned());
        context.remember_target_window_name(" ReportWindow ", "TID-spaced");
        context.remember_target_window_name("ReportWindow", "TID-exact");
        assert_eq!(
            context.target_id_for_window_name(" ReportWindow "),
            Some("TID-spaced")
        );
        assert_eq!(
            context.target_id_for_window_name("ReportWindow"),
            Some("TID-exact")
        );
        assert_eq!(context.target_id_for_window_name("reportwindow"), None);
    }

    #[test]
    fn page_target_host_keeps_protocol_and_owner_state_together() {
        let mut context = BrowserContext::new("BC-1".to_owned());
        context.stage_background_target(
            "TID-bg".to_owned(),
            Some("SID-bg".to_owned()),
            "https://bg.test/".to_owned(),
            None,
            None,
        );
        context.replace_parked_document_start_script_counter("TID-bg".to_owned(), 7);
        context.mutate_parked_page_session_state("TID-bg", |state| {
            state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary] =
                DevToolsSessionState {
                    runtime_session_state: TargetRuntimeSessionState {
                        runtime_frontend_enabled: true,
                        ..Default::default()
                    },
                    ..Default::default()
                };
        });

        let host = context
            .background_target("TID-bg")
            .expect("page target host should remain registered");

        assert_eq!(host.target_id(), "TID-bg");
        assert_eq!(host.owner_state.next_document_start_script_id, 7);
        assert!(
            host.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled
        );
        assert_eq!(context.background_target_count(), 1);
        assert_eq!(
            context.background_target_at(0).unwrap().target_id(),
            "TID-bg"
        );
        assert!(
            context
                .parked_page_session_state("TID-bg")
                .is_some_and(|state| state.devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .runtime_session_state
                    .runtime_frontend_enabled)
        );
        assert_eq!(
            context.take_parked_document_start_script_counter("TID-bg"),
            7
        );
    }

    #[test]
    fn remove_auxiliary_session_drops_session_local_fetch_config() {
        let mut context = BrowserContext::new("BC-1".to_owned());
        context.set_active_target_id("TID-active".to_owned());
        context.attach_active_session("SID-active".to_owned());
        assert!(
            context.assign_auxiliary_session_to_target("TID-active", "SID-aux".to_owned()),
            "auxiliary session should attach to active target"
        );
        context.active_page_state_mut().fetch_owner.configure(
            Some("SID-aux".to_owned()),
            false,
            vec![FetchInterceptionPattern {
                url_pattern: "*".to_owned(),
                resource_type_filter: None,
                request_stage: FetchRequestStage::Request,
            }],
        );
        assert!(
            context
                .active_page_state()
                .fetch_owner
                .config_snapshot()
                .is_enabled()
        );

        assert_eq!(
            context.remove_auxiliary_session("SID-aux").as_deref(),
            Some("TID-active")
        );

        assert!(
            !context
                .active_page_state()
                .fetch_owner
                .config_snapshot()
                .is_enabled(),
            "detaching the auxiliary target session must remove its Fetch.enable config"
        );
    }

    #[test]
    fn clearing_selection_keeps_the_page_target_host_and_its_state() {
        let mut context = BrowserContext::new("BC-1".to_owned());
        context.set_active_target_id("TID-active".to_owned());
        context.attach_active_session("SID-active".to_owned());
        context.set_target_url("https://active.test/".to_owned());
        context.set_target_security_origin("https://active.test".to_owned());
        context.set_target_secure_context_type("Secure".to_owned());
        context
            .active_page_state_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(42);
        context.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
        context.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .inspector_enabled = true;
        context
            .active_page_state_mut()
            .owner_state
            .next_document_start_script_id = 3;
        context
            .active_page_state_mut()
            .owner_state
            .document_start_scripts
            .push((
                "script-1".to_owned(),
                DocumentStartScript {
                    registry_key: None,
                    devtools_session: None,
                    source: "globalThis.fromDocumentStart = true".to_owned(),
                    world_name: None,
                    has_bidi_channel_argument: false,
                    bidi_channel_handoffs: Vec::new(),
                },
            ));

        context.clear_active_target_id();
        let host = context
            .page_target("TID-active")
            .expect("clearing foreground selection must not remove the host");

        assert_eq!(host.target_id(), "TID-active");
        assert_eq!(host.session_id(), Some("SID-active"));
        assert_eq!(host.target_url(), "https://active.test/");
        assert_eq!(
            host.target_identity().security_origin(),
            "https://active.test"
        );
        assert_eq!(host.target_identity().secure_context_type(), "Secure");
        assert_eq!(
            host.page_attachment_id()
                .map(|attachment_id| attachment_id.get()),
            Some(42)
        );
        assert_eq!(host.owner_state.document_start_scripts.len(), 1);
        assert!(
            host.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled
        );
        assert!(
            host.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled
        );
        assert_eq!(host.owner_state.next_document_start_script_id, 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn active_target_demote_to_background_slot_preserves_page_session_and_owner_state() {
        let mut ctx = TestContext::new();
        let active_page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<title>demote-active</title>")
            .await
            .expect("active page should load");

        let mut context = BrowserContext::new("BC-demote".to_owned());
        context.set_active_target_id("TID-demote".to_owned());
        context.attach_active_session("SID-demote".to_owned());
        context.set_target_url(active_page.final_url().as_str().to_owned());
        context.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
        context.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .inspector_enabled = true;
        context.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .console_enabled = true;
        context.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .log_enabled = true;
        assert!(
            context.active_page_state_mut().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .performance
                .enable(PerformanceTimeDomain::ThreadTicks)
        );
        context.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .page_lifecycle_events = true;
        context.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .page_file_chooser_opened_event_enabled = true;
        context.active_page_state_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .page_intercept_file_chooser_dialog_enabled = true;
        context
            .active_page_state_mut()
            .runtime_slot
            .set_primary_network_events_enabled(true);
        context
            .active_page_state_mut()
            .owner_state
            .next_document_start_script_id = 9;
        context
            .active_page_state_mut()
            .owner_state
            .document_start_scripts
            .push((
                "script-demote".to_owned(),
                DocumentStartScript {
                    registry_key: None,
                    devtools_session: None,
                    source: "globalThis.demoted = true".to_owned(),
                    world_name: None,
                    has_bidi_channel_argument: false,
                    bidi_channel_handoffs: Vec::new(),
                },
            ));
        context
            .active_page_state_mut()
            .runtime_slot
            .set_network_request_counters_for_test(77, 88);
        context
            .active_page_state_mut()
            .runtime_slot
            .mark_subresource_records_emitted(None, 0, 3);
        context.set_loaded_page_async(active_page).await;
        let active_attachment = context
            .active_page_state()
            .runtime_slot
            .current_renderer_attachment()
            .expect("loaded active page should have a renderer attachment");

        assert!(
            context
                .demote_active_target_to_background_slot_async()
                .await
                .expect("demote should not fail")
        );

        assert_eq!(context.active_target_id(), None);
        assert!(
            context
                .page_target("TID-demote")
                .is_some_and(|host| host.has_loaded_page()),
            "demoting should retain the loaded page in its stable host"
        );
        assert_eq!(context.background_target_count(), 1);
        let parked = &context.background_target_at(0).unwrap();
        assert_eq!(parked.target_id(), "TID-demote");
        assert_eq!(parked.session_id(), Some("SID-demote"));
        assert_eq!(
            parked.target_url(),
            "data:text/html,<title>demote-active</title>"
        );
        assert!(
            parked.has_loaded_page(),
            "demoting should move the loaded page back into the parked target"
        );
        assert_eq!(
            parked
                .runtime_slot()
                .current_renderer_attachment()
                .map(|attachment| attachment.id()),
            Some(active_attachment.id()),
            "demoting a target must move its renderer channel without allocating a new route lease"
        );
        assert_eq!(
            parked
                .loaded_page()
                .and_then(|page| page.renderer_agent_attachment_id()),
            Some(active_attachment.id()),
            "the parked Page and its renderer channel must retain the same attachment"
        );
        assert!(
            parked.runtime_slot.primary_network_events_enabled(),
            "demoted target runtime slot should mirror parked Network.enable state for direct owner backlog delivery"
        );
        assert!(
            context
                .parked_page_session_state("TID-demote")
                .is_some_and(|state| state.devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .runtime_session_state
                    .runtime_frontend_enabled),
            "session-scoped Runtime.enable state should be parked with the target"
        );
        let parked_page_session_state = context
            .parked_page_session_state("TID-demote")
            .expect("demoted target should retain parked page session state");
        assert!(
            parked_page_session_state.devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled,
            "Runtime.enable state should move with the demoted target"
        );
        assert!(
            parked_page_session_state.devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled,
            "Inspector.enable state should move with the demoted target"
        );
        assert!(
            parked_page_session_state.devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled,
            "Console.enable state should move with the demoted target"
        );
        assert!(
            parked_page_session_state.devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .log_enabled,
            "Log.enable state should move with the demoted target"
        );
        assert!(
            parked_page_session_state.devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .performance
                .enabled(),
            "Performance.enable state should move with the demoted target"
        );
        assert_eq!(
            parked_page_session_state.devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .performance
                .time_domain(),
            PerformanceTimeDomain::ThreadTicks,
            "Performance time domain should move with the demoted target"
        );
        assert!(
            parked_page_session_state.devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_lifecycle_events,
            "Page lifecycle listener state should move with the demoted target"
        );
        assert!(
            parked_page_session_state.devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_file_chooser_opened_event_enabled,
            "file chooser opened listener state should move with the demoted target"
        );
        assert!(
            parked_page_session_state.devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_intercept_file_chooser_dialog_enabled,
            "file chooser interception state should move with the demoted target"
        );
        assert_eq!(
            context.take_parked_document_start_script_counter("TID-demote"),
            9,
            "target owner state should be parked with the target"
        );
        assert_eq!(
            context
                .parked_target_owner_state("TID-demote")
                .expect("demoted target should retain owner state")
                .document_start_scripts
                .len(),
            1,
            "document-start scripts should be parked with the target owner state"
        );
        let parked_runtime = &context
            .background_target("TID-demote")
            .expect("demoted target should retain network state")
            .runtime_slot;
        assert_eq!(parked_runtime.next_fetch_request_id_for_test(), 77);
        assert_eq!(
            parked_runtime.next_subresource_fetch_request_id_for_test(),
            88
        );
        assert_eq!(
            parked_runtime.emitted_subresource_record_count_for_session_for_test(None),
            3,
            "target network artifacts should move into the parked target runtime slot"
        );
    }

    #[tokio::test]
    async fn active_target_demote_without_active_target_is_noop() {
        let mut context = BrowserContext::new("BC-demote-empty".to_owned());
        context.stage_background_target(
            "TID-existing-bg".to_owned(),
            Some("SID-existing-bg".to_owned()),
            "https://existing.test/".to_owned(),
            None,
            None,
        );

        assert!(
            !context
                .demote_active_target_to_background_slot_async()
                .await
                .expect("no-op demote should not fail")
        );

        assert_eq!(context.active_target_id(), None);
        assert_eq!(context.background_target_count(), 1);
        assert_eq!(
            context.background_target_at(0).unwrap().target_id(),
            "TID-existing-bg"
        );
        assert_eq!(
            context.background_target_at(0).unwrap().session_id(),
            Some("SID-existing-bg")
        );
    }

    #[test]
    fn active_target_demote_without_page_preserves_pending_initial_document_reason() {
        let mut context = BrowserContext::new("BC-demote-pending".to_owned());
        context.set_active_target_id("TID-old-active");
        context.set_target_url("about:blank#old".to_owned());
        context.begin_active_target_initial_empty_document("about:blank#old".to_owned());

        context.stage_active_target_demoting_current(
            "TID-new-active".to_owned(),
            Some("SID-new-active".to_owned()),
            "about:blank#new".to_owned(),
            Some("about:blank#new".to_owned()),
        );

        assert_eq!(
            context
                .background_target("TID-old-active")
                .expect("previous active target should be parked")
                .runtime_slot()
                .moli_memory_diagnostics()["loadedPageAbsenceReason"],
            json!("initial-document-page-build-pending"),
            "production demotion must preserve a pending initial document absence reason"
        );
        assert_eq!(
            context
                .active_page_state()
                .runtime_slot
                .moli_memory_diagnostics()["loadedPageAbsenceReason"],
            json!("initial-document-page-build-pending"),
            "the replacement target must expose its own pending initial document build"
        );
    }

    #[tokio::test]
    async fn background_target_promote_without_page_preserves_pending_initial_document_reason() {
        let mut context = BrowserContext::new("BC-promote-pending".to_owned());
        context.stage_background_target(
            "TID-pending-bg".to_owned(),
            Some("SID-pending-bg".to_owned()),
            "about:blank#pending".to_owned(),
            None,
            None,
        );

        assert!(
            context
                .promote_background_target_to_active_slot_async("TID-pending-bg")
                .await
                .expect("pending background target should still promote")
        );

        assert_eq!(
            context
                .active_page_state()
                .runtime_slot
                .moli_memory_diagnostics()["loadedPageAbsenceReason"],
            json!("initial-document-page-build-pending"),
            "production promotion must preserve a pending initial document absence reason"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn promote_first_background_target_prefers_first_loaded_target() {
        let mut ctx = TestContext::new();
        let first_loaded_page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<title>first-loaded</title>")
            .await
            .expect("first loaded page should load");
        let second_loaded_page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<title>second-loaded</title>")
            .await
            .expect("second loaded page should load");

        let mut context = BrowserContext::new("BC-promote-first".to_owned());
        context.stage_background_target(
            "TID-empty".to_owned(),
            Some("SID-empty".to_owned()),
            "https://empty.test/".to_owned(),
            None,
            None,
        );
        context.stage_background_target(
            "TID-first-loaded".to_owned(),
            Some("SID-first-loaded".to_owned()),
            "https://first-loaded.test/".to_owned(),
            None,
            None,
        );
        context.stage_background_target(
            "TID-second-loaded".to_owned(),
            Some("SID-second-loaded".to_owned()),
            "https://second-loaded.test/".to_owned(),
            None,
            None,
        );
        context
            .background_target_at_mut(1)
            .unwrap()
            .replace_loaded_page(Some(first_loaded_page));
        context
            .background_target_at_mut(2)
            .unwrap()
            .replace_loaded_page(Some(second_loaded_page));
        let first_attachment = context
            .background_target_at(1)
            .unwrap()
            .runtime_slot()
            .current_renderer_attachment()
            .expect("first loaded background target should have an attachment");
        let second_attachment = context
            .background_target_at(2)
            .unwrap()
            .runtime_slot()
            .current_renderer_attachment()
            .expect("second loaded background target should have an attachment");

        let promoted = context
            .promote_first_background_target_to_active_async()
            .await
            .expect("loaded background target should promote");

        assert_eq!(promoted, "TID-first-loaded");
        assert_eq!(context.active_target_id(), Some("TID-first-loaded"));
        assert_eq!(context.active_session_id(), Some("SID-first-loaded"));
        assert!(
            context.has_loaded_page(),
            "first loaded background target's page should become active"
        );
        assert_eq!(
            context
                .active_page_state()
                .runtime_slot
                .current_renderer_attachment()
                .map(|attachment| attachment.id()),
            Some(first_attachment.id()),
            "promotion must move the selected target's renderer channel with its Page"
        );
        assert!(
            context
                .background_target("TID-second-loaded")
                .is_some_and(PageTargetHost::has_loaded_page),
            "later loaded background target should remain parked"
        );
        assert_eq!(
            context
                .background_target("TID-second-loaded")
                .and_then(|target| target.runtime_slot().current_renderer_attachment())
                .map(|attachment| attachment.id()),
            Some(second_attachment.id()),
            "promoting one target must not replace another background target's route lease"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn active_background_swap_moves_each_target_renderer_channel_with_its_page() {
        let mut ctx = TestContext::new();
        let active_page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<title>active route</title>")
            .await
            .expect("active page should load");
        let background_page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<title>background route</title>")
            .await
            .expect("background page should load");
        let mut context = BrowserContext::new("BC-route-swap".to_owned());
        context.set_active_target_id("TID-active-route");
        context.attach_active_session("SID-active-route".to_owned());
        context.set_loaded_page_async(active_page).await;
        context.stage_background_target(
            "TID-background-route".to_owned(),
            Some("SID-background-route".to_owned()),
            "about:blank#background".to_owned(),
            None,
            None,
        );
        context
            .background_target_mut("TID-background-route")
            .expect("background target")
            .replace_loaded_page(Some(background_page));
        let active_attachment = context
            .active_page_state()
            .runtime_slot
            .current_renderer_attachment()
            .expect("active attachment");
        let background_attachment = context
            .background_target("TID-background-route")
            .and_then(|target| target.runtime_slot().current_renderer_attachment())
            .expect("background attachment");

        assert!(
            context
                .promote_background_target_to_active_slot_async("TID-background-route")
                .await
                .expect("target swap should succeed")
        );

        assert_eq!(
            context
                .active_page_state()
                .runtime_slot
                .current_renderer_attachment()
                .map(|attachment| attachment.id()),
            Some(background_attachment.id())
        );
        assert_eq!(
            context
                .background_target("TID-active-route")
                .and_then(|target| target.runtime_slot().current_renderer_attachment())
                .map(|attachment| attachment.id()),
            Some(active_attachment.id())
        );
    }

    #[tokio::test]
    async fn background_target_promotion_restores_nested_page_session_state() {
        let mut context = BrowserContext::new("BC-promote".to_owned());
        context.stage_background_target(
            "TID-bg".to_owned(),
            Some("SID-bg".to_owned()),
            "https://bg.test/".to_owned(),
            None,
            None,
        );
        let mut devtools_session_state = DevToolsSessionState {
            runtime_session_state: TargetRuntimeSessionState {
                runtime_frontend_enabled: true,
                runtime_contexts_reported_to_frontend: false,
                inspector_enabled: true,
                inspector_target_crashed_delivered: false,
            },
            page_session_state: TargetPageSessionState {
                page_lifecycle_events: true,
                log_enabled: true,
                performance: {
                    let mut performance = TargetPerformanceSessionState::default();
                    assert!(performance.enable(PerformanceTimeDomain::ThreadTicks));
                    performance
                },
                page_file_chooser_opened_event_enabled: true,
                page_intercept_file_chooser_dialog_enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };
        devtools_session_state
            .console_output_session_state
            .console_enabled = true;
        context.mutate_parked_page_session_state("TID-bg", |state| {
            state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary] =
                devtools_session_state;
        });
        context
            .background_target_mut("TID-bg")
            .expect("background target")
            .runtime_slot
            .set_session_observation_cursor_at_counts_for_test(None, 4, 5);

        assert!(
            context
                .promote_background_target_to_active_slot_async("TID-bg")
                .await
                .expect("promotion should not fail")
        );

        assert_eq!(context.active_target_id(), Some("TID-bg"));
        assert!(
            context.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled
        );
        assert!(
            context.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled
        );
        assert_eq!(
            context
                .active_page_state()
                .runtime_slot
                .emitted_subresource_record_count_for_session_for_test(None),
            4,
            "target network artifacts should restore from the background target runtime slot"
        );
        assert_eq!(
            context
                .active_page_state()
                .runtime_slot
                .emitted_websocket_event_count_for_session_for_test(None),
            5,
            "websocket observation cursor should restore with target network artifacts"
        );
        assert!(
            context.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_lifecycle_events
        );
        assert!(
            context.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .log_enabled
        );
        assert!(
            context.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled
        );
        assert!(
            context.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .performance
                .enabled()
        );
        assert_eq!(
            context.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .performance
                .time_domain(),
            PerformanceTimeDomain::ThreadTicks
        );
        assert!(
            context.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_file_chooser_opened_event_enabled
        );
        assert!(
            context.active_page_state().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_intercept_file_chooser_dialog_enabled
        );
    }
}
