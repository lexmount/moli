use super::*;

const ELEMENT_ANIMATIONS_SLOT: &str = "__moliElementAnimations";
const REGISTRY_TARGET_SLOT: &str = "__moliAnimationRegistryTarget";

pub(super) fn for_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(animations) = get_private_value(scope, target, ELEMENT_ANIMATIONS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        return Vec::new();
    };
    (0..animations.length())
        .filter_map(|index| animations.get_index(scope, index))
        .filter_map(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .collect()
}

pub(super) fn register<'s>(scope: &mut v8::PinScope<'s, '_>, animation: v8::Local<'s, v8::Object>) {
    let Some(target) = effect_target(scope, animation) else {
        return;
    };
    register_for_target(scope, animation, target);
}

pub(super) fn register_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    target: v8::Local<'s, v8::Object>,
) {
    let mut animations = for_element(scope, target);
    if animations
        .iter()
        .any(|candidate| candidate.strict_equals(animation.into()))
    {
        return;
    }
    set_private_value(scope, animation, REGISTRY_TARGET_SLOT, target.into());
    animations.push(animation);
    store(scope, target, &animations);
}

pub(super) fn unregister<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
) {
    let Some(target) = get_private_object(scope, animation, REGISTRY_TARGET_SLOT) else {
        return;
    };
    let mut animations = for_element(scope, target);
    let original_len = animations.len();
    animations.retain(|candidate| !candidate.strict_equals(animation.into()));
    if animations.len() != original_len {
        store(scope, target, &animations);
    }
}

fn effect_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let effect = get_private_object(scope, animation, ANIMATION_EFFECT_SLOT)?;
    get_private_object(scope, effect, KEYFRAME_EFFECT_TARGET_SLOT)
}

fn store<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    animations: &[v8::Local<'s, v8::Object>],
) {
    let values = animations
        .iter()
        .copied()
        .map(v8::Local::<v8::Value>::from)
        .collect::<Vec<_>>();
    let array = v8::Array::new_with_elements(scope, &values);
    set_private_value(scope, target, ELEMENT_ANIMATIONS_SLOT, array.into());
}
