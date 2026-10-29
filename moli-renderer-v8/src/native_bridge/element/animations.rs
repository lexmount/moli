use moli_animation::{AnimationPhase, CssTime, FillMode, SamplingMode};
use std::time::{Duration, Instant};

use crate::animation_model::{
    Animation, AnimationInstance, AnimationKind, CssLifecycleEvent, Easing,
};
use crate::document_runtime::{DomHandle, EventTargetHandle};
use crate::native_bridge::context_host::RunningTransition;
use crate::native_bridge::{JsContextHost, WindowDocumentOwner};

use super::ComputedStyleRead;
use super::events::dispatch_public_event;

pub(crate) fn queue_animation_start_for_listener_target(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    target: EventTargetHandle,
) {
    let _ = unsafe { &mut *runtime_ptr }.queue_animation_start_scan(scope, target);
}

/// Inline admission is conservative for escapes and `all: inherit`. Installed
/// stylesheets have their own parsed, revision-scoped classification.
fn css_lifecycle_source_may_apply(text: &str) -> bool {
    text.contains('\\')
        || [b"animation".as_slice(), b"transition", b"all"]
            .iter()
            .any(|word| {
                text.as_bytes()
                    .windows(word.len())
                    .any(|s| s.eq_ignore_ascii_case(word))
            })
}

pub(crate) fn document_may_have_css_lifecycle(
    runtime: &JsContextHost,
    document: DomHandle,
    owner: WindowDocumentOwner,
) -> bool {
    runtime
        .animation_transition_state()
        .document(owner)
        .is_some_and(|state| {
            state.elements.values().any(|element| {
                !element.animations.is_empty() || !element.running_transitions.is_empty()
            })
        })
        || runtime.document_has_css_lifecycle_style_sources(document)
}

pub(crate) fn element_tree_has_inline_css_lifecycle(
    runtime: &JsContextHost,
    root: DomHandle,
) -> bool {
    document_elements_in_tree_order(runtime, root)
        .into_iter()
        .any(|handle| {
            runtime
                .dom_host()
                .get_attribute(handle, "style")
                .is_some_and(|text| css_lifecycle_source_may_apply(&text))
        })
}

struct TransitionDeclaration {
    property: std::borrow::Cow<'static, str>,
    allow_discrete: bool,
    delay: CssTime,
    duration: Duration,
    easing: std::rc::Rc<Easing>,
}

fn transition_declarations(
    style: &ComputedStyleRead<'_>,
    durations: &[Duration],
    delays: &[CssTime],
) -> Vec<TransitionDeclaration> {
    let mut easings = crate::css_style::top_level_comma_separated_component_values(
        &style.property("transition-timing-function"),
    )
    .unwrap_or_default()
    .into_iter()
    .map(|css| std::rc::Rc::new(Easing::parse(&css).unwrap_or_else(Easing::linear)))
    .collect::<Vec<_>>();
    if easings.is_empty() {
        easings.push(std::rc::Rc::new(Easing::linear()));
    }
    let behavior = style.property("transition-behavior");
    let behaviors = css_list(&behavior).collect::<Vec<_>>();
    let raw = style.property("transition-property");
    let declared = css_list(&raw).collect::<Vec<_>>();
    let writing_mode = style
        .style_snapshot()
        .map(|s| s.computed_values().writing_mode)
        .unwrap_or_else(style::logical_geometry::WritingMode::horizontal_tb);
    crate::style_engine::transition_property_longhands(&declared, writing_mode)
        .into_iter()
        .map(|(property, index)| TransitionDeclaration {
            allow_discrete: cyclic_list_item(&behaviors, index)
                .is_some_and(|v| v.eq_ignore_ascii_case("allow-discrete")),
            delay: cyclic_list_item(delays, index).unwrap_or_default(),
            duration: cyclic_list_item(durations, index).unwrap_or_default(),
            easing: easings[index % easings.len()].clone(),
            property,
        })
        .collect()
}

fn css_time_list(value: &str) -> Vec<Duration> {
    css_list(value)
        .map(|value| parse_css_time(value).unwrap_or_default())
        .collect()
}

fn css_delay_list(value: &str) -> Vec<CssTime> {
    css_list(value)
        .map(|value| {
            parse_css_seconds(value)
                .and_then(CssTime::from_seconds)
                .unwrap_or_default()
        })
        .collect()
}

fn cyclic_list_item<T: Copy>(items: &[T], index: usize) -> Option<T> {
    if items.is_empty() {
        None
    } else {
        Some(items[index % items.len()])
    }
}

fn css_list(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
}

fn lifecycle_owner_is_current(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    owner: WindowDocumentOwner,
) -> bool {
    unsafe { &*runtime_ptr }
        .current_window_document_task_target(scope)
        .map(|target| target.owner())
        == Some(owner)
}

