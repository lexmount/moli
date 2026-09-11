use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use crate::native_bridge::node_runtime_and_handle_from_object;
use crate::util::{
    call_script_visible_function, get_private_object, get_private_value, set_private_value,
};

mod element_registry;

const ANIMATION_PLAY_STATE_SLOT: &str = "__moliAnimationPlayState";
const ANIMATION_ID_SLOT: &str = "__moliAnimationId";
const ANIMATION_READY_SLOT: &str = "__moliAnimationReady";
const ANIMATION_FINISHED_SLOT: &str = "__moliAnimationFinished";
const ANIMATION_FINISHED_RESOLVE_SLOT: &str = "__moliAnimationFinishedResolve";
const ANIMATION_PROMISE_RESOLVE_SLOT: &str = "__moliAnimationPromiseResolve";
const ANIMATION_EFFECT_SLOT: &str = "__moliAnimationEffect";
const ANIMATION_TIMELINE_SLOT: &str = "__moliAnimationTimeline";
const ANIMATION_START_TIME_SLOT: &str = "__moliAnimationStartTime";
const ANIMATION_ONFINISH_SLOT: &str = "__moliAnimationOnfinish";
const ANIMATION_FINISH_TOKEN_SLOT: &str = "__moliAnimationFinishToken";
const ANIMATION_MICROTASK_ANIMATION_SLOT: &str = "__moliAnimationMicrotaskAnimation";
const ANIMATION_MICROTASK_TOKEN_SLOT: &str = "__moliAnimationMicrotaskToken";
const CSS_ANIMATION_TARGET_SLOT: &str = "__moliCssAnimationTarget";
const KEYFRAME_EFFECT_TARGET_SLOT: &str = "__moliKeyframeEffectTarget";
const KEYFRAME_EFFECT_KEYFRAMES_SLOT: &str = "__moliKeyframeEffectKeyframes";

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct AnimationPromiseEntryDeclaration {
    #[webapi(slot = ANIMATION_PROMISE_RESOLVE_SLOT, init = "undefined")]
    resolve: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Animation, enumerable)]
struct AnimationPrototypeMethodsDeclaration {
    #[webapi(
        accessor_property,
        getter = animation_id_getter_callback,
        setter = animation_id_setter_callback
    )]
    id: (),
    #[webapi(method, length = 0, callback = animation_play_callback)]
    play: (),
    #[webapi(method, length = 0, callback = animation_pause_callback)]
    pause: (),
    #[webapi(method, length = 0, callback = animation_cancel_callback)]
    cancel: (),
    #[webapi(method, length = 0, callback = animation_finish_callback)]
    finish: (),
    #[webapi(method, length = 0, callback = animation_reverse_callback)]
    reverse: (),
    #[webapi(method, length = 0, callback = animation_commit_styles_callback)]
    commit_styles: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Animation)]
struct AnimationObjectDeclaration<'s> {
    #[webapi(slot = ANIMATION_ID_SLOT, constructor_default = "")]
    id: &'static str,
    #[webapi(slot = ANIMATION_PLAY_STATE_SLOT, constructor_default = "idle")]
    play_state: &'static str,
    #[webapi(slot = ANIMATION_EFFECT_SLOT)]
    effect: v8::Local<'s, v8::Value>,
    #[webapi(slot = ANIMATION_TIMELINE_SLOT)]
    timeline: v8::Local<'s, v8::Value>,
    #[webapi(slot = ANIMATION_START_TIME_SLOT, init = "null")]
    start_time: (),
    #[webapi(slot = ANIMATION_ONFINISH_SLOT, init = "null")]
    onfinish: (),
    #[webapi(slot = ANIMATION_FINISH_TOKEN_SLOT, constructor_default)]
    finish_token: f64,
    #[webapi(slot = ANIMATION_READY_SLOT)]
    ready: Option<v8::Local<'s, v8::Promise>>,
    #[webapi(slot = ANIMATION_FINISHED_SLOT)]
    finished: Option<v8::Local<'s, v8::Value>>,
    #[webapi(slot = ANIMATION_FINISHED_RESOLVE_SLOT)]
    finished_resolve: Option<v8::Local<'s, v8::Function>>,
}

