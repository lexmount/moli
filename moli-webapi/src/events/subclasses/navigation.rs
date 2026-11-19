pub const NAVIGATE_EVENT_SYNTHETIC_SLOT: &str = "__lmNavigateEventSynthetic";
use super::super::init::EventInit;
use super::{EventSubclassKind, SubclassInit, construct_trusted_subclass, native_event_interface};
use crate::interfaces as web_api_interfaces;
use moli_webidl as webidl;
use webidl::{WebIdlDictionarySerialize, WebIdlEnum};

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

impl<'s> NavigateEventInitMembers<'s> {
    pub(super) fn initialize(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
        synthetic: bool,
    ) {
        self.initialize_snapshot(scope, event)
            .expect("NavigateEvent state");
        super::super::state::set_event_private_value(
            scope,
            event,
            NAVIGATE_EVENT_SYNTHETIC_SLOT,
            v8::Boolean::new(scope, synthetic).into(),
        );
    }
}

pub fn construct_native_navigate_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    navigation_type: &str,
    destination: v8::Local<'s, v8::Object>,
    signal: v8::Local<'s, v8::Object>,
    cancelable: bool,
    can_intercept: bool,
    user_initiated: bool,
    hash_change: bool,
    download_request: Option<&str>,
    info: Option<v8::Local<'s, v8::Value>>,
    form_data: Option<v8::Local<'s, v8::Value>>,
    source_element: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let navigation_type = NavigationType::parse_token(navigation_type)?;
    let form_data = match form_data.filter(|value| !value.is_null_or_undefined()) {
        Some(form_data) => Some(native_event_interface(
            scope,
            form_data,
            "NavigateEventInit",
            "formData",
            webidl::InterfaceOptions {
                name: "FormData",
                brand_check: web_api_interfaces::FormData::is_instance,
            },
        )?),
        None => None,
    };
    let init = NavigateEventInitMembers {
        base: EventInit {
            cancelable,
            ..EventInit::default()
        },
        destination,
        signal,
        can_intercept,
        user_initiated,
        hash_change,
        download_request: download_request
            .map(|value| webidl::DomString16(value.encode_utf16().collect())),
        form_data,
        has_ua_visual_transition: false,
        info: info.filter(|value| !value.is_undefined()),
        navigation_type,
        source_element,
    };
    construct_trusted_subclass(
        scope,
        EventSubclassKind::NavigateEvent,
        "navigate",
        SubclassInit::Navigate(init, false),
    )
}

pub fn construct_native_current_entry_change_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    from: Option<v8::Local<'s, v8::Object>>,
    navigation_type: Option<&str>,
) -> Option<v8::Local<'s, v8::Object>> {
    let from = from
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into());
    let from = native_event_interface(
        scope,
        from,
        "NavigationCurrentEntryChangeEventInit",
        "from",
        webidl::InterfaceOptions {
            name: "NavigationHistoryEntry",
            brand_check: web_api_interfaces::NavigationHistoryEntry::is_instance,
        },
    )?;
    let navigation_type = match navigation_type {
        Some(token) => Some(NavigationType::parse_token(token)?),
        None => None,
    };
    let init = NavigationCurrentEntryChangeEventInitMembers {
        base: EventInit::default(),
        from,
        navigation_type,
    };
    construct_trusted_subclass(
        scope,
        EventSubclassKind::NavigationCurrentEntryChangeEvent,
        "currententrychange",
        SubclassInit::CurrentEntry(init),
    )
}
impl<'s> NavigationCurrentEntryChangeEventInitMembers<'s> {
    pub(super) fn initialize(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        self.initialize_snapshot(scope, event)
            .expect("current entry event state");
    }
}
