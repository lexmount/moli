//! Immutable resource attribution carried with classic-script continuations.
//! These private values contain strings, never callback or Window authority.
use crate::util::{get_private_value, set_private_value, v8_string};
use moli_url::WebOrigin;
use url::Url;

const BASE: &str = "moli.resourceInitiatorBaseURL";
const ANCHOR: &str = "moli.resourceInitiator";
const SOURCE: &str = "moli.resourceInitiatorSource";
const ORIGIN: &str = "moli.resourceInitiatorOrigin";

pub(crate) fn anchor<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
    let data =
        v8::Local::<v8::Object>::try_from(scope.get_continuation_preserved_embedder_data()).ok()?;
    v8::Local::<v8::Object>::try_from(get_private_value(scope, data, ANCHOR)?).ok()
}

pub(crate) fn base_url_value<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    let data = scope.get_continuation_preserved_embedder_data();
    if anchor(scope).is_some()
        && let Ok(data) = v8::Local::<v8::Object>::try_from(data)
    {
        return get_private_value(scope, data, BASE).unwrap_or_else(|| v8::undefined(scope).into());
    }
    data
}

fn data_with_anchor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    base: v8::Local<'s, v8::Value>,
    anchor: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    let data = v8::Object::new(scope);
    set_private_value(scope, data, BASE, base);
    set_private_value(scope, data, ANCHOR, anchor.into());
    data.into()
}

pub(crate) fn with_base_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    base: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    match anchor(scope) {
        Some(anchor) => data_with_anchor(scope, base, anchor),
        None => base,
    }
}

pub(crate) fn classic_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: Option<&Url>,
) -> v8::Local<'s, v8::Value> {
    let origin = crate::util::context_host_ptr_from_global_bridge(scope).and_then(|host| {
        let host = unsafe { &*host };
        let identity =
            host.window_execution_context_identity_for_access_check(scope.get_current_context())?;
        let owner = identity.dispatch_scope();
        let loader = host.document_resource_loader_for_dispatch_scope(owner)?;
        Some(
            host.subresource_request_environment(&loader, owner)?
                .request_origin,
        )
    });
    // A cross-origin classic without retained fetch metadata is explicitly
    // unknown. Its source must not become visible through a same-origin XHR.
    let source = source.filter(|source| {
        origin
            .as_ref()
            .is_some_and(|origin| origin.same_origin_url(source))
    });
    let base = base_url_value(scope);
    let anchor = v8::Object::new(scope);
    if let Some(value) = v8_string(scope, source.map(Url::as_str).unwrap_or("")) {
        set_private_value(scope, anchor, SOURCE, value.into());
    }
    if let Some(value) = v8_string(
        scope,
        origin
            .as_ref()
            .map(WebOrigin::ascii_serialization)
            .unwrap_or(""),
    ) {
        set_private_value(scope, anchor, ORIGIN, value.into());
    }
    data_with_anchor(scope, base, anchor)
}

pub(crate) fn current_url(scope: &mut v8::PinScope<'_, '_>, origin: &WebOrigin) -> String {
    if origin.is_opaque() {
        return String::new();
    }
    let Some(anchor) = anchor(scope) else {
        return String::new();
    };
    let captured_origin = get_private_value(scope, anchor, ORIGIN)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
        .map(|value| value.to_rust_string_lossy(scope));
    if captured_origin.as_deref() != Some(origin.ascii_serialization()) {
        return String::new();
    }
    get_private_value(scope, anchor, SOURCE)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default()
}

pub(crate) fn response_url(
    source: String,
    origin: &WebOrigin,
    final_url: &Url,
    cache: moli_fetch::ResponseCacheState,
) -> String {
    if cache == moli_fetch::ResponseCacheState::None && origin.same_origin_url(final_url) {
        source
    } else {
        String::new()
    }
}
