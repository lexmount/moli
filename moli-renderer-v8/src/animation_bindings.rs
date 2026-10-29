//! Realm wrappers are weak edges; CSS and the Document timeline own native effects.
use crate::{
    animation_model::Animation,
    common_runtime::CommonRuntimeState,
    util::{get_private_value, set_private_value},
};
use moli_animation::AnimationId;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

const INSTANCE: &str = "__moliAnimationInstance";
const BINDING: &str = "__moliAnimationBinding";

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum WrapperKind {
    Animation,
    Effect,
}

struct Binding {
    id: u64,
    instance: Animation,
    wrapper: v8::Weak<v8::Object>,
    realm: v8::Weak<v8::Context>,
    kind: WrapperKind,
}

pub(crate) struct AnimationBindings {
    pub(crate) origin: std::time::Instant,
    next_binding_id: Cell<u64>,
    entries: RefCell<HashMap<AnimationId, Vec<Binding>>>,
}

impl Default for AnimationBindings {
    fn default() -> Self {
        Self {
            origin: std::time::Instant::now(),
            next_binding_id: Cell::new(1),
            entries: RefCell::new(HashMap::new()),
        }
    }
}

impl AnimationBindings {
    pub(crate) fn wrapper<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        instance: &Animation,
        kind: WrapperKind,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let realm = scope.get_current_context();
        self.entries
            .borrow()
            .get(&instance.borrow().playback.id())?
            .iter()
            .find_map(|binding| {
                if binding.kind != kind {
                    return None;
                }
                let owner = binding.realm.to_local(scope)?;
                if owner != realm {
                    return None;
                }
                binding.wrapper.to_local(scope)
            })
    }
    pub(crate) fn wrappers<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        instance: &Animation,
    ) -> Vec<v8::Local<'s, v8::Object>> {
        self.entries
            .borrow()
            .get(&instance.borrow().playback.id())
            .into_iter()
            .flatten()
            .filter(|b| b.kind == WrapperKind::Animation)
            .filter_map(|b| b.wrapper.to_local(scope))
            .collect()
    }
}

pub(crate) fn bind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    instance: Animation,
    kind: WrapperKind,
) {
    let Some(context) = object.get_creation_context(scope) else {
        return;
    };
    let Some(common) = CommonRuntimeState::from_context(context) else {
        return;
    };
    let old_instance = get_private_value(scope, object, INSTANCE)
        .and_then(|v| v8::Local::<v8::BigInt>::try_from(v).ok())
        .map(|v| AnimationId(v.u64_value().0));
    let old_binding = get_private_value(scope, object, BINDING)
        .and_then(|v| v8::Local::<v8::BigInt>::try_from(v).ok())
        .map(|v| v.u64_value().0);
    if let (Some(old), Some(binding)) = (old_instance, old_binding) {
        let retired = {
            let mut entries = common.animations.entries.borrow_mut();
            let retired = entries
                .get_mut(&old)
                .map(|bindings| {
                    bindings
                        .extract_if(.., |b| b.id == binding)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if entries.get(&old).is_some_and(Vec::is_empty) {
                entries.remove(&old);
            }
            retired
        };
        drop(retired);
    }
    let id = instance.borrow().playback.id();
    set_private_value(
        scope,
        object,
        INSTANCE,
        v8::BigInt::new_from_u64(scope, id.0).into(),
    );
    let binding_id = common.animations.next_binding_id.get();
    common.animations.next_binding_id.set(
        binding_id
            .checked_add(1)
            .expect("animation wrapper identity exhausted"),
    );
    set_private_value(
        scope,
        object,
        BINDING,
        v8::BigInt::new_from_u64(scope, binding_id).into(),
    );
    let wrapper = v8::Weak::new(scope, object);
    let realm = v8::Weak::new(scope, context);
    // A private native ID cannot be forged by reflection or by prototype edits.
    common
        .animations
        .entries
        .borrow_mut()
        .entry(id)
        .or_default()
        .push(Binding {
            id: binding_id,
            instance,
            wrapper,
            realm,
            kind,
        });
    let common = Rc::downgrade(&common);
    crate::v8_finalizer::track_context_owned_v8_finalizer(scope, object, move || {
        if let Some(common) = common.upgrade() {
            let mut entries = common.animations.entries.borrow_mut();
            if let Some(bindings) = entries.get_mut(&id) {
                bindings.retain(|b| b.id != binding_id);
                if bindings.is_empty() {
                    entries.remove(&id);
                }
            }
        }
    });
}

pub(crate) fn instance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<Animation> {
    let value = get_private_value(scope, object, INSTANCE)?;
    let (id, lossless) = v8::Local::<v8::BigInt>::try_from(value).ok()?.u64_value();
    if !lossless {
        return None;
    }
    let realm = object.get_creation_context(scope)?;
    let common = CommonRuntimeState::from_context(realm)?;
    let instance = common
        .animations
        .entries
        .borrow()
        .get(&AnimationId(id))?
        .first()?
        .instance
        .clone();
    if let Some((_, crate::common_runtime::RuntimeExecutionContext::Window(host))) =
        crate::common_runtime::CommonRuntimeBinding::from_scope(scope)
    {
        crate::native_bridge::element::refresh_animation_timeline(unsafe { &*host }, &instance);
    }
    Some(instance)
}
