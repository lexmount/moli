use super::*;
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use crate::native_bridge::node_runtime_and_handle_from_object;
use crate::util::{get_private_object, get_private_value, set_private_value};

mod element_registry;
mod native;
mod timing;
use crate::{animation_bindings, animation_model::Animation};
pub(crate) use native::notify_animation_playback;

const ANIMATION_ID_SLOT: &str = "__moliAnimationId";
const ANIMATION_READY_SLOT: &str = "__moliAnimationReady";
const ANIMATION_FINISHED_SLOT: &str = "__moliAnimationFinished";
const ANIMATION_FINISHED_RESOLVER_SLOT: &str = "__moliAnimationFinishedResolver";
const ANIMATION_READY_RESOLVER_SLOT: &str = "__moliAnimationReadyResolver";
const ANIMATION_PROMISE_OWNER_SLOT: &str = "__moliAnimationPromiseOwner";
const ANIMATION_CANCEL_GENERATION_SLOT: &str = "__moliAnimationCancelGeneration";
const ANIMATION_FINISHED_EXPOSED_SLOT: &str = "__moliAnimationFinishedExposed";
const ANIMATION_FINISHED_SETTLED_SLOT: &str = "__moliAnimationFinishedSettled";
const ANIMATION_FINISHED_GENERATION_SLOT: &str = "__moliAnimationFinishedGeneration";
const ANIMATION_EFFECT_SLOT: &str = "__moliAnimationEffect";
const ANIMATION_TIMELINE_SLOT: &str = "__moliAnimationTimeline";
const ANIMATION_TIMELINE_ORIGIN_SLOT: &str = "__moliAnimationTimelineOrigin";
const TIMELINE_DOCUMENT_SLOT: &str = "__moliTimelineDocument";
const TIMELINE_ORIGIN_SLOT: &str = "__moliTimelineOrigin";
const ANIMATION_ONFINISH_SLOT: &str = "__moliAnimationOnfinish";
const KEYFRAME_EFFECT_TARGET_SLOT: &str = "__moliKeyframeEffectTarget";

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
    #[webapi(slot = ANIMATION_EFFECT_SLOT)]
    effect: v8::Local<'s, v8::Value>,
    #[webapi(slot = ANIMATION_TIMELINE_SLOT)]
    timeline: v8::Local<'s, v8::Value>,
    #[webapi(slot = ANIMATION_ONFINISH_SLOT, init = "null")]
    onfinish: (),
    #[webapi(slot = ANIMATION_READY_RESOLVER_SLOT)]
    ready_resolver: v8::Local<'s, v8::Value>,
    #[webapi(slot = ANIMATION_FINISHED_RESOLVER_SLOT)]
    finished_resolver: v8::Local<'s, v8::Value>,
    #[webapi(slot = ANIMATION_FINISHED_EXPOSED_SLOT, constructor_default)]
    finished_exposed: bool,
    #[webapi(slot = ANIMATION_FINISHED_SETTLED_SLOT, constructor_default)]
    finished_settled: bool,
    #[webapi(slot = ANIMATION_READY_SLOT)]
    ready: v8::Local<'s, v8::Promise>,
    #[webapi(slot = ANIMATION_FINISHED_SLOT)]
    finished: v8::Local<'s, v8::Promise>,
}

fn animation_id_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !web_api_interfaces::Animation::is_instance(scope, args.this())
        || animation_bindings::instance(scope, args.this()).is_none()
    {
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
    if !web_api_interfaces::Animation::is_instance(scope, args.this())
        || animation_bindings::instance(scope, args.this()).is_none()
    {
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
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::KeyframeEffect, enumerable)]
struct KeyframeEffectPrototypeDeclaration {
    #[webapi(method, length = 1, callback = keyframe_effect_set_keyframes_callback)]
    set_keyframes: (),
    #[webapi(accessor_property,getter=keyframe_effect_target_getter,setter=keyframe_effect_target_setter)]
    target: (),
    #[webapi(method,length=0,callback=keyframe_effect_update_timing_callback)]
    update_timing: (),
    #[webapi(method,length=0,callback=keyframe_effect_get_timing_callback)]
    get_timing: (),
    #[webapi(method,length=0,callback=keyframe_effect_get_computed_timing_callback)]
    get_computed_timing: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Element, enumerable, receiver)]
