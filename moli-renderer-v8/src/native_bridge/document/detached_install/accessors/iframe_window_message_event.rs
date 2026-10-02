use crate::util::v8str;
use url::Url;

pub(super) fn detached_window_origin(
    scope: &mut v8::PinScope<'_, '_>,
    window: v8::Local<'_, v8::Object>,
) -> Option<String> {
    let document = window
        .get(scope, v8str(scope, "document").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let url = document
        .get(scope, v8str(scope, "URL").into())
        .or_else(|| document.get(scope, v8str(scope, "baseURI").into()))
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .and_then(|value| Url::parse(&value).ok())?;
    Some(moli_url::origin_ascii_serialization(&url))
}

pub(super) fn detached_window_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    data: v8::Local<'s, v8::Value>,
    source: v8::Local<'s, v8::Value>,
    origin: &str,
    ports: v8::Local<'s, v8::Array>,
) -> Option<v8::Local<'s, v8::Object>> {
    crate::context_bootstrap::construct_original_message_event(
        scope,
        event_type,
        data,
        origin,
        &[],
        source,
        ports,
    )
}