fn animation_id_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if get_private_value(scope, args.this(), ANIMATION_PLAY_STATE_SLOT).is_none() {
        throw_type_error(
            scope,
            "Animation.id getter called on incompatible receiver.",
        );
        return;
    }
    let id = get_private_value(scope, args.this(), ANIMATION_ID_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if let Some(id) = v8_string(scope, &id) {
        rv.set(id.into());
    }
}

fn animation_id_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if get_private_value(scope, args.this(), ANIMATION_PLAY_STATE_SLOT).is_none() {
        throw_type_error(
            scope,
            "Animation.id setter called on incompatible receiver.",
        );
        return;
    }
    let id = match crate::webidl::convert::<crate::webidl::DomString>(
        scope,
        args.get(0),
        crate::webidl::Context::member("Animation", "id"),
    ) {
        Ok(id) => id.0,
        Err(error) => {
            crate::webidl::throw_error(scope, &error);
            return;
        }
    };
    if let Some(id) = v8_string(scope, &id) {
        set_private_value(scope, args.this(), ANIMATION_ID_SLOT, id.into());
    }
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::KeyframeEffect)]
struct KeyframeEffectObjectDeclaration<'s> {
    #[webapi(slot = KEYFRAME_EFFECT_TARGET_SLOT)]
    target: v8::Local<'s, v8::Value>,
    #[webapi(slot = KEYFRAME_EFFECT_KEYFRAMES_SLOT)]
    keyframes: v8::Local<'s, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::KeyframeEffect, enumerable)]
struct KeyframeEffectPrototypeDeclaration {
    #[webapi(method, length = 1, callback = keyframe_effect_set_keyframes_callback)]
    set_keyframes: (),
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

#[derive(WebApiObject)]
#[webapi(plain)]
struct AnimationPromiseResolverDeclaration<'s> {
    #[webapi(slot = ANIMATION_PROMISE_RESOLVE_SLOT)]
    resolve: v8::Local<'s, v8::Value>,
}

pub(super) fn install_animation_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "Animation" => {
            AnimationPrototypeMethodsDeclaration::initialize_prototype_template(scope, prototype);
        }
        "KeyframeEffect" => {
            KeyframeEffectPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "Element" => {
            ElementAnimationPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}

pub(super) fn animation_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'Animation': Please use the 'new' operator.",
        );
        return;
    }
    let effect = if args.get(0).is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        args.get(0)
    };
    let timeline = if args.get(1).is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        args.get(1)
    };
    initialize_animation_object(scope, args.this(), effect, timeline);
    rv.set(args.this().into());
}

pub(super) fn keyframe_effect_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'KeyframeEffect': Please use the 'new' operator.",
        );
        return;
    }
    KeyframeEffectObjectDeclaration::new(args.get(0), args.get(1))
        .initialize(scope, args.this())
        .expect("KeyframeEffect declaration should initialize object");
    rv.set(args.this().into());
}

fn keyframe_effect_set_keyframes_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    set_private_value(
        scope,
        args.this(),
        KEYFRAME_EFFECT_KEYFRAMES_SLOT,
        args.get(0),
    );
    rv.set_undefined();
}

fn initialize_animation_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    effect: v8::Local<'s, v8::Value>,
    timeline: v8::Local<'s, v8::Value>,
) {
    let ready = if let Some(ready_resolver) = v8::PromiseResolver::new(scope) {
        let ready = ready_resolver.get_promise(scope);
        let _ = ready_resolver.resolve(scope, animation.into());
        Some(ready)
    } else {
        None
    };
    let (finished, finished_resolve) = new_animation_pending_promise(scope)
        .map(|(finished, resolve)| (Some(finished), Some(resolve)))
        .unwrap_or((None, None));
    AnimationObjectDeclaration::new(effect, timeline, ready, finished, finished_resolve)
        .initialize(scope, animation)
        .expect("Animation declaration should initialize object");
}

fn require_animation_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    member: &str,
) -> bool {
    if web_api_interfaces::Animation::is_instance(scope, receiver) {
        return true;
    }
    throw_type_error(
        scope,
        &format!("Animation.{member} called on incompatible receiver."),
    );
    false
}

pub(super) fn animation_play_state_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "playState getter") {
        return;
    }
    rv.set(animation_play_state_value(scope, args.this()));
}

pub(super) fn animation_pending_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "pending getter") {
        return;
    }
    rv.set(animation_pending_value(scope));
}

pub(super) fn animation_ready_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "ready getter") {
        return;
    }
    rv.set(animation_ready_value(scope, args.this()));
}

