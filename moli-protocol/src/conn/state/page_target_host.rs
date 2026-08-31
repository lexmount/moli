use moli_core::network::SharedWebStorageStore;
use moli_core::runtime::NavigationEngine;

use super::{
    identity::TargetIdentityState,
    page_slot::TargetPageSlot,
    runtime_slot::TargetRuntimeSlot,
    session::{ActiveTargetState, TargetPageState},
    session_storage::TargetSessionStorageNamespace,
};

/// The stable owner of all state that belongs to one page target.
///
/// Selecting another page never replaces or reconstructs this object. The
/// browser context registry keeps every page host alive and records foreground
/// selection separately.
#[derive(Debug)]
pub struct PageTargetHost {
    target_id: String,
    primary_session_id: Option<String>,
    state: Box<TargetPageState>,
    navigation_engine: Option<NavigationEngine>,
}

impl PageTargetHost {
    pub(crate) fn empty(target_id: String) -> Self {
        Self {
            target_id,
            primary_session_id: None,
            state: Box::default(),
            navigation_engine: None,
        }
    }

    pub(crate) fn new(
        target_id: String,
        primary_session_id: Option<String>,
        target_identity: TargetIdentityState,
        target_page_slot: TargetPageSlot,
    ) -> Self {
        Self {
            target_id,
            primary_session_id,
            state: Box::new(TargetPageState {
                target_identity,
                active_target: ActiveTargetState {
                    runtime_slot: TargetRuntimeSlot::from_page_slot(target_page_slot),
                    ..Default::default()
                },
                ..Default::default()
            }),
            navigation_engine: None,
        }
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
        self.primary_session_id.as_deref()
    }

    pub(crate) fn has_session(&self) -> bool {
        self.primary_session_id.is_some()
    }

    pub(crate) fn is_session(&self, session_id: &str) -> bool {
        self.session_id() == Some(session_id)
    }

    pub(crate) fn attach_session(&mut self, session_id: String) {
        self.primary_session_id = Some(session_id);
    }

    pub(crate) fn detach_session(&mut self) -> Option<String> {
        self.primary_session_id.take()
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

    pub(crate) fn state(&self) -> &TargetPageState {
        &self.state
    }

    pub(crate) fn state_mut(&mut self) -> &mut TargetPageState {
        &mut self.state
    }

    pub(crate) fn navigation_engine(&self) -> Option<&NavigationEngine> {
        self.navigation_engine.as_ref()
    }

    pub(crate) fn navigation_engine_mut(&mut self) -> Option<&mut NavigationEngine> {
        self.navigation_engine.as_mut()
    }

    pub(crate) fn replace_navigation_engine(
        &mut self,
        engine: NavigationEngine,
    ) -> Option<NavigationEngine> {
        self.navigation_engine.replace(engine)
    }

    pub(crate) fn navigation_engine_and_runtime_slot_mut(
        &mut self,
    ) -> Option<(&mut NavigationEngine, &mut TargetRuntimeSlot)> {
        let engine = self.navigation_engine.as_mut()?;
        Some((engine, &mut self.state.active_target.runtime_slot))
    }

    pub(crate) fn has_page_domain_enabled_session(&self) -> bool {
        self.devtools_sessions
            .states()
            .any(|session| session.page_session_state.page_domain_enabled)
    }

    pub(crate) fn has_pending_javascript_dialog(&self) -> bool {
        self.state.has_pending_javascript_dialog()
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

impl std::ops::Deref for PageTargetHost {
    type Target = TargetPageState;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl std::ops::DerefMut for PageTargetHost {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

/// All page hosts in one browser context plus the foreground selector.
///
/// Insertion order is retained for Chromium-compatible fallback selection
/// when the foreground page closes.
#[derive(Debug, Default)]
pub(crate) struct PageTargetRegistry {
    active_target_id: Option<String>,
    hosts: Vec<PageTargetHost>,
}

impl PageTargetRegistry {
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
        self.hosts.iter().find(|host| host.is_target(target_id))
    }

    pub(crate) fn get_mut(&mut self, target_id: &str) -> Option<&mut PageTargetHost> {
        self.hosts.iter_mut().find(|host| host.is_target(target_id))
    }

    pub(crate) fn insert(&mut self, host: PageTargetHost) -> bool {
        if self.get(host.target_id()).is_some() {
            return false;
        }
        self.hosts.push(host);
        true
    }

    pub(crate) fn remove(&mut self, target_id: &str) -> Option<PageTargetHost> {
        let index = self
            .hosts
            .iter()
            .position(|host| host.is_target(target_id))?;
        if self.active_target_id() == Some(target_id) {
            self.active_target_id = None;
        }
        Some(self.hosts.remove(index))
    }

    pub(crate) fn select(&mut self, target_id: &str) -> bool {
        if self.get(target_id).is_none() {
            return false;
        }
        self.active_target_id = Some(target_id.to_owned());
        true
    }

    pub(crate) fn rekey_active(&mut self, target_id: String) -> bool {
        if self.get(&target_id).is_some() {
            return false;
        }
        let Some(active) = self.active_mut() else {
            return false;
        };
        active.replace_target_id(target_id.clone());
        self.active_target_id = Some(target_id);
        true
    }

    #[cfg(test)]
    pub(crate) fn clear_selection(&mut self) {
        self.active_target_id = None;
    }

    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = &PageTargetHost> {
        self.hosts.iter()
    }

    pub(crate) fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut PageTargetHost> {
        self.hosts.iter_mut()
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