pub(crate) fn animation_owner_for_node(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<WindowDocumentOwner> {
    let dispatch = runtime.owner_dispatch_scope_for_node(handle)?;
    Some(
        runtime
            .current_window_document_task_target_for_dispatch_scope(dispatch)?
            .owner(),
    )
}

/// Ordinary style updates reconcile declarations into the same instances that
/// getters and script methods observe. They collect values, never author callbacks.
pub(crate) fn animation_style_may_apply(runtime: &JsContextHost, handle: DomHandle) -> bool {
    if runtime.animation_transition_state().has_effects()
        || runtime
            .dom_host()
            .owner_document_handle(handle)
            .is_some_and(|d| runtime.document_has_css_lifecycle_style_sources(d))
    {
        return true;
    }
    // Inline effects on a flat-tree ancestor may be first observed through
    // its descendant. No full Document scan is needed for this admission.
    let mut current = Some(handle);
    while let Some(node) = current {
        if runtime
            .dom_host()
            .get_attribute(node, "style")
            .is_some_and(|s| css_lifecycle_source_may_apply(&s))
        {
            return true;
        }
        current = runtime.dom_host().flat_tree_parent(node);
    }
    false
}

pub(crate) fn observe_animation_style(runtime: &JsContextHost, style: &ComputedStyleRead<'_>) {
    let handle = style.handle_for_animation();
    if !runtime.animation_transition_state().has_effects()
        && !runtime
            .dom_host()
            .owner_document_handle(handle)
            .is_some_and(|document| runtime.document_has_css_lifecycle_style_sources(document))
        && !runtime
            .dom_host()
            .get_attribute(handle, "style")
            .is_some_and(|text| css_lifecycle_source_may_apply(&text))
    {
        return;
    }
    let Some(owner) = animation_owner_for_node(runtime, handle) else {
        return;
    };
    {
        let document = runtime.animation_transition_state().document_mut(owner);
        if document.reconciling_style {
            return;
        }
        if let (Some(before), Some(after)) =
            (document.before_styles.get(&handle), style.style_snapshot())
            && style::servo_arc::Arc::ptr_eq(&before.computed_values(), &after.computed_values())
        {
            return;
        }
    }
    let has_effect = runtime
        .animation_transition_state()
        .document(owner)
        .is_some_and(|d| d.elements.contains_key(&handle));
    let source = runtime
        .dom_host()
        .owner_document_handle(handle)
        .is_some_and(|d| runtime.document_has_css_lifecycle_style_sources(d));
    let inline = runtime
        .dom_host()
        .get_attribute(handle, "style")
        .is_some_and(|s| css_lifecycle_source_may_apply(&s));
    if !has_effect && !source && !inline {
        return;
    }
    let now = runtime.animation_time();
    let changed_at = {
        let mut document = runtime.animation_transition_state().document_mut(owner);
        document.reconciling_style = true;
        document.begin_style_sample(now)
    };
    let rendered = style.css_lifecycle_is_rendered();
    let before_samples = capture_transition_animation_snapshot(runtime, handle, now);
    let events = scan_element_animations(runtime, style, rendered, handle, now, changed_at, owner)
        .into_iter()
        .chain(scan_element_transitions(
            runtime,
            style,
            rendered,
            handle,
            now,
            owner,
            &before_samples,
        ))
        .map(|event| (handle, event))
        .collect::<Vec<_>>();
    let mut document = runtime.animation_transition_state().document_mut(owner);
    document.pending_events.extend(events);
    document.reconciling_style = false;
    let effects = document
        .elements
        .get(&handle)
        .map(|s| {
            s.animations
                .iter()
                .chain(s.running_transitions.iter().map(|t| &t.animation))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for effect in effects {
        document.pending_events.extend(
            observe_animation_phase(&effect, now)
                .into_iter()
                .map(|event| (handle, event)),
        );
        if effect.borrow().playback.needs_timeline_service(now) {
            document.activate(&effect);
        }
    }
    let needs_work = !document.pending_events.is_empty() || document.engine.is_active();
    document.finish_observed_style_sample();
    drop(document);
    if needs_work {
        runtime.queue_animation_timeline_tick(handle);
    }
}

/// Effect target lookup is independent of its timeline's Document owner. CSS
/// associations supply declaration order; the weak index supplies script and
/// retargeted effects without becoming a second owner of their lifetime.
fn effects_for_element(runtime: &JsContextHost, handle: DomHandle) -> Vec<Animation> {
    if !runtime.animation_transition_state().has_effects() {
        return Vec::new();
    }
    let mut css = Vec::new();
    let mut transitions = Vec::new();
    if let Some(owner) = animation_owner_for_node(runtime, handle)
        && let Some(document) = runtime.animation_transition_state().document(owner)
        && let Some(state) = document.elements.get(&handle)
    {
        css.extend(state.animations.iter().cloned());
        transitions.extend(
            state
                .running_transitions
                .iter()
                .map(|t| t.animation.clone()),
        );
    }
    css.extend(
        runtime
            .animation_transition_state()
            .effects_for_target(handle),
    );
    css.extend(transitions);
    let mut seen = std::collections::HashSet::new();
    css.retain(|a| {
        let a = a.borrow();
        a.target == Some(handle) && seen.insert(a.playback.id())
    });
    css.sort_by_key(|a| animation_composite_order(&a.borrow()));
    css
}

/// CSS ownership survives effect retargeting. Read through the owning element's
/// ordinary style boundary without holding an instance borrow across reconciliation.
pub(crate) fn synchronize_css_animation_style(runtime: &JsContextHost, animation: &Animation) {
    let owner = {
        let state = animation.borrow();
        state.css_owner.filter(|_| state.owned_by_css)
    };
    if let Some(owner) = owner {
        let _style = ComputedStyleRead::new(runtime, owner);
    }
}

fn animation_composite_order(animation: &AnimationInstance) -> (u8, u64) {
    (
        match animation.composite_kind() {
            AnimationKind::CssAnimation => 0,
            AnimationKind::Script => 1,
            AnimationKind::CssTransition => 2,
        },
        animation.composite_order(),
    )
}

/// Resolve the complete dependency context through this effect before
/// extracting its properties. Higher effects never enter the projection.
pub(crate) fn sample_animation_properties_through(
    runtime: &JsContextHost,
    handle: DomHandle,
    animation: &Animation,
) -> Vec<(String, String)> {
    let cutoff = animation_composite_order(&animation.borrow());
    let committing = animation.borrow().playback.id();
    let mut effects = effects_for_element(runtime, handle);
    // A no-fill effect may have left the target index after finishing. Its
    // wrapper still owns it and commitStyles must include its exact endpoint.
    if !effects
        .iter()
        .any(|effect| std::rc::Rc::ptr_eq(effect, animation))
    {
        effects.push(animation.clone());
        effects.sort_by_key(|effect| animation_composite_order(&effect.borrow()));
    }
    let end = effects.partition_point(|a| animation_composite_order(&a.borrow()) <= cutoff);
    let samples = sample_effect_values_for(
        runtime,
        &effects[..end],
        runtime.animation_time(),
        Some(committing),
    );
    if samples.is_empty() {
        return Vec::new();
    }
    let read = ComputedStyleRead::new(runtime, handle);
    let Some(projected) = read.project_animation_samples(&samples) else {
        return Vec::new();
    };
    animation
        .borrow()
        .frames
        .iter()
        .filter_map(|frame| {
            projected
                .property_value(&frame.property)
                .map(|value| (frame.property.clone(), value))
        })
        .collect()
}

pub(crate) fn sample_animation_property(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    sample_property_from_effects(
        runtime,
        handle,
        property,
        &effects_for_element(runtime, handle),
    )
}

fn sample_property_from_effects(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    effects: &[Animation],
) -> Option<String> {
    let samples = sample_effect_values(runtime, effects, runtime.animation_time());
    if !samples.iter().any(|sample| sample.property == property) {
        return None;
    }
    let style = ComputedStyleRead::new(runtime, handle);
    sample_property_from_intervals(&style, property, &samples)
}

fn sample_property_from_interval(
    style: &ComputedStyleRead<'_>,
    sample: crate::animation_model::SpecifiedSample,
) -> Option<String> {
    let sample = crate::animation_model::EffectSample {
        value: sample,
        kind: AnimationKind::CssTransition,
    };
    sample_property_from_intervals(style, &sample.property, std::slice::from_ref(&sample))
}

fn sample_property_from_intervals(
    style: &ComputedStyleRead<'_>,
    property: &str,
    samples: &[crate::animation_model::EffectSample],
) -> Option<String> {
    if samples.is_empty() {
        return None;
    }
    style
        .project_animation_samples(samples)?
        .property_value(property)
}

pub(crate) fn sample_animation_values(
    runtime: &JsContextHost,
    handle: DomHandle,
    now: Instant,
) -> Vec<crate::animation_model::EffectSample> {
    sample_effect_values(runtime, &effects_for_element(runtime, handle), now)
}

// Captured before CSS animation reconciliation. Transition comparison projects
// this same old configuration on both canonical style baselines.
struct TransitionAnimationSnapshot {
    values: Vec<crate::animation_model::EffectSample>,
}

fn capture_transition_animation_snapshot(
    runtime: &JsContextHost,
    handle: DomHandle,
    now: Instant,
) -> TransitionAnimationSnapshot {
    let effects = effects_for_element(runtime, handle)
        .into_iter()
        .filter(|effect| effect.borrow().composite_kind() != AnimationKind::CssTransition)
        .collect::<Vec<_>>();
    TransitionAnimationSnapshot {
        values: sample_effect_values(runtime, &effects, now),
    }
}

fn sample_effect_values(
    runtime: &JsContextHost,
    effects: &[Animation],
    now: Instant,
) -> Vec<crate::animation_model::EffectSample> {
    sample_effect_values_for(runtime, effects, now, None)
}
fn sample_effect_values_for(
    runtime: &JsContextHost,
    effects: &[Animation],
    now: Instant,
    committing: Option<moli_animation::AnimationId>,
) -> Vec<crate::animation_model::EffectSample> {
    // Stable per-property composition is independent of dependency ordering.
    let mut values = Vec::new();
    for effect in effects {
        refresh_animation_timeline(runtime, effect);
        let effect = effect.borrow();
        // Endpoint inclusion belongs to the committing animation. Lower effects
        // supply the same underlying values as ordinary presentation.
        let mode = if committing == Some(effect.playback.id()) {
            SamplingMode::CommitStyles
        } else {
            SamplingMode::Presentation
        };
        for frame in &effect.frames {
            if let Some(value) = effect.sample_interval_for(&frame.property, now, mode) {
                values.push(crate::animation_model::EffectSample {
                    value,
                    kind: effect.composite_kind(),
                });
            }
        }
    }
    values
}

pub(crate) fn animations_for_element(runtime: &JsContextHost, handle: DomHandle) -> Vec<Animation> {
    let _style = ComputedStyleRead::new(runtime, handle);
    let now = runtime.animation_time();
    let mut effects = effects_for_element(runtime, handle);
    effects.retain(|a| a.borrow().playback.is_relevant(now));
    // CSS transitions precede CSS animations in getAnimations() composite order.
    effects.sort_by_key(|a| {
        let a = a.borrow();
        (
            match a.composite_kind() {
                AnimationKind::CssTransition => 0,
                AnimationKind::CssAnimation => 1,
                AnimationKind::Script => 2,
            },
            a.composite_order(),
        )
    });
    effects
}

/// Run one coalesced CSS animation/transition rescan for the Document.
///
/// Animations that newly apply dispatch `animationstart`; animations whose
/// declared active duration has elapsed dispatch `animationend`; animations
/// removed before completion dispatch `animationcancel`. Transitions compare
/// each transitioned property against the previous scan and dispatch
/// `transitionrun`/`transitionstart`/`transitionend`/`transitioncancel`.
pub(crate) fn dispatch_animation_transition_scan(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    owner: WindowDocumentOwner,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let handles = document_elements_in_tree_order(runtime, document_handle);
    let now = runtime.animation_time();
    let _sample = runtime.animation_transition_state().enter_script_time(now);
    let mut events = Vec::new();

    // Cancellation is observable even after removal. Take the records before
    // releasing them, then publish their events with the rest of this batch.
    let connected: std::collections::HashSet<DomHandle> = handles.iter().copied().collect();
    let changed_at = {
        let runtime = unsafe { &*runtime_ptr };
        let (changed_at, removed) = {
            let mut document = runtime.animation_transition_state().document_mut(owner);
            (
                {
                    document.reconciling_style = true;
                    document.begin_style_sample(now)
                },
                document.take_disconnected(&connected),
            )
        };
        for (handle, state) in removed {
            events.extend(
                state
                    .animations
                    .into_iter()
                    .flat_map(|animation| animation_cancel_event(animation, now))
                    .map(|event| (handle, event)),
            );
            for transition in state.running_transitions {
                let mut cancelled = Vec::new();
                cancel_transition(&transition, now, &mut cancelled);
                events.extend(cancelled.into_iter().map(|event| (handle, event)));
            }
        }
        changed_at
    };

    for handle in handles.iter().copied() {
        let runtime = unsafe { &*runtime_ptr };
        if runtime.dom_host().owner_document_handle(handle) != Some(document_handle) {
            continue;
        }
        let style = ComputedStyleRead::new(runtime, handle);
        let rendered = style.css_lifecycle_is_rendered();
        let before_samples = capture_transition_animation_snapshot(runtime, handle, now);
        events.extend(
            scan_element_animations(runtime, &style, rendered, handle, now, changed_at, owner)
                .into_iter()
                .chain(scan_element_transitions(
                    runtime,
                    &style,
                    rendered,
                    handle,
                    now,
                    owner,
                    &before_samples,
                ))
                .map(|event| (handle, event)),
        );
    }
    {
        let mut document = unsafe { &*runtime_ptr }
            .animation_transition_state()
            .document_mut(owner);
        document.reconciling_style = false;
        document.engine.prune();
        events.append(&mut document.pending_events);
        let effects = handles
            .iter()
            .filter_map(|handle| document.elements.get(handle))
            .flat_map(|s| {
                s.animations
                    .iter()
                    .chain(s.running_transitions.iter().map(|t| &t.animation))
            })
            .cloned()
            .collect::<Vec<_>>();
        for effect in effects {
            if effect
                .borrow()
                .service_owner
                .is_some_and(|target| target.owner() == owner)
            {
                document.activate(&effect);
            }
        }
    }
    events.extend(advance_active_animations(
        unsafe { &*runtime_ptr },
        owner,
        now,
    ));
    // Complete the whole style-change event before invoking any script. An
    // earlier element's callback must not change another element's old phase.
    let dispatched = dispatch_lifecycle_events(scope, runtime_ptr, document_handle, owner, events);
    unsafe { &*runtime_ptr }
        .animation_transition_state()
        .finish_style_sample(owner);
    dispatched
}

fn document_elements_in_tree_order(
    runtime: &JsContextHost,
    document_handle: DomHandle,
) -> Vec<DomHandle> {
    let mut out = Vec::new();
    let mut stack = vec![document_handle];
    while let Some(handle) = stack.pop() {
        let Some(node) = runtime.dom_host().node(handle) else {
            continue;
        };
        if node.as_element().is_some() {
            out.push(handle);
        }
        let mut children = runtime.dom_host().child_handles(handle).collect::<Vec<_>>();
        if node.as_element().is_some()
            && let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle)
        {
            let mut shadow_children = runtime
                .dom_host()
                .child_handles(shadow_root)
                .collect::<Vec<_>>();
            shadow_children.append(&mut children);
            children = shadow_children;
        }
        stack.extend(children.into_iter().rev());
    }
    out
}

/// Establish the complete native association before timing can produce events.
/// Creating a JS wrapper only exposes it; it must not complete initialization.
fn initialize_css_animation(
    runtime: &JsContextHost,
    instance: &mut AnimationInstance,
    handle: DomHandle,
    owner: WindowDocumentOwner,
) -> bool {
    let Some(document) = runtime.dom_host().owner_document_handle(handle) else {
        return false;
    };
    let Some(origin) = runtime.document_animation_origin(document) else {
        return false;
    };
    let Some(dispatch) = runtime.owner_dispatch_scope_for_node(handle) else {
        return false;
    };
    instance.execution_owner = Some(crate::native_bridge::WindowDocumentTaskTarget::new(
        owner, dispatch,
    ));
    instance.service_owner = instance.execution_owner;
    instance.target = Some(handle);
    instance.timeline_document = Some(document);
    instance.timeline_origin = Some(origin);
    instance.css_owner = Some(handle);
    true
}

/// State changes are committed before any event callback runs. A callback may
/// queue another style scan or replace the Document; no state borrow crosses it.
fn scan_element_animations(
    runtime: &JsContextHost,
    style: &ComputedStyleRead<'_>,
    rendered: bool,
    handle: DomHandle,
    now: Instant,
    changed_at: Instant,
    owner: WindowDocumentOwner,
) -> Vec<CssLifecycleEvent> {
    let mut declarations = if rendered {
        style
            .style_snapshot()
            .map(|snapshot| {
                crate::style_engine::css_animation_declarations(&snapshot.computed_values())
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut document = runtime.animation_transition_state().document_mut(owner);
    if declarations.is_empty() && !document.elements.contains_key(&handle) {
        return Vec::new();
    }
    let mut previous = std::mem::take(&mut document.element_mut(handle).animations);
    drop(document);
    let mut current = Vec::with_capacity(declarations.len());
    // Match duplicate names from the end, but retain the actual instance.
    for (index, declaration) in declarations.drain(..).enumerate().rev() {
        let Some(frames) = super::css_animation_effect_keyframes(
            style,
            &declaration.name,
            &declaration.timing.easing,
        ) else {
            continue;
        };
        let animation = if let Some(index) = previous
            .iter()
            .rposition(|old| old.borrow().name == declaration.name)
        {
            previous.remove(index)
        } else {
            AnimationInstance::new(AnimationKind::CssAnimation, declaration.name.clone(), now)
                .shared()
        };
        {
            let mut instance = animation.borrow_mut();
            instance.composite_order = index as u64;
            if instance.execution_owner.is_none()
                && !initialize_css_animation(runtime, &mut instance, handle, owner)
            {
                continue;
            }
            instance.update_css_timing(declaration.timing, changed_at);
            if instance.css_effect_source == crate::animation_model::CssEffectSource::Css {
                instance.set_keyframes(frames);
            }
        }
        current.push(animation);
    }
    current.reverse();
    let mut document = runtime.animation_transition_state().document_mut(owner);
    document.element_mut(handle).animations = current;
    let mut events = Vec::new();
    events.extend(
        previous
            .into_iter()
            .flat_map(|animation| animation_cancel_event(animation, now)),
    );

    events
}

pub(crate) fn queue_animation_instance_sample(runtime: &JsContextHost, instance: &Animation) {
    let (owner, target) = {
        let state = instance.borrow();
        (
            state.service_owner,
            state
                .css_owner
                .or(state.target)
                .unwrap_or_else(|| runtime.document_handle()),
        )
    };
    let Some(owner) = owner else {
        return;
    };
    if runtime.current_window_document_task_target_for_dispatch_scope(owner.dispatch_scope())
        != Some(owner)
    {
        return;
    }
    let events = observe_animation_phase(instance, runtime.animation_time());
    runtime
        .animation_transition_state()
        .document_mut(owner.owner())
        .pending_events
        .extend(events.into_iter().map(|event| (target, event)));
}

/// Explicit finish synchronously queues its event, including when an earlier
/// style observation has already staged the same generation's notification.
pub(crate) fn promote_animation_instance_finish(runtime: &JsContextHost, instance: &Animation) {
    let owner = instance.borrow().service_owner;
    let Some(owner) = owner else {
        return;
    };
    if runtime.current_window_document_task_target_for_dispatch_scope(owner.dispatch_scope())
        != Some(owner)
    {
        return;
    }
    let mut document = runtime
        .animation_transition_state()
        .document_mut(owner.owner());
    if let Some((_, CssLifecycleEvent::Playback(playback))) =
        document.pending_events.iter_mut().find(|(_, event)| {
            matches!(event, CssLifecycleEvent::Playback(playback)
            if playback.notification.kind == moli_animation::PlaybackNotificationKind::Finish
                && std::rc::Rc::ptr_eq(&playback.animation, instance)
                && instance.borrow().playback.accepts(playback.notification))
        })
    {
        playback.queue(runtime.animation_time());
    }
}

/// The core has already committed phase and notification history. This adapter
/// only maps plain engine records to the browser's CSS/WAAPI event envelopes.
fn browser_animation_events(
    handle: &Animation,
    records: Vec<moli_animation::AnimationEvent>,
    now: Instant,
) -> Vec<CssLifecycleEvent> {
    use moli_animation::{AnimationEvent, PhaseEvent};
    let animation = handle.borrow();
    records
        .into_iter()
        .filter_map(|record| match record {
            AnimationEvent::Run { elapsed } => (animation.owned_by_css
                && animation.kind == AnimationKind::CssTransition)
                .then(|| {
                    CssLifecycleEvent::Transition("transitionrun", animation.name.clone(), elapsed)
                }),
            AnimationEvent::Playback(notification) => Some(CssLifecycleEvent::playback(
                handle.clone(),
                notification,
                now,
            )),
            AnimationEvent::Phase { kind, elapsed } if animation.owned_by_css => {
                match animation.kind {
                    AnimationKind::CssAnimation => Some(CssLifecycleEvent::Animation(
                        match kind {
                            PhaseEvent::Start => "animationstart",
                            PhaseEvent::End => "animationend",
                            PhaseEvent::Cancel => "animationcancel",
                        },
                        animation.name.clone(),
                        elapsed,
                    )),
                    AnimationKind::CssTransition => Some(CssLifecycleEvent::Transition(
                        match kind {
                            PhaseEvent::Start => "transitionstart",
                            PhaseEvent::End => "transitionend",
                            PhaseEvent::Cancel => "transitioncancel",
                        },
                        animation.name.clone(),
                        elapsed,
                    )),
                    AnimationKind::Script => None,
                }
            }
            AnimationEvent::Phase { .. } => None,
        })
        .collect()
}

fn observe_animation_phase(animation: &Animation, now: Instant) -> Vec<CssLifecycleEvent> {
    let records = animation
        .borrow_mut()
        .playback
        .advance(now, moli_animation::AdvanceMode::Observe);
    browser_animation_events(animation, records, now)
}

fn animation_cancel_event(animation: Animation, now: Instant) -> Vec<CssLifecycleEvent> {
    animation.borrow_mut().cancel(now);
    let events = observe_animation_phase(&animation, now);
    animation.borrow_mut().disassociate_css();
    events
}

/// Only start/end boundaries need a wake; a running infinite animation does
/// not keep a periodic timer or create repeated rendering turns.
pub(crate) fn next_animation_transition_deadline(
    runtime: &JsContextHost,
    owner: WindowDocumentOwner,
) -> Option<Instant> {
    runtime
        .animation_transition_state()
        .document(owner)?
        .engine
        .next_deadline(Instant::now())
}

fn animation_service_owner(
    runtime: &JsContextHost,
    animation: &Animation,
) -> Option<crate::native_bridge::WindowDocumentTaskTarget> {
    let state = animation.borrow();
    // Keep an existing live execution lane. Merely using another Document's
    // clock does not transfer rendering tasks to that Document. If this lane
    // retires, a live target/timeline can continue servicing the native state.
    if let Some(owner) = state.execution_owner {
        use crate::native_bridge::OwnerDispatchScope;
        let document = match owner.dispatch_scope() {
            OwnerDispatchScope::Top => Some(runtime.document_handle()),
            OwnerDispatchScope::Child(frame) => {
                runtime.child_browsing_context_document_handle(frame)
            }
            OwnerDispatchScope::LightweightPopup(id) => {
                runtime.lightweight_popup_document_handle(id)
            }
        };
        if document.is_some_and(|document| runtime.document_is_fully_active(document))
            && runtime
                .current_window_document_task_target_for_dispatch_scope(owner.dispatch_scope())
                == Some(owner)
        {
            return Some(owner);
        }
    }
    for node in [state.target, state.timeline_document]
        .into_iter()
        .flatten()
    {
        let document = runtime
            .dom_host()
            .owner_document_handle(node)
            .unwrap_or(node);
        if runtime.document_is_fully_active(document)
            && let Some(dispatch) = runtime.owner_dispatch_scope_for_node(node)
            && let Some(owner) =
                runtime.current_window_document_task_target_for_dispatch_scope(dispatch)
        {
            return Some(owner);
        }
    }
    None
}

// Service routing changes at admission/target changes and Document retirement.
// It is not a new permission check on every sample; queueing and notification
// delivery still validate their exact owners at their execution boundaries.
pub(crate) fn rehome_animation_service(runtime: &JsContextHost, animation: &Animation) {
    let service_owner = animation_service_owner(runtime, animation);
    let previous = animation.borrow().service_owner;
    if previous == service_owner {
        return;
    }
    animation.borrow_mut().service_owner = service_owner;
    if let Some(owner) = service_owner {
        runtime
            .animation_transition_state()
            .document_mut(owner.owner())
            .activate(animation);
        runtime.queue_animation_timeline_tick_for_owner(owner);
    }
}

pub(crate) fn refresh_animation_timeline(runtime: &JsContextHost, animation: &Animation) {
    let changed = {
        let mut state = animation.borrow_mut();
        let timeline =
            state
                .timeline_document
                .map_or(moli_animation::TimelineState::None, |document| {
                    if runtime.document_is_fully_active(document) {
                        moli_animation::TimelineState::Active
                    } else {
                        moli_animation::TimelineState::Inactive
                    }
                });
        if timeline == state.playback.timeline() {
            return;
        }
        let _ = state.playback.command(
            moli_animation::AnimationCommand::SetTimeline(timeline),
            runtime.animation_time(),
        );
        state.service_owner
    };
    if let Some(owner) = changed {
        runtime
            .animation_transition_state()
            .document_mut(owner.owner())
            .activate(animation);
        runtime.queue_animation_timeline_tick_for_owner(owner);
    }
}

fn advance_active_animations(
    runtime: &JsContextHost,
    owner: WindowDocumentOwner,
    now: Instant,
) -> Vec<(DomHandle, CssLifecycleEvent)> {
    let active = runtime
        .animation_transition_state()
        .document(owner)
        .map(|document| {
            document
                .engine
                .active_instances()
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for animation in active {
        refresh_animation_timeline(runtime, &animation);
    }
    let batch = runtime
        .animation_transition_state()
        .document_mut(owner)
        .engine
        .advance(now);
    if !batch.updates.is_empty() {
        runtime.invalidate_animation_layout();
    }
    let mut events = Vec::new();
    for update in batch.updates {
        let animation = update.animation;
        if !animation
            .borrow()
            .service_owner
            .is_some_and(|target| target.owner() == owner)
        {
            continue;
        }
        let (target, completed_transition) = {
            let state = animation.borrow();
            (
                state
                    .css_owner
                    .or(state.target)
                    .unwrap_or_else(|| runtime.document_handle()),
                (state.kind == AnimationKind::CssTransition
                    && state.playback.committed_phase() == AnimationPhase::After)
                    .then_some((state.css_owner, state.playback.id())),
            )
        };
        if let Some((Some(target), id)) = completed_transition
            && let Some(element) = runtime
                .animation_transition_state()
                .document_mut(owner)
                .elements
                .get_mut(&target)
        {
            // Retire the CSS running record before end callbacks can hide or
            // detach it. The shared instance remains available to its wrapper.
            element
                .running_transitions
                .retain(|t| t.animation.borrow().playback.id() != id);
        }
        events.extend(
            browser_animation_events(&animation, update.events, now)
                .into_iter()
                .map(|event| (target, event)),
        );
        let stale = {
            let mut state = animation.borrow_mut();
            if !state.playback.is_relevant(now) {
                state
                    .registered_target
                    .take()
                    .map(|target| (target, state.playback.id()))
            } else {
                None
            }
        };
        if let Some((target, id)) = stale {
            runtime
                .animation_transition_state()
                .remove_script_reference(target, id);
            events.push((
                target,
                CssLifecycleEvent::playback(
                    animation.clone(),
                    animation
                        .borrow()
                        .playback
                        .notification(moli_animation::PlaybackNotificationKind::Release),
                    now,
                ),
            ));
        }
    }
    events
}

pub(crate) fn dispatch_animation_timeline_tick(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document: DomHandle,
    owner: WindowDocumentOwner,
) -> bool {
    let now = unsafe { &*runtime_ptr }.animation_time();
    let _sample = unsafe { &*runtime_ptr }
        .animation_transition_state()
        .enter_script_time(now);
    let events = {
        let mut state = unsafe { &*runtime_ptr }
            .animation_transition_state()
            .document_mut(owner);
        std::mem::take(&mut state.pending_events)
    };
    let events = events
        .into_iter()
        .chain(advance_active_animations(
            unsafe { &*runtime_ptr },
            owner,
            now,
        ))
        .collect();
    dispatch_lifecycle_events(scope, runtime_ptr, document, owner, events)
}

fn scan_element_transitions(
    runtime: &JsContextHost,
    style: &ComputedStyleRead<'_>,
    rendered: bool,
    handle: DomHandle,
    now: Instant,
    owner: WindowDocumentOwner,
    before_samples: &TransitionAnimationSnapshot,
) -> Vec<CssLifecycleEvent> {
    if !rendered {
        let mut document = runtime.animation_transition_state().document_mut(owner);
        document.before_styles.remove(&handle);
        let Some(state) = document.elements.get_mut(&handle) else {
            return Vec::new();
        };
        let mut events = Vec::new();
        for transition in state.running_transitions.drain(..) {
            cancel_transition(&transition, now, &mut events);
        }
        return events;
    }
    let current_style = style.style_snapshot();
    let (before_style, has_running) = {
        let mut document = runtime.animation_transition_state().document_mut(owner);
        let before_style = match current_style {
            Some(style) => document.before_styles.insert(handle, style.clone()),
            None => document.before_styles.remove(&handle),
        };
        let has_running = document
            .elements
            .get(&handle)
            .is_some_and(|state| !state.running_transitions.is_empty());
        (before_style, has_running)
    };
    let durations = css_time_list(&style.property("transition-duration"));
    let delays = css_delay_list(&style.property("transition-delay"));
    let has_timing = durations.iter().any(|duration| !duration.is_zero())
        || delays.iter().any(|delay| delay.is_positive());
    // Timing changes affect new transitions, not the lifetime of existing ones.
    if !has_timing && !has_running {
        return Vec::new();
    }
    // Compare before/after animation-origin values at one instant. Transitions
    // sit above those effects and are sampled separately when interrupted.
    // Keep the canonical baselines for declaration invalidation, not as the
    // visual endpoints of an effect covered by CSS/WAAPI animation.
    let before_style = before_style.map(|before| {
        style.project_animation_samples_from(&before.computed_values(), &before_samples.values)
    });
    let current_style = current_style.map(|base| {
        style.project_animation_samples_from(&base.computed_values(), &before_samples.values)
    });
    // Finish style reads before borrowing state; even a lazy fallback may make
    // another ordinary style observation and establish its initial baseline.
    let declarations = transition_declarations(style, &durations, &delays);
    // Style dependency projection can observe ancestors in this Document.
    // Keep no mutable Document borrow while calculating interrupted effects.
    let mut running = {
        let mut document = runtime.animation_transition_state().document_mut(owner);
        std::mem::take(&mut document.element_mut(handle).running_transitions)
    };
    running.retain(|t| {
        let state = t.animation.borrow();
        !state.playback.is_idle() && state.playback.committed_phase() != AnimationPhase::After
    });
    let mut events = Vec::new();
    running.retain(|transition| {
        if declarations
            .binary_search_by(|declaration| {
                declaration
                    .property
                    .as_ref()
                    .cmp(transition.property.as_str())
            })
            .is_ok()
        {
            true
        } else {
            cancel_transition(transition, now, &mut events);
            false
        }
    });
    for TransitionDeclaration {
        property,
        allow_discrete,
        delay,
        mut duration,
        easing,
    } in declarations
    {
        // Expanding `all` must not serialize every unchanged longhand. Compare
        // the retained computed values through Stylo's transition metadata first.
        if let (Some(before), Some(after)) = (before_style.as_ref(), current_style.as_ref()) {
            use style::properties::{
                PropertyDeclarationId, PropertyId, animated_properties::AnimationValue,
            };
            let before = before.computed_values();
            let after = after.computed_values();
            let changed = match PropertyId::parse_enabled_for_all_content(&property) {
                Ok(PropertyId::Custom(name)) => AnimationValue::is_different_for(
                    PropertyDeclarationId::Custom(&name),
                    &before,
                    &after,
                ),
                Ok(PropertyId::NonCustom(id)) => id.as_longhand().is_some_and(|id| {
                    AnimationValue::is_different_for(
                        PropertyDeclarationId::Longhand(id),
                        &before,
                        &after,
                    )
                }),
                Err(_) => false,
            };
            if !changed {
                continue;
            }
        }
        let value = current_style
            .as_ref()
            .and_then(|style| style.property_value(&property))
            .unwrap_or_else(|| style.property(&property));
        let previous = before_style
            .as_ref()
            .and_then(|style| style.property_value(&property));
        if previous.as_ref().is_none_or(|previous| *previous == value) {
            continue;
        }
        let property = property.into_owned();
        let old = running
            .iter()
            .position(|transition| transition.property == property)
            .map(|index| running.remove(index));
        let mut from = previous.unwrap_or_default();
        let mut factor = 1.0;
        let mut adjusted = from.clone();
        let mut delay = delay;
        if let Some(old) = old {
            if old.to == value {
                running.push(old);
                continue;
            }
            from = old
                .animation
                .borrow()
                .sample_interval(&property, now)
                .and_then(|sample| sample_property_from_interval(style, sample))
                .unwrap_or_else(|| old.to.clone());
            if old.reversing_adjusted_start == value {
                let progress = old.animation.borrow().progress(now).unwrap_or(1.0);
                let timing = moli_animation::reverse_transition_timing(
                    duration,
                    delay,
                    progress,
                    old.shortening_factor,
                );
                factor = timing.shortening_factor;
                duration = timing.duration;
                delay = timing.delay;
                adjusted = old.to.clone();
            } else {
                adjusted = from.clone();
            }
            cancel_transition(&old, now, &mut events);
        }
        if from == value || !delay.add_duration(duration).is_positive() {
            continue;
        }
        if !current_style.as_ref().is_some_and(|base| {
            runtime.transition_values_are_eligible(
                runtime
                    .dom_host()
                    .owner_document_handle(handle)
                    .unwrap_or_else(|| runtime.document_handle()),
                handle,
                &base.computed_values(),
                crate::style_engine::TransitionValuePair {
                    property: &property,
                    from: &from,
                    to: &value,
                    allow_discrete,
                },
            )
        }) {
            continue;
        }
        let mut instance =
            AnimationInstance::new(AnimationKind::CssTransition, property.clone(), now);
        if !initialize_css_animation(runtime, &mut instance, handle, owner) {
            continue;
        }
        instance.playback.delay = delay;
        instance.playback.duration = duration;
        instance.playback.fill = FillMode::Backwards;
        instance.easing = (*easing).clone();
        let mut frames = crate::animation_model::KeyframeTrackBuilder::default();
        for (offset, value) in [(0.0, &from), (1.0, &value)] {
            frames.push_frame(
                offset,
                None,
                crate::animation_model::KeyframeDeclarations::from_css(vec![(
                    property.clone(),
                    value.clone(),
                )]),
            );
        }
        instance.set_keyframes(frames.finish());
        let animation = instance.shared();
        running.push(RunningTransition {
            property: property.clone(),
            animation,
            to: value,
            reversing_adjusted_start: adjusted,
            shortening_factor: factor,
        });
    }
    runtime
        .animation_transition_state()
        .document_mut(owner)
        .element_mut(handle)
        .running_transitions = running;
    events
}

fn cancel_transition(
    transition: &RunningTransition,
    now: Instant,
    events: &mut Vec<CssLifecycleEvent>,
) {
    transition.animation.borrow_mut().cancel(now);
    events.extend(observe_animation_phase(&transition.animation, now));
}

fn parse_css_seconds(value: &str) -> Option<f64> {
    moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::Time,
        moli_css_parse::CssNumericContext::supports_probe(),
    )?
    .time_seconds()
}

fn parse_css_time(value: &str) -> Option<Duration> {
    let seconds = parse_css_seconds(value)?;
    (seconds >= 0.0).then(|| Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX))
}

fn dispatch_lifecycle_events(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    owner: WindowDocumentOwner,
    mut events: Vec<(DomHandle, CssLifecycleEvent)>,
) -> bool {
    // Promise reactions observe the entire committed batch. Playback events
    // retain their own cycle token, so a reaction starting a new run cannot
    // resolve its new finished/ready promise with an old notification.
    let mut published = false;
    for (_, event) in &mut events {
        if !lifecycle_owner_is_current(scope, runtime_ptr, owner) {
            return false;
        }
        if let CssLifecycleEvent::Playback(playback) = event {
            crate::context_bootstrap::publish_animation_playback(scope, playback);
            published = true;
        }
    }
    if published {
        crate::script_execution::perform_animation_microtask_checkpoint(scope);
    }
    let mut dispatched = false;
    for (handle, event) in events {
        if !lifecycle_owner_is_current(scope, runtime_ptr, owner) {
            break;
        }
        if !matches!(&event, CssLifecycleEvent::Playback(..))
            && unsafe { &*runtime_ptr }
                .dom_host()
                .owner_document_handle(handle)
                != Some(document_handle)
        {
            continue;
        }
        if let CssLifecycleEvent::Playback(playback) = &event {
            crate::context_bootstrap::dispatch_animation_playback(scope, playback);
            continue;
        }
        let target =
            crate::host::event_target_value(scope, runtime_ptr, EventTargetHandle::Node(handle))
                .unwrap_or_else(|_| v8::undefined(scope).into());
        let event = match event {
            CssLifecycleEvent::Playback(..) => unreachable!("playback notifications handled above"),
            CssLifecycleEvent::Animation(kind, name, elapsed) => {
                crate::host::create_animation_event(
                    scope, kind, &name, elapsed, target, true, false,
                )
            }
            CssLifecycleEvent::Transition(kind, property, elapsed) => {
                crate::host::create_transition_event(
                    scope, kind, &property, elapsed, target, true, false,
                )
            }
        };
        if let Ok(event) = event {
            let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
            dispatched = true;
        }
    }
    dispatched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_timing_lists_repeat_every_component() {
        let values = css_time_list("1s, 2s");
        for (index, seconds) in [1, 2, 1, 2, 1, 2].into_iter().enumerate() {
            assert_eq!(
                cyclic_list_item(&values, index).unwrap_or_default(),
                Duration::from_secs(seconds)
            );
        }
        assert_eq!(cyclic_list_item::<Duration>(&[], 3), None);
    }

    #[test]
    fn css_times_outside_duration_range_do_not_panic() {
        assert!(
            parse_css_time("0.1s")
                .unwrap()
                .abs_diff(Duration::from_millis(100))
                < Duration::from_micros(1)
        );
        assert_eq!(parse_css_time("1e30s"), Some(Duration::MAX));
    }
}
