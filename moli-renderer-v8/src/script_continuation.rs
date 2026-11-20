//! Native execution state carried across Promise reactions.
use crate::native_bridge::{
    RuntimeObservableContextToken, WindowExecutionContextIdentity, WindowSecurityOrigin,
};
use crate::scheduling::SchedulingState;
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
    window: Option<WindowContinuation>,
    scheduling: Option<SchedulingState>,
    // Resource provenance remains a primitive string, including its native NUL fields.
    payload: Option<v8::Global<v8::String>>,
}

type Continuations = Rc<RefCell<HashMap<usize, Weak<ContinuationData>>>>;

fn data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<(Rc<ContinuationData>, Option<v8::Local<'s, v8::Object>>)> {
    let value = scope.get_continuation_preserved_embedder_data();
    let carrier = v8::Local::<v8::Array>::try_from(value).ok();
    let key = carrier.map_or(Some(value), |carrier| carrier.get_index(scope, 0))?;
    let external = v8::Local::<v8::External>::try_from(key).ok()?;
    // An External is only an opaque key. Validate it in this isolate's registry;
    // no author value or unrelated native pointer is ever dereferenced.
    let key = external.value() as usize;
    let registry = scope.get_slot::<Continuations>()?;
    let data = registry.borrow().get(&key).and_then(Weak::upgrade)?;
    let signal = carrier
        .and_then(|carrier| carrier.get_index(scope, 1))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    Some((data, signal))
}

pub(crate) fn base_url_value<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    match data(scope) {
        Some((data, _)) => {
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
    data(scope)?.0.window.clone()
}

pub(crate) fn scheduling_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<(SchedulingState, Option<v8::Local<'s, v8::Object>>)> {
    let (data, signal) = data(scope)?;
    Some((data.scheduling.clone()?, signal))
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

fn continuation_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: v8::Local<'s, v8::Value>,
    window: Option<WindowContinuation>,
    scheduling: Option<SchedulingState>,
    signal: Option<v8::Local<'s, v8::Object>>,
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
        scheduling,
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
    // The signal belongs to the V8 object graph. A native strong handle here
    // could retain a signal, its listeners and the promise reaction itself.
    match signal {
        Some(signal) => {
            v8::Array::new_with_elements(scope, &[external.into(), signal.into()]).into()
        }
        None => external.into(),
    }
}

pub(crate) fn with_base_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    match data(scope) {
        Some((data, signal)) => continuation_data(
            scope,
            payload,
            data.window.clone(),
            data.scheduling.clone(),
            signal,
        ),
        None => payload,
    }
}

pub(crate) fn with_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: WindowContinuation,
) -> v8::Local<'s, v8::Value> {
    let previous = data(scope);
    if previous.as_ref().is_some_and(|(data, _)| {
        data.window
            .as_ref()
            .is_some_and(|current| current.identity == owner.identity)
    }) {
        return scope.get_continuation_preserved_embedder_data();
    }
    let (scheduling, signal) = previous.map_or((None, None), |(data, signal)| {
        (data.scheduling.clone(), signal)
    });
    let payload = base_url_value(scope);
    continuation_data(scope, payload, Some(owner), scheduling, signal)
}

pub(crate) fn with_scheduling_state<'s, R>(
    scope: &mut v8::PinScope<'s, '_>,
    state: SchedulingState,
    signal: Option<v8::Local<'s, v8::Object>>,
    run: impl FnOnce(&mut v8::PinScope<'s, '_>) -> R,
) -> R {
    let previous = scope.get_continuation_preserved_embedder_data();
    let payload = base_url_value(scope);
    let window = window(scope);
    let current = continuation_data(scope, payload, window, Some(state), signal);
    scope.set_continuation_preserved_embedder_data(current);
    let result = run(scope);
    scope.set_continuation_preserved_embedder_data(previous);
    result
}