struct ElementAnimationPrototypeDeclaration {
    #[webapi(method, length = 0, callback = element_animate_callback)]
    animate: (),
    #[webapi(method, length = 0, callback = element_get_animations_callback)]
    get_animations: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface=web_api_interfaces::Animation,enumerable)]
struct AnimationTimeAccessors {
    #[webapi(accessor_property="currentTime",getter=animation_current_time_getter,setter=animation_current_time_setter)]
    current_time: (),
    #[webapi(accessor_property="playbackRate",getter=animation_playback_rate_getter,setter=animation_playback_rate_setter)]
    playback_rate: (),
}
#[derive(WebApiFunctionTemplate)]
#[webapi(interface=web_api_interfaces::CSSAnimation,enumerable)]
struct CssAnimationAccessors {
    #[webapi(accessor_property="animationName",getter=css_animation_name_getter)]
    animation_name: (),
}
#[derive(WebApiFunctionTemplate)]
#[webapi(interface=web_api_interfaces::CSSTransition,enumerable)]
struct CssTransitionAccessors {
    #[webapi(accessor_property="transitionProperty",getter=css_animation_name_getter)]
    transition_property: (),
}
#[derive(WebApiFunctionTemplate)]
#[webapi(interface=web_api_interfaces::Document,enumerable,receiver)]
struct DocumentAnimationAccessors {
    #[webapi(accessor_property,getter=document_timeline_getter)]
    timeline: (),
}
#[derive(WebApiFunctionTemplate)]
#[webapi(interface=web_api_interfaces::AnimationTimeline,enumerable)]
struct TimelineAccessors {
    #[webapi(accessor_property="currentTime",getter=timeline_current_time_getter)]
    current_time: (),
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
            AnimationTimeAccessors::initialize_prototype_template(scope, prototype);
        }
        "CSSAnimation" => CssAnimationAccessors::initialize_prototype_template(scope, prototype),
        "CSSTransition" => CssTransitionAccessors::initialize_prototype_template(scope, prototype),
        "Document" => DocumentAnimationAccessors::initialize_prototype_template(scope, prototype),
        "AnimationTimeline" => TimelineAccessors::initialize_prototype_template(scope, prototype),
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
    if !native::validate_effect(scope, args.get(0))
        || !native::validate_timeline(scope, args.get(1))
    {
        return;
    }
    let effect = if args.get(0).is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        args.get(0)
    };
    let timeline = if args.length() < 2 {
        native::default_timeline(scope)
    } else if args.get(1).is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        args.get(1)
    };
    initialize_animation_object(scope, args.this(), effect, timeline);
    native::script_instance(scope, args.this(), effect);
    set_animation_timeline_value(scope, args.this(), timeline);
    native::resolve_ready(scope, args.this());
    rv.set(args.this().into());
}

pub(super) fn keyframe_effect_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "Please use new KeyframeEffect()");
        return;
    }
    if !native::validate_target(scope, args.get(0)) {
        return;
    }
    let Some(frames) = timing::keyframes(scope, args.get(1)) else {
        return;
    };
    let Some(instance) = timing::parse_timing(scope, args.get(2), None) else {
        return;
    };
    instance.borrow_mut().set_keyframes(frames);
    KeyframeEffectObjectDeclaration::new(args.get(0))
        .initialize(scope, args.this())
        .expect("KeyframeEffect declaration");
    animation_bindings::bind(
        scope,
        args.this(),
        instance,
        crate::animation_bindings::WrapperKind::Effect,
    );
    rv.set(args.this().into());
}

fn keyframe_effect_set_keyframes_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_effect_receiver(scope, args.this()) {
        return;
    }
    let Some(frames) = timing::keyframes(scope, args.get(0)) else {
        return;
    };
    if let Some(instance) = animation_bindings::instance(scope, args.this()) {
        instance.borrow_mut().set_keyframes(frames);
        if instance.borrow().css_effect_source == crate::animation_model::CssEffectSource::Css {
            instance.borrow_mut().css_effect_source =
                crate::animation_model::CssEffectSource::ScriptKeyframes;
        }
        update_effect_owner(scope, &instance);
    }
    rv.set_undefined();
}

