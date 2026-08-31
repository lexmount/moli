use super::target_session_owner::{TargetSessionOwnerMut, TargetSessionOwnerRef};
use super::*;
use crate::conn::{
    EmulatedDeviceMetrics, EmulatedGeolocationOverrideState, EmulatedMediaOverrides,
    EmulatedNetworkConditions,
};

pub(crate) struct TargetEmulationSessionStateMut<'a> {
    pub(crate) network_conditions: &'a mut Option<EmulatedNetworkConditions>,
    pub(crate) geolocation_override: &'a mut Option<EmulatedGeolocationOverrideState>,
    pub(crate) emulated_media: &'a mut EmulatedMediaOverrides,
    pub(crate) emulated_device_metrics: &'a mut Option<EmulatedDeviceMetrics>,
    pub(crate) cpu_throttling_rate: &'a mut f64,
    pub(crate) touch_emulation_enabled: &'a mut bool,
    pub(crate) emit_touch_events_for_mouse: &'a mut bool,
    pub(crate) focus_emulation_enabled: &'a mut bool,
    pub(crate) script_execution_disabled: &'a mut bool,
}

impl TargetSessionOwnerMut<'_> {
    fn mutate_emulation_session_state(
        mut self,
        f: impl FnOnce(Option<TargetEmulationSessionStateMut<'_>>),
    ) -> bool {
        match &mut self {
            Self::ActiveTarget {
                browser_context, ..
            } => {
                let state = browser_context.active_page_state_mut();
                f(Some(TargetEmulationSessionStateMut {
                    network_conditions: &mut state.network_conditions,
                    geolocation_override: &mut state.geolocation_override,
                    emulated_media: &mut state.emulated_media,
                    emulated_device_metrics: &mut state.emulated_device_metrics,
                    cpu_throttling_rate: &mut state.cpu_throttling_rate,
                    touch_emulation_enabled: &mut state.touch_emulation_enabled,
                    emit_touch_events_for_mouse: &mut state.emit_touch_events_for_mouse,
                    focus_emulation_enabled: &mut state.focus_emulation_enabled,
                    script_execution_disabled: &mut state.script_execution_disabled,
                }))
            }
            Self::PageTargetHost {
                browser_context,
                target_id,
                ..
            } => browser_context.mutate_parked_page_session_state(target_id, |state| {
                f(Some(TargetEmulationSessionStateMut {
                    network_conditions: &mut state.network_conditions,
                    geolocation_override: &mut state.geolocation_override,
                    emulated_media: &mut state.emulated_media,
                    emulated_device_metrics: &mut state.emulated_device_metrics,
                    cpu_throttling_rate: &mut state.cpu_throttling_rate,
                    touch_emulation_enabled: &mut state.touch_emulation_enabled,
                    emit_touch_events_for_mouse: &mut state.emit_touch_events_for_mouse,
                    focus_emulation_enabled: &mut state.focus_emulation_enabled,
                    script_execution_disabled: &mut state.script_execution_disabled,
                }))
            }),
            Self::NoLoadedBrowserContext => {
                f(None);
                return false;
            }
        }
        true
    }

    fn set_devtools_locale_override(
        &mut self,
        locale_override: Option<String>,
    ) -> Result<(), &'static str> {
        self.mutate_page_state(|state, is_auxiliary_target_session, session_id| {
            state.set_devtools_locale_override(
                is_auxiliary_target_session,
                session_id,
                locale_override,
            )
        })
        .unwrap_or(Err("BrowserContextNotLoaded"))
    }

    fn set_devtools_timezone_override(
        &mut self,
        timezone_override: Option<String>,
    ) -> Result<(), &'static str> {
        self.mutate_page_state(|state, is_auxiliary_target_session, session_id| {
            state.set_devtools_timezone_override(
                is_auxiliary_target_session,
                session_id,
                timezone_override,
            )
        })
        .unwrap_or(Err("BrowserContextNotLoaded"))
    }

    fn set_base_locale_override(
        &mut self,
        locale_override: Option<String>,
        fallback_identity: &moli_browser_profile::BrowserIdentityProfile,
    ) -> bool {
        self.mutate_page_state(|state, _is_auxiliary_target_session, _session_id| {
            state.set_base_locale_override(locale_override.clone());
            state
                .network_policy
                .set_base_accept_language_override(locale_override, fallback_identity);
        })
        .is_some()
    }

    fn set_base_timezone_override(&mut self, timezone_override: Option<String>) -> bool {
        self.mutate_page_state(|state, _is_auxiliary_target_session, _session_id| {
            state.set_base_timezone_override(timezone_override);
        })
        .is_some()
    }
}

impl TargetSessionOwnerRef<'_> {
    fn emit_touch_events_for_mouse(&self) -> Option<bool> {
        match self {
            Self::ActiveTarget {
                browser_context, ..
            } => Some(browser_context.emit_touch_events_for_mouse),
            Self::PageTargetHost {
                browser_context,
                target_id,
                ..
            } => browser_context
                .parked_page_session_state(target_id)
                .map(|state| state.emit_touch_events_for_mouse),
            Self::NoLoadedBrowserContext => None,
        }
    }
}

impl CdpConnection {
    pub(crate) fn mutate_emulation_session_state_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        f: impl FnOnce(Option<TargetEmulationSessionStateMut<'_>>),
    ) -> bool {
        self.with_target_session_owner_mut(session_id, |owner| {
            owner.mutate_emulation_session_state(f)
        })
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

    pub(crate) fn set_base_locale_override_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        locale_override: Option<String>,
    ) -> bool {
        let fallback_identity = self.base_browser_identity.clone();
        self.target_session_owner_mut(session_id)
            .is_some_and(|mut owner| {
                owner.set_base_locale_override(locale_override, &fallback_identity)
            })
    }

    pub(crate) fn set_base_timezone_override_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        timezone_override: Option<String>,
    ) -> bool {
        self.target_session_owner_mut(session_id)
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
}

impl BrowserContext {
    pub(crate) fn effective_active_emulated_device_metrics(&self) -> Option<EmulatedDeviceMetrics> {
        self.emulated_device_metrics
            .clone()
            .or_else(|| self.default_emulated_device_metrics.clone())
    }

    pub(crate) fn effective_parked_emulated_device_metrics(
        &self,
        target_id: &str,
    ) -> Option<EmulatedDeviceMetrics> {
        self.parked_page_session_state(target_id)
            .and_then(|state| state.emulated_device_metrics.clone())
            .or_else(|| self.default_emulated_device_metrics.clone())
    }
}
