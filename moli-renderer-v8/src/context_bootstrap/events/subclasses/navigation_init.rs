use crate::{web_api_interfaces, webidl};
use moli_webapi_declare::WebApiInterfaceDescriptor;

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "NavigationType", rename_all = "kebab-case")]
pub(super) enum NavigationType {
    Push,
    Replace,
    Reload,
    Traverse,
}

impl NavigationType {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::Replace => "replace",
            Self::Reload => "reload",
            Self::Traverse => "traverse",
        }
    }
}

// Convert inherited EventInit members first, then derived members in Web IDL
// lexicographic order. Parse the entire dictionary before initializing an event.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "NavigateEventInit")]
pub(super) struct NavigateEventInitMembers<'s> {
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
    #[webidl(name = "canIntercept", default = false)]
    pub(super) can_intercept: bool,
    #[webidl(with = destination_member)]
    pub(super) destination: v8::Local<'s, v8::Value>,
    #[webidl(name = "downloadRequest", with = download_request_member)]
    pub(super) download_request: v8::Local<'s, v8::Value>,
    #[webidl(name = "formData", with = form_data_member)]
    pub(super) form_data: v8::Local<'s, v8::Value>,
    #[webidl(name = "hasUAVisualTransition", default = false)]
    pub(super) has_ua_visual_transition: bool,
    #[webidl(name = "hashChange", default = false)]
    pub(super) hash_change: bool,
    #[webidl(converter = "raw")]
    pub(super) info: Option<v8::Local<'s, v8::Value>>,
    #[webidl(name = "navigationType", converter = "enum", default = NavigationType::Push)]
    pub(super) navigation_type: NavigationType,
    #[webidl(with = signal_member)]
    pub(super) signal: v8::Local<'s, v8::Value>,
    #[webidl(name = "sourceElement", with = source_element_member)]
    pub(super) source_element: v8::Local<'s, v8::Value>,
    #[webidl(name = "userInitiated", default = false)]
    pub(super) user_initiated: bool,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "NavigationCurrentEntryChangeEventInit")]
pub(super) struct NavigationCurrentEntryChangeEventInitMembers<'s> {
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
    #[webidl(with = from_member)]
    pub(super) from: v8::Local<'s, v8::Value>,
    #[webidl(name = "navigationType", nullable, converter = "enum")]
    pub(super) navigation_type: Option<NavigationType>,
}

impl NavigateEventInitMembers<'_> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.bubbles, self.cancelable, self.composed)
    }
}

impl NavigationCurrentEntryChangeEventInitMembers<'_> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.bubbles, self.cancelable, self.composed)
    }
}

pub(super) fn parse_navigate_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<NavigateEventInitMembers<'s>> {
    parse_required_init(
        scope,
        args,
        "NavigateEvent",
        "NavigateEventInit",
        "destination",
    )
}

pub(super) fn parse_current_entry_change_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<NavigationCurrentEntryChangeEventInitMembers<'s>> {
    parse_required_init(
        scope,
        args,
        "NavigationCurrentEntryChangeEvent",
        "NavigationCurrentEntryChangeEventInit",
        "from",
    )
}

fn parse_required_init<'s, T: webidl::WebIdlDictionary<'s>>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    constructor: &'static str,
    dictionary: &'static str,
    required_member: &'static str,
) -> Option<T> {
    let result = webidl::dictionary_arg(args, 1, webidl::Context::argument(constructor, 2))
        .and_then(|object| {
            object.ok_or_else(|| {
                webidl::WebIdlError::missing_required(webidl::Context::member(
                    dictionary,
                    required_member,
                ))
            })
        })
        .and_then(|object| webidl::parse_dictionary_object(scope, object));
    match result {
        Ok(init) => Some(init),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn interface_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
    dictionary: &'static str,
    interface: WebApiInterfaceDescriptor,
    nullable: bool,
) -> Result<v8::Local<'s, v8::Value>, webidl::WebIdlError> {
    let context = webidl::Context::member(dictionary, name);
    let value = webidl::property_result(scope, object, name, context)?
        .filter(|value| !value.is_undefined());
    let Some(value) = value else {
        return if nullable {
            Ok(v8::null(scope).into())
        } else {
            Err(webidl::WebIdlError::missing_required(context))
        };
    };
    if nullable && value.is_null() {
        return Ok(value);
    }
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && interface.is_instance(scope, object)
    {
        return Ok(value);
    }
    Err(webidl::WebIdlError::cannot_convert(
        context,
        interface.name(),
    ))
}

fn destination_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::Value>, webidl::WebIdlError> {
    interface_member(
        scope,
        object,
        name,
        "NavigateEventInit",
        web_api_interfaces::NavigationDestination::DESCRIPTOR,
        false,
    )
}

fn signal_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::Value>, webidl::WebIdlError> {
    interface_member(
        scope,
        object,
        name,
        "NavigateEventInit",
        web_api_interfaces::AbortSignal::DESCRIPTOR,
        false,
    )
}

fn form_data_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::Value>, webidl::WebIdlError> {
    interface_member(
        scope,
        object,
        name,
        "NavigateEventInit",
        web_api_interfaces::FormData::DESCRIPTOR,
        true,
    )
}

fn source_element_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::Value>, webidl::WebIdlError> {
    interface_member(
        scope,
        object,
        name,
        "NavigateEventInit",
        web_api_interfaces::Element::DESCRIPTOR,
        true,
    )
}

fn from_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::Value>, webidl::WebIdlError> {
    interface_member(
        scope,
        object,
        name,
        "NavigationCurrentEntryChangeEventInit",
        web_api_interfaces::NavigationHistoryEntry::DESCRIPTOR,
        false,
    )
}

fn download_request_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::Value>, webidl::WebIdlError> {
    let context = webidl::Context::member("NavigateEventInit", name);
    let Some(value) = webidl::property_result(scope, object, name, context)? else {
        return Ok(v8::null(scope).into());
    };
    if value.is_null_or_undefined() {
        return Ok(v8::null(scope).into());
    }
    // Retain V8's UTF-16 string, including lone surrogates.
    value
        .to_string(scope)
        .map(Into::into)
        .ok_or_else(|| webidl::WebIdlError::pending_exception(context))
}
