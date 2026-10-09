use super::*;
use std::collections::HashMap;

const ELEMENT_CSS_ANIMATIONS_SLOT: &str = "__moliElementCssAnimations";

pub(super) fn synchronize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    metadata: &[crate::style_engine::CssAnimationMetadata],
) -> Vec<v8::Local<'s, v8::Object>> {
    let previous = get_private_value(scope, target, ELEMENT_CSS_ANIMATIONS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .map(|array| {
            (0..array.length())
                .filter_map(|index| array.get_index(scope, index))
                .filter_map(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut indices_by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, animation) in previous.iter().enumerate() {
        if let Some(name) = get_private_value(scope, *animation, CSS_ANIMATION_NAME_SLOT) {
            indices_by_name
                .entry(name.to_rust_string_lossy(scope))
                .or_default()
                .push(index);
        }
    }
    let mut retained = vec![false; previous.len()];
    let mut animations = Vec::with_capacity(metadata.len());
    // CSS Animations matches duplicate names from the end of both lists.
    // Reordering or inserting a name must preserve existing animation identity.
    for metadata in metadata.iter().rev() {
        let existing = indices_by_name.get_mut(&metadata.name).and_then(Vec::pop);
        if let Some(index) = existing {
            retained[index] = true;
            set_private_value(
                scope,
                previous[index],
                CSS_ANIMATION_START_ELAPSED_SLOT,
                v8::Number::new(scope, metadata.start_elapsed_time).into(),
            );
            animations.push(previous[index]);
        } else if let Some(animation) = new_css_animation(scope, target, metadata) {
            animations.push(animation);
        }
    }
    animations.reverse();
    for (animation, retained) in previous.into_iter().zip(retained) {
        if retained {
            continue;
        }
        set_private_value(
            scope,
            animation,
            CSS_ANIMATION_TARGET_SLOT,
            v8::null(scope).into(),
        );
        set_animation_play_state(scope, animation, "idle");
    }
    let values = animations
        .iter()
        .copied()
        .map(Into::into)
        .collect::<Vec<_>>();
    let array = v8::Array::new_with_elements(scope, &values);
    set_private_value(scope, target, ELEMENT_CSS_ANIMATIONS_SLOT, array.into());
    animations
}
