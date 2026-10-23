use std::time::{Duration, Instant};

use crate::document_runtime::{DomHandle, EventTargetHandle};
use crate::native_bridge::JsContextHost;
use crate::native_bridge::context_host::{RunningAnimation, RunningTransition};

use super::ComputedStyleRead;
use super::css_animation_start_applies;
use super::events::dispatch_public_event;
use super::styles::StyleObservation;

pub(crate) fn queue_animation_start_for_listener_target(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    target: EventTargetHandle,
) {
    let _ = unsafe { &mut *runtime_ptr }.queue_animation_start_scan(scope, target);
}

pub(crate) fn element_tree_has_css_lifecycle(runtime: &JsContextHost, root: DomHandle) -> bool {
    let mut stack = vec![root];
    while let Some(handle) = stack.pop() {
        let Some(node) = runtime.dom_host().node(handle) else {
            continue;
        };
        if node.as_element().is_some()
            && runtime
                .animation_transition_state()
                .has_lifecycle_state(handle)
        {
            return true;
        }
        let mut children = runtime.dom_host().child_handles(handle).collect::<Vec<_>>();
        if node.as_element().is_some()
            && let Some(shadow_root) = runtime.dom_host().shadow_root_handle(handle)
        {
            children.extend(runtime.dom_host().child_handles(shadow_root));
        }
        stack.extend(children);
    }
    false
}