fn initialize_animation_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    effect: v8::Local<'s, v8::Value>,
    timeline: v8::Local<'s, v8::Value>,
) {
    let ready = v8::PromiseResolver::new(scope).expect("Animation ready resolver");
    let finished = v8::PromiseResolver::new(scope).expect("Animation finished resolver");
    AnimationObjectDeclaration::new(
        effect,
        timeline,
        ready.into(),
        finished.into(),
        ready.get_promise(scope),
        finished.get_promise(scope),
    )
    .initialize(scope, animation)
    .expect("Animation declaration");
    initialize_simple_event_target(scope, animation);
    set_private_value(
        scope,
        finished.get_promise(scope).into(),
        ANIMATION_PROMISE_OWNER_SLOT,
        animation.into(),
    );
}

fn require_effect_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    if web_api_interfaces::KeyframeEffect::is_instance(scope, object)
        && animation_bindings::instance(scope, object).is_some()
    {
        return true;
    }
    throw_type_error(scope, "KeyframeEffect called on incompatible receiver");
    false
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
    let pending = animation_bindings::instance(scope, args.this())
        .is_some_and(|a| a.borrow().playback.is_pending());
    rv.set(v8::Boolean::new(scope, pending).into());
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
    let origin = native::animation_origin(scope, args.this());
    let value = animation_bindings::instance(scope, args.this()).and_then(|a| {
        let a = a.borrow();
        (!a.playback.is_idle()).then_some(())?;
        a.playback.clock().start_millis(origin?)
    });
    rv.set(
        value
            .map(|v| v8::Number::new(scope, v).into())
            .unwrap_or_else(|| v8::null(scope).into()),
    );
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
    if !require_animation_receiver(scope, args.this(), "play") {
        return;
    }
    animation_play(scope, args.this());
    rv.set_undefined();
}

pub(super) fn animation_pause_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "pause") {
        return;
    }
    if let Some(instance) = animation_bindings::instance(scope, args.this()) {
        native::apply_command(
            scope,
            args.this(),
            &instance,
            moli_animation::AnimationCommand::Pause,
            "Cannot pause this animation",
        );
    }
    rv.set_undefined();
}

pub(super) fn animation_cancel_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "cancel") {
        return;
    }
    if let Some(instance) = animation_bindings::instance(scope, args.this()) {
        native::apply_command(
            scope,
            args.this(),
            &instance,
            moli_animation::AnimationCommand::Cancel,
            "Cannot cancel this animation",
        );
    }
    rv.set_undefined();
}

pub(super) fn animation_finish_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "finish") {
        return;
    }
    if let Some(instance) = animation_bindings::instance(scope, args.this())
        && !native::apply_command(
            scope,
            args.this(),
            &instance,
            moli_animation::AnimationCommand::Finish,
            "Cannot finish this animation",
        )
    {
        return;
    }
    rv.set_undefined();
}

pub(super) fn animation_reverse_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "reverse") {
        return;
    }
    if let Some(instance) = animation_bindings::instance(scope, args.this())
        && !native::apply_command(
            scope,
            args.this(),
            &instance,
            moli_animation::AnimationCommand::Reverse,
            "Cannot reverse this animation",
        )
    {
        return;
    }
    rv.set_undefined();
}

pub(super) fn animation_commit_styles_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "commit_styles") {
        return;
    }
    commit_animation_effect_styles(scope, args.this());
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
        let Some(effect) =
            new_keyframe_effect_for_target(scope, args.this(), args.get(0), args.get(1))
        else {
            return;
        };
        effect.into()
    };
    let timeline = native::default_timeline(scope);
    let Some(animation) = constructor.new_instance(scope, &[effect, timeline]) else {
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
    let mut values = Vec::new();
    if let Ok((runtime, handle)) = node_runtime_and_handle_from_object(scope, args.this()) {
        let animations =
            crate::native_bridge::element::animations_for_element(unsafe { &*runtime }, handle);
        for instance in animations {
            if let Some(object) = native::wrapper(scope, args.this(), &instance) {
                values.push(object.into());
            }
        }
    }
    rv.set(v8::Array::new_with_elements(scope, &values).into());
}

