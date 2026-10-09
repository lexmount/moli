use super::*;
use crate::web_api_interfaces;
pub(super) use moli_webapi::animation::{
    ANIMATION_EFFECT_SLOT, ANIMATION_FINISH_TOKEN_SLOT, ANIMATION_FINISHED_RESOLVE_SLOT,
    ANIMATION_ONFINISH_SLOT, ANIMATION_PLAY_STATE_SLOT, ANIMATION_START_TIME_SLOT,
    KEYFRAME_EFFECT_KEYFRAMES_SLOT, KEYFRAME_EFFECT_TARGET_SLOT, animation_constructor_callback,
    animation_effect_getter, animation_start_time_getter, initialize_animation_object,
    keyframe_effect_constructor_callback, require_animation_receiver,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use crate::native_bridge::node_runtime_and_handle_from_object;
use crate::util::{
    call_script_visible_function, get_private_object, get_private_value, set_private_value,
};

mod css_registry;
mod element_registry;

const ANIMATION_MICROTASK_ANIMATION_SLOT: &str = "__moliAnimationMicrotaskAnimation";
const ANIMATION_MICROTASK_TOKEN_SLOT: &str = "__moliAnimationMicrotaskToken";
const CSS_ANIMATION_TARGET_SLOT: &str = "__moliCssAnimationTarget";
const CSS_ANIMATION_NAME_SLOT: &str = "__moliCssAnimationName";
const CSS_ANIMATION_START_ELAPSED_SLOT: &str = "__moliCssAnimationStartElapsed";
const CSS_TRANSITION_PROPERTY_SLOT: &str = "__moliCssTransitionProperty";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Animation, enumerable)]
struct AnimationPrototypeMethodsDeclaration {
    #[webapi(method, length = 0, callback = animation_play_callback, receiver = false)]
    play: (),
    #[webapi(method, length = 0, callback = animation_pause_callback)]
    pause: (),
    #[webapi(method, length = 0, callback = animation_cancel_callback)]
    cancel: (),
    #[webapi(method, length = 0, callback = animation_finish_callback, receiver = false)]
    finish: (),
    #[webapi(method, length = 0, callback = animation_reverse_callback)]
    reverse: (),
    #[webapi(method, length = 0, callback = animation_commit_styles_callback)]
    commit_styles: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSAnimation, require_prototype)]
struct CssAnimationObjectDeclaration<'s> {
    #[webapi(slot = CSS_ANIMATION_NAME_SLOT)]
    name: v8::Local<'s, v8::String>,
    #[webapi(slot = CSS_ANIMATION_TARGET_SLOT)]
    target: v8::Local<'s, v8::Object>,
    #[webapi(slot = CSS_ANIMATION_START_ELAPSED_SLOT)]
    start_elapsed_time: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSAnimation, enumerable)]
struct CssAnimationPrototypeDeclaration {
    #[webapi(accessor_property, getter = css_animation_metadata_getter, data = v8str(scope, CSS_ANIMATION_NAME_SLOT))]
    animation_name: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::CSSTransition, enumerable)]
struct CssTransitionPrototypeDeclaration {
    #[webapi(accessor_property, getter = css_animation_metadata_getter, data = v8str(scope, CSS_TRANSITION_PROPERTY_SLOT))]
    transition_property: (),
}

fn css_animation_metadata_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated CSS animation receiver validation");
    let slot = args.data().to_rust_string_lossy(scope);
    if let Some(value) = get_private_value(scope, receiver, &slot) {
        rv.set(value);
    }
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Element, enumerable)]
struct ElementAnimationPrototypeDeclaration {
    #[webapi(method, length = 0, callback = element_animate_callback)]
    animate: (),
    #[webapi(method, length = 0, callback = element_get_animations_callback)]
    get_animations: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct AnimationFinishMicrotaskPayloadDeclaration<'s> {
    #[webapi(slot = ANIMATION_MICROTASK_ANIMATION_SLOT)]
    animation: v8::Local<'s, v8::Object>,
    #[webapi(slot = ANIMATION_MICROTASK_TOKEN_SLOT)]
    token: f64,
}

pub(super) fn install_animation_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    moli_webapi::animation::install_value_template_bindings(scope, template, interface_name);
    let prototype = template.prototype_template(scope);
    match interface_name {
        "Animation" => {
            AnimationPrototypeMethodsDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSAnimation" => {
            CssAnimationPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "CSSTransition" => {
            CssTransitionPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "Element" => {
            ElementAnimationPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}

pub(super) fn animation_effect_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "effect setter") {
        return;
    }
    set_animation_effect_value(scope, args.this(), args.get(0));
    rv.set_undefined();
}

pub(super) fn animation_start_time_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "startTime setter") {
        return;
    }
    set_animation_start_time_value(scope, args.this(), args.get(0));
    rv.set_undefined();
}

