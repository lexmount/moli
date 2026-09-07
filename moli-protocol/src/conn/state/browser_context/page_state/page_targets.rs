//! Stable page-target registry behavior and foreground target selection.

use crate::conn::state::page_slot::TargetPageSlot;
use crate::conn::state::{SessionStorageNamespace, TargetPageAbsenceReason};
use crate::conn::{
    BrowserContext, DedicatedWorkerTargetState, InitialDocumentCreator, PageAgentHost,
    ServiceWorkerTargetState, SharedWorkerTargetState, TargetIdentityState,
};
use crate::devtools_runtime::{
    DevToolsBrowserContextId, DevToolsTargetId, DevToolsTargetInfo, DevToolsTargetKind,
};
use moli_core::browser::{WebContentsCreation, WebContentsHandle};
use moli_core::network::SharedWebStorageStore;

pub(crate) struct PendingWebContentsSelection {
    admission_error: Option<String>,
    surface_updates: Vec<crate::conn::PendingDocumentPolicyUpdate>,
}

pub(crate) struct CompletedWebContentsSelection {
    admission_error: Option<String>,
    surface_updates: Vec<crate::conn::CompletedDocumentPolicyUpdate>,
}

impl PendingWebContentsSelection {
    pub(crate) async fn wait(self) -> CompletedWebContentsSelection {
        let mut surface_updates = Vec::with_capacity(self.surface_updates.len());
        for update in self.surface_updates {
            surface_updates.push(update.wait().await);
        }
        CompletedWebContentsSelection {
            admission_error: self.admission_error,
            surface_updates,
        }
    }
}

impl BrowserContext {
    pub(in crate::conn) fn begin_web_contents_close(
        &mut self,
        handle: WebContentsHandle,
    ) -> Result<(PageAgentHost, moli_core::browser::PendingWebContentsClose), String> {
        let target_id = self
            .page_targets
            .get_for_web_contents(handle.id())
            .map(PageAgentHost::target_id)
            .ok_or_else(|| "WebContents projection unavailable".to_owned())?
            .to_owned();
        let closing = self.browser_context.close_web_contents(handle)?;
        let mut target = self
            .page_targets
            .remove(&target_id)
            .expect("retired WebContents must retain its resolved projection until removal");
        target.runtime_slot.retire_for_target_close();
        self.forget_target_popup_id_for_target(&target_id);
        Ok((target, closing))
    }

    pub(crate) fn stage_background_target(
        &mut self,
        target_id: String,
        session_id: Option<String>,
        url: String,
        initial_empty_document_url: Option<String>,
        creator: Option<InitialDocumentCreator>,
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
        creator: Option<InitialDocumentCreator>,
        session_storage_store: Option<SharedWebStorageStore>,
        initial_empty_document_storage_key: Option<moli_storage_key::MoliStorageKey>,
    ) {
        let session_storage_namespace = session_storage_store
            .map(SessionStorageNamespace::from_store)
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
        creator: Option<&InitialDocumentCreator>,
    ) -> Option<SessionStorageNamespace> {
        creator.and_then(|creator| {
            self.browser_context
                .clone_session_storage_namespace(creator.web_contents_id())
        })
    }

    fn stage_background_target_with_session_storage_namespace(
        &mut self,
        target_id: String,
        session_id: Option<String>,
        url: String,
        initial_empty_document_url: Option<String>,
        creator: Option<InitialDocumentCreator>,
        initial_empty_document_storage_key: Option<moli_storage_key::MoliStorageKey>,
        session_storage_namespace: Option<SessionStorageNamespace>,
    ) {
        let target_identity = background_target_identity_for_initial_url(&url, creator.as_ref());
        let mut creation = WebContentsCreation::with_initial_document(
            initial_empty_document_url.unwrap_or_else(|| url.clone()),
            creator,
            initial_empty_document_storage_key,
        );
        if let Some(namespace) = session_storage_namespace {
            creation = creation.with_session_storage(namespace);
        }
        let inserted = self.register_web_contents_target(
            target_id,
            session_id,
            target_identity,
            creation,
            TargetPageSlot::empty_for_initial_document_page_build(),
        );
        debug_assert!(inserted, "staged page target id must be unique");
    }

