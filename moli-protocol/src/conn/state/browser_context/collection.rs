use super::BrowserContext;
use crate::conn::state::{PageAgentHost, TargetIdentityState, page_slot::TargetPageSlot};
use moli_core::browser::web_contents::{
    EmulationPolicy, EmulationPolicyChange, WindowSurface, WindowSurfaceState,
};
use moli_core::browser::{WebContentsCreation, WebContentsHandle, WebContentsId};

#[cfg(test)]
mod tests;

impl BrowserContext {
    #[cfg(test)]
    pub(crate) fn set_active_document_fixture_for_test(
        &mut self,
        raw: u64,
    ) -> crate::conn::state::DocumentId {
        let target_id = self
            .active_target_id_owned()
            .expect("active fixture target");
        self.set_document_id_for_test_for_target(&target_id, raw)
    }

    /// Capture only the exact Browser document capability used by inspection
    /// independence tests. The physical Document remains on its Browser owner.
    #[cfg(test)]
    pub(in crate::conn) fn inspection_document_handle_for_test(
        &self,
        target_id: &str,
    ) -> Option<(
        moli_core::browser::BrowserContextHandle,
        moli_core::browser::DocumentHandle,
    )> {
        Some((
            self.browser_context.clone(),
            self.document_handle_for_target(target_id)?,
        ))
    }

    #[cfg(test)]
    pub(crate) fn register_page_target_fixture(
        &mut self,
        target_id: String,
        primary_session_id: Option<String>,
        identity: TargetIdentityState,
        page_projection: TargetPageSlot,
    ) -> bool {
        self.register_web_contents_target(
            target_id,
            primary_session_id,
            identity,
            WebContentsCreation::default(),
            page_projection,
        )
    }

    #[cfg(test)]
    pub(crate) fn register_page_target_url_fixture(
        &mut self,
        target_id: String,
        primary_session_id: Option<String>,
        url: String,
    ) -> bool {
        self.register_page_target_fixture(
            target_id,
            primary_session_id,
            TargetIdentityState::with_url(url),
            TargetPageSlot::empty_for_test_fixture(),
        )
    }

    pub(super) fn register_web_contents_target(
        &mut self,
        target_id: String,
        primary_session_id: Option<String>,
        identity: TargetIdentityState,
        creation: WebContentsCreation,
        page_projection: TargetPageSlot,
    ) -> bool {
        if self.page_targets.get(&target_id).is_some() {
            return false;
        }
        let (handle, main_frame) = self
            .browser_context
            .create_web_contents(creation)
            .expect("new WebContents identity must be unique");
        let id = handle.id();
        let projection = PageAgentHost::new(
            target_id,
            primary_session_id,
            identity,
            id,
            main_frame,
            page_projection,
        );
        #[cfg(test)]
        let mut projection = projection;
        #[cfg(test)]
        if self.page_targets.is_empty() {
            projection.document_cookie_manager_surface =
                self.default_document_cookie_manager_surface.clone();
        }
        let inserted = self.page_targets.insert(projection);
        debug_assert!(
            inserted,
            "checked page projection must remain absent during registration"
        );
        true
    }

    pub(super) fn select_registered_web_contents(
        &mut self,
        handle: WebContentsHandle,
    ) -> Result<(), String> {
        if !self.browser_context.contains_web_contents(handle) {
            return Err("WebContents unavailable".into());
        }
        let selected = self.browser_context.select_web_contents(handle.id());
        debug_assert!(selected, "registered WebContents must be selectable");
        Ok(())
    }

    pub(crate) fn selected_web_contents_id(&self) -> Option<WebContentsId> {
        self.browser_context.selected_web_contents_id()
    }

    pub(crate) fn selected_web_contents_handle(&self) -> Option<WebContentsHandle> {
        self.browser_context.selected_web_contents_handle()
    }

    pub(crate) fn web_contents_handle_for_target(
        &self,
        target_id: &str,
    ) -> Option<WebContentsHandle> {
        Some(WebContentsHandle::new(
            self.browser_context.id(),
            self.page_targets.get(target_id)?.web_contents_id(),
        ))
    }