pub(super) fn animation_finished_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "finished getter") {
        return;
    }
    rv.set(animation_finished_value(scope, args.this()));
}

pub(super) fn animation_effect_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "effect getter") {
        return;
    }
    rv.set(animation_slot_value(
        scope,
        args.this(),
        ANIMATION_EFFECT_SLOT,
    ));
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

pub(super) fn animation_timeline_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "timeline getter") {
        return;
    }
    rv.set(animation_slot_value(
        scope,
        args.this(),
        ANIMATION_TIMELINE_SLOT,
    ));
}

pub(super) fn animation_timeline_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "timeline setter") {
        return;
    }
    set_animation_timeline_value(scope, args.this(), args.get(0));
    rv.set_undefined();
}

pub(super) fn animation_start_time_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "startTime getter") {
        return;
    }
    rv.set(animation_slot_value(
        scope,
        args.this(),
        ANIMATION_START_TIME_SLOT,
    ));
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

pub(super) fn animation_onfinish_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "onfinish getter") {
        return;
    }
    rv.set(animation_slot_value(
        scope,
        args.this(),
        ANIMATION_ONFINISH_SLOT,
    ));
}

pub(super) fn animation_onfinish_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "onfinish setter") {
        return;
    }
    set_animation_onfinish_value(scope, args.this(), args.get(0));
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
    let mut animations = element_registry::for_element(scope, args.this())
        .into_iter()
        .map(v8::Local::<v8::Value>::from)
        .collect::<Vec<_>>();
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, args.this()) {
        let runtime = unsafe { &*runtime_ptr };
        if crate::native_bridge::element::css_animation_start_applies(runtime, handle)
            && let Some(animation) = new_running_animation(scope)
        {
            set_private_value(
                scope,
                animation,
                CSS_ANIMATION_TARGET_SLOT,
                args.this().into(),
            );
            animations.push(animation.into());
        }
    }
    rv.set(v8::Array::new_with_elements(scope, &animations).into());
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

fn new_running_animation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    let constructor = global_constructor_object(scope, "Animation")
        .and_then(|constructor| v8::Local::<v8::Function>::try_from(constructor).ok())?;
    let animation = constructor.new_instance(scope, &[])?;
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

fn animation_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> v8::Local<'s, v8::Value> {
    get_private_value(scope, animation, slot).unwrap_or_else(|| v8::null(scope).into())
}

fn animation_play_state_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    get_private_value(scope, animation, ANIMATION_PLAY_STATE_SLOT)
        .unwrap_or_else(|| v8str(scope, "idle").into())
}

fn animation_pending_value<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    v8::Boolean::new(scope, false).into()
}

fn animation_ready_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    get_private_value(scope, animation, ANIMATION_READY_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into())
}

fn animation_finished_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    get_private_value(scope, animation, ANIMATION_FINISHED_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into())
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

fn set_animation_timeline_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let value = if value.is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        value
    };
    set_private_value(scope, animation, ANIMATION_TIMELINE_SLOT, value);
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

fn set_animation_onfinish_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let value = if value.is_null_or_undefined() || value.is_function() {
        value
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, animation, ANIMATION_ONFINISH_SLOT, value);
}

fn new_animation_pending_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<(v8::Local<'s, v8::Value>, v8::Local<'s, v8::Function>)> {
    let entry = AnimationPromiseEntryDeclaration::default()
        .bind(scope)
        .expect("Animation promise entry declaration should bind");
    let executor = v8::Function::builder(animation_promise_executor_callback)
        .data(entry.into())
        .length(2)
        .build(scope)?;
    let promise_constructor = global_constructor_object(scope, "Promise")
        .and_then(|constructor| v8::Local::<v8::Function>::try_from(constructor).ok())?;
    let promise = promise_constructor.new_instance(scope, &[executor.into()])?;
    let resolve = get_private_value(scope, entry, ANIMATION_PROMISE_RESOLVE_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    Some((promise.into(), resolve))
}

fn animation_promise_executor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(entry) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    AnimationPromiseResolverDeclaration::new(args.get(0))
        .initialize(scope, entry)
        .expect("Animation promise resolver declaration should initialize object");
    rv.set_undefined();
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

fn private_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<String> {
    get_private_value(scope, object, slot).and_then(|value| {
        value
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
    })
}

fn private_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<f64> {
    get_private_value(scope, object, slot).and_then(|value| value.number_value(scope))
}