    pub(crate) fn stage_foreground_target(
        &mut self,
        target_id: String,
        session_id: Option<String>,
        url: String,
        initial_empty_document_url: Option<String>,
    ) {
        let creation = WebContentsCreation::with_initial_document(
            initial_empty_document_url.unwrap_or_else(|| url.clone()),
            None,
            None,
        );
        let inserted = self.register_web_contents_target(
            target_id.clone(),
            session_id,
            TargetIdentityState::with_url(url),
            creation,
            TargetPageSlot::empty_for_initial_document_page_build(),
        );
        debug_assert!(inserted, "new active page target id must be unique");
        let handle = self
            .web_contents_handle_for_target(&target_id)
            .expect("newly inserted target must have WebContents");
        self.select_registered_web_contents(handle)
            .expect("newly inserted WebContents must be selectable");
    }

    pub(crate) fn reusable_window_open_target_name(target_name: &str) -> Option<&str> {
        if target_name.is_empty() || target_name.eq_ignore_ascii_case("_blank") {
            return None;
        }
        Some(target_name)
    }

    pub(crate) fn target_id_for_window_name(&self, target_name: &str) -> Option<&str> {
        let id = self.web_contents_handle_for_window_name(target_name)?.id();
        self.page_targets
            .get_for_web_contents(id)
            .map(PageAgentHost::target_id)
    }

    pub(crate) fn web_contents_handle_for_window_name(
        &self,
        target_name: &str,
    ) -> Option<moli_core::browser::WebContentsHandle> {
        let name = Self::reusable_window_open_target_name(target_name)?;
        self.browser_context
            .web_contents_handle_for_window_name(name)
    }