pub(super) fn animation_play_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    animation_play(scope, args.this());
    rv.set_undefined();
}

pub(super) fn animation_pause_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_animation_play_state(scope, args.this(), "paused");
    bump_animation_finish_token(scope, args.this());
    rv.set_undefined();
}

pub(super) fn animation_cancel_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    clear_animation_effect_style(scope, args.this());
    element_registry::unregister(scope, args.this());
    set_animation_play_state(scope, args.this(), "idle");
    bump_animation_finish_token(scope, args.this());
    rv.set_undefined();
}

pub(super) fn animation_finish_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    animation_finish(scope, args.this(), true);
    rv.set_undefined();
}

pub(super) fn animation_reverse_callback(
    _scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_undefined();
}

pub(super) fn animation_commit_styles_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(target) = get_private_object(scope, args.this(), CSS_ANIMATION_TARGET_SLOT) {
        commit_css_animation_styles(scope, target);
    } else {
        commit_animation_effect_styles(scope, args.this());
    }
    rv.set_undefined();
}

fn element_animate_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(constructor) = global_constructor_object(scope, "Animation")
        .and_then(|constructor| v8::Local::<v8::Function>::try_from(constructor).ok())
    else {
        rv.set_undefined();
        return;
    };
    let effect = if args.get(0).is_null_or_undefined() && args.get(1).is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        new_keyframe_effect_for_target(scope, args.this(), args.get(0))
            .map(v8::Local::<v8::Value>::from)
            .unwrap_or_else(|| v8::null(scope).into())
    };
    let timeline = v8::null(scope);
    let Some(animation) = constructor.new_instance(scope, &[effect, timeline.into()]) else {
        rv.set_undefined();
        return;
    };
    if !args.get(0).is_null_or_undefined() || !args.get(1).is_null_or_undefined() {
        animation_play(scope, animation);
    }
    rv.set(animation.into());
}

fn element_get_animations_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated Element receiver validation");
    let mut animations = css_animations_for_element(scope, target);
    for animation in element_registry::for_element(scope, target) {
        if !animations
            .iter()
            .any(|value| value.strict_equals(animation.into()))
        {
            animations.push(animation);
        }
    }
    let values = animations.into_iter().map(Into::into).collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &values).into());
}

pub(crate) fn css_animations_for_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, target) else {
        return Vec::new();
    };
    let animations =
        crate::native_bridge::element::active_css_animations(unsafe { &*runtime_ptr }, handle);
    // Platform objects belong to the target's realm. The getAnimations Array
    // is created by its caller after leaving this context.
    let Some(context) = target.get_creation_context(scope) else {
        return Vec::new();
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    css_registry::synchronize(scope, target, &animations)
}

pub(crate) fn css_animation_start_events_for_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let Some(context) = target.get_creation_context(scope) else {
        return Vec::new();
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    css_animations_for_element(scope, target)
        .into_iter()
        .filter_map(|animation| {
            let name = get_private_value(scope, animation, CSS_ANIMATION_NAME_SLOT)
                .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())?;
            let elapsed_time =
                get_private_value(scope, animation, CSS_ANIMATION_START_ELAPSED_SLOT)?
                    .number_value(scope)?;
            super::events::construct_css_animation_start_event(scope, animation, name, elapsed_time)
        })
        .collect()
}

fn new_keyframe_effect_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    keyframes: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let constructor = global_constructor_object(scope, "KeyframeEffect")
        .and_then(|constructor| v8::Local::<v8::Function>::try_from(constructor).ok())?;
    constructor.new_instance(scope, &[target.into(), keyframes])
}

fn new_css_animation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    metadata: &crate::style_engine::CssAnimationMetadata,
) -> Option<v8::Local<'s, v8::Object>> {
    let name = v8_string(scope, &metadata.name)?;
    let animation = CssAnimationObjectDeclaration::new(name, target, metadata.start_elapsed_time)
        .bind(scope)
        .ok()?;
    let effect = moli_webapi::animation::new_keyframe_effect_object(
        scope,
        target.into(),
        v8::null(scope).into(),
    )?;
    let timeline = v8::null(scope);
    initialize_animation_object(scope, animation, effect.into(), timeline.into());
    set_animation_play_state(scope, animation, "running");
    Some(animation)
}

