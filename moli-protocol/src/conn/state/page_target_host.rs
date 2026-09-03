use std::collections::HashMap;

use indexmap::IndexMap;
use moli_core::network::SharedWebStorageStore;
use moli_core::runtime::NavigationEngine;
use serde_json::Value;

use super::{
    devtools_session::DevToolsSessionRegistry, emulation::EffectiveTargetEmulationState,
    fetch::TargetFetchOwner, identity::TargetIdentityState, page_slot::TargetPageSlot,
    runtime_slot::TargetRuntimeSlot, session::TargetNetworkPolicyState,
    session_storage::TargetSessionStorageNamespace, target_state::TargetOwnerState,
};
use crate::conn::cookie_manager_surface::BrowserContextCookieManagerSurface;

/// The stable owner of all state that belongs to one page target.
///
/// Selecting another page never replaces or reconstructs this object. The
/// browser context registry keeps every page host alive and records foreground
/// selection separately.
#[derive(Debug)]
pub struct PageTargetHost {
    target_id: String,
    navigation_engine: Option<NavigationEngine>,
    pub(crate) target_identity: TargetIdentityState,
    pub(crate) devtools_sessions: DevToolsSessionRegistry,
    pub(crate) network_policy: TargetNetworkPolicyState,
    pub(crate) http_proxy_override: Option<String>,
    pub(crate) http_no_proxy_override: Option<String>,
    pub(crate) tls_verify_host_override: Option<bool>,
    pub(crate) base_locale_override: Option<String>,
    pub(crate) base_timezone_override: Option<String>,
    pub(crate) effective_emulation_state: EffectiveTargetEmulationState,
    pub(crate) input_intercept_drags_enabled: bool,
    pub(crate) input_drag_intercepted: bool,
    pub(crate) css_enabled: bool,
    pub(crate) document_cookie_manager_surface: BrowserContextCookieManagerSurface,
    pub(crate) dom_remote_object_node_cache: HashMap<String, Value>,
    pub(crate) runtime_slot: TargetRuntimeSlot,
    pub(crate) fetch_owner: TargetFetchOwner,
    pub(crate) owner_state: TargetOwnerState,
    pub(crate) session_storage_namespace: TargetSessionStorageNamespace,
}

impl PageTargetHost {
    pub(crate) fn empty(target_id: String) -> Self {
        Self {
            target_id,
            navigation_engine: None,
            target_identity: TargetIdentityState::about_blank(),
            devtools_sessions: DevToolsSessionRegistry::default(),
            network_policy: TargetNetworkPolicyState::default(),
            http_proxy_override: None,
            http_no_proxy_override: None,
            tls_verify_host_override: None,
            base_locale_override: None,
            base_timezone_override: None,
            effective_emulation_state: EffectiveTargetEmulationState::default(),
            input_intercept_drags_enabled: false,
            input_drag_intercepted: false,
            css_enabled: false,
            document_cookie_manager_surface: BrowserContextCookieManagerSurface::default(),
            dom_remote_object_node_cache: HashMap::new(),
            runtime_slot: TargetRuntimeSlot::default(),
            fetch_owner: TargetFetchOwner::default(),
            owner_state: TargetOwnerState::default(),
            session_storage_namespace: TargetSessionStorageNamespace::default(),
        }
    }

    pub(crate) fn new(
        target_id: String,
        primary_session_id: Option<String>,
        target_identity: TargetIdentityState,
        target_page_slot: TargetPageSlot,
    ) -> Self {
        let mut host = Self::empty(target_id);
        host.target_identity = target_identity;
        host.runtime_slot = TargetRuntimeSlot::from_page_slot(target_page_slot);
        if let Some(session_id) = primary_session_id {
            host.devtools_sessions.attach_primary(session_id);
        }
        host
    }

    #[cfg(test)]
    pub(crate) fn with_url(
        target_id: String,
        primary_session_id: Option<String>,
        url: String,
    ) -> Self {
        Self::new(
            target_id,
            primary_session_id,
            TargetIdentityState::with_url(url),
            TargetPageSlot::empty_for_test_fixture(),
        )
    }

