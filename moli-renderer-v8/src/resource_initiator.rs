//! Immutable resource attribution carried with classic-script continuations.
//! Only primitive strings may survive with a reaction or registration: even a
//! prototype-free JS object can retain its creating Realm through its V8 map.
use crate::util::{v8_string, v8str};
use moli_url::WebOrigin;
use url::Url;

// Serialized URLs and origins cannot contain a literal NUL. The continuation
// is base\0source\0origin; a registration keeps only source\0origin. A legacy
// base-only continuation has no separator, while an empty anchor is unknown.
const SEPARATOR: char = '\0';

pub(crate) fn anchor<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::String>> {
    let data =
        v8::Local::<v8::String>::try_from(scope.get_continuation_preserved_embedder_data()).ok()?;
    let data = data.to_rust_string_lossy(scope);
    let (_, anchor) = data.split_once(SEPARATOR)?;
    v8_string(scope, anchor)
}

pub(crate) fn base_url_value<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    let data = scope.get_continuation_preserved_embedder_data();
    if let Ok(value) = v8::Local::<v8::String>::try_from(data)
        && let Some((base, _)) = value.to_rust_string_lossy(scope).split_once(SEPARATOR)
    {
        return v8_string(scope, base)
            .filter(|_| !base.is_empty())
            .map_or_else(|| v8::undefined(scope).into(), Into::into);
    }
    data
}

fn data_with_anchor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    base: v8::Local<'s, v8::Value>,
    anchor: v8::Local<'s, v8::String>,
) -> v8::Local<'s, v8::Value> {
    let base = v8::Local::<v8::String>::try_from(base).unwrap_or_else(|_| v8::String::empty(scope));
    let base = v8::String::concat(scope, base, v8str(scope, "\0"))
        .expect("continuation base URL must fit in a V8 string");
    v8::String::concat(scope, base, anchor)
        .expect("continuation URLs must fit in a V8 string")
        .into()
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
    let anchor = v8_string(
        scope,
        &format!(
            "{}\0{}",
            source.map(Url::as_str).unwrap_or(""),
            origin
                .as_ref()
                .map(WebOrigin::ascii_serialization)
                .unwrap_or(""),
        ),
    )
    .expect("resource initiator URLs must fit in a V8 string");
    data_with_anchor(scope, base, anchor)
}

pub(crate) fn current_url(scope: &mut v8::PinScope<'_, '_>, origin: &WebOrigin) -> String {
    if origin.is_opaque() {
        return String::new();
    }
    let Some(anchor) = anchor(scope) else {
        return String::new();
    };
    let anchor = anchor.to_rust_string_lossy(scope);
    let Some((source, captured_origin)) = anchor.split_once(SEPARATOR) else {
        return String::new();
    };
    if captured_origin == origin.ascii_serialization() {
        source.to_owned()
    } else {
        String::new()
    }
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

/// One registration's immutable attribution, independent of later registrations
/// of the same callback. The anchor holds no callback or execution authority.
#[derive(Clone)]
pub(crate) struct CallbackInitiator(std::rc::Rc<v8::Global<v8::String>>);

impl CallbackInitiator {
    pub(crate) fn capture(scope: &mut v8::PinScope<'_, '_>) -> Self {
        let anchor = anchor(scope).unwrap_or_else(|| v8::String::empty(scope));
        Self(std::rc::Rc::new(v8::Global::new(scope, anchor)))
    }

    pub(crate) fn value<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
        self.local(scope).into()
    }

    pub(crate) fn from_value<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
    ) -> Self {
        let anchor =
            v8::Local::<v8::String>::try_from(value).unwrap_or_else(|_| v8::String::empty(scope));
        Self(std::rc::Rc::new(v8::Global::new(scope, anchor)))
    }

    fn local<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::String> {
        v8::Local::new(scope, self.0.as_ref())
    }
}

/// Choose before entering a callback Realm. An active caller, including an
/// explicit unknown caller, precedes a callback's registration provenance.
pub(crate) fn callback_anchor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registration: Option<&CallbackInitiator>,
) -> Option<v8::Local<'s, v8::String>> {
    let Some(registration) = registration else {
        return anchor(scope);
    };
    let Some(stack) = v8::StackTrace::current_stack_trace(scope, 1) else {
        return Some(v8::String::empty(scope));
    };
    // A native microtask can inherit its trigger's continuation data. Its
    // callback uses registration provenance unless author JS is on the stack.
    if stack.get_frame_count() == 0 {
        return Some(registration.local(scope));
    }
    Some(anchor(scope).unwrap_or_else(|| v8::String::empty(scope)))
}

pub(crate) fn with_callback_anchor<'s, R>(
    scope: &mut v8::PinScope<'s, '_>,
    anchor: Option<v8::Local<'s, v8::String>>,
    invoke: impl FnOnce(&mut v8::PinScope<'s, '_>) -> R,
) -> R {
    let Some(anchor) = anchor else {
        return invoke(scope);
    };
    let previous = scope.get_continuation_preserved_embedder_data();
    let base = base_url_value(scope);
    let data = data_with_anchor(scope, base, anchor);
    scope.set_continuation_preserved_embedder_data(data);
    let result = invoke(scope);
    scope.set_continuation_preserved_embedder_data(previous);
    result
}