    pub(crate) fn has_attached_child_frame_id(&self, frame_id: &str) -> bool {
        self.page_targets
            .iter()
            .any(|target| target.owner_state.has_attached_child_frame_id(frame_id))
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

    pub(crate) fn set_target_opener_frame_attribution(
        &mut self,
        target_id: &str,
        opener_frame_id: String,
    ) {
        if let Some(target) = self.page_target_mut(target_id) {
            target.opener_frame_id = Some(opener_frame_id);
        }
    }

    pub(crate) fn update_target_url(&mut self, target_id: &str, url: String) -> bool {
        let is_active = self.is_active_target(target_id);
        let Some(target) = self.page_target_mut(target_id) else {
            return false;
        };
        target.set_target_url(url);
        if is_active {
            self.set_target_crash_state(target_id, false);
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
            .map(PageAgentHost::target_id)
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
    /// Disposal keeps the registry entry live while handlers resolve their
    /// exact Page, then removes the final contributions without renderer work.
    pub(crate) fn remove_page_session_binding(
        &mut self,
        target_id: &str,
        session_id: &str,
        session_key: &moli_page_types::DevToolsSessionKey,
    ) -> bool {
        self.page_targets.get(target_id).is_some()
            && self.dispose_devtools_session_for_target(target_id, session_id, session_key)
    }

    pub(crate) fn start_reset_primary_page_session_target_state(
        &mut self,
        target_id: &str,
        session_id: &str,
        browser_globals: &crate::conn::BrowserGlobalOverrides,
    ) -> (bool, Option<crate::conn::PendingDocumentPolicyBatch>) {
        let is_active = self.is_active_target(target_id);
        let Some(target) = self.page_target_mut(target_id) else {
            return (false, None);
        };
        if !target.is_session(session_id) {
            return (false, None);
        }
        self.reset_primary_session_target_state_fields_for_target(target_id);

        let effective_headers =
            self.effective_extra_headers_for_target(target_id, &browser_globals.extra_headers);
        let effective_policy = self.effective_policy_for_target(target_id);
        let script_execution_disabled = self
            .target_emulation_policy(target_id)
            .expect("live WebContents")
            .script_execution_disabled;
        let effective_locale = effective_policy
            .locale_override()
            .map(str::to_owned)
            .or_else(|| self.emulation_defaults().locale.clone());
        let effective_timezone = effective_policy
            .timezone_override()
            .map(str::to_owned)
            .or_else(|| self.emulation_defaults().timezone.clone());
        let Some(document) = self.document_handle_for_target(target_id) else {
            return (true, None);
        };
        let pending = self.start_document_policy_batch_with_surface(
            document,
            vec![
                crate::conn::DocumentPolicyUpdate::NetworkRequestPolicy {
                    extra_headers: effective_headers,
                    bypass_service_worker: effective_policy.bypass_service_worker(),
                    cache_disabled: effective_policy.cache_disabled(),
                    blocked_url_patterns: effective_policy.blocked_url_patterns().to_vec(),
                },
                crate::conn::DocumentPolicyUpdate::NetworkOffline(false),
                crate::conn::DocumentPolicyUpdate::ScriptExecutionDisabled(
                    script_execution_disabled,
                ),
                crate::conn::DocumentPolicyUpdate::LocaleOverride(effective_locale),
                crate::conn::DocumentPolicyUpdate::TimezoneOverride(effective_timezone),
            ],
            is_active,
            browser_globals.network_conditions,
            browser_globals.geolocation.as_ref(),
        );
        (true, Some(pending))
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
    ) -> Option<InitialDocumentCreator> {
        let target = self.page_target(target_id)?;
        Some(InitialDocumentCreator::new(
            target.web_contents_id(),
            target.target_identity().security_origin().to_owned(),
            target.target_identity().secure_context_type().to_owned(),
        ))
    }

    pub(crate) fn release_primary_session_binding_preserving_frontend_state(
        &mut self,
        session_id: &str,
    ) -> bool {
        let Some(target_id) = self
            .page_targets
            .iter()
            .find(|target| target.is_session(session_id))
            .map(|target| target.target_id().to_owned())
        else {
            return false;
        };
        self.dispose_devtools_session_for_target(
            &target_id,
            session_id,
            &moli_page_types::DevToolsSessionKey::Primary,
        )
    }

    pub(crate) fn last_selectable_background_target_id(&self) -> Option<String> {
        self.background_targets()
            .rev()
            .find(|target| self.target_has_loaded_page(target.target_id()))
            .map(|target| target.target_id().to_owned())
            .or_else(|| {
                self.background_targets()
                    .next_back()
                    .map(|target| target.target_id().to_owned())
            })
    }

    pub(crate) fn start_select_web_contents(
        &mut self,
        selected: WebContentsHandle,
        browser_globals: &crate::conn::BrowserGlobalOverrides,
    ) -> Result<PendingWebContentsSelection, String> {
        let selected_has_dialog = self.web_contents_has_pending_javascript_dialog(selected)?;
        let previous = self.selected_web_contents_handle();
        if previous == Some(selected) {
            return Ok(PendingWebContentsSelection {
                admission_error: None,
                surface_updates: Vec::new(),
            });
        }
        let previous_has_dialog = previous
            .map(|handle| self.web_contents_has_pending_javascript_dialog(handle))
            .transpose()?
            .unwrap_or(false);
        self.select_registered_web_contents(selected)?;

        let mut admission_error = None;
        let mut surface_updates = Vec::new();
        if !selected_has_dialog && !previous_has_dialog {
            for (handle, foreground) in
                std::iter::once((selected, true)).chain(previous.map(|handle| (handle, false)))
            {
                let Some(document) = self.document_handle_for_web_contents(handle)? else {
                    continue;
                };
                match self.start_document_page_surface_update(
                    document,
                    foreground,
                    browser_globals.network_conditions,
                    browser_globals.geolocation.as_ref(),
                ) {
                    Ok(update) => surface_updates.push(update),
                    Err(error) => {
                        admission_error.get_or_insert(error);
                    }
                }
            }
        }
        Ok(PendingWebContentsSelection {
            admission_error,
            surface_updates,
        })
    }

    pub(crate) fn finish_select_web_contents(
        &mut self,
        completed: CompletedWebContentsSelection,
    ) -> Result<(), String> {
        let mut first_error = completed.admission_error;
        for update in completed.surface_updates {
            if let Err(error) = self.finish_document_policy_update(update) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
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
        self.mark_loaded_page_absent_for_target(
            &target_id,
            TargetPageAbsenceReason::InitialDocumentPageBuildPending,
        );
        let handle = self
            .web_contents_handle_for_target(&target_id)
            .expect("selected WebContents");
        self.browser_context
            .begin_initial_empty_document(handle, initial_url, None, storage_key)
            .expect("selected WebContents");
    }

    #[cfg(test)]
    pub(crate) fn mark_target_initial_empty_document_materialized(&mut self, target_id: &str) {
        if let Some(handle) = self.web_contents_handle_for_target(target_id) {
            let _ = self
                .browser_context
                .mark_initial_empty_document_materialized_for_test(handle);
        }
    }

    pub(crate) fn mark_target_initial_url_replaces_empty_document(&mut self, target_id: &str) {
        if let Some(handle) = self.web_contents_handle_for_target(target_id) {
            let _ = self
                .browser_context
                .mark_initial_url_replaces_empty_document(handle);
        }
    }

    #[cfg(test)]
    pub(crate) fn mark_target_initial_empty_document_exited(&mut self, target_id: &str) {
        if let Some(handle) = self.web_contents_handle_for_target(target_id) {
            let _ = self
                .browser_context
                .mark_initial_empty_document_exited_for_test(handle);
        }
    }

    #[cfg(test)]
    pub(crate) fn target_info(&self, target_id: &str) -> Option<serde_json::Value> {
        self.devtools_target_info(target_id)
            .map(DevToolsTargetInfo::into_cdp_value)
    }

    pub(crate) fn devtools_target_info(&self, target_id: &str) -> Option<DevToolsTargetInfo> {
        if let Some(target) = self.page_target(target_id) {
            let handle = self.web_contents_handle_for_target(target_id)?;
            let opener = self
                .browser_context
                .web_contents_opener(handle)
                .ok()?
                .and_then(|(web_contents_id, can_access)| {
                    self.page_targets
                        .get_for_web_contents(web_contents_id)
                        .map(|target| (target, can_access))
                });
            let attached =
                target.has_session() || !self.attached_session_ids_for_target(target_id).is_empty();
            return Some(DevToolsTargetInfo {
                target_id: Some(DevToolsTargetId::from(target_id)),
                kind: DevToolsTargetKind::Page,
                title: target
                    .owner_state
                    .committed_document_title()
                    .map(str::to_owned)
                    .or_else(|| {
                        self.document_handle_for_target(target.target_id())
                            .and_then(|document| self.browser_context.document_title(document).ok())
                    })
                    .unwrap_or_default(),
                url: target.target_url().to_owned(),
                attached,
                opener_id: opener.map(|(target, _)| DevToolsTargetId::from(target.target_id())),
                opener_frame_id: target
                    .opener_frame_id
                    .as_deref()
                    .map(crate::devtools_runtime::DevToolsFrameId::from),
                can_access_opener: opener.is_some_and(|(_, can_access)| can_access),
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
        expected.browser_context_id() == self.id
            && expected
                .target_id()
                .and_then(|target_id| self.target_document_id(target_id))
                == Some(expected.document_id())
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
        let _ =
            self.attach_dedicated_worker_inspector_session(renderer_instance_id, Some(session_id));
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
            self.set_service_worker_devtools_attached(version_id, true);
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
            self.set_service_worker_devtools_attached(version_id, false);
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
    creator: Option<&InitialDocumentCreator>,
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
    use crate::conn::state::PerformanceTimeDomain;
    use crate::conn::{DocumentStartScript, TargetRuntimeSessionState};
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
    fn initial_document_creator_survives_target_rekey_without_following_reused_ids() {
        let mut context = BrowserContext::new("BC-creator-rekey".into());
        context.set_active_target_id("TID-opener");
        let opener_storage = context.page_storage_handles().session_storage_store;
        assert!(
            opener_storage
                .lock()
                .set_item("https://same.test", "session", "opener")
        );
        let creator = context
            .initial_empty_document_creator_for_target("TID-opener")
            .unwrap();

        assert!(context.rekey_active_target("TID-renamed-opener"));
        context.stage_background_target(
            "TID-opener".into(),
            None,
            "about:blank".into(),
            None,
            None,
        );
        let replacement_storage = context
            .page_storage_handles_for_target("TID-opener")
            .unwrap()
            .session_storage_store;
        assert!(
            replacement_storage
                .lock()
                .set_item("https://same.test", "session", "replacement")
        );

        context.stage_background_target(
            "TID-popup".into(),
            None,
            "about:blank".into(),
            None,
            Some(creator.clone()),
        );
        let popup_storage = context
            .page_storage_handles_for_target("TID-popup")
            .unwrap()
            .session_storage_store;
        assert_eq!(
            popup_storage
                .lock()
                .get_item("https://same.test", "session"),
            Some("opener".into())
        );
        assert!(!Arc::ptr_eq(&opener_storage, &popup_storage));

        let renamed_opener = context
            .web_contents_handle_for_target("TID-renamed-opener")
            .unwrap();
        drop(context.begin_web_contents_close(renamed_opener).unwrap());
        context.stage_background_target(
            "TID-orphan-popup".into(),
            None,
            "about:blank".into(),
            None,
            Some(creator),
        );
        let orphan_storage = context
            .page_storage_handles_for_target("TID-orphan-popup")
            .unwrap()
            .session_storage_store;
        assert_eq!(
            orphan_storage
                .lock()
                .get_item("https://same.test", "session"),
            None,
            "an expired creator must not resolve to the new WebContents using its old public TargetId"
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
    fn opener_follows_web_contents_across_target_rekey_and_id_reuse() {
        let mut context = BrowserContext::new("BC-opener-rekey".into());
        context.set_active_target_id("TID-opener");
        let opener = context.active_page_target().web_contents_id();
        context.stage_background_target("TID-popup".into(), None, "about:blank".into(), None, None);
        let popup_handle = context.web_contents_handle_for_target("TID-popup").unwrap();
        let opener_handle = context
            .web_contents_handle_for_target("TID-opener")
            .unwrap();
        context
            .set_web_contents_opener(popup_handle, Some(opener_handle), true)
            .unwrap();
        context.set_target_opener_frame_attribution("TID-popup", "FRAME-opener".into());

        assert!(context.rekey_active_target("TID-renamed"));
        context.stage_background_target(
            "TID-opener".into(),
            None,
            "about:blank".into(),
            None,
            None,
        );
        assert_eq!(context.active_page_target().web_contents_id(), opener);
        assert_eq!(
            context.target_info("TID-popup").unwrap()["openerId"],
            "TID-renamed"
        );
        let replacement_opener = context
            .web_contents_handle_for_target("TID-opener")
            .unwrap();
        drop(context.begin_web_contents_close(replacement_opener));
        let popup = context.target_info("TID-popup").unwrap();
        assert_eq!(popup["openerId"], "TID-renamed");
        assert_eq!(popup["canAccessOpener"], true);

        drop(context.begin_web_contents_close(opener_handle));
        context.set_active_target_id("TID-renamed");
        let popup = context.target_info("TID-popup").unwrap();
        assert!(popup.get("openerId").is_none());
        assert_eq!(popup["canAccessOpener"], false);
        assert_eq!(popup["openerFrameId"], "FRAME-opener");
    }

    #[test]
    fn window_name_follows_web_contents_and_dies_with_its_owner() {
        let mut context = BrowserContext::new("BC-window-rekey".into());
        context.set_active_target_id("TID-window");
        let handle = context
            .web_contents_handle_for_target("TID-window")
            .unwrap();
        context
            .set_web_contents_window_name(handle, Some("report".into()))
            .unwrap();
        assert!(context.rekey_active_target("TID-renamed"));
        context.stage_background_target(
            "TID-window".into(),
            None,
            "about:blank".into(),
            None,
            None,
        );
        assert_eq!(
            context.target_id_for_window_name("report"),
            Some("TID-renamed")
        );
        let replacement = context
            .web_contents_handle_for_target("TID-window")
            .unwrap();
        drop(context.begin_web_contents_close(replacement));
        assert_eq!(
            context.target_id_for_window_name("report"),
            Some("TID-renamed")
        );

        context
            .set_web_contents_window_name(handle, Some("renamed-report".into()))
            .unwrap();
        assert_eq!(context.target_id_for_window_name("report"), None);
        assert_eq!(
            context.target_id_for_window_name("renamed-report"),
            Some("TID-renamed")
        );
        drop(context.begin_web_contents_close(handle));
        context.set_active_target_id("TID-renamed");
        assert_eq!(context.target_id_for_window_name("renamed-report"), None);
    }

    #[test]
    fn window_open_target_registry_preserves_named_target_bytes() {
        assert_eq!(
            BrowserContext::reusable_window_open_target_name("_BlAnK"),
            None
        );
        assert_eq!(
            BrowserContext::reusable_window_open_target_name(" _blank "),
            Some(" _blank ")
        );
        assert_eq!(
            BrowserContext::reusable_window_open_target_name("ReportWindow"),
            Some("ReportWindow")
        );

        let mut context = BrowserContext::new("BC-window-name".to_owned());
        for id in ["TID-spaced", "TID-exact"] {
            context.stage_background_target(id.into(), None, "about:blank".into(), None, None);
        }
        let spaced = context
            .web_contents_handle_for_target("TID-spaced")
            .unwrap();
        let exact = context.web_contents_handle_for_target("TID-exact").unwrap();
        context
            .set_web_contents_window_name(spaced, Some(" ReportWindow ".into()))
            .unwrap();
        context
            .set_web_contents_window_name(exact, Some("ReportWindow".into()))
            .unwrap();
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
    fn page_agent_host_keeps_protocol_and_owner_state_together() {
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
            state
                .devtools_sessions
                .primary_mut()
                .runtime_session_state
                .runtime_frontend_enabled = true;
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
            context.background_targets().next().unwrap().target_id(),
            "TID-bg"
        );
        assert!(
            context
                .background_target("TID-bg")
                .filter(
                    |target| context.has_non_default_session_state_for_target(target.target_id())
                )
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
        let mut context = ctx
            .conn
            .new_browser_context_fixture_for_test("BC-deactivate".to_owned());
        context.set_active_target_id("TID-deactivate".to_owned());
        context.attach_active_session("SID-deactivate".to_owned());
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
        ctx.conn.install_browser_context_fixture_for_test(context);
        ctx.install_navigation_fixture_for_session_owner(
            "data:text/html,<title>deactivate-active</title>",
            Some("SID-deactivate"),
        )
        .await;
        let mut context = ctx.conn.browser_context.take().unwrap();
        context
            .active_page_target_mut()
            .runtime_slot
            .mark_subresource_records_emitted(None, 0, 3);
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

        let handle = context
            .web_contents_handle_for_target("TID-selected")
            .unwrap();
        let completed = context
            .start_select_web_contents(handle, &Default::default())
            .unwrap()
            .wait()
            .await;
        context.finish_select_web_contents(completed).unwrap();

        assert_eq!(context.active_target_id(), Some("TID-selected"));
        assert!(
            context.target_has_loaded_page("TID-deactivate"),
            "changing foreground selection must retain the loaded page in its stable host"
        );
        assert_eq!(context.background_target_count(), 1);
        let background_target = context.background_targets().next().unwrap();
        assert_eq!(background_target.target_id(), "TID-deactivate");
        assert_eq!(background_target.session_id(), Some("SID-deactivate"));
        assert_eq!(
            background_target.target_url(),
            "data:text/html,<title>deactivate-active</title>"
        );
        assert!(
            context.target_has_loaded_page(background_target.target_id()),
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
            context.target_document_renderer_agent_for_test(background_target.target_id()),
            Some(active_attachment.agent_token()),
            "the background Page and its renderer channel must retain the same physical agent"
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
                .filter(
                    |target| context.has_non_default_session_state_for_target(target.target_id())
                )
                .is_some_and(|state| state.devtools_sessions
                    [moli_page_types::DevToolsSessionKey::Primary]
                    .runtime_session_state
                    .runtime_frontend_enabled),
            "session-scoped Runtime.enable state must remain owned by the target"
        );
        let background_state = context
            .background_target("TID-deactivate")
            .filter(|target| context.has_non_default_session_state_for_target(target.target_id()))
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
            context.background_targets().next().unwrap().target_id(),
            "TID-existing-bg"
        );
        assert_eq!(
            context.background_targets().next().unwrap().session_id(),
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
            context.runtime_slot_diagnostics_for_target("TID-old-active")["loadedPageAbsenceReason"],
            json!("initial-document-page-build-pending"),
            "foreground selection must preserve a pending initial document absence reason"
        );
        assert_eq!(
            context.runtime_slot_diagnostics_for_target(context.active_target_id().unwrap())["loadedPageAbsenceReason"],
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

        let handle = context
            .web_contents_handle_for_target("TID-pending-bg")
            .expect("pending background target should remain selectable");
        let completed = context
            .start_select_web_contents(handle, &Default::default())
            .unwrap()
            .wait()
            .await;
        context.finish_select_web_contents(completed).unwrap();

        assert_eq!(
            context.runtime_slot_diagnostics_for_target(context.active_target_id().unwrap())["loadedPageAbsenceReason"],
            json!("initial-document-page-build-pending"),
            "foreground selection must preserve a pending initial document absence reason"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn select_first_background_target_prefers_first_loaded_target() {
        let mut ctx = TestContext::new();

        let mut context = ctx
            .conn
            .new_browser_context_fixture_for_test("BC-activate-first".to_owned());
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
        ctx.conn.install_browser_context_fixture_for_test(context);
        ctx.install_quiet_navigation_fixture_for_session_owner(
            "data:text/html,<title>first-loaded</title>",
            Some("SID-first-loaded"),
        )
        .await;
        ctx.install_quiet_navigation_fixture_for_session_owner(
            "data:text/html,<title>second-loaded</title>",
            Some("SID-second-loaded"),
        )
        .await;
        let mut context = ctx.conn.browser_context.take().unwrap();
        let first_attachment = context
            .background_targets()
            .nth(1)
            .unwrap()
            .runtime_slot()
            .current_renderer_attachment()
            .expect("first loaded background target should have an attachment");
        let second_attachment = context
            .background_targets()
            .nth(2)
            .unwrap()
            .runtime_slot()
            .current_renderer_attachment()
            .expect("second loaded background target should have an attachment");

        let selected = context
            .background_targets()
            .find(|target| context.target_has_loaded_page(target.target_id()))
            .map(|target| target.target_id().to_owned())
            .expect("loaded background target should be selectable");
        let handle = context.web_contents_handle_for_target(&selected).unwrap();
        let completed = context
            .start_select_web_contents(handle, &Default::default())
            .unwrap()
            .wait()
            .await;
        context.finish_select_web_contents(completed).unwrap();

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
            context.target_has_loaded_page("TID-second-loaded"),
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
        let mut context = ctx
            .conn
            .new_browser_context_fixture_for_test("BC-route-swap".to_owned());
        context.set_active_target_id("TID-active-route");
        context.attach_active_session("SID-active-route".to_owned());
        context.stage_background_target(
            "TID-background-route".to_owned(),
            Some("SID-background-route".to_owned()),
            "about:blank#background".to_owned(),
            None,
            None,
        );
        ctx.conn.install_browser_context_fixture_for_test(context);
        ctx.install_quiet_navigation_fixture_for_session_owner(
            "data:text/html,<title>active route</title>",
            Some("SID-active-route"),
        )
        .await;
        ctx.install_quiet_navigation_fixture_for_session_owner(
            "data:text/html,<title>background route</title>",
            Some("SID-background-route"),
        )
        .await;
        let mut context = ctx.conn.browser_context.take().unwrap();
        let active_attachment = context
            .active_page_target()
            .runtime_slot
            .current_renderer_attachment()
            .expect("active attachment");
        let background_attachment = context
            .background_target("TID-background-route")
            .and_then(|target| target.runtime_slot().current_renderer_attachment())
            .expect("background attachment");

        let handle = context
            .web_contents_handle_for_target("TID-background-route")
            .unwrap();
        let completed = context
            .start_select_web_contents(handle, &Default::default())
            .unwrap()
            .wait()
            .await;
        context
            .finish_select_web_contents(completed)
            .expect("target selection should succeed");

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
        let devtools_session_state = context
            .background_target_mut("TID-bg")
            .expect("background target must exist")
            .devtools_sessions
            .primary_mut();
        devtools_session_state.runtime_session_state = TargetRuntimeSessionState {
            runtime_frontend_enabled: true,
            runtime_contexts_reported_to_frontend: false,
            inspector_enabled: true,
            inspector_target_crashed_delivered: false,
        };
        let page_session = &mut devtools_session_state.page_session_state;
        page_session.page_lifecycle_events = true;
        page_session.log_enabled = true;
        assert!(
            page_session
                .performance
                .enable(PerformanceTimeDomain::ThreadTicks)
        );
        page_session.page_file_chooser_opened_event_enabled = true;
        page_session.page_intercept_file_chooser_dialog_enabled = true;
        devtools_session_state
            .console_output_session_state
            .console_enabled = true;
        context
            .background_target_mut("TID-bg")
            .expect("background target")
            .runtime_slot
            .set_session_observation_cursor_at_counts_for_test(None, 4, 5);

        let handle = context.web_contents_handle_for_target("TID-bg").unwrap();
        let completed = context
            .start_select_web_contents(handle, &Default::default())
            .unwrap()
            .wait()
            .await;
        context
            .finish_select_web_contents(completed)
            .expect("target selection should not fail");

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