fn commit_css_animation_styles<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, target) else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(transform) =
        crate::native_bridge::element::active_css_animation_transform_value(runtime, handle)
    else {
        return;
    };
    set_target_style_property(scope, target, "transform", &transform);
}

fn animation_play<'s>(scope: &mut v8::PinScope<'s, '_>, animation: v8::Local<'s, v8::Object>) {
    if animation_play_state(scope, animation) == "running" {
        return;
    }
    apply_animation_effect_style(scope, animation);
    set_animation_play_state(scope, animation, "running");
    element_registry::register(scope, animation);
    let token = bump_animation_finish_token(scope, animation);
    let payload = AnimationFinishMicrotaskPayloadDeclaration::new(animation, token as f64)
        .bind(scope)
        .expect("Animation finish microtask payload declaration should bind");
    if let Some(callback) = v8::Function::builder(animation_finish_microtask_callback)
        .data(payload.into())
        .build(scope)
    {
        scope.enqueue_microtask(callback);
    }
}

fn apply_animation_effect_style<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
) {
    let Some((target, background_image)) =
        animation_effect_style_property(scope, animation, &["backgroundImage", "background-image"])
    else {
        return;
    };
    set_target_background_image(scope, target, &background_image);
}

fn commit_animation_effect_styles<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
) {
    for (target, property, value) in animation_effect_committable_style_properties(scope, animation)
    {
        set_target_style_property(scope, target, property, &value);
    }
}

fn clear_animation_effect_style<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
) {
    let Some(effect) = get_private_object(scope, animation, ANIMATION_EFFECT_SLOT) else {
        return;
    };
    let Some(target) = get_private_object(scope, effect, KEYFRAME_EFFECT_TARGET_SLOT) else {
        return;
    };
    set_target_background_image(scope, target, "");
}

fn animation_effect_committable_style_properties<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
) -> Vec<(v8::Local<'s, v8::Object>, &'static str, String)> {
    // This only extracts supported Web Animations keyframe object members.
    // CSS property canonicalization and value normalization stay in the
    // CSSStyleDeclaration setter reached by set_target_style_property().
    const COMMITTABLE_PROPERTIES: &[(&str, &[&str])] = &[
        ("backgroundImage", &["backgroundImage", "background-image"]),
        ("borderColor", &["borderColor", "border-color"]),
    ];

    let Some(effect) = get_private_object(scope, animation, ANIMATION_EFFECT_SLOT) else {
        return Vec::new();
    };
    let Some(target) = get_private_object(scope, effect, KEYFRAME_EFFECT_TARGET_SLOT) else {
        return Vec::new();
    };
    let Some(keyframes) = get_private_object(scope, effect, KEYFRAME_EFFECT_KEYFRAMES_SLOT) else {
        return Vec::new();
    };
    COMMITTABLE_PROPERTIES
        .iter()
        .filter_map(|(property, keyframe_names)| {
            keyframes_style_value(scope, keyframes, keyframe_names)
                .map(|value| (target, *property, value))
        })
        .collect()
}

fn animation_effect_style_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    keyframe_names: &[&'static str],
) -> Option<(v8::Local<'s, v8::Object>, String)> {
    let effect = get_private_object(scope, animation, ANIMATION_EFFECT_SLOT)?;
    let target = get_private_object(scope, effect, KEYFRAME_EFFECT_TARGET_SLOT)?;
    let keyframes = get_private_object(scope, effect, KEYFRAME_EFFECT_KEYFRAMES_SLOT)?;
    let value = keyframes_style_value(scope, keyframes, keyframe_names)?;
    Some((target, value))
}

