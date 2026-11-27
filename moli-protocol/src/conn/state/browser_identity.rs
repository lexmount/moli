use moli_browser_profile::BrowserIdentityProfile;

/// Independent non-CDP browser identity contributions.
///
/// WebDriver BiDi exposes user-agent and locale as separate overrides. Keep
/// those inputs separate and materialize one coherent profile for network and
/// Navigator surfaces so setting or clearing either input cannot erase the
/// other one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct BaseBrowserIdentityOverrideState {
    user_agent: Option<String>,
    has_language_override: bool,
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
        self.user_agent = user_agent;
        self.refresh(fallback, None);
    }

    pub(crate) fn set_accept_language(
        &mut self,
        accept_language: Option<String>,
        fallback: &BrowserIdentityProfile,
    ) {
        self.has_language_override = accept_language.is_some();
        self.refresh(fallback, accept_language);
    }

    pub(crate) fn replace_profile(&mut self, profile: BrowserIdentityProfile) {
        self.user_agent = Some(profile.user_agent().to_owned());
        self.has_language_override = true;
        self.materialized = Some(profile);
    }

    fn refresh(&mut self, fallback: &BrowserIdentityProfile, accept_language: Option<String>) {
        let user_agent = self
            .user_agent
            .as_deref()
            .unwrap_or_else(|| fallback.user_agent());
        self.materialized =
            (self.user_agent.is_some() || self.has_language_override).then(
                || match accept_language {
                    Some(accept_language) => {
                        BrowserIdentityProfile::new(user_agent, accept_language)
                    }
                    None if self.has_language_override => self
                        .materialized
                        .as_ref()
                        .unwrap_or(fallback)
                        .with_user_agent(user_agent),
                    None => fallback.with_user_agent(user_agent),
                },
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_user_agent_and_language_layers_survive_reverse_order_and_clear() {
        let fallback = BrowserIdentityProfile::default()
            .with_languages(vec!["zh-CN".to_owned()])
            .unwrap();
        let mut state = BaseBrowserIdentityOverrideState::default();

        state.set_user_agent(Some("Moli BiDi".to_owned()), &fallback);
        assert_eq!(state.profile().unwrap().languages(), ["zh-CN"]);
        state.set_user_agent(None, &fallback);

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
        assert_eq!(state.profile().unwrap().languages(), ["zh-CN"]);

        state.set_user_agent(None, &fallback);
        assert!(state.profile().is_none());

        state.replace_profile(fallback.clone());
        state.set_user_agent(Some("Moli BiDi".to_owned()), &fallback);
        assert_eq!(state.profile().unwrap().languages(), ["zh-CN"]);
    }
}
