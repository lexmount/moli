use moli_webidl as webidl;

#[derive(Clone, Copy, PartialEq, Eq, strum::EnumString, webidl::WebIdlEnum)]
#[webidl(name = "NavigationHistoryBehavior")]
#[strum(serialize_all = "lowercase")]
pub enum NavigationNavigateHistoryKind {
    #[webidl(token = "auto")]
    #[strum(serialize = "auto")]
    Default,
    Push,
    Replace,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "NavigationNavigateOptions")]
pub struct NavigationNavigateOptionsMembers {
    #[webidl(name = "history", converter = "enum", default = NavigationNavigateHistoryKind::Default)]
    pub history: NavigationNavigateHistoryKind,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Navigation.navigate")]
pub struct NavigationNavigateArgs {
    #[webidl(
        required,
        name = "url",
        converter = "usv_string",
        missing_message = "Failed to execute 'navigate' on 'Navigation': 1 argument required, but only 0 present."
    )]
    pub url: String,
}

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
#[webidl(name = "NavigationFocusReset", rename_all = "kebab-case")]
pub enum NavigationFocusReset {
    #[default]
    AfterTransition,
    Manual,
}

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
#[webidl(name = "NavigationScrollBehavior", rename_all = "kebab-case")]
pub enum NavigationScrollBehavior {
    #[default]
    AfterTransition,
    Manual,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "NavigateEvent.intercept")]
pub struct NavigationInterceptOptionsMembers {
    #[webidl(name = "focusReset", converter = "enum")]
    pub focus_reset: Option<NavigationFocusReset>,
    #[webidl(converter = "callback_function")]
    pub handler: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(name = "precommitHandler", converter = "callback_function")]
    pub precommit_handler: Option<webidl::WebIdlCallbackFunction>,
    #[webidl(converter = "enum")]
    pub scroll: Option<NavigationScrollBehavior>,
}