    pub(crate) fn with_identity(
        target_id: String,
        primary_session_id: Option<String>,
        target_identity: TargetIdentityState,
    ) -> Self {
        Self::new(
            target_id,
            primary_session_id,
            target_identity,
            TargetPageSlot::empty_for_initial_document_page_build(),
        )
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    fn replace_target_id(&mut self, target_id: String) {
        self.target_id = target_id;
    }

    pub(crate) fn is_target(&self, target_id: &str) -> bool {
        self.target_id() == target_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.devtools_sessions.primary_session_id()
    }

    pub(crate) fn has_session(&self) -> bool {
        self.session_id().is_some()
    }

    pub(crate) fn is_session(&self, session_id: &str) -> bool {
        self.session_id() == Some(session_id)
    }

    pub(crate) fn attach_session(&mut self, session_id: String) {
        self.devtools_sessions.attach_primary(session_id);
    }

    #[cfg(test)]
    pub(crate) fn detach_session(&mut self) -> Option<String> {
        self.devtools_sessions.detach_primary()
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

    pub(crate) fn navigation_engine(&self) -> Option<&NavigationEngine> {
        self.navigation_engine.as_ref()
    }

    pub(crate) fn navigation_engine_mut(&mut self) -> Option<&mut NavigationEngine> {
        self.navigation_engine.as_mut()
    }

    pub(crate) fn replace_navigation_engine(
        &mut self,
        mut engine: NavigationEngine,
    ) -> Option<NavigationEngine> {
        let policy = self.effective_policy();
        engine.set_bypass_service_worker(policy.bypass_service_worker());
        engine.set_cache_disabled(policy.cache_disabled());
        self.navigation_engine.replace(engine)
    }

    pub(crate) fn set_base_cache_disabled(&mut self, disabled: bool) {
        self.network_policy.set_base_cache_disabled(disabled);
        let effective = self.effective_policy().cache_disabled();
        if let Some(engine) = self.navigation_engine.as_mut() {
            engine.set_cache_disabled(effective);
        }
    }

    pub(crate) fn navigation_engine_and_runtime_slot_mut(
        &mut self,
    ) -> Option<(&mut NavigationEngine, &mut TargetRuntimeSlot)> {
        let engine = self.navigation_engine.as_mut()?;
        Some((engine, &mut self.runtime_slot))
    }

    pub(crate) fn has_page_domain_enabled_session(&self) -> bool {
        self.devtools_sessions
            .states()
            .any(|session| session.page_session_state.page_domain_enabled)
    }

    pub(crate) fn has_pending_inspector_awaits(&self) -> bool {
        self.devtools_sessions.has_pending_inspector_awaits()
    }

    pub(crate) fn pending_inspector_await_count(&self) -> usize {
        self.devtools_sessions.pending_inspector_await_count()
    }

    pub(crate) fn has_runtime_remote_object_id(&self, object_id: &str) -> bool {
        self.devtools_sessions
            .states()
            .any(|session| session.has_runtime_remote_object_id(object_id))
    }

    pub(crate) fn has_runtime_remote_object_id_for_different_session(
        &self,
        devtools_session_id: Option<&str>,
        object_id: &str,
    ) -> bool {
        if devtools_session_id.is_some()
            && self
                .devtools_sessions
                .primary()
                .has_runtime_remote_object_id(object_id)
        {
            return true;
        }
        self.devtools_sessions
            .attached_entries()
            .any(|(session_id, session_state)| {
                Some(session_id) != devtools_session_id
                    && session_state.has_runtime_remote_object_id(object_id)
            })
    }
}

/// All page hosts in one browser context plus the foreground selector.
///
/// Insertion order is retained for Chromium-compatible fallback selection
/// when the foreground page closes.
#[derive(Debug, Default)]
pub(crate) struct PageTargetRegistry {
    active_target_id: Option<String>,
    hosts: IndexMap<String, PageTargetHost>,
}

impl PageTargetRegistry {
    pub(crate) fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.hosts.len()
    }

    pub(crate) fn active_target_id(&self) -> Option<&str> {
        self.active_target_id.as_deref()
    }

    pub(crate) fn active(&self) -> Option<&PageTargetHost> {
        self.get(self.active_target_id()?)
    }

    pub(crate) fn active_mut(&mut self) -> Option<&mut PageTargetHost> {
        let target_id = self.active_target_id.clone()?;
        self.get_mut(&target_id)
    }

    pub(crate) fn get(&self, target_id: &str) -> Option<&PageTargetHost> {
        self.hosts.get(target_id)
    }

    pub(crate) fn get_mut(&mut self, target_id: &str) -> Option<&mut PageTargetHost> {
        self.hosts.get_mut(target_id)
    }

    pub(crate) fn insert(&mut self, host: PageTargetHost) -> bool {
        let target_id = host.target_id().to_owned();
        if self.hosts.contains_key(&target_id) {
            return false;
        }
        self.hosts.insert(target_id, host);
        true
    }

    pub(crate) fn remove(&mut self, target_id: &str) -> Option<PageTargetHost> {
        if self.active_target_id() == Some(target_id) {
            self.active_target_id = None;
        }
        self.hosts.shift_remove(target_id)
    }

    pub(crate) fn select(&mut self, target_id: &str) -> bool {
        if self.get(target_id).is_none() {
            return false;
        }
        self.active_target_id = Some(target_id.to_owned());
        true
    }

    pub(crate) fn rekey_active(&mut self, target_id: String) -> bool {
        if self.hosts.contains_key(&target_id) {
            return false;
        }
        let Some(previous_target_id) = self.active_target_id.clone() else {
            return false;
        };
        let Some((index, _previous_target_id, mut active)) =
            self.hosts.shift_remove_full(&previous_target_id)
        else {
            return false;
        };
        active.replace_target_id(target_id.clone());
        self.hosts.shift_insert(index, target_id.clone(), active);
        self.active_target_id = Some(target_id);
        true
    }

    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = &PageTargetHost> {
        self.hosts.values()
    }

    pub(crate) fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut PageTargetHost> {
        self.hosts.values_mut()
    }

    pub(crate) fn background(&self) -> impl DoubleEndedIterator<Item = &PageTargetHost> {
        let active_target_id = self.active_target_id();
        self.iter()
            .filter(move |host| Some(host.target_id()) != active_target_id)
    }

    pub(crate) fn background_mut(
        &mut self,
    ) -> impl DoubleEndedIterator<Item = &mut PageTargetHost> {
        let active_target_id = self.active_target_id.clone();
        self.iter_mut()
            .filter(move |host| Some(host.target_id()) != active_target_id.as_deref())
    }

    pub(crate) fn background_at(&self, index: usize) -> Option<&PageTargetHost> {
        self.background().nth(index)
    }

    pub(crate) fn background_at_mut(&mut self, index: usize) -> Option<&mut PageTargetHost> {
        self.background_mut().nth(index)
    }

    pub(crate) fn background_len(&self) -> usize {
        self.background().count()
    }

    pub(crate) fn background_is_empty(&self) -> bool {
        self.background().next().is_none()
    }
}
