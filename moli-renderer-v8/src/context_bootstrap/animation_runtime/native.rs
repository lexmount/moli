use super::*;
use crate::{
    animation_bindings::{self, WrapperKind},
    animation_model::{Animation, AnimationInstance, AnimationKind},
    common_runtime::CommonRuntimeState,
};
use std::time::Instant;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSAnimation)]
struct CssAnimationWrapper {}
#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::CSSTransition)]
struct CssTransitionWrapper {}
#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::DocumentTimeline)]
struct DocumentTimelineWrapper<'s> {
    #[webapi(slot = TIMELINE_DOCUMENT_SLOT)]
    document: v8::Local<'s, v8::Object>,
    #[webapi(slot = TIMELINE_ORIGIN_SLOT)]
    origin: f64,
}

pub(super) fn animation_time(scope: &mut v8::PinScope<'_, '_>) -> Instant {
    if let Some((_, crate::common_runtime::RuntimeExecutionContext::Window(host))) =
        crate::common_runtime::CommonRuntimeBinding::from_scope(scope)
    {
        unsafe { &*host }.animation_time()
    } else {
        Instant::now()
    }
}

fn current_document<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
    let (_, crate::common_runtime::RuntimeExecutionContext::Window(host)) =
        crate::common_runtime::CommonRuntimeBinding::from_scope(scope)?
    else {
        return None;
    };
    let runtime = unsafe { &*host };
    let target = runtime.current_window_document_task_target(scope)?;
    let document = match target.dispatch_scope() {
        crate::native_bridge::OwnerDispatchScope::Top => runtime.document_handle(),
        crate::native_bridge::OwnerDispatchScope::Child(frame) => {
            runtime.child_browsing_context_document_handle(frame)?
        }
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(id) => {
            runtime.lightweight_popup_document_handle(id)?
        }
    };
    crate::host::event_target_value(
        scope,
        host,
        crate::document_runtime::EventTargetHandle::Node(document),
    )
    .ok()
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(super) fn effect_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let effect = get_private_object(scope, animation, ANIMATION_EFFECT_SLOT)?;
    get_private_object(scope, effect, KEYFRAME_EFFECT_TARGET_SLOT)
}

pub(super) fn script_instance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    effect: v8::Local<'s, v8::Value>,
) {
    let mut state =
        AnimationInstance::new(AnimationKind::Script, String::new(), animation_time(scope));
    if let Some((_, crate::common_runtime::RuntimeExecutionContext::Window(host))) =
        crate::common_runtime::CommonRuntimeBinding::from_scope(scope)
    {
        state.execution_owner = unsafe { &*host }.current_window_document_task_target(scope);
    }
    if let Ok(effect) = v8::Local::<v8::Object>::try_from(effect) {
        if let Some(source) = animation_bindings::instance(scope, effect) {
            state.copy_effect_from(&source.borrow());
            detach_from_animation(scope, effect, &source);
        }
        state.playback.has_effect = true;
        let instance = state.shared();
        animation_bindings::bind(scope, effect, instance.clone(), WrapperKind::Effect);
        animation_bindings::bind(scope, animation, instance, WrapperKind::Animation);
    } else {
        animation_bindings::bind(scope, animation, state.shared(), WrapperKind::Animation);
    }
}

