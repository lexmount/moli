use crate::util::{v8_string, v8str};
use moli_webapi_declare::WebApiObject;
use url::Url;

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct DetachedWindowMessageEventInitDeclaration<'scope> {
    data: v8::Local<'scope, v8::Value>,
    source: v8::Local<'scope, v8::Value>,
    origin: v8::Local<'scope, v8::String>,
    ports: v8::Local<'scope, v8::Array>,
}

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
    let global = scope.get_current_context().global(scope);
    let message_event_constructor = global
        .get(scope, v8str(scope, "MessageEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let origin = v8_string(scope, origin)?;
    let init = DetachedWindowMessageEventInitDeclaration::new(data, source, origin, ports)
        .bind(scope)
        .ok()?;
    let event_type = v8_string(scope, event_type)?;
    message_event_constructor.new_instance(scope, &[event_type.into(), init.into()])
}
