use super::target_session_owner::{TargetSessionOwnerMut, TargetSessionOwnerRef};
use super::*;
use crate::conn::{
    DevToolsEmulationSessionState, EffectiveTargetEmulationState,
    EffectiveTargetEmulationStateDelta, EmulatedDeviceMetrics, EmulatedGeolocationOverrideState,
    EmulatedMediaOverrides, EmulatedNetworkConditions,
};

pub(crate) struct TargetEmulationStateUpdate<'a> {
    raw: &'a mut DevToolsEmulationSessionState,
    effective: &'a mut EffectiveTargetEmulationState,
}

impl TargetEmulationStateUpdate<'_> {
    pub(crate) fn set_network_conditions(
        &mut self,
        network_conditions: Option<EmulatedNetworkConditions>,
    ) {
        self.raw.network_conditions = network_conditions;
        self.effective.network_conditions = network_conditions;
    }

    pub(crate) fn set_geolocation_override(
        &mut self,
        geolocation_override: Option<EmulatedGeolocationOverrideState>,
    ) {
        self.raw.geolocation_override = geolocation_override.clone();
        self.effective.geolocation_override = geolocation_override;
    }

    pub(crate) fn set_emulated_media(&mut self, emulated_media: EmulatedMediaOverrides) {
        self.raw.emulated_media = emulated_media.clone();
        self.effective.emulated_media = emulated_media;
    }

    pub(crate) fn set_emulated_device_metrics(
        &mut self,
        emulated_device_metrics: Option<EmulatedDeviceMetrics>,
    ) {
        self.raw.emulated_device_metrics = emulated_device_metrics.clone();
        self.effective.emulated_device_metrics = emulated_device_metrics;
    }

    pub(crate) fn set_cpu_throttling_rate(&mut self, cpu_throttling_rate: f64) {
        self.raw.cpu_throttling_rate = cpu_throttling_rate;
        self.effective.cpu_throttling_rate = cpu_throttling_rate;
    }

    pub(crate) fn set_touch_emulation_enabled(&mut self, enabled: bool) {
        self.raw.touch_emulation_enabled = enabled;
        self.effective.touch_emulation_enabled = enabled;
    }

    pub(crate) fn set_emit_touch_events_for_mouse(&mut self, enabled: bool) {
        self.raw.emit_touch_events_for_mouse = enabled;
        self.effective.emit_touch_events_for_mouse = enabled;
    }

    pub(crate) fn set_focus_emulation_enabled(&mut self, enabled: bool) {
        self.raw.focus_emulation_enabled = enabled;
        self.effective.focus_emulation_enabled = enabled;
    }

    pub(crate) fn set_script_execution_disabled(&mut self, disabled: bool) {
        self.raw.script_execution_disabled = disabled;
        self.effective.script_execution_disabled = disabled;
    }
}

impl TargetSessionOwnerMut<'_> {
    fn update_emulation_state(
        self,
        f: impl FnOnce(Option<TargetEmulationStateUpdate<'_>>),
    ) -> bool {
        let Some(state) = self.browser_context.page_target_mut(&self.target_id) else {
            f(None);
            return false;
        };
        let raw = &mut state
            .devtools_sessions
            .ensure_session(&self.session_key)
            .emulation_session_state;
        let effective = &mut state.effective_emulation_state;
        f(Some(TargetEmulationStateUpdate { raw, effective }));
        true
    }

    fn set_devtools_locale_override(
        &mut self,
        locale_override: Option<String>,
    ) -> Result<(), &'static str> {
        self.mutate_page_state(|state, session_key| {
            state.set_devtools_locale_override(session_key, locale_override)
        })
    }

    fn set_devtools_timezone_override(
        &mut self,
        timezone_override: Option<String>,
    ) -> Result<(), &'static str> {
        self.mutate_page_state(|state, session_key| {
            state.set_devtools_timezone_override(session_key, timezone_override)
        })
    }

    fn set_base_locale_override(
        &mut self,
        locale_override: Option<String>,
        fallback_identity: &moli_browser_profile::BrowserIdentityProfile,
    ) -> bool {
        self.mutate_page_state(|state, _session_key| {
            state.set_base_locale_override(locale_override.clone());
            state
                .network_policy
                .set_base_accept_language_override(locale_override, fallback_identity);
        });
        true
    }

    fn set_base_timezone_override(&mut self, timezone_override: Option<String>) -> bool {
        self.mutate_page_state(|state, _session_key| {
            state.set_base_timezone_override(timezone_override);
        });
        true
    }
}

