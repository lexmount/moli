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
    pub automation: bool,
}

/// Native query overrides in Inspector agent activation order. Page and Worker
/// sessions use the same precedence, but each target owns its own registry.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct NavigatorEmulationSessions {
    sessions: Vec<(crate::DevToolsSessionKey, NavigatorEmulationSession)>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct NavigatorEmulationSession {
    queries: NavigatorQueryOverrides,
    user_agent: Option<moli_browser_profile::UserAgentOverride>,
}

impl NavigatorEmulationSessions {
    /// The first probe-using Emulation command activates an agent. Subsequent
    /// commands preserve its position, even after clearing an individual value.
    pub fn session_mut(&mut self, key: &crate::DevToolsSessionKey) -> &mut NavigatorQueryOverrides {
        &mut self.activate(key).queries
    }

    fn activate(&mut self, key: &crate::DevToolsSessionKey) -> &mut NavigatorEmulationSession {
        let index = match self
            .sessions
            .iter()
            .position(|(existing, _)| existing == key)
        {
            Some(index) => index,
            None => {
                self.sessions
                    .push((key.clone(), NavigatorEmulationSession::default()));
                self.sessions.len() - 1
            }
        };
        &mut self.sessions[index].1
    }

    pub fn set_automation(&mut self, key: &crate::DevToolsSessionKey, enabled: bool) {
        if enabled {
            self.session_mut(key).automation = true;
        } else if let Some((_, settings)) = self
            .sessions
            .iter_mut()
            .find(|(existing, _)| existing == key)
        {
            // Disabling alone does not activate a new Inspector agent.
            settings.queries.automation = false;
        }
    }

    pub fn set_user_agent_override(
        &mut self,
        key: &crate::DevToolsSessionKey,
        value: moli_browser_profile::UserAgentOverride,
    ) {
        if value.activates_agent() {
            self.activate(key).user_agent = Some(value);
        } else if let Some((_, session)) = self
            .sessions
            .iter_mut()
            .find(|(existing, _)| existing == key)
        {
            session.user_agent = None;
        }
    }

    /// UA (together with its metadata) and language each use the last active
    /// agent contributing that field. Clearing a field does not reorder agents.
    pub fn effective_user_agent_override(&self) -> Option<moli_browser_profile::UserAgentOverride> {
        let mut user_agent = None;
        let mut accept_language = None;
        for (_, session) in &self.sessions {
            let Some(value) = &session.user_agent else {
                continue;
            };
            if !value.user_agent.is_empty() {
                user_agent = Some(value);
            }
            if let Some(language) = value
                .accept_language
                .as_ref()
                .filter(|value| !value.is_empty())
            {
                accept_language = Some(language.clone());
            }
        }
        (user_agent.is_some() || accept_language.is_some()).then(|| {
            moli_browser_profile::UserAgentOverride {
                user_agent: user_agent
                    .map(|value| value.user_agent.clone())
                    .unwrap_or_default(),
                accept_language,
                // Worker platform is fixed at creation. The parameter participates
                // in activation, but Chromium applies it only to Page settings.
                platform: None,
                user_agent_metadata: user_agent.and_then(|value| value.user_agent_metadata.clone()),
            }
        })
    }

    pub fn remove(&mut self, key: &crate::DevToolsSessionKey) {
        self.sessions.retain(|(existing, _)| existing != key);
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn effective(&self) -> NavigatorQueryOverrides {
        let mut effective = NavigatorQueryOverrides::default();
        for (_, session) in &self.sessions {
            let settings = &session.queries;
            effective.automation |= settings.automation;
            effective.data_saver = settings.data_saver.or(effective.data_saver);
            effective.hardware_concurrency = settings
                .hardware_concurrency
                .or(effective.hardware_concurrency);
        }
        effective
    }
}
