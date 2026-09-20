use super::slots::location_href_slot;
use super::*;

pub(super) fn v8_value_to_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<String> {
    if value.is_null_or_undefined() {
        Some(String::new())
    } else {
        value
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
    }
}

pub(super) fn set_return_string(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    value: &str,
) {
    if let Some(value) = v8_string(scope, value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(super) fn parsed_location_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<url::Url> {
    location_url(scope, object).and_then(|href| url::Url::parse(&href).ok())
}

pub(super) fn location_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let href = require_location_href_slot(scope, object)?;
    // A retained Location has no relevant Document after its Window loses its
    // browsing context. Its URL is then about:blank, independent of the old
    // Document's URL or any new Window created for the same iframe element.
    Some(
        if super::install::location_has_relevant_document(scope, object) {
            href
        } else {
            "about:blank".to_owned()
        },
    )
}

pub(super) fn require_location_href_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    let href = location_href_slot(scope, object);
    if href.is_none() {
        if crate::native_bridge::is_cross_origin_location_proxy(scope, object) {
            crate::native_bridge::throw_cross_origin_location_security_error(scope);
        } else {
            crate::util::throw_type_error(scope, "Illegal invocation");
        }
    }
    href
}

pub(super) fn navigate_modified_location_url<'s, F>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
    mutate: F,
) where
    F: FnOnce(&mut url::Url) -> bool,
{
    let Some(current_href) = require_location_href_slot(scope, holder) else {
        return;
    };
    let Ok(mut current) = url::Url::parse(&current_href) else {
        return;
    };
    if !mutate(&mut current) {
        return;
    }
    let next_href = current.to_string();
    if next_href == current_href {
        return;
    }
    let holder = v8::Global::new(scope, holder);
    let holder = v8::Local::new(scope, holder);
    navigate_location_object(
        scope,
        holder,
        LocationNavigationKind::Assign,
        Some(next_href),
    );
}

pub(super) fn location_host_string(url: &url::Url) -> String {
    url.host_str()
        .map(|host| {
            url.port()
                .map(|port| format!("{host}:{port}"))
                .unwrap_or_else(|| host.to_owned())
        })
        .unwrap_or_default()
}
