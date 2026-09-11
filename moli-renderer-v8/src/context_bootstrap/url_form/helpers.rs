use super::*;
use crate::util::{get_private_value, set_private_value};
use crate::web_api_interfaces;

pub(in crate::context_bootstrap) fn callback_value_string(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<String> {
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(in crate::context_bootstrap) fn callback_arg_url_like_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<String> {
    if value.is_null_or_undefined() {
        return None;
    }
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && let Some(href) = url_href_slot(scope, object)
    {
        return Some(href);
    }
    callback_value_string(scope, value)
}

pub(super) fn resolve_url_constructor_input(
    input: &str,
    base: Option<&str>,
) -> std::result::Result<url::Url, url::ParseError> {
    if let Ok(url) = url::Url::parse(input) {
        return Ok(url);
    }
    let Some(base) = base else {
        return Err(url::ParseError::RelativeUrlWithoutBase);
    };
    let base = url::Url::parse(base)?;
    base.join(input)
}

pub(super) fn can_parse_url_input(input: &str, base: Option<&str>) -> bool {
    match base {
        Some(base) => url::Url::parse(base).is_ok_and(|base| base.join(input).is_ok()),
        None => url::Url::parse(input).is_ok(),
    }
}

pub(in crate::context_bootstrap) fn url_href_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    get_private_value(scope, object, URL_HREF_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(in crate::context_bootstrap) fn require_url_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if web_api_interfaces::URL::is_instance(scope, object) {
        return Some(object);
    }
    throw_type_error(scope, "Illegal invocation");
    None
}

pub(in crate::context_bootstrap) fn url_object_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<url::Url> {
    url_href_slot(scope, object).and_then(|href| url::Url::parse(&href).ok())
}

pub(in crate::context_bootstrap) fn apply_url_update<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    url: &url::Url,
) {
    let href = url_href_slot(scope, object)
        .and_then(|old_href| href_after_opaque_query_removal(&old_href, url))
        .unwrap_or_else(|| url.as_str().to_owned());
    if let Some(href) = v8_string(scope, &href) {
        set_private_value(scope, object, URL_HREF_SLOT, href.into());
    }
}

pub(super) fn constructor_url_href(input: &str, url: &url::Url) -> String {
    if let Some(href) = href_after_opaque_query_removal(input, url) {
        return href;
    }
    url.as_str().to_owned()
}

fn href_after_opaque_query_removal(old_href: &str, url: &url::Url) -> Option<String> {
    if !url.cannot_be_a_base() || url.query().is_some() {
        return None;
    }
    let old_url = url::Url::parse(old_href).ok()?;
    old_url.query()?;
    let old_path = raw_opaque_path(old_href)?;
    let encoded_path = encode_final_trailing_space(old_path)?;
    let mut href = format!("{}:{encoded_path}", url.scheme());
    if let Some(fragment) = url.fragment() {
        href.push('#');
        href.push_str(fragment);
    }
    Some(href)
}

fn raw_opaque_path(href: &str) -> Option<&str> {
    let (_, after_scheme) = href.split_once(':')?;
    let end = after_scheme.find(['?', '#']).unwrap_or(after_scheme.len());
    Some(&after_scheme[..end])
}

fn encode_final_trailing_space(path: &str) -> Option<String> {
    let prefix = path.strip_suffix(' ')?;
    let mut encoded = String::with_capacity(path.len() + 2);
    encoded.push_str(prefix);
    encoded.push_str("%20");
    Some(encoded)
}
