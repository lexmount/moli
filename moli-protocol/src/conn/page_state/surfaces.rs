use super::super::cookie_manager_surface::BrowserContextCookieManagerSurfaceSnapshot;
use super::super::{
    BrowserContext, CdpConnection, DocumentStartScript, EmulatedGeolocationOverrideState,
    EmulatedNetworkConditions, EmulatedViewportSurface, PageTargetHost,
};
#[cfg(test)]
use moli_cookie_jar::{BrowserCookieFacadeContextOverrides, BrowserCookieFacadeOverrides};

struct SurfaceOverrideInputs {
    network_conditions: Option<EmulatedNetworkConditions>,
    geolocation_override: Option<EmulatedGeolocationOverrideState>,
    max_touch_points: u32,
    navigator_queries: moli_page_types::NavigatorQueryOverrides,
    focus_emulation_enabled: bool,
    active_target_surface: bool,
    window_is_focused: bool,
    window_document_hidden: bool,
}

impl SurfaceOverrideInputs {
    fn from_active(browser_context: &BrowserContext) -> Self {
        Self {
            navigator_queries: browser_context
                .active_page_target()
                .devtools_sessions
                .navigator_emulation
                .effective(),
            network_conditions: browser_context.effective_active_network_conditions(),
            geolocation_override: browser_context.effective_active_geolocation_override(),
            max_touch_points: browser_context
                .active_page_target()
                .effective_emulation_state
                .max_touch_points,
            focus_emulation_enabled: browser_context
                .active_page_target()
                .effective_emulation_state
                .focus_emulation_enabled,
            active_target_surface: true,
            window_is_focused: browser_context.window_is_focused,
            window_document_hidden: browser_context
                .active_page_target()
                .owner_state
                .window_document_hidden(),
        }
    }

    fn from_background(
        state: &PageTargetHost,
        default_network_conditions: Option<EmulatedNetworkConditions>,
        default_geolocation_override: Option<EmulatedGeolocationOverrideState>,
        selected_tab: bool,
    ) -> Self {
        Self {
            network_conditions: state
                .effective_emulation_state
                .network_conditions
                .or(default_network_conditions),
            geolocation_override: state
                .effective_emulation_state
                .geolocation_override
                .clone()
                .or(default_geolocation_override),
            max_touch_points: state.effective_emulation_state.max_touch_points,
            navigator_queries: state.devtools_sessions.navigator_emulation.effective(),
            focus_emulation_enabled: state.effective_emulation_state.focus_emulation_enabled,
            active_target_surface: selected_tab,
            window_is_focused: false,
            window_document_hidden: state.owner_state.window_document_hidden(),
        }
    }

    fn max_touch_points(&self) -> u32 {
        self.max_touch_points
    }

    fn document_has_focus(&self) -> bool {
        self.document_is_focused()
    }

    fn document_hidden(&self) -> bool {
        !self.document_is_visible()
    }

    fn document_visibility_state(&self) -> &'static str {
        if self.document_is_visible() {
            "visible"
        } else {
            "hidden"
        }
    }

    fn document_activity(&self) -> moli_page_types::DocumentActivity {
        moli_page_types::DocumentActivity::new(
            self.document_is_focused(),
            self.document_is_visible(),
        )
    }

    fn navigator_overrides(&self) -> moli_page_types::NavigatorOverrides {
        moli_page_types::NavigatorOverrides {
            online: self
                .network_conditions
                .map(|conditions| conditions.navigator_online()),
            max_touch_points: self.max_touch_points(),
            queries: self.navigator_queries,
            geolocation: self
                .geolocation_override
                .as_ref()
                .and_then(EmulatedGeolocationOverrideState::position)
                .map(|position| moli_page_types::GeolocationPositionOverride {
                    latitude: position.latitude,
                    longitude: position.longitude,
                    accuracy: position.accuracy,
                    altitude: position.altitude,
                    altitude_accuracy: position.altitude_accuracy,
                    heading: position.heading,
                    speed: position.speed,
                }),
        }
    }

    fn document_is_visible(&self) -> bool {
        (self.active_target_surface || self.focus_emulation_enabled) && !self.window_document_hidden
    }

    fn document_is_focused(&self) -> bool {
        // Focus and Page Visibility are target-state surfaces, not raw mirrors
        // of the CDP focus-emulation flag. Chrome's created active targets
        // report focused and visible by default; background targets
        // stay unfocused/hidden unless CDP explicitly asks to simulate a
        // focused and active page.
        (self.active_target_surface && self.window_is_focused && !self.window_document_hidden)
            || self.focus_emulation_enabled
    }
}

impl BrowserContext {
    pub(crate) fn navigator_overrides_for_target(
        &self,
        target_id: &str,
    ) -> Option<moli_page_types::NavigatorOverrides> {
        let target = self.page_target(target_id)?;
        Some(
            SurfaceOverrideInputs::from_background(
                target,
                self.default_network_conditions
                    .or(self.global_network_conditions),
                self.default_geolocation_override
                    .clone()
                    .or_else(|| self.global_geolocation_override.clone()),
                self.page_targets.is_selected_tab(target_id),
            )
            .navigator_overrides(),
        )
    }

