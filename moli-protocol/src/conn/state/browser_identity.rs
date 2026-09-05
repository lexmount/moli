use moli_browser_profile::BrowserIdentityProfile;

/// Independent non-CDP browser identity contributions.
///
/// WebDriver BiDi exposes user-agent and locale as separate overrides. Keep
/// those inputs separate and materialize one coherent profile for network and
/// Navigator surfaces so setting or clearing either input cannot erase the
/// other one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct BrowserIdentityOverrideInputs {
    pub(super) user_agent: Option<String>,
    pub(super) accept_language: Option<String>,
}

impl BrowserIdentityOverrideInputs {
    pub(super) fn from_profile(profile: &BrowserIdentityProfile) -> Self {
        Self {
            user_agent: Some(profile.user_agent().to_owned()),
            accept_language: Some(profile.accept_language().to_owned()),
        }
    }

    pub(super) fn materialize(
        &self,
        fallback: &BrowserIdentityProfile,
    ) -> Option<BrowserIdentityProfile> {
        if self.user_agent.is_none() && self.accept_language.is_none() {
            return None;
        }
        Some(BrowserIdentityProfile::new(
            self.user_agent
                .as_deref()
                .unwrap_or_else(|| fallback.user_agent()),
            self.accept_language
                .as_deref()
                .unwrap_or_else(|| fallback.accept_language()),
        ))
    }
}

/// Page-level base contribution retained beneath the frontend session overlays.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BaseBrowserIdentityOverrideState {
    inputs: BrowserIdentityOverrideInputs,
    materialized: Option<BrowserIdentityProfile>,
}

impl BaseBrowserIdentityOverrideState {
    pub(crate) fn profile(&self) -> Option<&BrowserIdentityProfile> {
        self.materialized.as_ref()
    }

    pub(crate) fn profile_owned(&self) -> Option<BrowserIdentityProfile> {
        self.profile().cloned()
    }

    pub(crate) fn set_user_agent(
        &mut self,
        user_agent: Option<String>,
        fallback: &BrowserIdentityProfile,
    ) {
        self.inputs.user_agent = user_agent;
        self.materialized = self.inputs.materialize(fallback);
    }

    pub(crate) fn set_accept_language(
        &mut self,
        accept_language: Option<String>,
        fallback: &BrowserIdentityProfile,
    ) {
        self.inputs.accept_language = accept_language;
        self.materialized = self.inputs.materialize(fallback);
    }

    pub(crate) fn replace_profile(&mut self, profile: BrowserIdentityProfile) {
        self.inputs = BrowserIdentityOverrideInputs::from_profile(&profile);
        self.materialized = Some(profile);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_user_agent_and_language_layers_survive_reverse_order_and_clear() {
        let fallback = BrowserIdentityProfile::default();
        let mut state = BaseBrowserIdentityOverrideState::default();

        state.set_accept_language(Some("fr-FR".to_owned()), &fallback);
        assert_eq!(state.profile().unwrap().user_agent(), fallback.user_agent());
        assert_eq!(state.profile().unwrap().accept_language(), "fr-FR");

        state.set_user_agent(Some("Moli BiDi".to_owned()), &fallback);
        assert_eq!(state.profile().unwrap().user_agent(), "Moli BiDi");
        assert_eq!(state.profile().unwrap().accept_language(), "fr-FR");

        state.set_accept_language(None, &fallback);
        assert_eq!(state.profile().unwrap().user_agent(), "Moli BiDi");
        assert_eq!(
            state.profile().unwrap().accept_language(),
            fallback.accept_language()
        );

        state.set_user_agent(None, &fallback);
        assert!(state.profile().is_none());
    }
}