fn active_animation_handles(runtime: &JsContextHost, document_handle: DomHandle) -> Vec<DomHandle> {
    let mut out = Vec::new();
    let mut stack = vec![document_handle];
    // The traversal has no script callbacks. Share its style observation, then
    // revalidate each candidate separately when dispatching reentrant listeners.
    let mut observation = StyleObservation::new(runtime);
    while let Some(handle) = stack.pop() {
        let Some(node) = runtime.dom_host().node(handle) else {
            continue;
        };
        if node.is_connected()
            && node.as_element().is_some()
            && observation.read(handle).css_animation_start_applies()
        {
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

/// Run one lightweight compatibility scan inside the exact Document context
/// authorized by the Page rendering-update arbiter.
pub(crate) fn dispatch_animation_start_scan(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
    target: EventTargetHandle,
) -> bool {
    match target {
        EventTargetHandle::Window => {
            let runtime = unsafe { &*runtime_ptr };
            let handles = active_animation_handles(runtime, document_handle);
            let mut dispatched = false;
            for handle in handles {
                dispatched |= dispatch_animation_start_if_applies(scope, runtime_ptr, handle);
            }
            dispatched
        }
        EventTargetHandle::Node(handle) => {
            dispatch_animation_start_if_applies(scope, runtime_ptr, handle)
        }
        EventTargetHandle::ChildWindow(_) => false,
    }
}

fn dispatch_animation_start_if_applies(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    // Revalidate at dispatch time so listener removal, DOM removal, or style
    // changes between registration and the queued task do not produce stale events.
    if !css_animation_start_applies(runtime, handle) {
        return false;
    }
    // Keep the shared state in sync with rendering-update scans, while preserving
    // the listener-registration compatibility behavior: each newly registered
    // listener gets its own retroactive animationstart scan.
    let names = active_animation_names(runtime, handle);
    let name = names.last().cloned().unwrap_or_default();
    unsafe { &mut *runtime_ptr }
        .animation_transition_state_mut()
        .started_animations_mut(handle)
        .insert(name.clone());
    if name.is_empty() {
        return false;
    }
    let target =
        crate::host::event_target_value(scope, runtime_ptr, EventTargetHandle::Node(handle))
            .unwrap_or_else(|_| v8::undefined(scope).into());
    if let Ok(event) = crate::host::create_animation_event(
        scope,
        "animationstart",
        &name,
        0.0,
        target,
        true,
        false,
    ) {
        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
        return true;
    }
    false
}

fn active_animation_names(runtime: &JsContextHost, handle: DomHandle) -> Vec<String> {
    let raw = ComputedStyleRead::new(runtime, handle).property("animation-name");
    raw.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("none"))
        .map(str::to_owned)
        .collect()
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
) -> bool {
    let handles = document_elements_in_tree_order(runtime_ptr, document_handle);
    let now = Instant::now();
    let mut dispatched = false;

    // Drop state for elements no longer connected before processing the tree.
    let connected: std::collections::HashSet<DomHandle> = handles.iter().copied().collect();
    {
        let runtime = unsafe { &mut *runtime_ptr };
        runtime
            .animation_transition_state_mut()
            .retain_connected(&connected);
    }

    for handle in handles {
        dispatched |= scan_element_animations(scope, runtime_ptr, handle, now);
        dispatched |= scan_element_transitions(scope, runtime_ptr, handle, now);
    }
    dispatched
}

fn document_elements_in_tree_order(
    runtime_ptr: *mut JsContextHost,
    document_handle: DomHandle,
) -> Vec<DomHandle> {
    let runtime = unsafe { &*runtime_ptr };
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

fn scan_element_animations(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    now: Instant,
) -> bool {
    let (applies, names) = {
        let runtime = unsafe { &*runtime_ptr };
        if !css_animation_start_applies(runtime, handle) {
            (false, Vec::new())
        } else {
            (true, active_animation_names(runtime, handle))
        }
    };

    let mut dispatched = false;
    if !applies {
        // The element no longer animates: cancel anything in flight and forget
        // the started set so a later re-application fires `animationstart` again.
        let cancelled = {
            let runtime = unsafe { &mut *runtime_ptr };
            let state = runtime.animation_transition_state_mut();
            let running = state.running_animations(handle).to_vec();
            state.forget_animations(handle);
            running
        };
        for animation in cancelled {
            dispatched |= dispatch_animation_cancel(scope, runtime_ptr, handle, &animation.name);
        }
        return dispatched;
    }

    let (delay, duration, iterations) = {
        let runtime = unsafe { &*runtime_ptr };
        animation_timing(runtime, handle)
    };

    // Newly applied animation names.
    let new_names = {
        let runtime = unsafe { &mut *runtime_ptr };
        let started = runtime
            .animation_transition_state_mut()
            .started_animations_mut(handle);
        let mut new_names = Vec::new();
        for name in &names {
            if started.insert(name.clone()) {
                new_names.push(name.clone());
            }
        }
        new_names
    };
    for name in &new_names {
        dispatched |= dispatch_animation_start(scope, runtime_ptr, handle, name);
        let runtime = unsafe { &mut *runtime_ptr };
        runtime
            .animation_transition_state_mut()
            .running_animations_mut(handle)
            .push(RunningAnimation {
                name: name.clone(),
                started: now,
                delay,
                duration,
                iterations,
            });
    }

    // Completion / iteration for running animations.
    let mut finished = Vec::new();
    {
        let runtime = unsafe { &mut *runtime_ptr };
        let running = runtime
            .animation_transition_state_mut()
            .running_animations_mut(handle);
        running.retain(|animation| {
            let active =
                animation.delay + active_duration(animation.duration, animation.iterations);
            let done =
                !active.is_zero() && now.saturating_duration_since(animation.started) >= active;
            let done = done || (active.is_zero() && now > animation.started);
            if done {
                finished.push(animation.name.clone());
            }
            !done
        });
    }
    for name in finished {
        dispatched |= dispatch_animation_end(scope, runtime_ptr, handle, &name, duration);
    }
    dispatched
}

fn active_duration(duration: Duration, iterations: f64) -> Duration {
    if iterations.is_finite() {
        duration.mul_f64(iterations.max(0.0))
    } else {
        // Infinite animations never complete.
        Duration::MAX
    }
}

fn animation_timing(runtime: &JsContextHost, handle: DomHandle) -> (Duration, Duration, f64) {
    let delay = first_time(ComputedStyleRead::new(runtime, handle).property("animation-delay"));
    let duration =
        first_time(ComputedStyleRead::new(runtime, handle).property("animation-duration"));
    let iterations = ComputedStyleRead::new(runtime, handle)
        .property("animation-iteration-count")
        .split(',')
        .next()
        .map(str::trim)
        .and_then(|value| {
            if value.eq_ignore_ascii_case("infinite") {
                Some(f64::INFINITY)
            } else {
                value.parse::<f64>().ok()
            }
        })
        .unwrap_or(1.0);
    (delay, duration, iterations)
}

fn scan_element_transitions(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    now: Instant,
) -> bool {
    let mut dispatched = false;
    let has_nonzero_duration = {
        let runtime = unsafe { &*runtime_ptr };
        ComputedStyleRead::new(runtime, handle)
            .property("transition-duration")
            .split(',')
            .filter_map(|value| parse_css_time(value.trim()))
            .any(|duration| !duration.is_zero())
    };
    if !has_nonzero_duration {
        let cancelled = {
            let runtime = unsafe { &mut *runtime_ptr };
            let state = runtime.animation_transition_state_mut();
            let running = state.running_transitions(handle).to_vec();
            state.forget_transitions(handle);
            running
        };
        for transition in cancelled {
            dispatched |=
                dispatch_transition_cancel(scope, runtime_ptr, handle, &transition.property);
        }
        return dispatched;
    }

    let properties = {
        let runtime = unsafe { &*runtime_ptr };
        transitioned_properties(runtime, handle)
    };
    if properties.is_empty() {
        let cancelled = {
            let runtime = unsafe { &mut *runtime_ptr };
            let state = runtime.animation_transition_state_mut();
            let running = state.running_transitions(handle).to_vec();
            state.forget_transitions(handle);
            running
        };
        for transition in cancelled {
            dispatched |=
                dispatch_transition_cancel(scope, runtime_ptr, handle, &transition.property);
        }
        return dispatched;
    }

    for property in properties {
        let current = {
            let runtime = unsafe { &*runtime_ptr };
            ComputedStyleRead::new(runtime, handle).property(&property)
        };
        let (delay, duration) = {
            let runtime = unsafe { &*runtime_ptr };
            transition_timing(runtime, handle, &property)
        };
        let previous = {
            let runtime = unsafe { &mut *runtime_ptr };
            runtime
                .animation_transition_state_mut()
                .transition_values(handle)
                .and_then(|values| values.get(&property).cloned())
        };
        if previous.as_deref() == Some(current.as_str()) {
            continue;
        }
        {
            let runtime = unsafe { &mut *runtime_ptr };
            runtime
                .animation_transition_state_mut()
                .transition_values_mut(handle)
                .insert(property.clone(), current.clone());
        }
        let Some(previous) = previous else {
            // First observation establishes a baseline; no transition starts.
            continue;
        };
        if previous == current {
            continue;
        }
        if duration.is_zero() {
            continue;
        }

        // Interrupt any in-flight transition of the same property.
        let interrupted = {
            let runtime = unsafe { &mut *runtime_ptr };
            let state = runtime.animation_transition_state_mut();
            let running = state.running_transitions_mut(handle);
            let mut removed = Vec::new();
            running.retain(|transition| {
                if transition.property == property {
                    removed.push(transition.clone());
                    false
                } else {
                    true
                }
            });
            removed
        };
        for transition in &interrupted {
            dispatched |=
                dispatch_transition_cancel(scope, runtime_ptr, handle, &transition.property);
        }

        dispatched |=
            dispatch_transition_event(scope, runtime_ptr, handle, "transitionrun", &property, 0.0);
        let mut started_event_sent = false;
        if delay.is_zero() {
            dispatched |= dispatch_transition_event(
                scope,
                runtime_ptr,
                handle,
                "transitionstart",
                &property,
                0.0,
            );
            started_event_sent = true;
        }
        let runtime = unsafe { &mut *runtime_ptr };
        runtime
            .animation_transition_state_mut()
            .running_transitions_mut(handle)
            .push(RunningTransition {
                property,
                started: now,
                delay,
                duration,
                started_event_sent,
            });
    }

    // Advance running transitions.
    let mut to_start: Vec<String> = Vec::new();
    let mut to_end: Vec<String> = Vec::new();
    {
        let runtime = unsafe { &mut *runtime_ptr };
        let running = runtime
            .animation_transition_state_mut()
            .running_transitions_mut(handle);
        running.retain_mut(|transition| {
            let elapsed = now.saturating_duration_since(transition.started);
            if !transition.started_event_sent && elapsed >= transition.delay {
                transition.started_event_sent = true;
                to_start.push(transition.property.clone());
            }
            if elapsed >= transition.delay + transition.duration {
                to_end.push(transition.property.clone());
                false
            } else {
                true
            }
        });
    }
    for property in to_start {
        let elapsed = {
            let runtime = unsafe { &mut *runtime_ptr };
            runtime
                .animation_transition_state_mut()
                .running_transitions(handle)
                .iter()
                .find(|transition| transition.property == property)
                .map(|transition| {
                    now.saturating_duration_since(transition.started)
                        .saturating_sub(transition.delay)
                        .as_secs_f64()
                })
                .unwrap_or(0.0)
        };
        dispatched |= dispatch_transition_event(
            scope,
            runtime_ptr,
            handle,
            "transitionstart",
            &property,
            elapsed,
        );
    }
    for property in to_end {
        let elapsed = {
            let runtime = unsafe { &mut *runtime_ptr };
            transition_timing(runtime, handle, &property)
                .1
                .as_secs_f64()
        };
        dispatched |= dispatch_transition_event(
            scope,
            runtime_ptr,
            handle,
            "transitionend",
            &property,
            elapsed,
        );
    }
    dispatched
}

fn transitioned_properties(runtime: &JsContextHost, handle: DomHandle) -> Vec<String> {
    let raw = ComputedStyleRead::new(runtime, handle).property("transition-property");
    if raw
        .split(',')
        .any(|name| name.trim().eq_ignore_ascii_case("all"))
    {
        return transition_candidate_properties()
            .iter()
            .map(|property| (*property).to_owned())
            .collect();
    }
    let mut properties = Vec::new();
    for name in raw.split(',') {
        let name = name.trim();
        if name.is_empty() || name.eq_ignore_ascii_case("all") || name.eq_ignore_ascii_case("none")
        {
            continue;
        }
        properties.push(name.to_owned());
    }
    properties
}

fn transition_candidate_properties() -> &'static [&'static str] {
    &[
        "color",
        "background-color",
        "opacity",
        "width",
        "height",
        "min-width",
        "min-height",
        "max-width",
        "max-height",
        "top",
        "right",
        "bottom",
        "left",
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
        "transform",
    ]
}

fn transition_timing(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> (Duration, Duration) {
    let raw = ComputedStyleRead::new(runtime, handle).property("transition-property");
    let index = raw
        .split(',')
        .map(str::trim)
        .position(|name| name.eq_ignore_ascii_case(property))
        .unwrap_or(0);
    let delay = nth_time(
        ComputedStyleRead::new(runtime, handle).property("transition-delay"),
        index,
    );
    let duration = nth_time(
        ComputedStyleRead::new(runtime, handle).property("transition-duration"),
        index,
    );
    (delay, duration)
}

fn first_time(value: String) -> Duration {
    nth_time(value, 0)
}

fn nth_time(value: String, index: usize) -> Duration {
    let items = split_top_level_commas(&value);
    let token = items
        .get(index)
        .or_else(|| items.first())
        .map(String::as_str);
    token.and_then(parse_css_time).unwrap_or(Duration::ZERO)
}

fn split_top_level_commas(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_css_time(value: &str) -> Option<Duration> {
    let seconds = moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::Time,
        moli_css_parse::CssNumericContext::supports_probe(),
    )?
    .time_seconds()?;
    (seconds >= 0.0).then(|| Duration::from_secs_f64(seconds))
}

fn dispatch_animation_start(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
) -> bool {
    dispatch_animation_event(scope, runtime_ptr, handle, "animationstart", name, 0.0)
}

fn dispatch_animation_end(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
    duration: Duration,
) -> bool {
    dispatch_animation_event(
        scope,
        runtime_ptr,
        handle,
        "animationend",
        name,
        duration.as_secs_f64(),
    )
}

fn dispatch_animation_cancel(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    name: &str,
) -> bool {
    dispatch_animation_event(scope, runtime_ptr, handle, "animationcancel", name, 0.0)
}

fn dispatch_animation_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    event_type: &str,
    name: &str,
    elapsed_time: f64,
) -> bool {
    let target =
        crate::host::event_target_value(scope, runtime_ptr, EventTargetHandle::Node(handle))
            .unwrap_or_else(|_| v8::undefined(scope).into());
    let Ok(event) = crate::host::create_animation_event(
        scope,
        event_type,
        name,
        elapsed_time,
        target,
        true,
        false,
    ) else {
        return false;
    };
    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    true
}

fn dispatch_transition_cancel(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    property: &str,
) -> bool {
    dispatch_transition_event(
        scope,
        runtime_ptr,
        handle,
        "transitioncancel",
        property,
        0.0,
    )
}

fn dispatch_transition_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    event_type: &str,
    property: &str,
    elapsed_time: f64,
) -> bool {
    let target =
        crate::host::event_target_value(scope, runtime_ptr, EventTargetHandle::Node(handle))
            .unwrap_or_else(|_| v8::undefined(scope).into());
    let Ok(event) = crate::host::create_transition_event(
        scope,
        event_type,
        property,
        elapsed_time,
        target,
        true,
        false,
    ) else {
        return false;
    };
    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
    true
}
