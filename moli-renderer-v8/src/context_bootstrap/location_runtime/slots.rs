use super::*;
use crate::util::{get_private_object, get_private_value, set_private_value};

const LOCATION_ANCESTOR_ORIGINS_SLOT: &str = "__moliLocationAncestorOrigins";
const LOCATION_EMPTY_ANCESTOR_ORIGINS_SLOT: &str = "__moliLocationEmptyAncestorOrigins";
const LOCATION_RELEVANT_DOCUMENT_ID_SLOT: &str = "__moliLocationRelevantDocumentId";
const LOCATION_RELEVANT_LOCAL_WINDOW_ID_SLOT: &str = "__moliLocationRelevantLocalWindowId";

pub(super) fn location_ancestor_origins_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, object, LOCATION_ANCESTOR_ORIGINS_SLOT)
}

pub(super) fn set_location_ancestor_origins_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Object>,
) {
    set_private_value(scope, object, LOCATION_ANCESTOR_ORIGINS_SLOT, value.into());
}

pub(super) fn clear_location_ancestor_origins_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    let undefined = v8::undefined(scope);
    set_private_value(
        scope,
        object,
        LOCATION_ANCESTOR_ORIGINS_SLOT,
        undefined.into(),
    );
}

pub(super) fn location_empty_ancestor_origins_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, object, LOCATION_EMPTY_ANCESTOR_ORIGINS_SLOT)
}

pub(super) fn set_location_empty_ancestor_origins_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        object,
        LOCATION_EMPTY_ANCESTOR_ORIGINS_SLOT,
        value.into(),
    );
}

pub(super) fn location_relevant_document_id_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    let value = get_private_value(scope, object, LOCATION_RELEVANT_DOCUMENT_ID_SLOT)?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (document_id, lossless) = value.u64_value();
    lossless.then_some(document_id)
}

pub(super) fn set_location_relevant_document_id_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    document_id: u64,
) {
    let value = v8::BigInt::new_from_u64(scope, document_id);
    set_private_value(
        scope,
        object,
        LOCATION_RELEVANT_DOCUMENT_ID_SLOT,
        value.into(),
    );
}

pub(super) fn location_relevant_local_window_id_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    let value = get_private_value(scope, object, LOCATION_RELEVANT_LOCAL_WINDOW_ID_SLOT)?;
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (local_window_id, lossless) = value.u64_value();
    lossless.then_some(local_window_id)
}

pub(super) fn set_location_relevant_local_window_id_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    local_window_id: u64,
) {
    let value = v8::BigInt::new_from_u64(scope, local_window_id);
    set_private_value(
        scope,
        object,
        LOCATION_RELEVANT_LOCAL_WINDOW_ID_SLOT,
        value.into(),
    );
}

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