pub(super) fn register_instance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    instance: &Animation,
) {
    let target = effect_target(scope, object).or_else(|| current_document(scope));
    let Some(target) = target else {
        return;
    };
    let Ok((runtime, handle)) = node_runtime_and_handle_from_object(scope, target) else {
        return;
    };
    let runtime = unsafe { &*runtime };
    let has_target = effect_target(scope, object).is_some();
    let (old, id, needs_service) = {
        let mut state = instance.borrow_mut();
        let old = state.registered_target;
        state.registered_target = Some(handle);
        state.target = has_target.then_some(handle);
        if state.execution_owner.is_none()
            && let Some(dispatch) = runtime.owner_dispatch_scope_for_node(handle)
        {
            state.execution_owner =
                runtime.current_window_document_task_target_for_dispatch_scope(dispatch);
        }
        (
            old,
            state.playback.id(),
            old != Some(handle) || state.service_owner.is_none(),
        )
    };
    if let Some(target) = old
        && target != handle
    {
        runtime
            .animation_transition_state()
            .remove_script_reference(target, id);
    }
    runtime
        .animation_transition_state()
        .index_effect(handle, instance);
    if needs_service {
        crate::native_bridge::element::rehome_animation_service(runtime, instance);
    }
    crate::native_bridge::element::queue_animation_instance_sample(runtime, instance);
    let owner = instance.borrow().service_owner;
    if let Some(owner) = owner {
        runtime
            .animation_transition_state()
            .document_mut(owner.owner())
            .activate(instance);
        runtime.queue_animation_timeline_tick_for_owner(owner);
    }
    element_registry::unregister(scope, object);
    element_registry::register_for_target(scope, object, target);
    runtime.invalidate_animation_layout();
    sync_finished_promise(scope, object, instance);
}

pub(super) fn timeline_for_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    const SLOT: &str = "__moliDocumentTimeline";
    if let Some(timeline) = get_private_object(scope, document, SLOT) {
        return Some(timeline);
    }
    let context = document.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let fallback = CommonRuntimeState::from_context(context)?.animations.origin;
    let origin = node_runtime_and_handle_from_object(scope, document)
        .ok()
        .and_then(|(runtime, handle)| {
            let runtime = unsafe { &*runtime };
            runtime
                .child_document_animation_origin(handle)
                .or_else(|| runtime.popup_document_animation_origin(handle))
        })
        .unwrap_or(fallback);
    let timeline = DocumentTimelineWrapper::new(document, origin_offset(origin, fallback))
        .bind(scope)
        .ok()?;
    set_private_value(scope, document, SLOT, timeline.into());
    Some(timeline)
}

fn origin_offset(origin: Instant, base: Instant) -> f64 {
    if origin >= base {
        origin.duration_since(base).as_secs_f64()
    } else {
        -base.duration_since(origin).as_secs_f64()
    }
}
fn object_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<Instant> {
    let base = object
        .get_creation_context(scope)
        .and_then(CommonRuntimeState::from_context)?
        .animations
        .origin;
    let offset = get_private_value(scope, object, slot)
        .and_then(|v| v8::Local::<v8::Number>::try_from(v).ok())
        .map_or(0.0, |v| v.value());
    let duration = std::time::Duration::try_from_secs_f64(offset.abs()).ok()?;
    if offset < 0.0 {
        base.checked_sub(duration)
    } else {
        base.checked_add(duration)
    }
}
pub(super) fn timeline_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    timeline: v8::Local<'s, v8::Object>,
) -> Option<Instant> {
    object_origin(scope, timeline, TIMELINE_ORIGIN_SLOT)
}
pub(super) fn animation_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
) -> Option<Instant> {
    object_origin(scope, animation, ANIMATION_TIMELINE_ORIGIN_SLOT)
}
pub(super) fn set_animation_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    animation: v8::Local<'s, v8::Object>,
    origin: Instant,
) {
    let Some(base) = animation
        .get_creation_context(scope)
        .and_then(CommonRuntimeState::from_context)
        .map(|c| c.animations.origin)
    else {
        return;
    };
    let offset = v8::Number::new(scope, origin_offset(origin, base));
    set_private_value(
        scope,
        animation,
        ANIMATION_TIMELINE_ORIGIN_SLOT,
        offset.into(),
    );
}
pub(super) fn timeline_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    timeline: v8::Local<'s, v8::Object>,
) -> Option<crate::document_runtime::DomHandle> {
    let document = get_private_object(scope, timeline, TIMELINE_DOCUMENT_SLOT)?;
    node_runtime_and_handle_from_object(scope, document)
        .ok()
        .map(|(_, handle)| handle)
}

