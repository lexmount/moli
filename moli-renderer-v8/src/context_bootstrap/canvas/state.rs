//! Native path state belongs to the V8 context object, never to the process.
//!
//! The isolate owns the table. Weak context handles remove entries on GC;
//! dropping the isolate also drops every remaining entry without needing GC.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use super::path::Canvas2dPathState;
use crate::util::{get_private_value, set_private_value};

const PATH_STATE_SLOT: &str = "__moliCanvasPathState";
type StateStore = Rc<RefCell<CanvasStates>>;

#[derive(Default)]
struct CanvasStates {
    next_id: u64,
    entries: HashMap<u64, CanvasStateEntry>,
}

struct CanvasStateEntry {
    _context: v8::Weak<v8::Object>,
    state: Rc<RefCell<Canvas2dPathState>>,
}

pub(super) fn canvas_path_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> Rc<RefCell<Canvas2dPathState>> {
    let store = if let Some(store) = scope.get_slot::<StateStore>() {
        store.clone()
    } else {
        let store = StateStore::default();
        scope.set_slot(store.clone());
        store
    };
    if let Some(id) = get_private_value(scope, context, PATH_STATE_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value().0)
    {
        return store
            .borrow()
            .entries
            .get(&id)
            .expect("a live context must retain its native path state")
            .state
            .clone();
    }
    let id = {
        let mut store = store.borrow_mut();
        store.next_id = store
            .next_id
            .checked_add(1)
            .expect("canvas state identity exhausted");
        store.next_id
    };
    let weak_store = Rc::downgrade(&store);
    let owner = v8::Weak::with_finalizer(
        scope,
        context,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().entries.remove(&id);
            }
        }),
    );
    let state = Rc::new(RefCell::new(Canvas2dPathState::default()));
    store.borrow_mut().entries.insert(
        id,
        CanvasStateEntry {
            _context: owner,
            state: state.clone(),
        },
    );
    let id = v8::BigInt::new_from_u64(scope, id);
    set_private_value(scope, context, PATH_STATE_SLOT, id.into());
    state
}

pub(super) fn reset_canvas_path_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) {
    if get_private_value(scope, context, PATH_STATE_SLOT).is_some() {
        *canvas_path_state(scope, context).borrow_mut() = Canvas2dPathState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_state_is_reclaimed_with_context_gc_and_isolate_destruction() {
        moli_v8_test_util::ensure_v8();
        let state = {
            let mut isolate = v8::Isolate::new(Default::default());
            let state = {
                let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
                let scope = &mut scope.init();
                let context = v8::Context::new(scope, Default::default());
                let scope = &mut v8::ContextScope::new(scope, context);
                let object = v8::Object::new(scope);
                let state = canvas_path_state(scope, object);
                state.borrow_mut().rect(0.0, 0.0, 10.0, 10.0);
                assert!(Rc::ptr_eq(&state, &canvas_path_state(scope, object)));
                Rc::downgrade(&state)
            };
            isolate.low_memory_notification();
            assert!(state.upgrade().is_none(), "GC must drop native path data");
            assert!(
                isolate
                    .get_slot::<StateStore>()
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
            Rc::downgrade(&canvas_path_state(scope, object))
        };
        assert!(
            state.upgrade().is_none(),
            "isolate teardown must not depend on GC"
        );
    }
}
