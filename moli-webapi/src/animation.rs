//! Animation and KeyframeEffect value state. DOM style updates and animation
//! playback/dispatch stay with the embedding runtime.
use crate::interfaces as web_api_interfaces;
use moli_v8_util::{
    get_private_value, global_constructor_object, set_private_value, throw_type_error, v8_string,
    v8str,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use moli_webidl as webidl;

pub const ANIMATION_PLAY_STATE_SLOT: &str = "__moliAnimationPlayState";

const ANIMATION_ID_SLOT: &str = "__moliAnimationId";

const ANIMATION_READY_SLOT: &str = "__moliAnimationReady";

const ANIMATION_FINISHED_SLOT: &str = "__moliAnimationFinished";

pub const ANIMATION_FINISHED_RESOLVE_SLOT: &str = "__moliAnimationFinishedResolve";

const ANIMATION_PROMISE_RESOLVE_SLOT: &str = "__moliAnimationPromiseResolve";

pub const ANIMATION_EFFECT_SLOT: &str = "__moliAnimationEffect";

const ANIMATION_TIMELINE_SLOT: &str = "__moliAnimationTimeline";

pub const ANIMATION_START_TIME_SLOT: &str = "__moliAnimationStartTime";

pub const ANIMATION_ONFINISH_SLOT: &str = "__moliAnimationOnfinish";

pub const ANIMATION_FINISH_TOKEN_SLOT: &str = "__moliAnimationFinishToken";

pub const KEYFRAME_EFFECT_TARGET_SLOT: &str = "__moliKeyframeEffectTarget";

pub const KEYFRAME_EFFECT_KEYFRAMES_SLOT: &str = "__moliKeyframeEffectKeyframes";

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct AnimationPromiseEntryDeclaration {
    #[webapi(slot = ANIMATION_PROMISE_RESOLVE_SLOT, init = "undefined")]
    resolve: (),
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
    #[webapi(method, length = 1, callback = keyframe_effect_set_keyframes_callback, receiver = false)]
    set_keyframes: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct AnimationPromiseResolverDeclaration<'s> {
    #[webapi(slot = ANIMATION_PROMISE_RESOLVE_SLOT)]
    resolve: v8::Local<'s, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Animation, enumerable)]
struct AnimationValuePrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = animation_id_getter_callback,
        setter = animation_id_setter_callback
    )]
    id: (),

    #[webapi(accessor_property = "playState", getter = animation_play_state_getter)]
    play_state: (),

    #[webapi(accessor_property, getter = animation_pending_getter)]
    pending: (),

    #[webapi(accessor_property, getter = animation_ready_getter)]
    ready: (),

    #[webapi(accessor_property, getter = animation_finished_getter)]
    finished: (),

    #[webapi(
        accessor_property,
        getter = animation_timeline_getter,
        setter = animation_timeline_setter
    )]
    timeline: (),

    #[webapi(
        accessor_property,
        getter = animation_onfinish_getter,
        setter = animation_onfinish_setter
    )]
    onfinish: (),
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
    let id = match webidl::convert::<webidl::DomString>(
        scope,
        args.get(0),
        webidl::Context::member("Animation", "id"),
    ) {
        Ok(id) => id.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if let Some(id) = v8_string(scope, &id) {
        set_private_value(scope, args.this(), ANIMATION_ID_SLOT, id.into());
    }
}

pub fn animation_constructor_callback<'s>(
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

pub fn keyframe_effect_constructor_callback<'s>(
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

pub fn new_keyframe_effect_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Value>,
    keyframes: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    KeyframeEffectObjectDeclaration::new(target, keyframes)
        .bind(scope)
        .ok()
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

pub fn initialize_animation_object<'s>(
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

pub fn require_animation_receiver<'s>(
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

fn animation_play_state_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "playState getter") {
        return;
    }
    rv.set(animation_play_state_value(scope, args.this()));
}

fn animation_pending_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "pending getter") {
        return;
    }
    rv.set(animation_pending_value(scope));
}

fn animation_ready_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "ready getter") {
        return;
    }
    rv.set(animation_ready_value(scope, args.this()));
}

fn animation_finished_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "finished getter") {
        return;
    }
    rv.set(animation_finished_value(scope, args.this()));
}

pub fn animation_effect_getter<'s>(
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

fn animation_timeline_getter<'s>(
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

fn animation_onfinish_getter<'s>(
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

pub fn animation_start_time_getter<'s>(
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

pub fn install_value_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "Animation" => {
            AnimationValuePrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        "KeyframeEffect" => {
            KeyframeEffectPrototypeDeclaration::initialize_prototype_template(scope, prototype)
        }
        _ => (),
    }
}

fn animation_timeline_setter<'s>(
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

fn animation_onfinish_setter<'s>(
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