pub(super) fn timeline_is_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    timeline: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_object(scope, timeline, TIMELINE_DOCUMENT_SLOT)
        .and_then(|document| node_runtime_and_handle_from_object(scope, document).ok())
        .is_some_and(|(runtime, handle)| unsafe { &*runtime }.document_is_fully_active(handle))
}

pub(super) fn default_timeline<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    current_document(scope)
        .and_then(|document| timeline_for_document(scope, document))
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into())
}

pub(super) fn wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    instance: &Animation,
) -> Option<v8::Local<'s, v8::Object>> {
    let realm = target.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, realm);
    let common = CommonRuntimeState::from_scope(scope)?;
    if let Some(object) = common
        .animations
        .wrapper(scope, instance, WrapperKind::Animation)
    {
        return Some(object);
    }
    let object = match instance.borrow().kind {
        AnimationKind::CssAnimation => CssAnimationWrapper::new().bind(scope).ok()?,
        AnimationKind::CssTransition => CssTransitionWrapper::new().bind(scope).ok()?,
        AnimationKind::Script => {
            return common
                .animations
                .wrappers(scope, instance)
                .into_iter()
                .next();
        }
    };
    let effect = KeyframeEffectObjectDeclaration::new(target.into())
        .bind(scope)
        .ok()?;
    animation_bindings::bind(scope, effect, instance.clone(), WrapperKind::Effect);
    let timeline = default_timeline(scope);
    initialize_animation_object(scope, object, effect.into(), timeline);
    if let Ok(timeline) = v8::Local::<v8::Object>::try_from(timeline)
        && let Some(origin) = timeline_origin(scope, timeline)
    {
        set_animation_origin(scope, object, origin);
        instance.borrow_mut().timeline_origin = Some(origin);
    }
    animation_bindings::bind(scope, object, instance.clone(), WrapperKind::Animation);
    element_registry::register(scope, object);
    resolve_ready(scope, object);
    sync_finished_promise(scope, object, instance);
    let finished = {
        let state = instance.borrow();
        state.published_finished_generation == Some(state.playback.finished_generation())
            && state.playback.play_state(animation_time(scope)) == "finished"
    };
    if finished {
        settle_finished_promise(scope, object);
    }
    Some(object)
}

pub(super) fn promise_execution_is_current<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(realm) = object.get_creation_context(scope) else {
        return false;
    };
    let scope = &mut v8::ContextScope::new(scope, realm);
    let Some((_, crate::common_runtime::RuntimeExecutionContext::Window(host))) =
        crate::common_runtime::CommonRuntimeBinding::from_scope(scope)
    else {
        return false;
    };
    let host = unsafe { &*host };
    host.window_execution_context_identity_for_v8_context(scope, realm)
        .is_some_and(|identity| host.window_execution_context_identity_is_current(identity))
}

