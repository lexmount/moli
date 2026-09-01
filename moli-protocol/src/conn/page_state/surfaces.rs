use super::super::cookie_manager_surface::BrowserContextCookieManagerSurfaceSnapshot;
use super::super::{
    BrowserContext, CdpConnection, DocumentStartScript, EmulatedDeviceMetrics,
    EmulatedGeolocationOverrideState, EmulatedNetworkConditions, EmulatedViewportSurface,
    PageTargetHost, viewport_surface_install_script,
};
#[cfg(test)]
use moli_cookie_jar::{BrowserCookieFacadeContextOverrides, BrowserCookieFacadeOverrides};
use serde_json::json;

struct SurfaceOverrideInputs {
    network_conditions: Option<EmulatedNetworkConditions>,
    geolocation_override: Option<EmulatedGeolocationOverrideState>,
    emulated_device_metrics: Option<EmulatedDeviceMetrics>,
    touch_emulation_enabled: bool,
    focus_emulation_enabled: bool,
    active_target_surface: bool,
    window_document_hidden: bool,
    window_fullscreen: bool,
}

impl SurfaceOverrideInputs {
    fn from_active(browser_context: &BrowserContext) -> Self {
        Self {
            network_conditions: browser_context.effective_active_network_conditions(),
            geolocation_override: browser_context.effective_active_geolocation_override(),
            emulated_device_metrics: browser_context.effective_active_emulated_device_metrics(),
            touch_emulation_enabled: browser_context.active_page_target().touch_emulation_enabled,
            focus_emulation_enabled: browser_context.active_page_target().focus_emulation_enabled,
            active_target_surface: true,
            window_document_hidden: browser_context
                .active_page_target()
                .owner_state
                .window_document_hidden(),
            window_fullscreen: browser_context
                .active_page_target()
                .owner_state
                .window_fullscreen(),
        }
    }

    fn from_parked(
        state: &PageTargetHost,
        default_network_conditions: Option<EmulatedNetworkConditions>,
        default_geolocation_override: Option<EmulatedGeolocationOverrideState>,
        default_emulated_device_metrics: Option<EmulatedDeviceMetrics>,
    ) -> Self {
        Self {
            network_conditions: state.network_conditions.or(default_network_conditions),
            geolocation_override: state
                .geolocation_override
                .clone()
                .or(default_geolocation_override),
            emulated_device_metrics: state
                .emulated_device_metrics
                .clone()
                .or(default_emulated_device_metrics),
            touch_emulation_enabled: state.touch_emulation_enabled,
            focus_emulation_enabled: state.focus_emulation_enabled,
            active_target_surface: false,
            window_document_hidden: false,
            window_fullscreen: false,
        }
    }

    fn max_touch_points(&self) -> u32 {
        if self.touch_emulation_enabled { 1 } else { 0 }
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

    fn window_fullscreen(&self) -> bool {
        self.window_fullscreen
    }

    fn navigator_online(&self) -> bool {
        self.network_conditions
            .is_none_or(|conditions| conditions.navigator_online())
    }

    fn document_is_visible(&self) -> bool {
        self.document_is_focused() && !self.window_document_hidden
    }

    fn document_is_focused(&self) -> bool {
        // Focus and Page Visibility are target-state surfaces, not raw mirrors
        // of the CDP focus-emulation flag. Chrome's created active targets
        // report focused and visible by default; parked/background targets
        // stay unfocused/hidden unless CDP explicitly asks to simulate a
        // focused and active page.
        (self.active_target_surface && !self.window_document_hidden) || self.focus_emulation_enabled
    }
}

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
        if let Some(script) = self.generated_surface_override_script() {
            scripts.push(script);
        }
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
        if self.active_page_target().touch_emulation_enabled {
            1
        } else {
            0
        }
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

    fn generated_surface_override_script(&self) -> Option<DocumentStartScript> {
        Self::generated_surface_override_script_from_inputs(&SurfaceOverrideInputs::from_active(
            self,
        ))
    }

    pub(crate) fn generated_surface_override_script_for_parked_target(
        &self,
        target_id: &str,
    ) -> Option<DocumentStartScript> {
        let target = self.background_target(target_id)?;
        self.generated_surface_override_script_for_parked_state(target)
    }

    pub(crate) fn generated_surface_override_script_for_parked_state(
        &self,
        state: &PageTargetHost,
    ) -> Option<DocumentStartScript> {
        Self::generated_surface_override_script_from_inputs(&SurfaceOverrideInputs::from_parked(
            state,
            self.default_network_conditions
                .or(self.global_network_conditions),
            self.default_geolocation_override
                .clone()
                .or_else(|| self.global_geolocation_override.clone()),
            self.default_emulated_device_metrics.clone(),
        ))
    }

    pub(crate) async fn apply_parked_target_surface_overrides_async(
        &mut self,
        target_id: &str,
    ) -> Result<bool, String> {
        let Some(script) = self.generated_surface_override_script_for_parked_target(target_id)
        else {
            return Ok(false);
        };
        let Some(page) = self
            .background_target_mut(target_id)
            .and_then(|target| target.runtime_slot.loaded_page_mut())
        else {
            return Ok(false);
        };
        page.run_page_surface_override_script_async(&script.source)
            .await
            .map_err(|error| format!("failed to hide demoted page surface: {error}"))?;
        Ok(true)
    }