fn keyframes_style_value(
    scope: &mut v8::PinScope<'_, '_>,
    keyframes: v8::Local<'_, v8::Object>,
    properties: &[&'static str],
) -> Option<String> {
    let mut committed_value = None;

    if let Ok(array) = v8::Local::<v8::Array>::try_from(keyframes) {
        for index in 0..array.length() {
            let Some(frame) = array.get_index(scope, index) else {
                continue;
            };
            let Ok(frame) = v8::Local::<v8::Object>::try_from(frame) else {
                continue;
            };
            for property in properties {
                if let Some(value) = keyframe_property_value(scope, frame, property) {
                    committed_value = Some(value);
                }
            }
        }
    } else {
        for property in properties {
            if let Some(value) = keyframe_property_value(scope, keyframes, property) {
                committed_value = Some(value);
            }
        }
    }
    committed_value
}

fn keyframe_property_value(
    scope: &mut v8::PinScope<'_, '_>,
    frame: v8::Local<'_, v8::Object>,
    property: &'static str,
) -> Option<String> {
    let value = frame.get(scope, v8str(scope, property).into())?;
    if value.is_null_or_undefined() {
        return None;
    }
    if let Ok(values) = v8::Local::<v8::Array>::try_from(value) {
        let mut last = None;
        for index in 0..values.length() {
            let Some(value) = values.get_index(scope, index) else {
                continue;
            };
            if value.is_null_or_undefined() {
                continue;
            }
            if let Some(value) = value.to_string(scope) {
                last = Some(value.to_rust_string_lossy(scope));
            }
        }
        return last;
    }
    value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
}

fn set_target_background_image<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    value: &str,
) {
    set_target_style_property(scope, target, "backgroundImage", value);
}

fn set_target_style_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    property: &str,
    value: &str,
) {
    let Some(style) = target.get(scope, v8str(scope, "style").into()) else {
        return;
    };
    let Ok(style) = v8::Local::<v8::Object>::try_from(style) else {
        return;
    };
    let Some(value) = v8::String::new(scope, value) else {
        return;
    };
    let Some(property) = v8::String::new(scope, property) else {
        return;
    };
    let _ = style.set(scope, property.into(), value.into());
}

fn animation_finish<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    invoke_onfinish: bool,
) {
    if animation_play_state(scope, animation) == "finished" {
        return;
    }
    element_registry::unregister(scope, animation);
    set_animation_play_state(scope, animation, "finished");
    bump_animation_finish_token(scope, animation);
    if let Some(resolve) = get_private_value(scope, animation, ANIMATION_FINISHED_RESOLVE_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let _ = resolve.call(scope, v8::undefined(scope).into(), &[animation.into()]);
    }
    if invoke_onfinish
        && let Some(callback) = get_private_value(scope, animation, ANIMATION_ONFINISH_SLOT)
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        let _ = call_script_visible_function(
            scope,
            callback,
            animation.into(),
            &[],
            "Animation.onfinish callback",
        );
    }
}

fn animation_finish_microtask_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(payload) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    let Some(animation) = get_private_object(scope, payload, ANIMATION_MICROTASK_ANIMATION_SLOT)
    else {
        return;
    };
    let expected_token =
        private_number(scope, payload, ANIMATION_MICROTASK_TOKEN_SLOT).unwrap_or(-1.0);
    let current_token =
        private_number(scope, animation, ANIMATION_FINISH_TOKEN_SLOT).unwrap_or(0.0);
    if (current_token - expected_token).abs() > f64::EPSILON
        || animation_play_state(scope, animation) != "running"
    {
        return;
    }
    animation_finish(scope, animation, true);
}

fn set_animation_effect_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let is_current = matches!(
        animation_play_state(scope, animation).as_str(),
        "running" | "paused"
    );
    if is_current {
        element_registry::unregister(scope, animation);
    }
    set_private_value(scope, animation, ANIMATION_EFFECT_SLOT, value);
    if is_current {
        element_registry::register(scope, animation);
    }
}

fn set_animation_start_time_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let stored = if value.is_null_or_undefined() {
        v8::null(scope).into()
    } else if let Some(number) = value.number_value(scope) {
        v8::Number::new(scope, number).into()
    } else {
        value
    };
    set_private_value(scope, animation, ANIMATION_START_TIME_SLOT, stored);
    if !value.is_null_or_undefined() {
        animation_play(scope, animation);
    }
}

fn animation_play_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
) -> String {
    private_string(scope, animation, ANIMATION_PLAY_STATE_SLOT).unwrap_or_else(|| "idle".to_owned())
}

fn set_animation_play_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    state: &'static str,
) {
    set_private_value(
        scope,
        animation,
        ANIMATION_PLAY_STATE_SLOT,
        v8str(scope, state).into(),
    );
}

fn bump_animation_finish_token<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
) -> u32 {
    let current = private_number(scope, animation, ANIMATION_FINISH_TOKEN_SLOT).unwrap_or(0.0);
    let next = current as u32 + 1;
    set_private_value(
        scope,
        animation,
        ANIMATION_FINISH_TOKEN_SLOT,
        v8::Number::new(scope, next as f64).into(),
    );
    next
}

use crate::util::private_string_property as private_string;

fn private_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<f64> {
    get_private_value(scope, object, slot).and_then(|value| value.number_value(scope))
}
