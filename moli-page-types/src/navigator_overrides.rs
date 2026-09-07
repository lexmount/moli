//! Browser-owned emulation inputs consumed by native Navigator/Geolocation APIs.
//!
//! These values are state, not document-start JavaScript. Updating or clearing
//! an override must not replace Web IDL descriptors or expose a second object.
//! Every `None` removes an override and restores the corresponding native source.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavigatorOverrides {
    /// `None` uses the renderer's actual network state.
    pub online: Option<bool>,
    /// `None` uses the native Navigator profile; `Some(0)` explicitly reports zero.
    pub max_touch_points: Option<u32>,
    /// `None` restores native positioning, unlike an emulated unavailable position.
    pub geolocation: Option<GeolocationOverride>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GeolocationOverride {
    Position(GeolocationPositionOverride),
    PositionUnavailable,
}

impl GeolocationOverride {
    pub fn position(&self) -> Option<&GeolocationPositionOverride> {
        match self {
            Self::Position(position) => Some(position),
            Self::PositionUnavailable => None,
        }
    }
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