pub(crate) fn notify_animation_playback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    playback: &crate::animation_model::PlaybackEvent,
    publish: bool,
) -> bool {
    let instance = &playback.animation;
    let notification = playback.notification;
    let event_realm = scope.get_current_context();
    let kind = notification.kind.as_str();
    let current = {
        let state = instance.borrow();
        state.playback.accepts(notification)
            && (kind != "finish" || state.playback.play_state(animation_time(scope)) == "finished")
    };
    let queued = publish && current && kind == "finish";
    if queued {
        let generation = instance.borrow().playback.finished_generation();
        instance.borrow_mut().published_finished_generation = Some(generation);
    }
    let Some(common) = CommonRuntimeState::from_scope(scope) else {
        return queued;
    };
    let wrappers = common.animations.wrappers(scope, instance);
    for object in wrappers {
        let can_publish_promises = promise_execution_is_current(scope, object);
        let Some(realm) = object.get_creation_context(scope) else {
            continue;
        };
        if CommonRuntimeState::from_context(realm).is_none() {
            continue;
        }
        let scope = &mut v8::ContextScope::new(scope, realm);
        if publish {
            // Dropping an effect's native registration does not execute JS and
            // must also happen when its promise realm has retired.
            if current
                && !instance
                    .borrow()
                    .playback
                    .is_relevant(animation_time(scope))
            {
                element_registry::unregister(scope, object);
            }
            // Promise reactions remain owned by the wrapper's execution realm.
            // DOM event callbacks have their own relevant-realm checks in the
            // shared dispatcher and can belong to a still-live parent page.
            if !can_publish_promises {
                continue;
            }
            sync_finished_promise(scope, object, instance);
            if !current {
                continue;
            }
            if kind == "release" {
                continue;
            }
            if kind == "ready" {
                apply_promise_update(scope, object, notification.promise_update().unwrap());
                continue;
            }
            if kind == "reset" {
                continue;
            }
            if kind == "finish" {
                settle_finished_promise(scope, object);
            } else {
                apply_promise_update(scope, object, notification.promise_update().unwrap());
            }
            continue;
        }
        if !matches!(kind, "finish" | "cancel") {
            continue;
        }
        // The rendering task's Document has already passed its owner check.
        // Create the event in that live realm, not in a retired creator realm.
        let scope = &mut v8::ContextScope::new(scope, event_realm);
        if let Ok(event) = crate::host::create_animation_playback_event(
            scope,
            kind,
            object.into(),
            playback.current_time,
            playback.timeline_time,
        ) {
            crate::context_bootstrap::dispatch_simple_event_target_event(
                scope, object, kind, event,
            );
        }
    }
    queued
}

// A queued finish event belongs to its original batch, but its promise
// settlement must never target a later finished promise on the same wrapper.
pub(super) fn sync_finished_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    instance: &Animation,
) {
    let generation = instance.borrow().playback.finished_generation();
    let previous = get_private_value(scope, object, ANIMATION_FINISHED_GENERATION_SLOT)
        .and_then(|v| v8::Local::<v8::BigInt>::try_from(v).ok())
        .map_or(0, |v| v.u64_value().0);
    if previous != generation {
        if private_bool(scope, object, ANIMATION_FINISHED_SETTLED_SLOT) {
            renew_finished_promise(scope, object);
        }
        let value = v8::BigInt::new_from_u64(scope, generation);
        set_private_value(
            scope,
            object,
            ANIMATION_FINISHED_GENERATION_SLOT,
            value.into(),
        );
    }
}

pub(super) fn private_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> bool {
    get_private_value(scope, object, slot).is_some_and(|v| v.boolean_value(scope))
}

pub(super) fn renew_finished_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    let Some(realm) = object.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, realm);
    let resolver = v8::PromiseResolver::new(scope).expect("Animation finished resolver");
    let promise = resolver.get_promise(scope);
    let unset = v8::Boolean::new(scope, false);
    set_private_value(scope, object, ANIMATION_FINISHED_SLOT, promise.into());
    set_private_value(
        scope,
        object,
        ANIMATION_FINISHED_RESOLVER_SLOT,
        resolver.into(),
    );
    set_private_value(
        scope,
        promise.into(),
        ANIMATION_PROMISE_OWNER_SLOT,
        object.into(),
    );
    set_private_value(scope, object, ANIMATION_FINISHED_EXPOSED_SLOT, unset.into());
    set_private_value(scope, object, ANIMATION_FINISHED_SETTLED_SLOT, unset.into());
}

pub(super) fn resolver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let value = get_private_value(scope, object, slot)?;
    // SAFETY: only PromiseResolver::new populates these inaccessible private
    // slots. They never accept a script-supplied value.
    Some(unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(value) })
}

pub(super) fn resolve_ready<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    if let Some(resolver) = resolver(scope, object, ANIMATION_READY_RESOLVER_SLOT) {
        set_private_value(
            scope,
            resolver.get_promise(scope).into(),
            ANIMATION_PROMISE_OWNER_SLOT,
            v8::undefined(scope).into(),
        );
        let _ = resolver.resolve(scope, object.into());
    }
}

