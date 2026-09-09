use super::JsContextHost;
use moli_page_types::NavigatorOverrides;

impl JsContextHost {
    pub(crate) fn navigator_overrides(&self) -> &NavigatorOverrides {
        &self.navigator_overrides
    }

    pub(crate) fn navigator_online(&self) -> bool {
        self.navigator_overrides
            .online
            .unwrap_or(!self.network_offline)
    }

    pub(crate) fn set_navigator_overrides(&mut self, overrides: &NavigatorOverrides) -> bool {
        let geolocation_changed = self.navigator_overrides.geolocation != overrides.geolocation;
        self.navigator_overrides = overrides.clone();
        geolocation_changed
    }

    pub(crate) fn init_window_touch_feature_detection(
        &mut self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> Option<bool> {
        let max_touch_points = self
            .navigator_overrides
            .max_touch_points
            .map(f64::from)
            .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.max_touch_points);
        self.frame_owner_store
            .init_window_touch_feature_detection(owner, max_touch_points > 0.0)
    }

    pub(crate) fn register_geolocation_object<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        geolocation: v8::Local<'s, v8::Object>,
    ) {
        self.geolocation_objects
            .retain(|object| object.to_local(scope).is_some());
        self.geolocation_objects
            .push(v8::Weak::new(scope, geolocation));
    }

    pub(crate) fn live_geolocation_objects<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Vec<v8::Local<'s, v8::Object>> {
        let mut live = Vec::new();
        self.geolocation_objects.retain(|object| {
            if let Some(object) = object.to_local(scope) {
                live.push(object);
                true
            } else {
                false
            }
        });
        live
    }
}
