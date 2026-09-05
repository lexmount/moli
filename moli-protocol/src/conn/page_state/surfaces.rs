#[cfg(test)]
use super::super::cookie_manager_surface::BrowserContextCookieManagerSurfaceSnapshot;
use super::super::{
    BrowserContext, CdpConnection, DocumentStartScript, EmulatedViewportSurface, PageTargetHost,
};
use crate::conn::state::PageSurface;
#[cfg(test)]
use moli_cookie_jar::{BrowserCookieFacadeContextOverrides, BrowserCookieFacadeOverrides};

impl BrowserContext {
    #[cfg(test)]
    async fn mutate_document_cookie_manager_surface_async(
        &mut self,
        mutate: impl FnOnce(
            &mut super::super::cookie_manager_surface::BrowserContextCookieManagerSurface,
        ) -> bool,
    ) -> bool {
        if let Some(host) = self.page_targets.active_mut() {
            let state = host;
            if !mutate(&mut state.document_cookie_manager_surface) {
                return false;
            }
            let surface = state.document_cookie_manager_surface.clone();
            if let Some(page) = state.runtime_slot.loaded_page_mut() {
                surface.apply_to_page_async(page).await;
            }
        } else if !mutate(&mut self.default_document_cookie_manager_surface) {
            return false;
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn raw_cookie_manager_surface_snapshot(
        &self,
    ) -> BrowserContextCookieManagerSurfaceSnapshot {
        self.page_targets
            .active()
            .map(|host| host.document_cookie_manager_surface.snapshot())
            .unwrap_or_else(|| self.default_document_cookie_manager_surface.snapshot())
    }

    pub fn document_start_script_descriptors(&self) -> Vec<DocumentStartScript> {
        let mut scripts = vec![Self::surface_preload_descriptor(
            self.generated_surface_override_script_for_active_target(),
        )];
        scripts.extend(self.default_document_start_script_descriptors());
        let target_id = self.active_target_id();
        scripts.extend(
            self.active_page_target()
                .owner_state
                .document_start_scripts
                .iter()
                .map(|(identifier, script)| {
                    Self::target_document_start_script_descriptor(target_id, identifier, script)
                }),
        );
        scripts
    }

    pub(crate) fn default_document_start_script_descriptors(&self) -> Vec<DocumentStartScript> {
        self.default_document_start_scripts
            .iter()
            .map(|(identifier, script)| {
                script
                    .with_registry_key(Self::default_document_start_script_registry_key(identifier))
            })
            .collect()
    }

    pub(crate) fn default_document_start_script_registry_key(identifier: &str) -> String {
        format!("default:{identifier}")
    }

    pub(crate) fn target_document_start_script_registry_key(
        target_id: Option<&str>,
        identifier: &str,
    ) -> String {
        match target_id {
            Some(target_id) => format!("target:{target_id}:{identifier}"),
            None => format!("target:{identifier}"),
        }
    }

    pub(crate) fn target_session_document_start_script_registry_key(
        target_id: Option<&str>,
        session_id: &str,
        identifier: &str,
    ) -> String {
        match target_id {
            Some(target_id) => {
                format!("target:{target_id}:session:{session_id}:{identifier}")
            }
            None => format!("target:session:{session_id}:{identifier}"),
        }
    }

    pub(crate) fn target_document_start_script_descriptor(
        target_id: Option<&str>,
        identifier: &str,
        script: &DocumentStartScript,
    ) -> DocumentStartScript {
        if script.registry_key.is_some() {
            return script.clone();
        }
        script.with_registry_key(Self::target_document_start_script_registry_key(
            target_id, identifier,
        ))
    }

    pub(crate) fn has_default_bidi_channel_preload_script(&self) -> bool {
        self.default_document_start_scripts
            .iter()
            .any(|(_, script)| script.has_bidi_channel_argument)
    }

    pub(crate) fn record_default_document_start_script(
        &mut self,
        script: &DocumentStartScript,
    ) -> String {
        let identifier = self.reserve_default_document_start_script_id();
        self.record_default_document_start_script_with_identifier(identifier.clone(), script);
        identifier
    }

    pub(crate) fn reserve_default_document_start_script_id(&mut self) -> String {
        self.next_default_document_start_script_id =
            self.next_default_document_start_script_id.wrapping_add(1);
        self.next_default_document_start_script_id.to_string()
    }

    pub(crate) fn record_default_document_start_script_with_identifier(
        &mut self,
        identifier: String,
        script: &DocumentStartScript,
    ) {
        let script = script.with_registry_key(Self::default_document_start_script_registry_key(
            &identifier,
        ));
        self.default_document_start_scripts
            .push((identifier, script));
    }

    pub(crate) fn remove_default_document_start_script(
        &mut self,
        script_id: &str,
    ) -> Option<String> {
        let index = self
            .default_document_start_scripts
            .iter()
            .position(|(identifier, _)| identifier == script_id)?;
        let (_, script) = self.default_document_start_scripts.remove(index);
        Some(
            script
                .registry_key
                .unwrap_or_else(|| Self::default_document_start_script_registry_key(script_id)),
        )
    }

    pub(crate) fn has_default_document_start_script(&self, script_id: &str) -> bool {
        self.default_document_start_scripts
            .iter()
            .any(|(identifier, _)| identifier == script_id)
    }

    pub(crate) fn merged_extra_headers_for_target_policy(
        &self,
        target_headers: &[(String, String)],
    ) -> Vec<(String, String)> {
        merge_extra_header_layers(&[
            self.global_extra_headers.as_slice(),
            self.default_extra_headers.as_slice(),
            target_headers,
        ])
    }

    pub fn effective_extra_headers(&self) -> Vec<(String, String)> {
        let target_headers = self
            .page_targets
            .active()
            .map(PageTargetHost::effective_policy)
            .unwrap_or_default();
        self.merged_extra_headers_for_target_policy(target_headers.extra_headers())
    }

    pub(crate) fn effective_extra_headers_for_target(
        &self,
        target_id: &str,
    ) -> Vec<(String, String)> {
        let target_headers = self
            .page_target(target_id)
            .map(PageTargetHost::effective_policy)
            .unwrap_or_default();
        self.merged_extra_headers_for_target_policy(target_headers.extra_headers())
    }

    pub fn viewport_width(&self) -> u32 {
        self.viewport_surface().inner_width
    }

    pub fn viewport_height(&self) -> u32 {
        self.viewport_surface().inner_height
    }

    pub fn device_pixel_ratio(&self) -> f64 {
        self.viewport_surface().device_pixel_ratio
    }

    pub fn screen_width(&self) -> u32 {
        self.viewport_surface().screen_width
    }

    pub fn screen_height(&self) -> u32 {
        self.viewport_surface().screen_height
    }

    pub fn screen_avail_width(&self) -> u32 {
        self.viewport_surface().screen_avail_width
    }

    pub fn screen_avail_height(&self) -> u32 {
        self.viewport_surface().screen_avail_height
    }

    pub(crate) fn viewport_surface(&self) -> EmulatedViewportSurface {
        let metrics = self.effective_active_emulated_device_metrics();
        EmulatedViewportSurface::from_metrics(metrics.as_ref())
    }

    pub fn max_touch_points(&self) -> u32 {
        if self
            .active_page_target()
            .emulation_policy()
            .touch_emulation_enabled
        {
            1
        } else {
            0
        }
    }

    pub fn document_has_focus(&self) -> bool {
        self.page_surface_for_state(self.active_page_target(), true)
            .document_has_focus()
    }

    pub fn document_hidden(&self) -> bool {
        self.page_surface_for_state(self.active_page_target(), true)
            .document_hidden()
    }

    pub fn document_visibility_state(&self) -> &'static str {
        self.page_surface_for_state(self.active_page_target(), true)
            .document_visibility_state()
    }

    // Context default resolution stays in this residence until Commit 7;
    // source generation itself only reads the embedded Browser object.
    fn page_surface_for_state(&self, state: &PageTargetHost, foreground: bool) -> PageSurface {
        state.runtime_slot.page_slot().contents.page_surface(
            foreground,
            self.emulation_defaults()
                .network_conditions
                .or(self.global_network_conditions),
            self.emulation_defaults()
                .geolocation
                .as_ref()
                .or(self.global_geolocation_override.as_ref()),
            self.emulation_defaults().device_metrics.as_ref(),
        )
    }

    pub(crate) fn generated_surface_override_script_for_active_target(&self) -> String {
        self.page_surface_for_state(self.active_page_target(), true)
            .script()
    }

    pub(crate) fn generated_surface_override_script_for_background_target(
        &self,
        target_id: &str,
    ) -> Option<String> {
        let target = self.background_target(target_id)?;
        Some(self.generated_surface_override_script_for_background_state(target))
    }

    pub(crate) fn generated_surface_override_script_for_background_state(
        &self,
        state: &PageTargetHost,
    ) -> String {
        self.page_surface_for_state(state, false).script()
    }

    // Navigation's legacy preload carrier is removed at Commits 12/14/20.
    // The Browser generator never receives this descriptor or its session fields.
    pub(in crate::conn) fn surface_preload_descriptor(source: String) -> DocumentStartScript {
        DocumentStartScript {
            registry_key: None,
            devtools_session: None,
            source,
            world_name: None,
            has_bidi_channel_argument: false,
            bidi_channel_handoffs: Vec::new(),
        }
    }

    pub(crate) async fn apply_background_target_surface_overrides_async(
        &mut self,
        target_id: &str,
    ) -> anyhow::Result<bool> {
        let Some(script) = self.generated_surface_override_script_for_background_target(target_id)
        else {
            return Ok(false);
        };
        let Some(page) = self
            .background_target_mut(target_id)
            .and_then(|target| target.runtime_slot.loaded_page_mut())
        else {
            return Ok(false);
        };
        page.run_page_surface_override_script_async(&script)
            .await
            .map_err(|error| anyhow::anyhow!("failed to hide background page surface: {error}"))?;
        Ok(true)
    }

    pub(crate) async fn apply_surface_overrides_to_loaded_page_async(
        &mut self,
    ) -> anyhow::Result<()> {
        let script = self.generated_surface_override_script_for_active_target();
        let Some(page) = self.active_page_target_mut().runtime_slot.loaded_page_mut() else {
            return Ok(());
        };
        page.run_page_surface_override_script_async(&script)
            .await
            .map_err(|error| anyhow::anyhow!("failed to apply page surface overrides: {error}"))
    }

    #[cfg(test)]
    pub(crate) async fn apply_cookie_manager_policy_overrides_async(
        &mut self,
        overrides: &BrowserCookieFacadeOverrides,
    ) {
        self.mutate_document_cookie_manager_surface_async(|surface| {
            surface.set_policy_overrides(overrides)
        })
        .await;
    }

    #[cfg(test)]
    pub(crate) async fn clear_cookie_manager_policy_overrides_async(&mut self) {
        self.mutate_document_cookie_manager_surface_async(|surface| {
            surface.clear_policy_overrides()
        })
        .await;
    }

    #[cfg(test)]
    pub(crate) async fn set_cookie_manager_policy_cookies_enabled_override_async(
        &mut self,
        enabled: bool,
    ) {
        self.mutate_document_cookie_manager_surface_async(|surface| {
            surface.set_policy_cookies_enabled_override(enabled)
        })
        .await;
    }

    #[cfg(test)]
    pub(crate) async fn clear_cookie_manager_policy_cookies_enabled_override_async(&mut self) {
        self.mutate_document_cookie_manager_surface_async(|surface| {
            surface.clear_policy_cookies_enabled_override()
        })
        .await;
    }

    #[cfg(test)]
    pub(crate) async fn set_cookie_manager_policy_browser_context_overrides_async(
        &mut self,
        overrides: &BrowserCookieFacadeContextOverrides,
    ) {
        self.mutate_document_cookie_manager_surface_async(|surface| {
            surface.set_policy_browser_context_overrides(overrides)
        })
        .await;
    }

    #[cfg(test)]
    pub(crate) async fn clear_cookie_manager_policy_browser_context_overrides_async(&mut self) {
        self.mutate_document_cookie_manager_surface_async(|surface| {
            surface.clear_policy_browser_context_overrides()
        })
        .await;
    }
}

impl CdpConnection {
    pub(crate) async fn remove_document_start_scripts_for_detached_session_async(
        &mut self,
        session_id: &str,
    ) -> anyhow::Result<()> {
        let renderer_inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_session(Some(session_id));
        let devtools_session = moli_page_types::DevToolsSessionKey::from_wire_session_id(
            renderer_inspector_session_id.as_deref(),
        );
        let registry_keys = self
            .target_owner_state_for_session(Some(session_id))
            .map(|owner_state| {
                owner_state.document_start_script_registry_keys_for_session(&devtools_session)
            })
            .unwrap_or_default();
        let has_loaded_page = self
            .runtime_session_owner_slot_mut(Some(session_id))
            .ok()
            .is_some_and(|slot| slot.loaded_page().is_some());
        if !has_loaded_page {
            let _ = self.with_target_owner_state_for_session_mut(Some(session_id), |owner_state| {
                owner_state.remove_document_start_scripts_for_session(&devtools_session)
            });
            return Ok(());
        }

        let mut first_error = None;
        for registry_key in registry_keys {
            let result: anyhow::Result<()> = async {
                let pending = self
                    .runtime_session_owner_slot_mut(Some(session_id))
                    .ok()
                    .and_then(|slot| slot.loaded_page_mut())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "page disappeared while removing detached-session document-start scripts"
                        )
                    })?
                    .start_remove_document_start_script_by_registry_key(&registry_key)
                    .map_err(|error| {
                        anyhow::anyhow!(
                            "failed to start detached-session document-start script cleanup: {error}"
                        )
                    })?;
                let completion = pending.wait().await.map_err(|error| {
                    anyhow::anyhow!(
                        "detached-session document-start script cleanup was canceled: {error}"
                    )
                })?;
                let page = self
                    .runtime_session_owner_slot_mut(Some(session_id))
                    .ok()
                    .and_then(|slot| slot.loaded_page_mut())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "page disappeared while finishing detached-session document-start script cleanup"
                        )
                    })?;
                page.finish_unit_runtime_page_command(
                    completion,
                    "remove detached-session document-start script",
                )
                .map_err(|error| {
                    anyhow::anyhow!(
                        "failed to finish detached-session document-start script cleanup: {error}"
                    )
                })?;
                self.with_target_owner_state_for_session_mut(
                    Some(session_id),
                    |owner_state| {
                        owner_state.remove_document_start_script_registry_key_for_session(
                            &devtools_session,
                            &registry_key,
                        )
                    },
                )
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "session owner disappeared during document-start script cleanup"
                    )
                })?;
                Ok(())
            }
            .await;
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
        }

        // Keep unresolved registry keys as cleanup authority. The centralized
        // session disposer will either retry after the renderer disappears or
        // keep the session binding alive; clearing them here would orphan a
        // script that may still execute in a later Document.
        if let Some(error) = first_error {
            return Err(error);
        }

        // Scripts without a renderer registry key never require a renderer
        // round trip. Successful keyed removals have already been committed.
        self.with_target_owner_state_for_session_mut(Some(session_id), |owner_state| {
            owner_state.remove_document_start_scripts_for_session(&devtools_session)
        })
        .ok_or_else(|| {
            anyhow::anyhow!("session owner disappeared during document-start script cleanup")
        })?;
        Ok(())
    }
}

fn merge_extra_header_layers(layers: &[&[(String, String)]]) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    for layer in layers {
        for (name, value) in *layer {
            headers.retain(|(existing, _)| existing != name);
            headers.push((name.clone(), value.clone()));
        }
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::WindowSurfaceState;

    #[test]
    fn background_surface_uses_the_owning_window_state() {
        let mut target = PageTargetHost::empty("TID-background-window".into());
        target
            .apply_emulation_policy_change(crate::conn::EmulationPolicyChange::FocusEnabled(true));
        target.set_window_surface_state(WindowSurfaceState::Minimized);
        let mut contents = std::mem::take(&mut target.runtime_slot.page_slot_mut().contents);
        drop(target);
        let minimized = contents.page_surface(false, None, None, None);
        assert!(minimized.document_has_focus());
        assert!(
            minimized.document_hidden(),
            "focus emulation must not unminimize a window"
        );

        contents.window.surface.state = WindowSurfaceState::Fullscreen;
        let fullscreen = contents.page_surface(false, None, None, None);
        assert!(!fullscreen.document_hidden());
        assert!(fullscreen.window_fullscreen());
    }
}
