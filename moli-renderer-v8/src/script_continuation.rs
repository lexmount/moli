//! Data V8 carries across Promise reactions and async continuations. Popup
//! execution owners share a concrete realm, so the realm alone is insufficient.
use crate::{
    native_bridge::{
        RuntimeObservableContextToken, WindowExecutionContextIdentity, WindowSecurityOrigin,
    },
    util::{get_private_value, set_private_value},
};

const BASE_URL_SLOT: &str = "moli.scriptContinuationBaseURL";
const WINDOW_SLOT: &str = "moli.scriptContinuationWindow";
const WINDOW_STATE_SLOT: &str = "moli.scriptContinuationWindowState";

#[derive(Clone)]
pub(crate) struct WindowContinuation {
    pub(crate) identity: WindowExecutionContextIdentity,
    pub(crate) origin: WindowSecurityOrigin,
}

pub(crate) fn base_url_value<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    let data = scope.get_continuation_preserved_embedder_data();
    if let Ok(object) = v8::Local::<v8::Object>::try_from(data)
        && get_private_value(scope, object, WINDOW_SLOT).is_some()
    {
        // Undefined is a valid base URL payload, not the absence of the
        // surrounding continuation record.
        return get_private_value(scope, object, BASE_URL_SLOT)
            .unwrap_or_else(|| v8::undefined(scope).into());
    }
    data
}

fn window_anchor<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
    let data =
        v8::Local::<v8::Object>::try_from(scope.get_continuation_preserved_embedder_data()).ok()?;
    v8::Local::<v8::Object>::try_from(get_private_value(scope, data, WINDOW_SLOT)?).ok()
}

fn window(scope: &mut v8::PinScope<'_, '_>) -> Option<WindowContinuation> {
    let anchor = window_anchor(scope)?;
    let value = get_private_value(scope, anchor, WINDOW_STATE_SLOT)?;
    let external = v8::Local::<v8::External>::try_from(value).ok()?;
    // The private native anchor owns this allocation through its finalizer.
    Some(unsafe { &*external.value().cast::<WindowContinuation>() }.clone())
}

pub(crate) fn running_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Context>,
) -> Option<WindowContinuation> {
    if context != scope.get_current_context()
        || !context
            .get_microtask_queue()
            .is_some_and(v8::MicrotaskQueue::is_running_microtasks)
    {
        return None;
    }
    let owner = window(scope)?;
    let token = context.get_slot::<RuntimeObservableContextToken>()?;
    (owner.identity.realm_token() == *token).then_some(owner)
}

fn data_with_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    base_url: v8::Local<'s, v8::Value>,
    anchor: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    let data = v8::Object::new(scope);
    set_private_value(scope, data, BASE_URL_SLOT, base_url);
    set_private_value(scope, data, WINDOW_SLOT, anchor.into());
    data.into()
}

pub(crate) fn with_base_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    base_url: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    match window_anchor(scope) {
        Some(anchor) => data_with_window(scope, base_url, anchor),
        None => base_url,
    }
}

pub(crate) fn with_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: WindowContinuation,
) -> v8::Local<'s, v8::Value> {
    if window(scope).is_some_and(|current| current.identity == owner.identity) {
        return scope.get_continuation_preserved_embedder_data();
    }
    let base_url = base_url_value(scope);
    let anchor = v8::Object::new(scope);
    let mut owner = Box::new(owner);
    let pointer = (&mut *owner as *mut WindowContinuation).cast();
    let external = v8::External::new(scope, pointer);
    set_private_value(scope, anchor, WINDOW_STATE_SLOT, external.into());
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, anchor, move || drop(owner));
    data_with_window(scope, base_url, anchor)
}
