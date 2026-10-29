use crate::context_bootstrap::events::EventInit;
use crate::{web_api_interfaces, webidl};

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "NavigationType", rename_all = "kebab-case")]
pub(super) enum NavigationType {
    Push,
    Replace,
    Reload,
    Traverse,
}

// Convert inherited EventInit members first, then derived members in Web IDL
// lexicographic order. Parse the entire dictionary before initializing an event.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "NavigateEventInit", snapshot)]
pub(super) struct NavigateEventInitMembers<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(name = "canIntercept", default = false, snapshot_order = 2)]
    pub(super) can_intercept: bool,
    #[webidl(required, interface = web_api_interfaces::NavigationDestination, snapshot_order = 1)]
    pub(super) destination: v8::Local<'s, v8::Object>,
    #[webidl(nullable, converter = "raw", snapshot_order = 7)]
    pub(super) download_request: Option<webidl::DomString16>,
    #[webidl(nullable, interface = web_api_interfaces::FormData, snapshot_order = 6)]
    pub(super) form_data: Option<v8::Local<'s, v8::Object>>,
    #[webidl(name = "hasUAVisualTransition", default = false, snapshot_order = 9)]
    pub(super) has_ua_visual_transition: bool,
    #[webidl(name = "hashChange", default = false, snapshot_order = 4)]
    pub(super) hash_change: bool,
    #[webidl(converter = "raw", snapshot_default = v8::undefined(scope).into(), snapshot_order = 8)]
    pub(super) info: Option<v8::Local<'s, v8::Value>>,
    #[webidl(name = "navigationType", converter = "enum", default = NavigationType::Push, snapshot_order = 0)]
    pub(super) navigation_type: NavigationType,
    #[webidl(required, interface = web_api_interfaces::AbortSignal, snapshot_order = 5)]
    pub(super) signal: v8::Local<'s, v8::Object>,
    #[webidl(nullable, interface = web_api_interfaces::Element, snapshot_order = 10)]
    pub(super) source_element: Option<v8::Local<'s, v8::Object>>,
    #[webidl(name = "userInitiated", default = false, snapshot_order = 3)]
    pub(super) user_initiated: bool,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "NavigationCurrentEntryChangeEventInit", snapshot)]
pub(super) struct NavigationCurrentEntryChangeEventInitMembers<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(required, interface = web_api_interfaces::NavigationHistoryEntry)]
    pub(super) from: v8::Local<'s, v8::Object>,
    #[webidl(name = "navigationType", nullable, converter = "enum")]
    pub(super) navigation_type: Option<NavigationType>,
}

impl NavigateEventInitMembers<'_> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }
}

impl NavigationCurrentEntryChangeEventInitMembers<'_> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "NavigateEvent")]
struct NavigateArgs<'s> {
    #[webidl(index = 1, required, dictionary)]
    init: NavigateEventInitMembers<'s>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "NavigationCurrentEntryChangeEvent")]
struct CurrentEntryChangeArgs<'s> {
    #[webidl(index = 1, required, dictionary)]
    init: NavigationCurrentEntryChangeEventInitMembers<'s>,
}

pub(super) fn parse_navigate_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<NavigateEventInitMembers<'s>> {
    webidl::parse_args::<NavigateArgs>(scope, args).map(|parsed| parsed.init)
}

pub(super) fn parse_current_entry_change_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<NavigationCurrentEntryChangeEventInitMembers<'s>> {
    webidl::parse_args::<CurrentEntryChangeArgs>(scope, args).map(|parsed| parsed.init)
}
