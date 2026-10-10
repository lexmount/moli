use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::util::{get_private_value, set_private_value};

use super::model::Constraints;

const ID: &str = "__moliMediaTrackConstraintsIdentity";
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

type State = Rc<RefCell<Constraints>>;
type Store = Rc<RefCell<Objects>>;
#[derive(Default)]
struct Objects {
    entries: HashMap<u64, (v8::Weak<v8::Object>, State)>,
}

pub(in crate::context_bootstrap::media_streams) fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    track: v8::Local<'s, v8::Object>,
) {
    let store = if let Some(store) = scope.get_slot::<Store>() {
        store.clone()
    } else {
        let store = Store::default();
        scope.set_slot(store.clone());
        store
    };
    let id = NEXT_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            next.checked_add(1)
        })
        .expect("MediaStreamTrack constraints identity exhausted");
    set_private_value(scope, track, ID, v8::BigInt::new_from_u64(scope, id).into());
    let weak_store = Rc::downgrade(&store);
    let weak = v8::Weak::with_finalizer(
        scope,
        track,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().entries.remove(&id);
            }
        }),
    );
    store
        .borrow_mut()
        .entries
        .insert(id, (weak, State::default()));
}

pub(super) fn get<'s>(scope: &mut v8::PinScope<'s, '_>, track: v8::Local<'s, v8::Object>) -> State {
    let id = v8::Local::<v8::BigInt>::try_from(
        get_private_value(scope, track, ID).expect("native track constraint identity"),
    )
    .expect("native track constraint identity type")
    .u64_value()
    .0;
    scope
        .get_slot::<Store>()
        .expect("native track constraints store")
        .borrow()
        .entries
        .get(&id)
        .expect("native track constraints state")
        .1
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constraint_state_is_released_when_native_wrappers_are_collected() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let first = v8::Object::new(scope);
            let second = v8::Object::new(scope);
            initialize(scope, first);
            initialize(scope, second);
            assert!(Rc::ptr_eq(&get(scope, first), &get(scope, first)));
            assert!(!Rc::ptr_eq(&get(scope, first), &get(scope, second)));
            assert_eq!(scope.get_slot::<Store>().unwrap().borrow().entries.len(), 2);
        }
        isolate.low_memory_notification();
        assert!(
            isolate
                .get_slot::<Store>()
                .unwrap()
                .borrow()
                .entries
                .is_empty()
        );
    }
}