    pub(crate) fn generated_surface_override_script_for_active_target(
        &self,
    ) -> Option<DocumentStartScript> {
        self.generated_surface_override_script()
    }

    fn generated_surface_override_script_from_inputs(
        inputs: &SurfaceOverrideInputs,
    ) -> Option<DocumentStartScript> {
        let geolocation_override = inputs.geolocation_override.as_ref();
        let navigator_online = inputs.navigator_online();
        // Preserve the renderer's native Window/Screen descriptors unless a
        // client explicitly enabled device emulation. Installing the default
        // profile as JS getters makes otherwise native attributes observable
        // as closure-backed properties and can mask child-frame dimensions.
        // An explicit override retains the original descriptors so a later
        // CDP clear can restore the native WebIDL surface.
        let viewport_surface_script = inputs
            .emulated_device_metrics
            .as_ref()
            .map(|metrics| viewport_surface_install_script(&metrics.viewport_surface(), true))
            .unwrap_or_default();
        let max_touch_points = inputs.max_touch_points();
        let document_has_focus = inputs.document_has_focus();
        let document_hidden = inputs.document_hidden();
        let document_visibility_state = inputs.document_visibility_state();
        let window_fullscreen = inputs.window_fullscreen();

        let source = format!(
            "(function() {{
                const defineGetter = (obj, key, getter) => {{
                    if (!obj) return;
                    try {{
                        Object.defineProperty(obj, key, {{ configurable: true, get: getter }});
                    }} catch (_error) {{}}
                }};
                const geolocationOverride = {geolocation_override};
                const navigatorOnline = {navigator_online};
                const maxTouchPoints = {max_touch_points};
                {viewport_surface_script}
                try {{
                    globalThis.__moliNavigatorOnline = navigatorOnline;
                }} catch (_error) {{}}
                const currentNavigatorOnline = () => {{
                    try {{
                        return globalThis.__moliNavigatorOnline !== false;
                    }} catch (_error) {{
                        return navigatorOnline;
                    }}
                }};
                defineGetter(globalThis, 'fullScreen', () => {window_fullscreen});
                try {{
                    const geoState = globalThis.__moliGeolocationState || {{
                        nextWatchId: 1,
                        watchers: new Map(),
                        object: null
                    }};
                    globalThis.__moliGeolocationState = geoState;
                    const previousOverrideKey = geoState.overrideKey || null;
                    geoState.override = geolocationOverride && typeof geolocationOverride === 'object'
                        ? geolocationOverride
                        : null;
                    geoState.overrideKey = JSON.stringify(geoState.override);
                    if (!(geoState.watchers instanceof Map)) {{
                        geoState.watchers = new Map();
                    }}
                    const queue = typeof queueMicrotask === 'function'
                        ? queueMicrotask
                        : (callback) => Promise.resolve().then(callback);
                    const makeError = (code, message) => {{
                        const error = {{ code, message }};
                        try {{
                            Object.defineProperty(error, 'PERMISSION_DENIED', {{ value: 1 }});
                            Object.defineProperty(error, 'POSITION_UNAVAILABLE', {{ value: 2 }});
                            Object.defineProperty(error, 'TIMEOUT', {{ value: 3 }});
                        }} catch (_error) {{}}
                        return error;
                    }};
                    const makePosition = () => {{
                        const override = geoState.override;
                        return {{
                            coords: {{
                                latitude: override.latitude,
                                longitude: override.longitude,
                                accuracy: override.accuracy,
                                altitude: override.altitude ?? null,
                                altitudeAccuracy: override.altitudeAccuracy ?? null,
                                heading: override.heading ?? null,
                                speed: override.speed ?? null
                            }},
                            timestamp: Date.now()
                        }};
                    }};
                    const deliverGeolocation = (success, error) => {{
                        queue(() => {{
                            const fail = (code, message) => {{
                                if (typeof error === 'function') {{
                                    error.call(geoState.object, makeError(code, message));
                                }}
                            }};
                            const succeed = () => {{
                                if (typeof success === 'function') {{
                                    success.call(geoState.object, makePosition());
                                }}
                            }};
                            const finish = () => {{
                                if (!geoState.override) {{
                                    fail(2, 'Position unavailable');
                                }} else {{
                                    succeed();
                                }}
                            }};
                            try {{
                                const permissions = navigator && navigator.permissions;
                                if (permissions && typeof permissions.query === 'function') {{
                                    const queried = permissions.query({{ name: 'geolocation' }});
                                    if (queried && typeof queried.then === 'function') {{
                                        queried.then((status) => {{
                                            if (status && status.state === 'denied') {{
                                                fail(1, 'User denied Geolocation');
                                            }} else {{
                                                finish();
                                            }}
                                        }}, finish);
                                        return;
                                    }}
                                }}
                            }} catch (_error) {{}}
                            finish();
                        }});
                    }};
                    if (!geoState.object) {{
                        geoState.object = {{
                            getCurrentPosition(success, error, _options) {{
                                deliverGeolocation(success, error);
                            }},
                            watchPosition(success, error, _options) {{
                                const id = geoState.nextWatchId++;
                                geoState.watchers.set(id, {{ success, error }});
                                deliverGeolocation(success, error);
                                return id;
                            }},
                            clearWatch(id) {{
                                geoState.watchers.delete(id);
                            }}
                        }};
                    }}
                    if (previousOverrideKey !== null && previousOverrideKey !== geoState.overrideKey) {{
                        for (const watcher of geoState.watchers.values()) {{
                            deliverGeolocation(watcher.success, watcher.error);
                        }}
                    }}
                    defineGetter(navigator, 'geolocation', () => geoState.object);
                }} catch (_error) {{}}
                defineGetter(navigator, 'onLine', () => currentNavigatorOnline());
                defineGetter(navigator, 'maxTouchPoints', () => maxTouchPoints);
                if (document) {{
                    // The renderer's Document bridge currently installs these
                    // surfaces as own accessors, so CDP emulation must shadow
                    // the document object directly for staged/background
                    // overrides to win in the same realm.
                    defineGetter(document, 'hidden', () => {document_hidden});
                    defineGetter(document, 'visibilityState', () => {document_visibility_state});
                    defineGetter(document, 'webkitIsFullScreen', () => {window_fullscreen});
                    try {{
                        Object.defineProperty(document, 'hasFocus', {{
                            configurable: true,
                            value: () => {document_has_focus}
                        }});
                    }} catch (_error) {{}}
                }}
            }})();",
            geolocation_override = geolocation_override
                .and_then(EmulatedGeolocationOverrideState::position)
                .map(|position| {
                    json!({
                        "latitude": position.latitude,
                        "longitude": position.longitude,
                        "accuracy": position.accuracy,
                        "altitude": position.altitude,
                        "altitudeAccuracy": position.altitude_accuracy,
                        "heading": position.heading,
                        "speed": position.speed,
                    })
                    .to_string()
                })
                .unwrap_or_else(|| "null".to_owned()),
            viewport_surface_script = viewport_surface_script,
            max_touch_points = max_touch_points,
            document_hidden = document_hidden,
            document_visibility_state = json!(document_visibility_state),
            document_has_focus = document_has_focus,
            window_fullscreen = window_fullscreen,
        );

        Some(DocumentStartScript {
            registry_key: None,
            devtools_session: None,
            source,
            world_name: None,
            has_bidi_channel_argument: false,
            bidi_channel_handoffs: Vec::new(),
        })
    }

    pub(crate) async fn apply_surface_overrides_to_loaded_page_async(
        &mut self,
    ) -> Result<(), String> {
        let Some(script) = self.generated_surface_override_script() else {
            return Ok(());
        };
        let Some(page) = self.active_page_target_mut().runtime_slot.loaded_page_mut() else {
            return Ok(());
        };
        page.run_page_surface_override_script_async(&script.source)
            .await
            .map_err(|error| format!("failed to apply page surface overrides: {error}"))
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
    pub(crate) async fn remove_document_start_scripts_for_detached_session_best_effort_async(
        &mut self,
        session_id: &str,
    ) {
        let renderer_inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_session(Some(session_id));
        let devtools_session = moli_page_types::DevToolsSessionKey::from_wire_session_id(
            renderer_inspector_session_id.as_deref(),
        );
        let registry_keys = self
            .with_target_owner_state_for_session_mut(Some(session_id), |owner_state| {
                owner_state.take_document_start_script_registry_keys_for_session(&devtools_session)
            })
            .unwrap_or_default();
        for registry_key in registry_keys {
            let pending = self
                .runtime_session_owner_slot_mut(Some(session_id))
                .ok()
                .and_then(|slot| slot.loaded_page_mut())
                .map(|page| page.start_remove_document_start_script_by_registry_key(&registry_key));
            let Some(pending) = pending else {
                continue;
            };
            let pending = match pending {
                Ok(pending) => pending,
                Err(error) => {
                    tracing::debug!(
                        %error,
                        session_id,
                        registry_key,
                        "failed to start detached-session document-start script cleanup"
                    );
                    continue;
                }
            };
            let completion = match pending.wait().await {
                Ok(completion) => completion,
                Err(error) => {
                    tracing::debug!(
                        %error,
                        session_id,
                        registry_key,
                        "detached-session document-start script cleanup was canceled"
                    );
                    continue;
                }
            };
            let Some(page) = self
                .runtime_session_owner_slot_mut(Some(session_id))
                .ok()
                .and_then(|slot| slot.loaded_page_mut())
            else {
                continue;
            };
            if let Err(error) = page.finish_unit_runtime_page_command(
                completion,
                "remove detached-session document-start script",
            ) {
                tracing::debug!(
                    %error,
                    session_id,
                    registry_key,
                    "failed to finish detached-session document-start script cleanup"
                );
            }
        }
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
