use std::collections::HashMap;

use indexmap::IndexMap;
use moli_core::browser::{DocumentId, MainFrameSlotId, WebContentsId};
use moli_core::network::SharedWebStorageStore;
use moli_core::runtime::NavigationEngine;
use serde_json::Value;

use super::{
    SessionStorageNamespace,
    devtools_session::DevToolsSessionRegistry,
    fetch::TargetFetchOwner,
    identity::TargetIdentityState,
    page_slot::TargetPageSlot,
    runtime_slot::TargetRuntimeSlot,
    session::TargetNetworkPolicyState,
    target_state::TargetOwnerState,
    web_contents::{
        EmulationPolicy, EmulationPolicyChange, EmulationPolicyDelta, WindowSurface,
        WindowSurfaceState,
    },
};
use crate::conn::cookie_manager_surface::BrowserContextCookieManagerSurface;

/// DevTools page projection with a temporarily embedded Browser residence.
///
/// Selecting another page never replaces or reconstructs this object. The
/// browser context registry keeps every page host alive and records foreground
/// selection separately. Browser identity and document ownership live in the
/// embedded WebContents, which moves out at the typed API cutover (Commit 24b).
#[derive(Debug)]
pub struct PageTargetHost {
    target_id: String,
    /// Immutable DevTools attribution, retained after the opener closes.
    pub(in crate::conn) opener_frame_id: Option<String>,
    pub(crate) target_identity: TargetIdentityState,
    pub(crate) devtools_sessions: DevToolsSessionRegistry,
    pub(crate) network_policy: TargetNetworkPolicyState,
    pub(in crate::conn::state) base_browser_identity: super::BaseBrowserIdentityOverrideState,
    pub(crate) http_proxy_override: Option<String>,
    pub(crate) http_no_proxy_override: Option<String>,
    pub(crate) tls_verify_host_override: Option<bool>,
    pub(in crate::conn::state) base_locale_override: Option<String>,
    pub(in crate::conn::state) base_timezone_override: Option<String>,
    pub(crate) input_intercept_drags_enabled: bool,
    pub(crate) input_drag_intercepted: bool,
    pub(crate) css_enabled: bool,
    pub(crate) document_cookie_manager_surface: BrowserContextCookieManagerSurface,
    pub(crate) dom_remote_object_node_cache: HashMap<String, Value>,
    pub(crate) runtime_slot: TargetRuntimeSlot,
    pub(crate) fetch_owner: TargetFetchOwner,
    pub(crate) owner_state: TargetOwnerState,
}

impl PageTargetHost {
    pub(crate) fn empty(target_id: String) -> Self {
        Self::new(
            target_id,
            None,
            TargetIdentityState::about_blank(),
            TargetPageSlot::default(),
        )
    }

