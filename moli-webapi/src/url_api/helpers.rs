use super::*;
use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, set_private_value};

pub fn callback_value_string(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<String> {
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(super) fn resolve_url_constructor_input(
    input: &str,
    base: Option<&str>,
) -> std::result::Result<url::Url, url::ParseError> {
    // A supplied base must be validated even for an absolute input. It also
    // participates in parsing same-scheme inputs such as `https:child`.
    match base {
        Some(base) => url::Url::parse(base)?.join(input),
        None => url::Url::parse(input),
    }
}

pub(super) fn can_parse_url_input(input: &str, base: Option<&str>) -> bool {
    resolve_url_constructor_input(input, base).is_ok()
}

pub(crate) fn url_href_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    get_private_value(scope, object, URL_HREF_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(crate) fn require_url_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if web_api_interfaces::URL::is_instance(scope, object) {
        return Some(object);
    }
    throw_type_error(scope, "Illegal invocation");
    None
}

pub(crate) fn url_object_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<url::Url> {
    url_href_slot(scope, object).and_then(|href| url::Url::parse(&href).ok())
}

pub(crate) fn apply_url_update<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    url: &url::Url,
) {
    if let Some(href) = v8_string(scope, url.as_str()) {
        set_private_value(scope, object, URL_HREF_SLOT, href.into());
    }
}
