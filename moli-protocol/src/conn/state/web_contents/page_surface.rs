use super::super::emulation::{
    EmulatedDeviceMetrics, EmulatedGeolocationOverrideState, EmulatedNetworkConditions,
    viewport_surface_install_script,
};
use super::WebContents;
use serde_json::json;

pub(crate) const LIVE_DEVICE_METRICS_CLEAR_SCRIPT: &str = r#"
(() => {
  const storeKey = '__moliDeviceMetricsOriginalDescriptors';
  const descriptors = globalThis[storeKey] || {};
  let isTopLevelWindow = true;
  try {
    isTopLevelWindow = globalThis.parent === globalThis;
  } catch (_) {
  }
  const restoreDescriptor = (scope, object, property) => {
    if (!object) {
      return;
    }
    const key = `${scope}.${property}`;
    try {
      if (Object.prototype.hasOwnProperty.call(descriptors, key)) {
        const original = descriptors[key];
        if (original && original.existed && original.descriptor) {
          Object.defineProperty(object, property, original.descriptor);
        } else {
          delete object[property];
        }
      } else {
        delete object[property];
      }
    } catch (_) {
    }
  };
  const windowProperties = ['outerWidth', 'outerHeight', 'devicePixelRatio'];
  if (isTopLevelWindow) {
    windowProperties.unshift('innerWidth', 'innerHeight');
  }
  for (const property of windowProperties) {
    restoreDescriptor('window', globalThis, property);
  }
  for (const property of ['width', 'height', 'availWidth', 'availHeight']) {
    restoreDescriptor('screen', globalThis.screen, property);
  }
  try {
    delete globalThis[storeKey];
  } catch (_) {
  }
})()
"#;

/// A derived Browser value, never stored alongside the installed policy.
pub(in crate::conn) struct PageSurface {
    network_conditions: Option<EmulatedNetworkConditions>,
    geolocation_override: Option<EmulatedGeolocationOverrideState>,
    emulated_device_metrics: Option<EmulatedDeviceMetrics>,
    touch_emulation_enabled: bool,
    focus_emulation_enabled: bool,
    foreground: bool,
    window_document_hidden: bool,
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

    pub(in crate::conn) fn navigator_overrides(&self) -> moli_page_types::NavigatorOverrides {
        moli_page_types::NavigatorOverrides {
            online: self
                .network_conditions
                .map(|conditions| conditions.navigator_online()),
            max_touch_points: self.max_touch_points(),
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
        self.document_is_focused() && !self.window_document_hidden
    }

    fn document_is_focused(&self) -> bool {
        // A foreground window is focused by default. Focus emulation can
        // focus a background window, but cannot make a minimized window visible.
        (self.foreground && !self.window_document_hidden) || self.focus_emulation_enabled
    }

    pub(in crate::conn) fn script(&self) -> String {
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
        let document_has_focus = self.document_has_focus();
        let document_hidden = self.document_hidden();
        let document_visibility_state = self.document_visibility_state();

        format!(
            "(function() {{
                const defineGetter = (obj, key, getter) => {{
                    if (!obj) return;
                    try {{
                        Object.defineProperty(obj, key, {{ configurable: true, get: getter }});
                    }} catch (_error) {{}}
                }};
                {viewport_surface_script}
                if (document) {{
                    // The renderer's Document bridge currently installs these
                    // surfaces as own accessors, so surface updates must shadow
                    // the document object directly for staged/background
                    // overrides to win in the same realm.
                    defineGetter(document, 'hidden', () => {document_hidden});
                    defineGetter(document, 'visibilityState', () => {document_visibility_state});
                    try {{
                        Object.defineProperty(document, 'hasFocus', {{
                            configurable: true,
                            value: () => {document_has_focus}
                        }});
                    }} catch (_error) {{}}
                }}
            }})();",
            viewport_surface_script = viewport_surface_script,
            document_hidden = document_hidden,
            document_visibility_state = json!(document_visibility_state),
            document_has_focus = document_has_focus,
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
        assert_eq!(defaults.navigator_overrides().online, Some(false));
        assert!(defaults.geolocation_override.is_some());
        assert!(contents.emulation_policy.network_conditions.is_none());
        assert!(contents.emulation_policy.geolocation_override.is_none());

        contents.window.surface.state = WindowSurfaceState::Minimized;
        let current = contents.page_surface(true, None, None, None);
        assert_eq!(current.navigator_overrides().online, None);
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
        let surface = contents.page_surface(true, None, None, None);
        page.set_navigator_overrides_async(&surface.navigator_overrides())
            .await
            .unwrap();
        page.run_page_surface_override_script_async(&surface.script())
            .await
            .unwrap();
        let expression = "JSON.stringify([navigator.onLine, navigator.maxTouchPoints, document.hasFocus(), document.hidden, 'fullScreen' in window, 'webkitIsFullScreen' in document, Object.prototype.hasOwnProperty.call(globalThis, '__moliDeviceMetricsOriginalDescriptors')])";
        assert_eq!(
            page.evaluate_runtime_expression_async(expression)
                .await
                .unwrap(),
            json!({"type": "string", "value": "[true,0,true,false,false,false,false]"})
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
        let surface = contents.page_surface(false, None, None, None);
        page.set_navigator_overrides_async(&surface.navigator_overrides())
            .await
            .unwrap();
        page.run_page_surface_override_script_async(&surface.script())
            .await
            .unwrap();
        assert_eq!(
            page.evaluate_runtime_expression_async(expression)
                .await
                .unwrap(),
            json!({"type": "string", "value": "[false,1,false,true,false,false,false]"})
        );
        page.close_async().await.unwrap();
    }
}
