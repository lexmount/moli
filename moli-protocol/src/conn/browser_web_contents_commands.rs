use moli_core::browser::WebContentsHandle;

use crate::conn::{
    CdpConnection, CommandOwnerScope, TargetPageResidenceIdentity, WindowSurface,
    WindowSurfaceState,
};

impl CdpConnection {
    /// Resolves a frontend route once into a stable physical WebContents
    /// capability. Browser operations below never receive Target or Session
    /// identifiers.
    pub(crate) fn browser_web_contents_for_owner(
        &self,
        owner: &CommandOwnerScope,
    ) -> Result<WebContentsHandle, String> {
        let (context_id, target_id) = self
            .target_owner_identity_for_owner(owner)
            .ok_or_else(|| "BrowserContextNotLoaded".to_owned())?;
        let context = self
            .browser_context_by_id(&context_id)
            .ok_or_else(|| "BrowserContextNotLoaded".to_owned())?;
        match target_id {
            Some(target_id) => context
                .web_contents_handle_for_target(&target_id)
                .ok_or_else(|| "NoSuchTarget".to_owned()),
            None => context
                .selected_web_contents_handle()
                .ok_or_else(|| "NoSuchTarget".to_owned()),
        }
    }

    pub(crate) fn browser_web_contents_for_target(
        &self,
        target_id: &str,
    ) -> Result<WebContentsHandle, String> {
        self.browser_contexts()
            .find_map(|context| context.web_contents_handle_for_target(target_id))
            .ok_or_else(|| "NoSuchTarget".to_owned())
    }

    pub(crate) fn browser_web_contents_for_page_residence(
        &self,
        page: &TargetPageResidenceIdentity,
    ) -> Result<WebContentsHandle, String> {
        if !self.target_page_residence_identity_is_current(page) {
            return Err("WebContents unavailable".to_owned());
        }
        let target_id = page
            .target_id()
            .ok_or_else(|| "WebContents unavailable".to_owned())?;
        self.browser_context_by_id(page.browser_context_id())
            .and_then(|context| context.web_contents_handle_for_target(target_id))
            .ok_or_else(|| "WebContents unavailable".to_owned())
    }

    pub(crate) fn browser_web_contents_for_window_id(
        &self,
        window_id: u64,
    ) -> Result<WebContentsHandle, String> {
        self.browser_contexts()
            .find_map(|context| context.web_contents_handle_for_window_id(window_id))
            .ok_or_else(|| "UnknownWindowId".to_owned())
    }

    pub(crate) fn browser_window_surface(
        &self,
        handle: WebContentsHandle,
    ) -> Result<WindowSurface, String> {
        self.browser_context_by_browser_id(handle.context())
            .ok_or_else(|| "WebContents unavailable".to_owned())?
            .web_contents_window_surface(handle)
    }

    pub(crate) fn crash_browser_web_contents_renderer_from_io(
        &self,
        handle: WebContentsHandle,
    ) -> Result<(), String> {
        self.browser_context_by_browser_id(handle.context())
            .ok_or_else(|| "WebContents unavailable".to_owned())?
            .browser_context
            .crash_web_contents_renderer_from_io(handle)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_browser_window_surface(
        &mut self,
        handle: WebContentsHandle,
        state: Option<WindowSurfaceState>,
        width: Option<u32>,
        height: Option<u32>,
        x: Option<i32>,
        y: Option<i32>,
    ) -> Result<(), String> {
        self.browser_context_by_browser_id_mut(handle.context())
            .ok_or_else(|| "WebContents unavailable".to_owned())?
            .update_web_contents_window_surface(handle, state, width, height, x, y)
    }

    pub(crate) async fn select_browser_web_contents_async(
        &mut self,
        handle: WebContentsHandle,
    ) -> Result<(), String> {
        let browser_globals = self.browser_global_overrides.clone();
        let pending = self
            .browser_context_by_browser_id_mut(handle.context())
            .ok_or_else(|| "WebContents unavailable".to_owned())?
            .start_select_web_contents(handle, &browser_globals)?;
        let completed = pending.wait().await;
        self.browser_context_by_browser_id_mut(handle.context())
            .ok_or_else(|| "WebContents unavailable".to_owned())?
            .finish_select_web_contents(completed)
    }

    pub(crate) async fn apply_browser_page_surface_async(
        &mut self,
        handle: WebContentsHandle,
        foreground: bool,
    ) -> Result<bool, String> {
        let browser_globals = self.browser_global_overrides.clone();
        let pending = {
            let context = self
                .browser_context_by_browser_id_mut(handle.context())
                .ok_or_else(|| "WebContents unavailable".to_owned())?;
            let Some(document) = context.document_handle_for_web_contents(handle)? else {
                return Ok(false);
            };
            context.browser_context.start_document_page_surface_update(
                document,
                foreground,
                browser_globals.network_conditions,
                browser_globals.geolocation.as_ref(),
            )?
        };
        let completed = pending.wait().await;
        self.finish_document_policy_update(completed)?;
        Ok(true)
    }
}
