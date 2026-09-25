//! Exact Window identity carried across Promise reactions without retaining a Realm.
use crate::native_bridge::{
    RuntimeObservableContextToken, WindowExecutionContextIdentity, WindowSecurityOrigin,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

#[derive(Clone)]
pub(crate) struct WindowContinuation {
    pub(crate) identity: WindowExecutionContextIdentity,
    pub(crate) origin: WindowSecurityOrigin,
}

struct ContinuationData {
    window: WindowContinuation,
    // Resource provenance remains a primitive string, including its native NUL fields.
    payload: Option<v8::Global<v8::String>>,
}

type Continuations = Rc<RefCell<HashMap<usize, Weak<ContinuationData>>>>;

fn data(scope: &mut v8::PinScope<'_, '_>) -> Option<Rc<ContinuationData>> {
    let external =
        v8::Local::<v8::External>::try_from(scope.get_continuation_preserved_embedder_data())
            .ok()?;
    // An External is only an opaque key. Validate it in this isolate's registry;
    // no author value or unrelated native pointer is ever dereferenced.
    let key = external.value() as usize;
    let registry = scope.get_slot::<Continuations>()?;
    registry.borrow().get(&key).and_then(Weak::upgrade)
}

pub(crate) fn base_url_value<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    match data(scope) {
        Some(data) => {
            if let Some(payload) = data.payload.as_ref() {
                v8::Local::new(scope, payload).into()
            } else {
                v8::undefined(scope).into()
            }
        }
        None => scope.get_continuation_preserved_embedder_data(),
    }
}

fn window(scope: &mut v8::PinScope<'_, '_>) -> Option<WindowContinuation> {
    data(scope).map(|data| data.window.clone())
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
    payload: v8::Local<'s, v8::Value>,
    window: WindowContinuation,
) -> v8::Local<'s, v8::Value> {
    let registry = if let Some(registry) = scope.get_slot::<Continuations>() {
        Rc::clone(registry)
    } else {
        let registry = Rc::new(RefCell::new(HashMap::new()));
        scope.set_slot(Rc::clone(&registry));
        registry
    };
    let data = Rc::new(ContinuationData {
        window,
        payload: v8::Local::<v8::String>::try_from(payload)
            .ok()
            .map(|payload| v8::Global::new(scope, payload)),
    });
    let key = Rc::as_ptr(&data) as usize;
    registry.borrow_mut().insert(key, Rc::downgrade(&data));
    let external = v8::External::new(scope, key as *mut std::ffi::c_void);
    crate::v8_finalizer::track_context_owned_external_finalizer(scope, external, move || {
        registry.borrow_mut().remove(&key);
        drop(data);
    });
    external.into()
}

pub(crate) fn with_base_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    match window(scope) {
        Some(window) => data_with_window(scope, payload, window),
        None => payload,
    }
}

pub(crate) fn with_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: WindowContinuation,
) -> v8::Local<'s, v8::Value> {
    if window(scope).is_some_and(|current| current.identity == owner.identity) {
        return scope.get_continuation_preserved_embedder_data();
    }
    let payload = base_url_value(scope);
    data_with_window(scope, payload, owner)
}
