//! Weak JS observers of native font resources. No font bytes or load progress
//! live in JS slots; collecting a wrapper releases only that observer.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::{
    font_loading::{FontFaceLoad, FontFaceStatus},
    util::{get_private_value, set_private_value},
};

const STATE_SLOT: &str = "__moliFontFaceState";
type Store = Rc<RefCell<Observers>>;

#[derive(Default)]
struct Observers {
    next_id: u64,
    entries: HashMap<u64, Observer>,
}

struct Observer {
    wrapper: v8::Weak<v8::Object>,
    load: FontFaceLoad,
    published: FontFaceStatus,
}

pub(super) fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    load: FontFaceLoad,
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
            .expect("FontFace observer identity exhausted");
        store.next_id
    };
    let weak_store = Rc::downgrade(&store);
    let wrapper = v8::Weak::with_finalizer(
        scope,
        face,
        Box::new(move |_| {
            if let Some(store) = weak_store.upgrade() {
                store.borrow_mut().entries.remove(&id);
            }
        }),
    );
    store.borrow_mut().entries.insert(
        id,
        Observer {
            wrapper,
            load,
            published: FontFaceStatus::Unloaded,
        },
    );
    set_private_value(
        scope,
        face,
        STATE_SLOT,
        v8::BigInt::new_from_u64(scope, id).into(),
    );
}

fn id<'s>(scope: &mut v8::PinScope<'s, '_>, face: v8::Local<'s, v8::Object>) -> Option<u64> {
    get_private_value(scope, face, STATE_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .map(|value| value.u64_value().0)
}

pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> Option<FontFaceLoad> {
    let id = id(scope, face)?;
    Some(
        scope
            .get_slot::<Store>()?
            .borrow()
            .entries
            .get(&id)?
            .load
            .clone(),
    )
}

pub(super) fn bind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
    load: FontFaceLoad,
) -> bool {
    let Some(id) = id(scope, face) else {
        return false;
    };
    let store = scope
        .get_slot::<Store>()
        .expect("FontFace observers installed");
    let mut store = store.borrow_mut();
    let observer = store.entries.get_mut(&id).expect("live FontFace observer");
    if !Rc::ptr_eq(&observer.load, &load) {
        observer.load = load;
        observer.published = FontFaceStatus::Unloaded;
    }
    true
}

pub(super) fn take_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    face: v8::Local<'s, v8::Object>,
) -> Option<(FontFaceStatus, FontFaceLoad)> {
    let id = id(scope, face)?;
    let mut store = scope.get_slot::<Store>()?.borrow_mut();
    let observer = store.entries.get_mut(&id)?;
    let current = observer.load.borrow().status();
    if current == observer.published {
        return None;
    }
    let previous = std::mem::replace(&mut observer.published, current);
    Some((previous, observer.load.clone()))
}

pub(super) fn changed_wrappers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(store) = scope.get_slot::<Store>() else {
        return Vec::new();
    };
    store
        .borrow()
        .entries
        .values()
        .filter(|observer| observer.published != observer.load.borrow().status())
        .filter_map(|observer| observer.wrapper.to_local(scope))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font_loading::FontFaceResource;

    #[test]
    fn font_face_observer_gc_preserves_css_owned_state_but_releases_js_only_state() {
        // The platform is process-global: do not let this small isolate install
        // V8's default platform before renderer tests need Moli's task routing.
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let css_load = FontFaceResource::new(
            "css".into(),
            moli_layout::WebFontFace::new("CSS"),
            Vec::new(),
        );
        let js_load = {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let css_face = v8::Object::new(scope);
            initialize(scope, css_face, css_load.clone());
            let js_face = v8::Object::new(scope);
            initialize(
                scope,
                js_face,
                FontFaceResource::new("js".into(), moli_layout::WebFontFace::new("JS"), Vec::new()),
            );
            Rc::downgrade(&get(scope, js_face).unwrap())
        };
        isolate.low_memory_notification();
        assert!(
            isolate
                .get_slot::<Store>()
                .unwrap()
                .borrow()
                .entries
                .is_empty()
        );
        assert!(js_load.upgrade().is_none());
        assert_eq!(Rc::strong_count(&css_load), 1);
    }
}