    pub(crate) fn web_contents_handle_for_window_id(
        &self,
        window_id: u64,
    ) -> Option<WebContentsHandle> {
        self.browser_context
            .web_contents_handle_for_window_id(window_id)
    }

    pub(crate) fn target_is_crashed(&self, target_id: &str) -> bool {
        self.web_contents_handle_for_target(target_id)
            .is_some_and(|handle| {
                self.browser_context
                    .web_contents_is_crashed(handle)
                    .unwrap_or(false)
            })
    }

    pub(crate) fn target_initial_empty_document_state(
        &self,
        target_id: &str,
    ) -> Option<crate::conn::state::InitialDocumentSnapshot> {
        let handle = self.web_contents_handle_for_target(target_id)?;
        self.browser_context
            .web_contents_initial_document_state(handle)
            .ok()?
    }

    pub(crate) fn target_initial_empty_document_loader_id_if_current(
        &self,
        target_id: &str,
    ) -> Option<String> {
        self.target_initial_empty_document_state(target_id)
            .filter(|document| document.is_on_initial_empty_document())
            .map(|_| format!("LID-INITIAL-{target_id}"))
    }

    pub(crate) fn commit_target_document_title(
        &mut self,
        target_id: &str,
        change: &moli_core::RendererDocumentTitleChanged,
    ) -> Option<bool> {
        let handle = self.web_contents_handle_for_target(target_id)?;
        let changed = self
            .browser_context
            .commit_document_title(handle, change)
            .ok()??;
        self.page_targets
            .get_mut(target_id)?
            .owner_state
            .committed_document_title = Some(change.title.clone());
        Some(changed)
    }

    pub(crate) fn set_target_crash_state(&mut self, target_id: &str, crashed: bool) {
        if let Some(handle) = self.web_contents_handle_for_target(target_id) {
            let _ = self
                .browser_context
                .set_web_contents_crashed(handle, crashed);
        }
    }

    pub(crate) fn web_contents_window_surface(
        &self,
        handle: WebContentsHandle,
    ) -> Result<WindowSurface, String> {
        self.browser_context.web_contents_window_surface(handle)
    }

    pub(crate) fn update_web_contents_window_surface(
        &mut self,
        handle: WebContentsHandle,
        state: Option<WindowSurfaceState>,
        width: Option<u32>,
        height: Option<u32>,
        x: Option<i32>,
        y: Option<i32>,
    ) -> Result<(), String> {
        self.browser_context
            .update_web_contents_window_surface(handle, state, width, height, x, y)
    }

    pub(crate) fn set_web_contents_window_name(
        &mut self,
        handle: WebContentsHandle,
        name: Option<String>,
    ) -> Result<(), String> {
        self.browser_context
            .set_web_contents_window_name(handle, name)
    }

    pub(crate) fn set_web_contents_opener(
        &mut self,
        handle: WebContentsHandle,
        opener: Option<WebContentsHandle>,
        can_access: bool,
    ) -> Result<(), String> {
        self.browser_context
            .set_web_contents_opener(handle, opener, can_access)
    }

    pub(crate) fn set_web_contents_network_offline(
        &mut self,
        handle: WebContentsHandle,
        offline: bool,
    ) -> Result<(), String> {
        self.browser_context
            .set_web_contents_network_offline(handle, offline)
    }

    pub(crate) fn target_emulation_policy(&self, target_id: &str) -> Option<EmulationPolicy> {
        let handle = self.web_contents_handle_for_target(target_id)?;
        self.browser_context
            .web_contents_emulation_policy(handle)
            .ok()
    }

    pub(crate) fn apply_target_emulation_policy_change(
        &mut self,
        target_id: &str,
        change: EmulationPolicyChange,
    ) {
        if let Some(handle) = self.web_contents_handle_for_target(target_id) {
            let _ = self
                .browser_context
                .apply_web_contents_emulation_policy_change(handle, change);
        }
    }

    pub(crate) fn apply_target_emulation_policy_changes(
        &mut self,
        target_id: &str,
        changes: Vec<EmulationPolicyChange>,
    ) {
        if let Some(handle) = self.web_contents_handle_for_target(target_id) {
            let _ = self
                .browser_context
                .apply_web_contents_emulation_policy_changes(handle, changes);
        }
    }
}
