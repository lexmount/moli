use super::target_session_owner::{TargetSessionOwnerMut, TargetSessionOwnerRef};
use super::*;
#[cfg(test)]
use crate::conn::DevToolsEmulationSessionState;
use crate::conn::{EmulatedDeviceMetrics, EmulationPolicyChange, EmulationPolicyDelta};

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
            .apply(change.clone());
        state.apply_emulation_policy_change(change);
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
            state.set_base_accept_language_override(locale_override, fallback_identity);
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
            .map(|state| state.emulation_policy().emit_touch_events_for_mouse)
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
    ) -> Option<EmulationPolicyDelta> {
        let owner = self.target_session_owner_mut(Some(session_id))?;
        let target = owner.browser_context.page_target_mut(&owner.target_id)?;
        let raw = std::mem::take(
            &mut target
                .devtools_sessions
                .ensure_session(&owner.session_key)
                .emulation_session_state,
        );
        Some(target.apply_emulation_policy_changes(raw.disable_policy_changes()))
    }
}

impl BrowserContext {
    pub(crate) fn effective_active_emulated_device_metrics(&self) -> Option<EmulatedDeviceMetrics> {
        self.active_page_target()
            .emulation_policy()
            .emulated_device_metrics
            .clone()
            .or_else(|| self.emulation_defaults().device_metrics.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emulation_policy_survives_projection_drop_and_updates_without_sessions() {
        let mut context = BrowserContext::new_with_page_for_test("CTX-policy", "TID-policy");
        context.attach_active_session("SID-primary");
        assert!(context.assign_attached_session_to_target("TID-policy", "SID-attached".into()));
        let id = context.active_page_target().web_contents_id();
        let mut conn = CdpConnection::default();
        conn.install_browser_context_fixture_for_test(context);
        for (session, change) in [
            ("SID-primary", EmulationPolicyChange::CpuThrottlingRate(4.0)),
            (
                "SID-attached",
                EmulationPolicyChange::CpuThrottlingRate(2.0),
            ),
            ("SID-primary", EmulationPolicyChange::FocusEnabled(true)),
        ] {
            assert!(conn.apply_emulation_override_for_session_owner(Some(session), change));
        }
        let primary = conn
            .emulation_session_state_for_session_owner(Some("SID-primary"))
            .unwrap();
        assert_eq!(primary.overrides.cpu_throttling_rate, 4.0);
        drop(primary);

        let mut contents = {
            let mut projection = conn
                .browser_context
                .as_mut()
                .unwrap()
                .take_page_target_for_close("TID-policy")
                .unwrap();
            std::mem::take(&mut projection.runtime_slot.page_slot_mut().contents)
        };
        drop(conn);
        assert_eq!(contents.id(), id);
        assert_eq!(contents.emulation_policy.cpu_throttling_rate, 2.0);
        assert!(contents.emulation_policy.focus_emulation_enabled);
        let snapshot = contents.emulation_policy.clone();
        contents
            .emulation_policy
            .apply(EmulationPolicyChange::ScriptExecutionDisabled(true));
        assert!(!snapshot.script_execution_disabled);
        assert!(contents.emulation_policy.script_execution_disabled);
    }
}