fn new_keyframe_effect_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    keyframes: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let constructor = global_constructor_object(scope, "KeyframeEffect")
        .and_then(|constructor| v8::Local::<v8::Function>::try_from(constructor).ok())?;
    constructor.new_instance(scope, &[target.into(), keyframes, options])
}

fn animation_play<'s>(scope: &mut v8::PinScope<'s, '_>, animation: v8::Local<'s, v8::Object>) {
    let Some(instance) = animation_bindings::instance(scope, animation) else {
        return;
    };
    native::apply_command(
        scope,
        animation,
        &instance,
        moli_animation::AnimationCommand::Play,
        "Cannot rewind this animation",
    );
}

fn commit_animation_effect_styles<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
) {
    let Some(instance) = animation_bindings::instance(scope, animation) else {
        return;
    };
    let Some(target) = native::effect_target(scope, animation) else {
        return;
    };
    let Ok((runtime, handle)) = node_runtime_and_handle_from_object(scope, target) else {
        return;
    };
    let values = crate::native_bridge::element::sample_animation_properties_through(
        unsafe { &*runtime },
        handle,
        &instance,
    );
    crate::native_bridge::element::commit_animation_style_values(scope, runtime, handle, &values);
}

fn settle_finished_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
) {
    if native::private_bool(scope, animation, ANIMATION_FINISHED_SETTLED_SLOT) {
        return;
    }
    set_private_value(
        scope,
        animation,
        ANIMATION_FINISHED_SETTLED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    if let Some(resolver) = native::resolver(scope, animation, ANIMATION_FINISHED_RESOLVER_SLOT) {
        set_private_value(
            scope,
            resolver.get_promise(scope).into(),
            ANIMATION_PROMISE_OWNER_SLOT,
            v8::undefined(scope).into(),
        );
        let _ = resolver.resolve(scope, animation.into());
    }
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
    let state = animation_bindings::instance(scope, animation)
        .map(|a| {
            a.borrow()
                .playback
                .play_state(native::animation_time(scope))
        })
        .unwrap_or("idle");
    v8str(scope, state).into()
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
    if let Some(instance) = animation_bindings::instance(scope, animation) {
        native::sync_finished_promise(scope, animation, &instance);
    }
    set_private_value(
        scope,
        animation,
        ANIMATION_FINISHED_EXPOSED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    get_private_value(scope, animation, ANIMATION_FINISHED_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into())
}

fn set_animation_effect_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    if native::validate_effect(scope, value) {
        native::replace_effect(scope, animation, value);
    }
}

fn set_animation_timeline_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    if !native::validate_timeline(scope, value) {
        return;
    }
    let attached = v8::Local::<v8::Object>::try_from(value)
        .ok()
        .is_some_and(|timeline| native::timeline_is_active(scope, timeline));
    let specified = !value.is_null_or_undefined();
    let value = if specified {
        value
    } else {
        v8::null(scope).into()
    };
    let Some(instance) = animation_bindings::instance(scope, animation) else {
        return;
    };
    let previous_origin = native::animation_origin(scope, animation);
    let next_origin = v8::Local::<v8::Object>::try_from(value)
        .ok()
        .and_then(|timeline| native::timeline_origin(scope, timeline));
    if let Some(origin) = next_origin {
        native::set_animation_origin(scope, animation, origin);
        if let Some(previous) = previous_origin {
            instance.borrow_mut().playback.rebase_timeline(
                previous,
                origin,
                native::animation_time(scope),
            );
        }
    }
    {
        let mut state = instance.borrow_mut();
        state.timeline_document = v8::Local::<v8::Object>::try_from(value)
            .ok()
            .and_then(|timeline| native::timeline_document(scope, timeline));
    }
    set_private_value(scope, animation, ANIMATION_TIMELINE_SLOT, value);
    native::apply_command(
        scope,
        animation,
        &instance,
        moli_animation::AnimationCommand::AttachTimeline(attached),
        "Cannot set the animation timeline",
    );
}