impl TargetSessionOwnerRef<'_> {
    fn emit_touch_events_for_mouse(&self) -> Option<bool> {
        self.browser_context
            .page_target(&self.target_id)
            .map(|state| state.effective_emulation_state.emit_touch_events_for_mouse)
    }

    #[cfg(test)]
    fn emulation_session_state(&self) -> Option<&DevToolsEmulationSessionState> {
        self.browser_context
            .page_target(&self.target_id)?
            .devtools_sessions
            .session(&self.session_key)
            .map(|session| &session.emulation_session_state)
    }
}

impl CdpConnection {
    pub(crate) fn update_emulation_state_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        f: impl FnOnce(Option<TargetEmulationStateUpdate<'_>>),
    ) -> bool {
        let owner = crate::conn::CommandOwnerScope::capture(self, session_id);
        self.update_emulation_state_for_owner(&owner, f)
    }

    pub(crate) fn update_emulation_state_for_owner(
        &mut self,
        owner: &crate::conn::CommandOwnerScope,
        f: impl FnOnce(Option<TargetEmulationStateUpdate<'_>>),
    ) -> bool {
        self.with_target_session_owner_mut_for_owner(owner, |owner| owner.update_emulation_state(f))
            .unwrap_or(false)
    }

    pub(crate) fn set_devtools_locale_override_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        locale_override: Option<String>,
    ) -> Result<(), &'static str> {
        self.target_session_owner_mut(session_id)
            .ok_or("BrowserContextNotLoaded")?
            .set_devtools_locale_override(locale_override)
    }

    pub(crate) fn set_devtools_timezone_override_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        timezone_override: Option<String>,
    ) -> Result<(), &'static str> {
        self.target_session_owner_mut(session_id)
            .ok_or("BrowserContextNotLoaded")?
            .set_devtools_timezone_override(timezone_override)
    }

    pub(crate) fn set_base_locale_override_for_owner(
        &mut self,
        owner: &crate::conn::CommandOwnerScope,
        locale_override: Option<String>,
    ) -> bool {
        let fallback_identity = self.base_browser_identity.clone();
        self.target_session_owner_mut_for_owner(owner)
            .is_some_and(|mut owner| {
                owner.set_base_locale_override(locale_override, &fallback_identity)
            })
    }

    pub(crate) fn set_base_timezone_override_for_owner(
        &mut self,
        owner: &crate::conn::CommandOwnerScope,
        timezone_override: Option<String>,
    ) -> bool {
        self.target_session_owner_mut_for_owner(owner)
            .is_some_and(|mut owner| owner.set_base_timezone_override(timezone_override))
    }

    pub(crate) fn emit_touch_events_for_mouse_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> bool {
        self.target_session_owner_ref(session_id)
            .and_then(|owner| owner.emit_touch_events_for_mouse())
            .unwrap_or(false)
    }

    #[cfg(test)]
    pub(crate) fn emulation_session_state_for_session_owner(
        &self,
        session_id: Option<&str>,
    ) -> Option<DevToolsEmulationSessionState> {
        self.target_session_owner_ref(session_id)?
            .emulation_session_state()
            .cloned()
    }

    pub(crate) fn disable_emulation_session_handler_for_session_owner(
        &mut self,
        session_id: &str,
    ) -> Option<EffectiveTargetEmulationStateDelta> {
        let mut owner = self.target_session_owner_mut(Some(session_id))?;
        Some(owner.mutate_page_state(|target, session_key| {
            let raw = std::mem::take(
                &mut target
                    .devtools_sessions
                    .ensure_session(session_key)
                    .emulation_session_state,
            );
            target
                .effective_emulation_state
                .disable_session_handler(&raw)
        }))
    }
}

impl BrowserContext {
    pub(crate) fn effective_active_emulated_device_metrics(&self) -> Option<EmulatedDeviceMetrics> {
        self.active_page_target()
            .effective_emulation_state
            .emulated_device_metrics
            .clone()
            .or_else(|| self.default_emulated_device_metrics.clone())
    }
}
