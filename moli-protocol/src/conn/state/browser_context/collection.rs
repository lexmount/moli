use super::BrowserContext;
use crate::conn::state::{
    PageTargetHost, TargetIdentityState,
    page_slot::TargetPageSlot,
    web_contents::{
        EmulationPolicy, EmulationPolicyChange, WebContents, WindowSurface, WindowSurfaceState,
    },
};
use moli_core::browser::WebContentsId;

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

    /// Move the physical Document out only in capability-independence tests.
    /// Deliberately leave its inspection binding installed: tests must prove
    /// that dispatch can start and finish without a borrowed Browser owner.
    #[cfg(test)]
    pub(in crate::conn) fn take_document_host_for_inspection_test(
        &mut self,
        target_id: &str,
    ) -> Option<crate::conn::state::web_contents::DocumentHost> {
        self.web_contents_for_target_mut(target_id)?
            .main_frame
            .current_document
            .take()
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
            WebContents::default(),
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
        mut contents: WebContents,
        page_projection: TargetPageSlot,
    ) -> bool {
        if self.page_targets.get(&target_id).is_some() {
            return false;
        }
        let id = contents.id();
        let mut projection = PageTargetHost::new(
            target_id,
            primary_session_id,
            identity,
            id,
            contents.main_frame.id(),
            page_projection,
        );
        #[cfg(test)]
        if self.page_targets.is_empty() {
            projection.document_cookie_manager_surface =
                self.default_document_cookie_manager_surface.clone();
        }
        projection.base_network_request_policy =
            crate::conn::state::session::BaseNetworkRequestPolicy::with_cache_disabled(
                self.global_cache_disabled,
            );
        contents.network_request_policy.cache_disabled = self.global_cache_disabled;
        if let Some(config) = self.page_navigation_runtime_config() {
            contents.install_navigation_engine(self.new_page_navigation_engine(config));
        }
        assert!(
            self.physical.web_contents.insert(id, contents).is_none(),
            "WebContents identity must be unique"
        );
        let inserted = self.page_targets.insert(projection);
        debug_assert!(
            inserted,
            "checked page projection must remain absent during registration"
        );
        true
    }

    pub(super) fn select_registered_page_target(&mut self, target_id: &str) -> bool {
        let Some(id) = self
            .page_targets
            .get(target_id)
            .map(PageTargetHost::web_contents_id)
        else {
            return false;
        };
        self.physical.select_web_contents(id)
    }

    pub(crate) fn selected_web_contents_id(&self) -> Option<WebContentsId> {
        self.physical.selected_web_contents_id()
    }

    pub(crate) fn target_is_crashed(&self, target_id: &str) -> bool {
        self.web_contents_for_target(target_id)
            .is_some_and(|contents| contents.crashed)
    }

    pub(crate) fn target_initial_empty_document_state(
        &self,
        target_id: &str,
    ) -> Option<&crate::conn::state::InitialDocument> {
        self.web_contents_for_target(target_id)?
            .navigation()
            .initial_empty_document_state()
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
        let changed = self
            .web_contents_for_target_mut(target_id)?
            .commit_document_title(change)?;
        self.page_targets
            .get_mut(target_id)?
            .owner_state
            .committed_document_title = Some(change.title.clone());
        Some(changed)
    }

    pub(crate) fn set_target_crash_state(&mut self, target_id: &str, crashed: bool) {
        if let Some(contents) = self.web_contents_for_target_mut(target_id) {
            contents.crashed = crashed;
        }
    }

    pub(crate) fn target_window_surface(&self, target_id: &str) -> Option<WindowSurface> {
        Some(self.web_contents_for_target(target_id)?.window.surface)
    }

    pub(crate) fn set_target_window_surface_state(
        &mut self,
        target_id: &str,
        state: WindowSurfaceState,
    ) {
        if let Some(contents) = self.web_contents_for_target_mut(target_id) {
            contents.window.surface.state = state;
        }
    }

    pub(crate) fn set_target_window_surface_geometry(
        &mut self,
        target_id: &str,
        width: Option<u32>,
        height: Option<u32>,
        x: Option<i32>,
        y: Option<i32>,
    ) {
        if let Some(contents) = self.web_contents_for_target_mut(target_id) {
            contents.window.surface.set_geometry(width, height, x, y);
        }
    }

    pub(crate) fn target_emulation_policy(&self, target_id: &str) -> Option<&EmulationPolicy> {
        Some(&self.web_contents_for_target(target_id)?.emulation_policy)
    }

    pub(crate) fn apply_target_emulation_policy_change(
        &mut self,
        target_id: &str,
        change: EmulationPolicyChange,
    ) {
        if let Some(contents) = self.web_contents_for_target_mut(target_id) {
            contents.emulation_policy.apply(change);
        }
    }

    pub(crate) fn apply_target_emulation_policy_changes(
        &mut self,
        target_id: &str,
        changes: Vec<EmulationPolicyChange>,
    ) {
        if let Some(contents) = self.web_contents_for_target_mut(target_id) {
            contents.emulation_policy.apply_changes(changes);
        }
    }
}
