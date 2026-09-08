//! The native codec follows the lifetime of the private transformer object.
//! Weak handles reclaim abandoned streams; isolate teardown also drops all
//! remaining codecs, without requiring a final GC.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use super::codec::Codec;
use crate::util::{get_private_value, set_private_value};

const STATE_SLOT: &str = "__moliCompressionState";
type Store = Rc<RefCell<States>>;
pub(super) type State = Rc<RefCell<Option<Codec>>>;

#[derive(Default)]
struct States {
    next_id: u64,
    entries: HashMap<u64, (v8::Weak<v8::Object>, State)>,
}

pub(super) fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    codec: Codec,
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
            .expect("compression identity exhausted");
        store.next_id
    };
    let weak_store = Rc::downgrade(&store);
    let weak = v8::Weak::with_finalizer(
        scope,
        owner,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().entries.remove(&id);
            }
        }),
    );
    store
        .borrow_mut()
        .entries
        .insert(id, (weak, Rc::new(RefCell::new(Some(codec)))));
    set_private_value(
        scope,
        owner,
        STATE_SLOT,
        v8::BigInt::new_from_u64(scope, id).into(),
    );
}

pub(super) fn get<'s>(scope: &mut v8::PinScope<'s, '_>, owner: v8::Local<'s, v8::Object>) -> State {
    let id = get_private_value(scope, owner, STATE_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .expect("compression transformer must retain its identity")
        .u64_value()
        .0;
    scope
        .get_slot::<Store>()
        .expect("compression store must exist")
        .borrow()
        .entries
        .get(&id)
        .expect("live transformer must retain codec state")
        .1
        .clone()
}

#[cfg(test)]
mod tests {
    use super::super::codec::Format;
    use super::*;

    #[test]
    fn abandoned_codecs_are_reclaimed_on_gc_and_isolate_drop() {
        for format in [Format::Gzip, Format::Brotli] {
            assert_abandoned_codec_is_reclaimed(format);
        }
    }

    fn assert_abandoned_codec_is_reclaimed(format: Format) {
        moli_v8_test_util::ensure_v8();
        let state = {
            let mut isolate = v8::Isolate::new(Default::default());
            let state = {
                let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
                let scope = &mut scope.init();
                let context = v8::Context::new(scope, Default::default());
                let scope = &mut v8::ContextScope::new(scope, context);
                let object = v8::Object::new(scope);
                initialize(scope, object, Codec::new(format, false));
                Rc::downgrade(&get(scope, object))
            };
            isolate.low_memory_notification();
            assert!(state.upgrade().is_none());
            assert!(
                isolate
                    .get_slot::<Store>()
                    .unwrap()
                    .borrow()
                    .entries
                    .is_empty()
            );
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let object = v8::Object::new(scope);
            initialize(scope, object, Codec::new(format, true));
            Rc::downgrade(&get(scope, object))
        };
        assert!(state.upgrade().is_none());
    }
}
