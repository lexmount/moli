use crate::{
    util::{context_host_ptr_from_window_object, throw_type_error},
    webidl,
};

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "EventInit")]
struct EventInitMembers {
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
}

pub(super) fn init_bool_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
    key: &'static str,
    default: bool,
) -> bool {
    let Some(object) = init else {
        return default;
    };
    webidl::legacy_bool_member_or(scope, object, "EventInit", key, default)
}

pub(super) fn init_number_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
    key: &'static str,
    default: f64,
) -> f64 {
    let Some(object) = init else {
        return default;
    };
    webidl::legacy_number_member_or(scope, object, "EventInit", key, default)
}

pub(super) fn init_string_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
    key: &'static str,
    default: &str,
) -> String {
    let Some(object) = init else {
        return default.to_owned();
    };
    webidl::legacy_string_member_or(scope, object, "EventInit", key, default)
}

pub(super) fn init_value_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
    key: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    init.and_then(|object| webidl::property_non_undefined(scope, object, key))
}

pub(super) fn init_window_view_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
    constructor_name: &'static str,
) -> Result<v8::Local<'s, v8::Value>, ()> {
    let Some(value) = init_value_property(scope, init, "view") else {
        return Ok(v8::null(scope).into());
    };
    if value.is_null() {
        return Ok(value);
    }
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        throw_type_error(
            scope,
            &format!(
                "Failed to construct '{constructor_name}': member view is not of type Window."
            ),
        );
        return Err(());
    };
    if context_host_ptr_from_window_object(scope, object).is_none() {
        throw_type_error(
            scope,
            &format!(
                "Failed to construct '{constructor_name}': member view is not of type Window."
            ),
        );
        return Err(());
    }
    Ok(value)
}

pub(super) fn read_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> (bool, bool, bool) {
    parse_event_init(scope, webidl::optional_object_arg(args, 1)).unwrap_or((false, false, false))
}

pub(in crate::context_bootstrap) fn parse_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Result<(bool, bool, bool), webidl::WebIdlError> {
    let Some(init) = init else {
        return Ok((false, false, false));
    };
    webidl::parse_dictionary_object::<EventInitMembers>(scope, init)
        .map(|parsed| (parsed.bubbles, parsed.cancelable, parsed.composed))
}