    pub(crate) fn active_navigator_overrides(&self) -> moli_page_types::NavigatorOverrides {
        SurfaceOverrideInputs::from_active(self).navigator_overrides()
    }

    pub(crate) fn document_activity_for_target(
        &self,
        target_id: &str,
    ) -> Option<moli_page_types::DocumentActivity> {
        let target = self.page_target(target_id)?;
        Some(
            if self.is_active_target(target_id) {
                SurfaceOverrideInputs::from_active(self)
            } else {
                SurfaceOverrideInputs::from_background(
                    target,
                    self.default_network_conditions
                        .or(self.global_network_conditions),
                    self.default_geolocation_override
                        .clone()
                        .or_else(|| self.global_geolocation_override.clone()),
                    self.page_targets.is_selected_tab(target_id),
                )
            }
            .document_activity(),
        )
    }

    pub(crate) fn active_document_activity(&self) -> moli_page_types::DocumentActivity {
        self.page_targets
            .active()
            .map(|_| SurfaceOverrideInputs::from_active(self).document_activity())
            .unwrap_or_default()
    }
    #[cfg(test)]
    pub(crate) fn start_cookie_manager_policy_mutation_for_test(
        &mut self,
        mutate: impl FnOnce(
            &mut super::super::cookie_manager_surface::BrowserContextCookieManagerSurface,
        ) -> bool,
    ) -> crate::conn::SessionPageUpdates {
        let mut updates = crate::conn::SessionPageUpdates::default();
        let Some(host) = self.page_targets.active_mut() else {
            mutate(&mut self.default_document_cookie_manager_surface);
            return updates;
        };
        if !mutate(&mut host.document_cookie_manager_surface) {
            return updates;
        }
        let overrides = host
            .document_cookie_manager_surface
            .snapshot()
            .policy
            .overrides;
        if let Some(page) = host.runtime_slot.loaded_page_mut() {
            let pending = if overrides == BrowserCookieFacadeOverrides::default() {
                page.start_clear_document_cookie_facade_overrides()
            } else {
                page.start_apply_document_cookie_facade_overrides(&overrides)
            };
            updates.push(page, "fixture cookie manager policy update", pending);
        }
        updates
    }

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

    pub(crate) fn raw_cookie_manager_surface_snapshot(
        &self,
    ) -> BrowserContextCookieManagerSurfaceSnapshot {
        self.page_targets
            .active()
            .map(|host| host.document_cookie_manager_surface.snapshot())
            .unwrap_or_else(|| self.default_document_cookie_manager_surface.snapshot())
    }

    pub fn document_start_script_descriptors(&self) -> Vec<DocumentStartScript> {
        let mut scripts = Vec::new();
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
        target_headers: &moli_fetch::RequestHeaders,
    ) -> moli_fetch::RequestHeaders {
        merge_extra_header_layers(&[
            &self.global_extra_headers,
            &self.default_extra_headers,
            target_headers,
        ])
    }

    pub fn effective_extra_headers(&self) -> moli_fetch::RequestHeaders {
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
    ) -> moli_fetch::RequestHeaders {
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
        (metrics.as_ref()).map_or_else(
            EmulatedViewportSurface::default,
            crate::conn::EmulatedDeviceMetrics::viewport_surface,
        )
    }

    pub fn max_touch_points(&self) -> u32 {
        self.active_page_target()
            .effective_emulation_state
            .max_touch_points
    }

    pub fn document_has_focus(&self) -> bool {
        SurfaceOverrideInputs::from_active(self).document_has_focus()
    }

    pub fn document_hidden(&self) -> bool {
        SurfaceOverrideInputs::from_active(self).document_hidden()
    }

    pub fn document_visibility_state(&self) -> &'static str {
        SurfaceOverrideInputs::from_active(self).document_visibility_state()
    }

    pub(crate) fn start_session_surface_restoration(
        &mut self,
        target_id: &str,
        updates: &mut crate::conn::SessionPageUpdates,
    ) {
        let Some(overrides) = self.navigator_overrides_for_target(target_id) else {
            return;
        };
        let Some(activity) = self.document_activity_for_target(target_id) else {
            return;
        };
        if let Some(page) = self
            .page_target_mut(target_id)
            .and_then(|target| target.runtime_slot.loaded_page_mut())
        {
            updates.push(
                page,
                "failed to restore detached session navigator surface",
                page.start_set_navigator_overrides(&overrides),
            );
            updates.push(
                page,
                "failed to restore detached session document activity",
                page.start_set_document_activity(activity),
            );
        }
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

impl CdpConnection {}

fn merge_extra_header_layers(layers: &[&moli_fetch::RequestHeaders]) -> moli_fetch::RequestHeaders {
    let mut headers = moli_fetch::RequestHeaders::default();
    for layer in layers {
        for (name, value) in *layer {
            headers.retain(|(existing, _)| !existing.eq_ignore_ascii_case(name));
            headers.push((name.clone(), value.clone()));
        }
    }
    headers
}
