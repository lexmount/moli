use super::WebContents;
use crate::browser::{
    EmulatedDeviceMetrics, EmulatedGeolocationOverrideState, EmulatedNetworkConditions,
};
#[cfg(test)]
use serde_json::json;

/// A derived Browser value, never stored alongside the installed policy.
pub struct PageSurface {
    network_conditions: Option<EmulatedNetworkConditions>,
    geolocation_override: Option<EmulatedGeolocationOverrideState>,
    max_touch_points: u32,
    pub navigator_queries: moli_page_types::NavigatorQueryOverrides,
    focus_emulation_enabled: bool,
    foreground: bool,
    window_document_hidden: bool,
}

impl WebContents {
    pub fn page_surface(
        &self,
        foreground: bool,
        default_network_conditions: Option<EmulatedNetworkConditions>,
        default_geolocation_override: Option<&EmulatedGeolocationOverrideState>,
        _default_emulated_device_metrics: Option<&EmulatedDeviceMetrics>,
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
            max_touch_points: self.emulation_policy.max_touch_points,
            navigator_queries: Default::default(),
            focus_emulation_enabled: self.emulation_policy.focus_emulation_enabled,
            foreground,
            window_document_hidden: self.window.surface.state.document_hidden(),
        }
    }
}

impl PageSurface {
    fn max_touch_points(&self) -> u32 {
        self.max_touch_points
    }

    pub fn document_has_focus(&self) -> bool {
        self.document_is_focused()
    }

    pub fn document_hidden(&self) -> bool {
        !self.document_is_visible()
    }

    pub fn document_visibility_state(&self) -> &'static str {
        if self.document_is_visible() {
            "visible"
        } else {
            "hidden"
        }
    }

    pub fn navigator_overrides(&self) -> moli_page_types::NavigatorOverrides {
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
        self.document_is_focused() && !self.window_document_hidden
    }

    fn document_is_focused(&self) -> bool {
        // A foreground window is focused by default. Focus emulation can
        // focus a background window, but cannot make a minimized window visible.
        (self.foreground && !self.window_document_hidden) || self.focus_emulation_enabled
    }

    pub fn document_activity(&self) -> moli_page_types::DocumentActivity {
        moli_page_types::DocumentActivity::new(
            self.document_is_focused(),
            self.document_is_visible(),
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
    async fn browser_page_surface_runs_without_devtools_registration() {
        use crate::runtime::{Browser, BrowserConfig};
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
        page.set_document_activity_async(surface.document_activity())
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
            .apply(EmulationPolicyChange::MaxTouchPoints(1));
        contents.window.surface.state = WindowSurfaceState::Fullscreen;
        let surface = contents.page_surface(false, None, None, None);
        page.set_navigator_overrides_async(&surface.navigator_overrides())
            .await
            .unwrap();
        page.set_document_activity_async(surface.document_activity())
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
