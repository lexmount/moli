//! Browser-owned emulation inputs consumed by native Navigator/Geolocation APIs.
//!
//! These values are state, not document-start JavaScript. Updating or clearing
//! an override must not replace Web IDL descriptors or expose a second object.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavigatorOverrides {
    /// None uses the renderer's actual network state.
    pub online: Option<bool>,
    pub max_touch_points: u32,
    pub queries: NavigatorQueryOverrides,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavigatorQueryOverrides {
    pub hardware_concurrency: Option<std::num::NonZeroU32>,
    pub data_saver: Option<bool>,
}

/// Native query overrides in Inspector agent activation order. Page and Worker
/// sessions use the same precedence, but each target owns its own registry.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct NavigatorEmulationSessions {
    sessions: Vec<(crate::DevToolsSessionKey, NavigatorQueryOverrides)>,
}

impl NavigatorEmulationSessions {
    /// The first probe-using Emulation command activates an agent. Subsequent
    /// commands preserve its position, even after clearing an individual value.
    pub fn session_mut(&mut self, key: &crate::DevToolsSessionKey) -> &mut NavigatorQueryOverrides {
        let index = match self
            .sessions
            .iter()
            .position(|(existing, _)| existing == key)
        {
            Some(index) => index,
            None => {
                self.sessions
                    .push((key.clone(), NavigatorQueryOverrides::default()));
                self.sessions.len() - 1
            }
        };
        &mut self.sessions[index].1
    }

    pub fn remove(&mut self, key: &crate::DevToolsSessionKey) {
        self.sessions.retain(|(existing, _)| existing != key);
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn effective(&self) -> NavigatorQueryOverrides {
        let mut effective = NavigatorQueryOverrides::default();
        for (_, settings) in &self.sessions {
            effective.data_saver = settings.data_saver.or(effective.data_saver);
            effective.hardware_concurrency = settings
                .hardware_concurrency
                .or(effective.hardware_concurrency);
        }
        effective
    }
}