pub(super) fn apply_command<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    instance: &Animation,
    command: moli_animation::AnimationCommand,
    error_message: &'static str,
) -> bool {
    let result = instance
        .borrow_mut()
        .command(command, animation_time(scope));
    let Ok(outcome) = result else {
        let error = crate::context_bootstrap::new_dom_exception_value(
            scope,
            error_message,
            "InvalidStateError",
        );
        scope.throw_exception(error);
        return false;
    };
    register_instance(scope, object, instance);
    let settles_promise = matches!(
        outcome.ready,
        moli_animation::ReadyPromiseUpdate::Resolve | moli_animation::ReadyPromiseUpdate::Reject
    ) || outcome.finished != moli_animation::FinishedPromiseUpdate::Unchanged;
    if settles_promise && !promise_execution_is_current(scope, object) {
        return true;
    }
    apply_promise_update(scope, object, outcome);
    if outcome.finished == moli_animation::FinishedPromiseUpdate::Resolve {
        let generation = instance.borrow().playback.finished_generation();
        instance.borrow_mut().published_finished_generation = Some(generation);
    }
    true
}

fn apply_promise_update<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    outcome: moli_animation::CommandOutcome,
) {
    if let Some(generation) = outcome.cancellation {
        let previous = get_private_value(scope, object, ANIMATION_CANCEL_GENERATION_SLOT)
            .and_then(|v| v8::Local::<v8::BigInt>::try_from(v).ok())
            .map(|v| v.u64_value().0);
        if previous == Some(generation) {
            return;
        }
        let value = v8::BigInt::new_from_u64(scope, generation);
        set_private_value(
            scope,
            object,
            ANIMATION_CANCEL_GENERATION_SLOT,
            value.into(),
        );
    }
    apply_ready_update(scope, object, outcome.ready);
    match outcome.finished {
        moli_animation::FinishedPromiseUpdate::Unchanged => {}
        moli_animation::FinishedPromiseUpdate::Resolve => {
            settle_finished_promise(scope, object);
        }
        moli_animation::FinishedPromiseUpdate::RejectAndRenew => {
            cancel_finished_promise(scope, object)
        }
    }
}

fn apply_ready_update<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    update: moli_animation::ReadyPromiseUpdate,
) {
    match update {
        moli_animation::ReadyPromiseUpdate::Pending => pending_ready(scope, object),
        moli_animation::ReadyPromiseUpdate::Resolve => resolve_ready(scope, object),
        moli_animation::ReadyPromiseUpdate::Reject => {
            if let Some(resolver) = resolver(scope, object, ANIMATION_READY_RESOLVER_SLOT) {
                set_private_value(
                    scope,
                    resolver.get_promise(scope).into(),
                    ANIMATION_PROMISE_OWNER_SLOT,
                    v8::undefined(scope).into(),
                );
                let error = crate::context_bootstrap::new_dom_exception_value(
                    scope,
                    "Animation canceled",
                    "AbortError",
                );
                resolver.get_promise(scope).mark_as_handled();
                let _ = resolver.reject(scope, error);
            }
            pending_ready(scope, object);
            resolve_ready(scope, object);
        }
        moli_animation::ReadyPromiseUpdate::Unchanged => {}
    }
}

fn pending_ready<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>) {
    let Some(realm) = object.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, realm);
    let resolver = v8::PromiseResolver::new(scope).expect("Animation ready resolver");
    let promise = resolver.get_promise(scope);
    set_private_value(scope, object, ANIMATION_READY_SLOT, promise.into());
    set_private_value(
        scope,
        object,
        ANIMATION_READY_RESOLVER_SLOT,
        resolver.into(),
    );
    set_private_value(
        scope,
        promise.into(),
        ANIMATION_PROMISE_OWNER_SLOT,
        object.into(),
    );
}

