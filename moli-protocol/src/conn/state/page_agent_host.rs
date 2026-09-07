use std::collections::HashMap;

use indexmap::IndexMap;
use moli_core::browser::{MainFrameSlotId, WebContentsId};
use serde_json::Value;

use super::{
    devtools_session::DevToolsSessionRegistry, fetch::TargetFetchOwner,
    identity::TargetIdentityState, page_slot::TargetPageSlot, runtime_slot::TargetRuntimeSlot,
    session::BaseNetworkRequestPolicy, target_state::TargetOwnerState,
};
#[cfg(test)]
use crate::conn::cookie_manager_surface::BrowserContextCookieManagerSurface;

/// DevTools projection of a stable Browser page and its main-frame slot.
/// Browser ownership and foreground selection belong to the physical Context.
#[derive(Debug)]
pub struct PageAgentHost {
    target_id: String,
    web_contents_id: WebContentsId,
    main_frame_slot_id: MainFrameSlotId,
    /// Immutable DevTools attribution, retained after the opener closes.
    pub(in crate::conn) opener_frame_id: Option<String>,
    pub(crate) target_identity: TargetIdentityState,
    pub(crate) devtools_sessions: DevToolsSessionRegistry,
    pub(in crate::conn::state) base_network_request_policy: BaseNetworkRequestPolicy,
    pub(in crate::conn::state) base_browser_identity: super::BaseBrowserIdentityOverrideState,
    pub(in crate::conn::state) base_locale_override: Option<String>,
    pub(in crate::conn::state) base_timezone_override: Option<String>,
    pub(crate) input_intercept_drags_enabled: bool,
    pub(crate) input_drag_intercepted: bool,
    pub(crate) css_enabled: bool,
    #[cfg(test)]
    pub(crate) document_cookie_manager_surface: BrowserContextCookieManagerSurface,
    pub(crate) dom_remote_object_node_cache: HashMap<String, Value>,
    pub(crate) runtime_slot: TargetRuntimeSlot,
    pub(crate) fetch_owner: TargetFetchOwner,
    pub(crate) owner_state: TargetOwnerState,
}

impl PageAgentHost {
    pub(crate) fn new(
        target_id: String,
        primary_session_id: Option<String>,
        target_identity: TargetIdentityState,
        web_contents_id: WebContentsId,
        main_frame_slot_id: MainFrameSlotId,
        target_page_slot: TargetPageSlot,
    ) -> Self {
        let mut host = Self {
            target_id,
            web_contents_id,
            main_frame_slot_id,
            target_identity,
            opener_frame_id: None,
            devtools_sessions: DevToolsSessionRegistry::default(),
            base_network_request_policy: BaseNetworkRequestPolicy::default(),
            base_browser_identity: super::BaseBrowserIdentityOverrideState::default(),
            base_locale_override: None,
            base_timezone_override: None,
            input_intercept_drags_enabled: false,
            input_drag_intercepted: false,
            css_enabled: false,
            #[cfg(test)]
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

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub fn web_contents_id(&self) -> WebContentsId {
        self.web_contents_id
    }

    pub fn main_frame_slot_id(&self) -> MainFrameSlotId {
        self.main_frame_slot_id
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

/// DevTools page projections. The Browser collection owns selection and close.
#[derive(Debug, Default)]
pub(crate) struct PageAgentHostRegistry {
    hosts: IndexMap<String, PageAgentHost>,
}

impl PageAgentHostRegistry {
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.hosts.len()
    }

    pub(crate) fn active(&self, selected: Option<WebContentsId>) -> Option<&PageAgentHost> {
        self.get_for_web_contents(selected?)
    }

    pub(crate) fn active_mut(
        &mut self,
        selected: Option<WebContentsId>,
    ) -> Option<&mut PageAgentHost> {
        let id = selected?;
        self.iter_mut().find(|host| host.web_contents_id() == id)
    }

    pub(in crate::conn) fn get_for_web_contents(
        &self,
        id: WebContentsId,
    ) -> Option<&PageAgentHost> {
        self.iter().find(|host| host.web_contents_id() == id)
    }

    pub(crate) fn get(&self, target_id: &str) -> Option<&PageAgentHost> {
        self.hosts.get(target_id)
    }

    pub(crate) fn get_mut(&mut self, target_id: &str) -> Option<&mut PageAgentHost> {
        self.hosts.get_mut(target_id)
    }

    pub(crate) fn insert(&mut self, host: PageAgentHost) -> bool {
        let target_id = host.target_id().to_owned();
        if self.hosts.contains_key(&target_id) {
            return false;
        }
        self.hosts.insert(target_id, host);
        true
    }

    pub(crate) fn remove(&mut self, target_id: &str) -> Option<PageAgentHost> {
        self.hosts.shift_remove(target_id)
    }

    pub(crate) fn rekey(&mut self, previous_target_id: &str, target_id: String) -> bool {
        if self.hosts.contains_key(&target_id) {
            return false;
        }
        let Some((index, _previous_target_id, mut active)) =
            self.hosts.shift_remove_full(previous_target_id)
        else {
            return false;
        };
        active.replace_target_id(target_id.clone());
        self.hosts.shift_insert(index, target_id, active);
        true
    }

    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = &PageAgentHost> {
        self.hosts.values()
    }

    pub(crate) fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut PageAgentHost> {
        self.hosts.values_mut()
    }

    pub(crate) fn background(
        &self,
        active_id: Option<WebContentsId>,
    ) -> impl DoubleEndedIterator<Item = &PageAgentHost> {
        self.iter()
            .filter(move |host| Some(host.web_contents_id()) != active_id)
    }

    pub(crate) fn background_len(&self, selected: Option<WebContentsId>) -> usize {
        self.background(selected).count()
    }

    pub(crate) fn background_is_empty(&self, selected: Option<WebContentsId>) -> bool {
        self.background(selected).next().is_none()
    }
}
