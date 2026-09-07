use super::target_session_owner::{TargetSessionOwnerMut, TargetSessionOwnerRef};
use super::*;
#[cfg(test)]
use crate::conn::DevToolsEmulationSessionState;
use crate::conn::{EmulatedDeviceMetrics, EmulationPolicyChange};

impl TargetSessionOwnerMut<'_> {
    fn apply_emulation_override(self, change: EmulationPolicyChange) -> bool {
        let Some(state) = self.browser_context.page_target_mut(&self.target_id) else {
            return false;
        };
        state
            .devtools_sessions
            .ensure_session(&self.session_key)
            .emulation_session_state
            .overrides
            .get_or_insert_default()
            .apply(change.clone());
        self.browser_context
            .apply_target_emulation_policy_change(&self.target_id, change);
        true
    }

    fn set_devtools_locale_override(
        &mut self,
        locale_override: Option<String>,
    ) -> Result<(), &'static str> {
        {
            self.browser_context
                .set_devtools_locale_override_for_target(
                    &self.target_id,
                    &self.session_key,
                    locale_override,
                )
        }
    }

    fn set_devtools_timezone_override(
        &mut self,
        timezone_override: Option<String>,
    ) -> Result<(), &'static str> {
        {
            self.browser_context
                .set_devtools_timezone_override_for_target(
                    &self.target_id,
                    &self.session_key,
                    timezone_override,
                )
        }
    }

    fn set_base_locale_override(
        &mut self,
        locale_override: Option<String>,
        fallback_identity: &moli_browser_profile::BrowserIdentityProfile,
    ) -> bool {
        {
            self.browser_context
                .set_base_locale_override_for_target(&self.target_id, locale_override.clone());
            self.browser_context
                .set_base_accept_language_override_for_target(
                    &self.target_id,
                    locale_override,
                    fallback_identity,
                );
        };
        true
    }

    fn set_base_timezone_override(&mut self, timezone_override: Option<String>) -> bool {
        {
            self.browser_context
                .set_base_timezone_override_for_target(&self.target_id, timezone_override);
        };
        true
    }
}

impl TargetSessionOwnerRef<'_> {
    fn emit_touch_events_for_mouse(&self) -> Option<bool> {
        self.browser_context
            .target_emulation_policy(&self.target_id)
            .map(|policy| policy.emit_touch_events_for_mouse)
    }

    fn emulation_disposal_is_effectively_noop(&self) -> Option<bool> {
        let target = self.browser_context.page_target(&self.target_id)?;
        let session = target.devtools_sessions.session(&self.session_key)?;
        let effective = self
            .browser_context
            .target_emulation_policy(&self.target_id)?;
        Some(
            session
                .emulation_session_state
                .disposal_is_effectively_noop(effective),
        )
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
    // In-place value dispatch until AgentHost/BrowserHandle policy cutover
    // (Commits 14/22); it exposes neither Browser state nor a mutable callback.
    pub(crate) fn apply_emulation_override_for_session_owner(
        &mut self,
        session_id: Option<&str>,
        change: EmulationPolicyChange,
    ) -> bool {
        let owner = crate::conn::CommandOwnerScope::capture(self, session_id);
        self.apply_emulation_override_for_owner(&owner, change)
    }

    pub(crate) fn apply_emulation_override_for_owner(
        &mut self,
        owner: &crate::conn::CommandOwnerScope,
        change: EmulationPolicyChange,
    ) -> bool {
        self.target_session_owner_mut_for_owner(owner)
            .is_some_and(|owner| owner.apply_emulation_override(change))
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

    pub(crate) fn emulation_disposal_is_effectively_noop_for_session_owner(
        &self,
        session_id: &str,
    ) -> bool {
        self.target_session_owner_ref(Some(session_id))
            .and_then(|owner| owner.emulation_disposal_is_effectively_noop())
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
    ) -> bool {
        let Some(owner) = self.target_session_owner_mut(Some(session_id)) else {
            return false;
        };
        let Some(target) = owner.browser_context.page_target_mut(&owner.target_id) else {
            return false;
        };
        let changes = target
            .devtools_sessions
            .ensure_session(&owner.session_key)
            .emulation_session_state
            .disable_policy_changes();
        owner
            .browser_context
            .apply_target_emulation_policy_changes(&owner.target_id, changes);
        true
    }

    pub(crate) fn set_emulation_renderer_cleanup_pending_for_session_owner(
        &mut self,
        session_id: &str,
        pending: bool,
    ) {
        if let Some(owner) = self.target_session_owner_mut(Some(session_id))
            && let Some(target) = owner.browser_context.page_target_mut(&owner.target_id)
        {
            target
                .devtools_sessions
                .ensure_session(&owner.session_key)
                .emulation_session_state
                .set_renderer_cleanup_pending(pending);
        }
    }
}

impl BrowserContext {
    pub(crate) fn effective_active_emulated_device_metrics(&self) -> Option<EmulatedDeviceMetrics> {
        self.active_target_id()
            .and_then(|id| self.target_emulation_policy(id))
            .and_then(|policy| policy.emulated_device_metrics.clone())
            .or_else(|| self.emulation_defaults().device_metrics.clone())
    }
}
