use super::super::emulation::{
    EmulatedDeviceMetrics, EmulatedGeolocationOverrideState, EmulatedNetworkConditions,
    viewport_surface_install_script,
};
use super::WebContents;
use serde_json::json;

/// A derived Browser value, never stored alongside the installed policy.
pub(in crate::conn) struct PageSurface {
    network_conditions: Option<EmulatedNetworkConditions>,
    geolocation_override: Option<EmulatedGeolocationOverrideState>,
    emulated_device_metrics: Option<EmulatedDeviceMetrics>,
    touch_emulation_enabled: bool,
    focus_emulation_enabled: bool,
    foreground: bool,
    window_document_hidden: bool,
    window_fullscreen: bool,
}

impl WebContents {
    pub(in crate::conn) fn page_surface(
        &self,
        foreground: bool,
        default_network_conditions: Option<EmulatedNetworkConditions>,
        default_geolocation_override: Option<&EmulatedGeolocationOverrideState>,
        default_emulated_device_metrics: Option<&EmulatedDeviceMetrics>,
    ) -> PageSurface {
        PageSurface {
            network_conditions: self
                .emulation_policy
                .network_conditions
                .or(default_network_conditions),
            geolocation_override: self
                .emulation_policy
                .geolocation_override
                .as_ref()
                .or(default_geolocation_override)
                .cloned(),
            emulated_device_metrics: self
                .emulation_policy
                .emulated_device_metrics
                .as_ref()
                .or(default_emulated_device_metrics)
                .cloned(),
            touch_emulation_enabled: self.emulation_policy.touch_emulation_enabled,
            focus_emulation_enabled: self.emulation_policy.focus_emulation_enabled,
            foreground,
            window_document_hidden: self.window.surface.state.document_hidden(),
            window_fullscreen: self.window.surface.state.is_fullscreen(),
        }
    }
}

impl PageSurface {
    fn max_touch_points(&self) -> u32 {
        if self.touch_emulation_enabled { 1 } else { 0 }
    }

    pub(in crate::conn) fn document_has_focus(&self) -> bool {
        self.document_is_focused()
    }

    pub(in crate::conn) fn document_hidden(&self) -> bool {
        !self.document_is_visible()
    }

    pub(in crate::conn) fn document_visibility_state(&self) -> &'static str {
        if self.document_is_visible() {
            "visible"
        } else {
            "hidden"
        }
    }

    pub(in crate::conn) fn window_fullscreen(&self) -> bool {
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
        // A foreground window is focused by default. Focus emulation can
        // focus a background window, but cannot make a minimized window visible.
        (self.foreground && !self.window_document_hidden) || self.focus_emulation_enabled
    }

    pub(in crate::conn) fn script(&self) -> String {
        let geolocation_override = self.geolocation_override.as_ref();
        let navigator_online = self.navigator_online();
        // Preserve the renderer's native Window/Screen descriptors unless a
        // client explicitly enabled device emulation. Installing the default
        // profile as JS getters makes otherwise native attributes observable
        // as closure-backed properties and can mask child-frame dimensions.
        // An explicit override retains the original descriptors so clearing
        // it can restore the native WebIDL surface.
        let viewport_surface_script = self
            .emulated_device_metrics
            .as_ref()
            .map(|metrics| viewport_surface_install_script(&metrics.viewport_surface(), true))
            .unwrap_or_default();
        let max_touch_points = self.max_touch_points();
        let document_has_focus = self.document_has_focus();
        let document_hidden = self.document_hidden();
        let document_visibility_state = self.document_visibility_state();
        let window_fullscreen = self.window_fullscreen();

        format!(
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
                    // surfaces as own accessors, so surface updates must shadow
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
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::{EmulationPolicyChange, WindowSurfaceState};
    use super::*;

    #[test]
    fn page_surface_resolves_defaults_without_installing_them_in_browser_policy() {
        let mut contents = WebContents::default();
        let defaults = contents.page_surface(
            true,
            Some(EmulatedNetworkConditions::offline()),
            Some(&EmulatedGeolocationOverrideState::PositionUnavailable),
            None,
        );
        assert!(!defaults.navigator_online());
        assert!(defaults.geolocation_override.is_some());
        assert!(contents.emulation_policy.network_conditions.is_none());
        assert!(contents.emulation_policy.geolocation_override.is_none());

        contents.window.surface.state = WindowSurfaceState::Minimized;
        let current = contents.page_surface(true, None, None, None);
        assert!(current.navigator_online());
        assert!(current.geolocation_override.is_none());
        assert!(current.document_hidden());
        assert!(
            !defaults.document_hidden(),
            "snapshots must not change retroactively"
        );
    }

    #[tokio::test]
    async fn browser_generated_page_surface_runs_without_devtools_registration() {
        use moli_core::runtime::{Browser, BrowserConfig};
        let browser = Browser::new(BrowserConfig::default()).unwrap();
        let mut page = browser
            .fetch("data:text/html,<p>page surface</p>")
            .await
            .unwrap();
        let mut contents = WebContents::default();
        let source = contents.page_surface(true, None, None, None).script();
        page.run_page_surface_override_script_async(&source)
            .await
            .unwrap();
        let expression = "JSON.stringify([navigator.onLine, navigator.maxTouchPoints, document.hasFocus(), document.hidden, fullScreen, Object.prototype.hasOwnProperty.call(globalThis, '__moliDeviceMetricsOriginalDescriptors')])";
        assert_eq!(
            page.evaluate_runtime_expression_async(expression)
                .await
                .unwrap(),
            json!({"type": "string", "value": "[true,0,true,false,false,false]"})
        );

        contents
            .emulation_policy
            .apply(EmulationPolicyChange::NetworkConditions(Some(
                EmulatedNetworkConditions::offline(),
            )));
        contents
            .emulation_policy
            .apply(EmulationPolicyChange::TouchEnabled(true));
        contents.window.surface.state = WindowSurfaceState::Fullscreen;
        let source = contents.page_surface(false, None, None, None).script();
        page.run_page_surface_override_script_async(&source)
            .await
            .unwrap();
        assert_eq!(
            page.evaluate_runtime_expression_async(expression)
                .await
                .unwrap(),
            json!({"type": "string", "value": "[false,1,false,true,true,false]"})
        );
        page.close_async().await.unwrap();
    }
}
