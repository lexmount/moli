//! Stable page-target registry behavior and foreground target selection.

use super::super::state::{
    EffectiveTargetPolicy, TargetPageAbsenceReason, TargetSessionStorageNamespace,
};
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
    pub(crate) fn take_page_target_for_close(&mut self, target_id: &str) -> Option<PageTargetHost> {
        let target = self.page_targets.remove(target_id)?;
        self.forget_target_opener_references_for_target(target_id);
        self.forget_target_window_names_for_target(target_id);
        self.forget_target_popup_id_for_target(target_id);
        Some(target)
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
            self.page_target(creator.target_id())
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

    pub(crate) fn stage_foreground_target(
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
        self.page_targets
            .iter()
            .any(|target| target.owner_state.has_attached_child_frame_id(frame_id))
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
        let is_active = self.is_active_target(target_id);
        let Some(target) = self.page_target_mut(target_id) else {
            return false;
        };
        target.set_target_url(url);
        if is_active {
            target.owner_state.target_crash_state.clear();
        }
        true
    }

    pub(crate) fn assign_session_to_target(&mut self, target_id: &str, session_id: String) -> bool {
        let Some(target) = self.page_target_mut(target_id) else {
            return false;
        };
        target.attach_session(session_id);
        true
    }

    pub(crate) fn assign_auto_attached_session_to_target(
        &mut self,
        target_id: &str,
        session_id: String,
    ) -> bool {
        let Some(target) = self.page_target_mut(target_id) else {
            return false;
        };
        if target.has_session() {
            target.devtools_sessions.ensure_attached(&session_id);
        } else {
            target.attach_session(session_id);
        }
        true
    }

    pub(crate) fn assign_attached_session_to_target(
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

    #[cfg(test)]
    pub(crate) fn attached_target_id_for_session(&self, session_id: &str) -> Option<&str> {
        self.page_targets
            .iter()
            .find(|target| target.devtools_sessions.attached(session_id).is_some())
            .map(PageTargetHost::target_id)
    }

    pub(crate) fn attached_session_ids_for_target(&self, target_id: &str) -> Vec<String> {
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
        let mut session_ids = if let Some(target) = self.page_target(target_id) {
            target.session_id().map(str::to_owned).into_iter().collect()
        } else if let Some(target) = self.shared_worker_target(target_id) {
            target.session_ids()
        } else if let Some(target) = self.service_worker_target(target_id) {
            target.session_ids()
        } else {
            Vec::new()
        };
        session_ids.extend(self.attached_session_ids_for_target(target_id));
        session_ids.sort();
        session_ids.dedup();
        session_ids
    }

    /// Commits removal of a Page session after all domain handlers have run.
    ///
    /// This deliberately has no renderer or domain side effects. Disposal
    /// keeps the registry entry live while handlers resolve their exact Page,
    /// then reaches this single irreversible step.
    pub(crate) fn remove_page_session_binding(
        &mut self,
        target_id: &str,
        session_id: &str,
        session_key: &moli_page_types::DevToolsSessionKey,
    ) -> bool {
        self.page_target_mut(target_id)
            .and_then(|target| target.devtools_sessions.dispose(session_id, session_key))
            .is_some()
    }

    pub(crate) async fn clear_devtools_network_session_policy_async(
        &mut self,
        target_id: &str,
        session_key: &moli_page_types::DevToolsSessionKey,
    ) -> anyhow::Result<()> {
        let Some(target) = self.page_target_mut(target_id) else {
            return Ok(());
        };
        let previous = target.effective_policy();
        let listener_session_id = session_key.wire_session_id().map(str::to_owned);
        match session_key {
            moli_page_types::DevToolsSessionKey::Primary => {
                target.runtime_slot.disable_primary_network_events();
            }
            moli_page_types::DevToolsSessionKey::Attached(attached_session_id) => {
                target
                    .runtime_slot
                    .remove_attached_network_session(attached_session_id);
            }
        }
        target
            .runtime_slot
            .remove_network_session_observation_cursor(listener_session_id.as_deref());
        target
            .runtime_slot
            .remove_captured_response_body_visibility_for_session(listener_session_id.as_deref());
        if !target.runtime_slot.has_network_event_listeners() {
            target.runtime_slot.clear_captured_response_bodies();
            target.runtime_slot.clear_websocket_request_ids();
        }
        target.clear_devtools_network_state(session_key);
        self.apply_effective_devtools_policy_delta_async(target_id, previous)
            .await?;
        Ok(())
    }

    pub(crate) async fn clear_devtools_emulation_session_policy_async(
        &mut self,
        target_id: &str,
        session_key: &moli_page_types::DevToolsSessionKey,
    ) -> anyhow::Result<bool> {
        let Some(target) = self.page_target_mut(target_id) else {
            return Ok(false);
        };
        let previous = target.effective_policy();
        target.clear_devtools_emulation_state(session_key);
        self.apply_effective_devtools_policy_delta_async(target_id, previous)
            .await
    }

    async fn apply_effective_devtools_policy_delta_async(
        &mut self,
        target_id: &str,
        previous: EffectiveTargetPolicy,
    ) -> anyhow::Result<bool> {
        let Some(target) = self.page_target(target_id) else {
            return Ok(false);
        };
        let effective = target.effective_policy();
        let delta = previous.delta(&effective);
        let browser_identity_changed = delta.browser_identity_changed();

        if delta.is_empty() {
            return Ok(false);
        }

        let effective_headers =
            self.merged_extra_headers_for_target_policy(effective.extra_headers());
        let effective_locale = effective
            .locale_override()
            .map(str::to_owned)
            .or_else(|| self.default_locale_override.clone());
        let effective_timezone = effective
            .timezone_override()
            .map(str::to_owned)
            .or_else(|| self.default_timezone_override.clone());
        let page = self
            .page_target_mut(target_id)
            .and_then(|target| target.runtime_slot.loaded_page_mut());
        let Some(page) = page else {
            return Ok(browser_identity_changed);
        };
        if delta.network_request {
            page.set_network_request_policy_async(
                &effective_headers,
                effective.bypass_service_worker(),
                effective.cache_disabled(),
                effective.blocked_url_patterns(),
            )
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "failed to restore detached session network request policy: {error}"
                )
            })?;
        }
        if delta.locale {
            page.set_locale_override_async(effective_locale.as_deref())
                .await
                .map_err(|error| {
                    anyhow::anyhow!("failed to restore detached session locale: {error}")
                })?;
        }
        if delta.timezone {
            page.set_timezone_override_async(effective_timezone.as_deref())
                .await
                .map_err(|error| {
                    anyhow::anyhow!("failed to restore detached session timezone: {error}")
                })?;
        }
        Ok(browser_identity_changed)
    }

    pub(crate) async fn reset_primary_page_session_target_state_async(
        &mut self,
        target_id: &str,
        session_id: &str,
    ) -> anyhow::Result<bool> {
        let is_active = self.is_active_target(target_id);
        let Some(target) = self.page_target_mut(target_id) else {
            return Ok(false);
        };
        if !target.is_session(session_id) {
            return Ok(false);
        }
        target.reset_primary_session_target_state_fields();

        let effective_headers = self.effective_extra_headers_for_target(target_id);
        let target = self
            .page_target(target_id)
            .expect("disposing page target must remain registered");
        let effective_policy = target.effective_policy();
        let effective_locale = effective_policy
            .locale_override()
            .map(str::to_owned)
            .or_else(|| self.default_locale_override.clone());
        let effective_timezone = effective_policy
            .timezone_override()
            .map(str::to_owned)
            .or_else(|| self.default_timezone_override.clone());
        let surface_script = if is_active {
            self.generated_surface_override_script_for_active_target()
        } else {
            self.generated_surface_override_script_for_background_target(target_id)
        };
        if let Some(page) = self
            .page_target_mut(target_id)
            .and_then(|target| target.runtime_slot.loaded_page_mut())
        {
            let mut first_error = None;
            if let Err(error) = page
                .set_network_request_policy_async(
                    &effective_headers,
                    effective_policy.bypass_service_worker(),
                    effective_policy.cache_disabled(),
                    effective_policy.blocked_url_patterns(),
                )
                .await
            {
                first_error = Some(anyhow::anyhow!(
                    "failed to clear page network request policy: {error}"
                ));
            }
            if let Err(error) = page.set_network_offline_async(false).await {
                first_error.get_or_insert_with(|| {
                    anyhow::anyhow!("failed to clear page offline state: {error}")
                });
            }
            if let Err(error) = page.set_script_execution_disabled_async(false).await {
                first_error.get_or_insert_with(|| {
                    anyhow::anyhow!("failed to clear page script execution disabled state: {error}")
                });
            }
            if let Err(error) = page
                .set_locale_override_async(effective_locale.as_deref())
                .await
            {
                first_error.get_or_insert_with(|| {
                    anyhow::anyhow!("failed to restore page locale: {error}")
                });
            }
            if let Err(error) = page
                .set_timezone_override_async(effective_timezone.as_deref())
                .await
            {
                first_error.get_or_insert_with(|| {
                    anyhow::anyhow!("failed to restore page timezone: {error}")
                });
            }
            if let Some(surface_script) = surface_script
                && let Err(error) = page
                    .run_page_surface_override_script_async(&surface_script.source)
                    .await
            {
                first_error.get_or_insert_with(|| {
                    anyhow::anyhow!("failed to restore page surface overrides: {error}")
                });
            }
            if let Some(error) = first_error {
                return Err(error);
            }
        }

        Ok(true)
    }

    #[cfg(test)]
    pub(crate) fn enable_attached_network_events(&mut self, session_id: &str) {
        if self.attached_target_id_for_session(session_id).is_some() {
            self.active_page_target_mut()
                .runtime_slot
                .enable_attached_network_events(session_id);
        }
    }

    #[cfg(test)]
    pub(crate) fn has_network_event_listeners(&self) -> bool {
        self.active_page_target()
            .runtime_slot
            .has_network_event_listeners()
    }

    #[cfg(test)]
    pub(crate) fn network_event_session_ids(
        &self,
        trigger_session_id: Option<&str>,
    ) -> Vec<Option<String>> {
        self.active_page_target()
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
        let target = self.page_target(target_id)?;
        Some(TargetInitialEmptyDocumentCreator::new(
            target.target_id().to_owned(),
            target.target_identity().security_origin().to_owned(),
            target.target_identity().secure_context_type().to_owned(),
        ))
    }

    pub(crate) fn release_primary_session_binding_preserving_frontend_state(
        &mut self,
        session_id: &str,
    ) -> bool {
        let Some(target) = self
            .page_targets
            .iter_mut()
            .find(|target| target.is_session(session_id))
        else {
            return false;
        };
        target
            .devtools_sessions
            .dispose(session_id, &moli_page_types::DevToolsSessionKey::Primary)
            .is_some()
    }

    #[cfg(test)]
    pub(crate) async fn select_first_background_target_async(&mut self) -> Option<String> {
        let selected_target_id = self
            .background_targets()
            .find(|target| target.has_loaded_page())
            .map(|target| target.target_id().to_owned())
            .or_else(|| {
                self.background_targets()
                    .next()
                    .map(|target| target.target_id().to_owned())
            })?;
        self.select_background_target_async(selected_target_id)
            .await
    }

    pub(crate) async fn select_last_background_target_async(&mut self) -> Option<String> {
        let selected_target_id = self.last_selectable_background_target_id()?;
        self.select_background_target_async(selected_target_id)
            .await
    }

    pub(crate) fn last_selectable_background_target_id(&self) -> Option<String> {
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

    async fn select_background_target_async(&mut self, target_id: String) -> Option<String> {
        self.select_page_target_async(&target_id)
            .await
            .expect("applying selected target visibility should succeed")
            .then_some(target_id)
    }

    pub(crate) async fn select_page_target_async(
        &mut self,
        target_id: &str,
    ) -> anyhow::Result<bool> {
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
        let previous_surface_script = if synchronize_loaded_page {
            previous_active_target_id.as_deref().and_then(|target_id| {
                let host = self.page_target(target_id)?;
                self.generated_surface_override_script_for_background_state(host)
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
            (previous_active_target_id, previous_surface_script)
            && let Some(page) = self
                .page_target_mut(&previous_active_target_id)
                .and_then(|host| host.runtime_slot.loaded_page_mut())
            && let Err(error) = page
                .run_page_surface_override_script_async(&script.source)
                .await
        {
            tracing::warn!(target_id = previous_active_target_id, %error, "failed to update background page visibility");
        }
        Ok(true)
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
        Some(mutate(&mut self.page_target_mut(target_id)?.owner_state))
    }

    #[cfg(test)]
    pub(crate) fn target_info(&self, target_id: &str) -> Option<serde_json::Value> {
        self.devtools_target_info(target_id)
            .map(DevToolsTargetInfo::into_cdp_value)
    }

    pub(crate) fn devtools_target_info(&self, target_id: &str) -> Option<DevToolsTargetInfo> {
        if let Some(target) = self.page_target(target_id) {
            let attached =
                target.has_session() || !self.attached_session_ids_for_target(target_id).is_empty();
            return Some(DevToolsTargetInfo {
                target_id: Some(DevToolsTargetId::from(target_id)),
                kind: DevToolsTargetKind::Page,
                title: target
                    .owner_state
                    .committed_document_title()
                    .map(str::to_owned)
                    .or_else(|| target.loaded_page().map(|page| page.document_title()))
                    .unwrap_or_default(),
                url: target.target_url().to_owned(),
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

        None
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
    use super::*;
    use crate::conn::state::{PerformanceTimeDomain, TargetPerformanceSessionState};
    use crate::conn::{
        DevToolsSessionState, DocumentStartScript, TargetPageSessionState,
        TargetRuntimeSessionState,
    };
    use crate::testing::TestContext;
    use serde_json::json;
    use std::sync::Arc;

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
    fn changing_foreground_selection_retains_each_session_storage_namespace() {
        let mut context = BrowserContext::new("BC-deactivated-storage".to_owned());
        context.set_active_target_id("TID-first");
        let first_session_storage = context.page_storage_handles().session_storage_store.clone();
        assert!(
            first_session_storage
                .lock()
                .set_item("https://same.test", "session", "first")
        );

        context.stage_foreground_target(
            "TID-second".to_owned(),
            None,
            "about:blank".to_owned(),
            None,
        );
        let first_target_storage = context
            .page_storage_handles_for_target("TID-first")
            .expect("previous target should retain storage");
        let second_storage = context.page_storage_handles();

        assert!(Arc::ptr_eq(
            &first_session_storage,
            &first_target_storage.session_storage_store
        ));
        assert!(!Arc::ptr_eq(
            &first_session_storage,
            &second_storage.session_storage_store
        ));
        assert_eq!(
            first_target_storage
                .session_storage_store
                .lock()
                .get_item("https://same.test", "session"),
            Some("first".to_owned())
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
        {
            let state = context
                .background_target_mut("TID-bg")
                .expect("background target must exist");
            state.owner_state.next_document_start_script_id = 7;
            state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary] =
                DevToolsSessionState {
                    runtime_session_state: TargetRuntimeSessionState {
                        runtime_frontend_enabled: true,
                        ..Default::default()
                    },
                    ..Default::default()
                };
        }

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
                .background_target("TID-bg")
                .filter(|target| target.has_non_default_session_state())
                .is_some_and(|state| state.devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .runtime_session_state
                    .runtime_frontend_enabled)
        );
        assert_eq!(host.owner_state.next_document_start_script_id, 7);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn selecting_another_foreground_target_preserves_page_session_and_owner_state() {
        let mut ctx = TestContext::new();
        let active_page = ctx
            .conn
            .load_page_via_runtime_async("data:text/html,<title>deactivate-active</title>")
            .await
            .expect("active page should load");

        let mut context = BrowserContext::new("BC-deactivate".to_owned());
        context.set_active_target_id("TID-deactivate".to_owned());
        context.attach_active_session("SID-deactivate".to_owned());
        context.set_target_url(active_page.final_url().as_str().to_owned());
        context.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .runtime_frontend_enabled = true;
        context.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .runtime_session_state
            .inspector_enabled = true;
        context.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .console_output_session_state
            .console_enabled = true;
        context.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .log_enabled = true;
        assert!(
            context.active_page_target_mut().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .performance
                .enable(PerformanceTimeDomain::ThreadTicks)
        );
        context.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .page_lifecycle_events = true;
        context.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .page_file_chooser_opened_event_enabled = true;
        context.active_page_target_mut().devtools_sessions
            [moli_page_types::DevToolsSessionKey::Primary]
            .page_session_state
            .page_intercept_file_chooser_dialog_enabled = true;
        context
            .active_page_target_mut()
            .runtime_slot
            .set_primary_network_events_enabled(true);
        context
            .active_page_target_mut()
            .owner_state
            .next_document_start_script_id = 9;
        context
            .active_page_target_mut()
            .owner_state
            .document_start_scripts
            .push((
                "script-deactivate".to_owned(),
                DocumentStartScript {
                    registry_key: None,
                    devtools_session: None,
                    source: "globalThis.deactivated = true".to_owned(),
                    world_name: None,
                    has_bidi_channel_argument: false,
                    bidi_channel_handoffs: Vec::new(),
                },
            ));
        context
            .active_page_target_mut()
            .runtime_slot
            .set_network_request_counters_for_test(77, 88);
        context
            .active_page_target_mut()
            .runtime_slot
            .mark_subresource_records_emitted(None, 0, 3);
        context.set_loaded_page_async(active_page).await;
        let active_attachment = context
            .active_page_target()
            .runtime_slot
            .current_renderer_attachment()
            .expect("loaded active page should have a renderer attachment");
        context.stage_background_target(
            "TID-selected".to_owned(),
            Some("SID-selected".to_owned()),
            "about:blank#selected".to_owned(),
            None,
            None,
        );

        assert_eq!(
            context
                .select_background_target_async("TID-selected".to_owned())
                .await
                .as_deref(),
            Some("TID-selected")
        );

        assert_eq!(context.active_target_id(), Some("TID-selected"));
        assert!(
            context
                .page_target("TID-deactivate")
                .is_some_and(|host| host.has_loaded_page()),
            "changing foreground selection must retain the loaded page in its stable host"
        );
        assert_eq!(context.background_target_count(), 1);
        let background_target = &context.background_target_at(0).unwrap();
        assert_eq!(background_target.target_id(), "TID-deactivate");
        assert_eq!(background_target.session_id(), Some("SID-deactivate"));
        assert_eq!(
            background_target.target_url(),
            "data:text/html,<title>deactivate-active</title>"
        );
        assert!(
            background_target.has_loaded_page(),
            "the loaded page must remain in the same stable target host"
        );
        assert_eq!(
            background_target
                .runtime_slot()
                .current_renderer_attachment()
                .map(|attachment| attachment.id()),
            Some(active_attachment.id()),
            "changing foreground selection must not reallocate the renderer channel"
        );
        assert_eq!(
            background_target
                .loaded_page()
                .and_then(|page| page.renderer_agent_attachment_id()),
            Some(active_attachment.id()),
            "the background Page and its renderer channel must retain the same attachment"
        );
        assert!(
            background_target
                .runtime_slot
                .primary_network_events_enabled(),
            "the stable target runtime slot must retain Network.enable state"
        );
        assert!(
            context
                .background_target("TID-deactivate")
                .filter(|target| target.has_non_default_session_state())
                .is_some_and(|state| state.devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .runtime_session_state
                    .runtime_frontend_enabled),
            "session-scoped Runtime.enable state must remain owned by the target"
        );
        let background_state = context
            .background_target("TID-deactivate")
            .filter(|target| target.has_non_default_session_state())
            .expect("background target should retain page session state");
        assert!(
            background_state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled,
            "Runtime.enable state must remain target-owned"
        );
        assert!(
            background_state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled,
            "Inspector.enable state must remain target-owned"
        );
        assert!(
            background_state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled,
            "Console.enable state must remain target-owned"
        );
        assert!(
            background_state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .log_enabled,
            "Log.enable state must remain target-owned"
        );
        assert!(
            background_state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .performance
                .enabled(),
            "Performance.enable state must remain target-owned"
        );
        assert_eq!(
            background_state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .performance
                .time_domain(),
            PerformanceTimeDomain::ThreadTicks,
            "Performance time domain must remain target-owned"
        );
        assert!(
            background_state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_lifecycle_events,
            "Page lifecycle listener state must remain target-owned"
        );
        assert!(
            background_state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_file_chooser_opened_event_enabled,
            "file chooser opened listener state must remain target-owned"
        );
        assert!(
            background_state.devtools_sessions[moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_intercept_file_chooser_dialog_enabled,
            "file chooser interception state must remain target-owned"
        );
        assert_eq!(
            context
                .background_target("TID-deactivate")
                .expect("previous target must remain registered")
                .owner_state
                .next_document_start_script_id,
            9,
            "owner state must remain target-owned"
        );
        assert_eq!(
            context
                .background_target("TID-deactivate")
                .expect("background target should retain owner state")
                .owner_state
                .document_start_scripts
                .len(),
            1,
            "document-start scripts must remain target-owned"
        );
        let background_runtime = &context
            .background_target("TID-deactivate")
            .expect("background target should retain network state")
            .runtime_slot;
        assert_eq!(background_runtime.next_fetch_request_id_for_test(), 77);
        assert_eq!(
            background_runtime.next_subresource_fetch_request_id_for_test(),
            88
        );
        assert_eq!(
            background_runtime.emitted_subresource_record_count_for_session_for_test(None),
            3,
            "network artifacts must remain in the stable target runtime slot"
        );
    }

    #[test]
    fn staging_background_target_in_empty_context_leaves_foreground_empty() {
        let mut context = BrowserContext::new("BC-deactivate-empty".to_owned());
        context.stage_background_target(
            "TID-existing-bg".to_owned(),
            Some("SID-existing-bg".to_owned()),
            "https://existing.test/".to_owned(),
            None,
            None,
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
    fn selecting_new_target_preserves_previous_pending_initial_document_reason() {
        let mut context = BrowserContext::new("BC-deactivate-pending".to_owned());
        context.set_active_target_id("TID-old-active");
        context.set_target_url("about:blank#old".to_owned());
        context.begin_active_target_initial_empty_document("about:blank#old".to_owned());

        context.stage_foreground_target(
            "TID-new-active".to_owned(),
            Some("SID-new-active".to_owned()),
            "about:blank#new".to_owned(),
            Some("about:blank#new".to_owned()),
        );

        assert_eq!(
            context
                .background_target("TID-old-active")
                .expect("previous active target should remain registered")
                .runtime_slot()
                .moli_memory_diagnostics()["loadedPageAbsenceReason"],
            json!("initial-document-page-build-pending"),
            "foreground selection must preserve a pending initial document absence reason"
        );
        assert_eq!(
            context
                .active_page_target()
                .runtime_slot
                .moli_memory_diagnostics()["loadedPageAbsenceReason"],
            json!("initial-document-page-build-pending"),
            "the replacement target must expose its own pending initial document build"
        );
    }

    #[tokio::test]
    async fn background_target_activate_without_page_preserves_pending_initial_document_reason() {
        let mut context = BrowserContext::new("BC-activate-pending".to_owned());
        context.stage_background_target(
            "TID-pending-bg".to_owned(),
            Some("SID-pending-bg".to_owned()),
            "about:blank#pending".to_owned(),
            None,
            None,
        );

        assert!(
            context
                .select_page_target_async("TID-pending-bg")
                .await
                .expect("pending background target should remain selectable")
        );

        assert_eq!(
            context
                .active_page_target()
                .runtime_slot
                .moli_memory_diagnostics()["loadedPageAbsenceReason"],
            json!("initial-document-page-build-pending"),
            "foreground selection must preserve a pending initial document absence reason"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn select_first_background_target_prefers_first_loaded_target() {
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

        let mut context = BrowserContext::new("BC-activate-first".to_owned());
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

        let selected = context
            .select_first_background_target_async()
            .await
            .expect("loaded background target should be selectable");

        assert_eq!(selected, "TID-first-loaded");
        assert_eq!(context.active_target_id(), Some("TID-first-loaded"));
        assert_eq!(context.active_session_id(), Some("SID-first-loaded"));
        assert!(
            context.has_loaded_page(),
            "first loaded background target's page should become active"
        );
        assert_eq!(
            context
                .active_page_target()
                .runtime_slot
                .current_renderer_attachment()
                .map(|attachment| attachment.id()),
            Some(first_attachment.id()),
            "selection must preserve the target's renderer channel and Page"
        );
        assert!(
            context
                .background_target("TID-second-loaded")
                .is_some_and(PageTargetHost::has_loaded_page),
            "later loaded background target should remain in the background"
        );
        assert_eq!(
            context
                .background_target("TID-second-loaded")
                .and_then(|target| target.runtime_slot().current_renderer_attachment())
                .map(|attachment| attachment.id()),
            Some(second_attachment.id()),
            "activating one target must not replace another background target's route lease"
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
            .active_page_target()
            .runtime_slot
            .current_renderer_attachment()
            .expect("active attachment");
        let background_attachment = context
            .background_target("TID-background-route")
            .and_then(|target| target.runtime_slot().current_renderer_attachment())
            .expect("background attachment");

        assert!(
            context
                .select_page_target_async("TID-background-route")
                .await
                .expect("target selection should succeed")
        );

        assert_eq!(
            context
                .active_page_target()
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
    async fn background_target_selection_preserves_nested_page_session_state() {
        let mut context = BrowserContext::new("BC-activate".to_owned());
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
        context
            .background_target_mut("TID-bg")
            .expect("background target must exist")
            .devtools_sessions[moli_page_types::DevToolsSessionKey::Primary] =
            devtools_session_state;
        context
            .background_target_mut("TID-bg")
            .expect("background target")
            .runtime_slot
            .set_session_observation_cursor_at_counts_for_test(None, 4, 5);

        assert!(
            context
                .select_page_target_async("TID-bg")
                .await
                .expect("target selection should not fail")
        );

        assert_eq!(context.active_target_id(), Some("TID-bg"));
        assert!(
            context.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled
        );
        assert!(
            context.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .inspector_enabled
        );
        assert_eq!(
            context
                .active_page_target()
                .runtime_slot
                .emitted_subresource_record_count_for_session_for_test(None),
            4,
            "target network artifacts should restore from the background target runtime slot"
        );
        assert_eq!(
            context
                .active_page_target()
                .runtime_slot
                .emitted_websocket_event_count_for_session_for_test(None),
            5,
            "websocket observation cursor should restore with target network artifacts"
        );
        assert!(
            context.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_lifecycle_events
        );
        assert!(
            context.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .log_enabled
        );
        assert!(
            context.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .console_output_session_state
                .console_enabled
        );
        assert!(
            context.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .performance
                .enabled()
        );
        assert_eq!(
            context.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .performance
                .time_domain(),
            PerformanceTimeDomain::ThreadTicks
        );
        assert!(
            context.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_file_chooser_opened_event_enabled
        );
        assert!(
            context.active_page_target().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .page_session_state
                .page_intercept_file_chooser_dialog_enabled
        );
    }
}
