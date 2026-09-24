use super::*;
use crate::util::{get_private_value, set_private_value};

pub(super) fn set_location_href_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    href: &str,
) {
    if let Some(href) = v8_string(scope, href) {
        set_private_value(scope, object, WINDOW_LOCATION_HREF_SLOT, href.into());
    }
}

pub(in crate::context_bootstrap) fn location_href_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<String> {
    // Only a real Location wrapper may resolve the shared Window URL. The
    // generic owner helper falls back to the current global for unbound
    // objects, which would accidentally accept forged Location receivers.
    let object = get_private_value(
        scope,
        object,
        super::super::location_history_storage::WINDOW_RUNTIME_OWNER_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    .and_then(|owner| super::super::navigation_window::window_location_for_holder(scope, owner))
    .unwrap_or(object);
    get_private_value(scope, object, WINDOW_LOCATION_HREF_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(super) fn sync_location_object_fields(
    _scope: &mut v8::PinScope<'_, '_>,
    _object: v8::Local<'_, v8::Object>,
    _href: &str,
) {
    // URL components are installed as live accessors. The href slot is the
    // single source of truth, so sync must not replace those accessors with
    // data properties after navigation.
}

pub(in crate::context_bootstrap) fn sync_location_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    href: &str,
) {
    set_location_href_slot(scope, object, href);
    sync_location_object_fields(scope, object, href);
}