fn set_animation_start_time_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let number = if value.is_null_or_undefined() {
        None
    } else {
        match crate::webidl::convert::<crate::webidl::Double>(
            scope,
            value,
            crate::webidl::Context::member("Animation", "startTime"),
        ) {
            Ok(v) => Some(v.0),
            Err(e) => {
                crate::webidl::throw_error(scope, &e);
                return;
            }
        }
    };
    let Some(instance) = animation_bindings::instance(scope, animation) else {
        return;
    };
    let Some(origin) = native::animation_origin(scope, animation) else {
        return;
    };
    native::apply_command(
        scope,
        animation,
        &instance,
        moli_animation::AnimationCommand::SetStartTime {
            origin,
            seconds: number.map(|v| v / 1000.0),
        },
        "Cannot set the animation start time",
    );
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
    crate::context_bootstrap::simple_object_event_set_handler(
        scope,
        animation,
        "finish",
        ANIMATION_ONFINISH_SLOT,
        value,
    );
}

fn update_effect_owner<'s>(scope: &mut v8::PinScope<'s, '_>, instance: &Animation) {
    let Some(common) = crate::common_runtime::CommonRuntimeState::from_scope(scope) else {
        return;
    };
    let objects = common.animations.wrappers(scope, instance);
    for object in objects {
        native::register_instance(scope, object, instance);
    }
}

