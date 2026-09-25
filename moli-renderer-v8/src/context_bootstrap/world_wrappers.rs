//! Weak wrapper identities for platform objects exposed by Navigation events.
//! A retained native object must not keep another world's realm alive merely
//! because that world once observed it.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::native_bridge::identity::contexts_share_wrapper_world;
use crate::util::{get_private_object, get_private_value, set_private_value};

const ID: &str = "__moliWorldWrapperIdentity";
const OWNER: &str = "__moliWorldWrapperOwner";
type Store = Rc<RefCell<Wrappers>>;
type WeakWrapper = (u64, Rc<v8::Weak<v8::Object>>);

#[derive(Default)]
struct Wrappers {
    next_id: u64,
    objects: HashMap<u64, Vec<WeakWrapper>>,
}

fn store(scope: &mut v8::PinScope<'_, '_>) -> Store {
    if let Some(store) = scope.get_slot::<Store>() {
        return store.clone();
    }
    let store = Store::default();
    scope.set_slot(store.clone());
    store
}

pub(super) fn owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    get_private_object(scope, object, OWNER).unwrap_or(object)
}

pub(super) fn belongs_to_world<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Context>,
) -> bool {
    object
        .get_creation_context(scope)
        .is_some_and(|creation| contexts_share_wrapper_world(creation, context))
}

fn identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    get_private_value(scope, object, ID)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|id| id.u64_value().0)
}

pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    context: v8::Local<'s, v8::Context>,
) -> Option<v8::Local<'s, v8::Object>> {
    let id = identity(scope, object)?;
    // Do not hold a RefCell borrow across V8 operations/weak finalizers.
    let candidates = store(scope).borrow().objects.get(&id)?.clone();
    candidates.into_iter().find_map(|(_, candidate)| {
        let wrapper = candidate.to_local(scope)?;
        belongs_to_world(scope, wrapper, context).then_some(wrapper)
    })
}

pub(super) fn insert<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    wrapper: v8::Local<'s, v8::Object>,
) {
    let store = store(scope);
    let wrapper_id = {
        let mut records = store.borrow_mut();
        records.next_id = records
            .next_id
            .checked_add(1)
            .expect("wrapper id exhausted");
        records.next_id
    };
    let id = identity(scope, object).unwrap_or_else(|| {
        set_private_value(
            scope,
            object,
            ID,
            v8::BigInt::new_from_u64(scope, wrapper_id).into(),
        );
        wrapper_id
    });
    if !object.strict_equals(wrapper.into()) {
        set_private_value(scope, wrapper, OWNER, object.into());
    }
    let weak_store = Rc::downgrade(&store);
    let weak = v8::Weak::with_finalizer(
        scope,
        wrapper,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                let mut records = store.borrow_mut();
                if let Some(wrappers) = records.objects.get_mut(&id) {
                    wrappers.retain(|(candidate, _)| *candidate != wrapper_id);
                    if wrappers.is_empty() {
                        records.objects.remove(&id);
                    }
                }
            }
        }),
    );
    store
        .borrow_mut()
        .objects
        .entry(id)
        .or_default()
        .push((wrapper_id, Rc::new(weak)));
}