    pub(crate) fn new(
        target_id: String,
        primary_session_id: Option<String>,
        target_identity: TargetIdentityState,
        target_page_slot: TargetPageSlot,
    ) -> Self {
        let mut host = Self {
            target_id,
            target_identity,
            opener_frame_id: None,
            devtools_sessions: DevToolsSessionRegistry::default(),
            network_policy: TargetNetworkPolicyState::default(),
            base_browser_identity: super::BaseBrowserIdentityOverrideState::default(),
            http_proxy_override: None,
            http_no_proxy_override: None,
            tls_verify_host_override: None,
            base_locale_override: None,
            base_timezone_override: None,
            input_intercept_drags_enabled: false,
            input_drag_intercepted: false,
            css_enabled: false,
            document_cookie_manager_surface: BrowserContextCookieManagerSurface::default(),
            dom_remote_object_node_cache: HashMap::new(),
            runtime_slot: TargetRuntimeSlot::from_page_slot(target_page_slot),
            fetch_owner: TargetFetchOwner::default(),
            owner_state: TargetOwnerState::default(),
        };
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

    pub fn web_contents_id(&self) -> WebContentsId {
        self.runtime_slot.page_slot().contents.id()
    }

    pub fn main_frame_slot_id(&self) -> MainFrameSlotId {
        self.runtime_slot.page_slot().contents.main_frame.id()
    }

    pub fn current_document_id(&self) -> Option<DocumentId> {
        self.runtime_slot.document_id()
    }

    pub(crate) fn is_crashed(&self) -> bool {
        self.runtime_slot.page_slot().contents.crashed
    }

    pub(crate) fn mark_crashed(&mut self) {
        self.runtime_slot.page_slot_mut().contents.crashed = true;
    }

    pub(crate) fn clear_crash_state(&mut self) {
        self.runtime_slot.page_slot_mut().contents.crashed = false;
    }

    pub(crate) fn window_surface(&self) -> WindowSurface {
        self.runtime_slot.page_slot().contents.window.surface
    }

    pub(crate) fn emulation_policy(&self) -> &EmulationPolicy {
        &self.runtime_slot.page_slot().contents.emulation_policy
    }

    pub(crate) fn apply_emulation_policy_change(&mut self, change: EmulationPolicyChange) {
        self.runtime_slot
            .page_slot_mut()
            .contents
            .emulation_policy
            .apply(change);
    }

    pub(in crate::conn) fn apply_emulation_policy_changes(
        &mut self,
        changes: Vec<EmulationPolicyChange>,
    ) -> EmulationPolicyDelta {
        self.runtime_slot
            .page_slot_mut()
            .contents
            .emulation_policy
            .apply_changes(changes)
    }

    pub(in crate::conn) fn set_window_surface_state(&mut self, state: WindowSurfaceState) {
        self.runtime_slot
            .page_slot_mut()
            .contents
            .window
            .surface
            .state = state;
    }

    pub(in crate::conn) fn set_window_surface_geometry(
        &mut self,
        width: Option<u32>,
        height: Option<u32>,
        x: Option<i32>,
        y: Option<i32>,
    ) {
        self.runtime_slot
            .page_slot_mut()
            .contents
            .window
            .surface
            .set_geometry(width, height, x, y);
    }

    pub(crate) fn initial_empty_document_state(&self) -> Option<&super::InitialDocument> {
        self.runtime_slot
            .page_slot()
            .contents
            .navigation
            .initial_empty_document_state()
    }

    pub(in crate::conn) fn initial_empty_document_loader_id_if_current(&self) -> Option<String> {
        self.initial_empty_document_state()
            .filter(|document| document.is_on_initial_empty_document())
            .map(|_| format!("LID-INITIAL-{}", self.target_id()))
    }

    pub(in crate::conn) fn commit_document_title(&mut self, title: String) -> bool {
        let changed = self
            .owner_state
            .committed_document_title()
            .unwrap_or_default()
            != title;
        self.owner_state.committed_document_title = Some(title.clone());
        self.runtime_slot
            .page_slot_mut()
            .contents
            .navigation
            .refresh_current_navigation_history_title(title);
        changed
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
        self.runtime_slot
            .page_slot()
            .contents
            .session_storage
            .store()
    }

    pub(crate) fn deep_clone_session_storage_namespace(&self) -> SessionStorageNamespace {
        self.runtime_slot
            .page_slot()
            .contents
            .session_storage
            .deep_clone()
    }

    pub(crate) fn replace_session_storage_namespace(&mut self, namespace: SessionStorageNamespace) {
        self.runtime_slot.page_slot_mut().contents.session_storage = namespace;
    }

    pub(crate) fn navigation_engine(&self) -> Option<&NavigationEngine> {
        self.runtime_slot
            .page_slot()
            .contents
            .navigation_engine
            .as_ref()
    }

    pub(crate) fn navigation_engine_mut(&mut self) -> Option<&mut NavigationEngine> {
        self.runtime_slot
            .page_slot_mut()
            .contents
            .navigation_engine
            .as_mut()
    }

    pub(crate) fn install_navigation_engine(&mut self, engine: NavigationEngine) {
        self.runtime_slot
            .page_slot_mut()
            .contents
            .install_navigation_engine(engine);
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
    // Browser selection; moves with the physical collection at Commit 7.
    active_web_contents_id: Option<WebContentsId>,
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
        self.active().map(PageTargetHost::target_id)
    }

    pub(crate) fn active(&self) -> Option<&PageTargetHost> {
        self.get_for_web_contents(self.active_web_contents_id?)
    }

    pub(crate) fn active_mut(&mut self) -> Option<&mut PageTargetHost> {
        let id = self.active_web_contents_id?;
        self.iter_mut().find(|host| host.web_contents_id() == id)
    }

    pub(in crate::conn) fn get_for_web_contents(
        &self,
        id: WebContentsId,
    ) -> Option<&PageTargetHost> {
        self.iter().find(|host| host.web_contents_id() == id)
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
        let removed = self.hosts.shift_remove(target_id)?;
        let removed_id = removed.web_contents_id();
        if self.active_web_contents_id == Some(removed_id) {
            self.active_web_contents_id = None;
        }
        for host in self.iter_mut() {
            let window = &mut host.runtime_slot.page_slot_mut().contents.window;
            if window
                .opener
                .is_some_and(|opener| opener.web_contents_id == removed_id)
            {
                window.opener = None;
            }
        }
        Some(removed)
    }

    pub(crate) fn select(&mut self, target_id: &str) -> bool {
        let Some(host) = self.get(target_id) else {
            return false;
        };
        self.active_web_contents_id = Some(host.web_contents_id());
        true
    }

    pub(crate) fn rekey_active(&mut self, target_id: String) -> bool {
        if self.hosts.contains_key(&target_id) {
            return false;
        }
        let Some(previous_target_id) = self.active_target_id().map(str::to_owned) else {
            return false;
        };
        let Some((index, _previous_target_id, mut active)) =
            self.hosts.shift_remove_full(&previous_target_id)
        else {
            return false;
        };
        active.replace_target_id(target_id.clone());
        self.hosts.shift_insert(index, target_id, active);
        true
    }

    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = &PageTargetHost> {
        self.hosts.values()
    }

    pub(crate) fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut PageTargetHost> {
        self.hosts.values_mut()
    }

    pub(crate) fn background(&self) -> impl DoubleEndedIterator<Item = &PageTargetHost> {
        let active_id = self.active_web_contents_id;
        self.iter()
            .filter(move |host| Some(host.web_contents_id()) != active_id)
    }

    pub(crate) fn background_mut(
        &mut self,
    ) -> impl DoubleEndedIterator<Item = &mut PageTargetHost> {
        let active_id = self.active_web_contents_id;
        self.iter_mut()
            .filter(move |host| Some(host.web_contents_id()) != active_id)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_and_crash_state_outlive_the_devtools_projection() {
        let mut target = PageTargetHost::empty("TID-window".into());
        target.attach_session("SID-window".into());
        target.set_window_surface_state(WindowSurfaceState::Minimized);
        target.set_window_surface_geometry(Some(800), Some(600), Some(-10), Some(20));
        target.mark_crashed();
        let id = target.web_contents_id();
        let surface = target.window_surface();
        let opener = WebContentsId::allocate();
        let window = &mut target.runtime_slot.page_slot_mut().contents.window;
        window.name = Some("report".into());
        window.opener = Some(super::super::WindowOpener {
            web_contents_id: opener,
            can_access: true,
        });
        target.opener_frame_id = Some("FRAME-opener".into());

        assert_eq!(target.detach_session().as_deref(), Some("SID-window"));
        assert!(target.is_crashed());
        assert_eq!(target.window_surface(), surface);
        // Only the non-Clone Browser subtree survives, not the Target shell.
        let contents = {
            let mut projection = target;
            std::mem::take(&mut projection.runtime_slot.page_slot_mut().contents)
        };
        assert_eq!(contents.id(), id);
        assert!(contents.crashed);
        assert_eq!(contents.window.surface, surface);
        assert_eq!(contents.window.name.as_deref(), Some("report"));
        let relationship = contents.window.opener.unwrap();
        assert_eq!(relationship.web_contents_id, opener);
        assert!(relationship.can_access);

        let replacement = PageTargetHost::empty("TID-window".into());
        assert_ne!(replacement.web_contents_id(), id);
        assert!(!replacement.is_crashed());
        assert_eq!(replacement.window_surface(), WindowSurface::default());
        assert!(
            replacement
                .runtime_slot
                .page_slot()
                .contents
                .window
                .name
                .is_none()
        );
        assert!(
            replacement
                .runtime_slot
                .page_slot()
                .contents
                .window
                .opener
                .is_none()
        );
    }
}
