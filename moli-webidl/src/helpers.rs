use crate::types::EventListenerOptionsMembers;
use crate::{
    Context, DomString, EventListenerOptions, UnrestrictedDouble, WebIdlError,
    legacy_optional_member, parse_dictionary_object,
};
pub use moli_v8_util::{
    is_nullish, throw_dom_exception, throw_index_size_error, throw_type_error, v8_string, v8str,
};

/// Throws a WebIDL conversion error as a JavaScript `TypeError`.
///
/// Pending exceptions are left alone because the conversion helper has already
/// rethrown the original JavaScript exception, usually from a property getter,
/// `toString`, `valueOf`, or iterator operation.
pub fn throw_error(scope: &mut v8::PinScope<'_, '_>, error: &WebIdlError) {
    if error.is_pending_exception() {
        return;
    }
    throw_type_error(scope, &error.to_string());
}

/// Reads a positional argument as an optional dictionary object.
///
/// Missing, `undefined`, and `null` arguments return `Ok(None)`. Non-object
/// values fail with a dictionary conversion error.
pub fn dictionary_arg<'s>(
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    context: Context,
) -> Result<Option<v8::Local<'s, v8::Object>>, crate::WebIdlError> {
    if args.length() <= index {
        return Ok(None);
    }
    dictionary_value(args.get(index), context)
}

/// Converts one JavaScript value into an optional dictionary object.
pub fn dictionary_value<'s>(
    value: v8::Local<'s, v8::Value>,
    context: Context,
) -> Result<Option<v8::Local<'s, v8::Object>>, crate::WebIdlError> {
    if is_nullish(value) {
        return Ok(None);
    }
    if !value.is_object() {
        return Err(crate::WebIdlError::new(
            context,
            crate::WebIdlErrorKind::CannotConvert("dictionary"),
        ));
    }
    v8::Local::<v8::Object>::try_from(value)
        .map(Some)
        .map_err(|_| {
            crate::WebIdlError::new(context, crate::WebIdlErrorKind::CannotConvert("dictionary"))
        })
}

/// Best-effort object extraction for legacy call sites.
///
/// This helper intentionally does not report conversion errors. Prefer
/// `dictionary_arg` or derived WebIDL parsing when a native binding must throw
/// browser-compatible TypeErrors.
pub fn optional_object_arg<'s>(
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Option<v8::Local<'s, v8::Object>> {
    if args.length() <= index {
        return None;
    }
    let value = args.get(index);
    if is_nullish(value) || !value.is_object() {
        return None;
    }
    v8::Local::<v8::Object>::try_from(value).ok()
}

/// Best-effort property read for legacy call sites.
pub fn property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    property_result(scope, object, key, Context::member("", key))
        .ok()
        .flatten()
}

/// Reads a named property while preserving JavaScript getter exceptions.
///
/// The returned `None` means V8 did not provide a value and no exception was
/// caught. If a getter throws, the original exception is rethrown and surfaced as
/// `WebIdlError::pending_exception(context)`.
pub fn property_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &'static str,
    context: Context,
) -> Result<Option<v8::Local<'s, v8::Value>>, WebIdlError> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let key = v8str(&scope, key);
    match object.get(&scope, key.into()) {
        Some(value) => Ok(Some(value)),
        None if scope.has_caught() => {
            let _ = scope.rethrow();
            Err(WebIdlError::pending_exception(context))
        }
        None => Ok(None),
    }
}

/// Reads a symbol-named property while preserving JavaScript getter exceptions.
pub fn symbol_property_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: v8::Local<'s, v8::Symbol>,
    context: Context,
) -> Result<Option<v8::Local<'s, v8::Value>>, WebIdlError> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    match object.get(&scope, key.into()) {
        Some(value) => Ok(Some(value)),
        None if scope.has_caught() => {
            let _ = scope.rethrow();
            Err(WebIdlError::pending_exception(context))
        }
        None => Ok(None),
    }
}

/// Best-effort property read that filters out `null` and `undefined`.
pub fn property_non_nullish<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    property(scope, object, key).filter(|value| !is_nullish(*value))
}

/// Best-effort property read that filters out only `undefined`.
pub fn property_non_undefined<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    property(scope, object, key).filter(|value| !value.is_undefined())
}

/// Legacy optional DOMString property helper.
pub fn optional_string_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &'static str,
) -> Option<String> {
    legacy_optional_member::<DomString>(scope, object, key, Context::member("", key))
        .ok()
        .flatten()
        .map(Into::into)
}

/// Legacy optional unrestricted-number property helper.
pub fn optional_number_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &'static str,
) -> Option<f64> {
    legacy_optional_member::<UnrestrictedDouble>(scope, object, key, Context::member("", key))
        .ok()
        .flatten()
        .map(Into::into)
}

/// Parses the third argument shape used by event listener registration.
///
/// Boolean values use the legacy capture-only path. Object values are parsed as
/// `AddEventListenerOptions`. When `observe_passive` is true, the `passive`
/// member is read even if the resulting value is not otherwise needed, matching
/// sites that observe getter side effects.
pub fn event_listener_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    observe_passive: bool,
) -> EventListenerOptions {
    if args.length() <= index {
        return EventListenerOptions::default();
    }
    event_listener_options_value(scope, args.get(index), observe_passive)
}

pub fn event_listener_options_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    observe_passive: bool,
) -> EventListenerOptions {
    if is_nullish(value) {
        return EventListenerOptions::default();
    }
    if !value.is_object() || value.is_boolean() {
        return EventListenerOptions {
            capture: value.boolean_value(scope),
            once: false,
            passive: None,
        };
    }
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return EventListenerOptions::default();
    };
    if observe_passive {
        let _ = property(scope, object, "passive");
    }
    parse_dictionary_object::<EventListenerOptionsMembers>(scope, object)
        .map(|parsed| EventListenerOptions {
            capture: parsed.capture,
            once: parsed.once,
            passive: parsed.passive,
        })
        .unwrap_or_default()
}

pub fn event_listener_once_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    event_listener_options_value(scope, value, false).once
}

pub fn event_listener_once_option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> bool {
    event_listener_options(scope, args, index, false).once
}

/// Converts `(AddEventListenerOptions or boolean)` through `passive`.
///
/// The caller must convert the platform-specific `signal` member next, before
/// changing the listener list. Getter exceptions stop conversion immediately.
pub fn add_event_listener_options_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<EventListenerOptions, WebIdlError> {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return Ok(EventListenerOptions {
            capture: value.boolean_value(scope),
            ..EventListenerOptions::default()
        });
    };
    let parsed = parse_dictionary_object::<EventListenerOptionsMembers>(scope, object)?;
    Ok(EventListenerOptions {
        capture: parsed.capture,
        once: parsed.once,
        passive: parsed.passive,
    })
}