pub(super) fn cancel_finished_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    if let Some(resolver) = resolver(scope, object, ANIMATION_FINISHED_RESOLVER_SLOT) {
        set_private_value(
            scope,
            resolver.get_promise(scope).into(),
            ANIMATION_PROMISE_OWNER_SLOT,
            v8::undefined(scope).into(),
        );
        if private_bool(scope, object, ANIMATION_FINISHED_EXPOSED_SLOT) {
            let error = crate::context_bootstrap::new_dom_exception_value(
                scope,
                "The animation was canceled",
                "AbortError",
            );
            resolver.get_promise(scope).mark_as_handled();
            let _ = resolver.reject(scope, error);
        }
    }
    renew_finished_promise(scope, object);
}

pub(super) fn validate_effect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    if value.is_null_or_undefined() {
        return true;
    }
    if v8::Local::<v8::Object>::try_from(value)
        .ok()
        .is_some_and(|o| {
            web_api_interfaces::KeyframeEffect::is_instance(scope, o)
                && animation_bindings::instance(scope, o).is_some()
        })
    {
        return true;
    }
    throw_type_error(scope, "Animation.effect must be a KeyframeEffect or null");
    false
}
pub(super) fn validate_timeline<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    if value.is_null_or_undefined() {
        return true;
    }
    if v8::Local::<v8::Object>::try_from(value)
        .ok()
        .is_some_and(|o| web_api_interfaces::AnimationTimeline::is_instance(scope, o))
    {
        return true;
    }
    throw_type_error(
        scope,
        "Animation.timeline must be an AnimationTimeline or null",
    );
    false
}
pub(super) fn validate_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    if value.is_null_or_undefined() {
        return true;
    }
    if v8::Local::<v8::Object>::try_from(value)
        .ok()
        .is_some_and(|o| web_api_interfaces::Element::is_instance(scope, o))
    {
        return true;
    }
    throw_type_error(scope, "KeyframeEffect.target must be an Element or null");
    false
}

/// Moving an effect detaches its previous Animation without canceling that
/// Animation's clock. The effect configuration has already been copied out.
fn detach_from_animation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    effect: v8::Local<'s, v8::Object>,
    instance: &Animation,
) {
    let Some(common) = CommonRuntimeState::from_scope(scope) else {
        return;
    };
    for object in common.animations.wrappers(scope, instance) {
        if get_private_object(scope, object, ANIMATION_EFFECT_SLOT) != Some(effect) {
            continue;
        }
        set_private_value(scope, object, ANIMATION_EFFECT_SLOT, v8::null(scope).into());
        instance.borrow_mut().playback.has_effect = false;
        register_instance(scope, object, instance);
    }
}
pub(super) fn replace_effect<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(instance) = animation_bindings::instance(scope, object) else {
        return;
    };
    let effect = v8::Local::<v8::Object>::try_from(value).ok();
    let old = get_private_object(scope, object, ANIMATION_EFFECT_SLOT);
    if old == effect {
        return;
    }
    if let Some(old) = old {
        let detached = instance.borrow().detached_effect();
        animation_bindings::bind(scope, old, detached, WrapperKind::Effect);
    }
    if let Some(effect) = effect {
        let Some(source) = animation_bindings::instance(scope, effect) else {
            return;
        };
        let configuration = source.borrow().detached_effect();
        detach_from_animation(scope, effect, &source);
        instance
            .borrow_mut()
            .copy_effect_from(&configuration.borrow());
        animation_bindings::bind(scope, effect, instance.clone(), WrapperKind::Effect);
    } else {
        let mut state = instance.borrow_mut();
        state.playback.has_effect = false;
        state.target = None;
    }
    instance.borrow_mut().css_effect_source = crate::animation_model::CssEffectSource::ScriptEffect;
    set_private_value(
        scope,
        object,
        ANIMATION_EFFECT_SLOT,
        effect
            .map(Into::into)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
    register_instance(scope, object, &instance);
}
pub(super) fn register_effect_owners<'s>(scope: &mut v8::PinScope<'s, '_>, instance: &Animation) {
    let Some(common) = CommonRuntimeState::from_scope(scope) else {
        return;
    };
    for object in common.animations.wrappers(scope, instance) {
        register_instance(scope, object, instance);
    }
}