fn keyframe_effect_update_timing_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_effect_receiver(scope, args.this()) {
        return;
    }
    let Some(instance) = timing::parse_timing(scope, args.get(0), Some(args.this())) else {
        return;
    };
    update_effect_owner(scope, &instance);
    rv.set_undefined();
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct EffectTimingSnapshot {
    #[webapi(data_property, enumerable)]
    delay: f64,
    #[webapi(data_property, enumerable)]
    direction: String,
    #[webapi(data_property, enumerable)]
    duration: f64,
    #[webapi(data_property, enumerable)]
    easing: String,
    #[webapi(data_property, enumerable)]
    end_delay: f64,
    #[webapi(data_property, enumerable)]
    fill: String,
    #[webapi(data_property, enumerable)]
    iteration_start: f64,
    #[webapi(data_property, enumerable)]
    iterations: f64,
}
fn effect_timing_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    instance: &Animation,
) -> Option<v8::Local<'s, v8::Object>> {
    let snapshot = {
        let state = instance.borrow();
        EffectTimingSnapshot::new(
            state.playback.delay.seconds() * 1000.0,
            state.playback.direction.clone(),
            if state.playback.duration_infinite {
                f64::INFINITY
            } else {
                state.playback.duration.as_secs_f64() * 1000.0
            },
            state.easing.css.clone(),
            state.playback.end_delay.seconds() * 1000.0,
            state.playback.fill.as_str().to_owned(),
            state.playback.iteration_start,
            state.playback.iterations,
        )
    };
    snapshot.bind(scope).ok()
}
fn keyframe_effect_get_timing_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_effect_receiver(scope, args.this()) {
        return;
    }
    if let Some(instance) = animation_bindings::instance(scope, args.this())
        && let Some(snapshot) = effect_timing_snapshot(scope, &instance)
    {
        rv.set(snapshot.into());
    }
}
fn keyframe_effect_get_computed_timing_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_effect_receiver(scope, args.this()) {
        return;
    }
    if let Some(instance) = animation_bindings::instance(scope, args.this()) {
        let Some(snapshot) = effect_timing_snapshot(scope, &instance) else {
            return;
        };
        let (end, active, local, progress) = {
            let state = instance.borrow();
            let now = native::animation_time(scope);
            (
                state.playback.end_time() * 1000.0,
                state
                    .playback
                    .active_duration()
                    .map_or(f64::INFINITY, |d| d.as_secs_f64() * 1000.0),
                state.playback.current_seconds(now).map(|v| v * 1000.0),
                state.progress(now),
            )
        };
        for (name, value) in [
            ("endTime", Some(end)),
            ("activeDuration", Some(active)),
            ("localTime", local),
            ("progress", progress),
        ] {
            let value = value
                .map(|v| v8::Number::new(scope, v).into())
                .unwrap_or_else(|| v8::null(scope).into());
            let _ = snapshot.create_data_property(scope, v8str(scope, name).into(), value);
        }
        rv.set(snapshot.into());
    }
}
fn animation_current_time_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "currentTime getter") {
        return;
    }
    let value = animation_bindings::instance(scope, args.this()).and_then(|a| {
        a.borrow()
            .playback
            .current_seconds(native::animation_time(scope))
    });
    rv.set(
        value
            .map(|v| v8::Number::new(scope, v * 1000.0).into())
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}
fn animation_current_time_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "currentTime setter") {
        return;
    }
    let Some(instance) = animation_bindings::instance(scope, args.this()) else {
        return;
    };
    if args.get(0).is_null_or_undefined() {
        if instance
            .borrow()
            .playback
            .current_seconds(native::animation_time(scope))
            .is_some()
        {
            throw_type_error(scope, "Cannot make a resolved current time unresolved");
        }
        return;
    }
    let value = match crate::webidl::convert::<crate::webidl::Double>(
        scope,
        args.get(0),
        crate::webidl::Context::member("Animation", "currentTime"),
    ) {
        Ok(v) => v.0,
        Err(e) => {
            crate::webidl::throw_error(scope, &e);
            return;
        }
    };
    native::apply_command(
        scope,
        args.this(),
        &instance,
        moli_animation::AnimationCommand::Seek(value / 1000.0),
        "Cannot set the animation current time",
    );
}
fn animation_playback_rate_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "playbackRate getter") {
        return;
    }
    if let Some(instance) = animation_bindings::instance(scope, args.this()) {
        rv.set_double(instance.borrow().playback.clock().rate);
    }
}
fn animation_playback_rate_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_animation_receiver(scope, args.this(), "playbackRate setter") {
        return;
    }
    let value = match crate::webidl::convert::<crate::webidl::Double>(
        scope,
        args.get(0),
        crate::webidl::Context::member("Animation", "playbackRate"),
    ) {
        Ok(v) => v.0,
        Err(e) => {
            crate::webidl::throw_error(scope, &e);
            return;
        }
    };
    if let Some(instance) = animation_bindings::instance(scope, args.this()) {
        native::apply_command(
            scope,
            args.this(),
            &instance,
            moli_animation::AnimationCommand::SetRate(value),
            "Cannot set the animation playback rate",
        );
    }
}
fn css_animation_name_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(instance) = animation_bindings::instance(scope, args.this()) {
        let name = instance.borrow().name.clone();
        if let Some(value) = v8::String::new(scope, &name) {
            rv.set(value.into());
        }
    }
}
fn document_timeline_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(timeline) = native::timeline_for_document(scope, args.this()) {
        rv.set(timeline.into());
    }
}
fn timeline_current_time_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !web_api_interfaces::AnimationTimeline::is_instance(scope, args.this()) {
        throw_type_error(scope, "Illegal AnimationTimeline receiver");
        return;
    }
    rv.set(v8::null(scope).into());
    let Some(document) = get_private_object(scope, args.this(), TIMELINE_DOCUMENT_SLOT) else {
        return;
    };
    let Ok((runtime, handle)) = node_runtime_and_handle_from_object(scope, document) else {
        return;
    };
    if !unsafe { &*runtime }.document_is_fully_active(handle) {
        return;
    }
    if let Some(origin) = native::timeline_origin(scope, args.this()) {
        rv.set_double(
            native::animation_time(scope)
                .saturating_duration_since(origin)
                .as_secs_f64()
                * 1000.0,
        );
    }
}

fn keyframe_effect_target_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_effect_receiver(scope, args.this()) {
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), KEYFRAME_EFFECT_TARGET_SLOT)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn keyframe_effect_target_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !require_effect_receiver(scope, args.this()) {
        return;
    }
    if !native::validate_target(scope, args.get(0)) {
        return;
    }
    let Some(instance) = animation_bindings::instance(scope, args.this()) else {
        return;
    };
    let value = if args.get(0).is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        args.get(0)
    };
    set_private_value(scope, args.this(), KEYFRAME_EFFECT_TARGET_SLOT, value);
    native::register_effect_owners(scope, &instance);
}
