//! Browser-owned emulation inputs consumed by native Navigator/Geolocation APIs.
//!
//! These values are state, not document-start JavaScript. Updating or clearing
//! an override must not replace Web IDL descriptors or expose a second object.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavigatorOverrides {
    /// None uses the renderer's actual network state.
    pub online: Option<bool>,
    pub max_touch_points: u32,
    /// None means that no emulated coordinate source is available.
    pub geolocation: Option<GeolocationPositionOverride>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GeolocationPositionOverride {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: f64,
    pub altitude: Option<f64>,
    pub altitude_accuracy: Option<f64>,
    pub heading: Option<f64>,
    pub speed: Option<f64>,
}
