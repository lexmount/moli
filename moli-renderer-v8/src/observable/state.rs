use std::{cell::RefCell, collections::HashMap, rc::Rc};

use moli_webapi_declare::WebApiObject;

use super::callbacks;
use crate::{
    abort_signal_route::ResolvedAbortSignal,
    context_bootstrap::ensure_intrinsic_interface_prototype,
    util::{get_private_value, set_private_value},
    web_api_interfaces, webidl,
};

const ID: &str = "__moliObservableId";
pub(super) const INITIALIZER: &str = "__moliObservableInitializer";
pub(super) const ACTIVE: &str = "__moliSubscriberActive";
pub(super) const SIGNAL: &str = "__moliSubscriberSignal";
pub(super) const OBSERVERS: &str = "__moliSubscriberObservers";
pub(super) const TEARDOWNS: &str = "__moliSubscriberTeardowns";
pub(super) const NEXT: &str = "__moliObserverNext";
pub(super) const ERROR: &str = "__moliObserverError";
pub(super) const COMPLETE: &str = "__moliObserverComplete";
pub(super) const INPUT_SIGNAL: &str = "__moliObserverInputSignal";
pub(super) const ABORT_ALGORITHM: &str = "__moliObserverAbortAlgorithm";

type Store = Rc<RefCell<WeakSubscribers>>;

#[derive(Default)]
struct WeakSubscribers {
    next_id: u64,
    entries: HashMap<u64, Entry>,
}

struct Entry {
    _observable: v8::Weak<v8::Object>,
    subscriber: Option<v8::Weak<v8::Object>>,
    event_source: Option<super::event_target::EventSource>,
}

pub(super) fn initialize_observable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    callback: webidl::WebIdlCallbackFunction,
) {
    set_callback(scope, object, INITIALIZER, callback);
    register_observable(scope, object, None);
}

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::Observable)]
struct ObservableInstance<'scope> {
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,
}

pub(super) fn new_event_observable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: super::event_target::EventSource,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = ensure_intrinsic_interface_prototype(scope, "Observable").ok()?;
    let object = ObservableInstance::new(prototype).bind(scope).ok()?;
    register_observable(scope, object, Some(source));
    Some(object)
}

pub(super) fn event_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observable: v8::Local<'s, v8::Object>,
) -> Option<super::event_target::PreparedEventSource<'s>> {
    let id = id(scope, observable)?;
    scope
        .get_slot::<Store>()?
        .borrow()
        .entries
        .get(&id)?
        .event_source
        .as_ref()?
        .prepare(scope)
}

fn register_observable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    event_source: Option<super::event_target::EventSource>,
) {
    let store = if let Some(store) = scope.get_slot::<Store>() {
        store.clone()
    } else {
        let store = Store::default();
        scope.set_slot(store.clone());
        store
    };
    let id = {
        let mut store = store.borrow_mut();
        store.next_id = store
            .next_id
            .checked_add(1)
            .expect("Observable identity exhausted");
        store.next_id
    };
    let value = v8::BigInt::new_from_u64(scope, id);
    set_private_value(scope, object, ID, value.into());
    let weak_store = Rc::downgrade(&store);
    let weak = v8::Weak::with_finalizer(
        scope,
        object,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().entries.remove(&id);
            }
        }),
    );
    store.borrow_mut().entries.insert(
        id,
        Entry {
            _observable: weak,
            subscriber: None,
            event_source,
        },
    );
}

fn id<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>) -> Option<u64> {
    get_private_value(scope, object, ID)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value().0)
}

pub(super) fn current_subscriber<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observable: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let id = id(scope, observable)?;
    scope
        .get_slot::<Store>()?
        .borrow()
        .entries
        .get(&id)?
        .subscriber
        .as_ref()?
        .to_local(scope)
}

pub(super) fn set_subscriber<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observable: v8::Local<'s, v8::Object>,
    subscriber: v8::Local<'s, v8::Object>,
) {
    let id = id(scope, observable).expect("Observable identity");
    let weak = v8::Weak::new(scope, subscriber);
    scope
        .get_slot::<Store>()
        .unwrap()
        .borrow_mut()
        .entries
        .get_mut(&id)
        .unwrap()
        .subscriber = Some(weak);
}

#[derive(WebApiObject)]
#[webapi(prototype = "Object", interface = web_api_interfaces::Subscriber)]
struct SubscriberInstance<'scope> {
    #[webapi(prototype)]
    prototype: v8::Local<'scope, v8::Object>,
    #[webapi(slot = ACTIVE)]
    active: bool,
    #[webapi(slot = SIGNAL)]
    signal: v8::Local<'scope, v8::Object>,
    #[webapi(slot = OBSERVERS)]
    observers: v8::Local<'scope, v8::Array>,
    #[webapi(slot = TEARDOWNS)]
    teardowns: v8::Local<'scope, v8::Array>,
}

pub(super) fn new_subscriber<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = ensure_intrinsic_interface_prototype(scope, "Subscriber").ok()?;
    let signal = ResolvedAbortSignal::new(scope)?.value();
    SubscriberInstance::new(
        prototype,
        true,
        signal,
        v8::Array::new(scope, 0),
        v8::Array::new(scope, 0),
    )
    .bind(scope)
    .ok()
}

pub(super) fn object_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    subscriber: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, subscriber, ACTIVE).is_some_and(|value| value.is_true())
}

pub(super) fn list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(array) = object_slot(scope, object, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return Vec::new();
    };
    (0..array.length())
        .filter_map(|i| {
            array
                .get_index(scope, i)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        })
        .collect()
}

pub(super) fn set_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
    values: &[v8::Local<'s, v8::Object>],
) {
    let values: Vec<v8::Local<'s, v8::Value>> =
        values.iter().map(|value| (*value).into()).collect();
    let array = v8::Array::new_with_elements(scope, &values);
    set_private_value(scope, object, slot, array.into());
}

pub(super) fn set_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
    callback: webidl::WebIdlCallbackFunction,
) {
    let callback = callbacks::trace(scope, callback);
    set_private_value(scope, object, slot, callback.into());
}
