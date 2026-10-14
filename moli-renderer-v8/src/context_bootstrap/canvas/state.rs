//! Native Canvas state belongs to the V8 context object, never to the process.
//!
//! The isolate owns the table. Weak context handles remove entries on GC;
//! dropping the isolate also drops every remaining entry without needing GC.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use super::{clipping::CanvasClipRegion, path::Canvas2dPathState};
use crate::util::{get_private_value, set_private_value};

const NATIVE_STATE_SLOT: &str = "__moliCanvasNativeState";
type StateStore = Rc<RefCell<CanvasStates>>;

#[derive(Default)]
struct CanvasStates {
    next_id: u64,
    entries: HashMap<u64, CanvasStateEntry>,
}

struct CanvasStateEntry {
    _context: v8::Weak<v8::Object>,
    state: Rc<RefCell<NativeCanvasState>>,
}

fn canvas_native_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> Rc<RefCell<NativeCanvasState>> {
    // Registered native proxies share the target's private identity. Saved
    // drawing-state objects are plain internal objects and keep their own key.
    let context = moli_webapi_declare::web_api_object_target(scope, context).unwrap_or(context);
    let store = if let Some(store) = scope.get_slot::<StateStore>() {
        store.clone()
    } else {
        let store = StateStore::default();
        scope.set_slot(store.clone());
        store
    };
    if let Some(id) = get_private_value(scope, context, NATIVE_STATE_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value().0)
    {
        return store
            .borrow()
            .entries
            .get(&id)
            .expect("a live object must retain its native Canvas state")
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
    let state = Rc::new(RefCell::new(NativeCanvasState::default()));
    store.borrow_mut().entries.insert(
        id,
        CanvasStateEntry {
            _context: owner,
            state: state.clone(),
        },
    );
    let id = v8::BigInt::new_from_u64(scope, id);
    set_private_value(scope, context, NATIVE_STATE_SLOT, id.into());
    state
}

#[derive(Default)]
struct NativeCanvasState {
    // Path2D, contexts and saved drawing-state objects share lifetime tracking,
    // but allocate only the native components they actually use.
    path: Option<Rc<RefCell<Canvas2dPathState>>>,
    clipping: Option<Rc<RefCell<CanvasClipRegion>>>,
    gradient: Option<Rc<RefCell<super::gradient::CanvasGradientData>>>,
    pattern: Option<Rc<RefCell<super::pattern::CanvasPatternData>>>,
}

pub(super) fn initialize_canvas_pattern_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    pattern: super::pattern::CanvasPatternData,
) {
    canvas_native_state(scope, object).borrow_mut().pattern = Some(Rc::new(RefCell::new(pattern)));
}

pub(super) fn canvas_pattern_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Rc<RefCell<super::pattern::CanvasPatternData>> {
    canvas_native_state(scope, object)
        .borrow()
        .pattern
        .as_ref()
        .expect("CanvasPattern native image is initialized at creation")
        .clone()
}

pub(super) fn canvas_gradient_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Rc<RefCell<super::gradient::CanvasGradientData>> {
    canvas_native_state(scope, object)
        .borrow_mut()
        .gradient
        .get_or_insert_with(|| {
            Rc::new(RefCell::new(super::gradient::CanvasGradientData::default()))
        })
        .clone()
}

pub(super) fn canvas_path_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Rc<RefCell<Canvas2dPathState>> {
    canvas_native_state(scope, object)
        .borrow_mut()
        .path
        .get_or_insert_with(|| Rc::new(RefCell::new(Canvas2dPathState::default())))
        .clone()
}

pub(super) fn canvas_clipping_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Rc<RefCell<CanvasClipRegion>> {
    canvas_native_state(scope, object)
        .borrow_mut()
        .clipping
        .get_or_insert_with(|| Rc::new(RefCell::new(CanvasClipRegion::default())))
        .clone()
}

pub(super) fn reset_canvas_native_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    if get_private_value(scope, object, NATIVE_STATE_SLOT).is_some() {
        let state = canvas_native_state(scope, object);
        let state = state.borrow();
        if let Some(path) = &state.path {
            *path.borrow_mut() = Canvas2dPathState::default();
        }
        if let Some(clipping) = &state.clipping {
            *clipping.borrow_mut() = CanvasClipRegion::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_clips_survive_owner_gc_and_native_state_is_reclaimed() {
        moli_v8_test_util::ensure_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let (context, original, saved) = {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let owner = v8::Object::new(scope);
            let snapshot = crate::util::new_null_prototype_object(scope);
            let region = canvas_clipping_state(scope, owner);
            let mut path = Canvas2dPathState::default();
            path.rect(0.0, 0.0, 2.0, 4.0);
            region
                .borrow_mut()
                .intersect(path, super::super::path2d::CanvasFillRule::Nonzero)
                .unwrap();
            let captured = region.borrow().clone();
            let retained = canvas_clipping_state(scope, snapshot);
            *retained.borrow_mut() = captured;
            let key = crate::util::v8str(scope, "retainedClip");
            context
                .global(scope)
                .create_data_property(scope, key.into(), snapshot.into())
                .unwrap();
            (
                v8::Global::new(scope, context),
                Rc::downgrade(&region),
                Rc::downgrade(&retained),
            )
        };
        isolate.low_memory_notification();
        assert!(original.upgrade().is_none());
        assert!(saved.upgrade().is_some());
        {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let local = v8::Local::new(scope, &context);
            let scope = &mut v8::ContextScope::new(scope, local);
            let key = crate::util::v8str(scope, "retainedClip");
            let global = local.global(scope);
            let object =
                v8::Local::<v8::Object>::try_from(global.get(scope, key.into()).unwrap()).unwrap();
            let region = canvas_clipping_state(scope, object);
            let mask = region.borrow().mask(4, 4).unwrap().unwrap();
            assert_eq!(mask.rgba[3], 255);
            assert_eq!(mask.rgba[3 * 4 + 3], 0);
            let null = v8::null(scope);
            global
                .create_data_property(scope, key.into(), null.into())
                .unwrap();
        }
        isolate.low_memory_notification();
        assert!(saved.upgrade().is_none());
        assert!(
            isolate
                .get_slot::<StateStore>()
                .unwrap()
                .borrow()
                .entries
                .is_empty()
        );
    }

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
